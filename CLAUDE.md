# CLAUDE.md — fastf development context

## What this project is

`fastf` (Fast Folder Creator) is a portable Rust CLI tool for creating structured project folders from **folder templates**. Universal use cases: code, research, finance, music video, photography, and film production workflows. Single-folder portable distribution — config, templates, and the counter live next to the binary.

**v1.0: production release.** Distribution-ready: data-dir precedence (env override → portable → user config dir), GitHub Actions CI + release binaries (linux-gnu, linux-musl static, windows-msvc — NO macOS, cristoc can't test it), AUR packages `fast-folder` (source) + `fast-folder-bin` (musl repack) in `packaging/aur/`, man pages via hidden `fastf mangen` (clap_mangen), `fastf paths`, project **unregister/delete/rename** (library + UI routes + TUI action menu), and a web-UI polish pass (promise-based confirm/prompt dialogs replacing native `confirm()`, offline banner + health poll, sortable projects table, focus-preserving `render()`, spinners). Crate `0.11.0 → 1.0.0`; 176 tests. See "Data-dir resolution (v1.0)" and "Unregister / delete / rename (v1.0)" below.

**v0.11: durable provisioning + verified moves.** Two flows that write bulk data are now crash-safe and never lose data. (1) **Deferred create copies** write a durable `.fastf-provisioning.json` marker into the new project root before the background copy starts; each file is flipped `done` as it lands and the marker is deleted on completion. (2) **Moves** never remove the source until the destination is copied **and** verified: same-filesystem moves stay an instant atomic `fs::rename`, while cross-filesystem / network moves stage the copy into a dot-prefixed `.<folder>.fastf-part` folder guarded by a `.fastf-move-<folder>.json` marker, run `assets::verify_tree` (size + count + existence), atomically rename into place, and only then remove the source. Both flows report live `copy → verify → finalize` progress (`assets::Progress.phase`) and are cancellable. **`fastf reconcile`** (and a UI banner + `POST /api/reconcile`, also driven by `/api/state`'s `provisioning` list) resumes interrupted copies and finishes-or-rolls-back interrupted moves. New core module `src/core/provisioning.rs`; UI `POST /api/project/move` now returns a `job_id` (backgrounded) and `POST /api/job/<id>/cancel` cancels. Crate `0.10.0 → 0.11.0`; 164 tests. See "Durable provisioning + verified moves (v0.11)" below.

**v0.10: base-aware projects.** Every discovered `Project` carries the `base` it lives under (`library::Project.base`, populated at discovery — the cache format is unchanged; base is implicit per-cache-file). All overviews show the base (CLI plain lists + picker via `library::base_label`, browser UI Base column + drawer), **`fastf move <query> [base]`** moves a project folder into another *configured* base (cross-filesystem falls back to `assets::copy_tree` + remove; both base caches and the metadata `path` are patched — see `library::move_project`), and every create surface lets you pick the target base (TUI Select when >1 base, UI create-form select, CLI `--base-dir=`). UI route: `POST /api/project/move`. See "Base-aware projects (v0.10)" below.

**v0.9: the filesystem is the source of truth.** There is no `projects.jsonl`. A folder is a project **iff** it contains a `PROJECT_INFO.md` (the `id` in its YAML frontmatter is authoritative; the folder name is cosmetic). `fastf` discovers projects across all bases (`base_dir` + config `bases`), accelerated by a disposable per-base `.fastf-index.json` cache that self-heals (base-mtime gate + per-entry existence check — no manual prune). The global counter self-heals: next ID = `max(counter_file, highest id discovered) + 1`. See "Filesystem-as-truth library (v0.9)" below. Core in `src/core/library.rs`.

**v0.8: a template is a folder.** `templates/<slug>/template.yaml` holds metadata (variables, naming_pattern, id, structure, post_create, tags, tag_from, verbatim/exclude globs); a sibling `templates/<slug>/files/` subtree IS the file spec — every file/dir under it is reproduced into each new project, with `{token}` interpolation on names and on UTF-8 text (≤1 MiB), binaries copied byte-for-byte. See "Folder templates + bundled assets (v0.8)" below.

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

# Test (176 total: 89 unit/lib + 55 core in integration.rs + 32 UI in ui_server.rs)
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
│   └── templates/            — Gallery templates in folder form
│                               (rust-project, python-project, web-project, finance-monthly,
│                               research-note), each <slug>/template.yaml + files/. NOT
│                               bundled — copy a folder into your templates/ dir to use one.
├── docs/
│   └── UI.md                 — Browser-UI reference (architecture, HTTP API, dev live-reload)
├── Launch Fast Folder UI.desktop — desktop launcher (Exec=`fastf ui --app`)
├── tests/
│   ├── integration.rs        — 48 hermetic core tests using FASTF_INSTALL_DIR + tempfile
│   │                           (write_template splits an inline files: block onto disk)
│   └── ui_server.rs          — 23 tests driving fastf::ui::route_request (v0.6/v0.7 core
│                               + v0.8 bundled-file reproduce + background copy job +
│                               template file list/save/add/delete + reserved/traversal guard
│                               + from-folder bundling report)
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
    │                           (the two bundled templates — general + client-project, universal by design; domain
    │                           templates live in examples/templates/ — no longer declare PROJECT_INFO.md —
    │                           auto-gen owns it now)
    ├── util/
    │   ├── mod.rs
    │   └── paths.rs          — install_dir(): FASTF_INSTALL_DIR override, else current_exe().
    │                           config/counters/templates paths. (No projects_index_path — v0.9
    │                           discovers projects from the filesystem, not a jsonl next to the binary.)
    ├── core/
    │   ├── mod.rs
    │   ├── assets.rs         — v0.8 copy engine. walk(files_dir) lists the files/ subtree;
    │   │                        interp_rel() (per-segment name interpolation); is_verbatim/
    │   │                        is_excluded glob matching; copy_file() (atomic .part+rename,
    │   │                        UTF-8→interpolate else byte copy, non-UTF-8 auto-verbatim);
    │   │                        TEXT_MAX_BYTES (1 MiB interpolation cap). Job model:
    │   │                        JOB_DEFER_BYTES (4 MiB), CopyJob, Progress (Serialize),
    │   │                        copy_job() (chunked byte copy w/ live progress).
    │   │                        v0.10: copy_tree(src,dst) — recursive verbatim dir copy
    │   │                        (move_project's cross-device fallback; never interpolates).
    │   ├── config.rs         — Config: base_dir, bases (v0.9 extra index dirs),
    │   │                        editor, date_format, default_template, preview_lines (8),
    │   │                        post_create, prompt_open_after_create, recent_default_limit,
    │   │                        confirm_create, show_banner, register_naming_pattern.
    │   │                        effective_bases() = dedup([base_dir] + bases), canonicalized.
    │   │                        v0.9 REMOVED project_info_enabled/project_info_filename
    │   │                        (metadata is mandatory, always PROJECT_INFO.md); old configs
    │   │                        with those keys still parse (serde ignores unknown fields).
    │   ├── counter.rs        — Global auto-increment ID (single 'global' field in counters.toml)
    │   ├── naming.rs         — apply_transform(), interpolate() [raw for file CONTENT],
    │   │                        interpolate_name() [collapses __ and trims for NAMES],
    │   │                        sanitize_name(), ensure_relative_safe_path()
    │   ├── project.rs        — ProjectPlan, plan(), create(run_post), print_dry_run(),
    │   │                        print_resolved_values(), print_file_previews(), print_tree(),
    │   │                        apply_plan(), apply(), print_apply_plan(), ApplyAction enum.
    │   │                        resolve_post_create() is pub so cli/new.rs can check for
    │   │                        double-open before offering the open prompt. v0.8: files come
    │   │                        from the template's files/ subtree via copy_template_files()
    │   │                        (walks core::assets), NOT template.files. create_deferred()
    │   │                        does the eager work + returns large-file CopyJobs for the UI
    │   │                        to copy in the background; create() stays fully synchronous.
    │   ├── project_info.rs   — Metadata struct (incl. tags), render(), write(plan,tmpl,tags),
    │   │                        read(dir), read_metadata(dir), pinfo_path(dir) [v0.9: fixed
    │   │                        filename, no cfg]. v0.4: write_frontmatter(path, mutator) for
    │   │                        atomic in-place tag mutation; append_journal_entry(path, msg)
    │   │                        for ## Journal section; read_journal_entries(); split_frontmatter_body()
    │   │                        is pub for byte-identical body round-trips.
    │   │                        v0.5: pub const RESERVED_FILENAME = "PROJECT_INFO.md" +
    │   │                        path_is_reserved(p) helper used by Template::strip_reserved_files
    │   │                        and the TUI template builder to lock the auto-gen filename.
    │   ├── provisioning.rs   — v0.11 durability layer. Create marker (.fastf-provisioning.json
    │   │                        in project root): write_create_marker/mark_done/clear_create.
    │   │                        Move marker (.fastf-move-<folder>.json at target base):
    │   │                        write_move_marker/clear_move/staging_path. reconcile(cfg) →
    │   │                        ReconcileReport (resume creates, finish/roll-back moves);
    │   │                        list_incomplete(cfg) for the UI banner. MARKER_CREATE /
    │   │                        MARKER_MOVE_PREFIX consts (referenced by assets::is_transient).
    │   ├── template.rs       — Template (+ post_create, tags, tag_from, v0.8 verbatim/exclude
    │   │                        globs + dir + files), Variable, FolderNode, FileEntry, IdConfig,
    │   │                        Transform. validate() is pub and rejects tag_from entries that
    │   │                        aren't declared variable slugs. v0.8: folder form only —
    │   │                        load_from_file(<slug>/template.yaml) sets `dir` and scans the
    │   │                        files/ subtree's UTF-8 text into the `files` buffer (NOT
    │   │                        serialized — files/ on disk is the source of truth; buffer is
    │   │                        for editors/previews). save_to_file writes template.yaml +
    │   │                        flushes text `files` into files/. load_all() iterates subdirs;
    │   │                        find_by_slug/file_path → <slug>/template.yaml. files_dir().
    │   │                        strip_reserved_files() still drops root PROJECT_INFO.md.
    │   ├── query.rs          — v0.4. Predicate enum (Field/After/Before/Tag/Free), Pattern
    │   │                        (Exact/Prefix), parse() and evaluate(). Bare terms become
    │   │                        Predicate::Free — case-insensitive substring across vars/tags/
    │   │                        folder/template/template_name/id (path EXCLUDED).
    │   ├── vars.rs           — collect_vars() shared by `new` and `apply`
    │   ├── library.rs        — v0.9 filesystem-as-truth. Project struct; discover(cfg) unions
    │   │                        effective_bases() newest-first (cache-first + staleness gate);
    │   │                        scan_base() reads depth-1 folders with PROJECT_INFO.md;
    │   │                        cache format .fastf-index.json (base-relative `dir`);
    │   │                        resolve(cfg,query) [replaces index::resolve_project];
    │   │                        max_id(cfg) [read-only, safe from plan()]; reindex(cfg);
    │   │                        cache_upsert/cache_remove/refresh_cache; now_iso8601().
    │   │                        v0.10: Project.base (the base a project was discovered
    │   │                        under); base_label(base) [short display name];
    │   │                        move_project(project,new_base) [rename or copy_tree+remove
    │   │                        fallback, patches metadata `path`, two-sided cache update].
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
    │   ├── template.rs       — list/show/edit/delete (delete removes the whole <slug>/ dir) +
    │   │                        from_folder() for template generation from existing dirs.
    │   │                        v0.8: import/export removed (templates are folders — share by
    │   │                        copying the folder). Phase 4: from_folder(source, slug, force,
    │   │                        bundle_assets) -> FromFolderReport is the non-interactive core
    │   │                        (UI + tests); run_from_folder() is the CLI shell (size-confirm +
    │   │                        summary). scan_source()/execute_scan() split so the CLI confirms
    │   │                        before writing; bundle_assets copies binary/large files verbatim.
    │   ├── config.rs         — config show/set. Keys: base-dir, bases (v0.9 comma-list),
    │   │                        prompt_open_after_create, confirm_create, show_banner,
    │   │                        recent_default_limit, preview_lines, post_create.* keys,
    │   │                        register_naming_pattern (rejects patterns without `{id}`).
    │   │                        v0.9 dropped project-info-enabled/-filename keys.
    │   ├── id.rs             — id show/reset/set
    │   ├── reindex.rs        — v0.9. `fastf reindex` → library::reindex(cfg): force full
    │   │                        rescan of every base + rewrite each .fastf-index.json.
    │   ├── reconcile.rs      — v0.11. `fastf reconcile` → provisioning::reconcile(cfg): resume
    │   │                        interrupted copies + finish/roll-back interrupted moves.
    │   ├── recent.rs         — `fastf recent`: defaults to interactive picker (TTY). v0.9:
    │   │                        sources from library::discover (no --prune). picker →
    │   │                        project_action_menu() → Open / Show metadata / Add tag /
    │   │                        Remove tag / Add journal note / Show journal / Back / Quit.
    │   │                        Inline tag display from project.tags (from discovery, no reload).
    │   │                        --tag <name> filter. run_picker(&[&Project]) pub so cli/search
    │   │                        reuses it. open(query) → library::resolve. --plain / non-TTY
    │   │                        gives classic list output.
    │   ├── tag.rs            — v0.4. add/remove/list/reauto. add is idempotent; remove no-ops
    │   │                        on missing tags; reauto preserves free-form, replaces derived.
    │   ├── note.rs           — v0.4. note add: inline / `-` (stdin) / omit ($EDITOR via cfg).
    │   │                        notes: prints filtered ## Journal entries (--since YYYY-MM-DD).
    │   ├── search.rs         — v0.4. `fastf search <terms...>`. Parses via core::query, walks
    │   │                        library::discover newest-first, reads metadata per project,
    │   │                        evaluates predicates (query::evaluate(preds, meta) — v0.9 no
    │   │                        record arg), renders via run_picker (TTY) or plain list.
    │   ├── apply.rs          — `fastf apply <slug> <dir>` with --dry-run (skip-only semantics)
    │   ├── move_project.rs   — v0.10. `fastf move <query> [base]` (module can't be named
    │   │                        `move` — keyword). resolve → validate target ∈ effective_bases
    │   │                        (full path OR base_label accepted; TTY picker when omitted) →
    │   │                        library::move_project. Positional args, no classify_extra.
    │   ├── ui.rs             — v0.6. `fastf ui` launcher: health-check → open browser
    │   │                        (--app = Chromium/Chrome app window, else default) → serve.
    │   │                        Calls fastf::ui::serve(); UiArgs { address, no_open, app }.
    │   └── register.rs       — `fastf register <path>` writes a PROJECT_INFO.md into an
    │                            existing folder (its whole job in v0.9 — no index). ID recovered
    │                            from an ID#### token in the folder name (parse_id_token) if
    │                            present, else minted from the self-healed floor. cache_upsert
    │                            after write. Optional --template (full metadata + tags), --apply
    │                            (requires --template), --rename, --use-today / --created.
    │                            v0.9: --recursive (+ --dry-run) → run_recursive() onboards every
    │                            metadata-less direct child of a base (PinfoConflict::Skip).
    │                            RegisterOutcome carries a library::Project. resolve_created() pub;
    │                            REGISTERED_SLUG = "(registered)".
    ├── tui/
        ├── mod.rs
        ├── menu.rs           — Interactive TUI menu. ASCII banner (suppressed if !show_banner).
        │                        Live base dir display. Top-level menu:
        │                          Create / Recent / Search projects (v0.4) / Manage templates
        │                          / Settings / Quit.
        │                        menu_search() prompts for a query string, splits on
        │                        whitespace, calls cli::search::run.
        │                        menu_settings() grouped submenus:
        │                          Project basics / Workflow prompts / Library bases (v0.9) /
        │                          Recent projects / Post-create actions / ID counter.
        │                        Every config field has a toggle/edit entry with inline state.
        │                        v0.10: menu_create() → pick_base_interactively() (base Select
        │                        when >1 mounted base; index 0 = default → None override).
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

### Data-dir resolution (v1.0)
`paths::try_install_dir() -> Result<(PathBuf, DirMode)>` resolves the one data dir (config + templates + counters) with three tiers: (1) `FASTF_INSTALL_DIR` env (tests + power users), (2) **portable mode** — the exe's canonicalized parent iff it contains `config.toml` or `templates/` (keeps binary-plus-data folders working: `target/release/`, USB sticks), (3) **user config dir** — `$XDG_CONFIG_HOME/fastf` / `~/.config/fastf` (unix) or `%APPDATA%\fastf` (Windows), hand-rolled (no `dirs` crate). Tier 3 is what makes a package-manager install to read-only `/usr/bin` work: `ensure_bootstrapped()` lazily `create_dir_all`s the dir on first run. `install_dir() -> PathBuf` keeps its infallible signature (~30 call sites untouched) but never panics — it exits(2) with an actionable message on the unreachable error path; `main.rs` calls `try_install_dir()?` first thing so real users get a pretty anyhow error instead. No memoization (tests swap the env var in-process). Surfaced via `fastf paths` (cli/paths_cmd.rs), a mode line in `config show`, and `dir_mode` in `/api/state`. Bootstrap is **skipped** for `completions`/`mangen` so packaging steps never write to $HOME. (Projects themselves live in the bases; each base carries its own `.fastf-index.json` cache — v0.9.)

### Unregister / delete / rename (v1.0)
Three `library` fns, mirroring move's conventions (callers restrict to configured bases; core fns guard the basics): `unregister_project` removes only `PROJECT_INFO.md` + cache entry (files stay); `delete_project` refuses unless the path is a direct child of its base AND still contains `PROJECT_INFO.md`, then `remove_dir_all` + cache_remove; `rename_project` sanitizes the name (rejects empty / dot-prefixed — discovery skips dot dirs / same-name), same-parent atomic `fs::rename`, patches metadata `folder`+`path` best-effort, cache remove+upsert. UI routes (all under WRITE_LOCK): `POST /api/project/unregister {path}`, `/delete {path, confirm_name}` (server re-checks the typed folder name), `/rename {path, folder}`. Drawer buttons gated on `record` (project must be discovered); delete uses a typed-phrase confirm modal. TUI: dynamic tail entries in `project_action_menu` (Rename / Unregister / Delete before Back/Quit — keep them index-relative).

### Cross-platform paths
Folder paths in templates (structure names, file paths) always use `/` as the separator in YAML — Rust's `PathBuf::join()` handles conversion to `\` on Windows at runtime. Users should always enter `/` in templates and `base-dir` config, though Windows also accepts backslashes in config values.

### Global ID counter
One counter for all templates: `counters.toml` with a single `global` field. Every project creation increments it. `fastf id set 46` → next project gets ID0047. This is intentional — IDs are unique across all project types.

### Template YAML schema (v0.8, folder form)
`templates/<slug>/template.yaml` is **metadata only** — the file spec lives in the
sibling `files/` directory (see below). Keys:
- `naming_pattern`: tokens `{date}`, `{YYYY}`, `{MM}`, `{DD}`, `{id}`, plus any variable slug
- Variables: `type: text` (free input) or `type: select` (pick from list)
- Transforms: `none`, `title_underscore`, `upper_underscore`, `lower_underscore`
- `structure`: nested `FolderNode` list (name + children) — the canonical, archive-safe way to declare **empty** dirs. Names support forward slashes when entered via the builder — parsed via `parse_paths_to_tree()`.
- `verbatim` (v0.8): glob list (relative to `files/`) whose files are copied literally even if text — preserves literal `{braces}`.
- `exclude` (v0.8): glob list never copied (e.g. `.DS_Store`, `*.tmp`).
- `post_create` (optional): per-template override of the global `config.post_create`.
- **No `files:` key.** (`FileEntry`/the in-memory `files` buffer still exist for editors/previews; they are `#[serde(skip)]` — never written to the manifest.)

`files/` subtree: every file/dir is reproduced into new projects. Names + UTF-8 text (≤ `TEXT_MAX_BYTES`) are interpolated; `verbatim`/oversize/non-UTF-8 files are byte-copied; `exclude` globs skipped; root `PROJECT_INFO.md` is reserved and skipped.

### Interpolation: `interpolate` vs `interpolate_name` (important)
Two separate functions in `core/naming.rs`:
- **`interpolate()`** — raw substitution only. Used for **file content** (templated files). Preserves `__` sequences so Python's `__version__`, `__init__`, etc. survive intact.
- **`interpolate_name()`** — calls `interpolate`, then collapses consecutive `__` → `_` and trims leading/trailing `_`. Used for **folder and file names** so empty optional variables don't leave dangling underscore gaps.

When adding new code: if you're building a *path component name*, call `interpolate_name`. If you're building *file contents*, call `interpolate`. Do not mix them.

### Path-escape safety
`ensure_relative_safe_path()` rejects absolute paths, Windows drive letters, leading separators, and any `..` segment. Enforced in two places:
1. `Template::validate()` at template-load time (so broken templates fail at `fastf template list`).
2. `create_file()` and `apply()` at disk-write time (defence in depth).

### Filesystem-as-truth library (`core/library.rs`, v0.9)
There is **no `projects.jsonl`**. The project list is discovered from the filesystem: a folder is a project iff it holds a `PROJECT_INFO.md`, whose frontmatter `id` is authoritative (folder name is cosmetic, never consulted for discovery). `discover(cfg)` unions `cfg.effective_bases()` (`base_dir` + config `bases`), newest-first.

Each base carries a **disposable** `.fastf-index.json` cache at its root, co-located with the projects so it travels with them and is portable (entries store a base-relative `dir`, valid across `/mnt/…` and `D:\…`). The cache is never authoritative — `discover_base` self-heals: if the base's mtime is newer than the cache (or either can't be stat'd) it rescans + rewrites; otherwise it trusts cached metadata but existence-checks each entry and drops (rewriting away) any whose folder disappeared. **No manual prune, ever** — the "missing" state is transient. `fastf reindex` forces a full rescan for external edits fastf can't observe.

`max_id(cfg)` is **read-only** (reads a fresh cache or scans, never writes) so it's safe to call from `plan()`/preview. `resolve(cfg, query)` replaces the old `index::resolve_project` (exact-id → id-prefix → name-substring). `cache_upsert`/`refresh_cache` keep the cache fresh after create / tag mutations without a rescan. All cache writes are best-effort and atomic (`.tmp` + rename); a cache error never fails a command. Counter self-heals: `plan()` computes the ID from `max(counters.get(), library::max_id(cfg)) + 1`.

### Durable provisioning + verified moves (v0.11)

**Invariant:** the source of a move is never removed until the destination is
copied **and** verified; every in-flight bulk copy has a durable on-disk marker
so a crash never strands data silently.

**Markers** (`src/core/provisioning.rs`):
- Create — `.fastf-provisioning.json` in the new project root, one entry per
  deferred copy (`write_create_marker` → `mark_done` per file → `clear_create`).
- Move — `.fastf-move-<folder>.json` at the *target base* root, recording
  `src` / `temp` (`.<folder>.fastf-part`) / `final` / `phase`.

**`assets` additions:** `Progress.phase` (`copying|verifying|finalizing|done`);
`copy_job(job, progress, cancel)` polls a `&AtomicBool` between chunks and removes
its `.part` on cancel (returns `CANCELLED_MSG`); `jobs_for_tree(src,dst)` →
`(dirs, files)`; `verify_tree(src,dst)` (size + count + existence, ignoring
transient scaffolding via `is_transient` — cache/markers/`.part`). `copy_tree`
is retained (unused by move now, still `pub`).

**`library::move_project`** delegates to `move_project_with(project, new_base,
&Mutex<Progress>, &AtomicBool)`: same-fs `fs::rename` fast path, else
`staged_copy_verify_commit` (marker → copy into staging with progress/cancel →
`verify_tree` → atomic `rename(temp,final)` → patch metadata `path` + both caches
→ remove source → clear marker). A verify failure or cancel aborts **before** the
source is touched (staging + marker cleaned). `scan_base` now skips dot-prefixed
dirs so a staging folder (which contains a `PROJECT_INFO.md`) never surfaces as a
phantom project.

**Recovery:** `provisioning::reconcile(cfg)` resumes pending create copies and
finishes (source removed) or rolls back (staging discarded, source intact) each
move marker. Surfaced by `fastf reconcile` (CLI), a UI banner + `POST
/api/reconcile`, and `provisioning::list_incomplete` folded into `/api/state`.
Reconcile is **not** run on every CLI command (only `fastf reconcile` /the UI) —
the never-delete-until-verified design means an unreconciled crash is always
safe, just untidy.

**UI:** `POST /api/project/move` pre-flights the cheap guards synchronously, then
returns `{ ok, job_id }` and runs the move on a background thread **off
`WRITE_LOCK`** (it only writes the target staging + atomic caches). The `JOBS`
registry value is now `JobHandle { progress, cancel }`; `POST /api/job/<id>/cancel`
sets the flag. Frontend: `showMoveProgress` overlay (copy→verify→finalize bar +
Cancel), `provisioningBanner()` + `reconcileProvisioning()`, `phaseLabel(job)`.

### Base-aware projects (v0.10)

Every discovered `Project` carries `base: PathBuf` — the effective base it was
found under, set at the two construction points (`CacheEntry::into_project(base)`
and `project_from_meta(meta, base, dir)`). The cache format is **unchanged**
(no `base` field in `CacheEntry` — the base is implicit: it's whichever base's
`.fastf-index.json` the entry lives in, which is what keeps caches portable).
`library::base_label(base)` renders the short display name (last path
component) used by all list surfaces.

**`library::move_project(project, new_base) -> Result<Project>`** relocates a
project folder into another base, keeping the folder name:
1. Bails when `new_base` isn't a dir, equals the current base (canonicalized),
   or `new_base.join(name)` already exists (mirror of register's rename guard).
2. Tries `fs::rename`; on ANY error falls back to `assets::copy_tree` +
   `fs::remove_dir_all` — covers EXDEV (btrfs `~` → NTFS `/mnt/proj`) without
   fragile errno matching. The copy is **verbatim** (no interpolation, ever).
3. Patches frontmatter `path` via `write_frontmatter` (best-effort warn —
   discovery never reads `path`, it's display-truth only). `folder` unchanged.
4. Two-sided cache update, best-effort: `cache_remove(old_base, old_rel_dir)` +
   `cache_upsert(new_base, &moved)`.

Move targets are **configured bases only** (`effective_bases()`), enforced by
every caller (CLI `fastf move`, TUI action menu, UI route) so a moved project
always stays discoverable — `move_project` itself doesn't consult config.
Surfaces: `fastf move <query> [base]` (base = full path or its label; TTY
picker when omitted), "Move to another base" in the recent/search action menu
(hidden when there's nowhere to go), `POST /api/project/move {path, base}` +
drawer select in the browser UI.

**Base pick on create:** the mechanism is unchanged since v0.5 — mutate
`config.base_dir` before `project::plan()`. v0.10 only added pickers: TUI
`pick_base_interactively()` (Select when >1 mounted base) and the UI create
form's base `<select>` (feeds the existing `PlanRequest.base_dir`). CLI stays
`--base-dir=`.

### `PROJECT_INFO.md` — structured per-project metadata
`core/project_info.rs` generates a `PROJECT_INFO.md` in each new project root. The file has two layers:

1. **YAML frontmatter** (between `---` lines) — the machine-readable layer. Typed struct `Metadata` serialized via `serde_yaml`. Contains: `id`, `template` (slug), `template_name`, `created` (ISO-8601), `folder`, `path`, and `variables: BTreeMap<String, String>` with **every** template variable regardless of whether it appears in `naming_pattern`. `BTreeMap` keeps keys alphabetical for diff-stability.

2. **Human body** — markdown table of variables (using template labels as column headers) + a `## Notes` section the user owns. The body also gains a `## Journal` section the first time `append_journal_entry` is called. Outside of those mutation helpers, fastf never modifies the file after creation.

`read_metadata(path)` slices out the frontmatter via `split_frontmatter_body()`, feeds it to `serde_yaml::from_str::<Metadata>`. Returns `Ok(None)` when no frontmatter block is present (older / hand-edited files). `read(path)` returns raw markdown for fallback display. **v0.9:** the filename is fixed (`RESERVED_FILENAME`); `write`/`read`/`read_metadata`/`read_journal_entries` no longer take a `cfg` (use `pinfo_path(dir)`); metadata is mandatory (no "disabled" toggle).

**Atomic mutation** (v0.4): `write_frontmatter(path, |meta| { ... })` reads → splits → parses → applies the closure → re-serializes via `serde_yaml::to_string` → writes via `.tmp` + rename. Body bytes are byte-identical after a no-op mutation — the dedicated integration test asserts this. `append_journal_entry(path, msg)` does the same atomic dance for the body. Both require frontmatter to exist; otherwise return a structured error naming the path.

The bundled templates (since 2026-07-16: `general` — `{date}_{name}_{id}`, one `00_Inbox` folder — and `client-project` — client/project/tier vars + a BRIEF.md that demonstrates content interpolation. Deliberately universal; the domain-specific `music-video`/`photography`/`video-production` moved to `examples/templates/` alongside the dev/finance/research gallery) no longer declare a `PROJECT_INFO.md` content file — auto-gen owns that file. **As of v0.5, `PROJECT_INFO.md` at the project root is a reserved filename**: `Template::load_from_file` and `save_to_file` silently strip any `files[].path == "PROJECT_INFO.md"` entry (case-insensitive on the leaf, root-only — `docs/PROJECT_INFO.md` is allowed). Older user-built templates that declared their own `PROJECT_INFO.md` keep loading; the entry is just ignored. The TUI template builder rejects the name inline. If you want a custom notes file, use a different name (e.g. `NOTES.md`).

The reservation is enforced via `core::project_info::path_is_reserved()` against the hard-coded constant `RESERVED_FILENAME = "PROJECT_INFO.md"`. **v0.9:** the filename is now fixed everywhere — the old `cfg.project_info_filename` / `project_info_enabled` config knobs are gone (metadata is the project's identity, so it is mandatory and always `PROJECT_INFO.md`).

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
`cli/recent.rs` decides interactive vs plain by `!args.plain && std::io::stdout().is_terminal()`. In interactive mode: `dialoguer::Select` picker over the filtered `library::Project`s + a `[Quit]` sentinel at the end. Selecting a project enters `project_action_menu()` which loops until Back/Quit. **v0.9:** `--prune` is gone (the cache self-heals); the picker sources from `library::discover` and reads tags straight off `project.tags`.

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

`cli/search::run` walks `library::discover` (newest-first), reads each metadata file (silently skipping projects with unreadable PROJECT_INFO.md), and renders matches via `recent::run_picker` on TTY or a plain list when piped/`--plain`. `query::evaluate(preds, meta)` takes only `&Metadata` (v0.9 dropped the `ProjectRecord` arg).

**Journal** entries are markdown lines under a `## Journal` section in the body. Format: `- 2026-04-20T14:32:11Z — message`. Append-only and chronological — `append_journal_entry` always appends at EOF after the section, never edits existing entries. `parse_journal_entries` walks the section and stops at the next `## ` heading. `notes --since YYYY-MM-DD` filters by lexicographic timestamp comparison (cheap and correct because ISO-8601 is sortable as string).

Project-resolution shared helper: `library::resolve(cfg, query)` does exact-id → id-prefix → name-substring (case-insensitive) over `discover`. Used by `tag`, `note`, and `fastf open`. Ambiguous queries return a structured error listing the candidates.

### `fastf register <path>` (reworked v0.9)
Register's whole job is now: **write a `PROJECT_INFO.md` into a folder that lacks one**, making it discoverable. No index, no counter-first ordering. Flow: canonicalize → resolve `created` → resolve template (or stub) → early PinfoConflict check → compute ID → optional rename → write metadata → `cache_upsert` → optional `--apply`.

**ID source (decision 8):** recover an `ID####` token from the folder name via `naming::parse_id_token(folder, prefix)` (any digit count → numeric value, ignoring zero-padding) — the *only* place folder names still influence identity. If absent, mint fresh from the self-healed floor `max(counter, library::max_id(cfg)) + 1`. The counter advances monotonically (`if id_value > counters.get()`), so recovering a low ID never lowers it.

Two-step pinfo write patches the historical `created`: `project_info::write(&plan, &tmpl, &tags)` (which sets `created = now`), then `write_frontmatter(path, |m| m.created = resolved)`. `resolve_created`: `--created YYYY-MM-DD` → `T00:00:00Z`; `--use-today` → now; default → `fs::metadata.created()` → `modified()` fallback.

Without `--template`, a stub `Template` is used (`slug = "(registered)"` = `REGISTERED_SLUG`, `IdConfig::default()`, empty everything). `--rename` uses `tmpl.naming_pattern` (with template) or `cfg.register_naming_pattern` (without, synthesising `{name}` via `slugify_folder_name`). `--apply` requires `--template`.

**PinfoConflict** (existing metadata policy): `Abort` (UI default) bails before any write; `Skip` keeps the file (used by `--recursive`); `Overwrite` rewrites. The CLI `run` resolves this from `--yes`/TTY prompts. `--recursive` (`run_recursive`) writes a `PROJECT_INFO.md` into every metadata-less direct child of a base (sorted); `--dry-run` previews (showing recover-vs-mint per folder) and writes nothing. `RegisterOutcome` carries a `library::Project`; `cache_upsert` only runs when metadata was actually written.

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

`menu_recent()` delegates directly to `recent::run` — keeps the picker one
keypress away from the main menu. **v0.9:** there's no prune maintenance (the
cache self-heals); Settings → Library bases (`menu_settings_bases`) edits the
`bases` list instead.

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
├── Library bases                (v0.9 — add/remove extra index dirs)
├── Recent projects              (default limit)
├── Post-create actions          (git / reveal / editor / path / commands)
├── ID counter
└── Back
```
Each toggle entry shows current `[on]`/`[off]` state inline via `label_toggle()`. `toggle_setting(key, current)` calls `config::set` under the hood.

## Testing

Integration tests live in `tests/integration.rs` (core flows) and `tests/ui_server.rs`
(browser-UI request layer). Both use:
- `FASTF_INSTALL_DIR` env var to redirect `paths::install_dir()` to a tempdir per test
- `tempfile::TempDir` for hermetic sandboxes
- A `static SERIAL: Mutex<()>` to run tests serially within the test binary (Rust 2024 edition made `std::env::set_var` unsafe — the mutex justifies the `unsafe` block). Each test binary has its own `SERIAL`; that's fine because `FASTF_INSTALL_DIR` is per-process and `cargo test`'s binaries are separate processes.

`integration.rs` covers: basic round-trip, transforms, counter persistence, duplicate-project rejection, dry-run no-write, apply skip-logic, from-folder round-trip, path-escape rejection (parent, absolute, drive letter), Windows forward-slash paths, gallery-YAML parsing, PROJECT_INFO.md frontmatter, variable capture, metadata round-trip via YAML, config back-compat (removed project_info_* keys still parse), bundled-template deduplication guard, from-folder asset bundling, and **v0.9**: create is discoverable without a jsonl (+ cache written), counter self-heals from existing projects, register recovers an ID from the folder name / mints fresh, `--recursive` onboards children (+ `--dry-run` writes nothing, skips existing metadata), Abort policy on existing metadata. `core/library.rs` unit tests cover discovery (only PROJECT_INFO.md folders), base-relative cache round-trip, staleness rescan, drop-missing, multi-base union+sort, `max_id`, `resolve` (+ ambiguity). `core/naming.rs` covers `parse_id_token` padding + `id_value`.

`ui_server.rs` drives `fastf::ui::route_request` directly (no socket). v0.6 core: health, preview-no-write, create makes the folder + is discovered, `/app.js` served, unknown route 404s. v0.7: search query language + path exclusion, project detail, tag add/remove, note, register, apply preview/create. v0.8: bundled-file reproduce (incl. byte-identical binary), background copy job, template file endpoints (list/save/add/delete + reserved/traversal guard), from-folder bundling. **v0.9:** create writes no `projects.jsonl` and shows via `/api/state`; discovery self-heals a deleted folder; `/api/reindex` rescans; the removed `/api/projects/prune` route now 404s.

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
- v0.9: the per-base `.fastf-index.json` cache is a *disposable accelerator*, never authority. All cache writes (`cache_upsert`/`refresh_cache`/`write_cache`) are best-effort and atomic — a failure never fails the command (folders are the truth). `library::max_id` MUST stay **read-only** (it's called from `plan()`/preview via the counter self-heal) — it uses `read_base_readonly` (fresh cache or scan, no write), NOT `discover` (which writes). If you route `max_id` through `discover`, previews start writing `.fastf-index.json` and the "preview writes nothing" tests fail.
- Post-create `commands` run synchronously through the user's shell (`cmd /c` on Windows, `sh -c` elsewhere). `{path}` is substituted before execution. There's no sandbox — template authors control this.
- `project_info::render()` builds frontmatter via `serde_yaml::to_string(&Metadata { ... })` — do NOT hand-format the YAML string. serde_yaml handles escaping of colons, quotes, multi-line values correctly; hand-formatting breaks on edge cases.
- v0.9: the `project_info_enabled` / `project_info_filename` (and legacy `pinfo_*`) config fields are **gone**. Metadata is mandatory and always `PROJECT_INFO.md`. Old configs that still carry those keys keep parsing because `Config` has no `deny_unknown_fields` — serde silently ignores them (there's a regression test). Don't re-add a filename knob; the reservation + discovery all assume the fixed name.
- `resolve_post_create()` in `project.rs` is `pub` — the open-prompt check in `cli/new.rs` calls it to avoid double-opening when `reveal: true` is already set in post_create.
- v0.4: `project_info::split_frontmatter_body()` is `pub` (not `pub(crate)`) so integration tests can assert byte-identity round-trips. The internal `extract_frontmatter` helper from v0.3 was folded into it — there's now one splitter.
- v0.4: `Predicate::Free` is the parser fallthrough, so any non-empty term that isn't `tag:`, `key=…`, `key>…`, `key<…` becomes a free-text predicate. Don't add another fallthrough below it (would be unreachable). Free terms search **case-insensitive substring** (not prefix) — keep it that way for grep-like UX.
- v0.4: `path` is intentionally NOT searched by `Predicate::Free`. There's a regression test (`free_does_not_match_path`) that asserts this; if you ever extend the field set, don't break that guarantee silently — home-dir leakage is a privacy footgun.
- v0.4: `cli::recent::run_picker` is `pub` because `cli::search` reuses it; `project_action_menu` stays private. If TUI/search both need a new picker action, add it inside `recent.rs`.
- v0.5: Bool flags after the slug used to silently drop. Fixed by `cli::new::classify_extra` — main.rs's New / Apply / Register arms all run their trailing `extra` Vec through it, then OR-combine the recognized flags into the relevant Args struct. Adding a new bool flag to `New`/`Apply`/`Register` requires updating `ExtraFlags`, the recognizer match in `classify_extra`, AND the OR-combine in each match arm — three coordinated edits. Forget the third and the flag works before the slug but mysteriously breaks after it.
- register writes PROJECT_INFO.md in two steps: `project_info::write(&plan, &tmpl, &tags)` (which uses `library::now_iso8601` inside `Metadata::from_plan`), then `project_info::write_frontmatter` to patch `created` to the resolved timestamp. Don't try to plumb the timestamp through `from_plan` — it'd break the byte-identity guarantee on the round-trip test and pollute the signature for a register-only concern.
- v0.5: `register` builds its `ProjectPlan` directly (pub struct fields) instead of calling `project::plan()` because plan always sets `root_path = cfg.base_dir.join(folder_name)`. Register's `root_path` is the canonical path of the existing folder. Don't refactor plan to take a path override — keep the two flows separate.
- v0.5: Without `--template`, register uses a `registered_stub_template()` (slug `"(registered)"`, `IdConfig::default()`). Recent and search will show these mixed with template-created projects. `project_info::render`'s "no variables" branch handles empty `tmpl.variables` correctly — don't add a special-case writer.
- v0.9: register no longer has an "already registered" dup-check (there's no index to consult). "Already a project" now means "the folder already has a `PROJECT_INFO.md`" — handled by `PinfoConflict` (Abort/Skip/Overwrite). `--recursive` pre-filters children that already have metadata and passes `Skip`; the CLI single-register resolves the policy from `--yes`/TTY prompts. `paths_equal` and the old duplicate bail are gone.
- v0.5: `sanitize_name` in `core/naming.rs` does NOT replace spaces — it only swaps filesystem-illegal chars (`/ \ : * ? " < > |`). For `fastf new`, the user-declared `transform` on each variable does space→underscore. Register's no-template path doesn't have a transform, so it uses `slugify_folder_name` (collapses whitespace runs to `_`, applies `sanitize_name`, preserves case). If you ever wire a no-template flow elsewhere, reach for `slugify_folder_name`, not `sanitize_name` alone.
- v0.5: `config::set "register-naming-pattern"` rejects patterns that don't contain `{id}`. This is a safety net — without `{id}`, registering multiple folders with the same `{name}` would all rename to the same target. Don't relax this check unless you've thought through the duplicate-rename UX.
- v0.5: `--apply` requires `--template` (still). `--rename` does not, as of v0.5 — it falls back to `cfg.register_naming_pattern`. If you add another "needs template" flag, encode the requirement in clap's `requires = "template"` AND in the defensive bail at the top of `register::run` (the public API can be called directly from tests, bypassing clap).
- `PROJECT_INFO.md` is reserved. `Template::load_from_file` and `save_to_file` both call `strip_reserved_files()` (which uses `project_info::path_is_reserved`). The check is root-only (leaf `==` reserved name, case-insensitive, AND no `/` in the normalised path) so `docs/PROJECT_INFO.md` is allowed. The reserved name is hardcoded to `RESERVED_FILENAME = "PROJECT_INFO.md"` — the safety net is independent of anything. `project_info::pinfo_path(dir)` is the one helper that builds `<dir>/PROJECT_INFO.md`; use it instead of hand-joining the filename.
- v0.9: the TUI Settings → "Project metadata" submenu (metadata toggle/filename) is gone — replaced by "Library bases" (`menu_settings_bases`), which add/removes entries in the `bases` config list via `config::set("bases", comma_joined)`.
- v0.5: Template builder's `collect_file()` example shows `NOTES.md`, not `PROJECT_INFO.md`. It also rejects the reserved name inline with a loop-back. If you change the example, keep `NOTES.md` (or another genuinely non-reserved name) — `PROJECT_INFO.md` as an example actively misleads users into creating template entries that get silently stripped.
- v0.5: Template builder no longer asks "Template vs Raw" content mode. `collect_file()` always writes to `FileEntry.template`. The `FileEntry.content` field still exists in the YAML schema (hand-written templates with raw byte content keep working — e.g. `music-video.yaml`'s `.gitignore`), but the builder never produces it. `create_file()` and `apply()` still pick `template` when non-empty else `content`, so the dual-field semantics are preserved at the writer. If you re-add a mode switch, remember that `interpolate()` is already a no-op on text without `{token}` markers, so the only real use-case for `content:` is preserving literal `{...}` braces.
- v0.5: The "Add another placeholder file?" prompt in `edit_files()` defaults to **No** and explicitly mentions that PROJECT_INFO.md is generated automatically. Don't flip the default back to Yes — the typical template doesn't need extra placeholder files and the auto-gen covers the common notes use-case.
- v0.8: **`files/` on disk is the source of truth for create/apply**, NOT `template.files`. `create`/`apply`/dry-run walk the dir via `core::assets`; `template.files` is a load-time scan of *text* files only, kept for the editors/previews/apply-var-detection. Building a `Template` in memory with `files` and calling `project::create` writes **nothing** unless those files are on disk under `files/` — this is why tests use a `write_template` helper that flushes an inline `files:` block onto disk before loading. Don't route create back through `template.files`.
- v0.8: `defer_over` in `copy_template_files` must stay ≥ `TEXT_MAX_BYTES` so every deferred file is verbatim (no interpolation needed off-thread). `create` passes `None` (copies all inline — CLI stays synchronous); only the UI's `create_deferred` passes `Some(JOB_DEFER_BYTES)`.
- v0.8: `Template.files`/`dir`/(text buffer) are `#[serde(skip)]`; `verbatim`/`exclude` use `skip_serializing_if = "Vec::is_empty"` so empty globs don't clutter every `template.yaml`. `load_from_file`/`save_to_file` take `&Path` (was `&PathBuf`) — `&PathBuf` still coerces.
- v0.8: `interp_rel` interpolates each path segment separately (via `interpolate_name`) so `__` collapse/trim happens *within* a name, never across `/`. Don't run `interpolate_name` on the whole slash-joined path.
- v0.8: Converting old flat `<slug>.yaml` templates → folder form: `template.yaml` (drop the `files:` block) + one real file per entry under `files/`. There is **no migrate command** and no flat-form fallback — an old flat `.yaml` sitting directly in `templates/` is simply ignored by `load_all` (it only reads subdirs with a `template.yaml`).
- v0.8 (phase 4): `cli::template::from_folder(source, slug, force, bundle_assets)` is the **non-interactive core** (returns `FromFolderReport`) called by the UI route and tests; `run_from_folder` is the **CLI shell** (interactive size-`Confirm` before bundling + colored summary). Don't call the interactive one from the UI/TUI-headless path — `run_from_folder`'s `Confirm` needs a TTY (piping stdin fails with "not a terminal", by design, like every other dialoguer prompt). The `scan_source` → `execute_scan` split exists so the CLI can confirm the total bundle size *before* any write; keep it. Text files (UTF-8 ≤ 64 KB) always reproduce as editable `FileEntry`s; only binary/large files are gated on `bundle_assets` (else counted `skipped`). Bundled assets are copied via `assets::copy_file(force_verbatim=true)` — stored raw, interpolated later at create time. Root `PROJECT_INFO.md` is skipped during scan (fastf owns it).
- v0.9: the counter self-heal lives in `project::plan()`: `counter_value = max(counters.get(), library::max_id(config)) + 1`, then `id_str`/`folder_name` derive from it. Doing it in `plan()` (not `create`) keeps `id_str`→`folder_name` consistent and makes preview show the true next ID. `create_inner` persists `plan.counter_value` and drops the old `index::append`, replacing it with `library::cache_upsert(abs_path.parent(), &project)` (base = the new folder's canonical parent, which matches `discover`'s canonicalized bases; even if `strip_prefix` fails the entry falls back to the basename = correct depth-1 `dir`).
- v0.9: `naming::parse_id_token(name, prefix)` (folder-name ID recovery, register-only) vs `naming::id_value(id)` (trailing-digit extraction for `max_id`, prefix-agnostic). `max_id` uses `id_value` because ids across templates have different prefixes; register uses `parse_id_token` with the template's prefix. Don't swap them.
- v0.9: discovery is **depth-1** (`SCAN_DEPTH` const in `library.rs`) — direct children of each base only. Matches the user's flat layouts. If you make it configurable, thread it through `scan_base`/`reindex`/`read_base_readonly`, not just `discover`.
- v0.9: `tag add/remove/reauto` (CLI + UI `project_tag`) call `library::refresh_cache(&project.path)` after mutating the frontmatter so the cache's tags stay fresh without a rescan. `note` doesn't (the cache stores no journal). `load_state`/`search_projects` also read metadata fresh per project for the `tags` field, so UI display is correct even if a cache is momentarily stale.
- v0.10: `Project.base` must be populated at BOTH construction points (`into_project` and `project_from_meta`) — adding a third constructor without threading the base will compile only if you remember the field, so any new `Project { .. }` literal needs a conscious `base:` decision (usually `path.parent()`). Do NOT add `base` to `CacheEntry` — base-relative portability is the point of the cache format.
- v0.10: `move_project` deliberately falls back to copy+remove on ANY `fs::rename` error (not just EXDEV) — errno matching is platform-fiddly and the fallback is safe (target-exists is pre-checked). The `copy_tree` copy is verbatim: never route a move through `copy_file`'s interpolation path.
- v0.10: move targets are validated against `effective_bases()` in the CALLERS (cli/move_project.rs, recent.rs action menu, ui project_move), not inside `library::move_project` — the core fn only guards is_dir/same-base/collision. If you add a new caller, do the configured-bases check there too, or the moved project may land somewhere discovery never looks.
- v0.10: in `project_action_menu` the Move/Back/Quit indices are dynamic (Move appears only when another mounted base exists) — they're handled by `if choice == move_idx/back_idx/quit_idx` guards ABOVE the numeric `match`. Adding a new fixed action means renumbering only the 0..=5 arms; the tail stays index-independent.
- v0.10: the UI create form's `#base-dir` is now a `<select>` — it fires `change`, not `input` (the preview listener was updated accordingly). The frontend `effectiveBases()`/`baseLabel()` helpers mirror `Config::effective_bases()`/`library::base_label` with trailing-slash-insensitive dedup; the server re-validates and canonicalizes on every move anyway.
- v0.11: `move_project_with` is the progress/cancel engine; `move_project` wraps it with throwaway handles (the CLI path stays synchronous but still verifies). The staged path is only reached when `fs::rename` fails — a same-fs test can't exercise it, so the unit tests call the private `staged_copy_verify_commit` directly. Don't route a move through `copy_file` (interpolation) — moves are ALWAYS verbatim (`jobs_for_tree` + `copy_job`, or `copy_tree`).
- v0.11: `assets::verify_tree` compares `{rel → size}` sets and MUST ignore transient scaffolding (`is_transient`: cache, `.fastf-provisioning.json`, `.fastf-move-*`, `*.part`) on BOTH sides — otherwise verifying a project that itself carries a stale marker fails spuriously. Verification is size + count + existence by design (not hashing — that doubles I/O over network); if you ever add hashing, make it opt-in.
- v0.11: the create marker is written in `spawn_copy_job` (UI) — the synchronous CLI `fastf new`/`project::create` copies everything inline and needs no marker. Don't add marker-writing to `create_inner`; only the deferred (UI) path can be interrupted with work outstanding.
- v0.11: reconcile decides a move's fate purely by "does `final` exist?" — if yes the commit happened (finish source removal), else roll back (discard staging, source intact). It never resumes a half-copy (simpler + safe). Keep the commit as the single atomic `rename(temp,final)` so this stays a clean boolean.
- v0.11: `POST /api/project/move` runs OFF `WRITE_LOCK` (background thread) so a slow network copy can't block other UI writes — mirroring the create copy job. The two base-cache writes it does are atomic + best-effort (last-writer-wins, self-heals via the staleness gate), so not holding the global lock is safe. Don't re-add `lock_writes()` to that arm.
- v0.11: `scan_base` skips dot-prefixed dirs — required so a staged move's `.<folder>.fastf-part` (which contains a full copy incl. `PROJECT_INFO.md`) isn't discovered as a duplicate mid-move. Real projects are never dot-prefixed, so this is free.
- v0.11: `JOBS` map value is `JobHandle { progress, cancel }` (was a bare `Arc<Mutex<Progress>>`). `register_job` evicts finished handles on each new job; a `/api/job/<id>` 404 still means "done" to the frontend. `Progress` gained `phase` — set it alongside `status` at terminal states.
- v0.11 (UI polish): the Projects table supports **multi-select bulk move** — a `.project-select` checkbox per row + a select-all header checkbox feed `state.selected` (a `Set` of paths); `bulkBar()` renders the toolbar (target-base `<select>` + `Move N`). `runBulkMove` moves them **sequentially** (one job at a time, never racing base caches) via `pollMoveJob`, skipping projects already in the target. The `.project-row` grid gained a leading `22px` checkbox column (header + rows must both add the `.project-select` cell or the grid misaligns). The row click handler skips `.project-select` so ticking a box doesn't open the drawer. `bindProjectBulk()` (toolbar/select-all) is bound in `bindCommon` only — NOT in `runProjectSearch`'s in-place rebind — to avoid double-binding page-level controls. Move overlays wrap long names via `overflow-wrap: anywhere` on `.success-modal h2/p` + `.job-label`.

- v1.0: `paths::install_dir()` must stay infallible-looking but non-panicking. Never re-add `.expect` there; never memoize the resolution (tests swap `FASTF_INSTALL_DIR` within one process). Portable detection keys on `config.toml`/`templates/` NEXT TO THE EXE — a bare binary copied without them lands in the user config dir by design (first-run banner names the mode).
- v1.0: `fastf completions` and `fastf mangen` skip `ensure_bootstrapped()` (see the matches! guard in main.rs's `run`). PKGBUILD/release workflows run the built binary for completions + man pages inside packaging sandboxes — bootstrap there would write into the builder's $HOME. Keep any future "no side effects" subcommand in that guard.
- v1.0.1 (docs overhaul, 2026-07-16): README is compact (~150 lines, hero + quick start + features + install + docs links); the deep material lives in `docs/` — `cli.md` (command reference + recipes), `templates.md` (authoring), `projects.md` (PROJECT_INFO.md/discovery/moves/reconcile), `windows.md` (MSI + PATH), `UI.md` (dev/API). When features change, update the matching docs/ file, not the README. Style rule from cristoc: minimal em dashes and comma chains in user-facing docs.
- v1.0.1: **Windows MSI** — `packaging/wix/main.wxs` (WiX v5 authoring; built in release.yml's windows leg via `dotnet tool install --global wix` + `wix build`). Installs fastf.exe to Program Files, appends INSTALLFOLDER to the system PATH (removed on uninstall), LICENSE included. The `UpgradeCode` GUID is permanent — never regenerate it. MSI version must be numeric (tag with the `v` stripped; dev dispatch runs use 0.0.0). The MSI lands in the release assets + SHA256SUMS automatically via the `fastf-*` globs.
- v1.0: release/packaging live in `.github/workflows/{ci,release}.yml` and `packaging/` (fastf.desktop, icons/ extracted from the official icon.ico, aur/fast-folder + aur/fast-folder-bin + update.sh + PUBLISHING.md). AUR pkgname is `fast-folder` (NOT fastf — fastfetch confusion), the installed command stays `fastf`. Release archives bundle completions + man + desktop + icons; the -bin PKGBUILD installs straight from the musl archive. NO macOS builds — cristoc can't test them.
- v1.0.1: **`fastf ui --app` ties the server's lifetime to the app window** — `cli::ui::run` serves on a background thread, `child.wait()`s the spawned Chromium process, then exits (after draining `ui::jobs_active()` so an in-flight copy is never stranded). Closing the window fully stops fastf; the next launcher click starts fresh. Only the app-window path does this — terminal `fastf ui`, `--no-open`, and the default-browser fallback still serve until Ctrl-C (a browser tab can't be waited on). `open_app_window` returns the `Child` for this; `open_browser` (already-running path) still spawn-and-drops.
- v1.0.1: `write_response` assembles the whole HTTP response and sends it in ONE `write_all` — `write!` straight to a `TcpStream` is unbuffered, so each format fragment became its own TCP segment and `health_check`'s single read could land mid-status-line ("HTTP/1.1 " = 9 bytes), flakily reporting a live server as dead (the launcher then tried to re-bind the busy port and died — the "sometimes the icon does nothing" bug). `health_check` also now loops its read until the 12-byte status prefix is complete. Don't revert either side to single-read/multi-write.
- v1.0.1: `ui::paths_match` canonicalizes both sides when the separator-normalized string compare misses — on Windows one folder arrives spelled multiple ways (`\\?\` verbatim canonical from discovery vs 8.3 short names like `RUNNER~1` from the frontend/tests). String-only comparison broke unregister/delete/rename on Windows CI.
- v1.0: frontend dialogs — `confirmModal()`/`promptModal()` (app.js) are promise-based and replace native `confirm()`/`prompt()` (which look alien in --app windows). `closeModal()` resolves confirm=false / prompt=null, so Escape/scrim always answer "no". The typed-phrase input toggles the danger button **in place** (no render — rendering would drop focus).
- v1.0: `render()` saves/restores the focused element by **id** + caret. New interactive inputs must have a stable `id` to survive re-renders. The offline banner (`setOffline`) and job overlays are managed imperatively outside `render()` on purpose.
- v1.0: `pollMoveJob(jobId, onUpdate, isAlive)` — pass `() => document.body.contains(overlay)` from any overlay-scoped caller, or a navigation mid-poll leaks the loop.

## Browser UI (`fastf ui`, v0.7 + v0.8 jobs)

`fastf ui` starts a local loopback HTTP server and opens the browser UI. It is
part of the `fastf` binary — **no separate `fastf-ui-server` binary**, no
external web directory. Full reference: `docs/UI.md`.

**v0.7 — feature-parity pass.** The UI now reaches the v0.4–v0.5 surface that was
CLI-only: a project **detail drawer** (variables table + tag add/remove + journal
notes, opened from any project row), real **search** using `core::query` (the
`/api/search` route), a **register** page for onboarding existing folders, an
**apply** modal (preview then create-missing), template **generate-from-folder**,
and Settings **ID-counter editor**. Every one of those maps to an existing
`pub` library function, so the work was endpoint wiring + frontend views plus one
refactor: `cli::register::register_core` / `RegisterOptions` / `PinfoConflict`
(the non-interactive engine the route calls). **v0.9** rewired the read routes to
`library::discover` (no `projects.jsonl`), removed `/api/projects/prune` + its
frontend button, added `POST /api/reindex` (+ a "Reindex library" button) and a
**Library bases** editor in Settings (a `bases` textarea; `set_config` accepts a
`bases` array). `project_json` now takes a `library::Project`.

**v0.8 (phase 2) — background copy jobs.** `POST /api/create` does the fast work
(structure + text/small files + counter + PROJECT_INFO.md + cache) synchronously
and returns `{ project, job_id }`; files over `assets::JOB_DEFER_BYTES` (4 MiB) are
copied on a background thread (via `project::create_deferred` → `spawn_copy_job`),
**outside `WRITE_LOCK`** — the copy only touches the new project's own folder.
`GET /api/job/<id>` returns `assets::Progress`; a missing id (evicted after done)
is a clean error the frontend treats as complete. The success modal shows a live
progress bar polled ~500 ms; "Open folder" works immediately. `job_id` is `null`
when nothing needed deferring. **v0.8 removed the `/api/templates/import` and
`/api/templates/export` routes + their frontend** (templates are folders now —
share by copying the folder).

**v0.8 (phase 3) — template file ingestion + editor.** The template editor's
Files section now works directly on the `files/` subtree on disk (since
`Template.files` is `#[serde(skip)]`, `/api/state` can't ship the file list — the
editor fetches it live). Four routes: `GET /api/template-files?slug=` lists the
subtree (`{path, size, is_text, content}` — text files ≤ `TEXT_MAX_BYTES` carry
content for in-place editing, binaries report size only, dirs omitted);
`POST /api/templates/file-save` `{slug, path, content}` writes/updates a UTF-8
text file (empty content = placeholder); `POST /api/templates/file-add`
`{slug, src, dest}` copies a file from a disk path into `files/` via
`assets::copy_file(.., force_verbatim=true, ..)` (byte-identical; the local-first
ingestion path — no large upload through the browser); `POST
/api/templates/file-delete` `{slug, path}` removes one. All three writes go
through `normalize_template_rel` (forward-slash, `ensure_relative_safe_path`,
reject reserved `PROJECT_INFO.md`) and `require_template_exists` (files ops need
the template saved first — `files/` on disk is the source of truth, so there's no
in-memory buffer to flush). These are **independent of `templates/save`**, which
still writes metadata only (a UI metadata save sends an empty `files` buffer → no
`files/` write). `verbatim`/`exclude` globs are ordinary metadata edited in the
automation section and saved via `templates/save`. `/api/state` injects a computed
`file_count` per template (via `assets::walk`) for the cards/nav. Frontend:
`state.templateFiles` holds the fetched list; `loadTemplateFiles(slug)` fetches +
re-renders; each op (`createTemplateFile` / `addTemplateFileFromPath` /
`deleteTemplateFile` / blur-`saveTemplateFileContent`) hits its endpoint then
reloads the list.

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
`Counters`, `template`, `library` discovery, `post_create`), so the UI and CLI
share one source of truth and the same on-disk files. The `Ui` arm in `main.rs`
forwards to `cli::ui::run`. `Response` derives `Debug` (tests `unwrap_err` on the router).

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
- v0.7/v0.8: Path/query GET routes (`/api/project?path=`, `/api/job/<id>`) are
  matched with `if path.starts_with(...)` guards placed BEFORE the static-asset
  catch-all (`("GET", path) if !path.starts_with("/api/")`). Order matters — a new
  `/api/...` GET route must go above the catch-all. Query values are percent-decoded by
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
- v0.7/v0.9: `PinfoConflict` controls the existing-`PROJECT_INFO.md` policy. `Abort`
  bails BEFORE any write (so the UI can confirm + retry with `overwrite:true`); `Skip`
  keeps the existing file (no rewrite, no cache_upsert — used by `--recursive`);
  `Overwrite` rewrites it. The UI sends `Abort` unless the user confirmed overwrite;
  the CLI single-register sends `Overwrite`/`Skip` from its prompt.
- v0.7: The UI `apply` routes pass **raw** variables to `project::apply_plan` /
  `apply` (no transform/sanitize) — matching `fastf apply`'s semantics, NOT `new`'s.
  Don't "fix" this to apply transforms; it would diverge the UI from the CLI apply.
- v0.7: `/api/search` reuses `core::query` exactly, so the deliberate "path excluded
  from free-text" guarantee holds (there's a regression test). Empty `terms` returns
  all projects (newest first) — the server-side equivalent of the plain list.
- v0.7: The frontend renders the drawer + apply modal + generic (from-folder)
  modal as state-driven layers appended after `shell(content)` in `render()`; each
  has a `bind*` call. Mutating actions re-fetch + full `render()` (acceptable — the
  only focus-sensitive surface is the projects search, which updates `#project-results`
  in place via `runProjectSearch` instead of re-rendering). `(registered)`-slug
  projects are filtered out of the apply/register template pickers.
- v0.8: The `JOBS` registry is `static Mutex<Option<HashMap<..>>>` (const-init'd to
  `None`, lazily filled) keyed by `job-<n>` (`AtomicUsize`). The copy thread updates
  an `Arc<Mutex<Progress>>`; `spawn_copy_job` evicts finished jobs on each new create
  so the map stays bounded, so the frontend `pollJob` treats a `/api/job` 404 as done.
  The background copy must NOT take `WRITE_LOCK` (it only writes inside the new
  project's folder) — holding it would serialize a 200 MB copy against every other UI
  write, defeating the point. Deferred files are always verbatim (threshold ≥ text cap),
  so `copy_job` never needs vars.
- v0.8: `showSuccess(result)` (frontend) now takes the whole create response (not just
  `result.project`) so it can read `job_id`. The progress bar updates only the
  `#job-progress` node inside the imperatively-built success overlay — no `render()`.
- v0.8 (phase 3): The template editor's **Files section is live-on-disk**, NOT part
  of the `templateEditor` object. Never re-add a `files` array to `newTemplateDraft`
  / the save payload — `Template.files` is `#[serde(skip)]`, so it round-trips to
  nothing; a metadata save deliberately doesn't touch `files/`. File CRUD uses the
  four dedicated endpoints and re-fetches `state.templateFiles`. Because they act on
  disk, they need the on-disk slug (`state.templateOriginalSlug`, not the edited
  slug) and the template to exist — new templates show a "save first" notice.
- v0.8 (phase 3): file writes reuse `assets::copy_file(force_verbatim=true)` for
  ingestion (byte copy, atomic `.part`+rename) and `normalize_template_rel` for the
  traversal/reserved guard. Don't interpolate at ingestion time — a template asset
  is stored raw and only interpolated at project-create time. `list_template_files`
  omits directories (empty dirs are the `structure:` section's job).
- v0.9: `load_state`/`search_projects`/`project_detail` all source from
  `library::discover(&config)` (no `index::load_all`). They still read each project's
  metadata fresh for the `tags` field so display stays correct even against a
  momentarily-stale cache. `project_json(project: &Project, metadata)` takes a
  discovered `Project`. `/api/reindex` (write route, takes `lock_writes`) calls
  `library::reindex`; `set_config` accepts a `bases` array (trimmed, non-empty).
  The `/api/projects/prune` route + `prune_projects` fn are **gone** (the cache
  self-heals) — a POST to it 404s (regression test).
