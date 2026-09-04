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
//!   cleaned up. This models hard process termination such as `taskkill /F` and
//!   proves that *recovery* works rather than only testing unwind cleanup.
//!
//! Several failpoints can be armed at once as a **comma list**, which trips
//! every named point on the way:
//!
//! ```text
//! FASTF_FAULT=move:force-staged,move:after-staging
//! ```
//!
//! This is how a pty test walks a whole failure shape in one run — the first
//! point switches the engine onto the staged path, the second fails it there.
//! `move:force-staged` is the one failpoint whose injected error is *handled*:
//! the move engine reads it as the signal to take the staged-copy path, exactly
//! as a cross-device `EXDEV` would, because a same-volume rename would never
//! reach the code the test is about.
//!
//! Compiled out entirely in release builds: `check` becomes an inlined `Ok(())`
//! and the environment is never consulted, so a stray `FASTF_FAULT` in a user's
//! shell cannot affect a shipped binary.

use anyhow::Result;

/// Environment variable naming the failpoint to trip.
pub const FAULT_ENV: &str = "FASTF_FAULT";

#[cfg(debug_assertions)]
thread_local! {
    /// Per-thread arming, used by in-process tests.
    ///
    /// The environment variable is process-global, and `cargo test` runs tests
    /// in parallel threads — so an env-armed failpoint fires inside *every*
    /// concurrently running test that happens to touch the same code. That cost
    /// three separate flaky failures before this existed. A thread-local is
    /// scoped exactly to the test that armed it, needs no lock, and cannot leak
    /// into a sibling. The env var remains for subprocess tests, which are a
    /// different process and so cannot be affected by anyone else's thread.
    static THREAD_FAULT: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Trip point `name` if it is one of the currently armed failpoints.
///
/// Call at a boundary where a crash must be survivable, e.g.
/// `faults::check("move:before-commit-rename")?;`
#[cfg(debug_assertions)]
pub fn check(name: &str) -> Result<()> {
    // Thread-local first: an in-process test's arming must not be visible to
    // tests running in parallel.
    let armed = THREAD_FAULT.with(|f| f.borrow().clone());
    let armed = match armed {
        Some(value) => value,
        None => match std::env::var(FAULT_ENV) {
            Ok(value) => value,
            Err(_) => return Ok(()),
        },
    };
    trip(name, &armed)
}

/// The armed value split into `(point, mode)` pairs — a comma list, each entry
/// `point[:mode]`. The mode suffix is split from the **right** of each entry,
/// since point names contain colons.
#[cfg(debug_assertions)]
fn specs(armed: &str) -> Vec<(&str, &str)> {
    armed
        .split(',')
        .map(|spec| match spec.rsplit_once(':') {
            Some((point, mode @ ("abort" | "error"))) => (point, mode),
            _ => (spec, "error"),
        })
        .collect()
}

#[cfg(debug_assertions)]
fn trip(name: &str, armed: &str) -> Result<()> {
    for (point, mode) in specs(armed) {
        if point == name {
            if mode == "abort" {
                // No unwinding, no destructors, no cleanup: a hard process stop.
                crate::util::diag::fatal(format!("fault injection aborting at '{name}'"));
                std::process::abort();
            }
            anyhow::bail!("injected fault at '{name}'");
        }
    }
    Ok(())
}

/// Whether the armed failpoints include `name`.
///
/// One failpoint — `move:force-staged` — is a *decision*, not a crash: the move
/// engine asks it before trying the rename, and an arm means "take the staged
/// path" (see the module docs). `check` cannot express that, because an
/// injected `Err` there is the signal, not a failure to propagate.
#[cfg(debug_assertions)]
pub fn is_armed(name: &str) -> bool {
    let armed = THREAD_FAULT.with(|f| f.borrow().clone());
    let armed = match armed {
        Some(value) => value,
        None => match std::env::var(FAULT_ENV) {
            Ok(value) => value,
            Err(_) => return false,
        },
    };
    specs(&armed).iter().any(|(point, _)| *point == name)
}

/// Arm a failpoint for the current thread only, for the duration of `body`.
///
/// Preferred over setting `FASTF_FAULT` in an in-process test: it cannot affect
/// another test running in parallel, so no lock is needed.
#[cfg(all(test, debug_assertions))]
pub fn with_thread_fault<R>(spec: &str, body: impl FnOnce() -> R) -> R {
    THREAD_FAULT.with(|f| *f.borrow_mut() = Some(spec.to_string()));
    let out = body();
    THREAD_FAULT.with(|f| *f.borrow_mut() = None);
    out
}

/// Release builds have no failpoints at all.
#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn check(_name: &str) -> Result<()> {
    Ok(())
}

/// Release builds have no failpoints at all.
#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn is_armed(_name: &str) -> bool {
    false
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
    "move:before-marker-write",
    "move:after-transaction-create",
    "move:force-staged",
    "move:mid-copy",
    "move:after-staging",
    "move:after-verify",
    "move:post-verification",
    "move:before-commit-rename",
    "move:after-publication",
    "move:after-commit-before-source-removal",
    "move:before-source-cleanup",
    "move:source-cleanup",
    "move:after-source-cleanup",
    "template:mid-save",
];

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    /// Thread-local arming: no shared state, so no lock and no interference.
    fn with_fault<R>(value: &str, body: impl FnOnce() -> R) -> R {
        with_thread_fault(value, body)
    }

    #[test]
    fn unarmed_points_pass_through() {
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

    /// A comma list arms every named point: the pty suites trip a whole
    /// failure shape in one run (`move:force-staged,move:after-staging`).
    #[test]
    fn comma_list_trips_every_named_point() {
        with_fault("move:force-staged,move:after-staging", || {
            assert!(check("move:force-staged").is_err());
            assert!(check("move:after-staging").is_err());
            assert!(check("move:mid-copy").is_ok(), "an unlisted point passes");
        });
    }

    /// Each comma entry keeps its own mode suffix.
    #[test]
    fn comma_list_entries_keep_their_modes() {
        with_fault("move:mid-copy:abort,move:after-verify", || {
            assert!(check("move:after-verify").is_err());
        });
        // The abort entry is exercised by the existing process tests; here it
        // is enough that it parses without swallowing the second entry.
        with_fault("create:mid-copy:error,move:after-staging", || {
            assert!(check("move:after-staging").is_err());
        });
    }

    /// `is_armed` answers the decision failpoints that `check` cannot: an arm
    /// means "take the staged path", not "fail here" — so the engine asks it
    /// and never trips `check` on the same name.
    #[test]
    fn is_armed_sees_the_list_without_tripping() {
        with_fault("move:force-staged,move:after-staging", || {
            assert!(is_armed("move:force-staged"));
            assert!(is_armed("move:after-staging"), "a check point is armed too");
            assert!(!is_armed("move:mid-copy"));
            // The decision arm does not disturb the ordinary points.
            assert!(check("move:after-staging").is_err());
            assert!(check("move:mid-copy").is_ok());
        });
        assert!(!is_armed("move:force-staged"), "unarmed point is not armed");
    }
}
