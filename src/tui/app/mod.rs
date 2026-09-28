//! The model, and `update`: one function of the app and one message, with no
//! I/O of its own. Everything it wants done comes back as an `Effect`.
//!
//! That split is what makes the guided app testable without a terminal
//! (`tests/tui_update/` builds an `App`, feeds it messages and asserts on the
//! effects) and what keeps a slow filesystem out of the key handler: nothing in
//! here blocks, because nothing in here reads a disk.
//!
//! This file holds the types, `new`/`start` and `update`. The rest of the
//! app's own `impl App` is in `geometry`, `status`, `listing`, `messages`,
//! `keys` and `run`; each flow module carries its own for the keys and answers
//! that belong to it.

pub mod actions;
pub mod attention;
pub mod background;
pub mod data;
pub mod jobs;
pub mod library;
pub mod modal;
pub mod palette;
pub mod pane;
pub mod register;
pub mod search;
pub mod settings;
pub mod studio;
pub mod wizard;

mod builder;
mod geometry;
mod keys;
mod listing;
mod messages;
mod pane_add;
mod pane_cursor;
mod pane_edit;
mod run;
mod status;
mod templates_tab;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;

use crate::core::library::Project;
use crate::tui::app::actions::{
    Confirm, ConfirmThen, MultiPick, MultiThen, NoteState, TextPrompt, TextThen,
};
use crate::tui::command::{self, Availability, CommandId, Context, Key};
use crate::tui::effect::{
    Action, ActionId, ActionOutcome, Effect, Exit, FollowUp, ListChange, SpawnKind, Suspended,
};
use crate::tui::entry::Entry;
use crate::tui::fuzzy::Fuzzy;
use crate::tui::layout;
use crate::tui::motion;
use crate::tui::msg::{Msg, Resumed};
use crate::tui::theme::Theme;
use crate::tui::validators;
use crate::util::diag::Level;
use crate::util::size_scan::SizeCell;
use data::{ProjectDetail, Summary, SummaryPart, TemplateCard};
use library::{LibraryState, Order, Sort};
use modal::{Activity, ActivityPage, MessageLevel, Modal, ModalStack, PickItem, PickState, Then};
use search::SearchState;
use settings::Editing;
use studio::{Builder, Open, Studio};
use wizard::{Flow, FlowKind, Step};

/// How long a status message stays.
const STATUS_MS: u64 = 6_000;
/// The wake a spinner and a countdown want: five frames a second.
const SLOW_FRAME_MS: u64 = 200;
/// How many project details the pane remembers.
const DETAIL_CACHE: usize = 64;

/// The most width the base column may claim from the layout. `BASE_MAX` in the
/// table is what it may *draw* at; this is what it may take from the detail
/// pane before the pane is worth more than the label.
const BASE_CLAIM_MAX: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Projects,
    Detail,
}

/// Which tab the app is on.
///
/// **A tab, not a dialog**: a tab keeps its place, and the templates are a
/// place to work, with their counts, a filter box of their own and their
/// verbs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Library,
    Templates,
}

impl Screen {
    pub const ALL: [Screen; 2] = [Screen::Library, Screen::Templates];

    pub fn label(self) -> &'static str {
        match self {
            Screen::Library => "library",
            Screen::Templates => "templates",
        }
    }
}

/// Every template known: the ones on disk, and the slugs projects still name
/// that no template answers to. The counts come from the library.
#[derive(Debug, Default)]
pub struct TemplatesState {
    pub cards: Vec<TemplateCard>,
    pub counts: HashMap<String, usize>,
    /// How many of the cards are templates on disk.
    pub on_disk: usize,
}

impl TemplatesState {
    /// Cards from the summary — the templates on disk — then a bare card for
    /// any slug the projects still name that no template answers to, so the
    /// tab can list it and say what it is. The first card is always a real
    /// template, so the tab never opens on `(registered)`.
    pub fn rebuild(&mut self, summary: Option<&Summary>, counts: HashMap<String, usize>) {
        let mut cards: Vec<TemplateCard> = summary.map(|s| s.templates.clone()).unwrap_or_default();
        for slug in counts.keys() {
            if !cards.iter().any(|c| &c.slug == slug) {
                cards.push(TemplateCard {
                    slug: slug.clone(),
                    name: slug.clone(),
                    description: String::new(),
                    variables: 0,
                    folders: 0,
                    naming_pattern: String::new(),
                    on_disk: false,
                });
            }
        }
        // Real templates first, then by slug. **Alphabetical, not busiest
        // first**: a list you scan and search wants to be in the same order
        // tomorrow. Creating one project should not move a row.
        // By the name the list shows, not the raw slug: `(registered)` is
        // displayed as `registered` and sorting it under `(` puts it in front
        // of every `d`, which reads as no order at all.
        cards.sort_by(|a, b| {
            b.on_disk.cmp(&a.on_disk).then_with(|| {
                Self::display_name(a)
                    .cmp(Self::display_name(b))
                    .then_with(|| a.slug.cmp(&b.slug))
            })
        });
        self.on_disk = cards.iter().filter(|c| c.on_disk).count();
        self.cards = cards;
        self.counts = counts;
    }

