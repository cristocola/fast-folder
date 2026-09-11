# CLAUDE.md — fastf test suites

**Every in-process suite uses `common::env`**: `with_fresh_install(&SERIAL, …)`
for one data directory, `with_sandbox(&SERIAL, …)` where a base is needed too;
fixtures live in `common::fixtures`. The rules those helpers enforce are at the
bottom, stated once.

**A file and a binary per subject — except the two app suites.** `cargo test`
runs binaries sequentially and links each one. The pty tests' fixed keystroke
schedules overlap only inside one binary (as three binaries they cost nineteen
more seconds), so `tui_pty.rs` is one binary with modules under `tests/tui_pty/`
(`app`, `list`, `flows`, plus the screenshot tool and its SVG renderer); and
`tui_update.rs` is one binary with a module per subject under
`tests/tui_update/`, which share `harness.rs`.

What each suite guards — the intent, not the case list:
- `create.rs`, `metadata.rs`, `search.rs`, `template_engine.rs`, `register.rs`,
  `move.rs`, `data_dir.rs` — the core flows, in process.
- `cli_counter.rs`, `cli_flags.rs`, `cli_output.rs` — what `fastf <args>` does,
  driven as a **real process**, because these defects live between clap and the
  core (a flag lost to `trailing_var_arg`, two callers computing an ID
  differently, a config field read raw).
- `crash_recovery.rs` — every create failpoint against the same invariants, plus
  subprocesses killed with abort. Debug-only.
- `concurrency.rs` — races real **processes**: a thread test passes against an
  in-process `Mutex` while production stays broken.
- `tui_update.rs` — the app's state machine with no terminal: a `tui::testing`
  fixture `App`, messages in, effects out. `tui_commands.rs` — the registry's
  invariants. `tui_snapshots.rs` — frames through `TestBackend`, mono theme, fixed
  sizes and dates, `/mnt/projects/…` paths, against `tests/snapshots/*.snap`;
  review a deliberate change with `INSTA_UPDATE=always cargo test --test
  tui_snapshots` and commit it.
