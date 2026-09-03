//! Everything that can happen to the app: a key, a worker's answer, a tick.
//!
//! `update` is a function of the app and one of these, and nothing else. That
//! is what makes the state machine testable without a terminal — a test builds
//! an `App`, feeds it messages, and asserts on the effects that come back.

use std::path::PathBuf;

use crate::core::library::Project;
use crate::core::project_info::Metadata;
use crate::tui::app::data::{ProjectDetail, Summary, TemplateInfo};
use crate::tui::app::wizard::Preview;
use crate::tui::command::Key;
use crate::tui::effect::{ActionId, ActionOutcome, SpawnKind};
use crate::util::diag::Level;

#[derive(Debug)]
pub enum Msg {
    /// A key press (or repeat — never a release).
    Key(Key),
    /// A mouse click or a wheel turn, in terminal cells.
    Mouse(Mouse),
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
    /// A running move reported its progress, once per tick.
    MoveProgress(crate::core::assets::Progress),
    /// One template was read in full for the open flow.
    TemplateLoaded {
        slug: String,
        result: Result<Box<TemplateInfo>, String>,
    },
    /// One template's whole document, for the builder to edit.
    TemplateSourceLoaded {
        slug: String,
        result: Result<Box<crate::core::template::Template>, String>,
    },
    /// One template's details, for the studio to show.
    TemplateViewLoaded {
        slug: String,
        lines: Vec<String>,
    },
    /// The settings, read back.
    SettingsLoaded(Box<crate::tui::app::data::Settings>),
    SettingsFailed(String),
    /// A flow's preview is ready.
    Previewed(Box<Preview>),
    /// A flow's preview could not be built. `field` names the answer that was
    /// wrong, so the refusal lands on the line that caused it and the rest of
    /// the form stays exactly as it was typed.
    PreviewFailed {
        field: Option<String>,
        error: String,
    },
    /// A read-only view's content landed.
    ViewLoaded {
        title: String,
        lines: Vec<String>,
    },
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

/// What the mouse did, and where. Only the three gestures a terminal reports
/// reliably: the wheel, and the left button going down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mouse {
    pub kind: MouseKind,
    pub column: u16,
    pub row: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Click,
    ScrollUp,
    ScrollDown,
}

/// What a suspended flow left behind.
#[derive(Debug)]
pub enum Resumed {
    /// The editor closed. `text` is `None` when nothing worth appending was
    /// written (the editor was cancelled, or left the scratch empty).
    Note {
        project: Box<Project>,
        text: Option<String>,
    },
    /// A new project's post-create actions ran on the main screen.
    PostCreate,
}