    /// What a card is called on screen: `(registered)` is a slug the engine
    /// writes, not a name a person chose.
    pub fn display_name(card: &TemplateCard) -> &str {
        if card.slug == crate::core::operations::REGISTERED_SLUG {
            "registered"
        } else {
            &card.slug
        }
    }

    pub fn count(&self, slug: &str) -> usize {
        self.counts.get(slug).copied().unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StatusLevel {
    #[default]
    Info,
    Good,
    Warn,
    Error,
}

/// The one-line message under the table.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Status {
    pub text: String,
    pub level: StatusLevel,
    /// The tick it disappears at; `None` stays until replaced.
    pub expires_at: Option<u64>,
    /// When it was set, for the wash it arrives under; `None` for a status
    /// nobody set — `Status::default()`, which a test assigns — so a clock at
    /// zero does not read as a message arriving.
    pub shown_at: Option<u64>,
}

/// One line of the session's message log: what the status line said, and
/// when. The line itself expires; the log keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub at: String,
    pub level: StatusLevel,
    pub text: String,
}

/// How many status lines the log keeps.
pub const LOG_CAP: usize = 200;

pub struct App {
    /// `fastf` with no arguments, as opposed to `recent`/`search`.
    pub is_menu: bool,
    pub theme: Theme,
    pub size: (u16, u16),
    pub library: LibraryState,
    pub search: SearchState,
    pub summary: Option<Summary>,
    /// The read each part of the summary last came from (`SummaryPart::slot`).
    pub summary_seen: [u64; 3],
    /// When the last summary landed (`elapsed_ms`): the clock the look
    /// again while something waits for a base runs on.
    pub summary_at: Option<u64>,
    pub summary_error: Option<String>,
    pub details: HashMap<PathBuf, ProjectDetail>,
    pub detail_open: bool,
    pub detail_scroll: usize,
    /// The pane's own cursor: the index into `pane_rows` of the row Enter
    /// would edit. Drawn only while the pane has the focus, which is what
    /// makes the focus unmistakable even with no colour to say it.
    pub pane_cursor: usize,
    /// An edit open on a pane row, if one is. While it is, the field has the
    /// keys (`Context::PaneEdit`).
    pub pane_edit: Option<pane::PaneEdit>,
    /// The pane rows an edit just landed on, and when — the pane's own
    /// `pulses`, keyed by row rather than by path.
    pub pane_pulses: motion::Pulses<usize>,
    /// What the last landed edit was about, until the detail it invalidated
    /// has been read again and the cursor has found that row.
    pane_return: Option<pane::PaneTarget>,
    /// A pane write on its way with no edit open for it — a todo toggled, a
    /// note or a todo added — so the answer lands on the row it was about:
    /// the cursor settles there and it pulses, as an edit's does.
    pane_pending: Option<pane::PaneTarget>,
    /// What the `$EDITOR` note was opened about, kept while the editor has
    /// the terminal: the text it comes back with goes to these.
    editor_note_for: Option<actions::Targets>,
    /// What the pane's cursor is on, so a rebuild of the rows under it — a
    /// re-read, a re-wrap at a new width — finds the same thing again rather
    /// than the same index (`refind_pane`).
    pane_anchor: Option<pane::PaneTarget>,
    /// The project the pane's cursor, scroll and pulses belong to. They are
    /// reset only when the selection is another project, so a discovery or
    /// a metadata read landing does not throw the cursor back to the top.
    pane_for: Option<PathBuf>,
    /// The section `<` or `>` left the pane's cursor in, to land in again
    /// on the next project once its rows are there.
    pane_seek: Option<pane::PaneSection>,
    pub focus: Focus,
    pub screen: Screen,
    pub templates: TemplatesState,
    /// The templates tab's own list. It is state on the app, not a modal:
    /// a tab you can leave and come back to keeps its place, and a modal
    /// cannot be a tab.
    pub studio: Studio,
    pub modals: ModalStack,
    /// What the one running mutation is doing, for the status line.
    pub busy: Option<&'static str>,
    pub busy_id: Option<ActionId>,
    /// Jobs — moves, copies, deletes, reconciles in processes of their own —
    /// and which one the progress dialog follows.
    pub background: background::Background,
    /// A batch job over the marked projects, while one is running.
    pub job: Option<jobs::Job>,
    pub status: Status,
    /// Every status line this session set, oldest first, `LOG_CAP` at most —
    /// so a warning that flashed under a dialog can be read back with `L`.
    pub log: std::collections::VecDeque<LogEntry>,
    /// Warnings that arrived while a dialog covered the status line, not yet
    /// looked at: the status line and the hint bar say so until `L` is pressed.
    pub unseen_warnings: usize,
    /// Messages said since the runtime last looked, for it to keep on disk —
    /// `update` does no I/O, so a status line is handed over rather than
    /// written. Stamped with the time when they are written.
    pub outbox: Vec<crate::util::messages::Message>,
    /// The clock a log line is stamped with. The runtime's is the wall clock;
    /// a fixture's stands still, so a snapshot never depends on the hour.
    pub clock: fn() -> String,
    /// Where the data lives, for the help's footer. Set by the runtime, which
    /// may look; `None` in a fixture, so no snapshot can name a real path.
    pub data_dir: Option<String>,
    /// Whether a window could be opened from here — a desktop session. Set by
    /// the runtime; `true` in a fixture, so a frame never depends on the
    /// machine that rendered it.
    pub has_display: bool,
    /// The last few things this session did, oldest first.
    pub session: Vec<String>,
    /// `fastf template new` / `edit`: the studio or the builder to open as
    /// soon as the app starts, since that is what the command asked for.
    pub studio_entry: Option<crate::tui::entry::StudioEntry>,
    /// A row to select once the list has caught up with what was just made.
    /// A create or a register produces a project no snapshot holds yet, so the
    /// selection is asked for by path and applied when discovery answers.
    pub select_when_found: Option<PathBuf>,
    /// The row the last run left the cursor on, applied once discovery has
    /// answered for the first time — by id, since a rename between runs must
    /// not lose it.
    pub select_id_when_found: Option<String>,
    /// Whether the editor's explanation panel is open. **On by default**, and
    /// the opposite of every other pane here on purpose: somebody meeting the
    /// template editor has more to gain from the panel than from the width, and
    /// the person who does not want it turns it off once and is remembered.
    pub explain_open: bool,
    /// Whether the template guide has ever been shown, on this machine.
    ///
    /// It offers itself once, unasked — the first time the templates tab is
    /// opened or the editor is, whichever comes first. **One flag for both
    /// doors**: two would show it twice in one afternoon to the person who
    /// looked at the tab and then pressed new, which is exactly the reader it
    /// is trying not to annoy.
    pub guide_seen: bool,
    /// Milliseconds since the app opened, stamped by the runtime before every
    /// message.
    ///
    /// **A clock rather than a count**: a tick counter makes every duration a
    /// multiple of the wake interval, and the interval is not one number — a
    /// fade wants twenty frames a second and a spinner wants five. A
    /// fixture's clock is whatever the test sets, which is what makes a frame
    /// mid-pulse assertable.
    pub elapsed_ms: u64,
    /// Whether the app moves at all: the `motion` setting, resolved where the
    /// theme is so `update` still reads no environment.
    pub motion: motion::Motion,
    /// The rows a verb has just changed, and when.
    pub pulses: motion::Pulses,
    /// Rows whose size a verb just invalidated, waiting for the rescan.
    ///
    /// A size cell pulses because the number under your eyes *changed*, and
    /// there are only two ways that happens: a number replaced a different
    /// number, or a verb touched the folder and the old number was thrown away
    /// (`ListChange::Patched`'s `stale`), so the one that comes back reads as a
    /// first arrival with nothing to compare it to. This set is the second
    /// case, and it is emptied by the size that answers it.
    ///
    /// Everything else is a page filling in — the first screenful at startup,
    /// the next screenful after a scroll — and a page filling in is not a
    /// change. Pulsing there lights every visible row at once, twenty at a
    /// time, which is a flash rather than a cue and reads as a fault.
    rescanning: std::collections::BTreeSet<PathBuf>,
    /// When the focus last moved, for the pulse on the pane it moved to.
    /// `None` once that pulse has let go, so an idle app asks for no wake.
    pub focus_moved_at: Option<u64>,
    pub fuzzy: Fuzzy,
    next_action: u64,
    next_generation: u64,
}

