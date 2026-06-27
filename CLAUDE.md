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

# Test (119 total: 58 unit + 61 integration — 47 core in integration.rs + 14 UI in ui_server.rs)
cargo test
cargo test <test_name>   # run a single test by name

# Lint — must be clean with -D warnings
cargo clippy --all-targets -- -D warnings
cargo fmt

# Browser UI — same `cargo build` (the server + embedded frontend live in the lib).
cargo run -- ui --no-open            # serve only (loopback)
cargo run -- ui --app                # serve + open a Chromium/Chrome app window
FASTF_UI_DIR=src/ui/web cargo run -- ui   # frontend live-reload (serve assets from disk)
node --check src/ui/web/app.js       # frontend sanity check
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
├── docs/
│   └── UI.md                 — Browser-UI reference (architecture, HTTP API, dev live-reload)
├── Launch Fast Folder UI.desktop — desktop launcher (Exec=`fastf ui --app`)
├── tests/
│   ├── integration.rs        — 47 hermetic core tests using FASTF_INSTALL_DIR + tempfile
│   └── ui_server.rs          — 14 tests driving fastf::ui::route_request (v0.6 core
│                               + v0.7 search/detail/tag/note/register/apply/import-export/prune)
└── src/
    ├── lib.rs                — Library entry: exposes core/, cli/, tui/, ui/, util/, bootstrap/
    │                           so integration tests can import fastf::...
    ├── main.rs               — Binary entry, `use fastf::{bootstrap, cli, tui, ui};`
    │                           clap commands include Recent (+ --plain --tag), Open, Register,
    │                           Apply, Tag (Add/Remove/List/Reauto), Search, Note (Add), Notes,
    │                           TemplateAction::FromFolder, Ui (v0.6 browser-UI launcher →
    │                           cli::ui::run). The New / Apply / Register arms run
    │                           their clap `extra` Vec through `cli::new::classify_extra` so
    │                           bool flags (--yes/--dry-run/--no-preview/--no-post/-y) and
    │                           `--base-dir=PATH` work BEFORE or AFTER the slug. Unknown
    │                           `--foo` tokens surface via `warn_unknown()` instead of
    │                           silently dropping (v0.5).
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
    │   │                        confirm_create, show_banner. v0.5: register_naming_pattern
    │   │                        (default "{date}_{name}_{id}") drives the no-template rename.
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
    │   ├── project_info.rs   — Metadata struct (incl. tags), render(), write(), read(),
    │   │                        read_metadata(). v0.4: write_frontmatter(path, mutator) for
    │   │                        atomic in-place tag mutation; append_journal_entry(path, msg)
    │   │                        for ## Journal section; read_journal_entries(); split_frontmatter_body()
    │   │                        is pub for byte-identical body round-trips.
    │   │                        v0.5: pub const RESERVED_FILENAME = "PROJECT_INFO.md" +
    │   │                        path_is_reserved(p) helper used by Template::strip_reserved_files
    │   │                        and the TUI template builder to lock the auto-gen filename.
    │   ├── template.rs       — Template (+ post_create, tags, tag_from), Variable,
    │   │                        FolderNode, FileEntry, IdConfig, Transform. validate() is pub
    │   │                        and rejects tag_from entries that aren't declared variable slugs.
    │   │                        v0.5: strip_reserved_files() drops file entries whose path
    │   │                        collides with the reserved auto-gen filename
    │   │                        (PROJECT_INFO.md at root, case-insensitive). Called from
    │   │                        load_from_file (silent back-compat for older templates) and
    │   │                        save_to_file (cleans up on re-save).
    │   ├── query.rs          — v0.4. Predicate enum (Field/After/Before/Tag/Free), Pattern
    │   │                        (Exact/Prefix), parse() and evaluate(). Bare terms become
    │   │                        Predicate::Free — case-insensitive substring across vars/tags/
    │   │                        folder/template/template_name/id (path EXCLUDED).
    │   ├── vars.rs           — collect_vars() shared by `new` and `apply`
    │   ├── index.rs          — ProjectRecord + append()/try_append()/load_all()/rewrite()
    │   │                        for projects.jsonl (JSONL append-only log) +
    │   │                        resolve_project(query) — exact-id → prefix → name substring.
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
    │   │                        v0.5: hosts `classify_extra(extra) -> ClassifiedExtra` —
    │   │                        the trailing_var_arg splitter shared by main.rs's New /
    │   │                        Apply / Register arms.
    │   ├── template.rs       — list/show/edit/delete/import/export +
    │   │                        from_folder() for template generation from existing dirs
    │   ├── config.rs         — config show/set. Handles new v0.3 keys:
    │   │                        project_info_enabled, project_info_filename (with pinfo_* aliases),
    │   │                        prompt_open_after_create, confirm_create, show_banner,
    │   │                        recent_default_limit, post_create.* keys, and
    │   │                        v0.5 register_naming_pattern. The setter rejects patterns
    │   │                        without `{id}` (would collide multiple registered folders).
    │   ├── id.rs             — id show/reset/set
    │   ├── recent.rs         — `fastf recent`: defaults to interactive picker (TTY).
    │   │                        picker → project_action_menu() → Open / Show metadata /
    │   │                        Add tag / Remove tag / Add journal note / Show journal /
    │   │                        Back / Quit. Inline tag display in picker labels (truncated
    │   │                        to 3 + "+N" overflow). --tag <name> filter. run_picker() pub
    │   │                        so cli/search.rs can reuse it. --plain (or non-TTY) gives
    │   │                        classic list output.
    │   ├── tag.rs            — v0.4. add/remove/list/reauto. add is idempotent; remove no-ops
    │   │                        on missing tags; reauto preserves free-form, replaces derived.
    │   ├── note.rs           — v0.4. note add: inline / `-` (stdin) / omit ($EDITOR via cfg).
    │   │                        notes: prints filtered ## Journal entries (--since YYYY-MM-DD).
    │   ├── search.rs         — v0.4. `fastf search <terms...>`. Parses via core::query, walks
    │   │                        index reverse-chronologically, reads metadata per record,
    │   │                        evaluates predicates, then renders via run_picker (TTY) or
    │   │                        plain list (--plain / pipe).
    │   ├── apply.rs          — `fastf apply <slug> <dir>` with --dry-run (skip-only semantics)
    │   ├── ui.rs             — v0.6. `fastf ui` launcher: health-check → open browser
    │   │                        (--app = Chromium/Chrome app window, else default) → serve.
    │   │                        Calls fastf::ui::serve(); UiArgs { address, no_open, app }.
    │   └── register.rs       — v0.5. `fastf register <path>` onboards an existing folder:
    │                            writes PROJECT_INFO.md, appends to projects.jsonl, bumps the
    │                            global counter. Optional --template (full metadata + tags),
    │                            --apply (fill missing structure via project::apply, requires
    │                            --template), --rename (renders tmpl.naming_pattern with a
    │                            template or cfg.register_naming_pattern without — `{name}`
    │                            token is synthesised via slugify_folder_name()),
    │                            --use-today / --created YYYY-MM-DD.
    │                            Exposes pub fn resolve_created() for unit-test isolation and
    │                            pub const REGISTERED_SLUG = "(registered)" for no-template runs.
    ├── tui/
        ├── mod.rs
        ├── menu.rs           — Interactive TUI menu. ASCII banner (suppressed if !show_banner).
        │                        Live base dir display. Top-level menu:
        │                          Create / Recent / Search projects (v0.4) / Manage templates
        │                          / Settings / Quit.
        │                        menu_search() prompts for a query string, splits on
        │                        whitespace, calls cli::search::run.
        │                        menu_settings() restructured into 5 grouped submenus:
        │                          Project basics / Workflow prompts / Project metadata /
        │                          Recent projects / Post-create actions.
        │                        Every config field has a toggle/edit entry with inline state.
        └── template_builder.rs — Step-by-step interactive template create/edit
                                  (sets post_create: None on new templates).
                                  v0.5: `collect_file(vars)` always stores user
                                  input in `FileEntry.template` and prints the
                                  available `{token}` strings + a post-input
                                  substitution summary. The "Template vs Raw"
                                  Select was removed — interpolate() is a no-op
                                  on text without braces so there's no behavior
                                  loss vs Raw, and `{slug}` markers just work.
    └── ui/                   — v0.6 browser UI (full notes in "Browser UI" section below)
        ├── mod.rs            — loopback HTTP server (std::net, thread-per-conn, no framework)
        │                        + all API handlers. pub serve() blocks; pub route_request()
        │                        is the pure router (tested in tests/ui_server.rs); pub
        │                        health_check(). Write routes take a private WRITE_LOCK.
        ├── assets.rs         — the 4 frontend files embedded via include_str!; if FASTF_UI_DIR
        │                        is set, served from disk instead (frontend live-reload).
        └── web/              — index.html, app.js, styles.css, icon.svg (vanilla JS, no deps)
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

