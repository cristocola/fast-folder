# CLAUDE.md — fastf development context

## What this project is

`fastf` (Fast Folder Creator) is a Rust CLI tool for creating structured project folders from **folder templates**. Universal use cases: code, research, finance, music video, photography, and film production workflows. Config, templates, and the counter live together in one data dir, resolved in three tiers (env override → portable-next-to-binary → user config dir) — see "Data-dir resolution (v1.0)".

## Build commands

Standard cargo throughout (`build`, `test`, `fmt`); clippy must be clean with
`--all-targets -- -D warnings`. The non-obvious parts:

```bash
# Cross-compile for Windows (from Linux)
cargo build --release --target x86_64-pc-windows-gnu
# Requires: rustup target add x86_64-pc-windows-gnu + mingw-w64-gcc (pacman)

# Cross-compile for Linux (from Windows) — static musl
cargo build --release --target x86_64-unknown-linux-musl

# Fault injection — trip a named boundary deterministically:
FASTF_FAULT=create:mid-copy cargo test            # returns an error there
FASTF_FAULT=move:before-commit-rename:abort ...   # kills the process there
# See util::faults::ALL_FAULT_POINTS for the list.

# Browser UI — same `cargo build` (server + embedded frontend live in the lib)
FASTF_UI_DIR=src/ui/web cargo run -- ui   # frontend live-reload (assets from disk)
node --check src/ui/web/app.js            # frontend sanity check — run after every edit
```

Debug and release test counts differ because failpoint tests are
`#[cfg(debug_assertions)]`, several Windows semantics cases run only on Windows,
and the pty suites are Unix-only. Do not hard-code a total here; run the complete
gates in [`ROADMAP.md`](ROADMAP.md).

## Project layout

`ls` and the module names cover the shape; this is only what a filename doesn't
tell you. Deep per-file notes are gone on purpose — read the module, then the
Gotchas sections below for the parts that bite.

- `src/lib.rs` exposes `core/ cli/ tui/ ui/ util/ bootstrap/` so integration tests
  can `use fastf::…`. `src/main.rs` is the clap binary; its New / Apply / Register
  arms run their trailing `extra` Vec through `cli::extra::classify_extra` and then
  each command's own `apply_extra`.
- `src/bin/fastf-ui.rs` — second `[[bin]]`, a windowless Windows launcher shim over
  `cli::ui::run(app: true)`. **Not a second server**; the server lives only in `src/ui/`.
- `src/bootstrap.rs` — first-run setup. Ships two deliberately universal templates
  (`general`, `client-project`); domain templates live in `examples/templates/` and
  are NOT bundled.
- `src/core/` — the library proper. `library.rs` (filesystem-as-truth discovery),
  `operations.rs` (shared mutation boundary), `project.rs` (plan/create/apply),
  `transactions.rs` (v2 staged moves), `provisioning.rs` (v2 recovery plus
  report-only pre-v2 discovery), `template_import.rs` (from-folder engine),
  `assets.rs` (the template-file copy engine: walk, classify, interpolate or
  byte-copy — moves live in `transactions.rs`), `validated.rs` (typed slugs/relative
  paths), `project_info.rs`
  (`PROJECT_INFO.md` read/write/mutate).
- `src/util/` — all v1.1 hardening primitives: `lockfile` (cross-process `DataLock`),
  `atomic` (THE atomic write), `fs_retry` (Windows sharing violations), `interrupt`
  (Ctrl-C rollback), `faults` (failpoints, compiled out of release), `paths`
  (`install_dir` resolution, `display_path`, and the shared `require_real_file` /
  `require_native_relative` boundary checks), `tree_size` (all-or-nothing,
  non-following logical-byte snapshots), plus the two live-size pieces:
  `size_scan` (background sizing workers) and `live_select` (the only picker in
  the codebase that is not `dialoguer::Select`).
- `src/cli/` — one module per subcommand. `move_project.rs` is named that way
  because `move` is a keyword. CLI modules gather prompts and render outcomes;
  noninteractive register/from-folder behavior lives under `core/` so UI code
  never depends on a CLI implementation.
- `src/tui/` — `menu.rs` carries the interactive menu and the grouped Settings submenus.
- `src/ui/` — browser UI. Has its own CLAUDE.md.
- `docs/` — the user-facing reference.
  **When features change, update the matching `docs/` file, not the README.**
- `examples/templates/` — the gallery, in folder form. Not bundled; copy one into
  your templates dir to use it.
- `packaging/` + `.github/workflows/` — release machinery. See the `release` skill.
  Release automation must not mutate installed packages or run system upgrades;
  the maintainer installs and smoke-tests the released package manually.
- `tests/` — integration binaries; see `tests/CLAUDE.md` for what each one
  guards and the shared harness rules.

## The repository is public

Nothing tracked here may describe the machine it was written on: no real home
directory (`/home/<name>`, `C:\Users\<name>`), no personal mount point, no
local project-folder path, and no maintainer's name in prose. Write
`/home/user`, `/mnt/projects/...`, and "the maintainer" instead. Attribution is
the exception and belongs in `LICENSE`, `Cargo.toml`, `README.md`, the
PKGBUILDs, and the installer, where it is expected.

`tests/repo_hygiene.rs` enforces this over `git ls-files`, so the rule fails the
build rather than relying on anyone remembering it. It runs only when the crate
directory is the root of the checkout, because the AUR source package builds an
unpacked tarball inside an ignored directory of a real clone, where `git` answers
about the wrong tree. Before tracking a file that was written as a private note,
read it as a stranger would.

## Key design decisions

### Data-dir resolution (v1.0)
`paths::try_install_dir() -> Result<(PathBuf, DirMode)>` resolves the one data dir (config + templates + counters) with three tiers: (1) `FASTF_INSTALL_DIR` env (tests + power users), (2) **portable mode** — the exe's canonicalized parent iff it contains `config.toml` or `templates/` (keeps binary-plus-data folders working: `target/release/`, USB sticks), (3) **user config dir** — `$XDG_CONFIG_HOME/fastf` / `~/.config/fastf` (unix) or `%APPDATA%\fastf` (Windows), hand-rolled (no `dirs` crate). Tier 3 is what makes a package-manager install to read-only `/usr/bin` work: `ensure_bootstrapped()` lazily `create_dir_all`s the dir on first run. `install_dir() -> PathBuf` keeps its infallible signature (~30 call sites untouched) but never panics — it exits(2) with an actionable message on the unreachable error path; `main.rs` calls `try_install_dir()?` first thing so real users get a pretty anyhow error instead. No memoization (tests swap the env var in-process). Surfaced via `fastf paths` (cli/paths_cmd.rs), a mode line in `config show`, and `dir_mode` in `/api/state`. Bootstrap is **skipped** for `completions`/`mangen` so packaging steps never write to $HOME. (Projects themselves live in the bases; each base carries its own `.fastf-index.json` cache — v0.9.)

