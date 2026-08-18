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

- Release: **v1.5.1 — shared mutation correctness**
- Status: **v1.5.1 released; GitHub and AUR publication verified; manual Windows drive smoke pending**
- Unreleased on `main`: **guided-TUI speed and feel** (see below) — merged, gated,
  and manually passed on Linux; not versioned, tagged, or packaged.
- Last reviewed: **2026-08-18**

### Guided-TUI speed and feel (unreleased)

The TUI is the primary surface in daily use, so five friction points in it were
fixed together. No config keys were added and no dependencies were introduced.

- [x] Esc (and `q`) backs out of every `Select`/`Confirm`/`MultiSelect`: Back in a
  submenu, Quit at the top level, No on a confirmation, cancel inside an action.
  Cancellation is not an error, so `contain`/`is_fatal` are unchanged.
- [x] Project sizes are measured when a project's action menu opens, never per
  page. The Size column is gone from the list; the session cache and its
  invalidation on mutation remain.
- [x] Tags are picked, not retyped: Remove tag is a checkbox list of the
  project's own tags, Add tag offers `library::known_tags` plus a new-tag row.
- [x] "Open in editor" action, via a public `post_create::open_in_editor`;
  `move_idx` is now found by name instead of a hard-coded index.
- [x] The template picker preselects `default_template` and marks it
  `(default)`; `fastf new` behaviour is unchanged.
- [x] A main-menu frame (`src/tui/dashboard.rs`): rule, base, cached library
  stats, and an in-memory session activity log. Stats refresh only after arms
  that can change them; `next_value` stays read-only.
- [x] `dev/tui-sandbox.sh` + `dev/README.md`: a disposable two-base fixture and
  the manual-pass checklist, because the automated gates cannot see feel.

Deferred deliberately, tracked in the backlog below: Esc inside the create
wizard, and progress/cancellation for `tree_size`.

## Release train

| Release | State | Acceptance gate |
|---|---|---|
| v1.4.1 | Delivered in v1.5.1; CI passed | fastf cannot touch a path merely because a filename or pre-v2 marker implies ownership |
| v1.5.0 | Delivered in v1.5.1; CI and Linux smoke passed | scoped v2 move/create journals recover idempotently after process crashes |
| v1.5.1 | Released; CI, assets, and AUR packages verified | every mutation shares validation, locking, authoritative reload, and cache refresh behavior |

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

## Release and documentation gates

Every release must pass:

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --all-targets`
- [x] `cargo test --release`
- [x] `node --check src/ui/web/app.js`
- [x] Windows cfg compile: `cargo check --all-targets --target
  x86_64-pc-windows-{gnu,msvc}`
- [x] Existing Linux CI target ([main run 30582924136](https://github.com/cristocola/fast-folder/actions/runs/30582924136))
- [x] Existing Windows CI targets, debug and release ([main run 30582924136](https://github.com/cristocola/fast-folder/actions/runs/30582924136))
- [x] GitHub Release workflow built Linux GNU/musl archives, the Windows ZIP,
  and the MSI; every asset matched `SHA256SUMS` ([run 30583196941](https://github.com/cristocola/fast-folder/actions/runs/30583196941)).
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

- **Cancellable text prompts.** `dialoguer::Input` has no opt variant, so Esc is
  inert in every text prompt — including `core::vars::collect_vars`, which is the
  create wizard's variable entry and the place a user will try it most. The only
  escape today is declining at the final confirm. Fixing it means
  `collect_vars` returning an optional map, which cascades into `new::run`,
  `apply::run`, and `apply::collect_if_needed`.
- **Progress and cancellation for `tree_size::directory_size`.** It is a blocking
  walk with no callback and no cancel flag, so a slow filesystem shows a static
  `measuring folder size…` for the whole duration. **Confirmed insufficient in
  real use (2026-08-18): seconds of stall on a network share, and worth fixing.**
  Measuring one project instead of a page moved the stall but did not remove it.
  The shape to copy already exists in `move_project_with` (`&Mutex<Progress>` +
  `&AtomicBool`, drawn from a scoped thread); the harder half is repainting a row
  underneath `dialoguer::Select`, which owns the terminal while it blocks on a
  keypress. Making the size an opt-in action row was considered and rejected: it
  would be discarded by this work.
- Lazy template loading and one library snapshot for UI state.
- Deterministic one-pass interpolation with a frozen render context.
- Further terminal-rendering extraction from core.
- Dependency and binary-size cleanup based on measurements.
- Portable project packages.
- Template upgrades.
- Template diagnostics and language-server support.
- Project lifecycle states.
- Declarative post-create workflows.
