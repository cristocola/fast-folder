# CLAUDE.md — fastf development context

## What this project is

`fastf` (Fast Folder Creator) is a portable Rust CLI tool for creating structured project folders from YAML templates. Universal use cases: code, research, finance, music video, photography, and film production workflows. Single-folder portable distribution — config, templates, counters, and project index live next to the binary.

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

# Cross-compile for Linux (from Windows or macOS) — static musl
cargo build --release --target x86_64-unknown-linux-musl

# Run directly
cargo run
cargo run -- new music-video --dry-run

# Test (31 total: 11 unit + 20 integration)
cargo test
cargo test <test_name>   # run a single test by name

# Lint — must be clean with -D warnings
cargo clippy --all-targets -- -D warnings
cargo fmt
```

## Project layout

```
fast-folder/
├── Cargo.toml
├── README.md
├── CLAUDE.md
├── .gitignore
├── examples/
│   └── templates/            — Gallery YAMLs (rust-project, python-project, web-project,
│                               finance-monthly, research-note). NOT bundled — users import
│                               with `fastf template import examples/templates/<slug>.yaml`.
├── tests/
│   └── integration.rs        — 20 hermetic tests using FASTF_INSTALL_DIR + tempfile
└── src/
    ├── lib.rs                — Library entry: exposes core/, cli/, tui/, util/, bootstrap/
    │                           so integration tests can import fastf::...
    ├── main.rs               — Binary entry, `use fastf::{bootstrap, cli, tui};`
    │                           clap commands include Recent (+ --plain), Open, Apply,
    │                           TemplateAction::FromFolder
    ├── bootstrap.rs          — First-run setup: creates config.toml, counters.toml, templates/
    │                           (the three bundled YAMLs no longer declare PROJECT_INFO.md —
    │                           auto-gen owns it now)
    ├── util/
    │   ├── mod.rs
    │   └── paths.rs          — install_dir(): FASTF_INSTALL_DIR override, else current_exe().
    │                           projects_index_path() → install_dir()/projects.jsonl
    ├── core/
    │   ├── mod.rs
    │   ├── config.rs         — Config: base_dir, editor, date_format, default_template,
    │   │                        preview_lines (8), post_create (PostCreate), and new v0.3 fields:
    │   │                        prompt_open_after_create, project_info_enabled,
    │   │                        project_info_filename, recent_default_limit,
    │   │                        confirm_create, show_banner.
    │   │                        Serde aliases `pinfo_enabled`/`pinfo_filename` accept
    │   │                        any interim configs from before the rename.
    │   ├── counter.rs        — Global auto-increment ID (single 'global' field in counters.toml)
    │   ├── naming.rs         — apply_transform(), interpolate() [raw for file CONTENT],
    │   │                        interpolate_name() [collapses __ and trims for NAMES],
    │   │                        sanitize_name(), ensure_relative_safe_path()
    │   ├── project.rs        — ProjectPlan, plan(), create(run_post), print_dry_run(),
    │   │                        print_resolved_values(), print_file_previews(), print_tree(),
    │   │                        apply_plan(), apply(), print_apply_plan(), ApplyAction enum.
    │   │                        resolve_post_create() is pub so cli/new.rs can check for
    │   │                        double-open before offering the open prompt.
    │   ├── project_info.rs   — NEW v0.3. Metadata struct, render(), write(), read(),
    │   │                        read_metadata(), extract_frontmatter().
    │   │                        Generates PROJECT_INFO.md with YAML frontmatter + variables
    │   │                        table + Notes section. Called from project::create().
    │   ├── template.rs       — Template (+ post_create: Option<PostCreate>), Variable,
    │   │                        FolderNode, FileEntry, IdConfig, Transform. validate() is pub.
    │   ├── vars.rs           — collect_vars() shared by `new` and `apply`
    │   ├── index.rs          — ProjectRecord + append()/try_append()/load_all()/rewrite()
    │   │                        for projects.jsonl (JSONL append-only log)
    │   └── post_create.rs    — PostCreate struct + run(): git_init, reveal, open_in_editor,
    │                            print_path, commands. Platform-specific reveal_folder()
    │                            via cfg(windows)/cfg(target_os="macos")/cfg(unix).
    │                            reveal_folder() and prompt_and_reveal() are pub.
    ├── cli/
    │   ├── mod.rs
    │   ├── new.rs            — `fastf new` with --no-preview, --no-post, --yes flags.
    │   │                        After print_success(): calls prompt_and_reveal() if:
    │   │                        not --yes, not --no-post, cfg.prompt_open_after_create,
    │   │                        stdout is TTY, and reveal not already in resolved post_create.
    │   │                        Also honors cfg.confirm_create (global --yes equivalent).
    │   ├── template.rs       — list/show/edit/delete/import/export +
    │   │                        from_folder() for template generation from existing dirs
    │   ├── config.rs         — config show/set. Handles new v0.3 keys:
    │   │                        project_info_enabled, project_info_filename (with pinfo_* aliases),
    │   │                        prompt_open_after_create, confirm_create, show_banner,
    │   │                        recent_default_limit, post_create.* keys.
    │   ├── id.rs             — id show/reset/set
    │   ├── recent.rs         — `fastf recent`: defaults to interactive picker (TTY).
    │   │                        picker → project_action_menu() → Open / Show metadata / Back / Quit.
    │   │                        "Show metadata" calls project_info::read_metadata() → structured
    │   │                        aligned key:value display; falls back to raw markdown if no frontmatter.
    │   │                        --plain flag (or non-TTY stdout) gives classic list output.
    │   └── apply.rs          — `fastf apply <slug> <dir>` with --dry-run (skip-only semantics)
    └── tui/
        ├── mod.rs
        ├── menu.rs           — Interactive TUI menu. ASCII banner (suppressed if !show_banner).
        │                        Live base dir display. Top-level menu adds:
        │                          "Apply template to existing folder" → menu_apply().
        │                        menu_recent() simplified: just calls recent::run().
        │                        menu_settings() restructured into 5 grouped submenus:
        │                          Project basics / Workflow prompts / Project metadata /
        │                          Recent projects / Post-create actions.
        │                        Every config field has a toggle/edit entry with inline state.
        └── template_builder.rs — Step-by-step interactive template create/edit
                                  (sets post_create: None on new templates)
