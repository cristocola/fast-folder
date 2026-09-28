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
//! A third mode makes a point slow instead of failing it: `delay-<ms>` sleeps
//! there and carries on. Two points fire once per entry for exactly this —
//! `move:each-file` for every file a move or a copy writes, and
//! `remove:each-entry` for every entry a removal takes — so a test, or a person
//! looking at a progress dialog, can watch a cloud mount's pace on a local disk:
//!
//! ```text
//! FASTF_FAULT=move:force-staged,remove:each-entry:delay-400
//! ```
//!
//! A fourth kind of mode fails a point **the way a filesystem does**, for the
//! boundaries that ask [`check_io`]: `eio`, `enotconn`, `enotempty`, `estale`,
//! `eacces`, `ebusy` or `enoent` returns that `io::Error`, and a `-<n>` suffix
//! fails only the first `n` times the point is reached, then lets it pass — a
//! mount that drops and comes back:
//!
//! ```text
//! FASTF_FAULT=remove:unlink:enotconn-3
//! ```
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
    /// concurrently running test that happens to touch the same code. A
    /// thread-local is scoped exactly to the test that armed it, needs no lock,
    /// and cannot leak into a sibling. The env var remains for subprocess tests,
    /// which are a different process and so cannot be affected by anyone else's
    /// thread.
    ///
    /// **A thread the armed one starts is armed the same way**
    /// ([`current`], [`with_arming`]): the engine's walks, copies and removals
    /// run on `util::pool`'s workers, and a failpoint armed by a test must
    /// still trip there — with one count of `-<n>` trips shared by them all.
    static THREAD_FAULT: std::cell::RefCell<Option<ThreadArming>> =
        const { std::cell::RefCell::new(None) };
}

/// One thread arming: the armed value, and the `-<n>` counts every thread
/// armed from it shares.
#[cfg(debug_assertions)]
#[derive(Debug, Clone)]
struct ThreadArming {
    value: String,
    counts: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u32>>>,
}

/// What a thread has armed, to arm a thread it starts the same way
/// ([`with_arming`]). Nothing in release builds, which have no failpoints.
#[derive(Debug, Clone, Default)]
pub struct Arming {
    #[cfg(debug_assertions)]
    thread: Option<ThreadArming>,
}

/// This thread's own arming — not the environment's, which every thread sees
/// already.
pub fn current() -> Arming {
    Arming {
        #[cfg(debug_assertions)]
        thread: THREAD_FAULT.with(|f| f.borrow().clone()),
    }
}

/// Run `body` armed as `arming` says, then as before: how a worker thread
/// takes on the arming of the thread that started it.
pub fn with_arming<R>(arming: &Arming, body: impl FnOnce() -> R) -> R {
    #[cfg(debug_assertions)]
    {
        let before = THREAD_FAULT.with(|f| f.replace(arming.thread.clone()));
        let out = body();
        THREAD_FAULT.with(|f| *f.borrow_mut() = before);
        out
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = arming;
        body()
    }
}

/// Trip point `name` if it is one of the currently armed failpoints.
///
/// Call at a boundary where a crash must be survivable, e.g.
/// `faults::check("move:before-commit-rename")?;`
#[cfg(debug_assertions)]
pub fn check(name: &str) -> Result<()> {
    check_io(name).map_err(|error| {
        if error.raw_os_error().is_some() {
            anyhow::Error::new(error).context(format!("injected fault at '{name}'"))
        } else {
            anyhow::anyhow!("{error}")
        }
    })
}

/// Trip point `name` as a filesystem call would fail: an `io::Error`, with the
/// errno an io mode names (see the module docs), so the code under test takes
/// the same path it takes on a real mount.
#[cfg(debug_assertions)]
pub fn check_io(name: &str) -> std::io::Result<()> {
    let Some((armed, scope)) = armed() else {
        return Ok(());
    };
    for (point, mode) in specs(&armed) {
        if point != name {
            continue;
        }
        if let Some(millis) = mode.strip_prefix("delay-") {
            let millis = millis.parse::<u64>().unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_millis(millis));
            continue;
        }
        if mode == "abort" {
            // No unwinding, no destructors, no cleanup: a hard process stop.
            crate::util::diag::fatal(format!("fault injection aborting at '{name}'"));
            std::process::abort();
        }
        if let Some((errno, times)) = io_mode(mode) {
            if let Some(times) = times
                && bump(&scope, &armed, name) > times
            {
                continue;
            }
            return Err(io_error(errno));
        }
        return Err(std::io::Error::other(format!("injected fault at '{name}'")));
    }
    Ok(())
}

/// Where the armed value came from: a thread's own arming, or the process's
/// environment. Each keeps its own `-<n>` counts, so a test in one thread never
/// spends another's.
#[cfg(debug_assertions)]
#[derive(Clone)]
enum Scope {
    Thread(std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u32>>>),
    Process,
}