impl App {
    pub fn new(entry: Entry, theme: Theme, size: (u16, u16)) -> Self {
        let mut app = Self {
            is_menu: entry.is_menu(),
            theme,
            size,
            library: LibraryState::new(),
            search: SearchState::default(),
            summary: None,
            summary_seen: [0; 3],
            summary_at: None,
            summary_error: None,
            details: HashMap::new(),
            detail_open: true,
            detail_scroll: 0,
            pane_cursor: 0,
            pane_edit: None,
            pane_pulses: motion::Pulses::default(),
            pane_return: None,
            pane_pending: None,
            editor_note_for: None,
            pane_anchor: None,
            pane_for: None,
            pane_seek: None,
            focus: Focus::Projects,
            screen: Screen::Library,
            templates: TemplatesState::default(),
            studio: Studio::default(),
            modals: ModalStack::default(),
            busy: None,
            busy_id: None,
            background: background::Background::default(),
            job: None,
            status: Status::default(),
            log: std::collections::VecDeque::new(),
            unseen_warnings: 0,
            outbox: Vec::new(),
            clock: crate::util::time::now_hms,
            data_dir: None,
            has_display: true,
            session: crate::tui::frame::recent_actions(),
            studio_entry: None,
            select_when_found: None,
            select_id_when_found: None,
            explain_open: true,
            guide_seen: false,
            elapsed_ms: 0,
            motion: motion::Motion::default(),
            pulses: motion::Pulses::default(),
            rescanning: std::collections::BTreeSet::new(),
            focus_moved_at: None,
            fuzzy: Fuzzy::new(),
            next_action: 0,
            next_generation: 0,
        };
        match entry {
            Entry::Menu => {}
            Entry::Recent { preset, initial } => {
                if !preset.is_empty() {
                    app.library.preset = Some(preset);
                }
                app.library.install_initial(initial);
            }
            Entry::Search { terms, initial } => {
                app.search = SearchState::with_text(&terms.join(" "));
                app.library.install_initial(initial);
            }
            Entry::Studio { open } => app.studio_entry = Some(open),
        }
        app.recompute();
        app
    }

