//! Cooperative interrupt handling for Ctrl-C.
//!
//! Left to the default, Ctrl-C during a `fastf new` terminates the process
//! wherever it is — typically part-way through copying a template's assets,
//! leaving a half-built project behind. Handlers here do the only thing that is
//! safe from a signal context: set a flag. The create path polls it between
//! files and unwinds normally, which lets the ordinary rollback remove the
//! partial folder and leave the ID counter untouched.
//!
//! Nothing here aborts a copy mid-file; the granularity is one file. That is a
//! deliberate trade — a torn file is exactly what we are avoiding, and
//! `assets::copy_file` writes through an operation-owned unique temp anyway.
//!
//! A second Ctrl-C ends the process from the handler itself, so a genuinely
//! stuck process can always still be killed from the keyboard. It exits the
//! way a shell reports the signal that ended it — 128 plus its number: 130
//! for SIGINT, 129 for SIGHUP, 143 for SIGTERM — after giving the terminal
//! back, and through `_exit`, the one exit a signal handler may call.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
/// The stop came from the terminal going away — a hang-up, a terminate, a
/// console window closed — rather than from somebody pressing Ctrl-C. A command
/// following a job lets the job go on for the first and cancels it only for
/// the second.
static HUNG_UP: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// How many programs fastf started are holding the terminal right now — the
/// agent `X` runs, the shell `$` opens. While one is, a Ctrl-C typed is that
/// program's: the terminal sends it to the whole foreground group, fastf
/// included, and a flag raised then quits the app the moment the program
/// gives the terminal back, or exits fastf under it on a second press.
static HANDED_OVER: AtomicUsize = AtomicUsize::new(0);
/// What the second signal undoes before it exits: the guided app's alternate
/// screen and raw mode, or an inline prompt's rows. A `fn()` stored as its
/// address, so the handler can load it without a lock.
static RESTORE: AtomicUsize = AtomicUsize::new(0);

/// True once the user has asked us to stop.
pub fn is_set() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

/// True when the stop came from the terminal going away (SIGHUP, SIGTERM, a
/// closed console window, a logoff), not from Ctrl-C.
pub fn hung_up() -> bool {
    HUNG_UP.load(Ordering::Relaxed)
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
/// otherwise be visible to any test running in parallel. It lives here, next
/// to the state it guards, so anything reaching for `raise_for_test` finds the
/// lock too.
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
    HUNG_UP.store(false, Ordering::Relaxed);
}

/// The terminal is a program's that fastf started and is waiting for; a
/// Ctrl-C typed until this is dropped is that program's alone. A hang-up or
/// a terminate still stops fastf, since the window it would come back to is
/// gone. Not `SIG_IGN` or `SetConsoleCtrlHandler(NULL, TRUE)`: an ignored
/// Ctrl-C is inherited by the child, and then it is nobody's.
#[must_use = "the terminal is handed back when this is dropped"]
pub struct HandedOver(());

/// Hand the terminal to a program for as long as the guard lives.
pub fn hand_over() -> HandedOver {
    HANDED_OVER.fetch_add(1, Ordering::SeqCst);
    HandedOver(())
}