```

## Key design decisions

### Portability
`paths::install_dir()` checks `FASTF_INSTALL_DIR` first (test-only escape hatch), then falls back to `std::env::current_exe().canonicalize().parent()` — the binary finds its own location at runtime. Config, templates, counters, and `projects.jsonl` always live next to the binary. No `~/.config/` or OS-specific paths.

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
- `post_create` (optional): per-template override of the global `config.post_create`.

### Interpolation: `interpolate` vs `interpolate_name` (important)
Two separate functions in `core/naming.rs`:
- **`interpolate()`** — raw substitution only. Used for **file content** (templated files). Preserves `__` sequences so Python's `__version__`, `__init__`, etc. survive intact.
- **`interpolate_name()`** — calls `interpolate`, then collapses consecutive `__` → `_` and trims leading/trailing `_`. Used for **folder and file names** so empty optional variables don't leave dangling underscore gaps.

When adding new code: if you're building a *path component name*, call `interpolate_name`. If you're building *file contents*, call `interpolate`. Do not mix them.

### Path-escape safety
`ensure_relative_safe_path()` rejects absolute paths, Windows drive letters, leading separators, and any `..` segment. Enforced in two places:
1. `Template::validate()` at template-load time (so broken templates fail at `fastf template list`).
2. `create_file()` and `apply()` at disk-write time (defence in depth).

### Project index (`projects.jsonl`)
Append-only JSONL log of created projects. One `{"id","template","path","name","created_at"}` record per line. Chosen over TOML for atomic appends (no read-modify-write) and crash safety. `fastf recent --prune` rewrites via tmp-file + rename to drop records whose folders no longer exist. Writes are best-effort — index failures never fail `fastf new`.

### `PROJECT_INFO.md` — structured per-project metadata
`core/project_info.rs` generates a `PROJECT_INFO.md` in each new project root. The file has two layers:

1. **YAML frontmatter** (between `---` lines) — the machine-readable layer. Typed struct `Metadata` serialized via `serde_yaml`. Contains: `id`, `template` (slug), `template_name`, `created` (ISO-8601), `folder`, `path`, and `variables: BTreeMap<String, String>` with **every** template variable regardless of whether it appears in `naming_pattern`. `BTreeMap` keeps keys alphabetical for diff-stability.

2. **Human body** — markdown table of variables (using template labels as column headers) + a `## Notes` section the user owns. Written once; never modified by fastf.

`read_metadata(path, cfg)` slices out the frontmatter via `extract_frontmatter()`, feeds it to `serde_yaml::from_str::<Metadata>`. Returns `Ok(None)` when no frontmatter block is present (older / hand-edited files). `read(path, cfg)` returns raw markdown for fallback display.

