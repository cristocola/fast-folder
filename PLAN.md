# PLAN — moves as background jobs, with progress, messages and logs

Five phases on one branch, `feat/background-jobs`, one commit per phase, one PR
at the end, released as **3.13.0**. One phase per session unless the maintainer
sets a `/goal`. The gate list is the `release` skill's; run all of it between
phases. Publication (the version bump, the tag, the AUR) waits for an explicit
"release".

## Why

The maintainer moved a 1,473-file project from a Google Drive rclone mount to a
local base. The copy finished in minutes. Then the app sat at "finalizing" with
a full bar for ten minutes, and he force-killed it. The header said "1 needs
attention". He pressed `!` and watched a bare spinner for nine more minutes.
Nothing was wrong. fastf was removing the set-aside original through the mount,
one Drive API call per file, and showed nothing while it did. It did so twice:
once in the killed move, and again in reconcile.

He asked for three things:

1. **Everything long is verbose.** Every phase of a move is named and counted
   as it happens. A batch shows the overall position and the current item.
   Reconcile is a job with the same kind of progress.
2. **Messages and logs are two things.** Messages are curated human lines with
   a recommendation. Logs are everything, timestamped, per operation, down to
   the file, kept on disk, and readable in the app and from the command line.
3. **A move is a separate process.** It never blocks the app. Another fastf
   session started meanwhile sees it, its progress, its outcome, and can cancel
   it. Closing or killing the app never kills the move.

### Defects this plan fixes

| # | Defect | Where |
|---|---|---|
| 1 | Removing the old copy reports nothing; the phase stays `Finalizing` with a full bar for the whole removal | `move_engine.rs:457-522`, `move_cleanup.rs::remove_tree` |
| 2 | The scan, the probe, both verify walks, the retire and `check_removable`'s walks report nothing | `move_engine.rs:354-390`, `transactions.rs::Walk::of` |
| 3 | Reconcile has no progress and no cancel; the app shows a bare spinner | `provisioning.rs:378-440`, `tui/runtime.rs:1258` |
| 4 | `Progress.status` never becomes `Failed` or `Cancelled`; `error`, `warning`, `cleanup_pending` are never written; a failed move is polled every tick | `core/assets.rs:40-117`, `tui/runtime.rs:347-361` |
| 5 | Copy-to in the app shows no progress and cannot be cancelled: no `MovingJob` is registered for it | `tui/runtime.rs:430-452`, `tui/app/mod.rs:2373` |
| 6 | A move is a thread of the app or the CLI; killing either kills the move mid-flight | `tui/runtime.rs:430`, `cli/move_project.rs:183` |
| 7 | `DataLock` is held through the removal of the old copy, so every other mutation waits 30 s and fails for as long as a cloud removal takes | `move_engine.rs:153`, `provisioning.rs:419` |
| 8 | A second session cannot see a running move, and counts its live record as "needs attention" | `provisioning.rs::list_incomplete` |
| 9 | The publish still polls cancel although the comment says cancel is too late; after publication a cancel has no defined answer | `move_engine.rs:399-412` |
| 10 | The message log is 200 lines in memory, lost on exit; `L` is a snapshot that does not update | `tui/app/mod.rs:192-199, 645` |
| 11 | `diag` warnings from `core` persist nowhere | `util/diag.rs` |
| 12 | The lock wait says "another fastf process" without naming it | `util/lockfile.rs:67` |
| 13 | A batch move is driven by the app one item at a time, so closing the app abandons the rest | `tui/app/jobs.rs:274` |
| 14 | `fastf delete` on a cloud mount removes the tree synchronously with no count | `core/library/lifecycle.rs` |

## The design in one page

**Principles** (these go into `src/core/CLAUDE.md` and the root `CLAUDE.md`):

1. **A long operation is a job: a process of its own, described by files in the
   data dir.** Any fastf process can list, watch and cancel any job. None owns
   one. The data dir, not a base, because a base may be a cloud mount that
   uploads every write.
