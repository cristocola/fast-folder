//! The one place fastf asks a question on the command line.
//!
//! Every prompt outside the guided app goes through here, and it is a thin
//! layer over [`crate::tui::inline`]: this module owns the *contract* — the
//! terminal guard, and `Ok(None)` meaning cancelled — while `inline` owns the
//! drawing. The split is what `tests/layering.rs` enforces, and the defect it
//! exists for was never a wrong behaviour but an inconsistent one: an earlier
//! attempt moved twenty-nine prompts to a cancellable form by hand and missed
//! several, so Esc backed out of some menus and was swallowed by others, which
//! is worse than Esc never working.
//!
//! **`Ok(None)` is a cancelled prompt. It is never an error**, so `cli`'s error
//! handling keeps classifying a *broken* prompt (no terminal, stdin at EOF) as
//! fatal and a cancelled one as an ordinary answer.

use anyhow::Result;
use colored::Colorize;

use crate::util::tty;

pub use crate::tui::inline::TextOpts;

/// Guard shared by all three: a prompt that cannot be drawn is refused with a
/// message, never left to fail inside the terminal.
///
/// Call sites that know a flag which answers the same question call
/// `tty::require_tty` themselves first, with that flag named; this is the
/// backstop for the ones that do not.
fn ready() -> Result<()> {
    tty::require_tty(
        "prompt",
        "run the command with its flags instead of interactively",
    )
}

/// Pick one item. `Ok(None)` is Esc or `q`.
pub fn select(prompt: &str, items: &[String], default: usize) -> Result<Option<usize>> {
    ready()?;
    crate::tui::inline::select(prompt, items, default)
}

/// Yes or no. A bare `y`/`n` answers without Enter, Enter takes the default,
/// Esc or `q` cancels.
pub fn confirm(prompt: &str, default: bool) -> Result<Option<bool>> {
    ready()?;
    crate::tui::inline::confirm(prompt, default)
}

/// Read a line. `Ok(None)` is Esc.
pub fn text(prompt: &str, opts: TextOpts<'_>) -> Result<Option<String>> {
    ready()?;
    crate::tui::inline::text(prompt, opts)
}

/// The one sentence a cancelled flow prints before returning to where it came
/// from. `what` completes "Cancelled — _": say what did *not* happen, since the
/// reassurance is the point ("nothing was created", not "aborted").
pub fn report_cancelled(what: &str) {
    println!("{}", format!("Cancelled — {what}.").dimmed());
}