The bundled templates (`music-video`, `photography`, `video-production`) no longer declare a `PROJECT_INFO.md` content file — auto-gen owns that file. Custom templates that previously defined their own `PROJECT_INFO.md` will have it overwritten on `fastf new` (write order: template files → pinfo). If you want to preserve a custom body, name it something different (e.g. `NOTES.md`) and let fastf own `PROJECT_INFO.md`.

**`apply` does NOT write PROJECT_INFO.md** — by design. Only `fastf new` writes it. It's not a new project.

### "Open project folder?" prompt
After `print_success()` in `cli/new.rs`, call `post_create::prompt_and_reveal(path)` when all of these are true:
- `cfg.prompt_open_after_create` is true (default)
- stdout is a TTY
- `args.yes` is false
- `args.no_post` is false
- the resolved `post_create` block does NOT already have `reveal: true` (avoids double-open)

Calls the existing platform-correct `reveal_folder()` on Yes.

### Interactive `fastf recent`
`cli/recent.rs` decides interactive vs plain by `!args.plain && std::io::stdout().is_terminal()`. In interactive mode: `dialoguer::Select` picker over the filtered records + a `[Quit]` sentinel at the end. Selecting a record enters `project_action_menu()` which loops until Back/Quit.

The metadata display (`show_metadata`) tries `read_metadata` first; on success it calls `print_structured_metadata` which computes max-key-width and emits aligned `key  value` pairs with a `variables:` sub-block. Dim `(empty)` for empty values. Falls back to raw markdown on `Ok(None)`. Yellow warning on missing file.

Scripting compat: `--plain` flag or non-TTY stdout → classic column-aligned list. `fastf open <query>` still exists as a one-shot alternative.

### Post-create actions
`PostCreate` struct on both `Config` and `Template`. Template-level overrides config-level entirely (same resolution model as `default_template`). All fields default to off:
- `git_init`: run `git init` in new folder
- `reveal`: open folder in system file manager (Windows: `cmd /c start`, macOS: `open`, Linux: `xdg-open`)
- `open_in_editor`: spawn `config.editor` with the folder path
- `print_path`: print absolute path on stdout (for `$(fastf new ...)` shell pipelines)
- `commands`: list of shell commands; `{path}` token replaced with project's absolute path

### Output display (`core/project.rs`)
`print_tree(nodes, indent)` is the single shared tree renderer — used by dry-run, `template show`, and the template builder summary. Call it with `"  "` indent for breathing room in dry-run, `""` for compact display in `template show`.

`print_project_path(path, folder_name)` renders a full path with the parent directory dimmed and the project/folder name bold white, prefixed by a cyan `→`. Used in both dry-run and success output. In success output, `canonicalize()` is called first since the folder exists.

`print_resolved_values()` + `print_file_previews()` — the rich dry-run additions. Show variable values, transforms applied, ID/counter delta, all built-in date tokens, and the first `config.preview_lines` (default 8) of every templated file.

## CLI help quality
All subcommands have thorough `about` strings and `after_help` examples. Key places:
- `fastf new --help` — shows variable flag syntax, `=` requirement, examples
- `fastf config set --help` — lists all valid keys with descriptions and path format notes for both Linux/macOS and Windows
- `fastf --help` — `long_about` with tool overview and getting-started commands

## TUI main menu (`tui/menu.rs`)
Below the ASCII banner (hidden when `cfg.show_banner` is false), the current project base directory is shown on every loop iteration (reloads config each time so it reflects settings changes immediately):
```
  project base  →  /home/user/  Projects
```
Parent path is dimmed, final directory name is bold cyan.

### Top-level menu entries
```
> Create new project
  Recent projects                          ← interactive picker
  Apply template to existing folder        ← menu_apply() → prompts slug + target dir
  Manage templates
  View / edit settings
  View ID counters
  Quit
```

### Settings menu structure (grouped submenus)
```
Settings
├── Project basics               (base dir / template / date / editor)
├── Workflow prompts             (open prompt / confirm / banner / preview lines)
├── Project metadata             (enabled / filename)
├── Recent projects              (default limit)
├── Post-create actions          (git / reveal / editor / path / commands)
└── Back
```
Each toggle entry shows current `[on]`/`[off]` state inline via `label_toggle()`. `toggle_setting(key, current)` calls `config::set` under the hood.

## Testing