### Unregister / delete / rename (v1.0)
Three compatibility `library` fns guard direct-child + real matching metadata;
application interfaces call their `*_configured` variants, which acquire
`DataLock`, reload config, and revalidate membership/identity before mutation.
`unregister` removes only `PROJECT_INFO.md` + cache entry (files stay); `delete`
then removes the tree; `rename` sanitizes, same-parent renames, patches display
metadata best-effort, and refreshes caches. UI delete still re-checks the typed
folder name. TUI tail action indices remain dynamic.

### Cross-platform paths
Folder paths in templates (structure names, file paths) always use `/` as the separator in YAML — Rust's `PathBuf::join()` handles conversion to `\` on Windows at runtime. Users should always enter `/` in templates and `base-dir` config, though Windows also accepts backslashes in config values.

### Global ID counter (v1.2: lives in the base — v1.2.1: converges across bases)
One counter for all templates, `global = 47`, stored as **`<base>/.fastf-counter.toml`** next to that base's `.fastf-index.json`. IDs stay unique across all project types.

It moved out of the data directory because of where it must be *readable* from. `%APPDATA%\fastf` on Windows and `~/.config/fastf` on Linux are different files, so a dual-boot machine had two counters and the only workaround was symlinking one home into the other — which breaks the moment either home is encrypted. The projects never had that problem: they already sit on a drive both systems mount, so the number that indexes them now sits there too.

`Counters::floor(cfg)` takes the max of three inputs:
1. every mounted base's `.fastf-counter.toml` — shared across operating systems, which is what removed the symlink;
2. this machine's data-dir `counters.toml` — spans **every base it has written to**, which is what stops an unplugged drive restarting numbering. Dropping this was a real regression: work in an archive base to ID0005, unplug it, create elsewhere → ID0001, and reconnecting gives two projects the same ID. Guarded by `unplugging_a_base_does_not_restart_numbering`;
3. `library::max_id(cfg)`, the highest ID actually in project metadata — which is why losing a counter file is untidy rather than harmful.

**The number only ever goes up, and every base converges on it.** `Counters::record` writes the target base and then `propagate`s to every other mounted base plus the data dir; `Counters::converge(cfg)` recomputes the full floor and pushes it out (`fastf id sync`, and `fastf id show`, which repairs as it displays). Three bases holding ID0004 / ID0082 / ID0017 all come out at 82. The comparison is global, never per base: `floor` takes the single highest value across all three inputs and `propagate` raises every base to it, which is what carries the number to a machine that cannot see the other drives. A counter file that is higher than the projects around it is therefore never overwritten downward — nothing anywhere is.

Propagating on **every create** is load-bearing, not tidiness: if Linux mints ID0101 in a base Windows cannot see, the base Windows *can* see has to learn about it now — there is no later.

`Counters::save_base` is monotonic and returns **whether it wrote**, so `propagate` can call `library::touch_cache(base)` only for bases it actually touched. That re-stamp matters: writing into a base bumps its directory mtime, which `cache_is_stale` reads as "a project appeared", so without it every create would force a full rescan of every base. The counter write provably changes no project, so saying the cache is still good is honest — and safe because fastf's own writers are serialized by `DataLock`. Guarded by `propagating_the_counter_does_not_invalidate_other_bases_caches`.

**`Counters::next_value(cfg, counters)` is the one expression for "which ID comes next"** — `project::plan`, `operations::register`, and register's rename preview all call it. They did not before, and the preview confirmed `..._ID0001` while the commit wrote `..._ID0011`.

**There is no way to lower it, by construction.** `fastf id set` refuses anything at or below the floor and names what is holding it; `fastf id reset` is gone (kept as a hidden subcommand that explains itself and points at `id sync`); `POST /api/counter` applies the same rule. v1.2.0 let all three *report success* for a write that `floor` then ignored.

**Known limit, unchanged:** `DataLock` is per data-directory, so two *different machines* writing one shared base are not serialized and can mint the same number. Same-machine concurrency is safe (`tests/concurrency.rs`).

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
`validated::TemplateSlug` accepts one ASCII alphanumeric/`-`/`_` component.
`validated::SafeRelativePath` normalizes slash styles and rejects empty/dot
components, absolute/drive paths, and `..`; safe nested paths such as
`src/components` remain valid. `naming::ensure_relative_safe_path` is only a
compatibility delegate.

Validation happens before path derivation at template lookup/save and on raw
`files` plus `structure` entries. `project::plan` then validates every physical
and structure path again **after interpolation**, before claiming a folder;
create/apply/UI file routes repeat the typed boundary check. Base overrides use
`config::resolve_base_dir_input` (`~` expansion + absolute-only + canonicalize),
including CLI `new`, UI preview/create, and UI settings.

### Filesystem-as-truth library (`core/library.rs`, v0.9)
There is **no `projects.jsonl`**. The project list is discovered from the filesystem: a folder is a project iff it holds a `PROJECT_INFO.md`, whose frontmatter `id` is authoritative (folder name is cosmetic, never consulted for discovery). `discover(cfg)` unions `cfg.effective_bases()` (`base_dir` + config `bases`), newest-first.

Each base carries a **disposable** `.fastf-index.json` cache at its root, co-located with the projects so it travels with them and is portable (entries store a base-relative `dir`, valid across `/mnt/…` and `D:\…`). The cache is never authoritative — `discover_base` self-heals: if the base's mtime is newer than the cache (or either can't be stat'd) it rescans + rewrites; otherwise it trusts cached metadata but existence-checks each entry and drops (rewriting away) any whose folder disappeared. **No manual prune, ever** — the "missing" state is transient. `fastf reindex` forces a full rescan for external edits fastf can't observe.

`max_id(cfg)` is **read-only** (reads a fresh cache or scans, never writes) so it's safe to call from `plan()`/preview. `resolve(cfg, query)` replaces the old `index::resolve_project` (exact-id → id-prefix → name-substring). `cache_upsert`/`refresh_cache` keep the cache fresh after create / tag mutations without a rescan. All cache writes are best-effort and atomic (an exclusively created unique sibling + rename); a cache error never fails a command. Counter self-heals: `plan()` computes the ID from `max(counters.get(), library::max_id(cfg)) + 1`.

