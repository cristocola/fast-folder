//! Everything that can happen to the app: a key, a worker's answer, a tick.
//!
//! `update` is a function of the app and one of these, and nothing else. That
//! is what makes the state machine testable without a terminal — a test builds
//! an `App`, feeds it messages, and asserts on the effects that come back.

use std::path::PathBuf;

use crate::core::library::Project;
use crate::core::project_info::Metadata;
use crate::tui::app::data::{ProjectDetail, Summary};
use crate::tui::command::Key;
use crate::tui::effect::{ActionId, ActionOutcome, ListChange, SpawnKind};
use crate::util::diag::Level;

#[derive(Debug)]
pub enum Msg {
    /// A key press (or repeat — never a release).
    Key(Key),
    /// Bracketed paste, straight into whichever text field has the caret.
    Paste(String),
    Resize(u16, u16),
    /// Sent only while `App::needs_tick` says something on screen is moving.
    Tick,
    /// Folder sizes that landed since the last tick.
    Sizes(Vec<(PathBuf, Option<u64>)>),
    Summary(Box<Summary>),
    SummaryFailed(String),
    Discovered {
        generation: u64,
        projects: Vec<Project>,
    },
    DiscoverFailed {
        generation: u64,
        error: String,
    },
    Detail {
        path: PathBuf,
        detail: Box<ProjectDetail>,
    },
    /// Metadata read on demand, for a query that needs template variables.
    MetaLoaded(Vec<(PathBuf, Option<Metadata>)>),
    ActionDone {
        id: ActionId,
        outcome: Result<Box<ActionOutcome>, String>,
    },
    /// A program was started on the user's behalf (or could not be).
    Spawned {
        what: SpawnKind,
        outcome: Result<String, String>,
    },
    /// The terminal is ours again after a suspended flow.
    Resumed(Resumed),
    /// A warning from `core`/`util` that would otherwise have hit stderr.
    Diag(Level, String),
    /// An external SIGINT/SIGTERM was observed.
    Interrupted,
}

/// What a suspended flow left behind.
#[derive(Debug)]
pub enum Resumed {
    /// One of the dialoguer flows that is not native yet ran and returned.
    Legacy { change: ListChange, quit: bool },
}
