//! The one sandbox for in-process tests.
//!
//! `FASTF_INSTALL_DIR`, `HOME` and `FASTF_FAULT` are process-wide, so a test
//! that sets them must hold a lock while it runs, and every test in the binary
//! must use the same lock. That much was already true; what was not is that the
//! *rule* was re-typed in four files. `tests/CLAUDE.md` says every harness
//! redirects `HOME`, and one of the four could quietly stop doing it — which is
//! exactly how five register tests came to scan the developer's real home
//! directory and self-heal the counter from their real projects.
//!
//! Each test binary still owns its own `static SERIAL`: separate binaries are
//! separate processes, so one lock per binary is both necessary and sufficient.
//!
//! **Restoration is a `Drop`, not a line after `body()`.** It used to be the
//! latter, which meant a panicking test skipped it and the next test in the
//! binary inherited a deleted tempdir as its `HOME`. The failure then landed on
//! whichever test happened to run next, not on the one that caused it.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// The variable that names the home directory on this platform.
pub fn home_var() -> &'static str {
    if cfg!(windows) { "USERPROFILE" } else { "HOME" }
}

/// Holds the binary's `SERIAL` and restores every variable it changed when
/// dropped, including on unwind.
///
/// **This is the only place under `tests/` that may call `set_var` or
/// `remove_var`** — `tests/layering.rs` enforces it. A second helper that
/// mutates the environment behind a second mutex looks like isolation and
/// provides none: `setenv` is not thread-safe at the libc level, so the two
/// race each other and every `env::var` in the binary.
pub struct EnvGuard<'a> {
    _serial: MutexGuard<'a, ()>,
    previous: HashMap<String, Option<OsString>>,
}

impl<'a> EnvGuard<'a> {
    /// Take `serial`, then apply `vars`: `Some` sets, `None` removes. Both are
    /// undone on drop.
    pub fn apply(serial: &'a Mutex<()>, vars: &[(&str, Option<&Path>)]) -> Self {
        // Recovered rather than propagated: nothing here holds an invariant a
        // panic could break, and one failing test must not fail every later one.
        let serial = serial.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = Self {
            _serial: serial,
            previous: HashMap::new(),
        };
        for (name, value) in vars {
            guard.record(name);
            // SAFETY: `serial` guarantees no other test thread in this binary
            // is touching these variables.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
        guard
    }

    /// Set one more variable under the guard already held, restored with it.
    pub fn set(&mut self, name: &str, value: &Path) {
        self.record(name);
        // SAFETY: this guard holds the binary's serial lock.
        unsafe { std::env::set_var(name, value) };
    }

    /// Remove one more variable under the guard already held, restored with it.
    pub fn remove(&mut self, name: &str) {
        self.record(name);
        // SAFETY: this guard holds the binary's serial lock.
        unsafe { std::env::remove_var(name) };
    }

    /// Remember the value only the first time a name is touched, so restoring
    /// puts back what was there before the *guard*, not before the last change.
    fn record(&mut self, name: &str) {
        self.previous
            .entry(name.to_string())
            .or_insert_with(|| std::env::var_os(name));
    }
}

impl Drop for EnvGuard<'_> {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain() {
            // SAFETY: the serial lock is held until this guard finishes.
            unsafe {
                match value {
                    Some(previous) => std::env::set_var(&name, previous),
                    None => std::env::remove_var(&name),
                }
            }
        }
    }
}

/// Acquire `serial` and run `body` against a fresh data directory, with `HOME`
/// redirected into the same sandbox.
///
/// Home matters as much as the data dir: an unconfigured `base_dir` falls back
/// to the home directory, so without this a test with a default config scans
/// whatever the developer has in `~`.
pub fn with_fresh_install<R>(serial: &Mutex<()>, body: impl FnOnce(&Path) -> R) -> R {
    let tmp = tempfile::tempdir().expect("tempdir");
    let _guard = sandbox_guard(serial, tmp.path(), tmp.path());
    fs::create_dir_all(tmp.path().join("templates")).unwrap();
    body(tmp.path())
}

/// The three variables every in-process sandbox sets.
///
/// `FASTF_FAULT` is cleared rather than set: it is process-wide like the other
/// two, and a failpoint left armed by one test fires inside the next.
fn sandbox_guard<'a>(serial: &'a Mutex<()>, install: &Path, home: &Path) -> EnvGuard<'a> {
    EnvGuard::apply(
        serial,
        &[
            ("FASTF_INSTALL_DIR", Some(install)),
            (home_var(), Some(home)),
            ("FASTF_FAULT", None),
        ],
    )
}

/// A data directory and a project base as siblings, for the suites that need
/// both (`crash_recovery`, `hostile_fs`).
pub struct Sandbox {
    /// Kept alive so the directory outlives the test.
    pub tmp: tempfile::TempDir,
    pub install: std::path::PathBuf,
    pub base: std::path::PathBuf,
}

/// `with_fresh_install`'s two-directory shape.
///
/// The body gets the guard as well as the sandbox, so a suite that needs to arm
/// `FASTF_FAULT` mid-test does it through the same guard rather than reaching
/// for `set_var` itself.
pub fn with_sandbox<R>(
    serial: &Mutex<()>,
    body: impl FnOnce(&Sandbox, &mut EnvGuard<'_>) -> R,
) -> R {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = tmp.path().join("install");
    let base = tmp.path().join("base");
    fs::create_dir_all(install.join("templates")).unwrap();
    fs::create_dir_all(&base).unwrap();

    let mut guard = sandbox_guard(serial, &install, tmp.path());
    let sandbox = Sandbox { tmp, install, base };
    body(&sandbox, &mut guard)
}