### Live project sizes

`util::tree_size::directory_size` is the one shared walker for guided-TUI and
web sizes. It sums regular-file logical lengths recursively (hidden files and
`PROJECT_INFO.md` included), never follows symlinks/junctions, ignores special
nodes, uses checked addition, and returns `None` on any read failure rather than
a partial number. It is crate-internal and read-only. `directory_size_until`
adds one thing: a cancel token checked once per directory entry, so teardown is
bounded on a share. **A cancelled walk also returns `None`**, so a caller that
cancels must discard the result rather than record it as `unavailable` —
`size_scan`'s worker is the only caller and does exactly that.

Sizes never enter `Project`, `.fastf-index.json`, project metadata, or
`/api/state`. The browser endpoint authorizes paths via fresh discovery and
walks without `WRITE_LOCK`; its frontend queue runs at most two scans,
prioritizes an open drawer, and drops old-generation responses after state
refresh.

**Nothing blocks on a size.** The guided browser draws its list first and shows
`scanning…` in a fixed-width cell until a snapshot lands. `util::size_scan`
owns two workers over one queue; `request` **replaces** that queue with the
visible page, selected row first, so turning the page or moving the selection
reprioritizes at once instead of finishing work nobody is looking at. Snapshots
live in the scanner for one browser session and die with it; a mutation calls
`forget` so the row is measured again. Standalone `fastf recent` and
`fastf search` are untouched — no size column, no new output for scripts.

The list repaints itself because `util::live_select` owns the key loop:
`dialoguer::Select` cannot, since `Term::read_key` has no timeout and a
read/write-pair `Term` reports `is_term() == false`. The key is read on a
throwaway thread and collected with `recv_timeout`, which makes the *wait*
interruptible without the *read* being so. Everything else in the TUI stays on
`dialoguer`, and `live_select` matches it key for key on purpose.

### Contained provisioning + recovery v2 (v1.5.0/v1.5.1)

**Invariant:** a move source is never removed until a complete destination has
been copied, verified, and published. Every ordinary filename is payload;
`.tmp`, `.part`, cache-looking, and marker-looking names are never skipped. Copy
and verification roots must be existing real directories, unsupported links and
special nodes are rejected, and only EXDEV/ERROR_NOT_SAME_DEVICE enables the
copy fallback.

**Move transactions** (`core::transactions`) live exclusively below the target
base at `.fastf-transactions/<timestamp-pid-counter>/`. `move.json` contains only
version, operation id, project id, configured source base, validated source and
target folder components, and `Copying|ReadyToCommit|CleanupPending`; target and
staging paths are derived from the transaction's owned location. `manifest.json`
uses native relative `PathBuf`s and records file/directory type, byte length, and
source modification time. Copying streams regular files into `staging/` with one
bounded buffer, creates empty directories, verifies exact path/type/size, then
rescans source metadata before publication. Target occupancy is rechecked with
`symlink_metadata`.

`library::move_project() -> Result<Project>` remains the compatibility wrapper;
application callers use `MoveOutcome { project, cleanup_pending }`. Same-filesystem
moves are a direct rename with no journal. Before publication, cancellation or a
failure removes only the owned transaction and leaves source. After publication,
cancellation is too late; a source-removal failure retains `CleanupPending` and
reports the destination as published.

**Create v2** writes `.fastf-create-v2.json` with only the validated template
slug and template/project-relative deferred-copy paths. A completed create clears
the metadata provisioning flag before removing the journal. An empty initial
journal cannot prove which inline interpolated files landed, so it is reported
for inspection; a deferred journal can resume missing files after identity,
type, and length checks.

**Reconcile** holds `DataLock` for the whole pass. `Copying` and an unpublished
`ReadyToCommit` discard only the owned transaction. A published
`ReadyToCommit` compares project identity and saved source/final manifests before
entering cleanup; `CleanupPending` repeats those checks before source removal.
Missing configured bases, malformed journals, identity mismatches, and unknown
states are report-only. The pass is idempotent.

**Pre-v2 markers** contain arbitrary absolute paths and are never read as
authority. `list_incomplete` identifies their own paths and reconcile reports
them as `obsolete` without parsing, migrating, following, copying, deleting, or
suffix-sweeping. Never resurrect v1 JSON migration.

**Application/UI:** `core::operations` is the shared mutation entry point.
Background create/move jobs run off UI `WRITE_LOCK` but retain cross-process
`DataLock` through completion; reads and cancellation remain available.
`Progress` keeps existing fields and adds optional `cleanup_pending`/`warning`.
Post-create commands, editors, and reveals run only after provisioning and after
the mutation lock is released.

### Base-aware projects (v0.10)

Every discovered `Project` carries `base: PathBuf` — the effective base it was
found under, set at the two construction points (`CacheEntry::into_project(base)`
and `project_from_meta(meta, base, dir)`). The cache format is **unchanged**
(no `base` field in `CacheEntry` — the base is implicit: it's whichever base's
`.fastf-index.json` the entry lives in, which is what keeps caches portable).
`library::base_label(base)` renders the short display name (last path
component) used by all list surfaces.

**`library::move_project(project, new_base) -> Result<Project>`** keeps the
historical compatibility shape. The current implementation validates an
unoccupied configured target, tries `fs::rename`, and falls back to the scoped
v2 transaction engine only for EXDEV/ERROR_NOT_SAME_DEVICE. Permission, sharing,
missing-path, and all other rename failures return unchanged. Contents are
always verbatim; metadata and both disposable base caches are refreshed after a
completed move.

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

**Atomic mutation** (v0.4): `write_frontmatter(path, |meta| { ... })` reads → splits → parses → applies the closure → re-serializes → writes via `.tmp` + rename. Body **and frontmatter** bytes are byte-identical after a no-op mutation — one integration test each.

**Unknown keys survive every mutation** (v1.6.1). The re-serialize step is `util::yaml::to_string_preserving_unknown(&meta, frontmatter, Metadata::OWNED_KEYS)`, not `serde_yaml::to_string`. It merges the fresh struct onto the parsed `serde_yaml::Mapping`, which is an `IndexMap`, so a key fastf has no field for keeps its **position**, not just its value. `OWNED_KEYS` is what distinguishes "ours and no longer emitted, so remove it" (`provisioning` once a create finishes) from "not ours, leave it"; `owned_keys_covers_every_serialized_field` fails if a field is added without updating the list.

