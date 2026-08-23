# fastf robustness roadmap

This is the canonical implementation plan for fastf's single-user correctness
work. Completed items stay here as history, with release and commit links added
when they are published.

## Product contract

fastf is a local, single-user project scaffolder for self-contained trees of
ordinary directories and regular files. One person may use its CLI, TUI, and
loopback browser UI; fastf commands may wait behind one coarse mutation lock.
It does not coordinate simultaneous writers on multiple computers.

A move first asks the operating system to rename the directory. Only a standard
cross-device failure switches to fastf's internal Rust copy path. That path
copies the directory topology and every regular file, checks source and staged
file paths and byte lengths, publishes the complete staging directory, and only
then attempts to remove the source. The project must remain untouched by other
programs while it is moving.

The contract deliberately does not include hashes, ACLs, extended attributes,
sparse-file layout, hard-link relationships, symlink/junction reproduction, or
storage-level durability. Links and special entries are rejected when copying
would be required. Process-crash recovery is in scope for the v2 journals below;
hardware failure, power loss, bit rot, and storage corruption remain the
responsibility of the filesystem and backups.

## Current phase

- Release: **v1.7.1 — the guided TUI, and the structure under it**
- Status: **v1.7.1 released; it supersedes v1.7.0, whose Windows CI leg found two defects that only a 1 MiB thread stack and a backslash separator could show**
- Last reviewed: **2026-08-23**

## Release train

| Release | State | Acceptance gate |
|---|---|---|
| v1.4.1 | Delivered in v1.5.1; CI passed | fastf cannot touch a path merely because a filename or pre-v2 marker implies ownership |
| v1.5.0 | Delivered in v1.5.1; CI and Linux smoke passed | scoped v2 move/create journals recover idempotently after process crashes |
| v1.5.1 | Released; CI, assets, and AUR packages verified | every mutation shares validation, locking, authoritative reload, and cache refresh behavior |
| v1.6.0 | Released; CI, assets, and AUR packages verified | the guided project browser never waits on a folder size |
| v1.6.1 | Released; CI, assets, and AUR packages verified | what fastf says happened is what happened, and every file it rewrites stays readable |
| v1.7.0 | Superseded by v1.7.1; Windows CI failed | the guided menu can always be backed out of, never loses what you typed, and reaches everything the command line can |
| v1.7.1 | Released | the same, with the recursion bound low enough for a Windows thread stack |

### v1.4.1 — containment and path safety

- [x] Treat every source filename as payload, including names ending in `.tmp`
  and `.part`.
- [x] Remove recursive scratch sweeping; active operations clean only the exact
  staging/temp path they created.
- [x] Require every copy/verification source root to be an existing real
  directory.
- [x] Fall back from directory rename only for the platform's standard
  cross-device error.
- [x] Refuse occupied conventional staging and marker paths without replacing or
  deleting them.
- [x] Recheck final-target occupancy with `symlink_metadata` immediately before
  rename or publication.
- [x] Propagate marker writes and copy flush/sync failures; after publication,
  report source-removal failure as cleanup pending and retain the marker.
- [x] Validate template slugs and portable relative paths centrally. Preserve
  safe nested syntax such as `src/components`; reject absolute paths, drive
  paths, empty/dot components, and `..` both before and after interpolation.
- [x] Route CLI and browser base overrides through the shared absolute-path and
  `~` resolver.
- [x] Re-resolve destructive application operations as direct children of a
  currently configured base with a real, matching `PROJECT_INFO.md`; caches are
  discovery hints only.
- [x] Treat all pre-v2 create/move markers as obsolete, report them without
  parsing their bytes, and leave markers plus related paths untouched.
- [x] Serialize complete move and reconcile operations with `DataLock`.
- [x] Document pre-v2 recovery as report-only.

Acceptance:

- [x] Real `.tmp`, `.part`, zero-byte, binary, and empty-directory payloads
  survive a staged move.
- [x] Raw and rendered path traversal and unsafe template slugs are rejected.
- [x] Forged/obsolete markers remain byte-identical and cannot alter outside
  sentinels; repeated reconciliation has the same result.
- [x] Missing sources, occupied targets, and non-cross-device rename errors do
  not start copying.
