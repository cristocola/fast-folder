//! Deterministic fault injection for the crash-unsafe boundaries.
//!
//! Testing "what if we die mid-copy?" by racing a real `kill` against a real
//! copy is slow, flaky, and only ever reaches the boundaries you happen to hit.
//! Instead, the code carries **named failpoints** at each boundary that must be
//! crash-safe, and a test names the one it wants to trip:
//!
//! ```text
//! FASTF_FAULT=move:before-commit-rename        # returns an error there
//! FASTF_FAULT=create:mid-copy:abort            # kills the process there
//! ```
//!
//! Two modes, because they prove different things:
//!
//! - `error` (default) — the failpoint returns `Err`, so the ordinary unwind and
//!   rollback run. This is what an interrupt or a full disk looks like.
//! - `abort` — `std::process::abort()`, no unwinding, no destructors, nothing
//!   cleaned up. This is what a power cut or `taskkill /F` looks like, and it is
//!   the only honest way to test that *recovery* works rather than that cleanup
//!   works.
//!
//! Compiled out entirely in release builds: `check` becomes an inlined `Ok(())`
//! and the environment is never consulted, so a stray `FASTF_FAULT` in a user's
//! shell cannot affect a shipped binary.

use anyhow::Result;

/// Environment variable naming the failpoint to trip.
pub const FAULT_ENV: &str = "FASTF_FAULT";

/// Trip point `name` if it is the one currently armed.
///
/// Call at a boundary where a crash must be survivable, e.g.
/// `faults::check("move:before-commit-rename")?;`
#[cfg(debug_assertions)]
pub fn check(name: &str) -> Result<()> {
    let Ok(armed) = std::env::var(FAULT_ENV) else {
        return Ok(());
    };
    // `point[:mode]`
    let (point, mode) = match armed.rsplit_once(':') {
        Some((point, mode @ ("abort" | "error"))) => (point, mode),
        _ => (armed.as_str(), "error"),
    };
    if point != name {
        return Ok(());
    }
    if mode == "abort" {
        // No unwinding, no destructors, no cleanup — exactly like losing power.
        eprintln!("fastf: fault injection aborting at '{name}'");
        std::process::abort();
    }
    anyhow::bail!("injected fault at '{name}'")
}

/// Release builds have no failpoints at all.
#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn check(_name: &str) -> Result<()> {
    Ok(())
}

/// Every failpoint the codebase defines.
///
/// Kept as a list so the invariant test can iterate them: a new boundary added
/// without a matching test entry is a gap, and this makes the gap visible. The
/// test asserts this list and the strings actually passed to [`check`] agree.
pub const ALL_FAULT_POINTS: &[&str] = &[
    "create:after-root-dir",
    "create:after-pinfo",
    "create:mid-copy",
    "create:before-counter-save",
    "move:after-staging",
    "move:after-verify",
    "move:before-commit-rename",
    "move:after-commit-before-source-removal",
];

/// Serializes every test that arms a failpoint.
///
/// `FASTF_FAULT` is a process-wide environment variable, so a test that arms one
/// is visible to every other test in the same binary. This lives here, beside the
/// state it guards, so anything reaching for fault injection finds the lock too —
/// the alternative (a private mutex per test module) looks right and silently
/// races, which is exactly how the move-failpoint test started failing whenever
/// it ran alongside this module's own tests.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    fn with_fault<R>(value: &str, body: impl FnOnce() -> R) -> R {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the lock keeps other tests in this binary off the variable.
        unsafe { std::env::set_var(FAULT_ENV, value) };
        let out = body();
        unsafe { std::env::remove_var(FAULT_ENV) };
        out
    }

    #[test]
    fn unarmed_points_pass_through() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::remove_var(FAULT_ENV) };
        assert!(check("create:mid-copy").is_ok());
    }

    #[test]
    fn armed_point_fails_and_others_do_not() {
        with_fault("create:mid-copy", || {
            let err = check("create:mid-copy").unwrap_err();
            assert!(err.to_string().contains("create:mid-copy"));
            // A different point must be unaffected.
            assert!(check("create:after-pinfo").is_ok());
        });
    }

    #[test]
    fn explicit_error_mode_is_accepted() {
        with_fault("move:after-verify:error", || {
            assert!(check("move:after-verify").is_err());
            assert!(check("move:after-staging").is_ok());
        });
    }

    /// Point names contain colons, so the mode suffix must be split off the
    /// *right*, or `move:after-verify` would parse as point `move`.
    #[test]
    fn mode_is_split_from_the_right() {
        with_fault("move:after-verify", || {
            assert!(check("move").is_ok(), "must not match the bare prefix");
            assert!(check("move:after-verify").is_err());
        });
    }
}
