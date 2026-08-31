//! Opening a terminal when there is not one — the command layer's half.
//!
//! `util::relaunch` decides whether this process has nowhere to write and knows
//! how to drive the emulators; this module is where that meets `Config` (which
//! `util` may not read) and the `--plain` flag, and where the fallbacks are
//! decided. It is also the one place the Windows/unix split is spelled out, so
//! no caller needs a `cfg` of its own.

use crate::core::config::Config;

/// Hand this command over to a terminal emulator, if it has none and the user
/// is at a desktop.
///
/// `true` means a terminal now owns the rerun and the caller must return
/// `Ok(())` without doing the work: doing it twice would create two projects.
/// `false` means carry on exactly as before — including when no emulator could
/// be started, because falling through to unreadable output is still better
/// than failing.
pub fn hand_off_to_a_terminal(cfg: &Config, plain: bool) -> bool {
    // `--plain` is a promise that the output is for a machine. It is also the
    // first of the three documented ways to switch this off.
    if plain {
        return false;
    }
    relaunch_impl(cfg)
}

/// Was this process started with no terminal at all, inside a graphical
/// session? The condition `copy` and `path` use to decide whether anyone could
/// possibly have seen what they printed.
pub fn headless_gui() -> bool {
    #[cfg(unix)]
    {
        crate::util::relaunch::headless_gui_session()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// A desktop notification, where the platform has one. `true` if it went out.
pub fn notify(summary: &str, body: &str) -> bool {
    #[cfg(unix)]
    {
        crate::util::notify::notify(summary, body)
    }
    #[cfg(not(unix))]
    {
        let _ = (summary, body);
        false
    }
}

#[cfg(unix)]
fn relaunch_impl(cfg: &Config) -> bool {
    use crate::core::config::TerminalPreference;
    use crate::util::relaunch;

    if !relaunch::headless_gui_session() {
        return false;
    }
    let preference = cfg.resolve_terminal();
    if preference == TerminalPreference::Disabled {
        return false;
    }
    match relaunch::respawn_in_terminal(preference.name()) {
        Ok(()) => true,
        Err(e) => {
            // Nothing printed here can be read — that is the situation. The
            // notification is the only channel left, and it is best-effort.
            notify("fastf needs a terminal", &format!("{e:#}"));
            false
        }
    }
}

/// Windows has no relaunch machinery: launching a console application from the
/// shell surface already allocates a console.
#[cfg(not(unix))]
fn relaunch_impl(cfg: &Config) -> bool {
    let _ = cfg;
    false
}
