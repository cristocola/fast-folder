//! One lock and one RAII guard for environment mutation inside the lib's test
//! binary.
//!
//! `setenv` is not thread-safe at the libc level. Two independent mutexes used
//! to guard it here — `trace::tests::TEST_LOCK` for `FASTF_TRACE_FILE` and
//! `interrupt::TEST_LOCK`, borrowed as `SERIAL` by `project`'s tests, for
//! `FASTF_INSTALL_DIR` — which means they raced each other and every `env::var`
//! in the binary. Two locks over one process-global is one lock too many.
//!
//! The guard also **restores on unwind**. The previous pattern was `set_var`,
//! run the body, `remove_var`: a panicking test skipped the reset, and the next
//! test in the binary inherited a deleted tempdir as its data directory.
//!
//! ## Lock order
//!
//! [`ENV_LOCK`] is taken **first**. A test that also needs
//! [`crate::util::interrupt::TEST_LOCK`] (the process-global interrupt flag)
//! takes it afterwards, never the other way round. The same note is on that
//! lock.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use tempfile::TempDir;

/// The one lock over process environment mutation in this binary.
pub static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holds [`ENV_LOCK`] and restores every variable it changed when dropped —
/// including on unwind, which is the point.
pub struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    /// `None` means the variable was unset before, so restoring means removing.
    previous: HashMap<String, Option<OsString>>,
}

impl EnvGuard {
    /// Take the lock and set `vars`, remembering what was there before.
    pub fn set(vars: &[(&str, &Path)]) -> Self {
        let lock = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut previous = HashMap::new();
        for (name, value) in vars {
            previous.insert((*name).to_string(), std::env::var_os(name));
            // SAFETY: `ENV_LOCK` is held, and it is the only lock in this
            // binary under which the environment is mutated.
            unsafe { std::env::set_var(name, value) };
        }
        Self {
            _lock: lock,
            previous,
        }
    }

    /// A guard with the data directory and the home directory pointed at a
    /// fresh tempdir.
    ///
    /// `HOME` is not optional. An unconfigured `base_dir` falls back to the home
    /// directory, so a test that redirects only `FASTF_INSTALL_DIR` scans the
    /// developer's real home and self-heals the counter from their real
    /// projects. `USERPROFILE` is the same variable on Windows.
    ///
    /// Returns the `TempDir` too: dropping it removes the sandbox, so the caller
    /// must keep it alive for as long as the guard.
    pub fn sandbox() -> (Self, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        let guard = Self::set(&[
            ("FASTF_INSTALL_DIR", &path),
            ("HOME", &path),
            ("USERPROFILE", &path),
        ]);
        (guard, dir)
    }

    /// Set one more variable under the guard already held, restored with it.
    pub fn also_set(&mut self, name: &str, value: &str) {
        self.previous
            .entry(name.to_string())
            .or_insert_with(|| std::env::var_os(name));
        // SAFETY: this guard holds `ENV_LOCK`.
        unsafe { std::env::set_var(name, value) };
    }

    /// Remove a variable under the guard already held, restored with it.
    pub fn also_remove(&mut self, name: &str) {
        self.previous
            .entry(name.to_string())
            .or_insert_with(|| std::env::var_os(name));
        // SAFETY: this guard holds `ENV_LOCK`.
        unsafe { std::env::remove_var(name) };
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain() {
            // SAFETY: the lock is held until this guard finishes dropping.
            unsafe {
                match value {
                    Some(previous) => std::env::set_var(&name, previous),
                    None => std::env::remove_var(&name),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard's whole reason for existing: a unit test that reaches
    /// `DataLock` must lock the sandbox, not the developer's real data
    /// directory. Before this, `cargo test` could block a `fastf` running in
    /// another terminal for the full 30-second timeout — and leave a
    /// `.fastf.lock` behind in their config directory.
    #[test]
    fn a_sandbox_guard_moves_the_data_lock_into_its_tempdir() {
        let (_guard, dir) = EnvGuard::sandbox();
        let lock = crate::util::lockfile::lock_path();
        assert!(
            lock.starts_with(dir.path()),
            "the lock is at {} rather than under {}",
            lock.display(),
            dir.path().display()
        );
    }

    /// The guard restores on **unwind**, not merely on a clean return. The
    /// pattern it replaces was `set_var`, run the body, `remove_var`: a
    /// panicking test skipped the reset, and the next test in the binary
    /// inherited a deleted tempdir as its data directory.
    ///
    /// Guards do not nest: `ENV_LOCK` is a plain `Mutex`, and taking it twice on
    /// one thread deadlocks. One guard per test, always.
    #[test]
    fn the_previous_value_comes_back_including_on_unwind() {
        const NAME: &str = "FASTF_TEST_ENV_GUARD_FIXTURE";

        let panicked = std::panic::catch_unwind(|| {
            let mut guard = EnvGuard::set(&[]);
            guard.also_set(NAME, "shadowed");
            assert_eq!(std::env::var(NAME).unwrap(), "shadowed");
            panic!("the body of a test that fails");
        });
        assert!(panicked.is_err(), "the fixture must actually panic");

        // Unwinding dropped the guard, which released the lock (poisoned, which
        // `EnvGuard::set` tolerates) and put the environment back.
        let _guard = EnvGuard::set(&[]);
        assert!(
            std::env::var_os(NAME).is_none(),
            "a variable unset before the guard must be unset after it"
        );
    }
}