- `tui_pty.rs` (unix) — the app and the command line's prompts through a real
  terminal: the runtime, the threads, the inline blocks.
  `tests/tui_pty/harness.rs` holds the anti-flake rules, above all that ratatui
  redraws only changed cells, so a frame is read through `app_screen` (a `vt100`
  replay), never matched in the raw stream; `pty::plain` is for cooked-mode
  output. `tests/tui_pty/screenshot.rs` is a tool, not a test:
  `FASTF_SHOT_KEYS="down enter" cargo test --test tui_pty screenshot -- --ignored
  --nocapture` prints the frame those keys leave in a planted sandbox
  (`FASTF_SHOT_REAL=1` for your own library, `FASTF_SHOT_ARGS="copy shared"` for a
  subcommand's inline prompt, `FASTF_SHOT_SIZE=80x24`, `FASTF_SHOT_SVG=<path>` for
  the README's SVG — sandbox only). **Look at every screen this way before writing
  its snapshot.**
- `relaunch.rs` (unix) — when fastf opens a terminal for itself and, mostly, when
  it must not (a pipe, a redirect, ssh, no display, either off switch, the loop
  guard). **Every test pins `config set terminal <recorder>` first**, so no run can
  open a real emulator. `common::recorder` appends its argv to a file (it also
  observes `notify-send`) and is **polled**, because fastf spawns and returns
  without waiting. `Sandbox::run_like_a_launcher` gives stdin `/dev/null` and one
  shared **socket** for stdout and stderr, as journald does.
- `term_cmd.rs` (unix) — `fastf term`: which emulator, argv and working directory
  it would open a shell with. **Same rule: every test pins the recorder first**;
  the recorder logs its working directory too, and a test sets `DISPLAY` itself
  where a window would need one.
- `layering.rs` — reads the source: `core`/`util` never prompt or print, only
  `tui::runtime` and `tui::inline` take the terminal, no key line is written by
  hand, the env guards stay single, `dialoguer` stays gone. An import is invisible
  to a runtime test.
- `windows_semantics.rs` — reserved names, trailing dots, control chars, unicode,
  >MAX_PATH, case-only rename, read-only files, a real sharing violation, junctions.
- `windows_live.rs` (windows; **opt-in**) — what no temporary directory can show:
  the move engine's **staged copy**, reached only by a real
  `ERROR_NOT_SAME_DEVICE`, and the **counter over a shared drive**. With either
  base unset it prints what it wanted and passes, so it is inert on Linux and in
  CI:

  ```powershell
  $env:FASTF_WIN_LOCAL_BASE = "D:\fastf-sandbox"                  # local NTFS
  $env:FASTF_WIN_SHARE_BASE = "\\yourserver\share\fastf-sandbox"  # an SMB share
  cargo test --test windows_live
  ```

  The bases must be on different volumes. Each case works in its own `live-…`
  folder and a temporary data dir; the real paths live in a runner script outside
  the repository. Every defect it finds also gets a cheap sibling CI runs, because
  **a suite CI never runs cannot be the only guard on a fix**.
- `hostile_fs.rs` — corrupt caches, markers and metadata, absent bases, vanishing
  paths: **degrade, never panic, never lose data.**
- `properties.rs` — proptest; above all, `sanitize_name` output is always
  creatable (verified by creating it).
- `repo_hygiene.rs` — no tracked file names a real home, a personal mount, a local
  project-folder path, the maintainer outside attribution, or a personal email.
  Scans `git ls-files`, and skips unless the crate is the checkout's root, because
  the AUR build unpacks inside an ignored directory of a real clone, where the
  scan would see nothing and fail `check()`.

`tests/common/mod.rs` is the process-driving harness: `Sandbox` (its own
`FASTF_INSTALL_DIR`, `HOME` redirected; `run`/`ok`/`fails`/`spawn`), `with_bases`,
`plant_project`, `unconfigured`, and `pty::run` (unix, `libc::forkpty`, 120×40),
because neither the app nor a prompt draws without a TTY. `#![allow(dead_code)]`,
since each binary uses a subset.

## Three ways a test passes over the thing it is for

**Do not read the artefact through the code that repairs it.** `discover` rescans
and rewrites the index on the way past, so asking it whether a restore worked can
only fail by luck. Read the file itself first.

**A test named after a screen asserts the screen is on it.** A key sequence that
lands one row off snapshots some other screen and catches nothing about this one.
Assert a word only that screen has (`frame.contains("one base per line")`) before
`snap()`.

**An assertion that restates its setup proves nothing.** Asserting a string holds
the id you formatted into it, or wrapping the output in an already-proven
sanitizer, passes whatever the code returns. Assert the result, in the caller's
units.

**Write the test against the broken build first.** One that passes before the fix
is a design guard and is labelled so; one that still fails after has found a case
the fix missed.

## Harness rules — every new harness follows all of them

- **`common::env` is the only module under `tests/` that calls `set_var` or
  `remove_var`, and `util::test_env` the only one under `src/`**
  (`tests/layering.rs`). Rust 2024 makes both unsafe and `setenv` is not
  thread-safe, so a second guard behind a second mutex races the first and every
  `env::var` in the binary.
- Go through `common::env::with_fresh_install` or `with_sandbox`: they take the
  binary's `SERIAL`, redirect `FASTF_INSTALL_DIR` and `HOME`/`USERPROFILE` into a
  `TempDir`, clear `FASTF_FAULT`, and **restore it all in `Drop`**, so a panicking
  test cannot hand the next one a deleted `HOME`. `with_sandbox` passes the guard
  to the body, which is how `crash_recovery` arms a failpoint.
- **A spawned fastf inherits nothing of whoever ran the suite**:
  `Sandbox::command` and `pty::run` drop `common::NOT_INHERITED`, then apply the
  test's own environment. A fastf-opened terminal exports `FASTF_RELAUNCHED` to
  every shell in it, which would otherwise fail the relaunch positives.
- **`HOME` is always redirected**: an unconfigured `base_dir` falls back to it, and
  a harness that skips it scans the developer's real projects and self-heals the
  counter from them.
- Each binary keeps its **own** `static SERIAL` — binaries are processes, so one
  lock each is necessary and sufficient. No suite needs `--test-threads=1`; keep
  it that way.
- A unit test under `src/` that reaches `DataLock` takes
  `util::test_env::EnvGuard::sandbox()` first, or `cargo test` locks the
  developer's real data dir (`install_dir().join(".fastf.lock")`), stalling any
  open `fastf` for the 30-second timeout and leaving a lock file behind.
- Lock order: `ENV_LOCK`, then `interrupt::TEST_LOCK`, which lives beside the
  process-global flag because a per-module mutex silently races. `faults` needs no
  lock — its arming is thread-local.
