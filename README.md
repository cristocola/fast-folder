# fastf — Fast Folder Creator

```
  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|
                       by Cristo Cola
```

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584.svg)](https://www.rust-lang.org/)

A fast, template-driven CLI for creating structured project folders — code, research, finance, creative work, whatever you repeat. Portable single-folder distribution, like ffmpeg. Cross-platform (Linux, macOS, Windows).

---

## Table of Contents

- [Features](#features)
- [Examples](#examples)
- [Installation](#installation)
- [Usage](#usage)
- [Template Reference](#template-reference)
- [Project Metadata](#project-metadata)
- [Contributing](#contributing)
- [License](#license)

---

## Features

- **Template-based** — YAML folder trees, placeholder files, naming patterns.
- **Interactive builder** — create/edit templates step-by-step, no YAML required. Edit mode jumps directly to the section you want to change.
- **Generate template from folder** — point at an existing project, get a template YAML out: `fastf template from-folder ./my-project my-template`.
- **Auto-incrementing global ID** — every project gets a unique `ID0047` shared across all templates.
- **Variable substitution** — artist, title, client, author, etc. via prompts or CLI flags.
- **Rich dry-run** — full tree + resolved variables + file-content previews before anything hits disk.
- **Post-create actions** — `git init`, reveal in file manager, open in editor, run custom shell commands, print the absolute path for shell pipelines.
- **Open-folder prompt** — "Open project folder? [Y/n]" offered at the end of every `fastf new` (configurable on/off).
- **Structured project metadata** — every new project gets a `PROJECT_INFO.md` with YAML frontmatter recording the ID, template, creation time, path, and **every variable** (even those not in the folder name). Parseable by Obsidian, Hugo, `yq`, and any future `fastf search` command.
- **Interactive `fastf recent`** — pick any project to open its folder or view its metadata. Falls back to a plain list with `--plain` or when piped.
- **Re-apply templates** — retrofit an existing folder when a template evolves. Skip-only, never overwrites.
- **Project index** — every created project is logged; `fastf recent` lists them, `fastf open <id>` jumps to one.
- **Non-interactive mode** — fully scriptable via flags + `--yes`.
- **Portable** — config, templates, counters, and project index live next to the binary. Move the folder, everything moves with it.
- **Shell completions** — bash, zsh, fish, PowerShell.

---

## Examples

fastf is a general-purpose scaffolder. A few concrete examples (all five are in [`examples/templates/`](examples/templates/) — import with `fastf template import`):

| Template | What it creates |
|---|---|
| `rust-project.yaml` | `src/ tests/ benches/ examples/ Cargo.toml .gitignore README.md` — prompts for crate name, author, license |
| `web-project.yaml` | `src/ public/ tests/ package.json` — prompts for package manager (npm/pnpm/yarn/bun) |
| `finance-monthly.yaml` | `{YYYY}-{MM}_<entity>_Finance/` with `INCOME/ EXPENSES/ RECEIPTS/ REPORT.md` pre-filled |
| `research-note.yaml` | Date-stamped `notes/ references/ data/ figures/ SUMMARY.md` |
| `music-video` *(built-in)* | Full music video production folder structure |

The three bundled templates (`music-video`, `photography`, `video-production`) are available on first run with no import needed.

---

## Installation

### On Linux

```bash
# 1. Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. Clone and build
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
# Output: target/release/fastf

# 3. Deploy — copy to any folder on your PATH
mkdir -p ~/bin
cp target/release/fastf ~/bin/
# If ~/bin is not yet on your PATH, add this to ~/.bashrc or ~/.zshrc:
# export PATH="$HOME/bin:$PATH"
```

### On Windows

```powershell
# 1. Install Rust — use rustup from https://rustup.rs (or via winget)
winget install Rustlang.Rustup
# Open a new terminal so cargo is on PATH.

# 2. Clone and build
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
# Output: target\release\fastf.exe

# 3. Deploy — copy to any folder on your PATH
mkdir "$env:USERPROFILE\bin"
copy target\release\fastf.exe "$env:USERPROFILE\bin\"
# Add %USERPROFILE%\bin to your PATH via System → Environment Variables.
```

### On macOS

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. Clone and build
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
# Output: target/release/fastf

# 3. Deploy
cp target/release/fastf /usr/local/bin/
```

**macOS universal binary** (Apple Silicon + Intel):

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create -output fastf \
  target/aarch64-apple-darwin/release/fastf \
  target/x86_64-apple-darwin/release/fastf
```

### Cross-compile

**Linux binary from Windows** (static musl, no glibc coupling):

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
# Output: target/x86_64-unknown-linux-musl/release/fastf
```

**Windows `.exe` from Linux/macOS**:

```bash
# Install mingw-w64 first:
#   Arch/CachyOS:  sudo pacman -S mingw-w64-gcc
#   Ubuntu/Debian: sudo apt install gcc-mingw-w64-x86-64
#   macOS (brew):  brew install mingw-w64

rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
# Output: target/x86_64-pc-windows-gnu/release/fastf.exe
```

### Portable install layout

The whole installation is one self-contained folder — copy it anywhere, everything moves with it:

```
fastf/
├── fastf             (fastf.exe on Windows)
├── config.toml
├── counters.toml
├── projects.jsonl
└── templates/
    ├── music-video.yaml
    ├── photography.yaml
    └── video-production.yaml
```

On first run, `fastf` creates `config.toml`, `counters.toml`, and `templates/` alongside itself. The binary resolves its own location at runtime, so symlinking also works.

---

## Usage

### Interactive mode

```bash
fastf
```

```
> Create new project
  Recent projects
  Manage templates
  View / edit settings
  Quit
```

- **Manage templates** — create, generate from folder, edit, apply to existing folder, list, show, delete, import.
- **View / edit settings** — project basics, workflow prompts, project metadata, recent projects, post-create actions, ID counter.

### Create a project

```bash
fastf new                                     # pick template + fill vars interactively
fastf new rust-project                        # named template, prompts for vars
fastf new rust-project --name=my-crate --author="You" --license=MIT
fastf new rust-project --dry-run              # preview tree + variables, nothing written
fastf new rust-project --no-preview           # skip file-content previews in dry-run
fastf new rust-project --no-post              # skip post-create actions
fastf new rust-project --yes                  # skip confirmation prompt
fastf new rust-project --base-dir=/tmp/tests  # override destination
```

After each successful `fastf new`, you are asked:

```
Open project folder? [Y/n]
```

Default is Yes — opens the new folder in your system file manager. Disable with `fastf config set prompt-open-after-create false`.

### Recent projects

```bash
fastf recent                         # interactive picker (default, on TTY)
fastf recent --plain                 # classic non-interactive list (script-friendly)
fastf recent --limit 50
fastf recent --template rust-project
fastf recent --since 2026-01-01
fastf recent --prune                 # remove records whose folders no longer exist

fastf open ID0047                    # reveal in system file manager
fastf open my-crate                  # substring match on project name
```

**Interactive picker** — select a project, then choose an action:

```
? What would you like to do?
> Open project folder
  Show project metadata
  Back to list
  Quit
```

"Show project metadata" renders the structured `PROJECT_INFO.md` as a clean aligned key:value display:

```
─────  Project metadata  ─────
id              ID0047
template        music-video
template_name   Music Video
created         2026-04-19T14:32:11Z
folder          2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
path            /home/cristo/Projects/MusicVideos/...

variables:
  artist        Ariana_Grande
  client_type   Indie
  title         Lullaby
──────────────────────────────
```

`--plain` or piping engages the non-interactive list automatically:

```bash
fastf recent | grep music-video
fastf recent --plain --prune
```

### Apply a template to an existing folder

```bash
fastf apply rust-project ./existing-crate --dry-run
fastf apply rust-project ./existing-crate     # creates missing items, never overwrites
```

### Manage templates

```bash
fastf template list
fastf template show <slug>
fastf template new                              # interactive builder
fastf template edit <slug>                      # jump directly to the section you want
fastf template delete <slug>
fastf template import <file.yaml>
fastf template import examples/templates/rust-project.yaml
fastf template export <slug>                    # to stdout
fastf template export <slug> -o my-template.yaml
fastf template from-folder ./my-project my-template   # generate YAML from an existing folder
fastf template from-folder ./my-project my-template --force
```

### Settings

```bash
fastf config show
fastf config set base-dir /path/to/projects
fastf config set default-template rust-project
fastf config set date-format "%Y-%m-%d"
fastf config set editor nvim                     # used by post_create.open_in_editor

# Prompts and UX
fastf config set prompt-open-after-create false  # disable the post-new open prompt
fastf config set confirm-create false            # skip "Create this project?" (like --yes)
fastf config set show-banner false               # hide ASCII banner in TUI

# Project metadata
fastf config set project-info-enabled false      # don't write PROJECT_INFO.md
fastf config set project-info-filename .info.md  # custom filename

# Recent
fastf config set recent-default-limit 50

# Post-create defaults
fastf config set post_create.git_init true
fastf config set post_create.reveal true
fastf config set post_create.open_in_editor true
fastf config set post_create.print_path true
```

### ID counter

```bash
fastf id show          # current global counter
fastf id set 46        # next project will be ID0047
fastf id reset         # reset to 0
```

### Shell completions

```bash
fastf completions bash >> ~/.bashrc
fastf completions zsh  >> ~/.zshrc
fastf completions fish >> ~/.config/fish/completions/fastf.fish
```

---

## Template Reference

Templates are YAML files stored in `templates/` next to the binary.

```yaml
name: "Rust Project"
slug: "rust-project"
description: "Cargo-style Rust project scaffold"
version: "1"

# Built-in tokens: {date} {YYYY} {MM} {DD} {id}
# Variable tokens: any {slug} defined below
naming_pattern: "{name}"

id:
  prefix: "RS"
  digits: 3           # RS047

variables:
  - slug: name
    label: "Crate name"
    type: text            # text | select
    required: true
    transform: lower_underscore   # none | title_underscore | upper_underscore | lower_underscore

  - slug: license
    label: "License"
    type: select
    options: ["MIT", "Apache-2.0", "GPL-3.0"]
    default: "MIT"

structure:
  - name: "src"
  - name: "tests"
  - name: "examples"

files:
  - path: "Cargo.toml"
    template: |          # interpolated — {name}, {id}, {date}, etc. are substituted
      [package]
      name = "{name}"
      license = "{license}"
  - path: ".gitignore"
    content: |           # verbatim — no interpolation
      target/

# Optional per-template override of the global post_create config.
post_create:
  git_init: true
  reveal: false
```

### Variable transforms

| Transform | Input | Output |
|---|---|---|
| `none` | `Ariana Grande` | `Ariana Grande` |
| `title_underscore` | `ariana grande` | `Ariana_Grande` |
| `upper_underscore` | `ariana grande` | `ARIANA_GRANDE` |
| `lower_underscore` | `Ariana Grande` | `ariana_grande` |

### Naming pattern tokens

| Token | Example |
|---|---|
| `{date}` | `2026-04-17` (respects `date_format` setting) |
| `{YYYY}` `{MM}` `{DD}` | `2026` `04` `17` |
| `{id}` | `RS047` |
| `{anything_else}` | value of the matching variable |

> **Note:** in file **content**, `__` sequences are preserved as-is (Python's `__init__`, `__version__`, etc. survive). In folder and file **names**, empty variables collapse to avoid double underscores (`{a}_{empty}_{b}` → `a_b`).

### Post-create actions

Configure globally in `config.toml` or override per-template with a `post_create:` block. All fields default to off:

```toml
[post_create]
git_init = true
reveal = false
open_in_editor = false   # opens config.editor (or $EDITOR) with the project folder
print_path = false       # prints absolute path — useful for shell pipelines: $(fastf new ...)
commands = []            # shell commands; {path} is replaced with the project's absolute path
```

---

## Project Metadata

Every project created with `fastf new` receives a `PROJECT_INFO.md` in its root. The file has two layers:

1. **YAML frontmatter** — machine-readable, parseable by Obsidian, Hugo, `yq`, `grep`. Contains the ID, template, timestamp, path, and every template variable regardless of whether it appears in the folder name.
2. **Markdown body** — a human-readable variables table and a `## Notes` section you can edit freely.

```markdown
---
id: ID0047
template: music-video
template_name: Music Video
created: 2026-04-19T14:32:11Z
folder: 2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
path: /home/cristo/Projects/MusicVideos/2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
variables:
  artist: Ariana_Grande
  client_type: Indie
  title: Lullaby
---

# Project Info

| Variable           | Value         |
|--------------------|---------------|
| Artist / Band Name | Ariana_Grande |
| Project Title      | Lullaby       |
| Client Type        | Indie         |

## Notes
```

The file is written once on `fastf new` and never modified again. To disable: `fastf config set project-info-enabled false`. To rename: `fastf config set project-info-filename .info.md`.

---

## Command Reference

| Command | Description |
|---|---|
| `fastf` | Launch interactive menu |
| `fastf new [slug]` | Create a project |
| `fastf recent` | Interactive project picker |
| `fastf recent --plain` | Non-interactive project list (script-safe) |
| `fastf open <query>` | Reveal a project folder by ID or name |
| `fastf apply <slug> <dir>` | Apply a template to an existing folder (skip-only) |
| `fastf template list` | List all templates |
| `fastf template show <slug>` | Print template YAML |
| `fastf template new` | Create a template interactively |
| `fastf template edit <slug>` | Edit a template interactively |
| `fastf template import <file>` | Install a YAML template |
| `fastf template export <slug>` | Export template YAML |
| `fastf template from-folder <dir> <slug>` | Generate a template from an existing folder |
| `fastf template delete <slug>` | Delete a template |
| `fastf config show` | Print current configuration |
| `fastf config set <key> <value>` | Set a configuration value |
| `fastf id show` / `set` / `reset` | Manage the global ID counter |
| `fastf completions <shell>` | Print shell completions |

---

## Contributing

```bash
# Run all tests
cargo test

# Lint — must pass with no warnings
cargo clippy --all-targets -- -D warnings

# Format check
cargo fmt --check
```

Integration tests use `FASTF_INSTALL_DIR` to point at a temporary directory, so they are hermetic and never touch a real install. See [`tests/integration.rs`](tests/integration.rs).

Pull requests are welcome. Please ensure `cargo test`, `cargo clippy`, and `cargo fmt --check` all pass before submitting.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `clap` | CLI commands and flags |
| `dialoguer` | Interactive prompts and menus |
| `serde` + `serde_yaml` | Template YAML parsing + YAML frontmatter |
| `serde` + `serde_json` | Project index (JSONL) |
| `serde` + `toml` | Config file |
| `chrono` | Date tokens + ISO-8601 timestamps |
| `anyhow` | Error handling |
| `colored` | Terminal color output |
| `clap_complete` | Shell completion generation |

---

## License

[MIT](LICENSE) © 2026 Cristo Cola