2. **The move is done when the original is set aside.** Removing the old copy
   is housekeeping. It runs after the data lock is released and after the
   outcome is reported. A cancelled or killed housekeeping pass is finished by
   reconcile, because the leftover is hidden and redundant by construction.
3. **A job is alive while it holds its own lock.** The same `flock` or
   `share_mode(0)` shape `DataLock` already uses, so the OS answers "is it
   running" on both platforms, a reused pid cannot lie, and there is no stale
   state to clean.
4. **Messages say what happened and what to do. Logs say everything.** A
   message is one or two sentences with a recommendation. A log line has a
   timestamp, a level, a job id and a fact.

### Files

```
<data dir>/
  jobs/<job-id>/
    request.json    written once by the spawner (create_new): kind, items, targets
    lock            held by the worker for its whole life
    progress.json   rewritten atomically by the worker, at most 4× a second
    cancel          created by anyone who wants it stopped
    seen            created by the first surface that reported the outcome
    log             this job's full log, debug level, every file
  logs/fastf.log    every process, info level; rotated at 4 MiB, 3 kept
  messages.log      curated lines, JSON per line; rotated at 512 KiB
```

The job id uses the operation-id shape (`transactions::next_operation_id`). One
directory per job, so pruning is one removal. Pruning (any surface, when it
lists): terminal, seen, lock free, and older than 30 days or beyond the newest
50.

`progress.json` (version 1, a tolerant reader: unknown fields ignored, a
missing one defaulted) holds: kind, pid, started, updated, state (`starting |
running | done | failed | cancelled`), `holds_lock`, the items with each item's
project id, name, from, to and outcome, the current item index, and the current
step. A step is phase, done, total, bytes done, bytes total, current path and a
note (such as "waiting for the move of X"). When the job ends it adds the
outcome: the curated message, any warning, and the list change a surface should
make. A job whose state is `running` while its lock is free is read as
**interrupted**: "the move of X stopped when its process ended; Reconcile
finishes it".

### The worker

`fastf --fastf-job <id>`, stripped from `argv[1]` before clap in `main.rs`,
the way `take_the_relaunch_flag` strips `--relaunched`, so completions never
offer it. It is dispatched before bootstrap and before clap, on both platforms.
The worker:

1. takes `jobs/<id>/lock`;
2. reads `request.json`;
3. sets `diag`'s sink to the log;
4. starts a watcher thread that snapshots the engine's `Mutex<Progress>` into
   `progress.json` every 250 ms and on each phase change, and turns a `cancel`
   file into the engine's `AtomicBool`;
5. runs the items;
6. writes the outcome and appends the message.

A panic is caught and becomes `failed` plus a log line. The worker never prints.

**Spawning** (`util::job::spawn`) passes `FASTF_INSTALL_DIR` explicitly, so the
worker uses the spawner's data dir in portable mode too.

- **Unix:** `current_exe`, stdio to `/dev/null`, and `setsid()` in `pre_exec`,
  so no terminal's SIGHUP or SIGINT reaches the worker. The worker also ignores
  SIGHUP; SIGTERM becomes a cancel.
- **Windows:** `creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP |
  CREATE_BREAKAWAY_FROM_JOB)`, retried without breakaway where the job object
  refuses it. std's `CommandExt` covers this, so no FFI is needed. Breakaway
  matters because OpenSSH on Windows kills its session's job object on
  disconnect.

The spawner waits up to two seconds for `progress.json` to say `running`. If it
never does, the spawn failed, and the surface says so with the log's path.

### The engine's side

The engine API keeps `&Mutex<Progress>` and `&AtomicBool`. What changes:

- **`JobPhase`** names every step: `Starting`, `Waiting` (for the data lock),
  `Scanning`, `Probing`, `Copying`, `Verifying`, `Publishing`, `SettingAside`,
  `Checking` (the old copy, before its removal), `Removing`, `Clearing`, `Done`.
- **`Progress`** gains a generic step count (`step_done`/`step_total`), the
  planned `steps` and the `finished` ones with their counts, `item` (n of N) and
  its label, `operation` (the record's id, once known), `committed` (past the
  point of no return), and in Phase 3 `holds_lock`. It derives `Deserialize` too. `status` finally reaches `Failed`
  and `Cancelled` on every path, one helper in each engine.
