//! The one sandbox for in-process tests.
//!
//! `FASTF_INSTALL_DIR` and `HOME` are process-wide, so a test that sets them
//! must hold a lock while it runs, and every test in the binary must use the
//! same lock. That much was already true; what was not is that the *rule* was
//! re-typed in four files. `tests/CLAUDE.md` says every harness redirects
//! `HOME`, and one of the four could quietly stop doing it — which is exactly
//! how five register tests came to scan the developer's real home directory and
//! self-heal the counter from their real projects.
//!
//! Each test binary still owns its own `static SERIAL`: separate binaries are
//! separate processes, so one lock per binary is both necessary and sufficient.

use std::fs;
use std::path::Path;
use std::sync::Mutex;

/// Acquire `serial` and run `body` against a fresh data directory, with `HOME`
/// redirected into the same sandbox.
///
/// Home matters as much as the data dir: an unconfigured `base_dir` falls back
/// to the home directory (v1.0.2), so without this a test with a default config
/// scans whatever the developer has in `~`.
pub fn with_fresh_install<R>(serial: &Mutex<()>, body: impl FnOnce(&Path) -> R) -> R {
    // Recovered rather than propagated: nothing here holds an invariant a panic
    // could break, and one failing test must not fail every later one.
    let guard = serial.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let old_home = std::env::var_os(home_var);
    // SAFETY: `serial` guarantees no other test thread in this binary is
    // touching these variables.
    unsafe {
        std::env::set_var("FASTF_INSTALL_DIR", tmp.path());
        std::env::set_var(home_var, tmp.path());
    }
    fs::create_dir_all(tmp.path().join("templates")).unwrap();

    let result = body(tmp.path());

    unsafe {
        std::env::remove_var("FASTF_INSTALL_DIR");
        match old_home {
            Some(value) => std::env::set_var(home_var, value),
            None => std::env::remove_var(home_var),
        }
    }
    drop(guard);
    result
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
/// Also clears `FASTF_FAULT` on the way out: it is process-wide like the other
/// two, and a failpoint left armed by one test fires inside the next.
pub fn with_sandbox<R>(serial: &Mutex<()>, body: impl FnOnce(&Sandbox) -> R) -> R {
    let guard = serial.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = tmp.path().join("install");
    let base = tmp.path().join("base");
    fs::create_dir_all(install.join("templates")).unwrap();
    fs::create_dir_all(&base).unwrap();

    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let old_home = std::env::var_os(home_var);
    // SAFETY: `serial` keeps other tests in this binary off these variables.
    unsafe {
        std::env::set_var("FASTF_INSTALL_DIR", &install);
        std::env::set_var(home_var, tmp.path());
    }

    let sandbox = Sandbox { tmp, install, base };
    let result = body(&sandbox);

    unsafe {
        std::env::remove_var("FASTF_INSTALL_DIR");
        std::env::remove_var("FASTF_FAULT");
        match old_home {
            Some(value) => std::env::set_var(home_var, value),
            None => std::env::remove_var(home_var),
        }
    }
    drop(guard);
    result
}
