//! Counting the work fastf does, so a claim about it can be tested.
//!
//! "The browser no longer rescans the library after a tag" is not observable
//! from output: the row looks the same either way, and the difference is seconds
//! on a network share and nothing at all on a local SSD. So the expensive
//! operations name themselves, and a test counts the names.
//!
//! ```text
//! FASTF_TRACE_FILE=/tmp/counts fastf   # one line per traced operation
//! ```
//!
//! Compiled out entirely in release builds, like [`crate::util::faults`]: `hit`
//! becomes an inlined no-op and the environment is never consulted, so a stray
//! `FASTF_TRACE_FILE` in a user's shell cannot make a shipped binary write to a
//! file.
//!
//! Writes are append-only, one `open`/`write`/`close` per hit, and every failure
//! is dropped. A trace that cannot be written must never change what the program
//! does — the whole point is to measure the program, not a different one.

/// Environment variable naming the file to append to.
pub const TRACE_ENV: &str = "FASTF_TRACE_FILE";

/// Record one occurrence of a named operation.
///
/// Names are free-form and grouped by prefix, e.g. `discover`, `scan_base`,
/// `template_load`, `read_metadata`.
#[cfg(debug_assertions)]
pub fn hit(name: &str) {
    use std::io::Write;

    let Ok(path) = std::env::var(TRACE_ENV) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{name}");
    }
}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn hit(_name: &str) {}

/// How many times `name` appears in a trace file. Test-facing.
pub fn count_in(contents: &str, name: &str) -> usize {
    contents.lines().filter(|line| *line == name).count()
}

#[cfg(test)]
mod tests {
    #[cfg(debug_assertions)]
    use super::{count_in, hit};
    use std::sync::Mutex;

    /// `TRACE_ENV` is process-global, like every other environment variable a
    /// test touches. See `tests/CLAUDE.md`: the lock lives beside the state it
    /// guards, not in whichever module happens to need it.
    #[allow(dead_code)]
    pub static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Debug only: `hit` is compiled to nothing in release, so in a release test
    /// build there is nothing to count. That is the guarantee, not a gap.
    #[cfg(debug_assertions)]
    #[test]
    fn hits_are_appended_one_per_line_and_counted_by_name() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace");

        unsafe { std::env::set_var(super::TRACE_ENV, &path) };
        hit("discover");
        hit("read_metadata");
        hit("discover");
        unsafe { std::env::remove_var(super::TRACE_ENV) };

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(count_in(&text, "discover"), 2);
        assert_eq!(count_in(&text, "read_metadata"), 1);
        assert_eq!(count_in(&text, "scan_base"), 0);
    }

    /// Unset, or set to nothing, and the operation is free and silent.
    #[cfg(debug_assertions)]
    #[test]
    fn tracing_is_off_unless_the_file_is_named() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::remove_var(super::TRACE_ENV) };
        hit("discover");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace");
        unsafe { std::env::set_var(super::TRACE_ENV, "") };
        hit("discover");
        unsafe { std::env::remove_var(super::TRACE_ENV) };
        assert!(!path.exists());
    }
}