**Do not replace this with `#[serde(flatten)]`.** It was the obvious design and it is wrong: `flatten` routes every field through serde's `Content` buffer, so a plain unquoted scalar in a hand-edited file (`year: 2026`) arrives as an integer and the `String` field rejects it — and `library::read_project_meta` drops that error, so the project disappears from discovery. Preserving unknown keys must not cost a new way to lose a project. Verified in `serde-1.0.229/src/private/de.rs:1255` and `serde_yaml-0.9.34/src/de.rs:1472`. `append_journal_entry(path, msg)` does the same atomic dance for the body. Both require frontmatter to exist; otherwise return a structured error naming the path.

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

`print_dry_run` and `print_apply_plan` take a `PreviewKind`: `DryRun` for `--dry-run` ("nothing will be created"), `BeforeCommit` for the plan printed immediately before the real create/apply ("Preview"). Both are called on both paths — a new caller has to say which side of the commit it is on, because printing the dry-run header over a real create is the defect this replaced.

## Gotchas

- `dialoguer::Input::interact_text()` takes ownership of `self`. Never reuse an `Input` struct across iterations — recreate it each time.
- The `console` crate was removed in v0.2 (unused). Reach `dialoguer::console` through dialoguer instead of adding it back as a direct dep — see the v1.0.2 clamp gotcha.
- `Template` needs `#[derive(Default)]` because `build_template` calls `.unwrap_or_default()`.
- `Template::validate()` is `pub` (was private before v0.2). Used by the gallery-parse integration test.
- `Template::save_to_file()` no longer has `#[allow(dead_code)]` — it's reached by both the interactive builder and `from_folder`.
- Windows cross-compile requires pacman-installed `mingw-w64-gcc`, NOT rustup-managed Rust installed via pacman. Use rustup for the Rust toolchain: `sudo pacman -Rs rust && sudo pacman -S rustup mingw-w64-gcc && rustup default stable`.
- `IdConfig` no longer has an `auto_increment` field — it was defined but never read. If per-template ID disable is needed in the future, add it back and check it in `project::plan()`.
- `print_tree` is in `core/project.rs` (pub). Do not add a duplicate in `cli/template.rs` or `tui/template_builder.rs` — import it from `project`.
- **Naming pattern** in `project::plan()` uses `interpolate_name()` (collapses `__`, trims edges). **File content** in `assets::copy_file()`, `project::apply()`, and `print_file_previews()` uses `interpolate()` (raw, no collapse). Mixing them up will either break Python dunders in generated files OR leave dangling underscores in folder names.
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
- v1.6.1: **`cli::extra::classify_extra` reads the flag list from clap, not from a hand-written match.** `trailing_var_arg` means every token after the first one clap cannot parse lands in `extra`; the classifier sorts that bucket using `cmd.get_arguments()` for *that* subcommand (long, short, `get_action().takes_values()`). So the rule for adding a flag is two steps, not three: declare it in clap, handle it in that command's `apply_extra` (`cli::new`, `cli::apply`, `RegisterFlags`). The `_ =>` arm of each `apply_extra` bails by name, and `main.rs`'s `every_declared_flag_is_handled_after_the_positional` calls it with every declared long, so forgetting the second step fails the suite instead of making the flag work before the positional and silently do nothing after it. An undeclared `--key=value` is a template variable; anything else is an error (`warn_unknown` is gone — an ignored request is not an outcome).
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
- v0.5/v0.8: Template builder no longer asks "Template vs Raw" content mode. `collect_file()` always writes to `FileEntry.template`, and the writers still pick `template` when non-empty else `content`. **But `FileEntry.content` is no longer reachable from a `template.yaml`** — v0.8 made `Template.files` `#[serde(skip)]`, so a hand-written `files:` block (with `content:` or anything else) is silently ignored on load. The field survives only for the in-memory editor/preview buffer. **To preserve literal `{...}` braces in a file today, list it in the template's `verbatim` globs** — that is the supported mechanism, not `content:`.
- v0.5: The "Add another placeholder file?" prompt in `edit_files()` defaults to **No** and explicitly mentions that PROJECT_INFO.md is generated automatically. Don't flip the default back to Yes — the typical template doesn't need extra placeholder files and the auto-gen covers the common notes use-case.
- v0.8: **`files/` on disk is the source of truth for create/apply**, NOT `template.files`. `create`/`apply`/dry-run walk the dir via `core::assets`; `template.files` is a load-time scan of *text* files only, kept for the editors/previews/apply-var-detection. Building a `Template` in memory with `files` and calling `project::create` writes **nothing** unless those files are on disk under `files/` — this is why tests use a `write_template` helper that flushes an inline `files:` block onto disk before loading. Don't route create back through `template.files`.
- v0.8: `defer_over` in `copy_template_files` must stay ≥ `TEXT_MAX_BYTES` so every deferred file is verbatim (no interpolation needed off-thread). `create` passes `None` (copies all inline — CLI stays synchronous); only the UI's `create_deferred` passes `Some(JOB_DEFER_BYTES)`.
- v0.8: `Template.files`/`dir`/(text buffer) are `#[serde(skip)]`; `verbatim`/`exclude` use `skip_serializing_if = "Vec::is_empty"` so empty globs don't clutter every `template.yaml`. `load_from_file`/`save_to_file` take `&Path` (was `&PathBuf`) — `&PathBuf` still coerces.
- v0.8: `interp_rel` interpolates each path segment separately (via `interpolate_name`) so `__` collapse/trim happens *within* a name, never across `/`. Don't run `interpolate_name` on the whole slash-joined path.
- v0.8: Converting old flat `<slug>.yaml` templates → folder form: `template.yaml` (drop the `files:` block) + one real file per entry under `files/`. There is **no migrate command** and no flat-form fallback — an old flat `.yaml` sitting directly in `templates/` is simply ignored by `load_all` (it only reads subdirs with a `template.yaml`).
- v1.5.1: `core::template_import` is the noninteractive from-folder engine and
  `core::operations::template_from_folder` is the locked mutation entry point
  used by CLI and UI. The CLI may perform a read-only pre-scan for its bundle-size
  confirmation, but the operation rescans beneath `DataLock` before writing.
  Text files (UTF-8 ≤ 64 KB) become editable template files; binary/large files
  are bundled only when requested. Root `PROJECT_INFO.md` remains excluded.
