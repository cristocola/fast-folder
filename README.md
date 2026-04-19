# fastf — Fast Folder Creator

```
  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|
                       by Cristo Cola
```

A fast, template-driven CLI for creating structured project folders — code, research, finance, creative work, whatever you repeat. Portable single-folder distribution, like ffmpeg. Cross-platform (Linux, macOS, Windows).

---

## Quickstart

```bash
# Build
cargo build --release

# Put the binary somewhere on PATH
cp target/release/fastf ~/bin/

# Launch interactive menu (first run bootstraps config + templates)
fastf
```

On first run, `fastf` bootstraps `config.toml`, `counters.toml`, and a `templates/` folder **next to the binary**. Move the folder, everything moves with it.

---

## Use cases

fastf is a general-purpose scaffolder. A few concrete examples (all five are in [`examples/templates/`](examples/templates/) — import with `fastf template import`):

- **Code** — `rust-project.yaml` creates `src/ tests/ benches/ examples/ Cargo.toml .gitignore README.md` with crate name, author, and license prompted.
- **Web** — `web-project.yaml` creates `src/ public/ tests/ package.json` with a chosen package manager (npm/pnpm/yarn/bun).
- **Finance** — `finance-monthly.yaml` creates `{YYYY}-{MM}_<entity>_Finance/` with `INCOME/ EXPENSES/ RECEIPTS/ REPORT.md` pre-filled.
- **Research** — `research-note.yaml` creates a date-stamped `notes/ references/ data/ figures/ SUMMARY.md` folder.
- **Creative** — the three bundled templates (`music-video`, `photography`, `video-production`) ship built-in.

---

## Features

- **Template-based** — YAML folder trees, placeholder files, naming patterns.
- **Interactive builder** — create/edit templates step-by-step, no YAML required.
- **Generate template from folder** — point at an existing project, get a template YAML out: `fastf template from-folder ./my-project my-template`.
- **Auto-incrementing global ID** — every project gets a unique `ID0047` shared across all templates.
- **Variable substitution** — artist, title, client, author, etc. via prompts or CLI flags.
- **Rich dry-run** — full tree + resolved variables + file-content previews before anything hits disk.
- **Post-create actions** — `git init`, reveal in file manager, open in editor, run custom shell commands, print the absolute path for shell pipelines.
- **Open-folder prompt** — "Open project folder? [Y/n]" offered at the end of every `fastf new` (configurable on/off).
- **Structured project metadata** — every new project gets a `PROJECT_INFO.md` with YAML frontmatter recording the ID, template, creation time, path, and **every variable** (even those not in the folder name). Parseable by Obsidian, Hugo, `yq`, and any future `fastf search` command.
- **Interactive `fastf recent`** — press Enter on any project to open its folder or view its metadata. Falls back to the classic list with `--plain` or when piped.
- **Re-apply templates** — retrofit an existing folder when a template evolves. Skip-only, never overwrites.
- **Project index** — every created project is logged; `fastf recent` lists them, `fastf open <id>` jumps to one.
- **Non-interactive mode** — fully scriptable via flags + `--yes`.
- **Portable** — config, templates, counters, and project index live next to the binary.
- **Shell completions** — bash, zsh, fish, PowerShell.

---

## Installation

### Build on Linux

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
# Make sure ~/bin is on your PATH (add to ~/.bashrc or ~/.zshrc if needed):
# export PATH="$HOME/bin:$PATH"

# First run bootstraps config.toml, counters.toml, and templates/ next to the binary
fastf
```

### Build on Windows

```powershell
# 1. Install Rust (if not already installed) — use rustup, NOT the pacman/scoop package
#    Download from https://rustup.rs  or:
winget install Rustlang.Rustup
# Then open a new terminal so cargo is on PATH.

# 2. Clone and build
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder
cargo build --release
# Output: target\release\fastf.exe

# 3. Deploy — copy to any folder on your PATH
mkdir "$env:USERPROFILE\bin"
copy target\release\fastf.exe "$env:USERPROFILE\bin\"
# Add that folder to your PATH via System → Environment Variables if not already there.