- [x] Same-filesystem rename and the ordinary staged-copy path remain covered.
- [x] All locally runnable automated release gates below pass.

### v1.5.0 — simple internal move and recovery v2

- [x] Reserve `.fastf-transactions/<operation-id>/` beneath the target base.
  Require a real transaction root and claim every operation directory
  exclusively.
- [x] Generate operation IDs from timestamp, process ID, and an atomic process
  counter; retry exclusive creation on collision.
- [x] Store a versioned move journal containing only `version`, `operation_id`,
  `project_id`, configured source base, validated source folder, validated target
  folder, and a typed phase. Derive target base and staging from journal
  location.
- [x] Use phases `Copying`, `ReadyToCommit`, and `CleanupPending`; persist every
  phase successfully before proceeding.
- [x] Add a move-only manifest with native relative `PathBuf`s, `File` or
  `Directory`, byte length, and source modification time. Do not reuse lossy
  template paths.
- [x] Scan once on copy fallback, reject non-regular entries, create empty
  directories, and stream all regular files directly into private staging with
  one reusable bounded buffer.
- [x] Verify exact paths, entry types, and lengths. Rescan source
  path/type/size/mtime metadata and abort without removing it if anything
  changed.
- [x] Recheck final occupancy, publish staging, persist `CleanupPending`, remove
  the source, refresh metadata/caches, and remove the transaction only after all
  cleanup succeeds.
- [x] Before publication, cancellation removes only the owned transaction and
  leaves the source. Publication makes cancellation too late.
- [x] Hold `DataLock` for the complete move/reconcile operation; reads and
  cancellation remain available while other mutations wait.
- [x] Keep same-filesystem moves as a direct rename without a journal; discovery
  repairs cache state after a crash.
- [x] Add create-journal v2 with template slug plus validated source/destination
  relative paths only. Clear the provisioning flag before clearing its marker.

Recovery rules:

- [x] `Copying`: source is authoritative; remove only the owned transaction.
- [x] `ReadyToCommit` with staging present: discard staging and let the user
  rerun.
- [x] `ReadyToCommit` with staging absent and a matching final project: compare
  source/final path-and-size manifests, then transition to cleanup.
- [x] `CleanupPending` with a matching final project: retry source removal and
  clear the transaction only after success.
- [x] Report missing configured bases, identity mismatches, malformed journals,
  and unknown states without mutation.
- [x] Make reconciliation idempotent; keep pre-v2 markers unsupported and
  untouched.
- [x] Expose `MoveOutcome { project, cleanup_pending }` at application level,
  retain `library::move_project() -> Result<Project>` as a compatibility
  wrapper, and only add optional warning fields to HTTP job responses.

### v1.5.1 — shared mutation correctness

- [x] Add `core::operations` free functions for create, register, apply, move,
  reconcile, configuration, tags, notes, rename, unregister, and delete.
- [x] Make every operation validate, acquire `DataLock`, reload authoritative
  state under the lock, mutate, and refresh caches. Interfaces only prompt,
  translate input, and render output.
- [x] Use one coarse mutation lock; do not add distributed or path-scoped locks.
  Never hold it across prompts, editors, folder reveal, or post-create commands.
- [x] Replace the fixed `PROJECT_INFO.md.tmp` writer with the existing unique
  atomic writer (landed early in v1.4.1).
- [x] Centralize variable validation and transforms for CLI, TUI, UI preview,
  create, register, and apply.
- [x] Return the realized create plan from the atomic folder claim and use it for
  success output and post-create actions.
- [x] Recompute apply plans under the mutation lock and consider every existing
  directory entry occupied.
- [x] Require registration beneath a configured base, reject duplicate recovered
  IDs, and make `Skip` an immediate no-op.
- [x] Commit registration first, then run rename/apply through shared operations;
  report partial outcomes truthfully.
- [x] Resolve post-create policy identically across interfaces and run
  file-dependent actions only after provisioning completes.
- [x] Move noninteractive register/from-folder behavior out of CLI modules.
- [x] Enforce loopback browser binding and validate `Host` plus same-origin
  `Origin`, preserving the local single-user/no-account model and JSON shapes.
- [x] Remove or correct any claim that the browser's process mutex serializes
  separate fastf processes.

