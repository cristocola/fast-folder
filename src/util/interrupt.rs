//! Cooperative interrupt handling for Ctrl-C.
//!
//! Without this, Ctrl-C during a `fastf new` terminated the process wherever it
//! happened to be — typically part-way through copying a template's assets,
//! leaving a half-built project behind. Handlers here do the only thing that is
//! safe from a signal context: set a flag. The create path polls it between
//! files and unwinds normally, which lets the ordinary rollback remove the
//! partial folder and leave the ID counter untouched.
//!
//! Nothing here aborts a copy mid-file; the granularity is one file. That is a
//! deliberate trade — a torn file is exactly what we are avoiding, and
//! `assets::copy_file` writes through an operation-owned unique temp anyway.
//!
//! A second Ctrl-C restores the default behaviour, so a genuinely stuck process
//! can always still be killed from the keyboard.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// What the second signal undoes before it exits: the guided app's alternate
/// screen and raw mode, or an inline prompt's rows. A `fn()` stored as its
/// address, so the handler can load it without a lock.
static RESTORE: AtomicUsize = AtomicUsize::new(0);

/// True once the user has asked us to stop.
pub fn is_set() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

/// Error returned by long-running work that noticed the interrupt.
pub(crate) fn interrupted_error() -> anyhow::Error {
    anyhow::anyhow!("interrupted")
}

/// `Err` if the user has asked us to stop — call between units of work.
pub fn check() -> anyhow::Result<()> {
    if is_set() {
        return Err(interrupted_error());
    }
    Ok(())
}

