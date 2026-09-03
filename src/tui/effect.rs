//! What `update` asks the runtime to do. `update` itself does no I/O.

use std::path::PathBuf;

use crate::core::library::Project;
use crate::util::size_scan::SizeCell;

/// Ties a worker's answer back to the request that started it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionId(pub u64);

#[derive(Debug, PartialEq)]
pub enum Effect {
    /// Probe the bases, read the indexes, list the templates — the header.
    LoadSummary,
    /// `library::discover` on a worker. The generation tells a late answer
    /// from a current one.
    Discover {
        generation: u64,
    },
    /// Metadata, journal and listing for the detail pane. Latest request wins.
    LoadDetail(PathBuf),
    /// Metadata for rows whose variables a query needs.
    LoadMeta(Vec<PathBuf>),
    /// What the size scanner should measure next, most important first.
    RequestSizes(Vec<PathBuf>),
    /// Snapshots that are stale after a mutation.
    ForgetSizes(Vec<PathBuf>),
    /// One mutation, on a worker; answered by `Msg::ActionDone`.
    Run(ActionId, Box<Action>),
    /// Start another program on the user's behalf; answered by `Msg::Spawned`.
    Spawn(SpawnKind),
    /// Give the terminal back, run something that needs it, take it again.
    Suspend(Suspended),
    Quit(Exit),
}

/// A mutation the runtime performs through `core::operations`.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Reindex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    /// Reveal the folder in the file manager.
    Reveal(Box<Project>),
    /// A terminal window in the folder.
    Terminal(Box<Project>),
    /// Put text on the clipboard.
    Clipboard(String),
}

/// Something that needs the terminal in cooked mode on the main screen.
#[derive(Debug, PartialEq)]
pub enum Suspended {
    Legacy(LegacyFlow),
}

/// The dialoguer flows that are not native yet. Each phase of the rebuild
/// removes the variants it makes native; the enum is gone when they all are.
#[derive(Debug, PartialEq)]
pub enum LegacyFlow {
    Create,
    Register,
    Templates,
    Settings,
    ActionMenu {
        project: Box<Project>,
        size: Option<SizeCell>,
        known_tags: Vec<String>,
    },
}

impl LegacyFlow {
    pub fn title(&self) -> &'static str {
        match self {
            LegacyFlow::Create => "create a project",
            LegacyFlow::Register => "register a folder",
            LegacyFlow::Templates => "templates",
            LegacyFlow::Settings => "settings",
            LegacyFlow::ActionMenu { .. } => "project actions",
        }
    }

    /// Whether the flow's last output deserves a pause before the app redraws.
    /// The menus loop until Back, so the user has already read their output;
    /// create and register print a result and return.
    pub fn pauses(&self) -> bool {
        matches!(self, LegacyFlow::Create | LegacyFlow::Register)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Normal,
    /// Ctrl-C: `main` says `aborted.` and exits 130.
    Interrupted,
}

/// What a mutation did to the list. The distinction that matters is `Patched`
/// versus `Reload`: a content change rewrites one row the list already holds,
/// and only a change the list cannot reason about re-reads every base.
///
/// `stale` names the paths whose size snapshot must be dropped: the new
/// location always, the old one too after a move or rename.
#[derive(Debug, PartialEq, Eq)]
pub enum ListChange {
    Patched {
        project: Box<Project>,
        stale: Vec<PathBuf>,
    },
    Removed {
        path: PathBuf,
    },
    Reload,
    None,
}

/// What a worker reports back from one action.
#[derive(Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    pub change: ListChange,
    /// The status line.
    pub message: String,
    pub warning: Option<String>,
    /// The header's session ring, e.g. `tagged ID0248 draft`.
    pub session: Option<String>,
}