### v1.6.0 — the guided browser stops waiting

Measuring a project means walking its whole tree, and the browser did that for
every row of a page before drawing anything. On a network share that is seconds
of dead interface per page, for a column that is useful but never urgent.

- [x] Draw the project list before any folder has been measured, and show a
  pending cell rather than a gap.
- [x] Measure folders on background workers, at most two at a time, taking the
  selected row first and the rest of the visible page after it.
- [x] Reprioritize on page and selection changes by replacing the queue rather
  than appending to it. Leave walks already in flight to finish.
- [x] Repaint the list in place as results land, with no keypress required.
- [x] Give the size walk a cancel token checked once per directory entry, so
  teardown is bounded on a slow filesystem. Discard a cancelled walk instead of
  recording it as unavailable.
- [x] Fix the width of the Size cell so a landing snapshot cannot reflow the
  table under the reader.
- [x] Drop a project's snapshot when an action changes it, so the row is
  measured again on return.
- [x] Keep sizes out of `Project`, `.fastf-index.json`, `PROJECT_INFO.md`, and
  `/api/state`, and leave `fastf recent` and `fastf search` output unchanged.
- [x] Restore the terminal from `Drop`, so a panic anywhere inside the picker
  cannot leave a shell without a cursor.

Regression coverage:

- [x] The size reaches a list whose selection has not been touched
  (`projects_browser_fills_in_sizes_without_any_input`, anchored on the
  highlight prefix, verified by breaking the repaint).
- [x] A landing size does not move the name column, compared in display columns.
- [x] Scanner queueing, re-measurement after `forget`, unreadable projects, and
  bounded teardown with work outstanding.
- [x] Viewport math for a list taller than the terminal.

### v1.6.1 — correctness and hygiene

Five phases, one per session, no new features: making the output honest, the
input forgiving, the files fastf rewrites readable, the server boundary
explicit, and the source free of code nothing calls.

