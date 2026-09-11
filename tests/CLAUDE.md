# CLAUDE.md — fastf test suites

**Every in-process suite uses `common::env`**: `with_fresh_install(&SERIAL, …)`
for one data directory, `with_sandbox(&SERIAL, …)` where a project base is
needed too. Fixtures live in `common::fixtures`. The rules those helpers exist
to enforce are at the bottom of this file, stated once.

**A file per subject, a binary per subject — except the pty suite.** `cargo test`
runs test *binaries* sequentially, so splitting is free for fast suites and
expensive for slow ones: making the pty tests three targets added nineteen
seconds of wall time, because their fixed keystroke schedules stopped
overlapping. `tui_pty.rs` is therefore one binary with three modules under
`tests/tui_pty/` (`app`, `list`, `flows`), plus the screenshot tool and its
SVG renderer. Everything else is a target per subject.

The suites, and what each guards — the intent, not the case list:
- `create.rs`, `metadata.rs`, `search.rs`, `template_engine.rs`, `register.rs`,
  `move.rs`, `data_dir.rs` — the core flows, in process.
- `cli_counter.rs`, `cli_flags.rs`, `cli_output.rs` — what `fastf <args>` does to
  disk, driven as a **real process** because their defects lived between clap
  and the core (flags dropped into `trailing_var_arg`, one caller computing an
  ID differently from another, a config field read raw instead of resolved) and
  only a process sees that.
- `crash_recovery.rs` — every create failpoint asserted against the same invariants,
  plus real subprocesses killed with abort. Debug-only (failpoints are compiled out
  of release).
- `concurrency.rs` — races real **processes**, not threads: a thread test passes
  against an in-process `Mutex` while production stays broken.
- `tui_update.rs` — the guided app's state machine without a terminal: an
  `App` from `tui::testing`'s fixtures, messages in, effects out.
  `tui_commands.rs` — the command registry's invariants (one key means one
  thing per context, every command has a title and a help entry).
  `tui_snapshots.rs` — the frames, through ratatui's `TestBackend` in the mono
  theme at fixed sizes, against `tests/snapshots/*.snap` (`insta`; review a
  deliberate change with `INSTA_UPDATE=always cargo test --test tui_snapshots`
  and commit the file).
- `tui_pty.rs` (unix; modules `app`, `list`, `flows`) — the guided app and
  the command line's own prompts through a real terminal: the runtime, the
  threads, the inline blocks, which a test backend cannot see. `tests/tui_pty/harness.rs` states the
  rules that keep these from being flaky — above all that ratatui redraws only
  the cells that changed, so a frame is read back through `app_screen` (a
  `vt100` replay of the transcript), never matched in the raw stream.
  `tests/tui_pty/screenshot.rs` is the suite's one tool rather than test:
  `FASTF_SHOT_KEYS="down enter" cargo test --test tui_pty screenshot --
  --ignored --nocapture` drives the real binary with those keys in a planted
  sandbox (or your own library with `FASTF_SHOT_REAL=1`) and prints the frame
  it left on screen; `FASTF_SHOT_ARGS="copy shared"` drives a subcommand
  instead, which is how the command line's inline prompts are looked at;
  `FASTF_SHOT_SIZE=80x24` picks the window; `FASTF_SHOT_SVG=<path>` writes
  the frame as the SVG the README embeds (sandbox only — the repo is public).
  **Look at every screen you build this way** before writing its snapshot — it
  is what a person will see.
- `relaunch.rs` (unix) — when fastf opens a terminal for itself and, mostly, when
  it must not: a pipe, a redirect, an ssh session, a missing display, either off
  switch, and the loop guard all keep today's behaviour exactly.
  **Harness rule: every test in this suite pins `config set terminal <recorder>`
  first**, so no run can start a real emulator on a developer's desktop or on a
  CI runner that happens to have a display. `common::recorder` is that fixture —
  a shell script that appends its argv to a file — and it is also how
  `notify-send` is observed. It **polls** for the call: fastf spawns the terminal
  and returns without waiting, which is the whole point, so reading the log once
  tests the scheduler. `Sandbox::run_like_a_launcher` is the environment itself:
  stdin on `/dev/null`, stdout and stderr sharing one **socket**, because that is
  what journald gives a launcher's children and a socket is precisely what a pipe
  is not.
- `layering.rs` — reads the source rather than running it: `core` and `util` may
  not prompt, because the same functions serve scripted runs where there is no
  terminal to answer one; only `tui::runtime` and `tui::inline` may take the
  terminal; and `dialoguer` may not come back. An import is not something a
  runtime test can see.
- `windows_semantics.rs` — reserved names, trailing dots, control chars, unicode,
  >MAX_PATH, case-only rename, read-only files, a real sharing violation, junctions.
- `windows_live.rs` (windows; **opt-in**) — the two things no temporary
  directory can show, because both need genuinely different filesystems: the
  move engine's **staged copy**, which is reached only by a real
  `ERROR_NOT_SAME_DEVICE` and is otherwise exercised by a synthetic error or a
  failpoint; and the **ID counter over a drive two machines mount**, which is
  what it was designed around. It takes its two bases from
  `FASTF_WIN_LOCAL_BASE` (local NTFS) and `FASTF_WIN_SHARE_BASE` (an SMB
  share) and **ships with no defaults**: with either unset it prints what it
  wanted and passes, so it is inert on Linux, in CI, and on a machine that has
  no share. It found two Windows-only defects on its first run — a read-only
  destination blocking `atomic::write`'s publish, and a non-ASCII case-only
  rename refused as its own target — and both now have cheap siblings in
  `fs_retry`'s unit tests and `windows_semantics.rs`, because **a suite CI
  never runs cannot be the only guard on a fix**.
  The real paths live in a runner script beside the sandbox itself and never
  in a tracked file: `repo_hygiene.rs` forbids exactly that, and a suite that
  hard-codes one machine's layout is a suite only that machine can run.
