# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`fastf` — a CLI tool (and interactive TUI) for creating template-driven project folders. Templates define a folder structure, file stubs, and variable prompts; each project gets an auto-incremented ID and a name resolved from a pattern.

The actual Rust crate lives in `fastf/`. The `dist/` directory inside it is the install location — config, counters, templates, and the binary all live together there.

## Commands

All commands are run from `fastf/`.

```bash
# Build
cargo build
cargo build --release

# Run (dev)
cargo run
cargo run -- new music-video --artist="Test" --title="Test"
cargo run -- template list
cargo run -- config show

# Test
cargo test
cargo test <test_name>   # run a single test by name

# Lint
cargo clippy
cargo fmt
```

## Architecture

### Data flow for `fastf new`

1. `main.rs` parses CLI args and dispatches to `cli::new::run()`
2. `cli::new` resolves the template (by slug, config default, or interactive prompt), collects variable values (CLI flags or interactive prompts via `dialoguer`), then delegates to `core::project`
3. `core::project::plan()` applies transforms (`core::naming::apply_transform`), resolves the global counter, and interpolates the `naming_pattern` into a folder name
4. `core::project::create()` writes folders/files to disk and increments the counter in `counters.toml`

### Key modules

- **`core/template.rs`** — `Template` struct (deserialized from YAML), `Variable`, `FolderNode`, `FileEntry`. Templates are stored as `<slug>.yaml` files in the install-dir `templates/` directory.
- **`core/project.rs`** — pure planning (`plan()`) and disk-writing (`create()`). Separated so dry-run preview works without side effects.
- **`core/naming.rs`** — token interpolation (`{date}`, `{YYYY}`, `{MM}`, `{DD}`, `{id}`, plus any variable slug), transforms (`TitleUnderscore`, `UpperUnderscore`, `LowerUnderscore`), and filename sanitization. This module has unit tests.
- **`core/counter.rs`** — single global counter (`counters.toml`: `global = N`). Shared across all templates; each project increments it by 1.
- **`core/config.rs`** — `Config` struct stored as `config.toml` (TOML) next to the binary.
- **`util/paths.rs`** — **important**: all data (config, counters, templates) is stored relative to the binary's own directory (`current_exe().parent()`), not `~/.config` or XDG paths. This is intentional — the tool is designed to be portable and self-contained.
- **`bootstrap.rs`** — runs on every invocation (idempotent). Creates `config.toml`, `templates/`, and writes the three bundled default templates if `templates/` is empty.
- **`tui/menu.rs`** — interactive menu (shown when no subcommand is given). Uses `dialoguer::Select`.
- **`cli/`** — thin handlers that load config/counters, call `core`, and print results.

### Template YAML format

Templates live in `dist/templates/<slug>.yaml`. Required fields: `name`, `slug`, `naming_pattern`. Variables support `type: text` (free input) or `type: select` (from `options` list). Transforms on variables run before the value is inserted into the pattern.

### Install layout (`dist/`)

```
dist/
  fastf              ← binary
  config.toml        ← settings
  counters.toml      ← global = N
  templates/
    music-video.yaml
    photography.yaml
    video-production.yaml
```