- [x] Phase 1 ([#7](https://github.com/cristocola/fast-folder/pull/7)): honest output and honest errors — a `config.toml` that does not
  parse stops every command instead of being replaced by defaults that resolve a
  different library; real creates and applies no longer print the dry-run
  header; the Library bases menu commits against configuration reloaded under
  the lock; the second Ctrl-C restores the cursor.
- [x] Phase 2 ([#8](https://github.com/cristocola/fast-folder/pull/8)): flags anywhere on the line, and prompts that know when there is
  no terminal — the trailing-argument classifier reads each subcommand's flag
  list from clap, so `register --rename` after the path renames instead of
  warning, and an unknown flag is refused rather than ignored; prompt
  availability is probed on stderr, where prompts are drawn, and every prompt
  that cannot run says which flag replaces it.
- [x] Phase 3 ([#9](https://github.com/cristocola/fast-folder/pull/9)): files fastf writes must stay readable — frontmatter and template
  keys fastf does not recognise survive every mutation in place instead of being
  deleted; a metadata file that cannot be serialized fails the create rather than
  being written unreadable; template manifests are written atomically; a dropped
  counter write and a rename that cannot be rolled back both say so; the
  source-cleanup failpoint can fire again, and `reconcile_unlocked` admits it
  does not hold the lock.
- [x] Phase 4 ([#10](https://github.com/cristocola/fast-folder/pull/10)): browser-server hardening, CI gates that match the docs, and the
  release procedure in git — an absurd `Content-Length` is refused instead of
  panicking the connection thread outside `catch_unwind`, and a malformed
  request is answered rather than dropped; `/api/open` and `/api/project` resolve
  their path through discovery instead of acting on any path on the machine;
  every response carries a content security policy; CI runs `node --check` and
  lints Windows code, and the Release workflow refuses a tag that does not match
  `Cargo.toml`; the release routine is tracked in the repository.
- [x] Phase 5 ([#11](https://github.com/cristocola/fast-folder/pull/11)): dead code out, stale gotchas corrected — the superseded move
  engine in `assets` and four uncalled move wrappers are deleted, the pre-v2
  marker writers are gone (the tests that need those bytes plant them), the
  duplicated path checks are one pair in `util::paths`, and every mutating
  library entry point that does not hold the lock says `_unlocked` in its name;
  `CLAUDE.md` now describes the code that exists.

### v1.7.0 — the guided TUI

The guided menu is the daily surface, so this release is about it: one way out
of every prompt, typed input that survives a validation failure, a browser that
stops rescanning the library, and parity with what the command line can already
do.

- [x] Phase 11: parity with the command line, and a builder that lets you change
  your mind — the menu can bulk-register a base after showing the same preview
  `--recursive --dry-run` prints, choose a created date, bundle assets when
  generating a template from a folder, reindex, run recovery, show data
  locations, and set the register naming pattern. The template builder's new
  mode ends in the review menu instead of a bare Save prompt; folders and files
  get per-item Add / Edit / Remove; a file can be declared empty.
- [x] Phase 10: the main-menu frame, a better action menu, more keys, one
  browser — the menu shows each base and whether it is there, indexed counts,
  the highest ID and this session's last few actions, all from the caches and a
  probe, scanning nothing (`show_frame`, on by default). Lists take PageUp /
  PageDown, Home / End and `/` to filter. The action menu leads with Open and
  Copy path, and folds tagging into a submenu where tags are ticked off a list
  instead of retyped, plus Re-derive. `fastf recent` and `fastf search` open the
  same browser as the menu; the second, size-less picker is gone.
- [x] Phase 9: the browser stops rescanning — a tag, a note, a rename or a move
  patches the one row it changed instead of re-running discovery across every
  base, and a delete drops the row; a search browser re-evaluates its query
  against the patched project so a row that stopped matching leaves. Base lists
  are probed with a timeout, so an unresponsive network mount is named rather
  than hanging the menu. `util::trace` (debug-only, `FASTF_TRACE_FILE`) is what
  makes the claim testable.
- [x] Phase 8: keep what you typed — a value with a local validity rule is
  checked at the prompt that asked for it and stays on the line to be corrected,
  and dependent questions come after the value they depend on. Register checks
  the path before the three questions that follow it, apply checks the target
  before the dry-run question and every variable, and a search that matches
  nothing comes back with the query still in the field.
- [x] Phase 7: one cancel contract — Esc (and `q` on a list) backs out of every
  menu, list, confirmation and text field, one level per press, and a cancelled
  create leaves no folder and consumes no ID. Every prompt goes through
  `tui::prompt`, which `tests/layering.rs` enforces; text prompts are a
  hand-rolled line editor because `dialoguer::Input` has no Esc at all. Menus
  match on labels rather than raw indices.
- [x] Phase 6: relocate the terminal picker library — every interactive terminal
  surface moved under `src/tui/` (`browser`, `actions`, `rows`, `pickers`,
  `vars`), so the guided menu is no longer a client of the CLI layer;
  `cli/recent.rs` is `fastf recent` and `fastf open` again. Two template
  pickers, three base pickers and two byte formatters became one each, and
  `tests/layering.rs` fails the build if `core` or `util` reaches for a prompt.

### v1.7.1 — core structure, and two Windows-only defects

This structural track is behaviour-neutral and shipped alongside the guided-TUI
work above. The tag exists because **v1.7.0's Windows CI leg failed**, on two
defects that no Linux run could show:

- `paths::MAX_WALK_DEPTH` was 256, chosen against a Linux main thread's 8 MiB
  stack. A Windows *thread* gets 1 MiB, and the browser's size scan runs on
  worker threads — 256 frames of `read_dir` iterator overflowed one, which is
  precisely the failure the limit exists to prevent. It is 64 now, still far past
  any real project layout.
- `tests/layering.rs` matched its own exception list with `/` suffixes against
  `Path::display()`, so on Windows the guard flagged `util::diag`, the module it
  was written to allow.

Both are fixed and both gates are green on all four target/profile combinations.

- [x] Phase 17: `CLAUDE.md` as a working document, dependencies refreshed — the
  development notes are organised by topic and split by directory (root 308
  lines, plus `src/core/`, `src/tui/`, `src/ui/` and `tests/`), so a rule sits
  beside the code it constrains. The archived `serde_yaml` is replaced by the
  maintained fork behind `util::yaml`, with a byte-identity snapshot captured
  from the old crate; `chrono` drops the default features nothing calls and
  `colored` is current. Deprecated dependencies 1 → 0; release binary
  3,905,368 → 3,905,136 bytes.
- [x] Phase 16: one test harness, faster suites, the coverage gaps — the sandbox
  and its `HOME` redirect are defined once in `tests/common/env`, the two
  2700-line and 1300-line suites are split by subject, and the modules that had
  no coverage at all (`expand_base_path`, `template_import`, `paths`, `reindex`,
  `reconcile`, `tag`, `template`) have some. The gallery test names all eight
  templates and plans each instead of counting to five.
- [x] Phase 15: types over strings, path fidelity, bounded recursion — job
  status and phase, the recovery kinds and the name-collision policy are enums
  whose serialized names are unchanged and asserted; a template file whose name
  is not valid UTF-8 reaches the new project spelled exactly as it was, instead
  of aborting the create at a path nobody wrote; a path that will be stored is
  refused rather than mangled; and every recursive walk stops at a depth limit
  and says where.
- [x] Phase 14: one clock, one load, lazy templates — every interpolation in an
  operation renders against one `RenderContext` built when the plan was, so a
  create spanning midnight cannot date the folder differently from the files in
  it, and substitution is one left-to-right pass whose result no longer depends
  on `HashMap` order. Listing templates no longer reads their file contents
  (4 scans to 0 on a two-template library), and `effective_bases()` memoizes
  against the configuration it was computed from.
- [x] Phase 13: split `library.rs` — 1268 production lines with nine
  responsibilities became a facade over `model` / `discovery` / `cache` /
  `guard` / `lifecycle` / `resolve`, with the staged move engine moved out to
  `core::move_engine` beside `transactions`. Every `library::…` path callers
  used still resolves; no logic changed.
- [x] Phase 12: rendering out of core, and the module cycles broken — `core`
  produces `DryRunReport`/`ApplyReport` and `cli/render.rs` turns them into text,
  so the dry-run's content is testable for the first time; `core::post_create`
  returns notes instead of printing them; every best-effort warning goes through
  `util::diag`; `now_iso8601`, `ProjectPlan` and `apply_transform` moved to
  where they belong and four module cycles went with them; register writes its
  metadata once. `tests/layering.rs` keeps it that way.

## Release and documentation gates

Every release must pass:

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --all-targets`
- [x] `cargo test --release`
- [x] `node --check src/ui/web/app.js`
- [x] Windows cfg compile: `cargo check --all-targets --target
  x86_64-pc-windows-{gnu,msvc}`
- [x] Windows clippy: `cargo clippy --all-targets -- -D warnings` on a Windows
  runner (CI's "fmt + clippy (windows-latest)" leg), so `#[cfg(windows)]` code is
  linted rather than merely compiled
- [x] `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --locked` (CI's "docs build
  clean"; a `pub` item's docs may not link to a `pub(crate)` one)
- [x] Existing Linux CI target ([main run 32171192192](https://github.com/cristocola/fast-folder/actions/runs/32171192192))
- [x] Existing Windows CI targets, debug and release ([main run 32171192192](https://github.com/cristocola/fast-folder/actions/runs/32171192192))
- [x] GitHub Release workflow built Linux GNU/musl archives, the Windows ZIP,
  and the MSI; every asset matched `SHA256SUMS` ([run 32171214147](https://github.com/cristocola/fast-folder/actions/runs/32171214147)).
- [x] `makepkg -f` completed for `fast-folder` and `fast-folder-bin`; the source
  package's release test suite passed before both AUR repositories were pushed.

Regression coverage grows with the relevant release:

- [x] Real `.tmp`/`.part`, zero-byte, binary, and empty-directory move payloads.
- [x] Missing source, occupied target/staging, cancellation, marker failure, and
  cleanup-pending behavior.
- [x] Template slug, structure, rendered-path, and base-override containment.
- [x] Obsolete/malformed markers remain byte-identical and cannot affect outside
  sentinels; reconciliation is idempotent.
- [x] Hard-abort subprocess cases at transaction creation, mid-copy,
  post-verification, post-publication, and before/after source cleanup (v1.5.0).
- [x] Cross-interface mutation-loss, registration partial-outcome, and browser
  Host/Origin cases (v1.5.1).
- [x] Guided browser draws before measuring, fills in without input, and never
  reflows a row as a size lands (v1.6.0).

Manual move smoke and follow-up:

- [x] Linux same-filesystem direct rename and genuine cross-filesystem staged
  move (`/tmp` to `/dev/shm`) using the release binary.
- [ ] Windows same-drive rename and ordinary move to another mounted drive/share
  using the published v1.5.1 MSI or ZIP. This remains the sole post-release
  validation item.

Behavior changes and user documentation land together. CLI flags, template and
cache schemas, and existing HTTP response fields remain compatible; rejecting
previously accepted unsafe input is intentional. Update this roadmap in every
implementation PR/commit. Update `CLAUDE.md` only after a decision has landed.
This work does not use GitHub issues, a separate ADR system, or a changelog.

## Completed history

| Release | Evidence |
|---|---|
| v1.4.0 | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.4.0) · [tag commit](https://github.com/cristocola/fast-folder/commit/847b020) |
| v1.4.1 + v1.5.0 | Delivered in v1.5.1: [implementation commit](https://github.com/cristocola/fast-folder/commit/f4f7d40) · [Windows portability fix](https://github.com/cristocola/fast-folder/commit/78a2e1d) |
| v1.5.1 | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.5.1) · [tag commit](https://github.com/cristocola/fast-folder/commit/23e9258) · [PR #1](https://github.com/cristocola/fast-folder/pull/1) · [main CI](https://github.com/cristocola/fast-folder/actions/runs/30582924136) · [release workflow](https://github.com/cristocola/fast-folder/actions/runs/30583196941) · [AUR source](https://aur.archlinux.org/packages/fast-folder) · [AUR binary](https://aur.archlinux.org/packages/fast-folder-bin) |

v1.4.1 and v1.5.0 were delivered as part of the cohesive v1.5.1 release rather
than as separate published tags. Their checked items remain above as history.

## Unscheduled backlog

- One library snapshot for UI state. (Lazy template loading landed in v1.7.1:
  `load_all` no longer reads template file contents.)
- Portable project packages.
- Template upgrades.
- Template diagnostics and language-server support.
- Project lifecycle states.
- Declarative post-create workflows.
- Scriptability: `--json`/`--format` output, `search --limit/--template/--since/--tag`,
  a `fastf path <query>` command, `print_path` as a `new` flag rather than only a
  config toggle, `--color=auto|always|never` (`colored` currently gates on stdout
  only, so stderr gets ANSI when redirected), documented exit codes, and
  `completions <shell>` as a typed `clap_complete::Shell` rather than a bare
  `String`.
- Browser UI: a tagged-template `html` helper (plus a source-scan test) in place
  of 125 manual `esc()` calls; the triplicated job-polling logic in `app.js`; a
  full `/api/state` reload on every mutation instead of a targeted patch; a modal
  focus trap and `:focus-visible` styling; `node --test` coverage for the pure
  frontend helpers.

Smaller findings from the v1.7.1 audit, not worth a phase on their own:

- `Counters::load().unwrap_or_default()` inside `propagate`
  (`src/core/counter.rs:126`) swallows a parse/IO error the way `Config::load`
  did before v1.6.1's Phase 1 — less damaging (the floor still self-heals from
  every base and `library::max_id`), but it silently drops the "an unplugged
  base can't restart numbering" protection that file exists for.
- `query::resolve_field` clones per field access and `Predicate::Free`
  lowercases per comparison (`src/core/query.rs`) — fine at current scale, would
  matter at a much larger library.
- `size_scan::request`'s queue dedup is an O(n²) `contains` scan
  (`src/util/size_scan.rs`) — bounded by page size today.
- The action menu only offers "Move to another base" when one is already
  mounted; an `Unresponsive` base (Phase 9's probe) could offer a "retry probe"
  item instead of just being left out.
- `tests/tui_pty/browser.rs:214`
  (`projects_browser_fills_in_sizes_without_any_input`) is timing-sensitive: it
  asserts a background size snapshot reaches the list within one repaint tick,
  and failed once under the CPU load of a full `cargo test --all-targets` while
  passing every standalone run. The guarantee is right; the deadline is thin —
  give it a longer window or anchor it on the scanner instead of the clock.
