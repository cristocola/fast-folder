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
        return Ok(());
    }
    bail!("no terminal to {what} on — {how}")
}