Integration tests live in `tests/integration.rs`. They use:
- `FASTF_INSTALL_DIR` env var to redirect `paths::install_dir()` to a tempdir per test
- `tempfile::TempDir` for hermetic sandboxes
- A `static SERIAL: Mutex<()>` to run tests serially within the test binary (Rust 2024 edition made `std::env::set_var` unsafe — the mutex justifies the `unsafe` block)

Tests cover: basic round-trip, transforms, counter persistence, duplicate-project rejection, dry-run no-write, apply skip-logic, index append, from-folder round-trip, path-escape rejection (parent, absolute, drive letter), Windows forward-slash paths, gallery-YAML parsing, PROJECT_INFO.md frontmatter, variable capture (including non-naming-pattern vars), metadata round-trip via YAML, disabled/custom-filename metadata, pinfo alias config compat, and bundled-template deduplication guard.

Run:
```bash
cargo test                                # all tests
cargo test <test_name>                    # single test
cargo clippy --all-targets -- -D warnings # lint must be clean
```

## Crates

| Crate | Purpose |
|---|---|
| `clap` (derive) | CLI subcommands and flags |
| `clap_complete` | Shell completion generation (bash/zsh/fish/powershell) |
| `dialoguer` | Interactive prompts — Input, Select, Confirm, MultiSelect |
| `serde` + `serde_yaml` | Template YAML parsing/serialization; YAML frontmatter in PROJECT_INFO.md |
| `serde` + `serde_json` | Project index JSONL |
| `serde` + `toml` | config.toml and counters.toml |
| `chrono` | Date tokens; validates `date_format` at config-set time; ISO-8601 timestamps |
| `anyhow` | Error handling throughout |
| `colored` | Terminal color output |
| `tempfile` (dev-dep only) | Integration test sandboxes |

`console` crate removed in v0.2 — was unused.

## Gotchas

- `dialoguer::Input::interact_text()` takes ownership of `self`. Never reuse an `Input` struct across iterations — recreate it each time.
- `Template` needs `#[derive(Default)]` because `build_template` calls `.unwrap_or_default()`.
- `Template::validate()` is `pub` (was private before v0.2). Used by the gallery-parse integration test.
- `Template::save_to_file()` no longer has `#[allow(dead_code)]` — it's reached by both the interactive builder and `from_folder`.
- Windows cross-compile requires pacman-installed `mingw-w64-gcc`, NOT rustup-managed Rust installed via pacman. Use rustup for the Rust toolchain: `sudo pacman -Rs rust && sudo pacman -S rustup mingw-w64-gcc && rustup default stable`.
- `IdConfig` no longer has an `auto_increment` field — it was defined but never read. If per-template ID disable is needed in the future, add it back and check it in `project::plan()`.
- `print_tree` is in `core/project.rs` (pub). Do not add a duplicate in `cli/template.rs` or `tui/template_builder.rs` — import it from `project`.
- **Naming pattern** in `project::plan()` uses `interpolate_name()` (collapses `__`, trims edges). **File content** in `create_file()`, `apply()`, and `print_file_previews()` uses `interpolate()` (raw, no collapse). Mixing them up will either break Python dunders in generated files OR leave dangling underscores in folder names.
- Rust 2024 edition makes `std::env::set_var`/`remove_var` unsafe. In tests they are wrapped in `unsafe { }` with the `SERIAL` mutex held.
- Clippy lint `field_reassign_with_default` is allowed at the test-file level (`#![allow(clippy::field_reassign_with_default)]`) — rewriting every test's `Config::default()` builder into struct-literal form adds churn for no benefit in tests.
- `projects.jsonl` append is best-effort. `index::append()` swallows errors; `try_append()` is for the test that actually asserts on write success.
- Post-create `commands` run synchronously through the user's shell (`cmd /c` on Windows, `sh -c` elsewhere). `{path}` is substituted before execution. There's no sandbox — template authors control this.
- `project_info::render()` builds frontmatter via `serde_yaml::to_string(&Metadata { ... })` — do NOT hand-format the YAML string. serde_yaml handles escaping of colons, quotes, multi-line values correctly; hand-formatting breaks on edge cases.
- Config fields `pinfo_enabled` / `pinfo_filename` were renamed to `project_info_enabled` / `project_info_filename` in v0.3. The fields carry `#[serde(alias = "pinfo_*")]` and `config::set` accepts both name forms, so interim configs / old scripts keep working. On `config save()` they serialize under the new names.
- `resolve_post_create()` in `project.rs` is `pub` — the open-prompt check in `cli/new.rs` calls it to avoid double-opening when `reveal: true` is already set in post_create.
