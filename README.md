# fastf — Fast Folder Creator

```
  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|
                       by Cristo Cola
```

A template-driven CLI tool for creating structured project folders for creative work — music videos, photography shoots, film production, and anything else you repeat. Portable single-folder distribution, like ffmpeg. Cross-platform (Linux, macOS, Windows).

---

## Features

- **Template-based** — define folder trees, placeholder files, and naming patterns in YAML
- **Interactive builder** — create and edit templates step-by-step from within the app, no YAML knowledge needed
- **Auto-incrementing global ID** — every project gets a unique ID (`ID0047`) shared across all templates
- **Variable substitution** — fill in artist, title, client, etc. interactively or via CLI flags
- **Naming patterns** — `{date}_{artist}_{title}_{client_type}_{id}` with configurable transforms
- **Dry-run preview** — see the full folder tree before committing
- **Non-interactive mode** — fully scriptable via flags
- **Portable** — config, templates, and counters live next to the binary. Move the folder, everything moves with it
- **Shell completions** — bash, zsh, fish, PowerShell

---

## Installation

### Build from source

```bash
git clone https://github.com/cristocola/fast-folder.git
cd fast-folder && cargo build --release
```

Binary is at `target/release/fastf`. Create your portable installation folder:

```bash
mkdir -p ~/tools/fastf
cp target/release/fastf ~/tools/fastf/
```

Add to PATH (add to your `.bashrc` / `.zshrc`):

```bash
export PATH="$HOME/tools/fastf:$PATH"
```

On first run, `fastf` bootstraps `config.toml`, `counters.toml`, and a `templates/` folder next to the binary.

### Cross-compile for Windows (from Linux/macOS)

```bash
rustup target add x86_64-pc-windows-gnu
# Arch/CachyOS: sudo pacman -S mingw-w64-gcc
# Ubuntu: sudo apt install gcc-mingw-w64-x86-64
cargo build --release --target x86_64-pc-windows-gnu
# Output: target/x86_64-pc-windows-gnu/release/fastf.exe
```

---

## Usage

### Interactive mode (no arguments)

```
fastf
```

Launches a full step-by-step menu — create projects, manage templates, configure settings.

### Create a project

```bash
# Pick template interactively, fill variables step-by-step
fastf new

# Named template, fill variables interactively
fastf new music-video

# Fully non-interactive (scriptable)
fastf new music-video \
  --artist="Ariana Grande" \
  --title="Lullaby" \
  --client-type="Client"

# Preview without creating anything
fastf new music-video --dry-run

# Override base directory for this run only
fastf new music-video --base-dir=/tmp/projects
```

### Manage templates

```bash
fastf template new              # interactive step-by-step builder
fastf template edit <slug>      # edit existing template (values pre-filled)
fastf template list             # list all templates
fastf template show <slug>      # show template structure and variables
fastf template delete <slug>    # delete with confirmation
fastf template import <file>    # import a .yaml template file
fastf template export <slug>    # print template YAML to stdout
fastf template export <slug> -o my-template.yaml
```

### Settings

```bash
fastf config show
fastf config set base-dir /path/to/projects   # default project directory
fastf config set default-template music-video  # skip template selection step
fastf config set date-format "%Y-%m-%d"
fastf config set editor nvim
```

### ID counter

```bash
fastf id show          # show current global counter
fastf id set 46        # set counter (next project will be ID0047)
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

Templates are YAML files in the `templates/` folder next to the binary.

```yaml
name: "Music Video"
slug: "music-video"
description: "Music video production project folder"
version: "1"

# Naming pattern for the project root folder.
# Built-in tokens: {date} {YYYY} {MM} {DD} {id}
# Variable tokens: any {slug} defined below
naming_pattern: "{date}_{artist}_{title}_{client_type}_{id}"

id:
  prefix: "ID"
  digits: 4           # ID0047

variables:
  - slug: artist
    label: "Artist / Band Name"
    type: text          # text | select
    required: true
    transform: title_underscore  # none | title_underscore | upper_underscore | lower_underscore

  - slug: client_type
    label: "Client Type"
    type: select
    options: ["Client", "Personal", "Collab", "Spec"]
    default: "Client"

structure:
  - name: "01_Assets"
    children:
      - name: "01_Audio"
      - name: "02_Footage"
      - name: "03_Images"
  - name: "02_Export"
  - name: "03_Project_Files"

files:
  - path: "PROJECT_INFO.md"
    template: |
      # {title}
      **Artist:** {artist}
      **Date:** {date}
      **Project ID:** {id}
  - path: ".gitignore"
    content: |
      *.tmp
      .DS_Store
```

### Variable transforms

| Transform | Input | Output |
|---|---|---|
| `none` | `Ariana Grande` | `Ariana Grande` |
| `title_underscore` | `ariana grande` | `Ariana_Grande` |
| `upper_underscore` | `ariana grande` | `ARIANA_GRANDE` |
| `lower_underscore` | `Ariana Grande` | `ariana_grande` |

### Name tokens

| Token | Example output |
|---|---|
| `{date}` | `2026-04-13` (respects `date_format` config) |
| `{YYYY}` | `2026` |
| `{MM}` | `04` |
| `{DD}` | `13` |
| `{id}` | `ID0047` |
| `{slug}` | value of any defined variable |

---

## Portability

The entire installation is one folder:

```
fastf/
├── fastf           (fastf.exe on Windows)
├── config.toml
├── counters.toml
└── templates/
    ├── music-video.yaml
    ├── photography.yaml
    └── video-production.yaml
```

The binary resolves its own location at runtime (`std::env::current_exe()` + `canonicalize()`), so symlinking the binary still works — it finds the real folder.

---

## Built with

| Crate | Purpose |
|---|---|
| `clap` | CLI commands and flags |
| `dialoguer` | Interactive prompts and menus |
| `serde` + `serde_yaml` | Template YAML parsing |
| `serde` + `toml` | Config file |
| `chrono` | Date tokens |
| `anyhow` | Error handling |
| `colored` | Terminal color output |
| `clap_complete` | Shell completion generation |

---

## License

MIT