# First run bootstraps config.toml, counters.toml, and templates\ next to the binary
fastf
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
# Install mingw-w64 toolchain first:
#   Arch/CachyOS:  sudo pacman -S mingw-w64-gcc
#   Ubuntu/Debian: sudo apt install gcc-mingw-w64-x86-64
#   macOS (brew):  brew install mingw-w64

rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
# Output: target/x86_64-pc-windows-gnu/release/fastf.exe
```

**macOS universal binary**:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create -output fastf \
  target/aarch64-apple-darwin/release/fastf \
  target/x86_64-apple-darwin/release/fastf
```

---

## Usage

### Interactive mode

```
fastf
```

Top-level menu:

```
> Create new project
  Recent projects
  Manage templates
  View / edit settings
  Quit
```

**Manage templates** sub-menu: create, generate from folder, edit (section-select menu — jump straight to what you want to change), apply to existing folder, list, show, delete, import.

**View / edit settings** sub-menu: project basics, workflow prompts, project metadata, recent projects, post-create actions, ID counter.

### Create a project

```bash
fastf new                                     # pick template + fill vars interactively
fastf new rust-project                        # named template, prompts for vars
fastf new rust-project --name=my-crate --author="You" --license=MIT
fastf new rust-project --dry-run              # preview only
fastf new rust-project --no-preview           # skip file-content previews
fastf new rust-project --no-post              # skip post-create actions
fastf new rust-project --yes                  # skip confirmation prompt
fastf new rust-project --base-dir=/tmp/tests  # override destination
```

After each successful `fastf new`, you'll be asked:

```
Open project folder? [Y/n]
```

Default is Yes — opens the new folder in your system file manager. Disable globally with `fastf config set prompt-open-after-create false`.

### Recent projects

```bash
fastf recent                         # interactive picker (default, on TTY)
fastf recent --plain                 # classic non-interactive list (script-friendly)
fastf recent --limit 50
fastf recent --template rust-project
fastf recent --since 2026-01-01
fastf recent --prune                 # drop records whose folders no longer exist

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

"Show project metadata" parses the YAML frontmatter in the project's `PROJECT_INFO.md` and renders it as a clean aligned key:value display — no markdown noise:

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

Piping or `--plain` bypasses the picker for scripts:

```bash
fastf recent | grep music-video        # plain auto-engages for non-TTY stdout
fastf recent --plain --prune
```

### Re-apply a template to an existing folder

```bash
fastf apply rust-project ./existing-crate --dry-run
fastf apply rust-project ./existing-crate     # creates missing items, never overwrites
```

### Manage templates

```bash
fastf template list
fastf template show <slug>
fastf template new                              # interactive builder
fastf template edit <slug>
fastf template delete <slug>
fastf template import <file.yaml>               # local file only
fastf template import examples/templates/rust-project.yaml
fastf template export <slug>                    # to stdout
fastf template export <slug> -o my-template.yaml
fastf template from-folder ./my-project my-template   # generate YAML from a real folder
fastf template from-folder ./my-project my-template --force
```

### Settings

```bash
fastf config show
fastf config set base-dir /path/to/projects
fastf config set default-template rust-project
fastf config set date-format "%Y-%m-%d"
fastf config set editor nvim

# Prompt and UX
fastf config set prompt-open-after-create false  # disable post-new open prompt
fastf config set confirm-create false            # skip "Create this project?" globally (like --yes)
fastf config set show-banner false               # hide ASCII banner in TUI

# Project metadata (PROJECT_INFO.md)
fastf config set project-info-enabled false      # don't write metadata files
fastf config set project-info-filename .info.md  # custom filename

# Recent
fastf config set recent-default-limit 50

# Post-create defaults
fastf config set post_create.git_init true
fastf config set post_create.reveal true
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

## Template schema

Templates are YAML files in `templates/` next to the binary.

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

### Transforms

| Transform | Input | Output |
|---|---|---|
| `none` | `Ariana Grande` | `Ariana Grande` |
| `title_underscore` | `ariana grande` | `Ariana_Grande` |
| `upper_underscore` | `ariana grande` | `ARIANA_GRANDE` |
| `lower_underscore` | `Ariana Grande` | `ariana_grande` |

