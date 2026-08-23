//! fastf as a library — exposes internals so the integration suites under
//! `tests/` can exercise the core logic without spawning a subprocess. The
//! binary at `src/main.rs` mirrors this layout.

pub mod bootstrap;
pub mod cli;
pub mod core;
pub mod tui;
pub mod util;
