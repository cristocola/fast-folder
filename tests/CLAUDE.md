# CLAUDE.md — fastf test suites

There are **nine** integration binaries — `integration.rs` (core flows),
`ui_server.rs` (browser-UI request layer), the five v1.1 suites
`crash_recovery.rs`, `concurrency.rs`, `windows_semantics.rs`, `hostile_fs.rs`,
`properties.rs`, the v1.2.1 `cli_surface.rs`, and the v1.3 `tui_pty.rs`. What
each guards — the intent, not the case list:
- `crash_recovery.rs` — every create failpoint asserted against the same invariants,
  plus real subprocesses killed with abort. Debug-only (failpoints are compiled out
  of release).
- `concurrency.rs` — races real **processes**, not threads: a thread test passes
  against an in-process `Mutex` while production stays broken.
- `cli_surface.rs` — what `fastf <args>` actually does to disk. Every case is a
  v1.2.0 regression: the bugs lived between clap and the core (flags dropped into
  `trailing_var_arg`, one caller computing an ID differently from another, a config
  field read raw instead of resolved), which only a process can see. **Write the
  test against the broken build first** — two of these passed pre-fix and were
  relabelled as design guards rather than left to look like regressions they aren't.
- `tui_pty.rs` (v1.3, unix) — the interactive menu through a real terminal, which
  is the only place its worst defect was visible: any recoverable error ended the
  session. Keystrokes must be **spaced**, not burst (`pty::Script` handles the
  cadence), and `Confirm` takes a bare `y`/`n` with no Enter — a trailing `\r`
  survives into the next prompt and silently accepts its default.
- `windows_semantics.rs` — reserved names, trailing dots, control chars, unicode,
  >MAX_PATH, case-only rename, read-only files, a real sharing violation, junctions.
- `hostile_fs.rs` — corrupt caches/markers/metadata, absent bases, vanishing paths:
  **degrade, never panic, never lose data.**
- `properties.rs` — proptest; above all, that `sanitize_name` output is always
  creatable (verified by creating it).

`tests/common/mod.rs` is the shared process-driving harness (v1.2.1): a `Sandbox`
that owns its `FASTF_INSTALL_DIR`, redirects `HOME` into itself, and runs the
built binary (`run`/`ok`/`fails`/`spawn`), plus `with_bases` for multi-base
fixtures and `plant_project` for "this base already holds ID0082". It also
carries `pty::run` (unix, `libc::forkpty`) — `dialoguer` refuses to prompt
without a TTY, so confirmations and pickers are invisible to a pipe-based test,
which is exactly where the rename prompt spent v1.2.0 offering one folder name
and committing another. `#![allow(dead_code)]` because each binary uses a
different subset. `concurrency.rs` and `cli_surface.rs` both `mod common;`.

Shared harness rules — every new harness must follow all of them:
- `FASTF_INSTALL_DIR` env var to redirect `paths::install_dir()` to a tempdir per test
- `tempfile::TempDir` for hermetic sandboxes
- **Redirect `HOME`/`USERPROFILE` into the sandbox too.** Since v1.0.2 an
  unconfigured `base_dir` falls back to the home directory, so a harness that
  skips this scans the developer's real home and self-heals the counter from
  their real projects. Any new harness must do the same.
- A `static SERIAL: Mutex<()>` to run tests serially within the test binary (Rust 2024 edition made `std::env::set_var` unsafe — the mutex justifies the `unsafe` block). Each test binary has its own `SERIAL`; that's fine because `FASTF_INSTALL_DIR` is per-process and `cargo test`'s binaries are separate processes.
- Process-global state needs its lock beside it: `faults::TEST_LOCK` and
  `interrupt::TEST_LOCK` exist because a private mutex per test module looks
  right and silently races.
