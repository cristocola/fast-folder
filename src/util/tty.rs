//! Is there a terminal to prompt on, and what to say when there is not.
//!
//! Every guard in fastf used to probe **stdout**, which is not where a prompt
//! happens: a prompt draws on stderr and reads from stdin (falling back to
//! `/dev/tty`). The probe therefore answered a different question than the one
//! being asked. `fastf new t > out.txt` refused although a terminal was right
//! there, and `fastf new t 2>/dev/null` passed the guard and died on
//! a bare "not a terminal" failure, which tells a script author
//! nothing about what to do.
//!
//! Stdout still decides **output format** — `recent`/`search` print their plain
//! list when piped, and the move progress line is skipped. That is a genuinely
//! different question, and those probes stay where they are.

use anyhow::{Result, bail};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

/// Did anything this run actually stop and wait for the user?
///
/// Only `main` reads it, and only to decide whether a relaunched terminal window
/// should pause before it closes: a window that just printed a list must not
/// vanish before it can be read, and one that ran a menu already had the user's
/// attention for as long as they wanted it.
static SURFACE_RAN: AtomicBool = AtomicBool::new(false);

/// Can a prompt be drawn and answered right now?
///
/// Stderr is the stream a prompt writes to, so it is the one that decides.
pub fn prompt_available() -> bool {
    std::io::stderr().is_terminal()
}

/// Refuse an action that needs a prompt when there is no terminal for it,
/// naming what it wanted to ask and how to get the same result without asking.
///
/// `what` completes "no terminal to _ on"; `how` is a full sentence naming the
/// flag or setting that avoids the prompt.
pub fn require_tty(what: &str, how: &str) -> Result<()> {
    if prompt_available() {
        // One of exactly two choke points — every prompt reaches here through
        // `prompt::ready()`, and every picker through that. The other is
        // `tui::runtime::Runtime::init`, which the guided app reaches after its
        // own `require_tty` and before it takes the screen.
        mark_interactive_surface();
        return Ok(());
    }
    bail!("no terminal to {what} on — {how}")
}

/// Record that a prompt, a picker or the app was drawn and waited on.
/// The terminal's settings before raw mode was switched on, kept so a signal
/// handler can put them back without going through crossterm — whose
/// `disable_raw_mode` takes a lock, which a handler may not.
#[cfg(unix)]
static COOKED: std::sync::OnceLock<libc::termios> = std::sync::OnceLock::new();

/// Remember the terminal's current settings, once — called before the first
/// `enable_raw_mode`, so what is remembered is the shell's own mode.
#[cfg(unix)]
pub fn remember_cooked_mode() {
    COOKED.get_or_init(|| {
        // SAFETY: a zeroed termios is a valid value for tcgetattr to fill.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: stderr is a descriptor this process owns; a failure leaves
        // the zeroed value, and `restore_cooked_mode` checks it was filled.
        let ok = unsafe { libc::tcgetattr(libc::STDERR_FILENO, &mut termios) } == 0;
        if !ok {
            termios.c_lflag = 0;
        }
        termios
    });
}

/// Put the remembered settings back. Async-signal-safe: one `tcsetattr`.
#[cfg(unix)]
pub fn restore_cooked_mode() {
    if let Some(termios) = COOKED.get()
        && termios.c_lflag != 0
    {
        // SAFETY: a termios `tcgetattr` filled, applied to the same descriptor.
        unsafe {
            libc::tcsetattr(libc::STDERR_FILENO, libc::TCSANOW, termios);
        }
    }
}

/// Write bytes to stderr without a lock or an allocation — what a signal
/// handler may do.
#[cfg(unix)]
pub fn write_raw(bytes: &[u8]) {
    // SAFETY: `write` is async-signal-safe and the descriptor is ours.
    unsafe {
        let _ = libc::write(libc::STDERR_FILENO, bytes.as_ptr().cast(), bytes.len());
    }
}

/// `write_raw` where there is no signal-safety rule to keep.
#[cfg(not(unix))]
pub fn write_raw(bytes: &[u8]) {
    use std::io::Write;
    let _ = std::io::stderr().write_all(bytes);
    let _ = std::io::stderr().flush();
}

/// Whether a person could see what a program started from here would draw: a
/// desktop session on unix (`DISPLAY` or `WAYLAND_DISPLAY`); always, on
/// Windows and macOS, where a window needs no variable.
pub fn has_display() -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        return true;
    }
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// Whether this console is a **pseudoconsole** — drawn by a terminal emulator
/// (Windows Terminal, VS Code, an ssh client) rather than the legacy console
/// window. Windows 11 hands a program started from the Start menu or a
/// shortcut to Windows Terminal *without* setting `WT_SESSION`, so the variable
/// alone takes Windows Terminal for the legacy console there. A pseudoconsole's
/// window is a hidden `PseudoConsoleWindow`; the legacy one is a
/// `ConsoleWindowClass`. Always `false` off Windows.
#[cfg(windows)]
pub fn pseudo_console() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> *mut core::ffi::c_void;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetClassNameW(hwnd: *mut core::ffi::c_void, name: *mut u16, capacity: i32) -> i32;
    }
    const PSEUDO: &str = "PseudoConsoleWindow";
    let mut name = [0u16; 64];
    // SAFETY: `GetConsoleWindow` takes nothing and may answer null, which is
    // checked; `GetClassNameW` writes at most `capacity` units into `name`,
    // which is that long, and answers how many it wrote.
    let written = unsafe {
        let window = GetConsoleWindow();
        if window.is_null() {
            return false;
        }
        GetClassNameW(window, name.as_mut_ptr(), name.len() as i32)
    };
    usize::try_from(written).is_ok_and(|len| String::from_utf16_lossy(&name[..len]) == PSEUDO)
}

#[cfg(not(windows))]
pub fn pseudo_console() -> bool {
    false
}

pub fn mark_interactive_surface() {
    SURFACE_RAN.store(true, Ordering::Relaxed);
}

/// Did any interactive surface run this process?
pub fn interactive_surface_ran() -> bool {
    SURFACE_RAN.load(Ordering::Relaxed)
}