2. **Human body** — markdown table of variables (using template labels as column headers) + a `## Notes` section the user owns. The body also gains a `## Journal` section the first time `append_journal_entry` is called. Outside of those mutation helpers, fastf never modifies the file after creation.

`read_metadata(path, cfg)` slices out the frontmatter via `split_frontmatter_body()`, feeds it to `serde_yaml::from_str::<Metadata>`. Returns `Ok(None)` when no frontmatter block is present (older / hand-edited files). `read(path, cfg)` returns raw markdown for fallback display.

**Atomic mutation** (v0.4): `write_frontmatter(path, |meta| { ... })` reads → splits → parses → applies the closure → re-serializes via `serde_yaml::to_string` → writes via `.tmp` + rename. Body bytes are byte-identical after a no-op mutation — the dedicated integration test asserts this. `append_journal_entry(path, msg)` does the same atomic dance for the body. Both require frontmatter to exist; otherwise return a structured error naming the path.

The bundled templates (`music-video`, `photography`, `video-production`) no longer declare a `PROJECT_INFO.md` content file — auto-gen owns that file. **As of v0.5, `PROJECT_INFO.md` at the project root is a reserved filename**: `Template::load_from_file` and `save_to_file` silently strip any `files[].path == "PROJECT_INFO.md"` entry (case-insensitive on the leaf, root-only — `docs/PROJECT_INFO.md` is allowed). Older user-built templates that declared their own `PROJECT_INFO.md` keep loading; the entry is just ignored. The TUI template builder rejects the name inline. If you want a custom notes file, use a different name (e.g. `NOTES.md`).