- v0.9: the counter self-heal lives in `project::plan()`: `counter_value = max(counters.get(), library::max_id(config)) + 1`, then `id_str`/`folder_name` derive from it. Doing it in `plan()` (not `create`) keeps `id_str`→`folder_name` consistent and makes preview show the true next ID. `create_inner` persists `plan.counter_value` and drops the old `index::append`, replacing it with `library::cache_upsert(abs_path.parent(), &project)` (base = the new folder's canonical parent, which matches `discover`'s canonicalized bases; even if `strip_prefix` fails the entry falls back to the basename = correct depth-1 `dir`).
- v0.9: `naming::parse_id_token(name, prefix)` (folder-name ID recovery, register-only) vs `naming::id_value(id)` (trailing-digit extraction for `max_id`, prefix-agnostic). `max_id` uses `id_value` because ids across templates have different prefixes; register uses `parse_id_token` with the template's prefix. Don't swap them.
- v0.9: discovery is **depth-1** (`SCAN_DEPTH` const in `library.rs`) — direct children of each base only. Matches the user's flat layouts. If you make it configurable, thread it through `scan_base`/`reindex`/`read_base_readonly`, not just `discover`.
- v0.9: `tag add/remove/reauto` (CLI + UI `project_tag`) call `library::refresh_cache(&project.path)` after mutating the frontmatter so the cache's tags stay fresh without a rescan. `note` doesn't (the cache stores no journal). `load_state`/`search_projects` also read metadata fresh per project for the `tags` field, so UI display is correct even if a cache is momentarily stale.
- v0.10: `Project.base` must be populated at BOTH construction points (`into_project` and `project_from_meta`) — adding a third constructor without threading the base will compile only if you remember the field, so any new `Project { .. }` literal needs a conscious `base:` decision (usually `path.parent()`). Do NOT add `base` to `CacheEntry` — base-relative portability is the point of the cache format.
- v1.4.1: `move_project` falls back to copy **only** for Unix `EXDEV` or
  Windows `ERROR_NOT_SAME_DEVICE` (17). Permission, sharing, missing-path, and
  every other rename error returns unchanged. Never broaden that match.
- v1.4.1/v1.6.1: application callers use `move_project_configured_with_outcome`
  (through `operations::move_project`), which acquires `DataLock`, reloads
  config, and revalidates source identity/direct child plus target membership
  under the lock. `library::move_project` remains as the one compatibility
  shape: it takes the lock and revalidates the recorded project, but validates
  no configured base. The other three wrappers had no callers and are gone.
- v0.10: in `project_action_menu` the Move/Back/Quit indices are dynamic (Move appears only when another mounted base exists) — they're handled by `if choice == move_idx/back_idx/quit_idx` guards ABOVE the numeric `match`. Adding a new fixed action means renumbering only the 0..=5 arms; the tail stays index-independent.
- v0.10: the UI create form's `#base-dir` is now a `<select>` — it fires `change`, not `input` (the preview listener was updated accordingly). The frontend `effectiveBases()`/`baseLabel()` helpers mirror `Config::effective_bases()`/`library::base_label` with trailing-slash-insensitive dedup; the server re-validates and canonicalizes on every move anyway.
- v1.5.0: `staged_copy_verify_commit` is the staged move body and it runs on
  `core::transactions`; debug-only forced-staged tests reach it on one
  filesystem. Moves are verbatim. Every walked source name is payload, including
  `.tmp`, `.part`, `.fastf-index.json`, and marker-looking names; there is no
  suffix-based transient filter.
- v1.5.0: move manifests compare native relative path, entry type, and byte
  length, then rescan source path/type/size/mtime before publication. They do not
  hash or promise advanced filesystem metadata. Transaction staging is written
  directly, with no sibling `.part` convention.
- v1.5.0: **every** create path writes the v2 journal, CLI included —
  `create_inner` writes metadata first with `provisioning: true`, then an empty
  initial journal. Deferred creates replace it with relative copy jobs. Never
  regress to UI-only journaling.
- v1.4.1/v1.5.0: pre-v2 reconcile never infers authority from marker contents or
  destination names. It never deserializes or mutates through those markers.
  Automatic confirmation/cleanup belongs only to validated v2 state.
- v1.5.0: `scan_base` skips dot-prefixed directories, including the reserved
  `.fastf-transactions` root, so private staging containing `PROJECT_INFO.md`
  cannot appear as a duplicate project.
- v0.11: `JOBS` map value is `JobHandle { progress, cancel }` (was a bare `Arc<Mutex<Progress>>`). `register_job` evicts finished handles on each new job; a `/api/job/<id>` 404 still means "done" to the frontend. `Progress` gained `phase` — set it alongside `status` at terminal states.

- v1.0: `paths::install_dir()` must stay infallible-looking but non-panicking. Never re-add `.expect` there; never memoize the resolution (tests swap `FASTF_INSTALL_DIR` within one process). Portable detection keys on `config.toml`/`templates/` NEXT TO THE EXE — a bare binary copied without them lands in the user config dir by design (first-run banner names the mode).
- v1.0.2: **`fastf-ui` launcher bin** (`src/bin/fastf-ui.rs`, second `[[bin]]`) — `#![cfg_attr(windows, windows_subsystem = "windows")]` shim that runs try_install_dir → ensure_bootstrapped → `cli::ui::run(app: true)` so the Start Menu shortcut opens the web UI with no console. NOT a second server (the server still lives only in `src/ui/`). Errors surface via a raw `MessageBoxW` extern (user32, edition-2024 `unsafe extern` — no new dependency); std discards println! when no console is attached. Built on all platforms; only Windows artifacts ship it (MSI + zip; the Linux tar copies just `fastf`).
- v1.0.2: **conhost ghosting fix** — `cli::recent::clamp_label(label, columns)` truncates picker labels to terminal width (`dialoguer::console::truncate_str`, unicode-width-aware, "…" tail; budget = columns − 3 for the "> " prefix + last-column margin; columns == 0 → passthrough). Applied in `run_picker` and the move-base picker. Wrapped Select lines are what ghosted on the legacy Windows console, so **picker labels must stay single-line, ANSI-free, and clamped** — don't add colored strings to Select items, and use `dialoguer::console` (don't add `console` as a direct dep). Unit tests in `recent.rs`.
- v1.0.2: **unconfigured `base_dir` falls back to the HOME directory, not the cwd** (`Config::resolve_base_dir` → `paths::home_dir()`, new pub helper: `%USERPROFILE%`/`$HOME`; cwd only if home is unset). The cwd fallback scattered projects and `.fastf-index.json` caches into whatever directory a command ran from. **Both test harnesses (`with_fresh_install` in integration.rs AND ui_server.rs) now redirect HOME/USERPROFILE into the sandbox** — without that, tests with a default config would scan the developer's real home and self-heal the counter from their real projects (5 register tests broke exactly that way). Any new test harness must do the same.
- v1.0.2: **onboarding is universal** — the shared core is `config::init_base_dir(raw)` (trim → `~` expansion → absolute-only → `create_dir_all` → canonicalize → persist `base_dir`) + `config::suggested_base_dir()` (`<home>/Projects`). The web UI's `/api/base/init` and the TUI's `onboard_first_run` (menu.rs, runs before the main loop when `base_dir` AND `bases` are both empty; empty answer skips, reappears next launch) are thin shells over it. Don't duplicate the expansion/validation anywhere else — call the core fn.

