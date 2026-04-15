# CLAUDE.md — fastf development context

## What this project is

`fastf` (Fast Folder Creator) is a portable Rust CLI tool for creating structured project folders from YAML templates. Primary use case: music video, photography, and film production workflows. Single-folder portable distribution — config, templates, and counters live next to the binary.

## Build commands

```bash
# Debug build (fast compile, unoptimized)
cargo build

# Release build (optimized + stripped)
cargo build --release
# Output: target/release/fastf

# Cross-compile for Windows (from Linux)
cargo build --release --target x86_64-pc-windows-gnu
# Output: target/x86_64-pc-windows-gnu/release/fastf.exe
# Requires: rustup target add x86_64-pc-windows-gnu + mingw-w64-gcc (pacman)

# Run directly
cargo run
cargo run -- new music-video --dry-run

# Test
cargo test
cargo test <test_name>   # run a single test by name

# Lint
cargo clippy
cargo fmt
```

## Project layout

```
fast-folder/
├── Cargo.toml
├── README.md
├── CLAUDE.md
├── .gitignore
└── src/
    ├── main.rs               — CLI entry, clap commands, parse_extra_vars
    ├── bootstrap.rs          — First-run setup: creates config.toml, counters.toml, templates/
    ├── util/
    │   ├── mod.rs
    │   └── paths.rs          — install_dir() via current_exe().canonicalize().parent()
    ├── core/
    │   ├── mod.rs
    │   ├── config.rs         — Config struct, base_dir, editor, date_format, default_template
    │   ├── counter.rs        — Global auto-increment ID (single 'global' field in counters.toml)
    │   ├── naming.rs         — apply_transform(), interpolate() for {token} substitution
    │   ├── project.rs        — ProjectPlan, plan(), print_dry_run(), print_success(), create(), print_tree()
    │   └── template.rs       — Template, Variable, FolderNode, FileEntry, IdConfig, Transform
    ├── cli/
    │   ├── mod.rs
    │   ├── new.rs            — `fastf new` — collect vars, warn on unknown flags, create project
    │   ├── template.rs       — template list/show/edit/delete/import/export
    │   ├── config.rs         — config show/set (date_format validated at set time)
    │   └── id.rs             — id show/reset/set
    └── tui/
        ├── mod.rs
        ├── menu.rs           — Interactive TUI menu (no-args mode), ASCII banner, live base dir display
        └── template_builder.rs — Step-by-step interactive template create/edit
```

## Key design decisions

### Portability
`paths::install_dir()` uses `std::env::current_exe().canonicalize().parent()` — the binary finds its own location at runtime. Config and templates always live next to the binary regardless of how it's invoked (PATH, symlink, etc.). No `~/.config/` or OS-specific paths.

### Cross-platform paths
Folder paths in templates (structure names, file paths) always use `/` as the separator in YAML — Rust's `PathBuf::join()` handles conversion to `\` on Windows at runtime. Users should always enter `/` in templates and `base-dir` config, though Windows also accepts backslashes in config values.

### Global ID counter
One counter for all templates: `counters.toml` with a single `global` field. Every project creation increments it. `fastf id set 46` → next project gets ID0047. This is intentional — IDs are unique across all project types.

### Template YAML schema
- `naming_pattern`: tokens `{date}`, `{YYYY}`, `{MM}`, `{DD}`, `{id}`, plus any variable slug
- Variables: `type: text` (free input) or `type: select` (pick from list)
- Transforms: `none`, `title_underscore`, `upper_underscore`, `lower_underscore`
- `structure`: nested `FolderNode` list (name + children). Names support forward slashes when entered via the builder — parsed via `parse_paths_to_tree()`.
- `files`: `template` (with `{token}` interpolation) or `content` (raw, no substitution). `path` supports subfolders using `/` — parent dirs are created automatically.

### Template builder (`tui/template_builder.rs`)
`build_template(existing: Option<Template>)` handles both create and edit:
- `None` → blank defaults
- `Some(t)` → all prompts pre-filled with existing values

Flat path strings like `01_Assets/01_Audio` are parsed into nested `FolderNode` trees via `parse_paths_to_tree()`. Edit mode shows current structure/variables/files and asks "Replace?" before collecting new ones.

### Output display (`core/project.rs`)
`print_tree(nodes, indent)` is the single shared tree renderer — used by dry-run, `template show`, and the template builder summary. Call it with `"  "` indent for breathing room in dry-run, `""` for compact display in `template show`.

`print_project_path(path, folder_name)` renders a full path with the parent directory dimmed and the project/folder name bold white, prefixed by a cyan `→`. Used in both dry-run and success output. In success output, `canonicalize()` is called first since the folder exists.

## CLI help quality
All subcommands have thorough `about` strings and `after_help` examples. Key places:
- `fastf new --help` — shows variable flag syntax, `=` requirement, examples
- `fastf config set --help` — lists all valid keys with descriptions and path format notes for both Linux/macOS and Windows
- `fastf --help` — `long_about` with tool overview and getting-started commands

## TUI main menu (`tui/menu.rs`)
Below the ASCII banner, the current project base directory is shown on every loop iteration (reloads config each time so it reflects settings changes immediately):
```
  project base  →  /home/user/  Projects
```
Parent path is dimmed, final directory name is bold cyan.

## Crates

| Crate | Purpose |
|---|---|
| `clap` (derive) | CLI subcommands and flags |
| `clap_complete` | Shell completion generation (bash/zsh/fish/powershell) |
| `dialoguer` | Interactive prompts — Input, Select, Confirm, MultiSelect |
| `serde` + `serde_yaml` | Template YAML parsing/serialization |
| `serde` + `toml` | config.toml and counters.toml |
| `chrono` | Date tokens in naming patterns; also used to validate `date_format` at config set time |
| `anyhow` | Error handling throughout |
| `colored` | Terminal color output |

## Gotchas

- `dialoguer::Input::interact_text()` takes ownership of `self`. Never reuse an `Input` struct across iterations — recreate it each time.
- `Template` needs `#[derive(Default)]` because `build_template` calls `.unwrap_or_default()`.
- The `save_to_file` and `file_path` methods on `Template` have `#[allow(dead_code)]` — they are called from the builder but Rust's lint fires because they're defined on a struct in a separate module.
- Windows cross-compile requires pacman-installed `mingw-w64-gcc`, NOT rustup-managed Rust installed via pacman. Use rustup for the Rust toolchain: `sudo pacman -Rs rust && sudo pacman -S rustup mingw-w64-gcc && rustup default stable`.
- `IdConfig` no longer has an `auto_increment` field — it was defined but never read. If per-template ID disable is needed in the future, add it back and check it in `project::plan()`.
- `print_tree` is in `core/project.rs` (pub). Do not add a duplicate in `cli/template.rs` or `tui/template_builder.rs` — import it from `project`.