The reservation is enforced via `core::project_info::path_is_reserved()` against the hard-coded constant `RESERVED_FILENAME = "PROJECT_INFO.md"`, NOT against `cfg.project_info_filename`. The config field still exists and still drives where fastf writes the auto-gen file, but the reservation is fixed so the safety net is consistent regardless of config. (Power users who customized `project_info_filename` to e.g. `.fastf-info.md` need to manage their template collisions themselves.)

**`apply` does NOT write PROJECT_INFO.md** — by design. Only `fastf new` and `fastf register` write it. `apply` retrofits structure into a folder that fastf doesn't necessarily own; `register` explicitly claims a folder. Different intents, different write behavior.

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

### Tags, search, journal (v0.4)

**Tags** live in `Metadata.tags: Vec<String>` (frontmatter, `#[serde(default)]` for back-compat). Two flavours coexist:
- **Free-form** — arbitrary strings (`draft`, `urgent`).
- **Auto-derived** — generated at creation from `Template.tag_from`. Slug `client_type` with value `Indie` becomes `client_type/Indie`. Empty values are skipped (no orphan `slug/` tags). Computed in `project::create()` before `project_info::write()`. Literal `Template.tags` are added too.

`Template::validate()` rejects `tag_from` entries that aren't declared variable slugs.

`fastf tag reauto <id>` is the safety valve: it loads the current frontmatter, looks up `tag_from` from the live template, removes any tag whose prefix matches `slug/` for slugs in `tag_from`, and re-adds the freshly-derived ones. Free-form tags survive untouched.

**Search** lives in `core/query.rs`. Predicates AND together; no OR or parens. Operators: bare term (free-text substring fallthrough), `key=value` (exact, ci), `key=prefix*` (prefix glob), `key>date`, `key<date`, `tag:value`, `tag:prefix*`. Fields resolve from `Metadata` first (id/template/template_name/created/folder/path), then `meta.variables.<slug>`. Unknown keys return `false`, never error — keeps it forward-compatible.

The free-text branch in `eval_one` searches `tags`, all `meta.variables.values()`, `folder`, `template`, `template_name`, and `id` — case-insensitive substring. **`path` is intentionally excluded** so home-dir text never produces phantom matches. There's a regression test that proves this.

`cli/search::run` walks the index reverse-chronologically, reads each metadata file (silently skipping records with missing/unreadable PROJECT_INFO.md), and renders matches via `recent::run_picker` on TTY or a plain list when piped/`--plain`.