- **A `core::progress::Ticker`** is handed to the walks and to `remove_tree`:
  entries counted, cancel polled, one debug log line per entry. `Ticker::none()`
  keeps every other caller unchanged. `remove_tree` counts n of the manifest's N
  (defect 1). A delete, which has no manifest, counts up with no total.
- **The lock is split** (defect 7). `move_project_*` returns after bookkeeping
  with a `Housekeeping` value (retired path, manifest, record). The caller drops
  the lock and then runs `move_cleanup::finish_housekeeping(hk, ticker)`. The
  in-process compatibility path runs both, so the library tests keep their
  meaning. Every thousand entries, and before each top-level folder, the removal
  checks that the moved copy still exists. If it is gone the removal stops, a
  kept-on-purpose leftover, because the retired copy may be the only one.
- **Cancel** before `published` rolls back, as today. After it, the answer is
  "too late: X is moved; the old copy's removal carries on" (defect 9). The
  publish copy no longer polls cancel.
- **Reconcile and `list_incomplete` skip a record whose job is alive.** An
  operation id carries its maker's pid, and a worker writes its pid first, so
  a record made by a live worker is that job's (`jobs::live_workers`,
  `jobs::owned_by`).
  The header counts running jobs apart from attention (defect 8). Reconcile
  decides and retires under the lock, then runs its removals as housekeeping
  after it, the same shape as a move.
- **`DataLock::acquire_waiting(cancel)`** waits indefinitely for a worker and
  stays cancellable. The existing 30 s wait names the job that holds the lock
  when a live one says `holds_lock` (defect 12).
- **Delete** retires under the lock, as today. The removal of
  `.fastf-deleted-<op>` becomes housekeeping in a job (defect 14).

**Batch:** one job, N items, run in the worker (defect 13). The moves come
first and the housekeeping after, so every project reaches its new base before
any old copy is removed. The data lock is taken per item, so other mutations can
land between items.

### Surfaces

- **CLI:** `move`, `copy-to`, `delete` and `reconcile` spawn a worker and
  attach to it.
  - On a TTY they draw one line per step, named and counted. On a pipe they
    print one line per phase change.
  - Ctrl-C before publication writes `cancel` and waits for the answer. Ctrl-C
    during housekeeping detaches and prints how to watch it.
  - `--detach` prints the job id and returns.
  - New verbs: `fastf jobs` lists, `fastf jobs watch [id]` attaches, and
    `fastf jobs cancel [id]` stops a job.
  - `fastf log [job] [--lines N] [--follow]` shows a log, and `fastf messages
    [--lines N]` shows the curated lines.
