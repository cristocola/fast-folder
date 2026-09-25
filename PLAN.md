# PLAN — a cross-drive move can never half-delete its source

Five phases on one branch, `fix/atomic-moves`, one commit per phase, one PR at the end,
released as **3.12.0**. The gate list is the `release` skill's; run all of it between
phases. Publication — the version bump, the tag, the AUR — waits for an explicit
"release".

## Why

A cross-drive `fastf move` of a Vite project, from a base on an sshfs mount to a local
base, deleted 1460 of the source's 1473 files, then stopped. The transaction stayed in
`CleanupPending`, and every `fastf reconcile` printed

```
source/final comparison failed (cannot move 'node_modules/.bin/nanoid': links are not
supported by cross-drive moves); left source untouched
```

which was false. Nothing was lost only because the copy had already been verified. The
bug report written at the time blamed "verify interleaved with delete" and "the link is
only discovered after the copy". **Neither is what the code does**: the scan runs before
the copy (`move_engine.rs:257`) and cleanup verifies both trees completely before it
deletes anything (`move_engine.rs:364-366`). What happened:

1. The mount used sshfs `follow_symlinks`, which resolves links on the server. The three
   `node_modules/.bin/*` links had live targets at scan time, so `lstat` reported them as
   **regular files**; they were copied as files and verification agreed.
2. Cleanup ran `fs_retry::remove_dir_all(source)` (`move_engine.rs:366`). std's unix
   implementation walks in readdir order; once `node_modules/vite/bin/vite.js` was gone,
   `.bin/vite` was a dangling link, which that mount hides (readdir lists it as
   `DT_UNKNOWN`, lookup answers `ENOENT`). std ignores `ENOENT` for children
   (`library/std/src/sys/fs/unix.rs`, `remove_dir_all_recursive`), then `rmdir .bin`
   failed `ENOTEMPTY` and the error unwound the walk, leaving what it had not reached.
3. Reconcile re-verified the half-deleted source, failed, and reported the one reassuring
   phrase it has — "left source untouched" — which is about *this pass*, not the disk.

**The general defect is step 2, and it needs no sshfs**: deleting a tree is not atomic,
so any failure part-way — a chmod-555 folder inside the project, a Windows file open in
an editor, a network drop, a source that turns read-only — leaves a husk that still
holds `PROJECT_INFO.md`, so the library lists it as the project. `fastf delete` has the
same shape (`library/lifecycle.rs:97`).

### Defects this plan fixes

| # | Defect | Where |
|---|---|---|
| 1 | Source cleanup deletes in place, so a failure part-way leaves a husk the library lists | `move_engine.rs:366`, `provisioning.rs:910` |
| 2 | `fastf delete` deletes in place the same way | `library/lifecycle.rs:97` |
| 3 | Reconcile says "left (source) untouched" about a source that is half gone | `provisioning.rs:823-905` |
| 4 | Verification errors say "source changed while it was being copied" with no path, including in recovery, long after any copy | `transactions.rs:181` |
| 5 | The scan stops at the first odd entry and names only it | `transactions.rs:232-263` |
| 6 | An entry readdir lists but `lstat` cannot examine reads as "classifying …: No such file or directory" | `transactions.rs:223-225` |
| 7 | Links are refused by cross-drive moves and copies at all | `transactions.rs:232` |
| 8 | The scan asks `is_symlink()` rather than the reparse tag, so Windows reparse points that are not name surrogates are copied without being classified | `transactions.rs:232` vs `paths::is_link_like` |
| 9 | A mount that resolves links itself is invisible: its links are copied as their targets and removal deletes through them | no check exists |
| 10 | A read-only (or unrenamable) source base is only discovered after the whole copy | no check exists |
| 11 | A nested mount inside a project is walked into, copied, and deleted through | `transactions.rs:243` recursion, std `remove_dir_all` |
| 12 | Name clashes (case-insensitive target), invalid names, too-long names are discovered file by file during the content copy | `copy_to_staging`, `transactions.rs:483` |
| 13 | Out-of-space is discovered at the end of the copy | no check exists |
| 14 | Publish uses `fs_retry::rename` (~310 ms of backoff), so a Windows indexer holding the fresh tree for a moment **discards a verified staging copy** | `move_engine.rs:297`, `copy_engine.rs:188` |
| 15 | A copy's transaction is indistinguishable from a move's — safe today only because a copy never advances its journal past `Copying`, which nothing enforces | `copy_engine.rs:146`, `provisioning.rs:762` |
| 16 | The command line prints cleanup-pending twice (engine `diag::warn` + CLI `eprintln`) | `move_engine.rs:341-374`, `cli/move_project.rs:165` |
| 17 | `verify_source_unchanged` compares whole manifests, `version` included, so any manifest-version bump would strand every older transaction | `transactions.rs:180` |
| 18 | `docs/cli.md:439-446` shows a link-refusal message the code has never printed | docs |