    /// Start where the last run left off: the sort order, the pane, the row.
    /// `fastf recent`/`search` keep their own order and rows and take only
    /// the pane's state. Called before `start`, so the first frame is already
    /// the remembered one.
    pub fn apply_session(&mut self, session: &crate::tui::session::Session) {
        if let Some(open) = session.detail_open {
            self.detail_open = open;
        }
        if let Some(open) = session.explain_open {
            self.explain_open = open;
        }
        self.guide_seen = session.guide_seen.unwrap_or(false);
        if !self.is_menu {
            return;
        }
        if let Some(order) = session.sort_order() {
            self.library.explicit_sort = Some(order);
        }
        self.select_id_when_found = session.selected.clone();
        self.recompute();
    }

    /// The first effects: the header's summary, and a discovery unless the
    /// rows were handed in.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut effects = vec![Effect::LoadSummary];
        // `fastf template new`/`edit` opened the app for one screen; put it up
        // before the first frame so the command lands where it was aimed. All
        // three open **on the templates tab**, so Esc out of the builder leaves
        // you among the templates rather than in a library nobody asked for.
        if let Some(entry) = self.studio_entry.take() {
            effects.extend(self.toggle_templates());
            match entry {
                crate::tui::entry::StudioEntry::List => {}
                crate::tui::entry::StudioEntry::New => effects.extend(self.open_builder(None)),
                crate::tui::entry::StudioEntry::Edit(slug) => {
                    effects.extend(self.open_builder(Some(slug)))
                }
            }
        }
        if self.library.loaded {
            let template_reads = self.refresh_templates();
            effects.extend(template_reads);
            effects.extend(self.after_rows_changed());
        } else {
            effects.push(self.discover());
        }
        effects
    }
}

/// Whether an action's warning or error is more than the status line can
/// show: it has more than one line, or it is a paragraph. The status line
/// shows one line, so either would arrive as its first few words.
fn needs_a_dialog(text: &str) -> bool {
    text.contains('\n') || text.chars().count() > 160
}

/// The state machine: the app and one message in, the effects out.
pub fn update(app: &mut App, msg: Msg) -> Vec<Effect> {
    app.handle(msg)
}