### Name tokens

| Token | Example |
|---|---|
| `{date}` | `2026-04-17` (respects `date_format`) |
| `{YYYY}` `{MM}` `{DD}` | `2026` `04` `17` |
| `{id}` | `RS047` |
| `{anything_else}` | value of the matching variable |

> **Note:** in file **content** (like `Cargo.toml`), `__` sequences are preserved as-is — Python's `__version__`, `__init__`, dunder names all survive. In folder/file **names**, empty variables collapse into single underscores (so `{a}_{empty}_{b}` → `a_b`, not `a__b`).

### Post-create actions

Configure globally in `config.toml` or per-template via `post_create:` key. All fields default to off:

```toml
[post_create]
git_init = true
reveal = false
open_in_editor = false
print_path = false
commands = []              # list of shell commands; {path} is replaced with the project's absolute path
```

---

## Project metadata (`PROJECT_INFO.md`)

Every new project created with `fastf new` receives a `PROJECT_INFO.md` file in its root. The file uses YAML frontmatter — parseable by Obsidian, Hugo, `yq`, or any future `fastf search` command — followed by a human-readable variables table and a free-form `## Notes` section.

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

**All template variables are recorded** — including those not present in the folder name. This is intentional: if your `naming_pattern` uses only `{id}_{title}_{date}`, variables like `artist` still land in `variables:` so you can search for them later.

The `## Notes` section is yours to edit. The file is written once at creation time and never modified by fastf again.

To disable: `fastf config set project-info-enabled false`
To rename: `fastf config set project-info-filename .info.md`

---

## Portability

The whole installation is one folder:

```
fastf/
├── fastf             (fastf.exe on Windows)
├── config.toml
├── counters.toml
├── projects.jsonl    (append-only log of created projects — used by `recent` / `open`)
└── templates/
    ├── music-video.yaml
    ├── photography.yaml
    └── video-production.yaml
```

Each created project also contains `PROJECT_INFO.md` with structured YAML metadata.

The binary resolves its own location via `std::env::current_exe()` + `canonicalize()`, so symlinking the binary still finds the real folder.

---

## Testing

```bash
cargo test                                # all tests
cargo clippy --all-targets -- -D warnings # lint (must be clean)
```

Integration tests use `FASTF_INSTALL_DIR` to point at a tempdir, so they're hermetic and never touch your real install. See [`tests/integration.rs`](tests/integration.rs).

---

## Command reference

| Command | What it does |
|---|---|
| `fastf` | Launch interactive menu |
| `fastf new [slug]` | Create a project |
| `fastf recent` | Interactive project picker (Open / Show metadata / Back / Quit) |
| `fastf recent --plain` | Classic non-interactive list (script-safe) |
| `fastf open <query>` | Reveal a project folder (by ID or name substring) |
| `fastf apply <slug> <dir>` | Re-apply a template to an existing folder (skip-only) |
| `fastf template list` | List all templates |
| `fastf template show <slug>` | Print template YAML |
| `fastf template new` / `edit` | Interactive builder |
| `fastf template import <file>` | Install a YAML template |
| `fastf template export <slug>` | Export YAML |
| `fastf template from-folder <dir> <slug>` | Generate a template from an existing folder |
| `fastf template delete <slug>` | Delete a template |
| `fastf config show` / `set` | Inspect/modify global config |
| `fastf id show` / `set` / `reset` | Global ID counter |
| `fastf completions <shell>` | Print shell completions |

---

## Built with

| Crate | Purpose |
|---|---|
| `clap` | CLI commands and flags |
| `dialoguer` | Interactive prompts and menus |
| `serde` + `serde_yaml` | Template YAML parsing + YAML frontmatter in `PROJECT_INFO.md` |
| `serde` + `serde_json` | Project index (JSONL) |
| `serde` + `toml` | Config file |
| `chrono` | Date tokens + ISO-8601 timestamps |
| `anyhow` | Error handling |
| `colored` | Terminal color output |
| `clap_complete` | Shell completion generation |

---

## License

MIT