## The design in one page

**Principles** (these go into `src/core/CLAUDE.md` › Moving projects):

1. **The source leaves the library in one rename.** Cleanup never deletes in place: it
   renames the source to `<source_base>/.fastf-moved-<operation>` (same parent, same
   filesystem, atomic; discovery skips dot-folders), then removes that. A failed rename
   leaves the source whole; a failed removal leaves a hidden, redundant, fastf-owned
   leftover — never a husk. `fastf delete` does the same through `.fastf-deleted-<op>`.
2. **One redundancy rule decides removal**, in-process and in reconcile: nothing is
   removed that exists nowhere else.
3. **Links are content**: copied as links with their exact target text, never followed,
   dangling allowed — what `mv` does and what the same-disk rename already does.
4. **Every question answerable before the copy is answered before the copy.** Those
   checks are a courtesy; correctness rests on the atomic retire and on verification.
5. **Reports say what is on disk**, never what this pass did or did not do.

**Publish first, then retire** (the existing order). A Windows "file in use" at retire
time then costs a reconcile, not a second copy, and the project is listed twice (same
id, two bases — already a supported state) until it is resolved.

### Records

- **Journal v3** (`MOVE_VERSION = 3`; the reader accepts 2 and 3). `set_phase` always
  stamps 3, and a v2 journal is rewritten as v3 *before* its first retire, or a 3.11
  binary reconciling it would find no source, do the bookkeeping and orphan the retired
  folder. New fields, `serde(default)`: phase `Retired`; `operation: Move | Copy`
  (defect 15); `host: Option<String>` — reconcile on another machine only reports on
  the source side (the maintainer runs several machines over shared bases). Derived,
  never stored: `retired_path() = source_base/.fastf-moved-<op>`,
  `probe_path() = source_base/.fastf-probe-<op>`. `published.json` beside the manifest:
  the destination walk `verify_destination` already takes, saved at publish.
- **Manifest v2** (`MANIFEST_VERSION = 2`; the reader accepts 1 and 2; a v1 manifest may
  not hold link kinds). Kinds `File | Directory | Symlink | DirSymlink | Junction` plus
  `link_target: Option<PathBuf>`. Every `verify_*` compares **entries only** (defect 17).

### The walk and the comparator

