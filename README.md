# fastf — Fast Folder Creator

A template-driven CLI tool for instantly creating organized project folders with auto-incremented IDs, variable prompts, and file stubs.

## Install

```bash
cargo build --release
cp target/release/fastf /usr/local/bin/fastf   # or any directory on your PATH
```

On first run, `fastf` bootstraps itself next to the binary: it creates `config.toml`, a `templates/` directory, and writes three default templates (Music Video, Photography, Video Production).

## Usage

```bash
fastf                          # interactive TUI menu
fastf new                      # pick template interactively
fastf new music-video          # use a specific template
fastf new music-video \
  --artist="Ariana Grande" \
  --title="Lullaby" \
  --dry-run                    # preview without creating
```

### Templates

```bash
fastf template list
fastf template show <slug>
fastf template edit <slug>
fastf template import path/to/template.yaml
fastf template export <slug>
fastf template delete <slug>
```

### Config

```bash
fastf config show
fastf config set base-dir /path/to/projects
fastf config set default-template music-video
fastf config set date-format "%Y-%m-%d"
fastf config set editor nvim
```

### ID Counter

```bash
fastf id show
fastf id set 100
fastf id reset
```

### Shell completions

```bash
fastf completions bash >> ~/.bashrc
fastf completions zsh  >> ~/.zshrc
fastf completions fish > ~/.config/fish/completions/fastf.fish
```

## Template format

Templates are YAML files stored next to the binary in `templates/<slug>.yaml`.

```yaml
name: "My Template"
slug: "my-template"
description: "Optional description"
naming_pattern: "{date}_{project}_{client}_{id}"

id:
  prefix: "ID"
  digits: 4

variables:
  - slug: project
    label: "Project Name"
    type: text          # text | select
    required: true
    transform: title_underscore   # none | title_underscore | upper_underscore | lower_underscore

  - slug: client
    label: "Client Type"
    type: select
    options: ["Client", "Personal", "Spec"]
    default: "Client"

structure:
  - name: "01_Assets"
    children:
      - name: "01_Footage"
      - name: "02_Audio"
  - name: "02_Delivery"

files:
  - path: "PROJECT_INFO.md"
    template: |
      # {project}
      **Client:** {client}
      **Date:** {date}
      **ID:** {id}
```

**Built-in tokens:** `{date}`, `{YYYY}`, `{MM}`, `{DD}`, `{id}` — plus any variable slug.

## How it works

All data (config, counters, templates) lives in the same directory as the binary, making the tool fully portable. The global counter increments by 1 each time a project is created, across all templates.
