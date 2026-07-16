<h1 align="center">fastf</h1>

<p align="center"><b>Fast Folder: a template-driven project scaffolder for any kind of structured work.</b></p>

<p align="center">
  <a href="https://github.com/cristocola/fast-folder/actions/workflows/ci.yml"><img src="https://github.com/cristocola/fast-folder/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/cristocola/fast-folder/releases"><img src="https://img.shields.io/github/v/release/cristocola/fast-folder" alt="Release"></a>
  <a href="https://aur.archlinux.org/packages/fast-folder"><img src="https://img.shields.io/aur/version/fast-folder" alt="AUR"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/built%20with-Rust-dea584.svg" alt="Built with Rust"></a>
</p>

fastf creates fully structured project folders from templates: nested directories, files with generated contents, unique IDs, and searchable metadata. It was built for people who set up the same kind of work over and over. That includes code, but also music video production, photography, research, finance, and client deliverables. One engine drives three interfaces, so you can work whichever way fits the moment:

```bash
fastf                       # interactive terminal menu
fastf new music-video --artist="Ariana Grande" --title="Lullaby" --yes
fastf ui                    # local browser UI, same engine, point and click
```

<p align="center">
  <img src="https://github.com/user-attachments/assets/08bb830c-1c85-42f7-b94c-85ddfa34e795" alt="Fast Folder browser UI" width="820">
</p>

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

Three templates (`music-video`, `photography`, `video-production`) are available on first run. Five more examples live in [`examples/templates/`](examples/templates/): `rust-project`, `python-project`, `web-project`, `finance-monthly`, and `research-note`. Copy any of them into your templates directory to use it.

## What it does

- **Templates generate contents, not just names.** Variables flow into file contents: `Cargo.toml`, client briefs, shot lists, report headers. Names and UTF-8 text get `{token}` interpolation, binaries are copied byte for byte.
- **A template is a folder.** `template.yaml` plus a `files/` tree that is reproduced into every project. Share one by copying its folder. Generate one from an existing project with `fastf template from-folder`.
- **Three first-class interfaces.** A guided TUI for exploring, a fully scriptable CLI (`--yes`, `--dry-run`, variable flags) for automation, and a local browser UI with a visual template editor. All three share the same config, templates, and counter.
- **The filesystem is the source of truth.** A folder is a project because it contains a `PROJECT_INFO.md` with YAML frontmatter. There is no database to drift out of sync. Delete a folder and it is simply gone.
- **Projects are trackable.** Every project gets a unique ID, timestamp, and metadata. Browse with `fastf recent`, jump anywhere with `fastf open ID0047`, filter with tags, keep timestamped journal notes.
- **Search that understands your metadata.** Free text plus a query grammar: `fastf search template=music-video tag:draft created>2026-01-01`.
- **Multiple project locations.** Index any number of base folders, including external drives that come and go. Unmounted bases are skipped quietly.
- **Safe moves.** `fastf move` relocates projects between bases. Cross-filesystem moves are copied, verified, and committed atomically before the source is removed. `fastf reconcile` recovers anything interrupted.
- **Onboard existing work.** `fastf register` makes folders fastf did not create discoverable, including bulk imports of a whole directory.
- **Post-create automation.** `git init`, reveal in file manager, open in editor, or run your own commands after each create.
- **Cross-platform and path-safe.** Linux and Windows binaries, macOS via source build. Templates use `/` everywhere and escape guards reject `..` and absolute paths.

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