A tolerant, depth-threaded walk that never aborts on an odd entry. It records files,
directories and links, and *problems*: unexaminable (readdir lists it, `lstat` fails),
special (FIFO, socket, device), another device (a directory whose `st_dev` differs from
the root's — a nested mount), non-UTF-8 name or link target (JSON cannot hold it),
refused Windows reparse tag, unreadable directory.

`MoveManifest::compare(&walk, Match) -> ManifestDiff { recorded, missing, added,
changed, problems }`, with `summary(10)` naming paths and counts ("1 changed, 1457 of
the 1473 recorded entries missing" then "node_modules/.bin/nanoid: a link now, was a
312-byte file"). `Match` says how two entries at one path must agree; the diff answers
two questions:

| Question | Built as | Used for |
|---|---|---|
| exact source | `compare(Exact).is_clean()` — every entry equal, folder times included | the source after the copy, before publish |
| content | `compare(Content).is_clean()` — path, kind, size, link target | the destination |
| whole | `compare(Whole).is_clean()` — `Exact` but folder times ignored (removing or renaming a child moves them) | a source about to be retired |
| residue | `compare(Whole).is_residue()` — every present entry recorded and unchanged; missing is fine; anything added, changed or problematic fails | a 3.11 half-delete, a retired folder |

An entry beneath a folder the walk could not read is unknown, not missing; a problem at
a recorded path reads as a change ("a link now, was …").

### Removal

- **Destination condition** (F = the published destination): F's `PROJECT_INFO.md` id
  matches the journal, and for every entry about to be removed F holds an entry of the
  same kind there which, for files and links, is either as published (size + mtime in
  `published.json`) or modified after publication. Edits to F are fine; an F restored
  from an older backup is not. A v2 journal has no `published.json` and falls back to
  a content check.
- **`retire(S → R)`** (new `src/core/move_cleanup.rs`): requires S a real directory
  carrying our id, whole, and the destination condition. If fastf's own working
  directory is inside S it steps out (Windows cannot rename a process's cwd). Rename
  through the new `fs_retry::rename_dir` (~2–3 s of backoff for 5/32, no read-only
  fallback). On error re-check both paths — a network rename can land on the server
  while the client sees an error: R present and S absent → `Done`; S present and R
  absent → `KeptWhole(reason)`, with Windows 5/32 translated to "a program has a file in
  it open, or it is a working directory"; anything else → `Unknown`.
- **`remove_retired(R)`**: first the whole of R must be a residue + the destination
  condition + one device + nothing unexaminable, else R is kept whole for inspection.
  Then a walk of its own (not std): never follows links, never crosses devices, threads
  the depth, re-checks each entry against the manifest just before removing it,
  directories bottom-up; on unix a directory that refuses with `EACCES` is chmodded
  `u+rwx` and retried; on Windows `fs_retry` clears read-only. Result `Removed` or
  `Leftover { remaining, first names, reason }`; litter (`.nfsXXXX`, `Thumbs.db`, `._*`)
  is named, never auto-deleted.
- **In-process order after publish** (`move_engine.rs`): fsync the target base dir
  (unix) → `CleanupPending` → retire → `Retired` → bookkeeping → GC → remove the
  transaction.
- `MoveOutcome.source: SourceOutcome::{Removed, Retired { leftover, reason },
  KeptWhole { reason }, Unknown { reason }}` replaces `cleanup_pending: bool` (keep a
  `cleanup_pending()` helper). The engine returns data; the CLI and the app each render
  it once (defect 16).

### Reconcile, as a table

S source, R `.fastf-moved-<op>`, T staging, F final; "ours" = `PROJECT_INFO.md` id
matches the journal.

| journal | phase | on disk | action |
|---|---|---|---|
| any | any | `host` differs, or `operation = Copy` | source side report-only; a Copy only ever discards its own T |
| any | Copying | S ours, F and R absent | remove P (known names only), discard the transaction |
| any | Copying | otherwise | report |
| any | ReadyToCommit | T, no F, S ours, no R | discard (as today) |
| any | ReadyToCommit | no T, F ours, S ours | exact source(S) + content(F) → CleanupPending → continue |
| any | ReadyToCommit | R present, or any other combination | report; never discard T, never touch R |
| v2 | CleanupPending | S present | residue(S) + destination condition (identity only if `PROJECT_INFO.md` survives) → rewrite as v3 → retire → GC. Finishes a 3.11 half-delete whose residue is provably redundant; otherwise reports the diff |
| v3 | CleanupPending | S ours, no R | retire, then as in-process |
| any | CleanupPending | R present | write `Retired`, continue as Retired (S is foreign; never touched) |
| any | CleanupPending | neither S nor R | bookkeeping, remove the transaction |
| any | Retired | F missing or not ours | report "R holds the only copy; rename it back"; never GC |
| any | Retired | R present | GC; on `Leftover` report and keep the transaction |
| any | Retired | no R | bookkeeping, remove the transaction |
| none | — | orphan `.fastf-moved-*` | report only |
| none | — | orphan `.fastf-probe-*` | remove known files, `rmdir` |
| none | — | `.fastf-deleted-*` | GC it (the user confirmed the delete with the word) |

`reconcile_base` and `list_incomplete` skip `.fastf-moved-*`, `.fastf-probe-*` and
`.fastf-deleted-*` for the create and case-rename checks (today they would resume a
create journal found *inside* R). Deleting S or R by hand is the supported way out of a
kept state: the next pass finds it absent and completes. Every message states the disk:
"the moved copy is at F; the original at S still holds 16 of the 1473 entries the move
recorded and 3 it did not (first: …); fastf removed nothing. Delete S yourself if F is
the copy you want, then run `fastf reconcile`." `ReconcileReport` gains `leftovers`, and
the CLI footer "Nothing was changed for these" (`cli/reconcile.rs:108`) must be true by
construction.

### Before a byte is copied (new `src/core/move_preflight.rs`)

1. Walk problems refuse the move, up to 10 named, "and N more". Unexaminable reads
   "listed by the filesystem but cannot be examined (…); a network mount that resolves
   links itself hides dangling links this way".
2. **Source-base probe, move only**: in `.fastf-probe-<op>`, create a file `f` and a link
   `l -> f`, `lstat l`. If it reads as a regular file, the mount resolves links itself →
   refuse, naming sshfs `follow_symlinks` (this alone would have stopped the incident).
   A link that cannot be created is "unknown", not a refusal. Rename P once (exercises
   rename), check sticky bit + ownership on unix. Any write failure refuses: "a move
   removes the source, so it needs to write in <base>: Read-only file system. Nothing was
   copied." — the read-only answer.
3. **Free space** (`src/util/disk_space.rs`): unix `statvfs` `f_bavail * f_frsize`
   (`f_blocks == 0` is unknown); Windows `GetDiskFreeSpaceExW` by hand-rolled FFI.
   Refuse only when the manifest's bytes exceed what is free.
4. **Names before content** in `copy_to_staging`: `create_dir` in manifest order, empty
   files with `create_new`, links last (so no later write can pass through one), then
   the content pass (unix `O_NOFOLLOW`). Errors collected (10) and mapped: `AlreadyExists`
   in fresh private staging → "the target's filesystem treats 'X' and 'x' as one name";
   `EINVAL`/123 → name not allowed there; `ENAMETOOLONG`/206 → too long there; `EPERM`
   on a link or 1314 → cannot hold links (on Windows: Developer Mode is off).

`copy_engine` runs 1, 3 and 4, never 2.

### Links

Unix `symlink(target, path)`. Windows: `src/util/win_reparse.rs`, hand-rolled FFI as in
`util/shell_open.rs` — `reparse_tag` (`CreateFileW` with `FILE_FLAG_OPEN_REPARSE_POINT |
FILE_FLAG_BACKUP_SEMANTICS`, then `GetFileInformationByHandleEx(FileAttributeTagInfo)`)
and `create_junction`, mirroring std's unstable `junction_point` (substitute name
`\??\…` from `read_link`'s `\\?\…`; byte lengths exclude the NULs; a drive root keeps its
trailing `\`; remove the empty directory if the ioctl fails; refuse UNC and
`Volume{GUID}` targets). `paths::classify` by tag: `SYMLINK` → `Symlink`/`DirSymlink` by
the link's **own** directory attribute (its target may dangle); `MOUNT_POINT` →
`Junction`, except `\??\Volume{…}` (a mounted volume) refused; `CLOUD*`, `DEDUP`, `WOF`
→ ordinary data, read through; `LX_SYMLINK`, `AF_UNIX`, unknown → refused.
`is_link_like` stays the strict write-safety check. Verify compares `read_link` text.
The outcome notes links that are absolute or climb out of the project, and absolute
links that point *inside* the source (they dangle afterwards, exactly as after a
rename). Never pass a link target through `display_path`.

### Failpoints

Added to `util/faults.rs` `ALL_FAULT_POINTS`, each a literal `check("…")` so
`every_failpoint_in_the_source_is_declared_and_vice_versa` sees it: `move:after-probe`,
`move:before-retire`, `move:after-retire` (renamed, `Retired` not yet written),
`move:mid-gc`, `delete:after-retire`. `move:source-cleanup` now means "the retire
fails"; `move:after-source-cleanup` fires after GC.

## How to work a phase

Scope is that phase only; anything else found goes to the Parking lot. Write each test
against the broken build first (`tests/CLAUDE.md`); if it passes before the fix, label
it a design guard. Read the result from the disk, not through `discover`. Permission
tests skip when `geteuid() == 0` (the install lab runs as root). Gates between phases:
`cargo fmt --all -- --check`; `cargo clippy --all-targets -- -D warnings` in debug,
`--release`, and `--target x86_64-pc-windows-gnu`; `cargo test` and `cargo test
--release`; `rm -rf target/doc && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
--locked`. Update `docs/` and the CLAUDE.md beside the code in the same commit. Tick a
box only when it is verified; record what happened in the Phase log.

---

## Phase 1 — the walk, the comparator, and messages that name paths

No behaviour change except better refusals and messages. Links are still refused.

- [x] Tolerant walk in `transactions.rs` (`walk_at` threading depth behind a zero entry
  point): entries plus `WalkProblem { path, kind }` for unexaminable, special, other
  device, non-UTF-8, unreadable directory, too deep. Devices via
  `std::os::unix::fs::MetadataExt::dev` (unix only; Windows mounted volumes are a
  reparse tag, Phase 4).
- [x] `MoveManifest::scan` refuses with every problem (first 10 named, "and N more"),
  links still among them ("links are not supported by cross-drive moves yet" stays
  until Phase 4). An unexaminable entry says what it is (defect 6).
- [x] `ManifestDiff` + `MoveManifest::compare(walk, Match)` with the three rules; every
  `verify_*` routes through it and compares entries only (defect 17); messages name the
  first paths and counts (defect 4). `verify_recovery_pair` keeps its behaviour.
- [x] Manifest v2 schema: `ManifestKind::{Symlink, DirSymlink, Junction}`,
  `link_target: Option<PathBuf>` (`serde(default, skip_serializing_if)`),
  `MANIFEST_VERSION = 2`, reader accepts 1 and 2, `validate` refuses link kinds in v1,
  a link without a target, a target on a non-link, bytes on a non-file.
- [x] Tests: a FIFO and a second problem are both named; a mode-000 directory is
  reported (skip as root); a v1 manifest compares equal to a v2 scan of the same tree;
  diff wording for missing / added / changed / link-vs-file; `validate` refusals.

Acceptance: `transactions` unit tests and `library/tests.rs` staged-move tests green;
the scan names every problem; no message says "changed while it was being copied".

## Phase 2 — the fix: journal v3, retire, GC, reconcile

- [x] Journal v3 fields, `set_phase` stamps 3, derived `retired_path`/`probe_path`,
  `published.json` written at publish (`move_engine.rs:297`) and read by reconcile.
- [x] `fs_retry::rename_dir` (longer backoff for 5/32, no read-only fallback) for the
  publish rename (`move_engine.rs:297`, `copy_engine.rs:188`, defect 14) and the retire.
- [x] `src/core/move_cleanup.rs`: destination condition, `retire`, `remove_retired`,
  as designed. Unit tests for each, including a chmod-555 folder (removed) and a
  retired folder holding an unrecorded file (kept whole).
- [x] `move_engine.rs` post-publish order; `SourceOutcome`; `report_cleanup_pending`
  and the engine's duplicate `diag::warn`s go; failpoints added and re-meant.
- [x] `provisioning.rs` follows the table; `finish_cleanup_pending` split into retire
  and GC steps; `.fastf-*` skipping in `reconcile_base` and `list_incomplete`;
  `IncompleteKind::Leftover` (serialized `leftover`) for the header count;
  `ReconcileReport.leftovers`; every "left (source) untouched" in move paths rewritten
  to state the disk (defect 3).
- [x] `copy_engine` writes `operation: Copy`; reconcile never removes a copy's source.
- [x] `library/lifecycle.rs` delete retires to `.fastf-deleted-<op>` then removes it
  (`delete:after-retire`); reconcile finishes a leftover one (defect 2).
- [x] Rendering: `cli/move_project.rs:165` (one message per `SourceOutcome`),
  `cli/reconcile.rs` (leftovers; footer true), `tui/runtime.rs:964-1003` (move status),
  `tui/runtime.rs:1224-1263` (reconcile), `tui/app/jobs.rs` batch report warnings; a refusal of more than one line (the scan's problem list, a diff) opens a message
  dialog for a single move too — the status line shows one line (`App::set_status`), so
  today a single failed move would show only the header.
- [x] Tests — `src/core/library/tests.rs`: the incident class — a staged move of a
  project with a chmod-555 folder leaves no folder, husk or `.fastf-*` in the source
  base and a complete F (skip as root); `move:mid-gc` reports a leftover and reconcile
  finishes it; `cleanup_failure_is_a_reported_success_and_retains_the_marker` moves to
  `KeptWhole` with S whole and no R. `provisioning.rs` units, planting v2 journal bytes
  literally: partial S completes and the journal is v3 on disk before the rename; S with
  an extra file is report-only and byte-identical; Retired with F deleted leaves R;
  S recreated after the retire is left alone; a create journal inside R is not resumed;
  orphan R reported; a Copy journal with F and S never removes S; a host mismatch is
  report-only. `tests/crash_recovery.rs`: `MOVE_ABORT_POINTS` gains `after-probe`
  (Phase 3), `after-retire`, `mid-gc`; the commit match (`:351`) includes them; after
  settling, no husk and no `.fastf-*` in the source base, read from the directory.

Acceptance: no code path deletes a source in place; every reconcile message about a
move states what is on disk.

## Phase 3 — before a byte is copied

- [x] `src/core/move_preflight.rs`: walk problems (Phase 1), the source-base probe with
  the link-follow check, sticky bit and ownership (unix), free space; `move:after-probe`.
  Probe P is removed on every path; a crash between leaves it for reconcile.
- [x] `src/util/disk_space.rs` (unix `statvfs`; Windows `GetDiskFreeSpaceExW`, a path
  ending in `\`); unknown never refuses.
- [x] `copy_to_staging` split into the names pass and the content pass, errors collected
  and mapped (defect 12). Progress shows the names pass as part of Copying.
- [x] `copy_engine` wired to 1, 3, 4.
- [x] Tests: a chmod-555 source base is refused before any staging, S intact, no
  transaction (skip as root); the link-follow decision and the error mapping as pure
  functions; `f_blocks == 0` is unknown; the names pass refuses a name collision before
  any content is written (a pure test of the mapping plus a planted pre-existing staging
  entry).

## Phase 4 — links are content

- [x] Manifest link kinds live: scan records them, the names pass creates them last,
  verification compares target text, the redundancy rule and GC treat them as entries
  (GC unlinks, never follows).
- [x] `paths::classify` (unix and Windows), `src/util/win_reparse.rs` (tag read,
  junction create). The scan uses `classify` (defect 8).
- [x] Outcome notes for outward links and absolute self-links; rendered by the CLI and
  the app.
- [x] Tests: relative, absolute, dangling and directory symlinks round-trip through a
  staged move and `copy-to`; a retargeted link and a link replaced by a file fail
  verification; flip `staged_move_pre_flight_refuses_links` (`library/tests.rs:905`),
  `scan_refuses_a_link_rather_than_skipping_it` (`transactions.rs:678`), the
  `tests/move.rs:341` doc comment and `windows_live.rs:365`. Windows
  (`windows_semantics.rs`, over `ssh win11`): a staged junction round-trips; a
  directory symlink round-trips (skip on 1314); a file held with `share_mode(0)` keeps S
  whole (`KeptWhole`) and reconcile completes once it is closed.

## Phase 5 — docs, the record, and ship

- [x] `docs/projects.md` › Moving projects and › What fastf promises: links reproduced;
  hard links become separate files; the source leaves in one rename; the limits — a
  mount that resolves links on the server (sshfs `follow_symlinks` is refused by the
  probe; a Samba share following links for a client without unix extensions cannot be
  probed, since the client cannot create a link), ownership/ACLs/xattrs not carried.
- [x] `docs/cli.md` › Moving projects, › Symlinks and junctions (defect 18), reconcile;
  `docs/windows.md` junctions; `README.md:44` if its wording drifts.
- [x] `src/core/CLAUDE.md` › Moving projects and › Recovery: the principles, the table,
  the limits.
- [x] A real cross-device run with the release binary (a `/dev/shm` base to a base on
  another filesystem): dangling `node_modules/.bin` links and a chmod-555 folder arrive,
  the source leaves in one step, reconcile's output is true. If sshfs and a local sshd
  are available, `follow_symlinks` is refused by the probe.
- [x] The app's move report and reconcile status checked with the screenshot tool.
- [x] Windows VM pass (clippy, tests, the phase-4 Windows tests).
- [ ] PR; on "release": the `release` skill for 3.12.0, both AUR packages; retire this
  file.

## Parking lot

- (empty)

## Phase log

- 2026-09-25 — plan written from the incident; branch `fix/atomic-moves` cut from
  `main` at 0737252.
- 2026-09-25 — Phase 1. `Walk`, `Problem`, `ManifestDiff`, `Match` in `transactions.rs`;
  `scan` refuses with every problem; every verify goes through `compare`, entries only;
  manifest v2 schema written (link kinds validated, still refused by the walk and the
  copy). The four modes of the design became three `Match` rules plus two questions
  (`is_clean`, `is_residue`) — the table above says which is which. The nested-mount
  check (unix, folders only) has no test: making a mount needs root, and a btrfs
  subvolume needs btrfs. `docs/cli.md`'s link-refusal example is left for Phase 4,
  where links stop being refused. Gates green: fmt, clippy debug/release/windows-gnu,
  test debug/release, doc.
- 2026-09-25 — Phase 2. `core::move_cleanup` (retire, the redundancy rule,
  `remove_tree`), journal v3 (`Retired`, `operation`, `host`, `legacy_cleanup`),
  `published.json`, `fs_retry::rename_dir`/`remove_dir`/`describe_rename_error`,
  `SourceOutcome` + `SourceOutcome::warning`, the reconcile table, `fastf delete`
  through `.fastf-deleted-<op>`, `ReconcileReport.{leftovers, cleared}`,
  `IncompleteKind::Leftover`; the app opens a dialog for a paragraph warning or a
  many-line error (`needs_a_dialog`). Found while testing, and fixed: (1) the
  bookkeeping rewrites the moved copy's `PROJECT_INFO.md` before the retired copy
  is removed, so without a published record (3.11) an exact content check kept
  every retired copy forever — the fallback is now "not older than the original";
  (2) rewriting a 3.11 journal as version 3 before a retire that then failed lost
  the fact that its source may be half-deleted, and the next pass demanded it
  whole — `legacy_cleanup` is set on reading a version-2 journal and carried.
  Outside the phase but on its gates: `util::trace`'s counting test shared
  `read_metadata`/`discover` with every parallel unit test that reads a project,
  and the new reconcile tests made that race fire; it now counts names only it
  writes. The in-process move writes `published.json` from the verified staging
  walk; reconcile reads it. The probe (`move:after-probe`) is Phase 3's.
  Gates green: fmt, clippy debug/release/windows-gnu, test debug/release, doc.
- 2026-09-25 — Phase 3. `core::move_preflight` (probe with the link-follow check,
  sticky ownership, `check_space`), `util::disk_space` (`statvfs`,
  `GetDiskFreeSpaceExW`), the names pass (`create_names`, `name_refusal`) and the
  contents pass opened `O_NOFOLLOW`; reconcile and `list_incomplete` clear and
  count a probe a killed move left. `move:after-probe` fires with the probe on
  disk, so the crash suite proves reconcile clears it. The probe renames itself
  once, to `<probe>-renamed`, which shares the prefix. The design's "reconcile's
  Copying arm removes P" became "reconcile removes any probe" — a pass holds the
  lock, so no move is mid-probe. Gates green.
- 2026-09-25 — Phase 4. Links are manifest entries (`entry_for` + `link_kind`;
  `paths::classify` was not needed — the one classification already lived in
  `entry_for`, and `is_link_like` stays the write-safety check), made last by the
  names pass (`make_link`, `link_refusal`), verified by target text, unlinked by
  `remove_tree`. `util::win_reparse` reads the tag and makes junctions (buffer
  layout unit-tested on every platform's Windows build, a real junction on the
  VM). `copied_summary` names links beside files; `link_notes` in both outcomes.
  The Windows VM ran all 455 unit tests green, including a staged junction
  that removal never went through, a directory symlink (the VM's ssh session is
  elevated, so it was really made), and a file held open with read-only sharing
  keeping the original whole until reconcile. Two Windows-only test mistakes
  fixed on the way: `read_link` answers the plain `C:\` form, and a moved
  project's path is canonical. Gates green.
- 2026-09-25 — Phase 5. Docs: `projects.md` (moving, recovery, promises, the
  mount limits), `cli.md` (moving, links, copy-to, recovery), `windows.md`
  (moving to another drive), `app.md` (the attention count), `README.md`.
  **A real cross-device run** (release binary, a `/dev/shm` base to the
  scratchpad): the incident's `node_modules/.bin` dangling links arrived
  verbatim, a mode-555 folder did not stop the removal, nothing was left behind,
  reconcile had nothing to do; a read-only source base and a socket plus a pipe
  were refused before any copy. **A real sshfs mount** (local `sftp-server`
  through `ssh_command`, no sshd login needed) found two things the unit tests
  could not: (1) on `follow_symlinks`, `symlink()` makes the link on the server
  and then fails `EIO`, so the probe took "cannot make links" and let the mount
  through — it now judges by what `lstat` finds (`move_preflight::observe`); (2)
  this sshfs (3.7.6) defaults to `contain_symlinks`, refusing `readlink` with
  `EPERM` for every link that is absolute or climbs with `..` — every
  `node_modules/.bin` link — so such a link is `Problem::LinkNotReadable`, whose
  message names `-o no_contain_symlinks`. With that option and without
  `follow_symlinks`, the project moved with its link intact. For contrast, the
  installed 3.11.0 on the `follow_symlinks` mount turned every link into a copy
  of its target. The app: a single move that keeps its original now opens the
  dialog, and — found by looking at the frame — the list patched the original's
  row into the moved one although the original is still a whole project on
  disk; it now reloads, listing both until reconcile (pty test
  `a_move_that_keeps_its_original_says_why_in_a_dialog`). Windows VM: 457 unit
  tests green. Gates green. Left: publication, on the word.