**Journal** entries are markdown lines under a `## Journal` section in the body. Format: `- 2026-04-20T14:32:11Z — message`. Append-only and chronological — `append_journal_entry` always appends at EOF after the section, never edits existing entries. `parse_journal_entries` walks the section and stops at the next `## ` heading. `notes --since YYYY-MM-DD` filters by lexicographic timestamp comparison (cheap and correct because ISO-8601 is sortable as string).

Project-resolution shared helper: `index::resolve_project(query)` does exact-id → id-prefix → name-substring (case-insensitive). Used by `tag`, `note`, and the legacy `fastf open` paths. Ambiguous queries return a structured error listing the candidates.

### `fastf register <path>` (v0.5)
Onboards an existing folder into the index without creating one. Same write order as `project::create`: counter → index → pinfo. Two-step pinfo write is the key trick:

1. `project_info::write(&plan, &tmpl, &cfg, &tags)` renders the full file (frontmatter + variables table + Notes), but `Metadata::from_plan` always sets `created = now_iso8601()`.
2. `project_info::write_frontmatter(path, |m| m.created = resolved.clone())` atomically patches only the `created` YAML field via `.tmp` + rename. Body bytes are byte-identical.

This avoids growing `Metadata::from_plan`'s signature for a register-only concern. The `resolved` timestamp comes from `resolve_created(path, use_today, override)`: explicit `--created YYYY-MM-DD` → `T00:00:00Z` ISO-8601; `--use-today` → now; default → `fs::metadata.created()` falling back to `modified()` (some Linux fs have no birth time).

Without `--template`, a stub `Template` is used: `slug = "(registered)"` (exposed as `REGISTERED_SLUG` const), `IdConfig::default()` for the ID format, empty variables/structure/files/tags/tag_from. The rest of the render path handles empty variables gracefully (the "no variables" branch in `project_info::render`). This avoids special-casing every call site.

