//! The guided app's state machine, driven without a terminal.
//!
//! `App` plus one `Msg` in, the `Effect`s out: these tests feed keys and worker
//! answers to an app built from fixtures and assert on what it asks the runtime
//! to do. Nothing here touches a disk or a screen.
//!
//! One binary with a module per subject under `tests/tui_update/`, so the crate
//! is linked once; `harness` holds the imports and helpers every module shares.

#[path = "tui_update/harness.rs"]
mod harness;

#[path = "tui_update/actions.rs"]
mod actions;
#[path = "tui_update/batches.rs"]
mod batches;
#[path = "tui_update/edges.rs"]
mod edges;
#[path = "tui_update/flows.rs"]
mod flows;
#[path = "tui_update/guide.rs"]
mod guide;
#[path = "tui_update/key_lines.rs"]
mod key_lines;
#[path = "tui_update/library.rs"]
mod library;
#[path = "tui_update/log.rs"]
mod log;
#[path = "tui_update/marks.rs"]
mod marks;
#[path = "tui_update/more_options.rs"]
mod more_options;
#[path = "tui_update/motion.rs"]
mod motion;
#[path = "tui_update/movement.rs"]
mod movement;
#[path = "tui_update/pane_cursor.rs"]
mod pane_cursor;
#[path = "tui_update/pane_editor.rs"]
mod pane_editor;
#[path = "tui_update/registry.rs"]
mod registry;
#[path = "tui_update/session.rs"]
mod session;
#[path = "tui_update/settings.rs"]
mod settings;
#[path = "tui_update/studio.rs"]
mod studio;
#[path = "tui_update/terminal.rs"]
mod terminal;
#[path = "tui_update/verbs.rs"]
mod verbs;