- **The app:** a job runs in its own process, and the runtime's watcher reads
  `jobs/` on a worker thread: once a second when idle, five times a second while
  a job it shows is live. It hands `Msg::Jobs` to `update`. The moving dialog
  becomes a **job dialog** with one row per step: done steps ticked with their
  counts, the current one with its bar and file, the rest dim.
  - Esc hides the dialog. The job carries on, and the header shows a quiet
    chip, a spinner and "moving Lullaby · 312/1473".
  - Ctrl-C cancels before publication and says "too late" after.
  - Verbs that need the data lock are `Disabled` with a reason while a live job
    holds it. Browsing is never blocked.
  - A job that ended while no surface watched, with no `seen` file, lands as a
    message at startup.
  - `L` opens an **activity** screen with three tabs: Jobs (live and recent,
    Enter opens that job's log), Messages, and Log. All three read the files
    and refresh while open.

## How to work a phase

Scope is that phase only; anything else found goes to the Parking lot. Write
each test against the broken build first (`tests/CLAUDE.md`). If it passes
before the fix, label it a design guard. Read results from the disk, not through
`discover`. Look at every new frame with the screenshot tool before blessing its
snapshot.

Run fastf on the host with `FASTF_NO_RELAUNCH=1` and output piped. Gates
between phases:

- `cargo fmt --all -- --check`;
- `cargo clippy --all-targets -- -D warnings` in debug, `--release` and
  `--target x86_64-pc-windows-gnu`;
- `cargo test` and `cargo test --release`;
- `rm -rf target/doc && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`.

Update `docs/` and the CLAUDE.md beside the code in the same commit. Tick a box
only when it is verified, and record what happened in the Phase log.

**A slow job on demand:** `util::faults` gains a `delay-<ms>` action, debug
only like the rest, plus two per-entry points: `move:each-file` in
`copy_contents` and `remove:each-entry` in `remove_tree`. So
`FASTF_FAULT=remove:each-entry:delay-20` makes any test's removal take visible
time. Both go into `ALL_FAULT_POINTS`.

---

## Phase 1 — every step named and counted, in process

No process split yet. Both surfaces show the new steps through today's threads.

- [x] `JobPhase` extended; `Progress` gains the step count, the plan and the
  finished steps, item, operation and `committed` (`holds_lock` moved to Phase
  3, which uses it), derives `Deserialize`, and reaches `Failed`/`Cancelled` on
  every path (defect 4). `as_str` and `past` give the words a person reads.
- [x] `core::progress::Ticker`, threaded through `Walk::of` (a `_with` entry
  point; the depth stays threaded), `remove_tree` and `check_removable` (the
  checks before the retire and before the removal). `Ticker::none()` everywhere
  else. The probe and `complete_destination`'s copies are short and stay
  uncounted (Phase log).
- [x] `move_engine` and `copy_engine` set every phase in order. `remove_tree`
  counts n of the manifest's N. The publish no longer polls cancel, and a cancel
  after publication gets its "too late" answer (defect 9).
- [x] Reconcile takes a `&Mutex<Progress>` and `&AtomicBool`: the item is the
  record (n of N over every base), the step is the record's own phase, and
  cancel is honoured between records and inside a removal.
- [x] The failpoint `delay-<ms>` action and the two per-entry points.
- [x] Surfaces: `cli::move_project::draw` names the phase and its count. The
  app's move dialog lists the steps. Copy-to registers progress and cancel in
  the app (defect 5). Reconcile in the app uses the same dialog instead of the
  spinner (defect 3).
- [x] Tests: a staged move's progress passes through every phase in order (a
  recording `Ticker` sink); `remove_tree` counts reach the manifest's total; a
  failed and a cancelled move end `Failed`/`Cancelled`; a cancel after
  publication is answered and removes nothing extra; reconcile counts records and
  stops between them on cancel. Snapshots for the step dialog, before and during
  removal, looked at first.

Acceptance: no phase of a move or a reconcile runs longer than a second without
its count moving; "finalizing" is gone from every surface.

## Phase 2 — messages and logs on disk

- [x] `util::log`: levels `error warn info debug`; one line per event
  (`2026-09-25T14:03:11.123Z INFO  <job|-> <pid> text`); appended with one
  `write` per line (`O_APPEND`, `FILE_APPEND_DATA`), so several processes
  interleave whole lines; rotation under a try-lock that skips when busy. A
  process-global current job routes debug lines to `jobs/<id>/log`. The config
  key `log_level` defaults to `info` for the central log; a job's own log is
  always debug.
- [x] `diag::warn`/`note` also append to the log. Engine phases log at info,
  entries at debug through the `Ticker`, and failures at error with their cause.
- [x] `util::messages`: append, read the last N, tolerant reader. The CLI's
  outcome lines and the app's status lines are messages, with the source
  (`app`, `cli`, a job id).
- [x] `fastf log` and `fastf messages` (`src/cli/log.rs`). They read the data
  dir, so they stay out of the bootstrap guard.
- [x] The app: `set_status` also emits `Effect::AppendMessage`, since `update`
  does no I/O. `L` opens the activity screen with Messages and Log tabs, read by
  a loader and refreshed while open. `LOG_CAP` stays as the in-session mirror
  the status line counts from.
- [x] Tests: two processes appending 1,000 lines each leave 2,000 whole lines;
  rotation keeps the newest; a torn last line is skipped by the reader; `fastf
  log --lines 5` prints five; the activity screen's tabs, snapshots looked at
  first; `layering.rs` still holds (`log.rs` writes files, never stdout).

