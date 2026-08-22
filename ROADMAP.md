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

- Release: **v1.6.0 — the guided browser stops waiting**
- Status: **v1.6.0 released; GitHub and AUR publication verified; manual network-share smoke pending**
- Last reviewed: **2026-08-18**

## Release train

| Release | State | Acceptance gate |
|---|---|---|
| v1.4.1 | Delivered in v1.5.1; CI passed | fastf cannot touch a path merely because a filename or pre-v2 marker implies ownership |
| v1.5.0 | Delivered in v1.5.1; CI and Linux smoke passed | scoped v2 move/create journals recover idempotently after process crashes |
| v1.5.1 | Released; CI, assets, and AUR packages verified | every mutation shares validation, locking, authoritative reload, and cache refresh behavior |
| v1.6.0 | Released; CI, assets, and AUR packages verified | the guided project browser never waits on a folder size |

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

### v1.6.1 — correctness and hygiene (in progress)

Track A of [`PLAN.md`](PLAN.md), one phase per session.

- [x] Phase 1: honest output and honest errors — a `config.toml` that does not
  parse stops every command instead of being replaced by defaults that resolve a
  different library; real creates and applies no longer print the dry-run
  header; the Library bases menu commits against configuration reloaded under
  the lock; the second Ctrl-C restores the cursor.
- [x] Phase 2: flags anywhere on the line, and prompts that know when there is
  no terminal — the trailing-argument classifier reads each subcommand's flag
  list from clap, so `register --rename` after the path renames instead of
  warning, and an unknown flag is refused rather than ignored; prompt
  availability is probed on stderr, where prompts are drawn, and every prompt
  that cannot run says which flag replaces it.
- [x] Phase 3: files fastf writes must stay readable — frontmatter and template
  keys fastf does not recognise survive every mutation in place instead of being
  deleted; a metadata file that cannot be serialized fails the create rather than
  being written unreadable; template manifests are written atomically; a dropped
  counter write and a rename that cannot be rolled back both say so; the
  source-cleanup failpoint can fire again, and `reconcile_unlocked` admits it
  does not hold the lock.
- [x] Phase 4: browser-server hardening, CI gates that match the docs, and the
  release procedure in git — an absurd `Content-Length` is refused instead of
  panicking the connection thread outside `catch_unwind`, and a malformed
  request is answered rather than dropped; `/api/open` and `/api/project` resolve
  their path through discovery instead of acting on any path on the machine;
  every response carries a content security policy; CI runs `node --check` and
  lints Windows code, and the Release workflow refuses a tag that does not match
  `Cargo.toml`; the release routine is tracked in the repository.

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

- Lazy template loading and one library snapshot for UI state.
- Deterministic one-pass interpolation with a frozen render context.
- Further terminal-rendering extraction from core.
- Dependency and binary-size cleanup based on measurements.
- Portable project packages.
- Template upgrades.
- Template diagnostics and language-server support.
- Project lifecycle states.
- Declarative post-create workflows.
