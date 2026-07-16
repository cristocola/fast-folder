# fastf on Windows

Windows 10 and 11 are fully supported. Every release ships two Windows artifacts on the [releases page](https://github.com/cristocola/fast-folder/releases):

| Artifact | What it is |
|---|---|
| `fastf-vX.Y.Z-x86_64.msi` | Installer. Installs `fastf.exe` and adds it to your PATH automatically. |
| `fastf-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Portable archive with `fastf.exe`, completions, and man pages. |

## Option 1: MSI installer (recommended)

1. Download the `.msi` from the releases page and run it.
2. Open a **new** terminal (PowerShell or cmd). PATH changes only apply to terminals started after the install.
3. Verify:

```powershell
fastf --version
```

The installer places `fastf.exe` under `Program Files\fastf` and adds that directory to the PATH. Upgrading is just running a newer MSI. Uninstall from Windows Settings > Apps like any other program.

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

With a normal install, config, templates, and the ID counter live in:

```
%APPDATA%\fastf
```

Run `fastf paths` at any time to see the resolved location and why it was chosen.

**Portable mode:** if you want everything in one folder (USB stick, network share), put an empty `config.toml` next to `fastf.exe` before first run. fastf then keeps all data beside the binary, and the whole folder moves as a unit.

## Paths in templates and config

Always write `/` in templates. fastf converts to `\` on Windows at runtime. Config values accept both styles:

```powershell
fastf config set base-dir C:\Users\you\Projects
fastf config set base-dir C:/Users/you/Projects   # equally fine
```

## Browser UI

`fastf ui` works the same as on Linux. With Chrome or Chromium installed, `fastf ui --app` opens a dedicated app window, and closing that window stops the server. Without one it falls back to your default browser.

## Building from source on Windows

Install Rust via [rustup](https://rustup.rs) (MSVC toolchain), then:

```powershell
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
# Output: target\release\fastf.exe
```