impl Drop for HandedOver {
    fn drop(&mut self) {
        HANDED_OVER.fetch_sub(1, Ordering::SeqCst);
    }
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
/// On unix this is reached from inside the signal handler, so it is written
/// to be async-signal-safe: `tcgetattr` and `write` are on POSIX's list
/// (`isatty` is not), while writing through `std::io::stdout()` takes std's
/// stream lock and can panic re-entering a `RefCell` the interrupted thread
/// already holds. Elsewhere the handler runs on its own thread — Windows
/// spawns one for a console control event — so the ordinary path is safe
/// there.
pub fn restore_terminal() {
    #[cfg(unix)]
    {
        const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
        for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            let mut settings = std::mem::MaybeUninit::<libc::termios>::uninit();
            // SAFETY: `tcgetattr` only fills the struct it is handed, and
            // fails on a descriptor that is no terminal — the question asked.
            let is_a_terminal = unsafe { libc::tcgetattr(fd, settings.as_mut_ptr()) } == 0;
            if is_a_terminal {
                // SAFETY: one `write` of a constant to a descriptor the
                // process owns. Nothing to do about a failure on the way out.
                let _ = unsafe { libc::write(fd, SHOW_CURSOR.as_ptr().cast(), SHOW_CURSOR.len()) };
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        show_console_cursor(std::io::stdout().as_raw_handle());
        show_console_cursor(std::io::stderr().as_raw_handle());
    }
}

/// Show the cursor of the console `handle` is, and answer whether it is one.
/// **The console's own call, not an escape sequence**: a console without
/// virtual terminal processing — the legacy one, and any a program has not
/// switched — prints `ESC [ ? 25 h` as text. A handle that is a pipe or a
/// file answers no to the first call and is never written to.
#[cfg(windows)]
fn show_console_cursor(handle: std::os::windows::io::RawHandle) -> bool {
    #[repr(C)]
    struct ConsoleCursorInfo {
        size: u32,
        visible: i32,
    }
    unsafe extern "system" {
        fn GetConsoleCursorInfo(
            console: std::os::windows::io::RawHandle,
            info: *mut ConsoleCursorInfo,
        ) -> i32;
        fn SetConsoleCursorInfo(
            console: std::os::windows::io::RawHandle,
            info: *const ConsoleCursorInfo,
        ) -> i32;
    }
    let mut info = ConsoleCursorInfo {
        size: 25,
        visible: 1,
    };
    // SAFETY: a handle the process owns and a struct of the size the call
    // documents; a handle that is no console makes it fail.
    if unsafe { GetConsoleCursorInfo(handle, &mut info) } == 0 {
        return false;
    }
    info.visible = 1;
    // SAFETY: the same handle, and the struct the call above filled.
    unsafe { SetConsoleCursorInfo(handle, &info) != 0 }
}

/// Register what the second signal must undo before the process exits. The
/// guided app registers its screen (raw mode, the alternate screen, the paste
/// reports); an inline prompt registers its rows. Whatever it is,
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

/// What runs on the second signal before the process leaves: the registered
/// surface restore, then the cursor. Separate from `flag` so it can be
/// tested in process — the exit is tested through a terminal
/// (`tests/tui_pty`).
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

/// The code a second Ctrl-C exits with: 128 + SIGINT, as a shell reports it.
const CTRL_C_EXIT: i32 = 130;

/// The code a console closed after a Ctrl-C exits with on Windows: a
/// hang-up's, as SIGHUP's would be on unix. A console closed first exits
/// with whatever `main` exits with, inside [`CLOSE_GRACE`].
#[cfg(any(windows, test))]
const HANG_UP_EXIT: i32 = 129;

/// How long the handler of a closed console, a logoff or a shutdown keeps
/// Windows from ending the process: Windows ends it as soon as the handler
/// returns, and at the latest five seconds after the event. Under that, so
/// `main` has all of it to notice the flag, roll back and exit with its own
/// code — which ends the handler's thread with the process.
#[cfg(any(windows, test))]
const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_millis(4500);

/// The console control events, as Windows hands them to the handler.
#[cfg(any(windows, test))]
mod console {
    pub const CTRL_C: u32 = 0;
    pub const CTRL_BREAK: u32 = 1;
    pub const CTRL_CLOSE: u32 = 2;
    pub const CTRL_LOGOFF: u32 = 5;
    pub const CTRL_SHUTDOWN: u32 = 6;
}

/// What the Windows handler does with `event`, `hold` being how it waits;
/// the answer is what the handler returns — `1` handled, `0` not fastf's.
///
/// **A closed console, a logoff and a shutdown hold the handler** for
/// [`CLOSE_GRACE`] after recording the hang-up: returning would let Windows
/// end the process there and then, before `main` could roll a create back.
#[cfg(any(windows, test))]
fn on_console_event(event: u32, hold: impl FnOnce(std::time::Duration)) -> i32 {
    match event {
        console::CTRL_C | console::CTRL_BREAK => {
            ctrl_c();
            1 // handled — do not run the default terminator
        }
        console::CTRL_CLOSE | console::CTRL_LOGOFF | console::CTRL_SHUTDOWN => {
            hang_up(HANG_UP_EXIT);
            hold(CLOSE_GRACE);
            1
        }
        _ => 0,
    }
}

/// Record the interrupt. Async-signal-safe: a single relaxed atomic swap, and
/// on the second signal the surface and cursor restore, which are written for
/// this context, then [`exit_now`] with `exit_code`.
fn flag(exit_code: i32) {
    // On the second interrupt, stop being polite. If the first one did not get
    // us out promptly the user should not have to reach for Task Manager.
    if INTERRUPTED.swap(true, Ordering::Relaxed) {
        // `main`'s error path never runs from here, so the screen has to be
        // given back before leaving or the shell is left in raw mode, blind.
        on_second_signal();
        exit_now(exit_code);
    }
}

/// Leave at once, from a signal handler.
///
/// **`_exit`, never `std::process::exit`, on unix**: the latter flushes
/// stdout under a lock the interrupted thread may hold inside `println!`
/// (a panic in the handler, an abort), and a second exit started while the
/// first is running waits on itself for ever — `main` exits after the first
/// interrupt, and a signal landing there would hang the process.
/// `tests/layering.rs` keeps `process::exit` off this path. A line still
/// buffered for stdout is lost, which a forced exit may lose.
#[cfg(unix)]
fn exit_now(code: i32) -> ! {
    // SAFETY: `_exit` is async-signal-safe (signal-safety(7)), touches no
    // lock and never returns.
    unsafe { libc::_exit(code) }
}

/// Windows runs a console control handler on a thread of its own, where the
/// ordinary exit is safe.
#[cfg(not(unix))]
fn exit_now(code: i32) -> ! {
    std::process::exit(code)
}

/// Ctrl-C, from either platform's handler: fastf's, unless a program it
/// started holds the terminal.
fn ctrl_c() {
    if HANDED_OVER.load(Ordering::SeqCst) == 0 {
        flag(CTRL_C_EXIT);
    }
}

/// The terminal going away — a hang-up, a terminate, a closed console —
/// which stops fastf whoever holds the terminal. `exit_code` is what a
/// second one exits with.
fn hang_up(exit_code: i32) {
    HUNG_UP.store(true, Ordering::Relaxed);
    flag(exit_code);
}

/// Install the handler. Idempotent, and never fatal: if the OS refuses, fastf
/// simply keeps the default terminate-on-Ctrl-C behaviour it had before.
pub fn install() {
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    install_platform();
}

/// Ctrl-C and Ctrl+Break, and a closed console, a logoff or a shutdown, each
/// answered on a thread Windows starts for it ([`on_console_event`]).
#[cfg(windows)]
fn install_platform() {
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
        on_console_event(ctrl_type, std::thread::sleep)
    }

    // SAFETY: registers a handler that lives as long as the process and
    // only touches atomics, and the restore hooks written for a handler.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

/// The signals fastf unwinds from: Ctrl-C, and the terminal going away —
/// a closed window or a dropped ssh session (SIGHUP) and a terminate — so a
/// create's partial folder is rolled back rather than left behind.
#[cfg(unix)]
const SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// Installed with `sigaction`, whose meaning is the same on every unix:
/// `SA_RESTART`, so a read the signal lands in carries on rather than
/// failing with `EINTR`, and all three signals blocked while the handler
/// runs, so a second one cannot start its restore inside the first's.
#[cfg(unix)]
fn install_platform() {
    extern "C" fn handler(signum: libc::c_int) {
        if signum == libc::SIGINT {
            ctrl_c();
        } else {
            hang_up(128 + signum);
        }
    }
    // Go through a plain function pointer before the integer cast. Casting the
    // function *item* straight to an integer is accepted by the compiler but
    // relies on its size matching, which clippy rightly refuses.
    let handler_ptr = handler as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: `sigaction` is a plain C struct of integers, pointers and a
    // signal set, for which all zeroes is a valid value.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handler_ptr;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: initialises the set the struct owns.
    unsafe { libc::sigemptyset(&mut action.sa_mask) };
    for signum in SIGNALS {
        // SAFETY: adds a valid signal number to the set initialised above.
        unsafe { libc::sigaddset(&mut action.sa_mask, signum) };
    }
    for signum in SIGNALS {
        // SAFETY: `handler` only swaps atomics, runs the restore hooks
        // written for a handler and `_exit`s, all async-signal-safe; the
        // previous action is not asked for.
        unsafe { libc::sigaction(signum, &action, std::ptr::null_mut()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream that is not a console is never written to: a pipe a script
    /// reads, a file a run was redirected into.
    #[cfg(windows)]
    #[test]
    fn a_handle_that_is_no_console_is_left_alone() {
        use std::os::windows::io::AsRawHandle;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("out.txt");
        let file = std::fs::File::create(&path).unwrap();
        assert!(!show_console_cursor(file.as_raw_handle()));
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"");
    }

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

    /// A Ctrl-C typed while a program fastf started holds the terminal is
    /// that program's; the window closing still stops fastf.
    #[test]
    fn a_handed_over_terminal_keeps_ctrl_c_and_not_the_hang_up() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        {
            let _held = hand_over();
            ctrl_c();
            assert!(!is_set(), "the Ctrl-C was the program's");
            hang_up(128 + 1);
            assert!(is_set() && hung_up(), "a closed window still stops fastf");
        }
        reset();
        ctrl_c();
        assert!(is_set(), "given back, Ctrl-C is fastf's again");
        reset();
    }

    /// **A closed console holds Windows off while `main` unwinds**: the
    /// hang-up is recorded, then the handler waits under Windows' five
    /// seconds rather than returning, which would end the process before a
    /// create could roll back. Ctrl-C is answered at once.
    #[test]
    fn a_closed_console_holds_the_handler_while_main_unwinds() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let mut held = None;
        let answer = on_console_event(console::CTRL_CLOSE, |grace| held = Some(grace));
        assert_eq!(answer, 1);
        assert!(is_set() && hung_up(), "the hang-up is recorded first");
        let held = held.expect("the handler waits before it returns");
        assert!(
            held >= std::time::Duration::from_secs(4) && held < std::time::Duration::from_secs(5),
            "{held:?}: long enough to unwind, inside Windows' five seconds"
        );

        reset();
        let mut waited = false;
        assert_eq!(on_console_event(console::CTRL_C, |_| waited = true), 1);
        assert!(is_set() && !hung_up() && !waited, "Ctrl-C does not wait");
        reset();
        assert_eq!(on_console_event(3, |_| waited = true), 0, "not fastf's");
        assert!(!is_set() && !waited);
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
