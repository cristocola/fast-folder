<h1 align="center">fastf</h1>

<p align="center"><b>Fast Folder: a template-driven project scaffolder for any kind of structured work.</b></p>

<p align="center">
  <a href="https://github.com/cristocola/fast-folder/actions/workflows/ci.yml"><img src="https://github.com/cristocola/fast-folder/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/cristocola/fast-folder/releases"><img src="https://img.shields.io/github/v/release/cristocola/fast-folder" alt="Release"></a>
  <a href="https://aur.archlinux.org/packages/fast-folder"><img src="https://img.shields.io/aur/version/fast-folder" alt="AUR"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/built%20with-Rust-dea584.svg" alt="Built with Rust"></a>
</p>

Everyone who works in projects has a folder convention: how a new project should be named, which subfolders it needs, which starter files belong inside. In practice the convention lives in someone's head or a wiki page, and every rushed deadline erodes it a little more. fastf makes the convention executable. You describe the structure once as a template. From then on, creating a project means answering a few questions, and the result is always right: consistent name, complete folder skeleton, starter files pre-filled with your answers, a unique project ID, and metadata that lets you find the project again months later.

It is the same tool for very different people:

- A **video editor** gets a delivery-ready episode folder for every new video.
- A **designer** gets brief and asset folders named for the client.
- A **journalist** gets a story folder with places for interviews, footage, and drafts.
- A **project manager** gets every engagement structured and numbered the same way.
- A **developer** gets a code scaffold with configs ready to build.
- A **team or agency** points everyone's fastf at the same master folder on the shared drive. The convention enforces itself, new hires inherit it on day one, and the whole team searches one project history. No server, no database, no accounts. The folders are the system.

One engine drives three interfaces, so you can work whichever way fits the moment:

```bash
fastf                       # interactive terminal menu
fastf new general --name="spring campaign"    # -> 2026-07-16_Spring_Campaign_ID0048/
fastf ui                    # local browser UI, same engine, point and click
```

<p align="center">
  <img src="https://github.com/user-attachments/assets/08bb830c-1c85-42f7-b94c-85ddfa34e795" alt="Fast Folder browser UI" width="820">
</p>

## The browser UI

That screenshot is `fastf ui`: a full point-and-click interface served by the same binary. No Electron, no Node, no separate server process. The frontend is embedded in the executable and talks to the same engine the CLI uses, so both always see the same templates, settings, and projects.

- **Visual template editor.** Build variables, folder structure, and templated files without touching YAML. Drop real assets (a logo, a delivery video) into a template from a disk path.
- **Create with live preview.** Fill in variables and watch the folder tree and generated file contents update before anything is written.
- **Manage everything.** Search with the full query language, tag projects, write journal notes, and bulk-move projects between drives with verified copies.
- **Local only.** The server binds to `127.0.0.1` and closing the app window shuts it down.

```bash
fastf ui --app      # dedicated app window (Chromium/Chrome), also in your app menu as "Fast Folder"
fastf ui            # same thing in your default browser
```

Prefer the terminal? Everything the UI does has a CLI or TUI equivalent, and the rest of this README speaks fluent shell.

The whole tool is a single Rust binary under 3 MB with no runtime dependencies. Install it from a package manager, or carry it as a portable folder on a USB stick. `fastf paths` always tells you where its data lives.

## Quick start

```bash
# Arch Linux
paru -S fast-folder-bin

# Any Linux (or grab a release archive, see Installation below)
cargo install --git https://github.com/cristocola/fast-folder

# First project
fastf                        # pick a bundled template, answer the prompts, done
```

Four templates are bundled on first run. `general` is the zero-setup starting point for any kind of work: it creates a dated, numbered folder (`2026-07-16_Spring_Campaign_ID0048`) with an inbox subfolder, and you shape it into your own convention from there. The other three (`music-video`, `photography`, `video-production`) show what a deeper domain template looks like. Five more examples live in [`examples/templates/`](examples/templates/): `rust-project`, `python-project`, `web-project`, `finance-monthly`, and `research-note`. Copy any of them into your templates directory to use it.

## What it does

