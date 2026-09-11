//! The model, and `update`: one function of the app and one message, with no
//! I/O of its own. Everything it wants done comes back as an `Effect`.
//!
//! That split is what makes the guided app testable without a terminal
//! (`tests/tui_update.rs` builds an `App`, feeds it messages and asserts on the
//! effects) and what keeps a slow filesystem out of the key handler: nothing in
//! here blocks, because nothing in here reads a disk.

pub mod actions;
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;

use crate::core::assets::Progress;
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
use data::{ProjectDetail, Summary, TemplateCard};
use library::{LibraryState, Order, Sort};
use modal::{MessageLevel, Modal, ModalStack, PickItem, PickState, Then};
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
/// **A tab, not a dialog.** Templates were an 84 %-wide modal over the library
/// plus a three-row strip along the bottom that filtered by Enter and nothing
/// else — two halves of one subject, neither of them a place you could work.
/// The strip is gone (three rows back to the table) and the studio is the
/// second tab, with the strip's counts, a filter box of its own, and the same
/// verbs it always had.
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
    /// Cards from the summary — the templates on disk, busiest first — then a
    /// bare card for any slug the projects still name that no template
    /// answers to, so the tab can list it and say what it is. The first card
    /// is always a real template, so the tab never opens on `(registered)`.
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
        // first**, which is what the horizontal strip used: a ribbon you read
        // left to right wants the popular ones near the start, a list you scan
        // and search wants to be in the same order tomorrow. Creating one
        // project should not move a row.
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
    /// The latest snapshot of the move job that is running, for the progress
    /// modal; `None` when no move is in flight.
    pub move_progress: Option<Progress>,
    /// A batch job over the marked projects, while one is running.
    pub job: Option<jobs::Job>,
    pub status: Status,
    /// Every status line this session set, oldest first, `LOG_CAP` at most —
    /// so a warning that flashed under a dialog can be read back with `L`.
    pub log: std::collections::VecDeque<LogEntry>,
    /// Warnings that arrived while a dialog covered the status line, not yet
    /// looked at: the status line and the hint bar say so until `L` is pressed.
    pub unseen_warnings: usize,
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
    /// Milliseconds since the app opened, carried by every `Msg::Tick`.
    ///
    /// **A clock rather than a count.** It was a tick counter, which made
    /// every duration a multiple of whatever the wake interval happened to be
    /// — and the interval is not one number any more, because a fade wants
    /// twenty frames a second and a spinner wants five. A fixture's clock is
    /// whatever the test sets, which is what makes a frame mid-pulse
    /// assertable.
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
    /// change. Pulsing there lit every visible row at once, twenty at a time,
    /// which is a flash rather than a cue and reads as a fault.
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
            summary_error: None,
            details: HashMap::new(),
            detail_open: true,
            detail_scroll: 0,
            pane_cursor: 0,
            pane_edit: None,
            pane_pulses: motion::Pulses::default(),
            pane_return: None,
            pane_pending: None,
            focus: Focus::Projects,
            screen: Screen::Library,
            templates: TemplatesState::default(),
            studio: Studio::default(),
            modals: ModalStack::default(),
            busy: None,
            busy_id: None,
            move_progress: None,
            job: None,
            status: Status::default(),
            log: std::collections::VecDeque::new(),
            unseen_warnings: 0,
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

    // --- geometry ---------------------------------------------------------

    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.size.0, self.size.1)
    }

    pub fn regions(&self) -> layout::Regions {
        layout::regions(self.area(), self.detail_open, self.table_min_width())
    }

    /// The width the table needs to show every folder name whole with the id
    /// and the size beside it: the cursor cell, the id, the name and the size
    /// cell, each followed by a space, inside the borders, plus the right
    /// gutter `view::projects::table` always reserves.
    pub fn table_min_width(&self) -> u16 {
        let (id_w, name_w) = self.library.widths;
        // The base column joins the claim once the rows come from more than one
        // base. It is elected right after the size there, and a table that did
        // not ask for its width never got it: a library of ninety-character
        // folder names left the split with room for the size and nothing else,
        // so the one column saying which drive a project is on never appeared
        // on the machine that had four of them.
        let base = if self.library.many_bases {
            self.library.base_width.min(BASE_CLAIM_MAX) + 1
        } else {
            0
        };
        // `choose_columns` adds a column only while it fits with its spacing.
        (2 + 1 + id_w + 1 + name_w + 1 + crate::tui::rows::SIZE_CELL + 1 + base + 1)
            .min(u16::MAX as usize) as u16
    }

    pub fn rows_on_screen(&self) -> usize {
        self.regions().table_rows()
    }

    pub fn detail_visible(&self) -> bool {
        self.regions().detail.is_some()
    }

    /// Where a key goes right now.
    pub fn context(&self) -> Context {
        if let Some(modal) = self.modals.top() {
            return modal.context();
        }
        if self.search.editing {
            return Context::SearchEdit;
        }
        if self.pane_edit.is_some() {
            return Context::PaneEdit;
        }
        self.focus_context()
    }

    fn focus_context(&self) -> Context {
        if self.screen == Screen::Templates {
            return Context::Templates;
        }
        match self.focus {
            Focus::Projects => Context::Projects,
            Focus::Detail => Context::Detail,
        }
    }

    /// How soon the runtime should wake the app with nothing to say, or
    /// `None` while nothing on screen is moving at all.
    ///
    /// Two speeds, because two kinds of thing move. A spinner turning and a
    /// toast counting down want five frames a second — five is enough to read
    /// as motion, and the wake is not free on a laptop. A pulse fading out
    /// wants twenty, because a fade drawn five times is a flicker. Asking for
    /// the faster one only while a pulse is in flight is what keeps the
    /// documented claim true: **the app costs nothing while idle.**
    pub fn tick_interval(&self) -> Option<std::time::Duration> {
        // Only where a frame could show it: off, or with no colour to show
        // it in, nothing moves and nothing asks to be redrawn for it.
        let visible = self.motion.is_on() && self.theme.kind != crate::tui::theme::ThemeKind::Mono;
        if visible
            && (!self.pulses.is_empty()
                || !self.pane_pulses.is_empty()
                || motion::focus_easing(self.focus_moved_at, self.elapsed_ms)
                || motion::arriving(self.status.shown_at, self.elapsed_ms))
        {
            return Some(std::time::Duration::from_millis(motion::FRAME_MS));
        }
        // A message about to go dims on its way out, which is a fade too — but
        // only for its last half second, so the slow wake is asked to cover it
        // rather than the fast one being held for six seconds.
        let slow = self.busy.is_some()
            || self.status.expires_at.is_some()
            || (self.library.loaded && self.library.sizes_pending(self.rows_on_screen()));
        slow.then(|| std::time::Duration::from_millis(SLOW_FRAME_MS))
    }

    /// Whether anything on screen is moving at all.
    pub fn needs_tick(&self) -> bool {
        self.tick_interval().is_some()
    }

    /// The size cell for `path`, as the browser drew it.
    pub fn size_cell(&self, path: &std::path::Path) -> SizeCell {
        match self.library.sizes.get(path) {
            Some(size) => SizeCell::Known(*size),
            None => SizeCell::Pending,
        }
    }

    // --- status -----------------------------------------------------------

    fn set_status(&mut self, level: StatusLevel, text: impl Into<String>) {
        let text = text.into();
        self.log.push_back(LogEntry {
            at: (self.clock)(),
            level,
            text: text.clone(),
        });
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
        // A warning under a full-height dialog is a warning nobody saw.
        if matches!(level, StatusLevel::Warn | StatusLevel::Error) && !self.modals.is_empty() {
            self.unseen_warnings += 1;
        }
        self.status = Status {
            text,
            level,
            expires_at: Some(self.elapsed_ms + STATUS_MS),
            shown_at: Some(self.elapsed_ms),
        };
    }

    /// `L`: the session's messages, newest first, as a scrollable dialog.
    fn open_log(&mut self) -> Vec<Effect> {
        self.unseen_warnings = 0;
        let g = self.theme.glyphs;
        let body = if self.log.is_empty() {
            "nothing yet".to_string()
        } else {
            self.log
                .iter()
                .rev()
                .map(|entry| {
                    let mark = match entry.level {
                        StatusLevel::Warn => format!("{} ", g.warn),
                        StatusLevel::Error => format!("{} ", g.cross),
                        StatusLevel::Good | StatusLevel::Info => String::new(),
                    };
                    format!("{}  {mark}{}", entry.at, entry.text)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.modals
            .push(Modal::message("messages", body, MessageLevel::Info));
        Vec::new()
    }

    fn info(&mut self, text: impl Into<String>) {
        self.set_status(StatusLevel::Info, text);
    }

    /// A success, with the theme's own tick in front of it.
    ///
    /// **The glyph belongs here and not in the message.** Twelve of
    /// `runtime::run_action`'s strings carried a literal `✓`, which
    /// `Glyphs::ascii` maps to `+` — so on a legacy Windows console, or under
    /// `FASTF_ASCII=1`, they drew a replacement box beside the app's own
    /// correctly-themed messages. `run_action` runs on a worker with no theme
    /// to ask, and this is the one place every one of its messages passes
    /// through.
    fn good(&mut self, text: impl Into<String>) {
        let text = format!("{}  {}", self.theme.glyphs.check, text.into());
        self.set_status(StatusLevel::Good, text);
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.set_status(StatusLevel::Warn, text);
    }

    fn error(&mut self, text: impl Into<String>) {
        self.set_status(StatusLevel::Error, text);
    }

    // --- the library ------------------------------------------------------

    fn discover(&mut self) -> Effect {
        self.next_generation += 1;
        self.library.inflight = Some(self.next_generation);
        Effect::Discover {
            generation: self.next_generation,
        }
    }

    fn recompute(&mut self) {
        self.library.recompute(&self.search.query, &mut self.fuzzy);
    }

    /// After anything that changed which rows are shown.
    fn after_rows_changed(&mut self) -> Vec<Effect> {
        // Long names can close the pane (`layout::regions`); the focus cannot
        // stay on a pane that is not drawn.
        if self.focus == Focus::Detail && !self.pane_present() {
            self.set_focus(Focus::Projects);
        }
        let rows = self.rows_on_screen();
        self.library.clamp_viewport(rows);
        self.detail_scroll = 0;
        self.pane_cursor = 0;

        let mut effects = self.selection_effects();
        if self.search.query.needs_metadata() {
            let missing = self.library.paths_without_meta();
            if !missing.is_empty() {
                effects.push(Effect::LoadMeta(missing));
            }
        }
        effects
    }

    /// After the selection moved.
    fn after_selection_change(&mut self) -> Vec<Effect> {
        let rows = self.rows_on_screen();
        self.library.clamp_viewport(rows);
        self.detail_scroll = 0;
        self.pane_cursor = 0;
        self.pane_edit = None;
        self.pane_return = None;
        self.pane_pending = None;
        self.selection_effects()
    }

    /// Measure what is on screen, selected row first, and read the selected
    /// project's detail if the pane will show it.
    fn selection_effects(&self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let wanted = self.library.visible_paths(self.rows_on_screen());
        if !wanted.is_empty() {
            effects.push(Effect::RequestSizes(wanted));
        }
        if self.detail_visible()
            && let Some(project) = self.library.selected()
        {
            effects.push(self.detail_effect(&project.path));
        }
        effects
    }

    /// Read the project's detail, or — when one is cached — check it is
    /// still what is on disk: a stat, and a read only if the file changed.
    /// The pane used to trust its cache until a verb inside the app dropped
    /// it, so a line added to `PROJECT_INFO.md` in an editor never showed.
    fn detail_effect(&self, path: &Path) -> Effect {
        match self.details.get(path) {
            Some(detail) => Effect::RefreshDetail {
                path: path.to_path_buf(),
                stamp: detail.stamp,
            },
            None => Effect::LoadDetail(path.to_path_buf()),
        }
    }

    fn after_query_change(&mut self) -> Vec<Effect> {
        // On the templates tab the bar filters the template list, which is a
        // plain substring over a handful of slugs — no grammar, no metadata
        // reads, and the cursor kept on a row the query still keeps.
        if self.screen == Screen::Templates {
            let rows = self.studio.rows(self.search.input.text());
            self.studio.reselect(&rows);
            self.studio
                .clamp_viewport(&rows, layout::template_rows(self.area()));
            return self
                .studio
                .selected_slug()
                .filter(|slug| self.studio.shown.as_deref() != Some(slug.as_str()))
                .map(|slug| vec![Effect::LoadTemplateView { slug }])
                .unwrap_or_default();
        }
        if !self.search.sync() {
            return Vec::new();
        }
        // A query the grammar cannot mean anything by is said so while it is
        // being typed, not answered with an empty list.
        if let Some(problem) = self.search.query.diagnose() {
            self.warn(problem);
        } else if self.status.level == StatusLevel::Warn && self.status.expires_at.is_some() {
            self.status = Status::default();
        }
        self.recompute();
        self.after_rows_changed()
    }

    /// Rebuild what is known about the templates and hand the tab its list.
    ///
    /// One call, because the two used to drift: the strip was rebuilt from the
    /// summary *and* the library's per-template counts, while the studio took
    /// the summary alone — so a slug projects still named that no template
    /// answered to was in one list and not the other.
    fn refresh_templates(&mut self) -> Vec<Effect> {
        self.templates
            .rebuild(self.summary.as_ref(), self.library.per_template());
        let cards = self.templates.cards.clone();
        self.studio.install(cards)
    }

    fn set_template_filter(&mut self, slug: Option<String>) -> Vec<Effect> {
        self.library.template_filter = slug;
        self.reordered()
    }

    fn set_base_filter(&mut self, base: Option<PathBuf>) -> Vec<Effect> {
        self.library.base_filter = base;
        self.reordered()
    }

    /// Both row filters off. `F` is one key because they are one question —
    /// "why am I not seeing everything" — and answering half of it leaves the
    /// list still short with no hint which half is left.
    fn clear_filters(&mut self) -> Vec<Effect> {
        self.library.template_filter = None;
        self.library.base_filter = None;
        self.reordered()
    }

    /// After a sort or a filter changed the order of the rows. **Find my
    /// row:** the selection is kept by path, so the row you were on is
    /// somewhere else on the screen now, and it pulses so the eye finds
    /// where it went. Not in `after_rows_changed`, which discovery, the size
    /// reports and every search keystroke run through: those change what is
    /// on screen without anyone having asked for a reorder.
    fn reordered(&mut self) -> Vec<Effect> {
        self.recompute();
        self.pulse_selected();
        self.after_rows_changed()
    }

    fn pulse_selected(&mut self) {
        if let Some(project) = self.library.selected() {
            self.pulses.start(project.path.clone(), self.elapsed_ms);
        }
    }

    /// What a finished verb does to the list, without the worker that finished
    /// it. Public because it is the one step a test about the *list* wants to
    /// drive — and **not** behind `cfg(debug_assertions)`: the suites are
    /// separate crates, so a seam hidden that way is missing from
    /// `cargo test --release`, which is a gate.
    pub fn apply_change(&mut self, change: ListChange) -> Vec<Effect> {
        let mut effects = Vec::new();
        match change {
            ListChange::Patched {
                project,
                was,
                stale,
            } => {
                // **What changed, said on the row it changed.** A batch tags
                // ten rows and the cursor is on one of them; without this the
                // frame after is identical to the frame before except for ten
                // cells nobody was looking at.
                self.pulses.start(project.path.clone(), self.elapsed_ms);
                if !self.library.patch(&was, *project) {
                    effects.push(self.discover());
                }
                for path in &stale {
                    self.library.sizes.remove(path);
                    self.details.remove(path);
                }
                self.rescanning.extend(stale.iter().cloned());
                effects.push(Effect::ForgetSizes(stale));
            }
            ListChange::Removed { path } => {
                self.library.remove(&path);
                self.details.remove(&path);
                self.rescanning.remove(&path);
                effects.push(Effect::ForgetSizes(vec![path]));
            }
            // Nothing on the row changed, so nothing on the row lights up
            // and the cursor stays where it is. The pane keeps what it shows
            // until the re-read lands — dropping the detail first would put a
            // `reading…` frame between the keypress and the answer — and the
            // pane's own pulse says what landed.
            ListChange::DetailOnly { path } => {
                if self.detail_visible() && self.library.selected().is_some_and(|p| p.path == path)
                {
                    effects.push(Effect::LoadDetail(path));
                } else {
                    self.details.remove(&path);
                }
                return effects;
            }
            ListChange::Reload => {
                effects.push(self.discover());
                effects.push(Effect::LoadSummary);
            }
            ListChange::SummaryOnly => effects.push(Effect::LoadSummary),
            ListChange::None => {}
        }
        self.recompute();
        effects.extend(self.refresh_templates());
        effects.extend(self.after_rows_changed());
        effects
    }

    // --- messages ---------------------------------------------------------

    fn handle(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Key(key) => self.on_key(key),
            Msg::Paste(text) => self.on_paste(&text),
            Msg::Resize(width, height) => {
                self.size = (width, height);
                self.after_selection_change()
            }
            Msg::Tick => {
                if self
                    .status
                    .expires_at
                    .is_some_and(|at| at <= self.elapsed_ms)
                {
                    self.status = Status::default();
                }
                self.pulses.retire(self.elapsed_ms);
                self.pane_pulses.retire(self.elapsed_ms);
                if !motion::focus_easing(self.focus_moved_at, self.elapsed_ms) {
                    self.focus_moved_at = None;
                }
                Vec::new()
            }
            Msg::Sizes(cells) => {
                for (path, size) in cells {
                    // **A number that changed, not a number that arrived.** The
                    // table is measured from the rows, never from the sizes, so
                    // a landing number cannot reflow anything — the only
                    // question a pulse answers here is whether the figure you
                    // are looking at is the one that was there a moment ago.
                    // The first fill of a row cannot be that, and every visible
                    // row fills at once on the first screenful and on every
                    // scroll: `rescanning` is the one arrival that *is* a
                    // change, a size a verb threw away coming back.
                    let previous = self.library.sizes.insert(path.clone(), size);
                    let rescanned = self.rescanning.remove(&path);
                    if rescanned || previous.is_some_and(|had| had != size) {
                        self.pulses.start(path, self.elapsed_ms);
                    }
                }
                if self.library.effective_sort(&self.search.query).order == Order::Size {
                    self.recompute();
                    let rows = self.rows_on_screen();
                    self.library.clamp_viewport(rows);
                }
                Vec::new()
            }
            Msg::Summary(summary) => {
                self.summary = Some(*summary);
                self.summary_error = None;
                // A template written or deleted is a change to the templates
                // tab's list, whether or not that tab is the one on screen.
                self.refresh_templates()
            }
            Msg::SummaryFailed(error) => {
                self.summary_error = Some(error.clone());
                self.error(format!("the library summary could not be read: {error}"));
                Vec::new()
            }
            Msg::Discovered {
                generation,
                projects,
            } => {
                if !self.library.install(generation, projects) {
                    return Vec::new();
                }
                self.recompute();
                let mut effects = self.refresh_templates();
                // A create or a register asked for its new project to be
                // selected; it exists only once discovery has seen it.
                if let Some(path) = self.select_when_found.clone()
                    && self.library.select_path(&path)
                {
                    self.select_when_found = None;
                }
                // The remembered row is applied once, on the first answer: a
                // later discovery is a reload, and the cursor is wherever the
                // user has since put it.
                if let Some(id) = self.select_id_when_found.take() {
                    self.library.select_id(&id);
                }
                effects.extend(self.after_rows_changed());

                if self.library.dirty {
                    self.library.dirty = false;
                    effects.push(self.discover());
                }
                effects
            }
            Msg::DiscoverFailed { generation, error } => {
                if self.library.inflight == Some(generation) {
                    self.library.inflight = None;
                    self.library.loaded = true;
                    self.library.error = Some(error.clone());
                    self.modals.push(Modal::message(
                        "the library could not be read",
                        format!(
                            "{error}\n\nfix the configuration (`fastf config show`), then reload with {}.",
                            command::key_of(CommandId::Reload)
                        ),
                        MessageLevel::Error,
                    ));
                }
                Vec::new()
            }
            Msg::Detail { path, detail } => {
                if self.details.len() >= DETAIL_CACHE {
                    self.details.clear();
                }
                let mut effects = Vec::new();
                // The file is the truth, and the pane just read it: a row
                // whose tags or names disagree with the metadata — edited
                // outside the app, or an index that went stale — takes the
                // file's, and the base's index entry is written to match.
                if let Some(meta) = &detail.meta
                    && let Some(mut row) = self.project_at(&path)
                {
                    let mut changed = false;
                    if row.tags != meta.tags {
                        row.tags = meta.tags.clone();
                        changed = true;
                    }
                    if !meta.template_name.is_empty() && row.template_name != meta.template_name {
                        row.template_name = meta.template_name.clone();
                        changed = true;
                    }
                    if !meta.created.is_empty() && row.created != meta.created {
                        row.created = meta.created.clone();
                        changed = true;
                    }
                    if changed && self.library.patch(&path, row) {
                        self.recompute();
                        let rows = self.rows_on_screen();
                        self.library.clamp_viewport(rows);
                        effects.push(Effect::RefreshCache(path.clone()));
                    }
                }
                let selected = self.library.selected().is_some_and(|p| p.path == path);
                self.details.insert(path, *detail);
                if selected {
                    if let Some(target) = self.pane_return.take() {
                        self.settle_pane_cursor(&target);
                    }
                    // An edit open on a row keeps its row: the rows were
                    // rebuilt under it, and its note may have moved.
                    if let Some(edit) = &self.pane_edit
                        && let Some(row) = pane::find_row(&self.pane_rows(), &edit.target())
                        && let Some(edit) = &mut self.pane_edit
                    {
                        edit.set_row(row);
                    }
                    let rows = self.pane_rows();
                    self.pane_cursor = pane::step_cursor(&rows, self.pane_cursor, 0);
                    self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
                        self.detail_scroll,
                        Some(self.pane_cursor),
                        rows.len(),
                        self.pane_rows_on_screen(),
                    );
                }
                effects
            }
            Msg::MetaLoaded(loaded) => {
                self.library.absorb_meta(loaded);
                self.recompute();
                self.after_rows_changed()
            }
            Msg::MoveProgress(progress) => {
                if self.move_progress.is_some() {
                    self.move_progress = Some(progress);
                }
                Vec::new()
            }
            Msg::TemplateLoaded { slug, result } => self.on_template_loaded(&slug, result),
            Msg::TemplateSourceLoaded { slug, result } => {
                let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
                    return Vec::new();
                };
                // A read for a template this builder is no longer waiting on is
                // an answer to a question nobody is asking any more — the same
                // guard `on_template_loaded` and `TemplateViewLoaded` make. Esc
                // out of one pending builder and open another, and on a slow
                // disk the first read used to arrive and become the second's
                // contents, wiping anything typed meanwhile.
                if builder.pending.as_deref() != Some(slug.as_str()) {
                    return Vec::new();
                }
                builder.pending = None;
                match result {
                    Ok(template) => **builder = Builder::new(Some(*template)),
                    Err(error) => {
                        self.modals.pop();
                        self.error(format!("template '{slug}' could not be read: {error}"));
                    }
                }
                Vec::new()
            }
            Msg::TemplateViewLoaded { slug, lines } => {
                if self.studio.selected_slug().as_deref() == Some(slug.as_str()) {
                    self.studio.shown = Some(slug);
                    self.studio.lines = lines;
                    self.studio.scroll = 0;
                }
                Vec::new()
            }
            Msg::SettingsLoaded(loaded) => {
                // The screen went up when `,` was pressed, saying it was
                // reading; a read that lands after it was closed has nothing
                // to fill in and is dropped.
                let (theme, motion) = (loaded.theme.clone(), loaded.motion.clone());
                if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                    state.refresh(*loaded);
                }
                // A theme — or a motion setting — written on this screen takes
                // effect on the frame that shows it was written.
                vec![Effect::Retheme { theme, motion }]
            }
            Msg::Themed { theme, motion } => {
                self.theme = *theme;
                self.motion = motion;
                Vec::new()
            }

            Msg::SettingsFailed(error) => {
                if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                    state.pending = false;
                }
                self.error(format!("the settings could not be read: {error}"));
                Vec::new()
            }
            Msg::Previewed(preview) => self.on_previewed(*preview),
            Msg::PreviewFailed { field, error } => {
                let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
                    return Vec::new();
                };
                flow.pending = false;
                flow.step = Step::Form;
                flow.form.fail(field.as_deref(), error);
                Vec::new()
            }
            Msg::ViewLoaded { title, lines } => {
                // The dialog went up when the key was pressed, saying it was
                // reading; fill it in if it is still the one on top, else
                // the user has moved on and the read is dropped.
                if let Some(Modal::Message {
                    title: shown,
                    lines: body,
                    ..
                }) = self.modals.top_mut()
                    && *shown == title
                {
                    *body = lines;
                }
                Vec::new()
            }
            Msg::ActionDone { id, outcome } => self.on_action_done(id, outcome),
            Msg::Spawned { what, outcome } => self.on_spawned(what, outcome),
            Msg::Resumed(Resumed::PostCreate) => {
                self.session = crate::tui::frame::recent_actions();
                Vec::new()
            }
            Msg::Resumed(Resumed::Shell) => Vec::new(),

            Msg::Resumed(Resumed::Note { project, text }) => {
                self.session = crate::tui::frame::recent_actions();
                match text {
                    Some(text) if !text.trim().is_empty() => {
                        if self.batching() {
                            // The editor ran once; the note goes to every mark.
                            self.start_job(jobs::JobKind::Note(text), None)
                        } else {
                            self.run_action("adding a note…", Action::AppendNote { project, text })
                        }
                    }
                    _ => {
                        self.info("no note written");
                        Vec::new()
                    }
                }
            }

            Msg::Diag(level, text) => {
                match level {
                    Level::Warn => self.warn(format!("warning: {text}")),
                    Level::Note => self.info(format!("note: {text}")),
                }
                Vec::new()
            }
            Msg::Interrupted => vec![Effect::Quit(Exit::Interrupted)],
        }
    }

    fn on_action_done(
        &mut self,
        id: ActionId,
        outcome: Result<Box<ActionOutcome>, String>,
    ) -> Vec<Effect> {
        if self.busy_id != Some(id) {
            return Vec::new();
        }
        self.busy = None;
        self.busy_id = None;
        self.move_progress = None;
        if self.job.is_some() {
            return self.on_job_item_done(outcome);
        }
        match outcome {
            Ok(outcome) => {
                let outcome = *outcome;
                if let Some(entry) = outcome.session {
                    crate::tui::frame::record(entry);
                    self.session = crate::tui::frame::recent_actions();
                }
                match outcome.warning {
                    Some(warning) => {
                        self.warn(format!("{}  —  warning: {warning}", outcome.message))
                    }
                    None => self.good(outcome.message),
                }
                if let Some(path) = outcome.select {
                    self.select_when_found = Some(path);
                }
                let reload_settings = outcome.reload_settings;
                // The first-run question is answered once the folder exists.
                if matches!(self.modals.top(), Some(Modal::Onboarding(_))) {
                    self.modals.pop();
                }
                // A template that saved is on disk; the builder holding it has
                // nothing left to hold. Only now, and only here — see
                // `save_template`.
                if matches!(self.modals.top(), Some(Modal::Builder(builder)) if builder.saving) {
                    self.modals.pop();
                }
                // A pane edit that landed: the edit closes, the row it was on
                // pulses, and the cursor stays where the edit was made —
                // `apply_change` would otherwise put it back on the name,
                // which is the one row nobody who just changed a variable is
                // looking at.
                let landed = self
                    .pane_edit
                    .take_if(|edit| edit.pending())
                    .map(|edit| edit.target())
                    .or_else(|| self.pane_pending.take());
                let mut effects = self.apply_change(outcome.change);
                if let Some(target) = landed {
                    self.settle_pane_cursor(&target);
                    // The detail was just dropped and will be read again;
                    // the cursor finds the row once more when it lands.
                    self.pane_return = Some(target);
                }
                if let Some(FollowUp::PostCreate {
                    root,
                    template_slug,
                }) = outcome.follow_up
                {
                    effects.push(Effect::Suspend(Suspended::PostCreate {
                        root,
                        template_slug,
                    }));
                }
                if reload_settings && matches!(self.modals.top(), Some(Modal::Settings(_))) {
                    if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                        state.editing = None;
                        state.pending = true;
                    }
                    effects.push(Effect::LoadSettings);
                }
                effects
            }
            Err(error) => {
                // A refusal belongs on the field that earned it, wherever one
                // is open: `config set`'s own message, under the value that is
                // still there to be corrected.
                if let Some(edit) = &mut self.pane_edit
                    && edit.pending()
                {
                    edit.fail(error);
                    return Vec::new();
                }
                match self.modals.top_mut() {
                    Some(Modal::Settings(state)) if state.editing.is_some() => {
                        state.pending = false;
                        state.fail(error);
                    }
                    Some(Modal::Onboarding(state)) => {
                        state.pending = false;
                        state.error = Some(error);
                    }
                    // The refusal goes under the list the template is still
                    // sitting in, in the same place `Cannot save:` already
                    // appears, with every answer untouched.
                    Some(Modal::Builder(builder)) if builder.saving => {
                        builder.saving = false;
                        builder.error = Some(format!("Cannot save: {error}"));
                    }
                    _ => self.error(format!("error: {error}")),
                }
                Vec::new()
            }
        }
    }

    fn on_spawned(&mut self, what: SpawnKind, outcome: Result<String, String>) -> Vec<Effect> {
        match (what, outcome) {
            (SpawnKind::Reveal(project), Ok(_)) => {
                self.good(format!("Opened {} in the file manager", project.name));
            }
            (SpawnKind::Reveal(_), Err(error)) => {
                self.error(format!("could not open the folder: {error}"))
            }
            (SpawnKind::Terminal(_), Ok(_)) => {
                self.good("Terminal opened");
            }
            (SpawnKind::Terminal(_), Err(error)) => {
                self.error(format!("could not open a terminal: {error}"));
            }
            (SpawnKind::Clipboard(_), Ok(tool)) => {
                self.good(format!("Copied with {tool}"));
            }
            (SpawnKind::Clipboard(text), Err(_)) => {
                self.modals.push(Modal::message(
                    "no clipboard tool found — here is the path:",
                    format!("{text}\n\ninstall wl-copy, xclip or xsel to copy from here."),
                    MessageLevel::Warn,
                ));
            }
        }
        Vec::new()
    }

    /// Pasted text goes into whichever field has the caret, and nowhere
    /// else. A single-line field takes the first line and says how many it
    /// dropped; a text area takes them all; with no field open the paste is
    /// ignored and said so — it is never read as keystrokes, which is how a
    /// pasted paragraph once ran a dozen commands.
    fn on_paste(&mut self, text: &str) -> Vec<Effect> {
        let lines = text.lines().count();
        let first = text.lines().next().unwrap_or_default().to_string();
        let dropped = lines.saturating_sub(1);
        let mut kept_first = false;
        let effects = match self.modals.top_mut() {
            Some(Modal::Palette(palette)) => {
                palette.input.paste(&first);
                kept_first = true;
                self.refresh_palette();
                Vec::new()
            }
            Some(Modal::Pick(pick)) => {
                pick.query.paste(&first);
                kept_first = true;
                pick.rank(&mut self.fuzzy);
                Vec::new()
            }
            Some(Modal::TextPrompt(prompt)) => {
                prompt.input.paste(&first);
                prompt.error = None;
                kept_first = true;
                Vec::new()
            }
            Some(Modal::Note(note)) => {
                note.area.paste(text);
                Vec::new()
            }
            Some(Modal::Flow(flow)) => {
                if let Some(field) = flow.form.focused_mut() {
                    field.paste(&first);
                    kept_first = true;
                }
                Vec::new()
            }
            Some(Modal::Builder(builder)) => {
                match &mut builder.open {
                    Some(Open::Metadata(form)) | Some(Open::Id(form)) => {
                        if let Some(field) = form.focused_mut() {
                            field.paste(&first);
                            kept_first = true;
                        }
                    }
                    Some(Open::Variables(list)) => {
                        if let Some((_, form)) = &mut list.editing
                            && let Some(field) = form.focused_mut()
                        {
                            field.paste(&first);
                            kept_first = true;
                        }
                    }
                    Some(Open::Structure(area)) => area.paste(text),
                    Some(Open::Files(list)) => {
                        if let Some(edit) = &mut list.editing {
                            if edit.in_body {
                                edit.body.paste(text);
                            } else {
                                edit.path.paste(&first);
                                kept_first = true;
                            }
                        }
                    }
                    None => {}
                }
                Vec::new()
            }
            Some(Modal::Settings(state)) => {
                match &mut state.editing {
                    Some(Editing::Value { input, error, .. }) => {
                        input.paste(&first);
                        *error = None;
                        kept_first = true;
                    }
                    Some(Editing::Bases { area, .. }) => area.paste(text),
                    Some(Editing::Filter) => {
                        state.filter.paste(&first);
                        state.apply_filter();
                        kept_first = true;
                    }
                    None => {}
                }
                Vec::new()
            }
            Some(Modal::Onboarding(state)) => {
                state.input.paste(&first);
                kept_first = true;
                Vec::new()
            }
            Some(_) => {
                self.info("pasted text ignored — nothing here takes typing");
                Vec::new()
            }
            None if self.search.editing => {
                self.search.input.paste(&first);
                kept_first = true;
                self.after_query_change()
            }
            None => {
                self.info(format!(
                    "pasted text ignored — press {} to search, or open a field first",
                    command::key_of(CommandId::Search)
                ));
                Vec::new()
            }
        };
        if kept_first && dropped > 0 {
            self.warn(format!(
                "pasted {lines} lines — kept the first, this field takes one"
            ));
        }
        effects
    }

    // --- keys -------------------------------------------------------------

    fn on_key(&mut self, key: Key) -> Vec<Effect> {
        if layout::too_small(self.area()) {
            // The guard takes only the two quit gestures — and a job that is
            // running still turns them into a cancel, exactly as it does on
            // a screen big enough to show it.
            if key != Key::ch('q') && key != Key::ctrl('c') {
                return Vec::new();
            }
            return self.quit(if key == Key::ctrl('c') {
                Exit::Interrupted
            } else {
                Exit::Normal
            });
        }
        if key == Key::ctrl('c') {
            // Declared in the registry so it is in the help, but dispatched
            // here rather than through `lookup`: the interrupt key answers
            // from inside a text field and from under a modal that consumes
            // every key, and no availability state may swallow it.
            return self.run(CommandId::Interrupt);
        }
        // A move that is running turns the other quit gestures — `q`, and Esc
        // once it has closed whatever was open — into cancels too (`run`); see
        // the Ctrl-C case above.
        if !self.modals.is_empty() {
            return self.on_modal_key(key);
        }
        if self.search.editing {
            return self.on_search_key(key);
        }
        if self.pane_edit.is_some() {
            return self.on_pane_edit_key(key);
        }
        match command::lookup(self.context(), key, self) {
            Some(id) => self.run(id),
            None => Vec::new(),
        }
    }

    /// **The field has first refusal; the registry answers the rest.** Every
    /// printable key is a letter of the query and every caret chord is the
    /// field's — `Ctrl-u` here is kill-to-start, whatever it means on a list —
    /// and what is left is exactly what `Context::SearchEdit` declares. That
    /// is why the arrows can be `CommandId::Down` and `Up` themselves, moving
    /// the library under the query, rather than a second pair written out
    /// here; and it is what lets `Ctrl-p` open the palette from the bar while
    /// `c` types a `c`.
    ///
    /// `command::keys_in` is the other half of the same rule: it takes what
    /// the field claims back out of what this context advertises, so the help
    /// never names a key the bar will swallow.
    fn on_search_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_some() || command::field_claims(&key) {
            return if self.search.input.apply(&key) {
                self.after_query_change()
            } else {
                Vec::new()
            };
        }
        match command::lookup(self.context(), key, self) {
            Some(id) => self.run(id),
            None => {
                if self.search.input.apply(&key) {
                    self.after_query_change()
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn on_modal_key(&mut self, key: Key) -> Vec<Effect> {
        match self.modals.top() {
            Some(Modal::Palette(_)) => self.on_palette_key(key),
            Some(Modal::Pick(_)) => self.on_pick_key(key),
            Some(Modal::Actions(_)) => self.on_actions_key(key),
            Some(Modal::TextPrompt(_)) => self.on_text_prompt_key(key),
            Some(Modal::Note(_)) => self.on_note_key(key),
            Some(Modal::Confirm(_)) => self.on_confirm_key(key),
            Some(Modal::MultiPick(_)) => self.on_multi_pick_key(key),
            Some(Modal::Flow(_)) => self.on_flow_key(key),
            Some(Modal::Builder(_)) => self.on_builder_key(key),
            Some(Modal::Settings(_)) => self.on_settings_key(key),
            Some(Modal::Onboarding(_)) => self.on_onboarding_key(key),
            Some(Modal::Guide(_)) => self.on_guide_key(key),
            Some(Modal::Help { .. }) | Some(Modal::Message { .. }) => self.on_scroll_modal_key(key),
            None => Vec::new(),
        }
    }

    /// The key a dialog did not take itself: whatever the registry binds in
    /// the dialog's context, or nothing. This is how every list on a dialog
    /// answers the same keys the help overlay lists for it.
    fn lookup_and_run(&mut self, key: Key) -> Vec<Effect> {
        match command::lookup(self.context(), key, self) {
            Some(id) => self.run(id),
            None => Vec::new(),
        }
    }

    /// The project at `path` in the list as it stands now.
    ///
    /// A dialog carries the path it was opened for, and this is how it gets
    /// back to a project at submit time — by the one thing that is unique
    /// whatever else has happened, which is what `ListChange::Patched` also
    /// keys on.
    fn project_at(&self, path: &Path) -> Option<Project> {
        self.library
            .snapshot
            .iter()
            .find(|project| project.path == path)
            .cloned()
    }

    /// The named project is not in the library any more, so the verb does not
    /// run — on a neighbour least of all.
    fn gone_from_the_library(&mut self) -> Vec<Effect> {
        self.warn("That project is no longer in the library — nothing was done.");
        Vec::new()
    }

    /// Whether a verb acts on the marks rather than the selection.
    ///
    /// **Asked of `targets()`, not of the mark set.** Marks are kept by path
    /// and survive a filter change; `targets()` intersects them with the rows
    /// on screen. When those two disagreed, `batching()` said yes and
    /// `targets()` came back empty, and every batch verb hit its
    /// `if targets.is_empty() { return Vec::new(); }` — no picker, no dialog,
    /// no message. Marking three rows and then typing a query made `A` do
    /// nothing at all, which is what "batch tagging doesn't work" was.
    fn batching(&self) -> bool {
        !self.library.targets().is_empty() && !self.library.marks.is_empty()
    }

    /// The folder names a confirmation is about: the marks, or the selection.
    fn target_names(&self) -> Vec<String> {
        self.library
            .targets()
            .iter()
            .map(|project| project.name.clone())
            .collect()
    }

    /// A screenful, for the pagers: the height of the list on screen.
    fn page_rows(&self) -> usize {
        self.rows_on_screen().max(1)
    }

    // --- commands ---------------------------------------------------------

    /// Run a command as its key or its palette entry would.
    pub fn run(&mut self, id: CommandId) -> Vec<Effect> {
        let command = command::find(id);
        match (command.available)(self) {
            Availability::Enabled => {}
            Availability::Disabled(reason) => {
                self.warn(format!("{}: {reason}", command.title));
                return Vec::new();
            }
            Availability::Hidden => return Vec::new(),
        }
        match id {
            CommandId::Quit => self.quit(Exit::Normal),
            CommandId::Back => {
                // Anything running is cancelled first: a batch job and a
                // single move alike, before a keystroke can clear something
                // the user was looking at.
                if self.job.is_some() || self.move_progress.is_some() {
                    return self.request_cancel();
                }
                // On the templates tab, the first step back is to the library:
                // Esc is "one level out", and a tab is a level.
                if self.screen == Screen::Templates {
                    if !self.search.input.is_empty() {
                        self.search.input.clear();
                        return Vec::new();
                    }
                    return self.toggle_templates();
                }
                if !self.search.input.is_empty() {
                    self.search.input.clear();
                    return self.after_query_change();
                }
                if self.library.template_filter.is_some() {
                    return self.set_template_filter(None);
                }
                if !self.library.marks.is_empty() {
                    // Marks can hide nothing, but clearing them first matches
                    // the Esc ladder: one keystroke at a time, nothing lost.
                    self.library.marks.clear();
                    self.info("marks cleared");
                    return Vec::new();
                }
                if self.library.preset.is_some() {
                    // `fastf recent --tag draft` opened the app already
                    // narrowed; the chip is a filter like any other, and Esc
                    // takes it off rather than leaving the app inside it.
                    self.library.preset = None;
                    self.recompute();
                    self.info("showing every project");
                    return self.after_rows_changed();
                }
                vec![Effect::Quit(Exit::Normal)]
            }
            CommandId::Help => {
                // The help for where the keys go right now — a dialog's own
                // context when one is open, else the focused pane's.
                let ctx = self.context();
                self.modals.push(Modal::Help { ctx, scroll: 0 });
                Vec::new()
            }
            CommandId::Close => self.close_top(),
            CommandId::Interrupt => {
                // A job or a move is running: Ctrl-C cancels it rather than
                // quitting under a worker that is still mutating the
                // filesystem.
                if self.job.is_some() || self.move_progress.is_some() {
                    return self.request_cancel();
                }
                // Here Ctrl-C is a close, and a close may not throw away a
                // template that has been worked on — the one gesture that
                // reached past the question Esc and `q` now ask, and the
                // quietest, since it did not even leave a status line behind.
                //
                // **A save in flight is deliberately not part of this.** Esc
                // and `q` are ignored while one runs, because it is about to
                // land and its refusal needs the list to land on. Ctrl-C is
                // the opposite case: `DataLock::acquire` waits up to thirty
                // seconds when another fastf holds it, so routing Ctrl-C into
                // the same guard left the only way out of a half-minute wait
                // doing nothing at all. It keeps its ordinary meaning instead
                // — the write is a single atomic publish on a worker, and
                // interrupting the app over it is exactly what the interrupt
                // key is for.
                if matches!(
                    self.modals.top(),
                    Some(Modal::Builder(builder)) if builder.is_dirty() && !builder.saving
                ) {
                    return self.close_top();
                }
                if self.modals.pop().is_some() {
                    Vec::new()
                } else {
                    vec![Effect::Quit(Exit::Interrupted)]
                }
            }
            // One rule, six lists: `→` runs whatever Enter runs where you are.
            // Dispatched on the context rather than declared six times, so the
            // help states the axis once instead of hanging two more keys off
            // every opener's row.
            CommandId::GuideNext => self.turn_guide(1),
            CommandId::GuidePrevious => self.turn_guide(-1),
            CommandId::FocusList => {
                self.set_focus(Focus::Projects);
                Vec::new()
            }
            CommandId::FocusDetail => {
                self.set_focus(Focus::Detail);
                Vec::new()
            }
            CommandId::BackToLibrary => self.toggle_templates(),
            CommandId::ShowLog => self.open_log(),
            CommandId::Suspend => vec![Effect::Suspend(Suspended::Shell)],

            CommandId::ActionsRun => {
                let chosen = match self.modals.top() {
                    Some(Modal::Actions(actions)) => crate::tui::app::actions::action_entries(self)
                        .get(actions.selected)
                        .map(|(id, _)| *id),
                    _ => None,
                };
                let Some(id) = chosen else {
                    return Vec::new();
                };
                self.modals.pop();
                self.run(id)
            }
            CommandId::Guide => {
                // The page for what is under the cursor: opened from a part of
                // the editor it lands on that part, opened from the tab it
                // starts at the beginning. A guide that always opens at page
                // one is a guide nobody opens twice.
                let page = match self.modals.top() {
                    Some(Modal::Builder(builder)) if builder.pending.is_none() => {
                        crate::tui::guide::page_for_row(builder.row())
                    }
                    _ => 0,
                };
                self.open_guide(page)
            }
            CommandId::BuilderExplain => {
                self.explain_open = !self.explain_open;
                self.info(if self.explain_open {
                    "explanation panel shown"
                } else {
                    "explanation panel hidden"
                });
                Vec::new()
            }
            CommandId::StudioNew => self.open_builder(None),
            CommandId::StudioEdit => match self.studio.selected_slug() {
                Some(slug) => self.open_builder(Some(slug)),
                None => Vec::new(),
            },
            CommandId::StudioFromFolder => {
                self.modals.push(Modal::Flow(Box::new(Flow::new(
                    FlowKind::FromFolder,
                    wizard::from_folder_form(),
                ))));
                Vec::new()
            }
            CommandId::StudioDelete => {
                let Some(slug) = self.studio.selected_slug() else {
                    return Vec::new();
                };
                self.modals.push(Modal::Confirm(Confirm {
                    prompt: format!("Delete template '{slug}' and its bundled files?"),
                    then: ConfirmThen::DeleteTemplate(slug),
                }));
                Vec::new()
            }
            CommandId::BuilderOpen => self.builder_open(),
            CommandId::BuilderAdd => self.builder_add(),
            CommandId::BuilderRemove => self.builder_remove(),
            CommandId::BuilderMoveUp | CommandId::BuilderMoveDown => {
                self.builder_move(id == CommandId::BuilderMoveUp)
            }
            CommandId::BuilderSave => self.save_template(),
            CommandId::SettingsChange => self.settings_change(),
            CommandId::SettingsFilter => {
                if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                    state.begin_filter();
                }
                Vec::new()
            }
            CommandId::Palette => {
                self.open_palette();
                Vec::new()
            }
            CommandId::Reload => {
                let mut effects = vec![self.discover(), Effect::LoadSummary];
                // And the pane: F5 is the key a person presses after editing
                // the file in another window.
                if self.detail_visible()
                    && let Some(project) = self.library.selected()
                {
                    effects.push(self.detail_effect(&project.path));
                }
                effects
            }
            CommandId::Reindex => self.run_action("reindexing…", Action::Reindex),
            CommandId::FocusNext | CommandId::FocusPrevious => {
                let forward = id == CommandId::FocusNext;
                let next = self.next_focus(forward);
                self.set_focus(next);
                Vec::new()
            }
            CommandId::Down | CommandId::Up => {
                let delta = if id == CommandId::Down { 1 } else { -1 };
                if !self.modals.is_empty() {
                    return self.step_top_modal(delta);
                }
                if self.screen == Screen::Templates {
                    return self.step_templates_or_pane(delta);
                }
                match self.focus {
                    Focus::Projects => {
                        self.library.step(delta);
                        self.after_selection_change()
                    }
                    Focus::Detail => {
                        self.move_pane_cursor(delta);
                        Vec::new()
                    }
                }
            }
            CommandId::PageDown | CommandId::PageUp | CommandId::HalfDown | CommandId::HalfUp => {
                let rows = self.page_rows() as isize;
                // Half a page is at least one row: on a window short enough
                // for `page_rows` to be 1, a half of it rounds to nothing and
                // the key would do nothing at all.
                let step = match id {
                    CommandId::HalfDown | CommandId::HalfUp => (rows / 2).max(1),
                    _ => rows,
                };
                let delta = if matches!(id, CommandId::PageDown | CommandId::HalfDown) {
                    step
                } else {
                    -step
                };
                if !self.modals.is_empty() {
                    return self.page_top_modal(delta);
                }
                if self.screen == Screen::Templates {
                    return self.step_templates_or_pane(delta);
                }
                match self.focus {
                    Focus::Detail => {
                        self.move_pane_cursor(delta);
                        Vec::new()
                    }
                    _ => {
                        self.library.jump(delta);
                        self.after_selection_change()
                    }
                }
            }
            CommandId::First | CommandId::Last => {
                let first = id == CommandId::First;
                if !self.modals.is_empty() {
                    return self.page_top_modal(if first { isize::MIN } else { isize::MAX });
                }
                if self.screen == Screen::Templates {
                    if self.focus == Focus::Detail {
                        self.studio.scroll = if first { 0 } else { self.studio_scroll_max() };
                        return Vec::new();
                    }
                    return self.jump_templates(first);
                }
                match self.focus {
                    Focus::Detail => {
                        self.move_pane_cursor(if first { isize::MIN } else { isize::MAX });
                        Vec::new()
                    }
                    Focus::Projects => {
                        if first {
                            self.library.select_first();
                        } else {
                            self.library.select_last();
                        }
                        self.after_selection_change()
                    }
                }
            }

            // Each of these dispatches on which dialog is on top, the way
            // `Close` always has: one key, one meaning, three shapes of
            // prompt under it.
            CommandId::PromptConfirm => match self.modals.top() {
                Some(Modal::TextPrompt(_)) => self.submit_text_prompt(),
                Some(Modal::Note(_)) => self.save_note(),
                Some(Modal::Onboarding(_)) => self.submit_onboarding(),
                _ => Vec::new(),
            },
            CommandId::PromptNewline => {
                if let Some(Modal::Note(note)) = self.modals.top_mut() {
                    note.area.apply(&Key::plain(KeyCode::Enter));
                }
                Vec::new()
            }
            CommandId::PromptCancel => {
                let skipped = matches!(self.modals.top(), Some(Modal::Onboarding(_)));
                self.modals.pop();
                if skipped {
                    self.info(validators::ONBOARDING_SKIPPED);
                }
                Vec::new()
            }
            CommandId::PickChoose => match self.modals.top() {
                Some(Modal::MultiPick(_)) => self.submit_multi_pick(),
                _ => self.choose_pick(),
            },
            CommandId::PickToggle => self.toggle_multi_pick(),
            CommandId::PickNext => self.step_pick(1),
            CommandId::PickPrevious => self.step_pick(-1),
            CommandId::PickCancel => {
                self.modals.pop();
                Vec::new()
            }
            CommandId::PaletteRun => self.run_palette_entry(),
            CommandId::PaletteNext => self.step_palette(1),
            CommandId::PalettePrevious => self.step_palette(-1),
            CommandId::PaletteClose => {
                self.modals.pop();
                Vec::new()
            }
            CommandId::SearchAccept => {
                self.search.editing = false;
                Vec::new()
            }
            // The bar's own Esc, rather than a rung bolted onto `Back`'s
            // ladder: it is the same family as `PickCancel` and
            // `PromptCancel`, each naming what it leaves.
            CommandId::SearchCancel => {
                if self.search.input.is_empty() {
                    self.search.editing = false;
                    return Vec::new();
                }
                self.search.input.clear();
                self.after_query_change()
            }
            CommandId::Search => {
                self.search.editing = true;
                self.set_focus(Focus::Projects);
                Vec::new()
            }
            CommandId::ClearSearch => {
                self.search.input.clear();
                self.after_query_change()
            }
            CommandId::SortCycle => {
                // The cycle walks the orders forwards. A direction is the
                // picker's to choose, so `s` from a reversed order lands on
                // the next one the right way up rather than staying upside
                // down for the rest of the session.
                let current = self.library.effective_sort(&self.search.query);
                self.library.explicit_sort = Some(Sort::new(current.order.next()));
                let sort = self.library.effective_sort(&self.search.query);
                self.info(format!("sorted by {}", sort.label()));
                self.reordered()
            }
            CommandId::SortPick => {
                // Both directions of every order that has two, each beside the
                // other: a person looking for the biggest folder and a person
                // looking for the smallest are reading the same list.
                let items = Order::CYCLE
                    .iter()
                    .flat_map(|order| {
                        let forwards = Sort::new(*order);
                        let backwards = Sort {
                            order: *order,
                            reversed: true,
                        };
                        [Some(forwards), order.reversible().then_some(backwards)]
                    })
                    .flatten()
                    .map(|sort| PickItem {
                        label: sort.label(),
                        detail: if sort.reversed {
                            sort.order.reversed_detail().to_string()
                        } else {
                            String::new()
                        },
                        value: sort.label(),
                    })
                    .collect();
                self.modals.push(Modal::Pick(PickState::new(
                    "Sort by",
                    items,
                    Then::SortPick,
                )));
                Vec::new()
            }
            CommandId::FilterTemplate => {
                let slug = self.library.selected().map(|p| p.template.clone());
                self.set_template_filter(slug)
            }
            CommandId::FilterBase => self.open_base_filter(),
            CommandId::FilterTag => {
                let items = self
                    .library
                    .known_tags
                    .iter()
                    .map(|tag| PickItem {
                        label: tag.clone(),
                        detail: String::new(),
                        value: tag.clone(),
                    })
                    .collect();
                self.modals.push(Modal::Pick(PickState::new(
                    "Filter by tag",
                    items,
                    Then::TagFilter,
                )));
                Vec::new()
            }
            CommandId::ClearFilters => self.clear_filters(),
            CommandId::Actions | CommandId::ActionsEnter => {
                self.modals.push(Modal::Actions(
                    crate::tui::app::actions::ActionsState::default(),
                ));
                Vec::new()
            }
            CommandId::PaneEdit => self.pane_edit_start(),
            CommandId::PaneEditConfirm => self.pane_edit_confirm(),
            CommandId::PaneEditSave => self.pane_edit_save(),
            CommandId::PaneEditCancel => {
                self.pane_edit = None;
                Vec::new()
            }
            CommandId::OpenFolder => self.spawn_for_selection(SpawnKind::Reveal),
            CommandId::OpenTerminal => self.spawn_for_selection(SpawnKind::Terminal),
            CommandId::CopyPath => match self.library.selected() {
                Some(project) => vec![Effect::Spawn(SpawnKind::Clipboard(
                    crate::util::paths::display_path(&project.path),
                ))],
                None => Vec::new(),
            },
            CommandId::ShowPath => {
                if let Some(project) = self.library.selected() {
                    let path = crate::util::paths::display_path(&project.path);
                    self.info(path);
                }
                Vec::new()
            }
            CommandId::ToggleDetail => {
                self.detail_open = !self.detail_open;
                if !self.pane_present() && self.focus == Focus::Detail {
                    self.set_focus(Focus::Projects);
                }
                self.after_selection_change()
            }
            CommandId::MarkToggle => {
                // Toggle the selected row and move on: marking a run is one
                // keystroke per row, the same shape every mark-and-act list has.
                let Some(path) = self.library.selected().map(|p| p.path.clone()) else {
                    return Vec::new();
                };
                if !self.library.marks.remove(&path) {
                    self.library.marks.insert(path.clone());
                }
                // Where `v` reaches from. Set on a mark *and* on an unmark:
                // the anchor is "the row Space last acted on", so changing
                // your mind about a row does not leave the anchor on it.
                self.library.last_mark = Some(path);
                self.library.step(1);
                Vec::new()
            }
            CommandId::MarkToHere => {
                let added = self.library.mark_to_here();
                let total = self.library.marks.len();
                self.info(format!(
                    "{added} more marked {} {total} in all",
                    self.theme.glyphs.sep
                ));
                Vec::new()
            }
            CommandId::MarkAll => {
                let before = self.library.marks.len();
                for row in 0..self.library.len() {
                    if let Some(project) = self.library.row(row) {
                        self.library.marks.insert(project.path.clone());
                    }
                }
                let now = self.library.marks.len();
                if now == before && before > 0 {
                    self.info(format!("{before} already marked"));
                } else {
                    self.info(format!("{now} marked"));
                }
                Vec::new()
            }
            CommandId::MarkNone => {
                let cleared = self.library.marks.len();
                self.library.marks.clear();
                self.info(format!("{cleared} marks cleared"));
                Vec::new()
            }
            CommandId::AddTag => self.open_add_tag(),
            CommandId::RemoveTags => {
                // Over marks the list is every tag any of them has; a project
                // that lacks one of the picked tags is simply left as it is.
                let targets = self.library.targets();
                let mut tags: Vec<String> = targets
                    .iter()
                    .flat_map(|project| project.tags.iter().cloned())
                    .collect();
                tags.sort();
                tags.dedup();
                if tags.is_empty() {
                    self.warn("no tags to remove");
                    return Vec::new();
                }
                let title = if targets.len() > 1 {
                    format!("Remove tags from {} projects", targets.len())
                } else {
                    "Remove tags".to_string()
                };
                self.modals.push(Modal::MultiPick(MultiPick::new(
                    title,
                    tags,
                    MultiThen::RemoveTags,
                )));
                Vec::new()
            }
            CommandId::ReautoTags => {
                if self.batching() {
                    return self.start_job(jobs::JobKind::ReautoTags, None);
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                self.run_action("re-deriving tags…", Action::ReautoTags(Box::new(project)))
            }
            CommandId::AddNote => {
                // The editor opens once; over marks the text it comes back
                // with goes to every one of them (`Msg::Resumed`).
                let Some(project) = self.library.targets().into_iter().next() else {
                    return Vec::new();
                };
                vec![Effect::Suspend(Suspended::Note(Box::new(project)))]
            }
            CommandId::NoteInline => {
                let count = self.library.targets().len();
                self.modals.push(Modal::Note(NoteState::new(count)));
                Vec::new()
            }
            CommandId::Rename => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                let mut prompt = TextPrompt::new(
                    validators::RENAME_PROMPT,
                    TextThen::Rename(project.path.clone()),
                );
                prompt.input =
                    crate::tui::widgets::input::LineEdit::with_text(project.name.clone());
                self.modals.push(Modal::TextPrompt(prompt));
                Vec::new()
            }
            CommandId::Move => self.open_move_picker(),
            CommandId::CopyTo => {
                let mut prompt = TextPrompt::new(validators::COPY_TO_PROMPT, TextThen::CopyTo);
                // The move progress modal is what a copy reports through too:
                // it is the same staged copy underneath.
                self.move_progress = None;
                prompt.input = crate::tui::widgets::input::LineEdit::default();
                self.modals.push(Modal::TextPrompt(prompt));
                Vec::new()
            }
            CommandId::Unregister => {
                // Nothing is lost by unregistering, so a yes/no is enough —
                // but the question names the folders it is about.
                let names = self.target_names();
                if names.is_empty() {
                    return Vec::new();
                }
                let then = match self.library.selected() {
                    _ if self.batching() => ConfirmThen::UnregisterBatch,
                    Some(project) => ConfirmThen::Unregister(project.path.clone()),
                    None => return Vec::new(),
                };
                self.modals.push(Modal::Confirm(Confirm {
                    prompt: validators::unregister_prompt(&names),
                    then,
                }));
                Vec::new()
            }
            CommandId::Delete => {
                // One word confirms a delete, single or batch, and the prompt
                // names every folder it is about.
                let names = self.target_names();
                if names.is_empty() {
                    return Vec::new();
                }
                let then = match self.library.selected() {
                    Some(project) => TextThen::Delete(project.path.clone()),
                    // A batch still needs a variant; the marks are the target
                    // and the path is ignored.
                    None => TextThen::Delete(std::path::PathBuf::new()),
                };
                self.modals.push(Modal::TextPrompt(TextPrompt::new(
                    validators::delete_prompt(&names),
                    then,
                )));
                Vec::new()
            }

            CommandId::ShowMetadata | CommandId::ShowJournal => self.open_view(id),
            CommandId::NewProject => self.open_create(),
            CommandId::Register => self.open_register(),
            CommandId::ApplyTemplate => self.open_apply(),
            CommandId::Templates => self.toggle_templates(),
            CommandId::Settings => self.open_settings(),
            CommandId::Reconcile => self.run_job(settings::Job::Reconcile),
            // `f` on the templates tab: filter the library by this template
            // **and go back to it**. The old strip set the filter and left you
            // looking at the strip, which is the one place the answer is not.
            CommandId::StripFilter => {
                let slug = self.studio.selected_slug();
                let next = if slug == self.library.template_filter {
                    None
                } else {
                    slug
                };
                let mut effects = self.toggle_templates();
                effects.extend(self.set_template_filter(next));
                effects
            }
        }
    }

    /// Esc on a dialog: one level at a time. A builder section goes back to
    /// the section list; the section list discards the template; everything
    /// else simply closes.
    /// Leave, from whichever gesture asked to.
    ///
    /// **Every quit goes through here**, because there are three of them and
    /// they used to answer differently: `CommandId::Quit` ran `Effect::Quit`
    /// on the spot, and it is reachable from the palette (`c`, "quit", Enter)
    /// and from the too-small-window guard as well as from `q` — so a template
    /// worked on for ten minutes could be thrown away with no question at all,
    /// while Esc on the same screen asked. `close_top` owns the question;
    /// this owns who has to ask it.
    fn quit(&mut self, exit: Exit) -> Vec<Effect> {
        // Quitting under a running move would abandon it mid-write.
        if self.job.is_some() || self.move_progress.is_some() {
            return self.request_cancel();
        }
        let dirty_builder = matches!(
            self.modals.top(),
            Some(Modal::Builder(builder)) if !builder.saving && builder.is_dirty()
        );
        if dirty_builder {
            let Some(Modal::Builder(builder)) = self.modals.top() else {
                return vec![Effect::Quit(exit)];
            };
            let prompt = validators::discard_template_prompt(builder.original_slug.is_some());
            self.modals.push(Modal::Confirm(Confirm {
                prompt,
                then: ConfirmThen::DiscardTemplate {
                    then_quit: Some(exit),
                },
            }));
            return Vec::new();
        }
        vec![Effect::Quit(exit)]
    }

    fn close_top(&mut self) -> Vec<Effect> {
        match self.modals.top_mut() {
            Some(Modal::Builder(builder)) => {
                // A save is in flight and the answer is nearly here; closing
                // now would leave the outcome nothing to land on.
                if builder.saving {
                    return Vec::new();
                }
                if builder.open.is_some() {
                    builder.open = None;
                } else if builder.is_dirty() {
                    // **A template that has been worked on is not thrown away
                    // on one keystroke.** Esc and `q` popped the builder and
                    // said so afterwards, on the status line, by which time
                    // every answer was gone — and `q` is the key this app
                    // teaches you to close things with everywhere else.
                    let prompt =
                        validators::discard_template_prompt(builder.original_slug.is_some());
                    self.modals.push(Modal::Confirm(Confirm {
                        prompt,
                        then: ConfirmThen::DiscardTemplate { then_quit: None },
                    }));
                } else {
                    self.modals.pop();
                    self.info("Closed — nothing was written.");
                }
            }
            Some(_) => {
                self.modals.pop();
            }
            None => {}
        }
        Vec::new()
    }

    /// Whether there is a pane beside the list right now: the library's
    /// closes under `layout::DETAIL_MIN_WIDTH` or on `i`, the templates tab's
    /// is always drawn.
    pub fn pane_present(&self) -> bool {
        self.screen == Screen::Templates || self.detail_visible()
    }

    /// The one way focus moves, so every mover leaves the same trace: the
    /// pane it moved to pulses, once, and only when it really moved.
    pub fn set_focus(&mut self, focus: Focus) {
        if self.focus != focus {
            self.focus_moved_at = Some(self.elapsed_ms);
            // An edit belongs to the row it was opened on; leaving the pane
            // leaves the row as it was.
            self.pane_edit = None;
            self.pane_pending = None;
        }
        self.focus = focus;
    }

    fn next_focus(&self, forward: bool) -> Focus {
        let mut ring = vec![Focus::Projects];
        if self.pane_present() {
            ring.push(Focus::Detail);
        }
        let at = ring.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = if forward {
            (at + 1) % ring.len()
        } else {
            (at + ring.len() - 1) % ring.len()
        };
        ring[next]
    }

    /// Start one mutation on a worker.
    ///
    /// **Refused while one is already running.** The runtime answers with the
    /// `ActionId` it was given and `on_action_done` drops anything that is not
    /// the one in flight, so a second action started over the first would make
    /// the first's outcome vanish — the row unpatched, the message never shown.
    /// The command registry's `not_busy` guards the keys; this guards the
    /// screens whose rows are not commands.
    fn run_action(&mut self, what: &'static str, action: Action) -> Vec<Effect> {
        if let Some(running) = self.busy {
            self.warn(format!("still {running}"));
            return Vec::new();
        }
        self.next_action += 1;
        let id = ActionId(self.next_action);
        self.busy = Some(what);
        self.busy_id = Some(id);
        vec![Effect::Run(id, Box::new(action))]
    }

    fn spawn_for_selection(&self, kind: fn(Box<Project>) -> SpawnKind) -> Vec<Effect> {
        match self.library.selected() {
            Some(project) => vec![Effect::Spawn(kind(Box::new(project.clone())))],
            None => Vec::new(),
        }
    }

    /// `b`: pick the base to restrict the list to. Every configured base is
    /// offered, mounted or not — an unmounted one showing `0` is the answer to
    /// "where did those projects go", where hiding it is not.
    fn open_base_filter(&mut self) -> Vec<Effect> {
        let Some(summary) = &self.summary else {
            return Vec::new();
        };
        let mut items: Vec<PickItem> = summary
            .bases
            .iter()
            .map(|base| PickItem {
                label: base.label.clone(),
                detail: base.note(),
                value: base.path.display().to_string(),
            })
            .collect();
        if items.is_empty() {
            return Vec::new();
        }
        // The way out is in the list, not only on a second key: a picker whose
        // only escape is Esc cannot say that "every base" is a choice.
        items.insert(
            0,
            PickItem {
                label: "every base".to_string(),
                detail: String::new(),
                value: String::new(),
            },
        );
        self.modals.push(Modal::Pick(PickState::new(
            "Show which base?",
            items,
            Then::BaseFilter,
        )));
        Vec::new()
    }
}

/// The state machine: the app and one message in, the effects out.
pub fn update(app: &mut App, msg: Msg) -> Vec<Effect> {
    app.handle(msg)
}
