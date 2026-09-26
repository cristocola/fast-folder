# PLAN — moves that finish clean on fragile mounts

Eight phases on one branch, `fix/moves-that-finish`, one commit per phase,
one PR at the end, released as **3.14.0**. One phase per session unless the
maintainer sets a `/goal`. **No GitHub CI while the phases are written**: the
gates run locally, in the Windows VM and in the Docker boxes, and CI runs once,
on the release PR. Publication (the version bump, the tag, the AUR) waits for
an explicit "release".

## Why

The maintainer tested moves on three fragile bases — a local scratch base, an
sshfs mount of a laptop, and an rclone mount of an S3 bucket (Cloudflare R2) —
with a real web project (node_modules, a running dev server). His words: fastf
creates the new folder, compares it very well, starts deleting the source, some
small thing stops it, and it leaves half-baked files there. The header says "1
needs attention". Reconcile answers that it cannot, for a reason he cannot act
on. He asked for a system that is clean, fast, antifragile and very robust:
**nothing may stop a move but a permission problem or a lock, and then it must
say so.**

His own logs and job records (3.13.0, 2026-09-25) show every one of these:

- Reconcile's report said, for four old copies: "a moved project's retired
  original, with no record of the move left; fastf will not remove it without
  one. Look inside, and delete it yourself". Three were on the R2 mount, after
  a move out of it that reported "removed the old copy 79 entries" and
  "cleared the record"; the fourth was a retired copy whose record lived in a
  base he later removed from `bases`.
- A move out of R2 hit `Input/output error` and `Transport endpoint is not
  connected` mid-removal (the mount restarted); the job ended with a raw
  errno list.
