//! The guided app, driven through a real terminal.
//!
//! The app cannot draw without a terminal, and the defects this suite exists
//! for were only ever visible from one: the list drawing before a single
//! folder had been walked, a mutation patching one row rather than rescanning
//! the library, a flow that returned to the dashboard instead of ending the
//! session. The pure state machine is covered without a terminal in
//! `tui_update.rs`; what is left here is the runtime — the screen, the
//! threads, and the command line's own inline prompts — which a test backend
//! cannot see.
//!
//! Unix only by construction: the harness is `libc::forkpty`.
//!
//! Three suites in one binary — see `harness` for why one — plus the
//! screenshot tool and its SVG renderer.

#![cfg(unix)]

mod common;

#[path = "tui_pty/harness.rs"]
mod harness;

#[path = "tui_pty/app.rs"]
mod app;
#[path = "tui_pty/flows.rs"]
mod flows;
#[path = "tui_pty/list.rs"]
mod list;
#[path = "tui_pty/screenshot.rs"]
mod screenshot;
#[path = "tui_pty/svg.rs"]
mod svg;
