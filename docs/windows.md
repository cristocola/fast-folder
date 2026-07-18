# fastf on Windows

Windows 10 and 11 are fully supported. Every release ships two Windows artifacts on the [releases page](https://github.com/cristocola/fast-folder/releases):

| Artifact | What it is |
|---|---|
| `fastf-vX.Y.Z-x86_64.msi` | Installer with a setup wizard. Installs the CLI, adds it to your PATH, and creates a Start Menu app for the browser UI. |
| `fastf-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Portable archive with `fastf.exe`, the `fastf-ui.exe` launcher, PowerShell completions, and the docs. |

## Option 1: MSI installer (recommended)

1. Download the `.msi` from the releases page and run it. A standard setup wizard walks you through the license and the install folder.
2. Launch **Fast Folder** from the Start Menu. It opens the browser UI in its own app window, with no console attached. Closing the window stops fastf. On the first launch it asks where your projects should live, suggesting `C:\Users\<you>\Projects`, and creates that folder for you (the terminal TUI asks the same on its first run).
3. For the CLI, open a **new** terminal (PowerShell or cmd). PATH changes only apply to terminals started after the install. Verify:

```powershell
fastf --version
```

The installer places two programs under `Program Files\fastf` and adds that directory to the PATH:

- `fastf.exe` is the CLI and TUI.
- `fastf-ui.exe` is what the Start Menu shortcut runs. It is the same program in app-window mode, equivalent to `fastf ui --app`.

Upgrading is just running a newer MSI. Uninstall from Windows Settings > Apps like any other program. Uninstalling removes the shortcut and the PATH entry.

## Option 2: portable zip

1. Download and extract the `.zip`. Inside is a folder containing `fastf.exe` and `fastf-ui.exe`.
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

`fastf ui` works the same as on Linux. `fastf ui --app` opens a dedicated app window using Chrome if you have it, or Microsoft Edge otherwise. Edge ships with Windows, so the app window works on every stock machine. Closing the window stops the server. If neither browser can be found it falls back to a tab in your default browser.

The Start Menu shortcut runs `fastf-ui.exe`, which is exactly this app mode without a console window.

## Building from source on Windows

Install Rust via [rustup](https://rustup.rs) (MSVC toolchain), then:

```powershell
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
# Output: target\release\fastf.exe
```
