//! What `update` asks the runtime to do. `update` itself does no I/O.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::library::Project;
use crate::tui::app::register;

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
    /// Read one project's full metadata or journal for a read-only view;
    /// answered by `Msg::ViewLoaded`.
    LoadView {
        title: String,
        path: PathBuf,
        kind: ViewKind,
    },
    /// Read one template in full — its variables — for the flow that asked.
    LoadTemplate {
        slug: String,
    },
    /// Read one template as the builder needs it: the whole document, with the
    /// text under `files/` buffered so the editor can show it.
    LoadTemplateSource {
        slug: String,
    },
    /// Read one template as the studio shows it: `template show`'s lines.
    LoadTemplateView {
        slug: String,
    },
    /// Read every setting the settings screen shows.
    LoadSettings,
    /// The `theme` setting was read back: pick the palette again with it,
    /// answered by `Msg::Themed`. `update` cannot read the environment, so the
    /// choice is the runtime's.
    Retheme(String),
    /// Work out what a flow's answers would do, without touching a disk.
    /// Answered by `Msg::Previewed`, or by `Msg::PreviewFailed` naming the
    /// field that was wrong.
    Preview(Box<Request>),
    /// Cancel the move job that is running.
    CancelMove,
    /// Give the terminal back, run something that needs it, take it again.
    Suspend(Suspended),
    Quit(Exit),
}

/// What a flow wants previewed, and then committed. The same value serves
/// both, so the screen cannot show a plan built one way and commit one built
/// another — which is exactly how a rename prompt came to offer `ID0001` and
/// write `ID0011`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Create(CreateRequest),
    Apply(ApplyRequest),
    Register(register::Request),
    FromFolder(FromFolderRequest),
}

/// Generate a template from a folder that already has the shape you want.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromFolderRequest {
    pub source: PathBuf,
    pub slug: String,
    /// Overwrite a template that already answers to this slug.
    pub force: bool,
    /// Copy binary and oversized files into the template byte for byte.
    pub bundle_assets: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequest {
    pub template_slug: String,
    pub vars: HashMap<String, String>,
    /// `None` uses the configured base.
    pub base_dir_override: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyRequest {
    pub template_slug: String,
    pub target: PathBuf,
    pub vars: HashMap<String, String>,
}

/// A mutation the runtime performs through `core::operations`.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Reindex,
    /// Create a project. Post-create actions are **not** run here: they spawn
    /// the user's editor and shell commands, which need the main screen — see
    /// `Suspended::PostCreate`.
    Create(Box<CreateRequest>),
    Apply(Box<ApplyRequest>),
    /// Register one folder, or every unregistered child of a base.
    Register(Box<register::Request>),
    /// Write the template the builder assembled. `original_slug` is what it
    /// was loaded under, so a renamed template moves rather than forking.
    SaveTemplate {
        template: Box<crate::core::template::Template>,
        original_slug: Option<String>,
    },
    DeleteTemplate(String),
    TemplateFromFolder(Box<FromFolderRequest>),
    /// One setting, written by `cli::config::apply` — the same expression
    /// `fastf config set` uses, so a refusal is the refusal it has always made.
    SetConfig {
        key: &'static str,
        value: String,
    },
    /// First run: create the projects folder and record it.
    InitBaseDir(String),
    /// Raise the global counter. It never goes down.
    RaiseCounter(u64),
    /// Make every mounted base agree on the highest ID seen anywhere.
    SyncCounters,
    /// Finish or roll back work a crash left half-done.
    Reconcile,
    AddTag {
        project: Box<Project>,
        tag: String,
    },
    RemoveTags {
        project: Box<Project>,
        tags: Vec<String>,
    },
    ReautoTags(Box<Project>),
    Rename {
        project: Box<Project>,
        name: String,
    },
    /// A single move, one item for the one-item job runner. The runtime owns
    /// the progress and cancel handles.
    Move {
        project: Box<Project>,
        target: PathBuf,
    },
    /// Copy a project to a folder outside every base, keeping its id.
    CopyTo {
        project: Box<Project>,
        destination: PathBuf,
    },
    Unregister(Box<Project>),
    Delete(Box<Project>),
    AppendNote {
        project: Box<Project>,
        text: String,
    },
}

/// Which read-only view `LoadView` is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Metadata,
    Journal,
    /// Where fastf keeps its things. Reads nothing at the path it is given —
    /// the one view that is about the installation rather than a project.
    DataLocations,
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
    /// Open `$EDITOR` on a scratch file for the selected project's journal,
    /// then come back with what was written.
    Note(Box<Project>),
    /// Run a finished create's post-create actions. They run `git init`, open
    /// the user's editor and execute the template's own shell commands, all of
    /// which want a terminal and print to it — so the screen goes back before
    /// they start, exactly as the note editor's does.
    PostCreate {
        root: PathBuf,
        template_slug: String,
    },
    /// Ctrl-Z: give the terminal back and stop, as any program does; `fg`
    /// brings the app back with its screen retaken. Unix only.
    Shell,
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
        /// Where the row was before the action. **Two projects can carry the
        /// same id** once `copy-to` has put one on a backup drive and that
        /// drive is added as a base, so the row is found by its old path first
        /// and only then by id — which is still what a rename or a move needs,
        /// since those are exactly the actions that change the path.
        was: PathBuf,
        stale: Vec<PathBuf>,
    },
    Removed {
        path: PathBuf,
    },
    Reload,
    /// The projects did not move, but what the header and the templates tab
    /// say about the library did — a template was written, renamed or deleted.
    /// Re-reading every base for that would be a walk to answer a question
    /// none of the folders were asked.
    SummaryOnly,
    None,
}

/// What still has to happen on the main screen once an action has committed.
#[derive(Debug, PartialEq, Eq)]
pub enum FollowUp {
    PostCreate {
        root: PathBuf,
        template_slug: String,
    },
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
    /// A row to put the cursor on once the list has caught up: what a create
    /// or a register just made, which is never in the list the action started
    /// from.
    pub select: Option<PathBuf>,
    /// Work that needs the main screen; run after the list has been updated.
    pub follow_up: Option<FollowUp>,
    /// Re-read the settings: this action changed one of them, and the screen
    /// showing them is a function of what is on disk, not of what was typed.
    pub reload_settings: bool,
}

impl ActionOutcome {
    /// The two things every outcome has. The rest are chained on, so adding a
    /// field does not touch every verb that never sets it.
    pub fn new(change: ListChange, message: impl Into<String>) -> Self {
        Self {
            change,
            message: message.into(),
            warning: None,
            session: None,
            select: None,
            follow_up: None,
            reload_settings: false,
        }
    }

    /// The settings screen re-reads itself after this.
    pub fn settings(mut self) -> Self {
        self.reload_settings = true;
        self
    }

    pub fn warning(mut self, warning: Option<String>) -> Self {
        self.warning = warning;
        self
    }

    pub fn session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    pub fn select(mut self, path: PathBuf) -> Self {
        self.select = Some(path);
        self
    }

    pub fn follow_up(mut self, follow_up: FollowUp) -> Self {
        self.follow_up = Some(follow_up);
        self
    }
}