- `hostile_fs.rs` — corrupt caches/markers/metadata, absent bases, vanishing paths:
  **degrade, never panic, never lose data.**
- `properties.rs` — proptest; above all, that `sanitize_name` output is always
  creatable (verified by creating it).
- `repo_hygiene.rs` — the repository is published, so no tracked file may name a
  real home directory, a personal mount point, a local project-folder path,
  the maintainer outside an attribution file, or a personal email address. Scans `git ls-files`; skips unless
  the crate directory is itself the root of the checkout. That is stricter than
  "is there a checkout": the AUR source package unpacks the release tarball into
  an ignored directory *inside* a real clone, where `git ls-files` succeeds and
  returns nothing — which the first version read as an empty repository and
  failed on, breaking `check()` for everyone building the package.

`tests/common/mod.rs` is the shared process-driving harness: a `Sandbox` that
owns its `FASTF_INSTALL_DIR`, redirects `HOME` into itself, and runs the built
binary (`run`/`ok`/`fails`/`spawn`), plus `with_bases` for multi-base fixtures,
`plant_project` for "this base already holds ID0082", and `unconfigured` for a
brand-new install with no base at all. It also carries `pty::run` (unix,
`libc::forkpty`, a 120×40 window) — neither the app nor a prompt draws without a
TTY, so the dashboard, confirmations and pickers are invisible to a pipe-based
test, which is exactly where the
rename prompt once spent a release offering one folder name and committing
another. `#![allow(dead_code)]` because each binary uses a different subset.

## Three ways a test passes over the thing it is for

All three cost a release each; all three are cheap to avoid.

**Do not read the artefact through the code that repairs it.** `reconcile`'s
case-rename test asked `library::discover` whether the project was back, and
discovery rescans and rewrites the index on the way past — so the assertion
could only ever fail where the base directory's mtime happened not to move. On
Linux a rename moves it and the test was green; the Windows leg found the real
defect, which was that the restore never told the index anything. Read the file
itself, **before** calling anything that would put it right.

**A test named after a screen has to assert the screen is on it.**
`settings_bases_as_text` pressed Down ten times and Enter, and ten is *Theme*,
whose Enter cycles the value where it stands and opens nothing. The snapshot
named after the bases editor had never contained one — so the test written to
look at the thing that later panicked could not have caught the panic. Assert a
word only that screen has (`frame.contains("one base per line")`) before
`snap()`.

**An assertion that restates its own setup proves nothing.** `properties.rs`
formatted `id: {id}` into a string and then asserted the string contained the
id; another wrapped `interpolate_name`'s output in the `sanitize_name` whose
output is already proved safe for arbitrary input, so the function under test
could have returned `../../etc` and passed. Assert the *result of the thing*, in
the units the caller uses.

**Write a test against the broken build first.** Several have passed pre-fix and
were relabelled as design guards rather than left looking like regressions they
are not — and several more caught a defect the fix was assumed to have covered.

Shared harness rules — every new harness must follow all of them:
- **`common::env` is the only module under `tests/` that may call `set_var` or
  `remove_var`**, and `util::test_env` is the only one under `src/`.
  `tests/layering.rs` reads the source and fails the build otherwise. `setenv`
  is not thread-safe at the libc level, so a second helper behind a second mutex
  looks like isolation and provides none — the lib genuinely had two, and they
  raced each other and every `env::var` in the binary.
- Go through `common::env::with_fresh_install` or `with_sandbox`. Both take the
  binary's `SERIAL`, redirect `FASTF_INSTALL_DIR` and `HOME`/`USERPROFILE` into
  a `TempDir`, clear `FASTF_FAULT`, and **restore all of it in `Drop`** — the
  restore used to be a line after `body()`, so a panicking test skipped it and
  the next test inherited a deleted tempdir as its `HOME`. `with_sandbox` hands
  the guard to the body, which is how `crash_recovery` arms a failpoint.
- **A spawned fastf inherits nothing of whoever ran the suite.**
  `Sandbox::command` and `pty::run` both drop `common::NOT_INHERITED` — every
  variable fastf reads that the test did not set itself. `FASTF_RELAUNCHED` is
  the one that proved it: the relaunch exports it onto the terminal it opens, so
  every shell in that window carries it and so does everything typed into that
  shell, and a package build started from such a window failed the four
  positives in `relaunch.rs` and nothing else in the crate. A test that wants one
  of these sets it itself; both spawners apply their own environment afterwards.
- Redirecting `HOME` is not optional: an unconfigured `base_dir` falls back to
  the home directory, so a harness that skips it scans the developer's real home
  and self-heals the counter from their real projects.
- Each test binary keeps its **own** `static SERIAL`. Separate binaries are
  separate processes, so one lock per binary is both necessary and sufficient.
  There is no `--test-threads=1` anywhere; keep it that way.
- A unit test under `src/` that reaches `DataLock` must take
  `util::test_env::EnvGuard::sandbox()` first. The lock path is
  `install_dir().join(".fastf.lock")`, so without it `cargo test` locks the
  developer's real data directory — blocking any `fastf` they have open for the
  full 30-second timeout, and leaving a lock file behind.
- Lock order when a test needs both: `ENV_LOCK` first, then
  `interrupt::TEST_LOCK` (the process-global interrupt flag, which lives beside
  the state it guards because a private mutex per test module silently races).
  `faults` needs no lock — its arming is thread-local.