Acceptance: after any move, `fastf log` shows its phases with counts and
`fastf messages` its one-line outcome, across a restart.

## Phase 3 — the job: a process of its own

- [x] `util::job`: the files above, `spawn` (unix `setsid`; Windows flags with
  the breakaway retry), `JobLock` (the `DataLock` shape, made `pub(crate)` and
  reused), `list`, `live_operations`, `request_cancel`, `mark_seen`, `prune`.
- [x] `main.rs` strips `--fastf-job <id>` before clap; `cli::job_worker::run`
  is the worker as designed.
- [x] The lock split and `Housekeeping`; reconcile and delete run their
  removals after the lock; reconcile and `list_incomplete` skip live jobs'
  records (defects 7, 8, 14). `DataLock::acquire_waiting(cancel)`; the 30 s wait
  names the job (defect 12).
- [x] `Progress.holds_lock`, set while the worker holds the data lock.
- [x] Batch jobs in the worker: moves first, housekeeping after, the lock taken
  per item.
- [x] CLI: `move`, `copy-to`, `delete`, `reconcile` through the worker, attached;
  `--detach`; `fastf jobs [watch|cancel]`. The worker gets its data dir
  explicitly, so the test harness's `NOT_INHERITED` list needs no new entry.
- [x] Tests, as real processes:
  - a SIGKILLed CLI `move` leaves the worker to finish (poll `progress.json`,
    then read both bases from the disk);
  - a SIGKILLed worker mid-copy reads as interrupted, and reconcile rolls it back;
  - killed mid-housekeeping, reconcile finishes the removal;
  - `fastf jobs cancel` from a second process before publication rolls back,
    and after it answers "too late";
  - reconcile run during a live job's housekeeping leaves that record alone and
    reports no attention for it;
  - a tag added during housekeeping lands at once, and one added during the
    copy waits and names the job;
  - two detached moves serialise on the data lock, the second saying
    `waiting`.

  `crash_recovery.rs` gains the worker as a subject: its abort points run
  through `--fastf-job`.

Acceptance: the first defect's exact scenario, simulated with
`remove:each-entry:delay-400` over 1,473 files, returns the CLI's "moved" line
as soon as the original is set aside, and removal carries on with its count
after the CLI is killed.

## Phase 4 — the app watches jobs

- [x] The runtime's jobs watcher beside `watch_detail`, on a worker thread,
  latest wins; `Msg::Jobs`; `App.jobs`; `tick_interval` asks for the slow tick
  while a shown job is live.
- [x] `Action::Move`, `CopyTo`, `Delete`, `Reconcile` and the batch verbs spawn
  jobs (`Effect::StartJob`, `Msg::JobStarted`); `MovingJob`, `move_progress` and
  the in-app batch runner for those kinds go. Tag, note and the other quick
  verbs stay in-process batches.
- [x] The job dialog (steps, Esc hides, Ctrl-C cancels or says "too late"); the
  header chip; the Jobs tab of the activity screen with Enter on a job opening
  its log; outcomes of jobs nobody watched as messages at startup; a job's end
  reloads the list and the summary.
- [x] Availability: the verbs that take the data lock are `Disabled("a move
  holds the library until it is copied — L shows it")` while a live job says
  `holds_lock`; `command.rs` declares the new keys, `guide.rs` any new words.
- [x] Tests: `tui_update` (a job's progress drives the dialog; Esc hides it and
  the chip stays; a finished unseen job becomes one message; a lock-holding job
  disables the right verbs); snapshots of the dialog in each step, the chip and
  all three tabs at 40×12 and up (`every_state_draws_at_every_size`); pty:
  **quitting the app mid-move leaves the move running to completion**, and a
  second app started meanwhile shows the same job.

Acceptance: move a project, press `q`, start `fastf` again, and the job is
there with its count; kill that app too, and the move still finishes.

## Phase 5 — real mounts, Windows, docs, ship

