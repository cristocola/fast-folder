//! The interactive menu, driven through a real terminal.
//!
//! The TUI had no tests at all, and the defect that mattered most could only be
//! seen from a terminal: **any recoverable error ended the session**. A mistyped
//! path in Register — after three answered prompts — or an out-of-range value in
//! Settings unwound all the way to `main` and exited 1, dropping the user back
//! to the shell with everything they had typed gone.
//!
//! Unix only by construction: `dialoguer` refuses to prompt without a pty, and
//! the logic behind containment is covered cross-platform by the `is_fatal` unit
//! tests in `tui::menu`.
//!
//! Three suites in one binary — see `harness` for why one.

#![cfg(unix)]

mod common;

#[path = "tui_pty/harness.rs"]
mod harness;

#[path = "tui_pty/browser.rs"]
mod browser;
#[path = "tui_pty/flows.rs"]
mod flows;
#[path = "tui_pty/menu.rs"]
mod menu;