- v1.6.1: **`Config::load()` is never `unwrap_or_default()`ed.** It already
  returns `Ok(default)` when the file is *absent*, so a fallback can only mask a
  parse or I/O error — and the config decides which directories are the library,
  so defaulting answers a different question with a success. Every surface
  propagates it: the CLI exits 1 (`main.rs` adds a `hint:` line when the chain
  names `parsing` + `config.toml`), the TUI refuses to open a menu,
  `run_paged_browser`'s loader closure returns `Result`, and `src/ui` loads
  through `ui::load_config`, whose `SERVER_ERROR_PREFIX` makes `status_for`
  answer 500 rather than blaming the request.
- v1.6.1: **`util::tty::prompt_available` probes *stderr*, because that is where a
  prompt is drawn.** `dialoguer` writes to stderr and reads stdin (falling back to
  `/dev/tty`), so the old `stdout().is_terminal()` guards answered a different
  question: `fastf new t > out.txt` refused although a terminal was right there,
  and `2>/dev/null` sailed past the guard into dialoguer's bare "IO error: not a
  terminal". Every prompt in `src/cli/` goes through `tty::require_tty(what, how)`,
  whose message must name the flag that gets the same result without asking.
  **Stdout still decides output *format*** (`recent`/`search` plain list, the move
  progress line) — a genuinely different question; those three probes stay. A
  prompt whose absence changes what happens on disk (`fastf move`'s confirm)
  refuses rather than proceeding unconfirmed. The TUI is guarded once, at
  `tui::menu::run`, before the banner.
- v1.6.1: **`util::interrupt::restore_terminal` is the one cursor restore**,
  called from `main`'s error path and from the signal handler before the second
  Ctrl-C exits 130. The unix branch is raw `isatty` + `write` because a signal
  handler may not take std's stream lock; the non-unix branch keeps `Term`
  (Windows runs console control handlers on their own thread). Keep the
  per-stream terminal guard: an unguarded restore puts `\x1b[?25h` into piped
  output.

### v1.1 hardening gotchas

- v1.1: **`util::lockfile::DataLock` is the cross-process lock** over the data
  dir (`.fastf.lock`). Any read-modify-write of `counters.toml` or `config.toml`
  must hold it — `ui::WRITE_LOCK` is an in-process `Mutex` and cannot see a CLI
  running alongside the UI. Windows uses `share_mode(0)` (no FFI); Unix uses
  `flock`. Both are released by the OS on process death, so there is no stale
  lock to recover. **Never hold it across a prompt or a post-create hook** —
  `cli::new` re-plans inside the lock and runs `project::run_post_create`
  outside it for exactly that reason.
- v1.5.1: shared `core::operations` mutations hold `DataLock`, reload config and
  authoritative project identity/state beneath it, then refresh disposable
  caches. Move and reconcile hold it for their complete operation. A cached
  `Project` is only a hint and never authorizes deletion by itself.
- v1.1: `create_inner` claims the folder with `fs::create_dir` (NOT
  `create_dir_all`, which succeeds on an existing dir and let two racers merge
  into one folder). Everything after the claim lives in `provision_project` so a
  failure rolls the folder back; **nothing may sit between the claim and that
  call** — an early return there skips the rollback and leaks the folder. A
  failpoint placed one line too early found exactly that bug.
- v1.5.0: `PROJECT_INFO.md` is written first with `provisioning: true` and the
  flag is cleared before the create journal. An empty initial journal is
  reported because inline interpolation cannot be reconstructed. A deferred v2
  journal stores already-realized relative copies and can safely resume those
  verbatim files after validation.
- v1.1/v1.6.1: `assets::AssetEntry` carries an `EntryKind` enum, not an `is_dir`
  bool. This is deliberate — a new variant makes the compiler point at every
  consumer that must decide, and `walk` reports links instead of skipping them
  (template copying decides what to do about one by asking `is_file()`).
  The move-side invariant now lives in `core::transactions`:
  `MoveManifest::scan` is **deny-by-default** — a link or special entry fails
  the whole move rather than being omitted — and `verify_destination` compares
  the exact path/type/size manifest the scan produced. Verification must never
  be narrower than the copy it checks, which is how a move used to delete a
  source whose junctions never reached the destination. (v1.1's `assets`
  `copy_tree`/`jobs_for_tree`/`verify_tree`/`find_links` were the previous
  engine; Phase 5 deleted them once nothing called them.)
- v1.1: links are refused only on the **staged** move path (`MoveManifest::scan`,
  reached after `fs::rename` fails with `EXDEV`). The same-filesystem rename
  copies nothing and preserves links perfectly, so refusing there would block
  the common case for no benefit.
- v1.1: `util::fs_retry` wraps the destructive fs calls (Windows sharing
  violations from Defender/indexer, plus read-only attribute clearing). The
  same-fs `fs::rename` probe in `move_project_unlocked` is deliberately NOT wrapped:
  its failure is the signal to take the staged path, so retrying would add the
  full backoff to every cross-drive move.
- v1.1: `paths::display_path` strips `\\?\` for **display and metadata only**.
  Filesystem calls keep the canonical path — the verbatim form is what makes
  paths past MAX_PATH work, and long-path support without it is an opt-in system
  setting that is off on many machines. Strip at display, never at storage.
- v1.1: `library::load_cache` rejects a cache whose `version` is not
  `CACHE_VERSION`. The field existed but was never checked, so a cache from a
  newer fastf was *trusted* and the projects it failed to describe vanished —
  which matters because caches travel with projects between machines.
- v1.1: `util::faults` failpoints are compiled out of release. Tests that trip
  them are `#[cfg(debug_assertions)]`. `FASTF_FAULT` and the interrupt flag are
  process-global, so their test locks (`faults::TEST_LOCK`,
  `interrupt::TEST_LOCK`) live beside the state they guard — a private mutex per
  test module looks right and silently races.
- v1.1: concurrency tests must spawn **processes**, not threads. A thread test
  passes against an in-process `Mutex` while production stays broken.
- v1.1: Test count 183 → 263 on Linux (255 in release; the gap is the failpoint
  tests). `windows_semantics.rs` contributes only 3 outside Windows.
  New suites: `crash_recovery.rs`, `concurrency.rs`, `windows_semantics.rs`,
  `hostile_fs.rs`, `properties.rs` (proptest).

### v1.2 gotchas

**Counter in the base.** See "Global ID counter" above. `Counters::load()` is the
*legacy* data-dir read and is only one input to `Counters::floor`; `Counters::save`
is private, and `propagate` is its only caller, so the data-dir file can never
drift below the bases it backs up. `naming::id_value` **rejects any id containing a hyphen** — an
interim build wrote UUID (`019fa635-…-74a0bcb20044`) and word-handle
(`simple-panda-fennec`) ids, and reading the trailing digits of that UUID would
put the floor at 20044 and every later project at ID20045+. Such an id must
contribute nothing.

**`interpolate_name` collapses runs of `_` AND `-`, not just `__`.** A pattern
like `{date}_{user}_{artist}-{title}` with an empty `artist` used to produce
`..._french_-Seeping`: the orphaned `_` survived because the run was `_-`. A run
of two or more separators now collapses to the **last** one — "last wins" is what
makes a mixed run come out right, because the leading separator belonged to the
variable that vanished. Runs at either end are dropped, trimming a stray leading
or trailing `-` too. **Single separators are never touched**, so a `{date}` of
`2026-07-28` passes through intact — that is the property to protect.

**The `_2` collision suffix.** Bundled patterns keep `{id}`, so a collision is
rare — but a pattern need not contain one (several gallery templates use
`{name}` alone), and then the same answers twice in one day collide for real.
`create_inner` walks `name`, `name_2`, `name_3`, each a single atomic
`create_dir`, so racing processes land on different suffixes and can never merge.
The loop wraps **only** the claim — the v1.1 rule still holds: nothing between a
successful claim and `provision_project`, or the rollback is skipped and the
folder leaks. `create`/`create_deferred` therefore return the plan **as
realized**; callers must report from that, not from the plan they passed in.
`config.on_name_collision = "error"` restores the old refuse-a-duplicate
behaviour.

**Two tooling traps, both hit while building this.** Do not bulk-edit source with
PowerShell `Get-Content -Raw` + `Set-Content`: 5.1 reads as the ANSI codepage and
writes UTF-8, double-encoding every non-ASCII character (this repo is full of
`—`, `→`, `…`, `✓`). It silently corrupted nine files and broke a `char` literal.
Use the Edit tool or Node. And never put a backtick inside a JS template literal
in `app.js`, including inside an HTML comment — it terminates the string and the
error points at the following identifier. `node --check src/ui/web/app.js` catches
it; run it after every frontend edit.

### v1.2.1 gotchas

**The counter converges — see "Global ID counter", which was rewritten for it.**
The three rules that bite: `next_value` is the only place that computes the next
ID (three callers drifted before); `propagate` must `touch_cache` every base it
writes, or creates thrash every cache; and nothing may lower the counter, so `id
set` / `id reset` / `POST /api/counter` refuse rather than pretend.

**`trailing_var_arg` means clap's `requires`/`conflicts_with` only see flags typed
*before* the positional.** `register --dry-run` after the path lands in `extra`,
so the attribute never fires — which is how `--dry-run` came to write the folder
for real. Since v1.6.1 the constraint lives in exactly one place that sees the
whole line: `RegisterFlags::validate`, run on the merged set (clap's fields plus
what `apply_extra` lifted out of `extra`). Keep the clap attributes for the help
text and the early error; put the enforcement in `validate`.

**Don't read a `Config` field raw when a `resolve_*` exists.** `cli::note` passed
`&cfg.editor` where everything else calls `cfg.resolve_editor()`, so the
documented `$EDITOR` fallback never ran and the default install failed with
`launching editor ''`. Same shape as `resolve_base_dir`.

**`--force` on `template from-folder` must clear `files/` first.** Since v0.8
that subtree *is* the create spec, so regenerating without clearing merged the old
generation into the new template and shipped stale files into every project — with
the manifest's `structure` correctly replaced, so the two disagreed silently.

**Slice a timestamp with `.get(..10)`, never `[..10]`.** Journal timestamps come
from a parsed markdown body, so a hand-edited `PROJECT_INFO.md` can put anything
there; byte-slicing panicked mid-character on the first multi-byte one. Two sites
(`cli::note::notes`, `cli::recent::show_journal`) and `hostile_fs.rs`'s
degrade-never-panic promise covers metadata, not bodies.

### v1.3 gotchas

**The TUI contains errors; the discriminator is `dialoguer::Error`, not
`io::Error`.** `tui::menu::contain` reports a failure and returns to the current
submenu instead of unwinding to `main` (which exited 1 and threw away every
answer already given). What must *never* be contained is a failure of the prompt
itself — no TTY, stdin at EOF — because that returns to a loop which prompts and
fails again forever. The obvious rule, "propagate anything with an `io::Error` in
the chain", is exactly backwards: a mistyped path fails with `canonicalize`'s
`NotFound` wrapped in context, which is the case containment exists for.
`dialoguer` returns one error type and only from prompt calls, so that is the
test. `is_fatal` has unit tests for both shapes; `tests/tui_pty.rs` drives the
real flows.

**Menu arms yield a `Result` instead of using `?` inline.** Each `match` builds
an outcome and passes it to `contain(...)?`. Adding an arm that uses `?` directly
silently opts that path out of containment.

**Only the render thread may write while a live list is up.** On Windows
`console`'s `move_cursor_up` derives its target from the *live* console cursor
position, so one stray `println!` from another thread corrupts every later
redraw of `live_select`'s block. That is why the old `scan_page_sizes` progress
output was deleted rather than moved to the scanner, and why the scanner threads
are silent by construction. The same block is taken back by **line count**
(`clear_last_lines`), so `live_select`'s items must stay single-line and
ANSI-free — `clamp_label` and `ProjectRowTheme` are what guarantee that, and a
styled or unclamped label desynchronises the repaint.

**The Size cell is a fixed width (`SIZE_CELL`), not the page's widest value.**
Sizing the column to its contents — which the blocking scan did — shifts every
row sideways each time a background snapshot lands, and a table that reflows
while you read it is worse than one that is slow. Guarded by
`a_landing_size_does_not_reflow_the_row`, which compares **display columns**:
the pending cell's `…` is three bytes and one column, so a byte-offset
comparison fails a correctly aligned row.

**A pty test that proves "no input was needed" must anchor on the highlight.**
Ordering alone cannot: within one frame the project rows are written before the
navigation rows, so `size before Back` is true even when the size only arrived
after a keypress. `ProjectRowTheme` prefixes only the active row with `> `, so
`> ID0001 … 3.0 MB` on one line proves the size reached a list whose selection
had not yet been touched. Verified by breaking it — with the repaint tick raised
to 60s, `projects_browser_fills_in_sizes_without_any_input` fails.

**Guard `show_cursor` with `is_terminal`.** `Term::show_cursor` emits the escape
whatever it is writing to, so restoring the cursor unconditionally on the error
path put a literal `\x1b[?25h` into every piped error — corrupting the output a
script reads. Caught only by looking at `cat -v` of a piped failure.

**`config::expand_base_path` / `resolve_base_dir_input` are the only way in.**
The first expands `~` and requires an absolute path; the second adds
`create_dir_all` + canonicalize. Extra `bases` use the **first** deliberately:
creating a missing one would plant an empty directory at an unmounted mount
point and shadow the drive it stands for. Neither takes the lock — `DataLock` is
not reentrant and `config::set` already holds it.
CLI `new --base-dir`, UI preview/create overrides, and UI Settings use the same
functions; do not assign those request strings directly to `Config.base_dir`.

**Prompt first, then lock, then reload.** `edit_postcreate_commands` and
`menu_settings_bases` collect the answer, *then* call
`operations::update_config`, which takes `DataLock` and re-reads config. Holding
a loaded `Config` across a human prompt and saving it afterwards reverted
whatever the browser UI or another `config set` had written meanwhile. Both
remove paths match by text rather than by the index the user saw, since the list
may have changed while the prompt was open. `cli::config::normalize_base_entry`
is the one validator for an extra base, shared by `config set bases` and the
menu.

**`fastf move` polls, it does not block.**
`operations::move_project` already took a `&Mutex<Progress>` and `&AtomicBool`;
the CLI runs it on a scoped thread
and draws from the main one, feeding `interrupt::is_set()` into the cancel flag
so Ctrl-C aborts before the source is touched. Totals stay zero until the staged
path scans the tree and never fill in for a same-fs rename — so "nothing to draw"
is the normal case, not a bug.

### v1.6.1 gotchas

**`util::yaml::to_string_preserving_unknown` is how fastf rewrites a file it
does not fully own** — `PROJECT_INFO.md` frontmatter and `template.yaml`. Both
have an `OWNED_KEYS` const with an exhaustiveness test. `Template::OWNED_KEYS`
also lists `files` and `dir`, which are `#[serde(skip)]` and therefore never
serialized: without them a pre-v0.8 flat `files:` block would stop being dropped
on save and start being *preserved*, which is the opposite of what v0.8 decided.
The reasoning against `#[serde(flatten)]` is under "PROJECT_INFO.md" above; do
not revisit it without re-reading serde's `ContentDeserializer`.

**`render` returns `Result` because the fallback it replaced wrote an invisible
project.** `serde_yaml::to_string(&meta).unwrap_or_else(|e| format!("#
yaml-serialize-error: {e}"))` put a comment between valid `---` delimiters. That
parses as an empty document, so `read_project_meta` failed, so the folder was not
a project — from the moment it was created, with a success message on screen.
Never substitute a placeholder for content that defines a file's identity.

**A `.ok()` on `faults::check` disarms the failpoint.** `provisioning.rs`'s
source-cleanup boundary had one, so the single point where the source is already
gone and the bookkeeping is not yet done could not be tested at all. Every
`check` either propagates with `?` or is handled with `if let Err`;
`every_failpoint_in_the_source_is_declared_and_vice_versa` keeps
`ALL_FAULT_POINTS` matching the call sites, which its own doc comment had
claimed for two releases while nothing referenced the list.

**A `pub fn` under `core/` that mutates without `DataLock` says `_unlocked`.**
`provisioning::reconcile` did not, and reconcile resumes copies and removes
sources. It is now `#[doc(hidden)] pub fn reconcile_unlocked`, with
`reconcile_locked` as the only application entry point (it takes no `Config`:
it loads one beneath the lock, and the argument it used to accept was ignored).
The twelve test call sites build their `Config` in memory and never save it, so
switching them to `reconcile_locked` would have reconciled the wrong base — the
rename is what makes the signature honest without changing what the tests
exercise. The rule now holds across `library` too:
`unregister_project_unlocked`, `delete_project_unlocked`, and
`rename_project_unlocked` are the renamed compatibility shapes (they revalidate
the recorded project but take no lock), each `#[doc(hidden)]` with
`*_configured` as the application entry point and a private `*_inner` body
shared by both. `library::move_project` keeps its name because it does acquire
the lock.

**Deleting dead code is a phase, not a side effect (Phase 5).** What it left
behind, so nobody re-derives it: the pre-v2 marker *writers* are gone, and the
four tests that need those bytes plant the JSON literally — fastf having no
writer for a format it must never resurrect is the point, so do not add one
back for test convenience. `util::paths::require_real_file` and
`require_native_relative` are the shared boundary checks (they were duplicated
byte-for-byte in `transactions` and `provisioning`); `assets::require_real_directory`
is their directory sibling. `ReconcileReport.swept` stays in the JSON because
`docs/UI.md` promises the field, but nothing writes it and `is_empty` no longer
consults it. `copy_template_files` lost its `verbose` flag — both call sites
passed `false`, so the per-file printing had been unreachable since v0.8.

## Browser UI (`fastf ui`)

`fastf ui` defaults to a local loopback HTTP server and opens the browser UI. The
server and all API logic live only in the `fastf` lib (`src/ui/`) — no separate
server binary, no external web directory. (The v1.0.2 `fastf-ui` bin is a
windowless *launcher shim* over `cli::ui::run`, not a second server.)

**Working on the UI? The detail lives in two places, not here:**
`src/ui/CLAUDE.md` (loads automatically when you touch `src/ui/`) carries the
layout, the three create/template-file thresholds, and every UI gotcha;
`docs/UI.md` is the route-by-route reference. Don't re-list endpoints here.
