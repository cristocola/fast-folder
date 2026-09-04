//! Opening a terminal when there is not one — the command layer's half.
//!
//! `util::relaunch` decides whether this process has nowhere to write and knows
//! how to drive the emulators; this module is where that meets `Config` (which
//! `util` may not read) and the `--plain` flag, and where the fallbacks are
//! decided. It is also the one place the Windows/unix split is spelled out, so
//! no caller needs a `cfg` of its own.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::config::Config;

/// Set by [`mark_relaunched_window`] from the parsed `--relaunched`. See
/// [`relaunched_window`].
static RELAUNCHED_WINDOW: AtomicBool = AtomicBool::new(false);

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

/// Record that this process was started by [`crate::util::relaunch`] — `main`
/// calls it for the hidden `--relaunched` flag, and nothing else may.
pub fn mark_relaunched_window() {
    RELAUNCHED_WINDOW.store(true, Ordering::SeqCst);
}

/// **Is this process the rerun the relaunch started, in the window it opened?**
///
/// The claim rides on argv (`--relaunched`), not on `FASTF_RELAUNCHED`, and that
/// is the whole point: an environment variable is inherited, so every shell in
/// that window has it and so does everything typed into that shell, none of
/// which is the rerun. Read the wrong way it made `fastf completions bash` stop
/// for a keypress in a package build, and `fastf term proj` replace the shell
/// it was typed into instead of opening a window.
///
/// The variable keeps the one job inheritance cannot spoil: in
/// [`crate::util::relaunch::headless_gui_session`] it only ever *suppresses* a
/// relaunch, so a descendant that wrongly inherits it opens no window, which is
/// the safe direction and the one a runaway loop would need.
pub fn relaunched_window() -> bool {
    RELAUNCHED_WINDOW.load(Ordering::SeqCst)
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

/// Open a terminal window whose shell starts at `dir`.
///
/// An *explicit* request — `fastf term`, the action menu's "Open terminal
/// here" — so `terminal = "none"`, which switches off the automatic relaunch,
/// does not switch this off: `Disabled` degrades to probing, exactly like an
/// unconfigured terminal.
pub fn open_terminal_at(cfg: &Config, dir: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use crate::core::config::TerminalPreference;
        let preference = cfg.resolve_terminal();
        let name = match &preference {
            TerminalPreference::Named(_) => preference.name(),
            TerminalPreference::Disabled | TerminalPreference::Probe => None,
        };
        crate::util::term_open::open_terminal_at(name, dir)
    }
    #[cfg(not(unix))]
    {
        let _ = cfg;
        crate::util::term_open::open_terminal_at(None, dir)
    }
}

/// Is this process the sole occupant of a window the relaunch opened for it —
/// started with `--relaunched`, with a real terminal on stdin and stderr?
///
/// When it is, "open a terminal at the project" means *become the shell here*:
/// this window exists only because fastf had a picker to show, and spawning a
/// second one would strand it.
///
/// [`relaunched_window`] is the same guard `main`'s pause needs and for the same
/// reason: read from the inherited variable instead, `fastf term proj` typed
/// into any shell that carries one replaced *that* shell rather than opening
/// the window it was asked for.
pub fn window_is_ours() -> bool {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        relaunched_window() && std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
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
