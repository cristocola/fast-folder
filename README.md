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

fastf is a **single-user** tool for self-contained project trees made from
ordinary files and directories. Both its surfaces share the same state on one
computer, but it does not coordinate simultaneous writers on multiple computers
or promise team-wide locking for a shared drive. It has no network surface at
all: it reads and writes files on the machine it runs on, and nothing else.

It is the same tool for very different people:

- A **video editor** gets a delivery-ready episode folder for every new video.
- A **designer** gets brief and asset folders named for the client.
- A **journalist** gets a story folder with places for interviews, footage, and drafts.
- A **project manager** gets every engagement structured and numbered the same way.
- A **developer** gets a code scaffold with configs ready to build.

One engine drives two interfaces, so you can work whichever way fits the moment:

```bash
fastf                       # guided terminal menu: pick, answer, done
fastf new general --name="spring campaign"    # -> 2026-07-16_Spring_Campaign_ID0048/
```

## The guided app

Running `fastf` with no arguments opens the guided app, which is how most work
gets done: one full-screen dashboard that shows the whole library — every base,
the newest project, the busiest templates, folder sizes filling in as they are
measured — and acts on it. Type to search (a word matches a name, ID, template or tag, a
typo forgiven; `tag:draft template=music-video created>2026-01-01` match
exactly), press a key
to open a project's folder, a terminal in it, or its action menu, and `c` for a
command palette that finds any command or project by name. Creating a project
with a live preview of the folder tree, registering folders, building templates
step by step and editing every setting are one keystroke away. Esc always goes
back, a rejected answer comes back editable rather than lost, and a network
share that has gone away is reported rather than left as a frozen screen.

Nothing it does is menu-only: every action has a scriptable `fastf <command>`
equivalent, and the rest of this README speaks fluent shell.

The whole tool is a single self-contained Rust binary (under 4 MB) with no runtime dependencies. Install it from a package manager, or carry it as a portable folder on a USB stick. `fastf paths` always tells you where its data lives.

## Quick start

```bash
# Arch Linux
paru -S fast-folder-bin

# Any Linux (or grab a release archive, see Installation below)
cargo install --git https://github.com/cristocola/fast-folder

# First project
fastf                        # pick a bundled template, answer the prompts, done
```

Two universal templates are bundled on first run. `general` is the zero-setup starting point: a dated, numbered folder (`2026-07-16_Spring_Campaign_ID0048`) with an inbox subfolder, ready for any kind of work. `client-project` adds working and delivery folders plus a brief that fills itself in with the client's name and project details.

Domain-specific templates live in the [`examples/templates/`](examples/templates/) gallery: `music-video`, `photography`, `video-production`, `rust-project`, `python-project`, `web-project`, `finance-monthly`, and `research-note`. Copy any folder into your templates directory to adopt it, then edit it to match your own convention.

## What it does

- **Fills in file contents, not just folder names.** Your answers land inside the files themselves: a client brief with the client's name already written in, a shot list titled for the artist, a report header with the right month, a code project's config ready to build. Text files get placeholders substituted, and binary files (a logo, a video asset) are copied exactly as they are.
- **A template is just a folder.** One small settings file plus a folder tree that gets reproduced into every project. Share a template by copying its folder. Or point fastf at a finished project and it generates a template from it (`fastf template from-folder`).
- **Two ways to work, one engine.** A guided app in the terminal and a scriptable command line for automation. Both read the same templates and settings, so nothing gets out of sync.
- **No hidden database.** A folder is a project because it contains a small `PROJECT_INFO.md` metadata file inside it. Delete the folder and the project is simply gone. Nothing to maintain, nothing to drift out of sync.
- **Every project is findable again.** Unique IDs, creation dates, tags, and searchable metadata. Jump to any project with `fastf open ID0047` — or just `fastf open 47` — browse recent work, or keep timestamped journal notes per project.
- **Works from your app launcher, not only a shell.** Bind fastf to a hotkey and `fastf copy lullaby` puts that project's folder on the clipboard with a notification; ask it something that needs a list or a question and it opens a terminal for itself instead of answering into the void. Pipes, redirects, cron and CI are untouched.
- **Search that understands your projects.** Plain text works (`fastf search ariana`), and so do precise filters: `fastf search template=music-video tag:draft created>2026-01-01`.
- **Projects can live on several drives.** Index any number of folders, including external drives that come and go. A disconnected drive is skipped quietly and comes back when remounted.
- **Several drives, one person's library.** Keep active and archived projects on local, external, or mounted network storage. A disconnected base is skipped and rediscovered when it returns.
- **Contained, verified moves.** `fastf move` first tries an OS rename. Across
  drives it copies every ordinary file (including `.tmp` and `.part` names),
  verifies file paths and byte lengths, publishes the destination, and only then
  removes the source. Keep the project untouched while it moves. Scoped v2
  journals let `fastf reconcile` safely discard unpublished staging or finish a
  verified cleanup; pre-v2 markers remain report-only and are never followed.