- **Fills in file contents, not just folder names.** Your answers land inside the files themselves: a client brief with the client's name already written in, a shot list titled for the artist, a report header with the right month, a code project's config ready to build. Text files get placeholders substituted, and binary files (a logo, a video asset) are copied exactly as they are.
- **A template is just a folder.** One small settings file plus a folder tree that gets reproduced into every project. Share a template by copying its folder. Or point fastf at a finished project and it generates a template from it (`fastf template from-folder`).
- **Three ways to work, one engine.** A guided menu in the terminal, a browser UI with a visual template editor, and a scriptable command line for automation. All three read the same templates and settings, so nothing gets out of sync.
- **No hidden database.** A folder is a project because it contains a small `PROJECT_INFO.md` metadata file inside it. Delete the folder and the project is simply gone. Nothing to maintain, nothing to drift out of sync.
- **Every project is findable again.** Unique IDs, creation dates, tags, and searchable metadata. Jump to any project with `fastf open ID0047`, browse recent work, or keep timestamped journal notes per project.
- **Search that understands your projects.** Plain text works (`fastf search ariana`), and so do precise filters: `fastf search template=music-video tag:draft created>2026-01-01`.
- **Projects can live on several drives.** Index any number of folders, including external drives that come and go. A disconnected drive is skipped quietly and comes back when remounted.
- **Teams share one project system.** Point every install at the same master folder on a NAS or shared drive. Everyone creates projects through the same convention and searches the same history. Because the metadata travels inside the folders, there is no server and nothing to administer.
- **Moves that cannot lose data.** `fastf move` relocates a project to another drive by copying, verifying, and only then removing the original. If anything is interrupted mid-move, `fastf reconcile` finishes or rolls it back.
- **Adopts your existing folders.** `fastf register` onboards work that fastf did not create, one folder or a whole directory at once.
- **Optional automation after each create.** Open the new folder, launch your editor, initialize a git repository, or run your own commands.
- **Cross-platform and path-safe.** Linux and Windows binaries, macOS via source build. Templates use `/` everywhere, and unsafe paths (`..`, absolute) are rejected outright.

## Installation

### Arch Linux (AUR)

```bash
paru -S fast-folder-bin    # prebuilt static binary
paru -S fast-folder        # build from source
```

Both install the `fastf` command, shell completions, man pages, and a "Fast Folder" app-menu entry for the browser UI.

### Linux (release archive)

Download from the [releases page](https://github.com/cristocola/fast-folder/releases). The `musl` build is fully static and runs on any distro; checksums are in `SHA256SUMS`.

```bash
tar xzf fastf-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz
install -Dm755 fastf-vX.Y.Z-x86_64-unknown-linux-musl/fastf ~/.local/bin/fastf
fastf --version
```

### Windows

Download the `.msi` installer from the [releases page](https://github.com/cristocola/fast-folder/releases) and run it. It installs `fastf.exe` and adds it to your PATH. A portable `.zip` is also available. Full instructions, including manual PATH setup: [docs/windows.md](docs/windows.md).

### Build from source

Works on Linux, macOS, and Windows. Install Rust via [rustup](https://rustup.rs), then:

```bash
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
install -Dm755 target/release/fastf ~/.local/bin/fastf   # or copy fastf.exe onto your PATH
```

No prebuilt macOS binaries are published because they cannot be tested here, but the source build is the same three commands.

## Where fastf keeps its data

Config, templates, and the ID counter live together in one data folder. Check yours with `fastf paths`.

| Priority | Location | When |
|---|---|---|
| 1 | `$FASTF_INSTALL_DIR` | The env var is set (scripting, testing) |
| 2 | Portable: the binary's own directory | A `config.toml` or `templates/` sits next to the binary |
| 3 | User dir: `~/.config/fastf` or `%APPDATA%\fastf` | Everything else, including package installs |

Portable mode keeps the classic single-folder layout. To opt in, put an empty `config.toml` next to the binary before first run, then move the folder anywhere and everything moves with it. Projects themselves live wherever you create them, and each base directory carries its own portable index cache.

## Documentation

| Guide | Contents |
|---|---|
| [docs/cli.md](docs/cli.md) | Full command reference and usage recipes: create, search, tags, journal, register, move, config |
| [docs/templates.md](docs/templates.md) | Template authoring: `template.yaml`, variables, transforms, tokens, bundled assets |
| [docs/projects.md](docs/projects.md) | The project model: `PROJECT_INFO.md`, discovery, bases, safe moves, crash recovery |
| [docs/UI.md](docs/UI.md) | Browser UI architecture, HTTP API, frontend development |
| [docs/windows.md](docs/windows.md) | Windows install, PATH setup, data locations |

A note on the browser UI: the server binds to loopback (`127.0.0.1`) only and has no authentication. Keep it that way. With `fastf ui --app`, closing the app window stops the server.

## Contributing

```bash
cargo test                                # unit + integration + UI server tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
node --check src/ui/web/app.js            # frontend sanity check
```

Tests are hermetic: they redirect all state through `FASTF_INSTALL_DIR` into temp directories and never touch a real install. Core flows live in [`tests/integration.rs`](tests/integration.rs), the browser UI request layer in [`tests/ui_server.rs`](tests/ui_server.rs). For frontend work, `FASTF_UI_DIR=src/ui/web fastf ui` serves assets from disk so you can edit and refresh without rebuilding.

Pull requests are welcome. Please make sure the three checks above pass first.

## License

[MIT](LICENSE) © 2026 Cristo Cola