- [ ] The rclone lab (an S3 remote served locally, mounted with
  `--vfs-cache-mode full --tpslimit 2`): a 1,473-file move from it, watched in
  the app and from a second terminal, with the app closed mid-removal. Record
  the times in the log.
- [ ] The maintainer's real Drive mount, release build: the same move, verified
  against Drive with `rclone lsf`, never only through the mount.
- [ ] The sshfs base, if mounted: a move out of it and a reconcile.
- [ ] Closing the terminal window itself, a desktop emulator, with a move
  running: the worker survives its window's scope.
- [ ] Windows VM over ssh: clippy, unit tests, a detached move that survives the
  ssh session ending, and a cancel from a second session. Shut the VM down.
- [ ] Docs: `cli.md` (moving, reconcile, the jobs, log and messages verbs),
  `app.md` (the job dialog, the chip, the activity screen), `projects.md`
  (process-crash recovery, what fastf promises), `config.md` (`log_level`, the
  data dir's new files), `windows.md`. The four CLAUDE.md files.
  `.github/release-notes/v3.13.0.md`.
- [ ] A review subagent over the whole branch; every finding confirmed and
  fixed or parked with a reason.
- [ ] PR; on "release": the `release` skill for 3.13.0, both AUR packages;
  retire this file.

## Parking lot

- Housekeeping of one batch item running beside the next item's copy. On one
  rate-limited remote it buys nothing.
- A desktop notification (`util::notify`) when a job ends and no surface is
  attached.
- A row marker on a project a live job is moving.

## Phase log

- 2026-09-25 — plan written from the maintainer's Drive move; branch
  `feat/background-jobs` cut from `main` at 9981ed6.
- 2026-09-25 — Phase 1. `core::progress` (`Ticker`, `settle`), threaded
  through `Walk::of_with`, `MoveManifest::scan_with`/`verify_*_with`,
  `Cleanup.ticker` and `remove_tree`; `JobPhase` names every step and
  `Progress` keeps the plan (`MOVE_STEPS`, `COPY_STEPS`), the finished steps with
  their counts, item n of N and `committed`; `cleanup_pending` and `warning`
  (never written) are gone. Reconcile counts `list_incomplete`'s items and stops
  between them or mid-removal (`ReconcileReport.cancelled`,
  `operations::reconcile_with`). `cli::progress::run_watched` is the one loop
  for `move`, `copy-to` and `reconcile`: a live line, and a line of record per
  finished step (the only output on a pipe). The app arms its dialog in
  `run_action` from `Action::reports_progress`, so copy-to and reconcile have
  progress and cancel now; the dialog draws a row per step, the current step
  alone in a short window. Three departures from the design: (1)
  `JobPhase::can_cancel` became `Progress::committed`, because a reconcile can
  stop mid-removal while a move cannot, so "too late" is the job's fact, not
  the step's; (2) a `Checking` step, since removing the old copy is preceded by
  two walks re-checking it, which would otherwise count to 2N under
  "removing"; (3) the publish step reads "publishing PROJECT_INFO.md", because
  the copy step counts every file but that one and the outcome counts all of
  them, and 1472 beside 1473 needed the explanation. Checks never stop for a
  cancel (`cleanup.ticker.uncancellable()`), only removals do. Verified by
  hand under a pty: a slowed staged move (`remove:each-entry:delay-25`) counts
  1 to 63 while removing, and a reconcile after a move killed at
  `move:after-retire` says `1 of 1 … removing the old copy 7 of 63 entries`.
  Uncounted still: the probe, the publish, the clearing (whose record removal
  can wait 20 s on `EIO`), and `complete_destination`'s copies. The late-cancel
  test was run against a build whose removal honoured the cancel, and caught it.
  Gates green: fmt, clippy debug/release/windows-gnu, test debug/release, doc.
- 2026-09-25 — Phase 2. `util::log` (levels, `format_line`, one `write` per
  event on an append-mode file, rotation under `DataLock::try_acquire_at`,
  `tail` reading only the file's end, `set_job` ready for Phase 3) and
  `util::messages` (JSON lines, tolerant reader, every message also logged).
  `diag` writes to the log; the `Ticker` logs each step once
  `Ticker::subject` names the job, each entry at debug. `log_level` is a config
  key and a settings row; `main` passes it to the log, since `util` may not
  read `Config`. `fastf log [-n] [--follow]`, `fastf messages [-n]`; `move`,
  `copy-to` and `reconcile` keep their outcome (and a failure) as messages, and
  reconcile's one sentence moved to `ReconcileReport::summary` so both surfaces
  say it alike. The app hands every status line to `App.outbox`, which the
  runtime writes after each `update`; `L` is `Modal::Activity`, two pages read
  from disk and re-read once a second while open. Decided on the way: Tab turns
  its page (it is the global `FocusNext` key, so no second binding), the page
  on show wears the cursor glyph so mono can tell; the waiting step is entered
  only when the lock is really held (`DataLock::acquire_then`), since the log
  said "waiting for another fastf" on every move; a unit's name agrees with its
  count ("1 file"). Messages rotate like the log (three kept). Verified with
  the screenshot tool in a real pty. Gates green.
- 2026-09-25 — Phase 3. `core::jobs` (request, `state.json`, lock, cancel,
  seen, prune; `start` spawns `fastf --fastf-job <id>` detached — `setsid` on
  unix, `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` with a breakaway attempt on
  Windows — and reaps it on a thread), `cli::job_worker` (moves first,
  housekeeping after, per-item progress, a watcher thread writing state four
  times a second and turning `cancel` into the flag, patient for the data
  lock), `cli::jobs` (follow a state file with `cli::progress::Printer`, Ctrl-C
  cancels or lets go, `finish` prints what the verbs always printed) and `fastf
  jobs [watch|cancel]`, `--detach` on the four verbs. The lock split:
  `move_project_in_parts` / `finish_housekeeping`, `move_cleanup::set_aside` +
  `Housekeeping` (old copy or deleted folder), `delete_project_in_parts` /
  `finish_delete`, and reconcile's `Deferred` removals after its lock. Decided
  on the way: **a live job's records are found by pid, not by a list of
  operations** — the operation id carries its maker's pid and a worker writes
  its pid before any work, so there is no window between a record's birth and
  its owner being known, which a state rewritten four times a second would
  leave; `SourceOutcome::SetAside` for "done, old copy going"; a lock wait
  names its holder through a hook `main` sets, since `util` may not read
  `core`. The worker-as-subject cases live in the new `tests/jobs.rs` (nine
  process tests, stable over five runs) rather than in `crash_recovery.rs`,
  whose in-process driver still covers every abort point. Verified by hand:
  `kill -9` on `fastf move` mid-copy, and the job finished the move. Gates
  green, the Windows clippy leg included.
- 2026-09-25 — Phase 4. `app::background` (the jobs the app knows of, the one
  its dialog follows, start/started/ended, hide and cancel), the runtime's
  `watch_jobs` and `Effect::{StartJob, WatchJobs, CancelJob, MarkSeen,
  LoadJobLog}`; `MovingJob`, `move_progress` and the in-process `Move`,
  `CopyTo`, `Delete` and `Reconcile` actions are gone, and the in-app batch
  runner keeps only the quick verbs. The dialog reads the followed job's state;
  Esc hides it, Ctrl-C cancels or says too late, `q` quits and leaves the job;
  the header's chip (`moving ID0248 · copying 12 of 34 files`); `L` gained a
  jobs page with a cursor, Enter opening that job's own log; `not_busy` dims
  the mutating verbs while a live job holds the lock. Decided on the way: a
  finished delete drops its rows by path (no rescan, the promise the pty suite
  keeps), a finished move reloads (its rows land in another base, and a job's
  state says where, not what the row now is — the batch-move pty test now
  expects that one read); jobs other surfaces started are reported only as a
  reload, except on the first look, when what ended unseen while no app was
  open is said. New tests: the state machine (start, follow, end, the startup
  report, the dimmed verbs), snapshots of the jobs page and the chip, the frame
  sweep over three job states, and a pty test that quits the app mid-move,
  starts a second app that shows the same job, and waits for the move to
  finish without either. Gates green.