/// Serializes every test that touches the interrupt flag.
///
/// The flag is process-global by nature, so a test that raises it would
/// otherwise be visible to any test running in parallel — which is exactly how
/// this module's own test started failing. It lives here, next to the state it
/// guards, so anything reaching for `raise_for_test` finds the lock too.
///
/// **Lock order:** a test that also needs
/// [`crate::util::test_env::ENV_LOCK`] takes that one **first**. The same note
/// is on that lock.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Raise the flag without a real signal.
///
/// The guided app runs the terminal in raw mode, where Ctrl-C arrives as a key
/// rather than as SIGINT; when the user presses it at the root the app raises
/// the flag itself so that `main`'s exit path says `aborted.` and exits 130
/// exactly as a signal would have.
pub fn raise() {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

/// [`raise`] for tests, which hold [`TEST_LOCK`] across the raise-and-reset.
/// Sending an actual console control event on Windows reaches the whole process
/// group — including the test runner — so simulating the flag is both safer and
/// more precise: it drives exactly the code a real Ctrl-C reaches.
#[cfg(test)]
pub fn raise_for_test() {
    raise();
}

/// Reset the flag. Only for tests, which run many creates in one process.
#[cfg(test)]
pub fn reset() {
    INTERRUPTED.store(false, Ordering::Relaxed);
}

/// Show the terminal cursor again, on whichever standard stream is a terminal.
///
/// Every prompt hides the cursor and shows it again on the way out
/// — but not when it returns an error, and not when a menu unwinds past it.
/// Left alone, leaving fastf that way hands the shell back an invisible cursor
/// until the user thinks to run `tput cnorm`.
///
/// Guarded per stream: prompts draw on stderr, output lands on stdout, and
/// either can be the terminal. Without the guard the escape lands in whatever
/// a script is reading, which is a worse bug than the one being fixed.
///
/// On unix this is reached from inside the SIGINT handler, so it is written to
/// be async-signal-safe: `isatty` and `write` are, while `Term::show_cursor`
/// takes std's stream lock and can panic re-entering a `RefCell` the
/// interrupted thread already holds. The bytes are exactly what `console`
/// writes on unix, so no output changes. Elsewhere the handler runs on its own
/// thread — Windows spawns one for a console control event — so the ordinary
/// path is safe there, and the console API is what a legacy conhost needs.
pub fn restore_terminal() {
    #[cfg(unix)]
    {
        const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
        for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            // SAFETY: both calls are async-signal-safe and touch only a
            // descriptor the process already owns.
            unsafe {
                if libc::isatty(fd) == 1 {
                    // Nothing to do about a failed write on the way out.
                    let _ = libc::write(fd, SHOW_CURSOR.as_ptr().cast(), SHOW_CURSOR.len());
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Windows has no async-signal-safety rule to keep, so the escape can
        // simply be written through the ordinary handles.
        use std::io::{IsTerminal, Write};
        const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
        if std::io::stdout().is_terminal() {
            let _ = std::io::stdout().write_all(SHOW_CURSOR);
            let _ = std::io::stdout().flush();
        }
        if std::io::stderr().is_terminal() {
            let _ = std::io::stderr().write_all(SHOW_CURSOR);
            let _ = std::io::stderr().flush();
        }
    }
}

/// Register what the second signal must undo before the process exits. The
/// guided app registers its screen (raw mode, the alternate screen, the mouse
/// and paste reports); an inline prompt registers its rows. Whatever it is,
/// it runs from a signal handler and must be async-signal-safe: `write`,
/// `tcsetattr`, an atomic — never a lock or an allocation.
pub fn set_restore(restore: fn()) {
    RESTORE.store(restore as usize, Ordering::SeqCst);
}

/// The surface has been given back the ordinary way; the second signal has
/// nothing of it to undo.
pub fn clear_restore() {
    RESTORE.store(0, Ordering::SeqCst);
}

/// What runs on the second signal before `exit`: the registered surface
/// restore, then the cursor. Separate from `flag` so it can be tested — the
/// exit cannot be.
pub(crate) fn on_second_signal() {
    let restore = RESTORE.load(Ordering::SeqCst);
    if restore != 0 {
        // SAFETY: the only writer is `set_restore`, which stores the address
        // of a `fn()`; a non-zero value is one of those.
        let restore: fn() = unsafe { std::mem::transmute::<usize, fn()>(restore) };
        restore();
    }
    restore_terminal();
}

/// Record the interrupt. Async-signal-safe: a single relaxed atomic store, and
/// on the second signal the surface and cursor restore, which are written for
/// this context.
fn flag() {
    // On the second interrupt, stop being polite. If the first one did not get
    // us out promptly the user should not have to reach for Task Manager.
    if INTERRUPTED.swap(true, Ordering::Relaxed) {
        // `main`'s error path never runs from here, so the screen has to be
        // given back before leaving or the shell is left in raw mode, blind.
        on_second_signal();
        std::process::exit(130); // 128 + SIGINT, the conventional shell code
    }
}

/// Install the handler. Idempotent, and never fatal: if the OS refuses, fastf
/// simply keeps the default terminate-on-Ctrl-C behaviour it had before.
pub fn install() {
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    install_platform();
}

#[cfg(windows)]
fn install_platform() {
    // Also covers CTRL_CLOSE_EVENT (window closed) and logoff/shutdown, where
    // Windows gives a short grace period before killing the process — enough to
    // notice the flag and unwind.
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
        const CTRL_C_EVENT: u32 = 0;
        const CTRL_BREAK_EVENT: u32 = 1;
        const CTRL_CLOSE_EVENT: u32 = 2;
        const CTRL_LOGOFF_EVENT: u32 = 5;
        const CTRL_SHUTDOWN_EVENT: u32 = 6;
        match ctrl_type {
            CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT => {
                flag();
                1 // handled — do not run the default terminator
            }
            _ => 0,
        }
    }

    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(unix)]
fn install_platform() {
    extern "C" fn handler(_signum: i32) {
        flag();
    }
    // Go through a plain function pointer before the integer cast. Casting the
    // function *item* straight to an integer is accepted by the compiler but
    // relies on its size matching, which clippy rightly refuses.
    let handler_ptr = handler as extern "C" fn(i32) as libc::sighandler_t;
    // SAFETY: `handler` only performs an atomic store (and `exit` on the second
    // signal), both of which are safe from a signal context.
    //
    // SIGHUP too: a closed terminal window or a dropped ssh session must
    // unwind a create the same cooperative way, so the partial folder is
    // rolled back rather than left behind.
    unsafe {
        libc::signal(libc::SIGINT, handler_ptr);
        libc::signal(libc::SIGTERM, handler_ptr);
        libc::signal(libc::SIGHUP, handler_ptr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent_and_starts_clear() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        install();
        install();
        assert!(!is_set(), "no interrupt has been raised");
        assert!(check().is_ok());
    }

    static RESTORED: AtomicBool = AtomicBool::new(false);

    #[test]
    fn the_second_signal_runs_the_registered_restore_first() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fn restore() {
            RESTORED.store(true, Ordering::SeqCst);
        }
        set_restore(restore);
        on_second_signal();
        assert!(RESTORED.load(Ordering::SeqCst), "the surface is given back");
        clear_restore();
        RESTORED.store(false, Ordering::SeqCst);
        on_second_signal();
        assert!(!RESTORED.load(Ordering::SeqCst), "cleared: nothing to undo");
    }

    #[test]
    fn raised_flag_makes_check_fail_until_reset() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        assert!(check().is_ok());
        raise_for_test();
        assert!(is_set());
        assert!(check().is_err(), "long-running work must see the interrupt");
        reset();
        assert!(check().is_ok(), "reset clears it for the next operation");
    }
}