#[cfg(debug_assertions)]
fn armed() -> Option<(String, Scope)> {
    if let Some(arming) = THREAD_FAULT.with(|f| f.borrow().clone()) {
        return Some((arming.value, Scope::Thread(arming.counts)));
    }
    std::env::var(FAULT_ENV)
        .ok()
        .map(|value| (value, Scope::Process))
}

/// An io mode's errno name and how many times it fails, if `mode` is one.
#[cfg(debug_assertions)]
fn io_mode(mode: &str) -> Option<(&str, Option<u32>)> {
    let (errno, times) = match mode.split_once('-') {
        Some((errno, n)) => (errno, Some(n.parse::<u32>().ok()?)),
        None => (mode, None),
    };
    matches!(
        errno,
        "eio" | "enotconn" | "enotempty" | "estale" | "eacces" | "ebusy" | "enoent"
    )
    .then_some((errno, times))
}

/// The error an io mode stands for, as this platform's filesystem reports it.
#[cfg(debug_assertions)]
fn io_error(errno: &str) -> std::io::Error {
    #[cfg(unix)]
    let code = match errno {
        "eio" => libc::EIO,
        "enotconn" => libc::ENOTCONN,
        "enotempty" => libc::ENOTEMPTY,
        "estale" => libc::ESTALE,
        "eacces" => libc::EACCES,
        "ebusy" => libc::EBUSY,
        _ => libc::ENOENT,
    };
    // The nearest Windows answers: ERROR_IO_DEVICE, ERROR_NETNAME_DELETED,
    // ERROR_DIR_NOT_EMPTY, ERROR_UNEXP_NET_ERR, ERROR_ACCESS_DENIED,
    // ERROR_SHARING_VIOLATION, ERROR_FILE_NOT_FOUND.
    #[cfg(not(unix))]
    let code = match errno {
        "eio" => 1117,
        "enotconn" => 64,
        "enotempty" => 145,
        "estale" => 59,
        "eacces" => 5,
        "ebusy" => 32,
        _ => 2,
    };
    std::io::Error::from_raw_os_error(code)
}

#[cfg(debug_assertions)]
static PROCESS_COUNTS: std::sync::Mutex<Option<std::collections::HashMap<String, u32>>> =
    std::sync::Mutex::new(None);

/// Count one more trip of `name` under this arming; answers the new count.
#[cfg(debug_assertions)]
fn bump(scope: &Scope, armed: &str, name: &str) -> u32 {
    let key = format!("{armed}\u{0}{name}");
    let step = |counts: &mut std::collections::HashMap<String, u32>| {
        let count = counts.entry(key.clone()).or_insert(0);
        *count += 1;
        *count
    };
    match scope {
        Scope::Thread(counts) => {
            step(&mut counts.lock().unwrap_or_else(|error| error.into_inner()))
        }
        Scope::Process => {
            let mut counts = PROCESS_COUNTS
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            step(counts.get_or_insert_with(std::collections::HashMap::new))
        }
    }
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
            Some((point, mode)) if mode.starts_with("delay-") => (point, mode),
            Some((point, mode)) if io_mode(mode).is_some() => (point, mode),
            _ => (spec, "error"),
        })
        .collect()
}

/// Whether the armed failpoints include `name`.
///
/// One failpoint — `move:force-staged` — is a *decision*, not a crash: the move
/// engine asks it before trying the rename, and an arm means "take the staged
/// path" (see the module docs). `check` cannot express that, because an
/// injected `Err` there is the signal, not a failure to propagate.
#[cfg(debug_assertions)]
pub fn is_armed(name: &str) -> bool {
    armed().is_some_and(|(armed, _)| specs(&armed).iter().any(|(point, _)| *point == name))
}

