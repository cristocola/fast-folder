# fastf — Fast Folder Creator

```
  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|
             project scaffolder
```

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584.svg)](https://www.rust-lang.org/)

A blazing-fast, Rust-native project generator for people who repeatedly set up structured work — **code, research, finance, creative, business, whatever you repeat.** Not just a folder duplicator: fastf builds nested, variable-driven project systems with dynamically-generated file contents, persistent metadata, and per-project tracking — all driven by either a friendly interactive TUI or a fully scriptable CLI.

Portable single-folder distribution (like ffmpeg), under **3 MB**, cross-platform (Linux, macOS, Windows). No runtime, no plugin ecosystem, no config directories to hunt for.

```bash
fastf                      # interactive TUI — pick template, answer prompts, done
fastf new music-video --artist="Ariana Grande" --title="Lullaby" --yes
```

---

## Table of Contents

- [Why fastf](#why-fastf)
- [The TUI](#the-tui)
- [Features](#features)
- [Examples](#examples)
- [Installation](#installation)
- [Usage](#usage)
- [Template Reference](#template-reference)
- [Project Metadata](#project-metadata)
- [Contributing](#contributing)
- [License](#license)

---

## Why fastf

Most templaters either target coders (Cookiecutter, Yeoman) or duplicate static folder structures (Post Haste). fastf sits in a different spot:

- **Not only for coders.** Works for music video production, photography, film, research archives, finance workflows, client deliverables, and yes — software projects too. Same engine, same workflow, different templates.
- **TUI *and* CLI, first-class.** Beginners get a guided interactive menu with live settings preview. Power users and scripts drive everything with flags (`--yes`, `--dry-run`, variable injection, base-dir override). AI agents, launchers, and shell pipelines plug in naturally.
- **Rust-native and portable.** Single executable under 3 MB, near-instant startup, no runtime to install. Drop the folder on a USB stick, a network share, a new laptop — the binary finds its own config, templates, and project index next to itself.
- **Generates file *contents* from metadata, not just folder *names*.** Variables flow into templated files: `Cargo.toml`, `README.md`, client briefs, slate info, shot lists, report headers. The output is materially tailored to the project.
- **Nested, variable-driven project systems.** A single template can carry deliverables, notes, exports, contracts, assets, code, references, and generated metadata — all in one coherent tree, with paths and contents driven by the variables you supply.
- **Projects are trackable objects, not one-shot output.** Every created project is logged with an ID, timestamp, template, and variables. Browse with `fastf recent`, jump to any folder with `fastf open <id>`, or parse `PROJECT_INFO.md` frontmatter with `yq`, Obsidian, Hugo, or your own tooling.
- **Author templates any way you like.** Build interactively in the TUI (no YAML required), write YAML directly, or generate a starting template from an existing real-world folder (`fastf template from-folder ./my-project`). Import, export, edit, share.
- **Path-safe, cross-platform.** Templates use `/` universally; fastf translates to `\` on Windows at runtime. Path-escape guards reject `..`, absolute paths, and drive letters at both template-load and write time.

---

## The TUI

Run `fastf` with no arguments and you land in a guided menu. Arrow keys, Enter, Esc — no YAML, no flags, no docs needed to get started.

```

  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|
             project scaffolder · v0.1.0

  project base  →  /home/cristo/Projects

? What would you like to do?
❯ Create new project
  Recent projects
  Search projects
  Register existing folder
  Manage templates
  View / edit settings
  Quit
```

The current project base directory is shown live at every loop — change it in settings and it updates on the next iteration.

**Settings are fully editable from the TUI**, grouped into five submenus so you never have to touch `config.toml` unless you want to:

```
? Settings
❯ Project basics       (base dir / template / date / editor)
  Workflow prompts     (open prompt / confirm / banner / preview)
  Project metadata     (PROJECT_INFO.md enabled / filename)
  Recent projects      (default limit)
  Post-create actions  (git / reveal / editor / path / commands)
  ID counter
  Back
```

Toggles show their current `[on]`/`[off]` state inline, so you always see what's set without leaving the menu.

**Template management is interactive too** — build from scratch with a step-by-step builder, edit an existing template by jumping straight to the section you want, or generate a starting template from an existing real-world folder:

```
? Templates
❯ Create new template
  Generate template from existing folder
  Edit a template
  Apply template to existing folder
  List templates
  Show template details
  Delete a template
  Import template from file
  Back
```

**Recent projects is a picker, not a wall of text.** Inline tags show next to each project; select one to open it, view metadata, manage tags, or write a journal note:

```
  1. ID0047  music-video       2026-04-19  Ariana_Grande_Lullaby_Indie  [draft  client_type/Indie]
  2. ID0046  research-note     2026-04-18  2026-04-18_protein_folding   [in-progress]
  3. ID0045  rust-project      2026-04-17  my_crate
❯ 4. ID0044  finance-monthly   2026-04-01  2026-04_Acme_Finance
  5. [Quit]

? What would you like to do?
❯ Open project folder
  Show project metadata
  Add tag
  Remove tag
  Add journal note
  Show journal
  Back to list
  Quit
```

**Search projects** is a separate top-level entry — type a free-text term (`ariana`) or explicit grammar (`tag:draft template=music-video`) and the same picker opens over only the matching projects.

Every interactive step has a non-interactive CLI equivalent — use the menu when you're exploring, use flags when you're scripting.

---

## Features

### Authoring
- **Interactive template builder** — create and edit templates step-by-step in the TUI. No YAML knowledge required. Edit mode jumps directly to the section you want to change.
- **Generate template from folder** — point at an existing project, get a ready-to-edit template YAML: `fastf template from-folder ./my-project my-template`.
- **Import / export / share** — YAML templates are plain text. Version them, commit them, send them to teammates.
- **Rich variable system** — `text` (free input) and `select` (pick from list) with validation, defaults, and four case transforms (`title_underscore`, `upper_underscore`, `lower_underscore`, `none`).

### Generation
- **Nested folder structures** with variable-driven paths — folders and subfolders named from any combination of variables, dates, and IDs.
- **Dynamic file contents** — templated files with full `{token}` interpolation for code, configs, READMEs, briefs, reports. Or verbatim `content:` when you want exact bytes (license text, `.gitignore`, etc.).
- **Built-in tokens** — `{date}`, `{YYYY}`, `{MM}`, `{DD}`, `{id}` plus every variable you define.
- **Auto-incrementing global ID** — every project gets a unique `ID0047` shared across all templates. Unique per install, monotonic, inspectable and editable (`fastf id set 100`).
- **Rich dry-run** — full tree + resolved variables + file-content previews (first N lines) before anything hits disk.

### Project tracking
- **Structured metadata file** — every new project gets a `PROJECT_INFO.md` with YAML frontmatter recording the ID, template, creation time, path, and **every variable** (even ones not in the folder name). Parseable by Obsidian, Hugo, `yq`, `grep`, or any future tooling.
- **Project index** — append-only `projects.jsonl` log of every created project.
- **Interactive `fastf recent`** — pick a project to open its folder, view metadata, add tags, or write a journal note. Shows inline tags. Falls back to a plain list with `--plain` or when piped.
- **Quick access** — `fastf open ID0047` or `fastf open my-crate` jumps to any project folder.
- **Re-apply to existing folders** — `fastf apply` retrofits missing files and folders when a template evolves. Skip-only, never overwrites.
- **Tags** — free-form (`draft`, `urgent`) and auto-derived from template variables (`client_type/Indie`, `artist/Ariana_Grande`). Set `tags:` and `tag_from:` in a template; manage later with `fastf tag add/remove/list/reauto`.
- **Search** — bare terms search across vars/tags/folder/template/id (`fastf search ariana`); explicit grammar adds field, date, and tag operators (`fastf search template=music-video tag:draft`). Interactive on TTY, pipe-safe with `--plain`.
- **Journal** — append timestamped notes to any project over its lifetime: `fastf note add ID0047 "finished mix"`. View with `fastf notes ID0047 --since 2026-04-01`.

### Workflow integration
- **Post-create actions** (global or per-template) — `git init`, reveal in file manager, open in editor, run custom shell commands, print the absolute path for shell pipelines.
- **Open-folder prompt** — "Open project folder? [Y/n]" offered after every `fastf new` (configurable).
- **Non-interactive mode** — `--yes`, inline variable flags, `--no-preview`, `--no-post`, `--dry-run`, `--base-dir`. Scriptable end-to-end.
- **Shell completions** for bash, zsh, fish, PowerShell.

### Deployment
- **One self-contained folder.** Binary, config, templates, counters, and project index all live together. Move the folder, everything moves with it.
- **Under 3 MB.** Single Rust binary, statically linked (musl build available). No Python, no Node, no runtime dependencies.
- **Cross-platform.** Linux, macOS (Intel + Apple Silicon), Windows. Cross-compile instructions below.

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
  Search projects
  Register existing folder
  Manage templates
  View / edit settings
  Quit
```

- **Recent projects** — interactive picker; pick a project to open / view metadata / add tag / remove tag / add journal note / show journal.
- **Search projects** — type a free-text term (`ariana`) or an explicit query (`tag:draft template=music-video`); matching projects open in the same picker.
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
fastf recent --tag draft             # only projects with this tag
fastf recent --prune                 # remove records whose folders no longer exist

fastf open ID0047                    # reveal in system file manager
fastf open my-crate                  # substring match on project name
```

**Interactive picker** — projects show inline tags; select a project, then choose an action:

```
? Projects (5 shown) — pick one
> ID0047  music-video  2026-04-19  Ariana_Grande_Lullaby  [draft  client/Indie]
  ID0046  rust-project 2026-04-18  my-crate
  ...
  [Quit]

? What would you like to do?
> Open project folder
  Show project metadata
  Add tag
  Remove tag
  Add journal note
  Show journal
  Back to list
  Quit
```

"Show project metadata" renders the structured `PROJECT_INFO.md` as a clean aligned key:value display including tags:

```
─────  Project metadata  ─────
id              ID0047
template        music-video
template_name   Music Video
created         2026-04-19T14:32:11Z
folder          2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
path            /home/cristo/Projects/MusicVideos/...

tags:
  • draft
  • client_type/Indie

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

### Tags

Free-form and auto-derived tags live in `PROJECT_INFO.md` frontmatter.

```bash
# Manual tags
fastf tag add ID0047 draft urgent
fastf tag remove ID0047 draft
fastf tag list ID0047
fastf tag reauto ID0047          # re-derive auto tags from template tag_from

# Filter recent picker
fastf recent --tag draft
```

Declare auto-derived tags in a template:

```yaml
tags: ["music-video", "creative"]   # every project from this template gets these
tag_from: ["client_type", "artist"] # lifted from variable values: client_type/Indie, artist/Ariana_Grande
```

### Search

```bash
# Default: bare term — case-insensitive substring across vars, tags,
# folder name, template slug/name, and ID (path is excluded).
fastf search ariana
fastf search ariana lullaby                      # both terms must appear somewhere

# Explicit grammar (each clause ANDs with the rest)
fastf search tag:draft
fastf search tag:client/*                        # glob on tag
fastf search template=music-video tag:draft      # AND clauses
fastf search artist=Aria* created>2026-01-01

# Mix free + explicit
fastf search ariana template=music-video

fastf search tag:draft --plain                   # pipe-friendly
```

Search opens the same interactive picker as `fastf recent` on TTY. Selecting a result enters the project action menu (open / metadata / add tag / etc.).

### Journal

```bash
fastf note add ID0047 "finished final mix"       # inline message
fastf note add ID0047 -                          # read from stdin
fastf note add ID0047                            # open $EDITOR

fastf notes ID0047                               # all entries
fastf notes ID0047 --since 2026-04-01
```

Journal entries are timestamped lines in `## Journal` in `PROJECT_INFO.md` — append-only, grows over the project's lifetime. Also accessible from the interactive `fastf recent` picker via "Add journal note" / "Show journal".

### Register an existing folder

Onboard a folder that already exists into fastf's index — useful for pre-fastf projects you want to find in `recent`, `search`, or tag/journal:

```bash
fastf register ./old-project                                     # minimal, no template
fastf register ./old-project --template music-video --artist=X --title=Y
fastf register ./old-project -t music-video --apply              # also fill missing template structure
fastf register ./old-project --rename                            # standardize folder name to {date}_{name}_{id}
fastf register ./old-project -t music-video --rename             # rename to the template's naming_pattern
fastf register ./old-project --created 2024-06-15                # historical date
fastf register ./old-project --use-today                         # ignore folder mtime, mark as now
```

`register` writes a `PROJECT_INFO.md` to the folder, appends a record to `projects.jsonl`, and bumps the global ID counter. Without `--template` you get a minimal record (template = `(registered)`); with one, you get the full metadata + tags shape, identical to `fastf new`. The `created` timestamp defaults to the folder's filesystem creation time (falling back to mtime on filesystems without birth-time, e.g. ext4) — override with `--use-today` or `--created YYYY-MM-DD`.

`--rename` standardizes the folder name. With `--template`, it renders the template's `naming_pattern`. Without one, it uses `config.register_naming_pattern` (default `"{date}_{name}_{id}"`, where `{name}` is the existing folder name with whitespace collapsed to underscores). Example: `fastf register "./random project" --rename` → `./2026-05-11_random_project_ID0001`. Configure with `fastf config set register-naming-pattern "{id}-{name}"` if you prefer a different layout. `--rename` confirms before moving on disk unless `--yes` is set.

In the TUI, "Register existing folder" is a top-level entry — it walks you through path, optional template, the rename step (default Yes, with the fs move asking once more before it happens), and optional `--apply` for filling in missing template structure.

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

# Register
fastf config set register-naming-pattern "{date}_{name}_{id}"   # default
fastf config set register-naming-pattern "{id}_{name}"          # ID first, no date

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

The file is written once on `fastf new` and modified only by `fastf tag` / `fastf note` afterward. `PROJECT_INFO.md` is a reserved filename — templates that try to declare their own file entry called `PROJECT_INFO.md` have it silently stripped on load (fastf always owns this file). To disable metadata generation entirely: `fastf config set project-info-enabled false`. The `project-info-filename` config key still exists for backwards compatibility with older configs but is no longer surfaced in the TUI.

---

## Command Reference

| Command | Description |
|---|---|
| `fastf` | Launch interactive menu |
| `fastf new [slug]` | Create a project |
| `fastf recent` | Interactive project picker (shows tags inline) |
| `fastf recent --tag <tag>` | Filter recent picker to projects with a specific tag |
| `fastf recent --plain` | Non-interactive project list (script-safe) |
| `fastf open <query>` | Reveal a project folder by ID or name |
| `fastf register <dir>` | Onboard an existing folder into the index (no folder created) |
| `fastf apply <slug> <dir>` | Apply a template to an existing folder (skip-only) |
| `fastf tag add <id> <tag>…` | Add free-form tags to a project |
| `fastf tag remove <id> <tag>…` | Remove tags from a project |
| `fastf tag list <id>` | List tags on a project |
| `fastf tag reauto <id>` | Re-derive auto tags from template `tag_from` |
| `fastf search <expr>…` | Search projects by field, date, or tag |
| `fastf note add <id> [msg]` | Add a timestamped journal note (inline / stdin / `$EDITOR`) |
| `fastf notes <id>` | Show journal entries for a project |
| `fastf notes <id> --since <date>` | Show journal entries on/after a date |
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