`--rename` calls `interpolate_name(pattern, vars, date_format)` — same renderer as `project::plan`. Two pattern sources: `tmpl.naming_pattern` with a template, `cfg.register_naming_pattern` without (default `{date}_{name}_{id}`). For the no-template path a synthetic `{name}` token is injected via `slugify_folder_name(folder_basename)` — collapses whitespace runs to `_`, applies `sanitize_name`, preserves case. Confirms before `fs::rename` unless `--yes` is set. Aborts if the target already exists rather than overwriting. `--apply` calls `project::apply` after the optional rename. `--apply` still requires `--template` (there's no structure to fill in without one), but `--rename` works either way as of v0.5.

PROJECT_INFO.md overwrite policy: if a file already exists, prompt (default No). With `--yes`, overwrite without asking. In non-TTY without `--yes`, refuse and warn (still register the project — index/counter happen regardless).

Path equality for "already registered" uses `paths_equal` (both sides normalised to `/`) so Windows backslash variations don't slip past the duplicate check.

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
  Recent projects                          ← menu_recent() → straight to interactive picker
  Search projects                          ← menu_search() → splits query, runs cli::search
  Register existing folder                 ← menu_register() (v0.5)
  Manage templates                         ← contains "Apply template to existing folder"
  View / edit settings
  Quit
```

`menu_recent()` delegates directly to `recent::run` with prune=false — keeps
the picker one keypress away from the main menu. Maintenance lives under
**Settings → Recent projects → Prune missing entries**, which calls
`menu_recent_prune()`: loads the index, lists up to 10 stale records (with
`+N more` overflow), Confirms (default Yes), then delegates to `recent::run`
with prune=true. The CLI `fastf recent --prune` is the same code path.

`menu_register()` walks: folder path → optional template (Confirm + picker) →
"Standardize folder name?" (default Yes) → optional `--apply` (only when a
template is attached) → calls `cli::register::run`. The fs::rename inside
`register::run` prompts again before moving, so the user has a second chance
to back out after seeing the proposed new name.

### Settings menu structure (grouped submenus)
```
Settings
├── Project basics               (base dir / template / date / editor)
├── Workflow prompts             (open prompt / confirm / banner / preview lines)
├── Project metadata             (PROJECT_INFO.md enabled — filename is reserved)
├── Recent projects              (default limit / prune missing entries)
├── Post-create actions          (git / reveal / editor / path / commands)
└── Back
```
Each toggle entry shows current `[on]`/`[off]` state inline via `label_toggle()`. `toggle_setting(key, current)` calls `config::set` under the hood.

## Testing

Integration tests live in `tests/integration.rs` (core flows) and `tests/ui_server.rs`
(browser-UI request layer). Both use:
- `FASTF_INSTALL_DIR` env var to redirect `paths::install_dir()` to a tempdir per test
- `tempfile::TempDir` for hermetic sandboxes
- A `static SERIAL: Mutex<()>` to run tests serially within the test binary (Rust 2024 edition made `std::env::set_var` unsafe — the mutex justifies the `unsafe` block). Each test binary has its own `SERIAL`; that's fine because `FASTF_INSTALL_DIR` is per-process and `cargo test`'s binaries are separate processes.

`integration.rs` covers: basic round-trip, transforms, counter persistence, duplicate-project rejection, dry-run no-write, apply skip-logic, index append, from-folder round-trip, path-escape rejection (parent, absolute, drive letter), Windows forward-slash paths, gallery-YAML parsing, PROJECT_INFO.md frontmatter, variable capture (including non-naming-pattern vars), metadata round-trip via YAML, disabled/custom-filename metadata, pinfo alias config compat, and bundled-template deduplication guard.

`ui_server.rs` drives `fastf::ui::route_request` directly (no socket). v0.6 core: health route, preview produces a plan without writing, create makes the folder + appends the index, an embedded static asset (`/app.js`) is served, and an unknown route 404s. v0.7 adds: search respects the query language + path exclusion, project detail returns metadata + journal, tag add/remove roundtrip, note appends journal, register onboards an existing folder, apply preview is non-writing while apply creates missing, template import→export roundtrip, and prune drops missing records.

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
- v0.4: `project_info::split_frontmatter_body()` is `pub` (not `pub(crate)`) so integration tests can assert byte-identity round-trips. The internal `extract_frontmatter` helper from v0.3 was folded into it — there's now one splitter.
- v0.4: `Predicate::Free` is the parser fallthrough, so any non-empty term that isn't `tag:`, `key=…`, `key>…`, `key<…` becomes a free-text predicate. Don't add another fallthrough below it (would be unreachable). Free terms search **case-insensitive substring** (not prefix) — keep it that way for grep-like UX.
- v0.4: `path` is intentionally NOT searched by `Predicate::Free`. There's a regression test (`free_does_not_match_path`) that asserts this; if you ever extend the field set, don't break that guarantee silently — home-dir leakage is a privacy footgun.
- v0.4: `cli::recent::run_picker` is `pub` because `cli::search` reuses it; `project_action_menu` stays private. If TUI/search both need a new picker action, add it inside `recent.rs`.
- v0.5: Bool flags after the slug used to silently drop. Fixed by `cli::new::classify_extra` — main.rs's New / Apply / Register arms all run their trailing `extra` Vec through it, then OR-combine the recognized flags into the relevant Args struct. Adding a new bool flag to `New`/`Apply`/`Register` requires updating `ExtraFlags`, the recognizer match in `classify_extra`, AND the OR-combine in each match arm — three coordinated edits. Forget the third and the flag works before the slug but mysteriously breaks after it.
- v0.5: `register` writes PROJECT_INFO.md in two steps: `project_info::write` (which uses `now_iso8601` inside `Metadata::from_plan`), then `project_info::write_frontmatter` to patch `created` to the resolved timestamp. Don't try to plumb the timestamp through `from_plan` — it'd break the byte-identity guarantee on the round-trip test and pollute the signature for a register-only concern.
- v0.5: `register` builds its `ProjectPlan` directly (pub struct fields) instead of calling `project::plan()` because plan always sets `root_path = cfg.base_dir.join(folder_name)`. Register's `root_path` is the canonical path of the existing folder. Don't refactor plan to take a path override — keep the two flows separate.
- v0.5: Without `--template`, register uses a `registered_stub_template()` (slug `"(registered)"`, `IdConfig::default()`). Recent and search will show these mixed with template-created projects. `project_info::render`'s "no variables" branch handles empty `tmpl.variables` correctly — don't add a special-case writer.
- v0.5: `paths_equal` (in `cli/register.rs`) is a `/`-vs-`\` normaliser used to detect duplicate registration on Windows. Don't replace with raw `==` on path strings — re-registering a folder whose index record was written with backslashes would slip past.
- v0.5: `sanitize_name` in `core/naming.rs` does NOT replace spaces — it only swaps filesystem-illegal chars (`/ \ : * ? " < > |`). For `fastf new`, the user-declared `transform` on each variable does space→underscore. Register's no-template path doesn't have a transform, so it uses `slugify_folder_name` (collapses whitespace runs to `_`, applies `sanitize_name`, preserves case). If you ever wire a no-template flow elsewhere, reach for `slugify_folder_name`, not `sanitize_name` alone.
- v0.5: `config::set "register-naming-pattern"` rejects patterns that don't contain `{id}`. This is a safety net — without `{id}`, registering multiple folders with the same `{name}` would all rename to the same target. Don't relax this check unless you've thought through the duplicate-rename UX.
- v0.5: `--apply` requires `--template` (still). `--rename` does not, as of v0.5 — it falls back to `cfg.register_naming_pattern`. If you add another "needs template" flag, encode the requirement in clap's `requires = "template"` AND in the defensive bail at the top of `register::run` (the public API can be called directly from tests, bypassing clap).
- v0.5: `PROJECT_INFO.md` is reserved. `Template::load_from_file` and `save_to_file` both call `strip_reserved_files()` (which uses `project_info::path_is_reserved`). The check is root-only (leaf `==` reserved name, case-insensitive, AND no `/` in the normalised path) so `docs/PROJECT_INFO.md` is allowed. The reserved name is hardcoded to `"PROJECT_INFO.md"` — NOT pulled from `cfg.project_info_filename` — so the safety net is independent of user config. If you change the auto-gen filename concept (e.g. multi-file project metadata), update `RESERVED_FILENAME` and consider whether the strip should become config-driven.
- v0.5: The TUI Settings → Project metadata submenu intentionally hides the filename customization. The toggle for `project_info_enabled` is still there. `fastf config set project-info-filename` still works for v0.3-era configs but is no longer surfaced in any interactive flow. Don't re-add the filename input to the TUI — it just leads users back to the foot-gun.
- v0.5: Template builder's `collect_file()` example shows `NOTES.md`, not `PROJECT_INFO.md`. It also rejects the reserved name inline with a loop-back. If you change the example, keep `NOTES.md` (or another genuinely non-reserved name) — `PROJECT_INFO.md` as an example actively misleads users into creating template entries that get silently stripped.
- v0.5: Template builder no longer asks "Template vs Raw" content mode. `collect_file()` always writes to `FileEntry.template`. The `FileEntry.content` field still exists in the YAML schema (hand-written templates with raw byte content keep working — e.g. `music-video.yaml`'s `.gitignore`), but the builder never produces it. `create_file()` and `apply()` still pick `template` when non-empty else `content`, so the dual-field semantics are preserved at the writer. If you re-add a mode switch, remember that `interpolate()` is already a no-op on text without `{token}` markers, so the only real use-case for `content:` is preserving literal `{...}` braces.
- v0.5: The "Add another placeholder file?" prompt in `edit_files()` defaults to **No** and explicitly mentions that PROJECT_INFO.md is generated automatically. Don't flip the default back to Yes — the typical template doesn't need extra placeholder files and the auto-gen covers the common notes use-case.

## Browser UI (`fastf ui`, v0.7)

`fastf ui` starts a local loopback HTTP server and opens the browser UI. It is
part of the `fastf` binary — **no separate `fastf-ui-server` binary**, no
external web directory. Full reference: `docs/UI.md`.

**v0.7 — feature-parity pass.** The UI now reaches the v0.4–v0.5 surface that was
CLI-only: a project **detail drawer** (variables table + tag add/remove + journal
notes, opened from any project row), real **search** using `core::query` (the
`/api/search` route), a **register** page for onboarding existing folders, an
**apply** modal (preview then create-missing), template **import / export /
generate-from-folder**, and Settings **ID-counter editor + prune**. Every one of
those maps to an existing `pub` library function, so the work was endpoint wiring
+ frontend views plus one refactor: `cli::register::register_core` /
`RegisterOptions` / `PinfoConflict` (the non-interactive engine the route calls).

Layout:
- `src/ui/mod.rs` — the HTTP server (`std::net::TcpListener`, one thread per
  connection, no web framework) + all API handlers. `pub fn serve(address)`
  blocks; `pub fn route_request(method, route, body) -> Response` is the pure
  router (no socket) and is what `tests/ui_server.rs` drives. `pub fn
  health_check(address)` lets `fastf ui` detect an already-running server. Write
  routes serialize through a private `static WRITE_LOCK: Mutex<()>`.
- `src/ui/assets.rs` — the four frontend files embedded via `include_str!`. If
  `FASTF_UI_DIR` is set, files are read from disk instead (frontend live-reload).
- `src/ui/web/` — `index.html`, `app.js`, `styles.css`, `icon.svg` (vanilla JS,
  no framework/npm/bundler).
- `src/cli/ui.rs` — the `fastf ui` command: health-check → open browser → serve.
  `--address`, `--no-open`, `--app` (Chromium/Chrome app window with a dedicated
  `~/.cache/fast-folder-ui/chromium` profile; falls back to the default browser).

The server calls the library directly (`project::plan`/`create`, `Config`,
`Counters`, `template`, `index`, `post_create`), so the UI and CLI share one
source of truth and the same on-disk files. The `Ui` arm in `main.rs` forwards to
`cli::ui::run`. `Response` derives `Debug` (tests `unwrap_err` on the router).

### UI gotchas
- v0.6: Only `GET` (assets + read APIs) and `POST` (writes) are routed —
  `HEAD`/others 404. Browsers GET, so this is fine; don't be surprised when
  `curl -I` shows the JSON 404 error body's content-type.
- v0.6: Adding a new write endpoint? Take `lock_writes()` inside the match arm
  (like `/api/create`) so it serializes with the other writers; reads don't lock.
- v0.6: Keep the server **loopback-only**. There is no auth/CSRF. `FASTF_UI_DIR`
  is the frontend dev override (serve assets from disk instead of embedded).
- v0.6: Embedded assets mean a frontend edit needs a `cargo build` to ship — but
  `FASTF_UI_DIR=$PWD/src/ui/web fastf ui` serves from disk for dev without rebuilding.
- v0.7: Query-string GET routes (`/api/project?path=`, `/api/templates/export?slug=`)
  are matched with `if path.starts_with(...)` guards placed BEFORE the static-asset
  catch-all (`("GET", path) if !path.starts_with("/api/")`). Order matters — a new
  `/api/...?` GET route must go above the catch-all. Values are percent-decoded by
  the in-module `query_param` + `percent_decode` (no url crate); the frontend uses
  `encodeURIComponent`, which emits `%20` (not `+`) for spaces, so the decoder does
  NOT treat `+` as space (paths can legitimately contain `+`).
- v0.7: `register_core` is the non-interactive engine; `fastf register`'s `run` is a
  thin interactive shell over it (rename preview + pinfo-overwrite prompts there,
  not in core). `RegisterArgs` (CLI) and `RegisterOptions` (engine) are separate
  structs — `run` translates one to the other. The CLI rename-confirm shows the
  exact `old → new` name via a preview computed from `build_plan_vars` +
  `desired_rename` (shared helpers), reading `counter.get()+1` without incrementing;
  the engine recomputes the same value and is authoritative.
- v0.7: `PinfoConflict` controls the existing-`PROJECT_INFO.md` policy. `Abort`
  bails BEFORE the counter/index writes (so the UI can confirm + retry with
  `overwrite:true` without a duplicate-registration bail); `Skip` keeps the file but
  still registers (index + counter); `Overwrite` rewrites it. The UI sends `Abort`
  unless the user confirmed overwrite; the CLI sends `Overwrite`/`Skip` from its
  prompt and never `Abort` (preserving the old "register regardless" behavior).
- v0.7: The UI `apply` routes pass **raw** variables to `project::apply_plan` /
  `apply` (no transform/sanitize) — matching `fastf apply`'s semantics, NOT `new`'s.
  Don't "fix" this to apply transforms; it would diverge the UI from the CLI apply.
- v0.7: `/api/search` reuses `core::query` exactly, so the deliberate "path excluded
  from free-text" guarantee holds (there's a regression test). Empty `terms` returns
  all projects (newest first) — the server-side equivalent of the plain list.
- v0.7: The frontend renders the drawer + apply modal + generic (import/from-folder)
  modal as state-driven layers appended after `shell(content)` in `render()`; each
  has a `bind*` call. Mutating actions re-fetch + full `render()` (acceptable — the
  only focus-sensitive surface is the projects search, which updates `#project-results`
  in place via `runProjectSearch` instead of re-rendering). `(registered)`-slug
  projects are filtered out of the apply/register template pickers.
