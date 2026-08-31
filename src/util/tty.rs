//! Is there a terminal to prompt on, and what to say when there is not.
//!
//! Every guard in fastf used to probe **stdout**, which is not where a prompt
//! happens: `dialoguer` draws on stderr and reads from stdin (falling back to
//! `/dev/tty`). The probe therefore answered a different question than the one
//! being asked. `fastf new t > out.txt` refused although a terminal was right
//! there, and `fastf new t 2>/dev/null` passed the guard and died on
//! dialoguer's bare "IO error: not a terminal", which tells a script author
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
/// Stderr is the stream dialoguer writes to, so it is the one that decides.
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
        // One of exactly two choke points — every dialoguer prompt reaches here
        // through `prompt::ready()`, and every picker through that. The other is
        // `live_select`, which the browser reaches without passing this way.
        mark_interactive_surface();
        return Ok(());
    }
    bail!("no terminal to {what} on — {how}")
}

/// Record that a prompt, picker or menu was drawn and waited on.
pub fn mark_interactive_surface() {
    SURFACE_RAN.store(true, Ordering::Relaxed);
}

/// Did any interactive surface run this process?
pub fn interactive_surface_ran() -> bool {
    SURFACE_RAN.load(Ordering::Relaxed)
}
