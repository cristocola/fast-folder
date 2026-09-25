# fastf on Windows

Windows 10 and 11 are fully supported. Every release ships two Windows artifacts on the [releases page](https://github.com/cristocola/fast-folder/releases):

| Artifact | What it is |
|---|---|
| `fastf-vX.Y.Z-x86_64.msi` | Installer with a setup wizard. Installs `fastf.exe`, adds it to your PATH, and creates a Start Menu entry. |
| `fastf-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Portable archive with `fastf.exe`, PowerShell completions, and the docs. |

Both are self-contained. `fastf.exe` is a single file that uses nothing beyond Windows itself. There is no Visual C++ Redistributable, .NET runtime, or other prerequisite to install first, on a fresh machine or a clean VM.

## Option 1: MSI installer (recommended)

1. Download the `.msi` from the releases page and run it. A standard setup wizard walks you through the license and the install folder.
2. Launch **Fast Folder** from the Start Menu. It opens the guided app in a console window. On the first launch it asks where your projects should live, suggesting `C:\Users\<you>\Projects`, and creates that folder for you.
3. For the CLI, open a **new** terminal (PowerShell or cmd). PATH changes only apply to terminals started after the install. Verify:

```powershell
fastf --version
```

The installer places `fastf.exe` — the CLI and the guided app, one program — under `Program Files\fastf` and adds that directory to the PATH. The Start Menu shortcut runs it.

Upgrading is just running a newer MSI. Uninstall from Windows Settings > Apps like any other program. Uninstalling removes the shortcut and the PATH entry.

## Option 2: portable zip

1. Download and extract the `.zip`. Inside is a folder containing `fastf.exe`.
2. Move the folder somewhere permanent, for example `C:\Tools\fastf`.
3. Add that folder to your PATH. Two ways:

**The Settings way:**

1. Press Win, type "environment variables", open **Edit the system environment variables**.
2. Click **Environment Variables**.
3. Under *User variables*, select **Path**, click **Edit**, then **New**.
4. Enter the folder path (for example `C:\Tools\fastf`) and confirm with OK.
5. Open a new terminal and run `fastf --version`.

**The terminal way** (PowerShell, current user, no admin needed):

```powershell
[Environment]::SetEnvironmentVariable(
  "Path",
  [Environment]::GetEnvironmentVariable("Path", "User") + ";C:\Tools\fastf",
  "User"
)
```

Open a new terminal afterwards.

## Where your data lives

With a normal install, config and templates live in `%APPDATA%\fastf`; `fastf
paths` shows the resolved location and why it was chosen. The ID counter is
**not** kept there: each base directory carries its own `.fastf-counter.toml`
next to the projects it numbers, which is what lets a dual-boot machine hand
out the same next ID from either operating system. For portable mode — every
file beside `fastf.exe` on a USB stick — put an empty `config.toml` next to the
binary before the first run. The details are in [config.md](config.md).

## Paths in templates and config

Always write `/` in templates. fastf converts to `\` on Windows at runtime. Config values accept both styles:

```powershell
fastf config set base-dir C:\Users\you\Projects
fastf config set base-dir C:/Users/you/Projects   # equally fine
```

## Folder names

Windows reserves a handful of names and quietly rewrites others, so fastf adjusts anything that would not survive:

| You type | fastf creates | Why |
|---|---|---|
| `CON`, `NUL`, `COM1`, `LPT9` | `CON_`, `NUL_`, `COM1_`, `LPT9_` | MS-DOS device names, still reserved. Applies with an extension too (`CON.txt`). |
| `Draft.` or `Draft ` | `Draft` | Windows drops trailing dots and spaces, so the folder would not match the name fastf recorded. |
| control characters | `_` | Illegal in Windows filenames. |

These rules run on every platform, not just Windows, so a project created on Linux still opens here.

## Editing templates in Notepad

Save `template.yaml` as **UTF-8**. Notepad and `Out-File -Encoding utf8` in Windows PowerShell 5.1 add a byte-order mark; fastf skips it, so either encoding loads. If a template fails to parse, the error names the file and reminds you to check the encoding.

## Launching without a terminal

None of the terminal-opening machinery described for Linux in
[cli.md](cli.md#launched-from-a-desktop-launcher) exists on Windows, and nothing
is missing: `fastf.exe` is a console application, so starting it from Win+R, a
shortcut, or the Start menu already allocates a console for it. The `terminal`
config key is read on every platform but only acted on where the relaunch
exists.

`fastf term <query>` works here too: it opens Windows Terminal (`wt`) at the
project's folder when it is installed, and a new `cmd` console there otherwise.

## The guided app in the old console

The app draws with box-drawing characters and a small alphabet of its own —
`▸` the cursor, `✓` a mark, `●` a tag, `⌕` search, `⚠` a warning. Windows
Terminal shows all of it. The **legacy console host** (the black window a
double-click on `fastf.exe` opens on Windows 10, or `conhost.exe`) has a font
that often does not, so fastf switches its own alphabet to plain ASCII there
automatically: `>` `*` `*` `/` `!`. Box-drawing borders stay, because the
console has always had those.

The detection is "a Windows host that announces no emulator": no `WT_SESSION`
(Windows Terminal), no `TERM_PROGRAM`, no `TERM`, none of the variables
Alacritty, WezTerm and ConEmu set, and a console that is not a pseudoconsole.
That last check is what finds Windows Terminal when the Start menu or a
shortcut opened fastf: Windows 11 hands the program to Windows Terminal without
setting `WT_SESSION`, but the console is then a pseudoconsole, which the legacy
console never is. Force the alphabet either way:

```powershell
$env:FASTF_ASCII = "1"    # plain ASCII, wherever you are
$env:FASTF_ASCII = "0"    # the Unicode alphabet, even in the old console
```

Colour follows the same rule the app uses everywhere: `NO_COLOR` turns it off,
a terminal that announces truecolor (`COLORTERM=truecolor`, Windows Terminal
however it was opened, a `TERM` or `TERM_PROGRAM` that names a truecolor
emulator) gets Doom One, the default look, and anything else gets the sixteen ANSI
colours used sparingly. `fastf config set theme doom-one|rich|ansi|mono` pins
one, and `FASTF_THEME` pins one for a single run.


## Drives that rclone, WinFsp or a RAM disk provide

A drive letter that a user-mode filesystem mounts — `rclone mount` of an S3 or
SFTP remote, any other WinFsp mount such as a Cryptomator vault, an ImDisk RAM
disk — is not registered with Windows' mount manager, so Windows cannot give
its paths their final drive-letter name. fastf works on such a drive like on
any other: it resolves paths there itself, checking that each folder on the way
exists and is not a link. A junction or symbolic link inside a base on such a
drive is refused rather than followed, because only the name Windows cannot
give would say where it leads. The rclone cache modes (`off`, `writes`,
`full`) all work.

## Moving a project to another drive

A move between drive letters copies, verifies, publishes, and then takes the
original out of the library in one rename before removing it
([projects.md](projects.md#moving-projects-between-bases)). It runs as a job of
its own, with no console window: closing Windows Terminal, or an ssh session
ending, leaves it to finish, and `fastf jobs` in any other window shows it.
Three things are particular to Windows:

- **A file open in another program** — a timeline in an editor, a document in
  Word — stops Windows renaming the folder around it. The move then publishes
  the copy, reports that the original is still there, whole, with nothing
  removed, and says a program has a file in it open. Close the program and run
  `fastf reconcile`: it finishes the move without copying again.
- **Junctions** travel as junctions, pointing at the same folder, and need
  nothing special. **Symbolic links** travel as symbolic links of the same kind
  (to a file, or to a folder), but Windows lets an account make one only with
  Developer Mode on (Settings › System › For developers) or from an elevated
  prompt; without it, a move of a project holding one says so before copying
  anything.
- **OneDrive and other cloud placeholders** are read as ordinary files, which
  downloads them; a mounted volume inside a project, or a link WSL made, is
  named and refused before anything is copied.

## The mouse

fastf never takes the mouse, so text selects as in any program. The wheel
scrolls the list and the panes because Windows Terminal turns it into arrow
keys on the alternate screen; a console that does not leaves the scrolling to
the arrow keys and PageUp/PageDown.

## "VCRUNTIME140.dll was not found"

A `fastf.exe` that asks for that DLL is from an old release that needed the
Visual C++ Redistributable. Every current release carries its own C runtime:
download a newer one and run the MSI over the old install, or replace the
portable `fastf.exe`.

## Building from source on Windows

Install Rust via [rustup](https://rustup.rs) (MSVC toolchain), then:

```powershell
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
# Output: target\release\fastf.exe
```

The repository's `.cargo/config.toml` links the C runtime statically for the MSVC target, so a binary you build yourself is as self-contained as a released one. A `RUSTFLAGS` environment variable set in your shell replaces that config rather than adding to it, so unset it if your own build starts asking for the redistributable.
