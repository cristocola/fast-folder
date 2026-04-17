//! fastf as a library — exposes internals so `tests/integration.rs` can
//! exercise the core logic without spawning a subprocess. The binary at
//! `src/main.rs` mirrors this layout.

pub mod bootstrap;
pub mod cli;
pub mod core;
pub mod tui;
pub mod util;
