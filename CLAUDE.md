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
    │   ├── project.rs        — ProjectPlan, plan(), print_dry_run(), create()
    │   └── template.rs       — Template, Variable, FolderNode, FileEntry, IdConfig, Transform
    ├── cli/
    │   ├── mod.rs
    │   ├── new.rs            — `fastf new` — collect vars, create project
    │   ├── template.rs       — template list/show/edit/delete/import/export
    │   ├── config.rs         — config show/set
    │   └── id.rs             — id show/reset/set
    └── tui/
        ├── mod.rs
        ├── menu.rs           — Interactive TUI menu (no-args mode), ASCII banner
        └── template_builder.rs — Step-by-step interactive template create/edit
```

## Key design decisions

### Portability
`paths::install_dir()` uses `std::env::current_exe().canonicalize().parent()` — the binary finds its own location at runtime. Config and templates always live next to the binary regardless of how it's invoked (PATH, symlink, etc.). No `~/.config/` or OS-specific paths.

### Global ID counter
One counter for all templates: `counters.toml` with a single `global` field. Every project creation increments it. `fastf id set 46` → next project gets ID0047. This is intentional — IDs are unique across all project types.

### Template YAML schema
- `naming_pattern`: tokens `{date}`, `{YYYY}`, `{MM}`, `{DD}`, `{id}`, plus any variable slug
- Variables: `type: text` (free input) or `type: select` (pick from list)
- Transforms: `none`, `title_underscore`, `upper_underscore`, `lower_underscore`
- `structure`: nested `FolderNode` list (name + children)
- `files`: `template` (with `{token}` interpolation) or `content` (raw, no substitution)

### Template builder (`tui/template_builder.rs`)
`build_template(existing: Option<Template>)` handles both create and edit:
- `None` → blank defaults
- `Some(t)` → all prompts pre-filled with existing values

Flat path strings like `01_Assets/01_Audio` are parsed into nested `FolderNode` trees via `parse_paths_to_tree()`. Edit mode shows current structure/variables/files and asks "Replace?" before collecting new ones.

## Crates

| Crate | Purpose |
|---|---|
| `clap` (derive) | CLI subcommands and flags |
| `clap_complete` | Shell completion generation (bash/zsh/fish/powershell) |
| `dialoguer` | Interactive prompts — Input, Select, Confirm, MultiSelect |
| `serde` + `serde_yaml` | Template YAML parsing/serialization |
| `serde` + `toml` | config.toml and counters.toml |
| `chrono` | Date tokens in naming patterns |
| `anyhow` | Error handling throughout |
| `colored` | Terminal color output |

## Gotchas

- `dialoguer::Input::interact_text()` takes ownership of `self`. Never reuse an `Input` struct across iterations — recreate it each time.
- `Template` needs `#[derive(Default)]` because `build_template` calls `.unwrap_or_default()`.
- The `save_to_file` and `file_path` methods on `Template` have `#[allow(dead_code)]` — they are called from the builder but Rust's lint fires because they're defined on a struct in a separate module.
- Windows cross-compile requires pacman-installed `mingw-w64-gcc`, NOT rustup-managed Rust installed via pacman. Use rustup for the Rust toolchain: `sudo pacman -Rs rust && sudo pacman -S rustup mingw-w64-gcc && rustup default stable`.