/// Arm a failpoint for the current thread only, for the duration of `body`.
///
/// Preferred over setting `FASTF_FAULT` in an in-process test: it cannot affect
/// another test running in parallel, so no lock is needed.
#[cfg(all(test, debug_assertions))]
pub fn with_thread_fault<R>(spec: &str, body: impl FnOnce() -> R) -> R {
    let arming = Arming {
        thread: Some(ThreadArming {
            value: spec.to_string(),
            counts: Default::default(),
        }),
    };
    with_arming(&arming, body)
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
pub fn check_io(_name: &str) -> std::io::Result<()> {
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
    // The source base's probe exists and is not yet removed.
    "move:after-probe",
    "move:force-staged",
    "move:mid-copy",
    "move:after-staging",
    "move:after-verify",
    "move:post-verification",
    "move:before-commit-rename",
    // The publish's `PROJECT_INFO.md` is written; the step reports an error
    // anyway (a `sync_all` that failed after the bytes landed).
    "move:after-publish-write",
    "move:after-publication",
    "move:after-commit-before-source-removal",
    "move:before-source-cleanup",
    "move:before-retire",
    // The retire — the rename that takes the source out of the library — fails.
    "move:source-cleanup",
    // Renamed out of the library; `Retired` not yet recorded.
    "move:after-retire",
    // The retired copy's removal stops after its first entry.
    "move:mid-gc",
    // After the retired copy is removed, before the transaction is.
    "move:after-source-cleanup",
    // Once per file a move or a copy writes, and once per entry a removal
    // takes: armed with `delay-<ms>`, they make a job as slow as a cloud mount.
    "move:each-file",
    "remove:each-entry",
    // A copy is a move that keeps its source, so it trips at the same places
    // minus every one about removing the source — there is nothing to remove,
    // and after publication there is nothing left to go wrong.
    "copy:before-marker-write",
    "copy:after-staging",
    "copy:after-verify",
    "template:mid-save",
    // A deleted project is renamed out of the library, not yet removed.
    "delete:after-retire",
    // A decision: every folder reads as an rclone mount (`util::fs_kind`), so
    // the settle and the rclone strategies run on a local disk.
    "fs:as-rclone",
    // Every `util::paths::presence` look, failed the way a mount fails
    // (`presence:lstat:eio`): a path that does not answer.
    "presence:lstat",
    // A decision, like `move:force-staged`: canonicalize every path the way a
    // drive Windows cannot name forces (`util::paths::canonical`).
    "paths:unnamed-volume",
    // A decision: every pool runs one worker (`util::pool::width_for`), so a
    // job paced with `delay-<ms>` takes one entry at a time.
    "pool:serial",
    // A walk's listing of one folder, and its look at one entry, failed the
    // way a mount fails (`walk:readdir:eio`).
    "walk:readdir",
    "walk:lstat",
    // A removal's unlink or rmdir, and a copy's write, failed the way a mount
    // fails (`remove:unlink:enotconn-3`, `copy:write:eio-1`).
    "remove:unlink",
    "copy:write",
    // A folder asked to go again after a pause, once its listing had not
    // caught up (`remove:again:enotempty-4`).
    "remove:again",
    // Taking back the probe a copy makes to ask whether its target ignores
    // case, failed the way a mount fails (`copy:case-probe:eacces`).
    "copy:case-probe",
    // A decision: a mount that does not answer is waited for a second, not
    // two minutes (`util::fs_retry::mount_wait`), so a move pauses in a test.
    "fs:short-mount-wait",
    // A decision: a look into a folder holding `.fastf-test-stall` does not
    // come back while the file is there (`util::paths::stall_if_marked`) —
    // a base on a mount that stopped answering.
    "paths:stall-base",
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

    /// `delay-<ms>` makes a point slow, never failing: the entry after it in
    /// the list still trips.
    #[test]
    fn a_delay_waits_and_carries_on() {
        with_fault("remove:each-entry:delay-30,move:mid-copy", || {
            let started = std::time::Instant::now();
            assert!(check("remove:each-entry").is_ok());
            assert!(started.elapsed() >= std::time::Duration::from_millis(30));
            assert!(check("move:mid-copy").is_err());
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

    /// An io mode fails the point the way the filesystem would: with that
    /// errno, so a classifier sees what a real mount would hand it.
    #[test]
    fn an_io_mode_fails_with_that_errno() {
        with_fault("remove:unlink:enotconn,walk:readdir:eio", || {
            let error = check_io("remove:unlink").unwrap_err();
            #[cfg(unix)]
            assert_eq!(error.raw_os_error(), Some(libc::ENOTCONN));
            #[cfg(not(unix))]
            assert_eq!(error.raw_os_error(), Some(64));
            #[cfg(unix)]
            assert_eq!(
                check_io("walk:readdir").unwrap_err().raw_os_error(),
                Some(libc::EIO)
            );
            assert!(check_io("copy:write").is_ok(), "an unlisted point passes");
            // The anyhow form names the point and keeps the errno underneath.
            let error = check("remove:unlink").unwrap_err();
            assert!(error.to_string().contains("remove:unlink"));
            assert!(error.downcast_ref::<std::io::Error>().is_some());
        });
    }

    /// `-<n>` fails the first n trips, then passes: a mount that drops and
    /// comes back. Each arming counts afresh.
    #[test]
    fn an_io_mode_with_a_count_fails_that_many_times() {
        with_fault("remove:unlink:eio-2", || {
            assert!(check_io("remove:unlink").is_err());
            assert!(check_io("remove:unlink").is_err());
            assert!(check_io("remove:unlink").is_ok());
            assert!(check_io("remove:unlink").is_ok());
        });
        with_fault("remove:unlink:eio-2", || {
            assert!(
                check_io("remove:unlink").is_err(),
                "a new arming counts from zero"
            );
        });
    }

    /// A count that is not a number is not an io mode: the whole entry is then
    /// a point name, which nothing trips — rather than a mode silently read as
    /// "fail for ever".
    #[test]
    fn a_malformed_count_is_not_an_io_mode() {
        with_fault("remove:unlink:eio-x", || {
            assert!(check_io("remove:unlink").is_ok());
        });
    }
}