- **Adopts your existing folders.** `fastf register` onboards work that fastf did not create, one folder or a whole directory at once.
- **Optional automation after each create.** Open the new folder, launch your editor, initialize a git repository, or run your own commands.
- **Cross-platform and path-safe.** Linux and Windows binaries, macOS via source build. Templates use `/` everywhere, and unsafe paths (`..`, absolute) are rejected outright.

## Installation

### Arch Linux (AUR)

```bash
paru -S fast-folder-bin    # prebuilt static binary
paru -S fast-folder        # build from source
```

Both install the `fastf` command, shell completions, man pages, and a "Fast Folder" app-menu entry that opens the guided app in your terminal.

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

Config and templates live together in one data folder. Check yours with `fastf paths`. The ID counter is kept with your projects instead — each base carries its own `.fastf-counter.toml`, so every operating system that mounts the drive reads the same number.

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
| [docs/windows.md](docs/windows.md) | Windows install, PATH setup, data locations |

## Contributing

The [robustness roadmap](ROADMAP.md) is the canonical release plan and records
the current phase, acceptance gates, and deferred work. Update it with every
implementation PR or commit.

```bash
cargo test                                # the whole suite
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Tests are hermetic: they redirect all state through `FASTF_INSTALL_DIR` (and `HOME`) into temp directories and never touch a real install.

| Suite | Covers |
|---|---|
| [`create.rs`](tests/create.rs) · [`metadata.rs`](tests/metadata.rs) · [`search.rs`](tests/search.rs) · [`template_engine.rs`](tests/template_engine.rs) · [`register.rs`](tests/register.rs) · [`move.rs`](tests/move.rs) · [`data_dir.rs`](tests/data_dir.rs) | core flows end to end |
| [`cli_counter.rs`](tests/cli_counter.rs) · [`cli_flags.rs`](tests/cli_flags.rs) · [`cli_output.rs`](tests/cli_output.rs) | what `fastf <args>` actually does to disk, as a process |
| [`crash_recovery.rs`](tests/crash_recovery.rs) | interruption at each unsafe boundary, via fault injection |
| [`concurrency.rs`](tests/concurrency.rs) | several fastf **processes** racing each other |
| [`windows_semantics.rs`](tests/windows_semantics.rs) | reserved names, long paths, links, read-only files |
| [`hostile_fs.rs`](tests/hostile_fs.rs) | corrupt caches, markers and metadata |
| [`properties.rs`](tests/properties.rs) | generated-input properties (proptest) |
| [`tui_pty.rs`](tests/tui_pty.rs) | the interactive menu through a real terminal (unix) |
| [`repo_hygiene.rs`](tests/repo_hygiene.rs) | no tracked file describes the machine it was written on |
| [`layering.rs`](tests/layering.rs) | `core` and `util` never render, prompt, or reach for a surface |

Two things worth knowing before you change the copy or move paths:

- **Fault injection.** Boundaries that must survive a crash carry named
  failpoints. Trip one with `FASTF_FAULT=move:before-commit-rename` (returns an
  error there) or `FASTF_FAULT=create:mid-copy:abort` (kills the process there).
  See `util::faults::ALL_FAULT_POINTS`. Compiled out of release builds.
- **Work counting.** Operations that cost real I/O name themselves, so a claim
  like "the project browser no longer rescans the library" can be asserted rather
  than believed. `FASTF_TRACE_FILE=/tmp/counts fastf` appends one line per traced
  operation. Also compiled out of release builds.
- **Lint the other platform too.** `#[cfg(unix)]` code does not compile on a
  Windows machine and `#[cfg(windows)]` code does not compile on a Linux one, so
  `cargo clippy --all-targets --target x86_64-pc-windows-gnu` (or
  `--target x86_64-unknown-linux-gnu` from Windows) catches what your local
  clippy cannot. CI lints on both platforms regardless.

Pull requests are welcome. Please make sure the checks above pass first.

## License

[MIT](LICENSE) © 2026 Cristo Cola