- A running `astro dev` rewrote `.astro/dev.log` in the original: the move
  first failed after the whole copy ("the source changed after it was scanned:
  6 changed", copy discarded), then on a retry kept the old copy "whole,
  because it is not what was moved: 1 changed", which no reconcile ever
  resolves. The dev server then re-created the original path, and moving the
  project back failed with "move target already exists".
- Checking 3044 entries of an old copy on R2 took 2 min 10 s; removing ran at
  about 250 ms an entry. One move of twelve projects left an 85 MB, 399,000
  line job log.

### Defects this plan fixes

MC = `src/core/move_cleanup.rs`, TX = `transactions.rs`, ME = `move_engine.rs`,
PV = `provisioning.rs`, MP = `move_preflight.rs`, CE = `copy_engine.rs`
(anchors are 3.13.0's).

| # | Defect | Where |
|---|---|---|
| 1 | "Is it gone?" is `symlink_metadata(p).is_ok()`: an EIO, ENOTCONN or EACCES reads as gone, the removal reports `Removed`, and the record is deleted while the old copy is still there | MC:1073 (used at 567, 660, 668, 816), PV:1580 (506, 1251, 1299, 1320) |
| 2 | A cloud mount re-creates a removed folder minutes later (uploads in flight); the record is already gone | MC:606-642 |
| 3 | An old copy that differs from the record is "kept whole" and re-reported by every reconcile for ever | MC:141-143, PV:1532 |
| 4 | A retired folder whose record is on a base no longer configured, or not answering, is "no record … delete it yourself" | PV:502-515 |
| 5 | The publish's bytes land but its `sync_all` fails: `published` stays false, the record is deleted, and both copies are listed | ME:559-594, TX:1990 |
| 6 | A folder rename that is not atomic (S3 through rclone) leaves part in S and part in R; reconcile removes R and leaves S | MC:420-422, PV:1354-1367 |
| 7 | `PROJECT_INFO.md` left out of `published.json` when its lstat fails: every later check fails for ever | ME:574 |
| 8 | After the retire, `complete_destination` repairs from the old path, which is empty | MC:205-213 |
| 9 | A listing error part-way through a folder is dropped by `.flatten()` | MC:890 |
| 10 | A killed `copy-to`'s record lives outside every base and is never visited | CE:180-187 |
| 11 | `fastf delete` writes no record; it also walks the whole project under the data lock only to find a mount inside it | `library/lifecycle.rs:137-206` |
| 12 | A transaction folder without a readable `move.json` is "invalid" for ever | PV:641, 982, 1008, 1026 |
| 13 | Unix never retries anything: EIO, ENOTCONN, ESTALE, a lagging ENOTEMPTY | `util/fs_retry.rs:58-61` |
| 14 | A folder's mtime moving (a temp file made and removed by a dev server) fails a move after the whole copy, and the copy is discarded; so does any file changed before it was copied | TX:461-470 (`Match::Exact`), TX:1940-1946 |
| 15 | Everything is sequential; S is walked 3×, F 3×+, R 2–3×, plus 2 lstat + 1 unlink per removed entry; every copied file is fsynced alone | TX:958, MC:867, TX:1990 |
| 16 | A DEBUG line per entry, written under the progress mutex (mkdir+stat+stat+open+write per line); the job log never rotates; `jobs::prune` has no size cap | `core/progress.rs:133-150`, `util/log.rs:162-164`, `core/jobs.rs:660` |
| 17 | Moves lose every file's mtime and permission bits (`+x` included) | TX:1923-1999 |
| 18 | The app's header summary and every row wait on every base, in series, with no timeout (canonicalize before the probe; `list_incomplete` ignores the probe; discovery sends one message at the end); quit joins size workers stuck on a dead mount | `tui/loaders.rs:36-84`, `library/discovery.rs:21-32`, `util/size_scan.rs:152` |
| 19 | "Needs attention" is a count: no list, no key named, report-only items lit for ever, the reconcile report gone when its dialog closes | `tui/loaders.rs:77,123`, `tui/view/dashboard.rs:127-142`, `tui/app/background.rs:219-317` |
| 20 | A reconcile run from **another data dir** on the same machine treats a live move's record as abandoned and discards its copy mid-write: liveness is only this data dir's `jobs/` (found by the Phase 0 lab: two moves failed, "657 of the 1639 recorded entries missing") | `core/jobs.rs::live_workers`/`owned_by`, PV:187-188 |
| 21 | A base's index built from a scan older than a new project can be written after it, pass the mtime gate, and hide that project until the next rescan ("no project matches"), because the gate compares the index file's own mtime with the base's. **On an rclone S3 base the gate can never fire**: a folder's mtime reads 2000-01-01 once rclone's directory cache expires, so a project copied in from elsewhere stays invisible until `fastf reindex` (found by the lab on R2) | `library/discovery.rs:93-104`, `library/cache.rs:177-194` |

## Decisions

1. **Only what fastf cannot fix stops a move**, before anything is copied,
   naming the path and, for a lock, the program: no permission; a program
   holding a file in the project open for writing (Linux) or holding it at all
   (Windows); no space; a read-only drive; a name the target will not take. A
   program merely *running* in the folder — a dev server, a shell, a language
   server — does not stop it: the move absorbs what it changes and the result
   names it ("node (pid 4242) is still running in the old folder").
2. **Everything else is waited out, retried or resumed** — EIO, a dropped or
   restarting mount, a stall, a listing lag. Nothing ends in "delete it
   yourself".
3. **A change made during a move is kept.** Before publishing, the copy
   re-copies what changed. At the retire, a file changed in the old copy
   replaces the moved one when the moved one is untouched since the publish
   and it is within the first hour. Only when *both* changed does fastf ask,
   and the record stays until it is answered.
4. **fastf finishes its own leftovers**: after the app's first paint, after
   every job, and every few minutes while it waits on a base. "Needs
   attention" means "needs you".
5. **Moves keep mtimes and permission bits**, like `mv`.
6. **On an rclone mount the old copy is removed in place, not renamed**
   (`PROJECT_INFO.md` first): no rename with uploads in flight, and a third of
   the requests.

## The design in one page

**Principles** (they go into the CLAUDE.md files as each phase lands):

1. **A record outlives everything it owns.** Absence is proven only by
   ENOENT/ENOTDIR; any other answer is *unknown*, and unknown never clears a
   record. On a filesystem that can resurrect a folder (rclone, unknown FUSE)
   "gone" is confirmed again at least ten minutes later before the record goes.
2. **Removal is a merge, entry by entry.** The old copy is walked once; each
   entry is removed, completed into the moved copy, fast-forwarded into it, or
   kept as a conflict. What is left is exactly what needs a person.
3. **The engine runs in parallel**, by filesystem: local 4 workers, NFS/SMB/
   sshfs 8, rclone/FUSE 16 (tuned by measurement).
4. **Only permissions and locks stop a move**; everything else is classified
   and retried, waited out or paused with its record kept.
5. **fastf finishes what it can by itself**; attention lists what it cannot,
   each item with its reason and its actions.
6. **The app never waits on a base.**

### Files

```
<target base>/.fastf-transactions/<op>/     as today: move.json (+ optional
                                             "retire": "in-place"), manifest.json
                                             (written once the source settles),
                                             published.json, phase.* markers
<source base>/.fastf-moved-<op>/             a renamed old copy (as today)
<source base>/.fastf-moved-<op>.json         in-place retire only: a pointer to
                                             the record, removed last
<source base>/.fastf-deleted-<op>.json       in-place delete only: the manifest
<data dir>/records/<op>.json                 index of every record: where it is,
                                             and gone_at for the settle; advisory,
                                             never authorises a removal
<data dir>/attention.json                    the last pass's verdicts (conflicts)
<data dir>/jobs/<id>/log, log.1              rotated at 16 MiB
```

A 3.13 binary (the other OS of a dual-boot machine) reads rename records as
before, refuses an in-place record rather than finishing it wrongly (`move.json`
has `deny_unknown_fields`), and ignores the pointer files and the data-dir
files. New `JobPhase`/`JobStatus` variants get a `#[serde(other)] Unknown`.

### Recovery table (replaces the one in `src/core/CLAUDE.md` as phases land)

| phase | on disk | action |
|---|---|---|
| any | another machine's record | needs you, report only |
| any | S, R, F or the record unknown (not answering) | waiting; nothing changed |
| any | a copy's record (also found through the index) | discard T if unpublished, clear; never touch S |
| Copying | S ours; F absent or our unpublished copy | discard (`paused` → waiting, resume) |
| Copying (in place) | F holds our `PROJECT_INFO.md` | as CleanupPending |
| CleanupPending, Retired | F not ours | needs you; R may be the only copy |
| CleanupPending (rename) | S ours, no R | F-coverage walk, retire, merge R |
| CleanupPending (rename) | R and S | split rename: merge R (`Full`), then S (`Residue`) |
| CleanupPending/Retired (in place) | S has or had our `PROJECT_INFO.md` | pointer, unlink it, `Retired`, merge S |
| CleanupPending, Retired | residue absent; local, or `gone_at` ≥ 10 min | bookkeeping, clear record, pointer, index |
| same | residue absent on rclone/FUSE, `gone_at` unset or young | set `gone_at`; waiting |
| same | the merge left conflicts | needs you, per-entry actions |
| no record | `.fastf-moved-<op>` | lookup chain → finished if the move is proven, else needs you |
| no record | a pointer alone | remove it |
| no record | `.fastf-deleted-<op>` (± its `.json` manifest) | remove (only what the manifest lists) |

## How to work a phase

Scope is that phase only; anything else found goes to the Parking lot. Write
each test against the broken build first (`tests/CLAUDE.md`); one that passes
before the fix is labelled a design guard. Read results from the disk, not
through `discover`. Look at every new frame with the screenshot tool before
blessing its snapshot. Run fastf on the host with `FASTF_NO_RELAUNCH=1` and
output piped.

Gates between phases:

- `cargo fmt --all -- --check`;
- `cargo clippy --all-targets -- -D warnings` in debug, `--release` and
  `--target x86_64-pc-windows-gnu`;
- `cargo test`, `cargo test --release`, and `taskset -c 0,1 cargo test`
  (CI's runners have two cores, and this work adds threads);
- `rm -rf target/doc && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`;
- the lib's tests natively in the Windows VM;
- the private mount lab's scenarios (it lives beside the repository, never in
  it), compared with the 3.13.0 baseline it recorded in Phase 0.

Update `docs/` and the CLAUDE.md beside the code in the same commit. Tick a box
only when it is verified, and record what happened in the Phase log.

---

## Phase 0 — the lab, the fixtures, the baseline

Outside the repository, except this file.

- [x] The maintainer's problem project (a vite site with node_modules) moved
  out of the working tree into the lab's fixtures.
- [x] The mount lab: an isolated data dir; the three real test bases plus
  private fault mounts the lab owns (a second sshfs, a local `rclone serve
  s3` behind an `rclone mount`), so stopping one never freezes the
  maintainer's own services; fixtures (links of every kind, 20k small files,
  big files, mode-555 folders, `+x` scripts, old mtimes, odd names); faults (a
  fake dev server, a writer holding a file open, a shell in the folder,
  stall, drop, resurrect, the debug failpoints); a runner that starts jobs in
  parallel, polls their state, times every step and checks the invariants;
  a teardown that always restores.
- [x] The baseline: every scenario against 3.13.0, recorded.

Acceptance: the baseline reproduces defects 1–4, 13, 14 and 15 on the real
mounts, with timings.

## Phase 1 — records are never lost; logs stay sane

- [x] `util::paths::presence(p) -> Presence { Present(Metadata), Absent,
  Unknown(io::Error) }` (absent only on ENOENT/ENOTDIR) replaces `entry_exists`
  and `entry_exists_quiet` and the same pattern in `sweep_strays`,
  `MoveTransaction::remove` and `clear_probe`. Unknown never clears a record
  and never counts as removed; reconcile calls it waiting. `retire()` refuses
  to rename past an unknown. A removal is `Removed` only when its root is
  absent afterwards; a listing error marks its folder incomplete (defects 1, 9).
- [x] The publish: attempted is set before `PROJECT_INFO.md` is written; on an
  error, a file that is not absent keeps the record as published. The entry
  in `published.json` comes from the open handle's fstat; a record without it
  falls back to the manifest's time, which unsticks existing records
  (defects 5, 7). *As built:* the entry is read back with up to three tries
  rather than from the write's own handle; the fallback covers the rest.
- [x] `complete_destination` reads from the tree being checked. A split
  rename (R and S both there) is finished on both halves, S as residue, the
  record kept until both are absent (defects 6, 8).
- [x] `core::records`: `<data dir>/records/<op>.json`, written at
  `MoveTransaction::begin`, cleared with the record. Reconcile visits indexed
  records outside the bases (a copy-to's) and finds a record whose base is no
  longer configured. An orphan `.fastf-moved-<op>` is looked up in: the
  configured bases, the index, the pointer, the job history
  (`ItemReport.operation` added), R's own `PROJECT_INFO.md`. A record on a base
  that is not answering is waiting (defects 4, 10). *As built:* configured
  bases, then the index; for a folder with no record anywhere, R's own
  `PROJECT_INFO.md` or a one-item job names the project and where it is now.
  The pointer comes with Phase 3's in-place retire; `ItemReport.operation` is
  in the Parking lot.
- [x] Settle, on rclone and unknown FUSE only: a removal that ends absent sets
  `gone_at` and keeps the record; a pass ten minutes later that still finds
  nothing clears it; whatever reappeared is removed as residue (defect 2).
- [x] A transaction folder with no readable `move.json` and nothing at its
  destination is removed (defect 12) — *as built:* one named by an operation
  id that holds nothing but a torn `move.json`; a complete but invalid one
  stays report-only. The temp-file sweep was dropped (Parking lot).
- [x] **A record belongs to its live worker, whatever data dir started it**
  (defect 20): an operation id names its worker's pid; that pid is alive, a
  fastf process, and started before the operation was minted (`/proc/<pid>/stat`
  start time; `OpenProcess` + `GetProcessTimes` on Windows) → the record is
  the worker's, and reconcile and the attention scan leave it alone. The data
  dir's `jobs/` stays the first answer.
- [x] `util::fs_kind` (Local, Nfs, Smb, Sshfs, Rclone, OtherFuse, Unknown; statfs
  plus `/proc/self/mountinfo`, `GetDriveTypeW`/`GetVolumeInformationW`),
  memoised — *as built:* on Linux it reads only `/proc/self/mountinfo`, never
  the mount, so it needs no timeout. `fs_retry::classify` (Gone, Denied, Locked,
  Transient, NotConnected, Full, ReadOnly, NameRefused, Other).
  `links_hidden_in` memoised per base and skipped on Local and NFS.
- [x] Logs: a `trace` level for per-entry lines, formatted in the progress lock
  and written after it; the job log keeps one handle and rotates at 16 MiB;
  `jobs::prune` caps the total at 64 MiB; `JobPhase`/`JobStatus` get
  `#[serde(other)] Unknown` (defect 16).
- [x] Failpoints: `faults::check_io(name)` with modes
  `eio|enotconn|enotempty|estale|eacces|ebusy[-N]` (fail N times, then pass);
  points `move:after-publish-write`, `presence:lstat`, `walk:readdir`; the
  crash-recovery registry scan learns `check_io`. *As built:* `walk:readdir`
  moves to Phase 2 with the walk it would sit in; `fs:as-rclone` (a decision:
  every folder reads as rclone) was added for the settle's tests.
- [x] Tests: a reconcile from a second data dir during a move leaves the move
  alone (a real second process, as the lab found it); an old copy under an unreadable base keeps its record;
  `presence:lstat:eio` on R keeps it; a split rename is finished on both
  halves; a `published.json` without `PROJECT_INFO.md` finishes; a moved copy
  missing entries after the retire is completed from R; an orphan R is found
  through the index when its base is no longer configured; settle with
  `gone_at` old and young; `move:after-publish-write` in the abort points;
  the first delete case in `tests/jobs.rs` (worker killed mid-removal,
  reconcile clears it); a 2000-entry move's job log stays under 200 KB at the
  default level; trace gating and job-log rotation.

Acceptance: no path clears a record while anything it owns is present or
unknown; the lab's drop and resurrect scenarios leave no recordless folder.

## Phase 2 — the engine runs in parallel

- [x] `util::pool`: a queue on `std::thread::scope` with a stop flag and a
  first-error slot; each worker runs under the spawner's fault arming
  (`faults::current()`/`with_arming`, no-ops in release); concurrency from
  `fs_kind`; a `pool:serial` decision point keeps pacing tests meaningful.
  *As built:* `-<n>` counts are shared by every thread armed from one arming;
  a pool started on a worker runs inline; a system out of threads still gets
  the work done.
- [x] `Walk::of_with` on the pool over a `(dir, depth)` queue, with the same
  `Problem` rules; entries and problems sorted at the end, so its output is
  unchanged. *As built:* a listing hands its entries on in batches of 32, so a
  flat folder of twenty thousand files spreads over every worker; failpoints
  `walk:readdir` and `walk:lstat`.
- [x] Copy on the pool: folders and links first as now, then files in
  parallel, each `create_new|O_NOFOLLOW`, its mode and mtime set from the
  handle, then its fsync. Folder modes and times deepest-first after the
  publish, best effort (defect 17). Removal (deletes, unpublished copies) in
  parallel: files, then folders by descending depth. *As built:* folders are
  made a level at a time; the project folder itself keeps the target's
  defaults (the bookkeeping rewrites its `PROJECT_INFO.md` next). The removal
  is `core::removal`: each entry is examined and removed by the same worker,
  one right after the other, and asked of a `Judge` (`Recorded`,
  `Everything`; the merge plugs in here in Phase 3); a folder something was
  left in is never asked to go.
- [ ] Fewer walks: in-process, the pre-retire whole-tree checks of S and F go
  (the merge proves each removal), and so does the walk that only counts
  leftovers. Target: S walked twice, F twice, R once (defect 15). *As built:*
  the counting walk now runs only when something is left; the pre-retire
  checks go with Phase 3's merge, which is what replaces them.
- [x] Tests: the parallel walk equals a sequential reference, problems and a
  70-deep tree included (property test); an arming trips inside pool threads;
  `+x`, file and folder times survive a move; Windows read-only files and
  junctions under parallel removal.

Acceptance: the lab's before/after table and concurrency sweep are recorded;
an old copy on R2 is removed at least ten times faster. *Met in part:* the
table is below; the R2 check and removal together are 9.5 times faster
(Phase 3's merge removes the separate check). The sweep waits for the merge.

## Phase 3 — the retire is a merge; changes during a move are kept

- [x] `core::merge`: a pure `decide(manifest, published, F, R, policy)` →
  `Remove | CompleteThenRemove | ReplaceThenRemove | CopyNewThenRemove |
  Keep(reason)`, and `merge_remove` on the pool (F folders made before their
  children, `rmdir` by descending depth, the moved copy's identity checked
  every 500 entries, writes into F through `atomic::copy`, never
  `PROJECT_INFO.md`). Policy `Full` (R after a clean retire, within an hour of
  the publish) may complete, fast-forward and copy new files into F;
  `Residue` (a split rename's S, anything that reappeared after `gone_at`, a
  3.11 record, anything later than the hour) removes only what matches the
  manifest and is covered by F, and never writes into F. What is left is the
  conflicts; the record stays, and the verdict goes to `attention.json`. This
  replaces "kept whole" (defect 3): core CLAUDE.md and `docs/projects.md` are
  rewritten. *As built:* plus `CompareContent`, for a size that agrees and a
  time that moved. **Writes into F never go through `atomic::copy`**, whose
  temp-and-rename a cloud mount misplaces: each is `create_new` at its final
  path, announced first by a create-only `write.<n>` marker in the record, so
  a torn write is redone by the next pass. The merge plugs into
  `core::removal` as a `Judge`; the verdicts go to the report's leftovers now
  and to `attention.json` with Phase 5.
- [x] Absorbing source changes before publishing: the source check compares
  with `Match::Whole`; a file changed before its copy is copied as it is;
  `delta_round` re-copies what changed or is new and removes what vanished,
  up to three rounds; `manifest.json` is written once the source settles; a
  source that never settles fails the move naming the files (defect 14).
  *As built:* `transactions::settle_copy`; each file is recorded as copied
  (its time from its own handle, its size the bytes that arrived). **A source
  that never settles does not fail the move** — a dev server appends to its
  log every tenth of a second, and it is neither a permission nor a lock:
  after the third round the copy is published as that round left it,
  consistent with its record, and the merge carries the rest.
- [x] Retire strategy from `fs_kind(source base)`: rename on local, sshfs, SMB
  and NFS; on every rclone mount in place — pointer, S's `PROJECT_INFO.md`
  unlinked (only if it matches the manifest, else kept whole),
  `phase.Retired`, bookkeeping (discovery trusts cached rows by `is_dir`, so
  the source index must drop the row), `merge_remove(S, Full)`, the pointer
  last. `move.json` carries `retire: "in-place"`. The old folder name stays
  taken until housekeeping finishes, and a collision says so. *As built:*
  unknown FUSE mounts go in place too; "matches" is by time or, failing that,
  by text without `path`/`folder`.
- [x] Delete: the same switch; in place it writes `.fastf-deleted-<op>.json`
  with the manifest, unlinks `PROJECT_INFO.md`, and removes only what the
  manifest lists. The walk under the data lock becomes a mountinfo lookup on
  Linux (defect 11). *As built:* a local disk is still walked for btrfs
  subvolumes, which mount nothing; a delete record waits out the settle too.
- [x] Recordless old copies left by 3.13 or earlier: removed automatically when
  the index or the job history proves the move (what is identical in the
  moved copy by kind, size and content; `PROJECT_INFO.md` compared without
  `path`/`folder`), otherwise an explicit action. *As built:* the proof is the
  content itself — every entry the project with that ID holds the same, byte
  for byte, goes, whoever moved it; one holding nothing but folders goes; a
  project fastf cannot find keeps it all. The explicit action is Phase 5's.
- [x] Fewer walks (from Phase 2): nothing walks the original or the moved
  copy before the retire; the merge walks the moved copy once and the old
  copy once. S twice (scan, settle), F twice (verify, merge), R once.
- [x] An empty folder at the move's own target name gives way
  (`transactions::clear_target`).
- [x] Tests: the `decide` table for both policies; after a merge every removed
  entry is in F, same kind, not older than published; `Residue` never writes
  into F; a temp file created and removed mid-move does not fail it; an append
  during the copy ends up whole in F; `.astro/dev.log` written into R after
  the retire is fast-forwarded and R disappears; a file changed in both keeps
  only that entry and the record; in-place retire and delete killed at every
  point are finished; no `.fastf-moved-*` folder or pointer survives the abort
  points under either strategy.

Acceptance: the lab's dev-server scenario moves without failing and leaves
nothing behind; the resurrect scenario leaves nothing after ten minutes;
`editmove-r2` (edited on R2, moved away seconds later) finishes with nobody
asked — an entry whose size agrees and only its time moved is compared by
content before it is called changed, since a cloud mount's rename or upload
can move a time; and an empty folder a mount left at the move's own target
name (rclone re-creates directory markers) does not block the move.

## Phase 4 — only what fastf cannot fix stops a move

- [ ] `fs_retry::with_retry` by class: Transient and Locked back off 0.2 s →
  5 s, at most six tries; NotConnected waits for the mount (the probe, the pool
  paused); Denied gives the owner write once, then stops — before publishing
  the move fails naming the path, after it the entry is kept and needs you;
  Full, ReadOnly and NameRefused stop; ENOTEMPTY on an rmdir lists the folder
  again and sends what appeared through the merge. Wired into the copy, the
  walk, the merge, the record's removal and the probe (defect 13).
- [ ] `core::holders` (Linux): `/proc/*/{fd,fdinfo,cwd}`, fastf's own processes
  skipped by exe; a writer refuses the move before copying, named; a
  working-folder holder is a note in the result; an unsettled delta round
  names them. The worker, and the app before starting a job, step out of the
  project.
- [ ] Stalls: pool workers stamp each call; `Progress.stalled_ms`/`stalled_on`
  drive "no answer from <mount> for N s" in the dialog and on the command line.
- [ ] Pause and resume: when the wait runs out before publishing, a `paused`
  marker and a clean exit (waiting); re-running the move or housekeeping
  adopts F — entries whose size and mtime match are kept, the rest replaced,
  extras removed. Only a cancel rolls back.
- [ ] Messages: one sentence, the category and the next step; errno lists go
  to the report and the log.
- [ ] Tests: the `classify` table; `with_retry` with scripted closures;
  `remove:unlink:enotconn-3` resumes; `copy:write:eio-1` recopies;
  `remove:unlink:eacces` names the path and keeps the record; a child holding
  a file open for writing refuses the move naming it, a child only sitting in
  the folder is a note; a stall shows in `state.json`; a paused move resumes
  copying only what is missing.

Acceptance: in the lab only the planted permission and lock scenarios stop a
move, each naming the path and the program; drops and stalls end moved.

## Phase 5 — fastf finishes its own leftovers; attention means you

- [ ] `provisioning::attention(cfg)`: typed items (kind, project, path, base,
  record, phase, reason, state `Auto | Waiting | NeedsYou`, actions), each base
  read under a timeout — a silent one is a waiting item, not skipped;
  `list_incomplete` becomes a wrapper; verdicts persist in `attention.json`.
- [ ] Reconcile gains `Scope { All, Auto }`; the per-category advice moves from
  `cli/reconcile.rs` into core as data; locked operations: finish, resume,
  remove what the copy holds, discard (typed word), take the old or keep the
  new version of a conflict.
- [ ] The app: an automatic reconcile (`JobRequest.auto`) on the first
  attention answer with auto items and no live job, after any job ends, and
  every five minutes while items wait — reporting only new needs-you items;
  the header's `⚠ 2 need you  !` and a dim "finishing 3"; `!` opens an
  attention page with the items, their actions and the last reconcile report.
- [ ] `fastf reconcile --list`.
- [ ] Tests: the classification table; the app's automatic start, debounce, no
  start beside a live job, chip counts, actions to effects; snapshots of the
  page and the chip at 40×12 and 80×24; `reconcile --list` output.

Acceptance: after the lab's scenarios, with the app open, nothing is left and
nothing needs attention without a human decision.

## Phase 6 — the first frame never waits on a base

- [ ] Every base probed in parallel under one deadline; canonicalized under a
  timeout, falling back to the raw path.
- [ ] The summary split: templates and prefs (data dir), base probes, and
  attention, each with a generation (defect 18).
- [ ] Discovery per base: `Msg::DiscoveredBase { generation, base, rows |
  unresponsive }`, cached rows first, verified after; `install_base` keeps
  marks, meta and the cursor by path; `busy_bases` stops F5 stacking workers
  on a blocked base; a silent base is named in the header.
- [ ] `SizeScanner::drop` waits at most 200 ms, then detaches.
- [ ] The index remembers the base mtime its scan saw and is stale when the
  base's differs, instead of comparing its own file's mtime (defect 21) — and,
  since an rclone folder's mtime says nothing, it also compares the folder
  names it holds with a names-only listing of the base (one request).
- [ ] Tests: `paths:stall-base` (a debug decision, stalls only bases holding
  `.fastf-test-stall`); the pty suite sees the healthy base's rows within a
  second beside a stalled base, and quits in under one; per-base install,
  generations and F5 in the update suite.

Acceptance: with one base stopped dead, the others' rows are on screen within a
second and `q` returns at once.

## Phase 7 — Windows, docs, release

- [ ] Windows: holders through the Restart Manager (hand-declared
  `rstrtmgr` FFI, files in batches of 1000); a `CreateFileW(S, DELETE)` probe
  predicts a refused retire; the WinFsp rclone drive recognised; folder times
  with `FILE_FLAG_BACKUP_SEMANTICS`; `assert-standalone.ps1` allows
  `rstrtmgr.dll`. The VM scenarios: a handle without share-delete, a console
  sitting in the folder, parallel removal on NTFS, a cross-volume move, the
  rclone drive.
- [ ] Docs: `projects.md` (retire strategies, the merge and conflicts, the
  fidelity promise, recovery), `app.md` (the attention page, first paint),
  `cli.md` and `config.md` (`reconcile --list`, `trace`).
- [ ] The four CLAUDE.md files: the principles, arming inheritance,
  `check_io`, the recovery table.
- [ ] Release 3.14.0 through the `release` skill, on an explicit "release".

---

## Phase log

**Phase 0 (2026-09-26).** The problem project moved out of the working tree
into the private lab's fixtures; the lab lives beside the repository with its
own README. The first parallel run shared the bases between lanes and found
two defects the plan did not have: **20**, a second data dir's reconcile
discarding a live move's copy mid-write ("657 of the 1639 recorded entries
missing"), and **21**, the index race ("no project matches" right after a
project appeared). Lanes then got their own folder in every base. The 3.13.0
baseline reproduced defect 1 (rclone killed mid-removal: the move said
"moved", cleared its record, and left the old copy), 14 (a dev server and a
held file: "move source changed before copying"), 16 (job logs), 18 (one
frozen base: no rows in fifteen seconds) and 20 again, and measured the slow
paths. The numbers every later phase is compared with (gjiro is the
maintainer's vite project, 1640 entries):

| 3.13.0 | time | what happened |
|---|---|---|
| move out of R2 | 314 s | set aside 28 s, check 88 s, removal 194 s (≈120 ms an entry) |
| delete on R2 | 349 s | and 181 entries left ("Directory not empty", rclone's listing lag) |
| S3 at 10 req/s | 737 s | the one-rename set-aside alone (a copy and a delete per object) |
| sshfs, 20k files | 93 s | delete |
| into R2 and straight back | — | a recordless `.fastf-moved-*` back on the bucket 12 minutes later |
| rclone killed mid-removal | — | "kept whole, because reading … Transport endpoint is not connected" |
| every move | — | all 1472 file mtimes and `+x` lost; ~2.7 MB of job log |

**Phase 1 (2026-09-26).** As planned, with the deviations marked *as built*
above. Presence is three-way everywhere a removal or a record is decided;
`MoveTransaction::publication` answers the third way too, since an EIO reading
the moved copy's `PROJECT_INFO.md` read as "not published" and let `remove`
discard a published copy. An unmounted configured base is `waiting`, not
"needs a look". `crash_recovery`'s `move:after-transaction-create` case
pinned defect 12's old answer (retained, reported) and now asserts removal.
In the lab, against 3.13.0: rclone killed mid-removal (on the maintainer's R2
unit, and on the private S3) keeps the record, reconcile says it is waiting
while the mount is down and finishes the removal once it is back; a second
data dir's reconcile leaves a live move alone; job logs are 4–6 KB instead of
2.4–2.7 MB. What it cannot do yet: a project edited on R2 and moved away
seconds later ends "kept whole: PROJECT_INFO.md modified since it was scanned"
(rclone's object-by-object rename moves the file's time) — the record is kept,
but a person is still asked. That is Phase 3's merge and in-place retire; the
lab's `editmove-r2` is its acceptance test.

**Phase 2 (2026-09-26).** As planned, with the deviations marked *as built*
above. The lab against 3.13.0, same bases and fixtures (gjiro = 1640
entries; the sshfs base is the laptop over the LAN):

| scenario | 3.13.0 | Phase 2 | notes |
|---|---|---|---|
| move out of R2: check the old copy | 88 s | 7.8 s | |
| move out of R2: remove the old copy | 194 s | 21.8 s | 16 workers |
| move out of R2: whole job | 314 s | 62 s | the set-aside rename (31 s) is unchanged: Phase 3 |
| move out of sshfs | 42 s | 12 s | |
| move into sshfs | 54 s | 48 s | one `fsync` per file on the laptop's disk, and sftp-server answers one request at a time |
| delete on sshfs (gjiro) | 13.5 s | 4.4 s | |
| delete on sshfs (20k files) | 93 s | 50 s | |
| fidelity (content, modes, mtimes differing) | 0, 4, 1472 | 0, 0, 0 | every scenario |

The per-file `fsync` stays: it is what makes the moved copy durable before
the original goes, and on sshfs it is the server's disk that pays it. A
delete on R2 took 35 s and left nothing (3.13: 349 s and 181 entries left).

**Phase 3 (2026-09-26).** As planned, with the deviations marked *as built*
above; the two that change behaviour are that a source that never holds
still no longer fails the move, and that writes into the moved copy are
create-only with a marker instead of `atomic::copy`. The lab against 3.13.0
and Phase 2:

| scenario | 3.13.0 | Phase 2 | Phase 3 |
|---|---|---|---|
| move out of R2 (gjiro): set the original aside | 28 s | 31 s | 0.1 s (in place) |
| move out of R2: whole job | 314 s | 62 s | 24 s |
| move into R2 | 17 s | 17 s | 7 s |
| delete on R2 | 349 s, 181 left | 35 s | 28 s |
| a dev server writing during the move | fails: "source changed before copying" | fails | moves; its log and temp folders arrive; nothing left |
| a file held open and appended | fails | fails | moves (Phase 4 refuses it up front: its later writes would land in the old copy) |
| edited on R2, moved away seconds later | "kept whole" | "kept whole" | moves, nobody asked |

An old copy's record and pointer on rclone wait out the ten-minute settle, as
designed. The Phase 0 recordless copies on R2 were settled by reconcile: the
empty directory marker removed, the two whose project no longer exists kept
and named.

## Parking lot

- **Temp-file sweep (dropped from Phase 1).** `util::atomic` is documented to
  never sweep by name; a stray temp needs a kill mid-write, and the
  maintainer's leftovers were old copies, not temps.
- `ItemReport.operation`, so a many-item job's history names each item's
  operation (the orphan note uses one-item jobs today).
- A test for an old copy under a base the user cannot read (root-owned):
  `presence:lstat:eio` covers the logic, not the permission.
