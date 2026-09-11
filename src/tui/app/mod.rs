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
    Action, ActionId, ActionOutcome, ApplyRequest, CreateRequest, Effect, Exit, FollowUp,
    ListChange, Request, SpawnKind, Suspended, ViewKind,
};
use crate::tui::entry::Entry;
use crate::tui::fuzzy::Fuzzy;
use crate::tui::layout;
use crate::tui::motion;
use crate::tui::msg::{Mouse, MouseKind, Msg, Resumed};
use crate::tui::theme::Theme;
use crate::tui::validators;
use crate::tui::widgets::form::FormEvent;
use crate::util::diag::Level;
use crate::util::size_scan::SizeCell;
use data::{Prefs, ProjectDetail, Summary, TemplateCard};
use library::{LibraryState, Order, Sort};
use modal::{GuideState, MessageLevel, Modal, ModalStack, PickItem, PickState, Then};
use palette::{PaletteState, PaletteTarget};
use search::SearchState;
use settings::{Editing, Kind, Onboarding, SettingsState};
use studio::{Builder, Open, Row, Studio};
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
    /// Whether the terminal reports the mouse: the `mouse` setting, carried
    /// here so the palette can flip it and the settings screen can see it.
    pub mouse: bool,
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
            mouse: false,
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

    /// The detail cached for the selected row.
    pub fn selected_detail(&self) -> Option<&ProjectDetail> {
        self.library
            .selected()
            .and_then(|p| self.details.get(&p.path))
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
            Msg::Mouse(mouse) => self.on_mouse(mouse),
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
                let mouse = crate::core::config::on_off(&loaded.mouse).unwrap_or(false);
                if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                    state.refresh(*loaded);
                }
                // A theme — or a motion or mouse setting — written on this
                // screen takes effect on the frame that shows it was written.
                let mut effects = vec![Effect::Retheme { theme, motion }];
                if mouse != self.mouse {
                    self.mouse = mouse;
                    effects.push(Effect::Mouse(mouse));
                }
                effects
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

    // --- the mouse --------------------------------------------------------

    /// What a click and a wheel turn mean.
    ///
    /// **The wheel needs no geometry at all**: it is `↑`/`↓`, three at a time,
    /// wherever the keys already go — so it is right in every list, every
    /// scrollable dialog and the detail pane without a second copy of the
    /// layout to drift from the first.
    ///
    /// A click needs to know what is under it, so it is answered only where
    /// `layout` already owns the geometry: the dashboard's regions, and the
    /// palette's centred box (`palette_rows` computes it either way). Anywhere
    /// else a click does nothing, which is better than a click that guesses.
    fn on_mouse(&mut self, mouse: Mouse) -> Vec<Effect> {
        if layout::too_small(self.area()) {
            return Vec::new();
        }
        match mouse.kind {
            MouseKind::ScrollUp | MouseKind::ScrollDown => {
                let key = if mouse.kind == MouseKind::ScrollUp {
                    Key::plain(KeyCode::Up)
                } else {
                    Key::plain(KeyCode::Down)
                };
                let mut effects = Vec::new();
                for _ in 0..3 {
                    effects.extend(self.on_key(key));
                }
                effects
            }
            MouseKind::Click => self.on_click(mouse.column, mouse.row),
        }
    }

    fn on_click(&mut self, column: u16, row: u16) -> Vec<Effect> {
        if matches!(self.modals.top(), Some(Modal::Palette(_))) {
            return self.click_palette(column, row);
        }
        if !self.modals.is_empty() {
            return Vec::new();
        }
        let regions = self.regions();
        if inside(regions.search, column, row) {
            self.search.editing = true;
            self.set_focus(Focus::Projects);
            return Vec::new();
        }
        if let Some(detail) = regions.detail
            && inside(detail, column, row)
        {
            self.set_focus(Focus::Detail);
            return Vec::new();
        }
        if !inside(regions.table, column, row) {
            return Vec::new();
        }
        self.set_focus(Focus::Projects);
        // The table's border and its header row: the first project sits two
        // rows below the top of the region.
        let Some(offset_row) = row.checked_sub(regions.table.y + 2) else {
            return Vec::new();
        };
        // Only the rows that are drawn. `inside` accepts the whole region,
        // border included, so a click on the bottom edge picked the row one
        // past the last visible one and scrolled the viewport to reach it.
        if offset_row as usize >= regions.table_rows() {
            return Vec::new();
        }
        let at = self.library.offset + offset_row as usize;
        if at >= self.library.len() {
            return Vec::new();
        }
        self.library.selected = Some(at);
        self.after_selection_change()
    }

    /// A click in the palette picks the entry under it and runs it, the way a
    /// click in a menu does.
    fn click_palette(&mut self, column: u16, row: u16) -> Vec<Effect> {
        let box_area = layout::centered(self.area(), 70, 70);
        if !inside(box_area, column, row) {
            return Vec::new();
        }
        // One border row, the query line, then a blank one.
        let Some(offset_row) = row.checked_sub(box_area.y + 3) else {
            return Vec::new();
        };
        let at = match self.modals.top() {
            Some(Modal::Palette(palette)) => palette.offset + offset_row as usize,
            _ => return Vec::new(),
        };
        let picked = match self.modals.top_mut() {
            Some(Modal::Palette(palette)) if at < palette.entries.len() => {
                palette.selected = Some(at);
                true
            }
            _ => false,
        };
        if !picked {
            return Vec::new();
        }
        self.on_palette_key(Key::plain(KeyCode::Enter))
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

    /// A pane edit is open: the field has first refusal on anything typed,
    /// the registry answers the rest (`Enter`, `Esc`, `Ctrl-S`), and what
    /// neither takes goes to the field — which is how Enter in the notes is a
    /// new line: `PaneEditConfirm` is hidden there, so the text area gets it.
    fn on_pane_edit_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::PaneEdit, key, self)
        {
            return self.run(id);
        }
        let Some(edit) = &mut self.pane_edit else {
            return Vec::new();
        };
        if edit.pending() {
            return Vec::new();
        }
        let changed = match edit {
            pane::PaneEdit::Line { input, .. } => input.apply(&key),
            pane::PaneEdit::Note { area, .. } => area.apply(&key),
        };
        if changed {
            edit.clear_error();
        }
        Vec::new()
    }

    /// Enter on a pane row: what the row is decides what opens — or, for a
    /// todo, what is written at once, since a toggle has nothing to type.
    fn pane_edit_start(&mut self) -> Vec<Effect> {
        let rows = self.pane_rows();
        let Some(row) = rows.get(self.pane_cursor).cloned() else {
            return Vec::new();
        };
        let at = self.pane_cursor;
        match row {
            pane::PaneRow::Name => self.run(CommandId::Rename),
            pane::PaneRow::AddTag => self.open_add_tag(),
            pane::PaneRow::AddNote => self.run(CommandId::NoteInline),
            pane::PaneRow::EarlierNotes(_) => self.run(CommandId::ShowJournal),
            pane::PaneRow::AddTodo => {
                self.modals.push(Modal::TextPrompt(TextPrompt::new(
                    validators::ADD_TODO_PROMPT,
                    TextThen::AddTodo,
                )));
                Vec::new()
            }
            pane::PaneRow::Todo { ordinal, text, .. } => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                let effects = self.run_action(
                    "writing…",
                    Action::ToggleTodo {
                        project: Box::new(project),
                        ordinal,
                        was: text,
                    },
                );
                if !effects.is_empty() {
                    self.pane_pending = Some(pane::PaneTarget::Todo(ordinal));
                }
                effects
            }
            pane::PaneRow::Tag(tag) => {
                self.pane_edit = Some(pane::PaneEdit::Line {
                    row: at,
                    input: crate::tui::widgets::input::LineEdit::with_text(tag.clone()),
                    target: pane::EditTarget::Tag(tag),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            pane::PaneRow::Variable {
                slug,
                label,
                kind: pane::VarKind::Select(options),
                value,
            } => {
                // A select offers its options and nothing else — the picker
                // is the one shape that cannot hold anything outside them.
                let items: Vec<PickItem> = options
                    .into_iter()
                    .map(|option| PickItem {
                        detail: if option == value {
                            "current".to_string()
                        } else {
                            String::new()
                        },
                        label: option.clone(),
                        value: option,
                    })
                    .collect();
                self.modals.push(Modal::Pick(PickState::new(
                    format!(" {label} "),
                    items,
                    Then::PaneVariable(slug),
                )));
                Vec::new()
            }
            pane::PaneRow::Variable {
                slug,
                kind: pane::VarKind::Text,
                value,
                ..
            } => {
                self.pane_edit = Some(pane::PaneEdit::Line {
                    row: at,
                    input: crate::tui::widgets::input::LineEdit::with_text(value),
                    target: pane::EditTarget::Variable(slug),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            pane::PaneRow::Note { ordinal, .. } => {
                let text = self
                    .library
                    .selected()
                    .and_then(|project| self.details.get(&project.path))
                    .and_then(|detail| detail.notes.get(ordinal))
                    .map(|note| note.text.clone())
                    .unwrap_or_default();
                self.pane_edit = Some(pane::PaneEdit::Note {
                    row: at,
                    ordinal,
                    was: text.clone(),
                    area: Box::new(crate::tui::widgets::text_area::TextArea::with_text(&text)),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Enter on the pane's line editor: what was typed goes to the project —
    /// unless nothing changed, which is a cancel, or a tag is not a tag, which
    /// is a refusal under the line.
    fn pane_edit_confirm(&mut self) -> Vec<Effect> {
        let Some(pane::PaneEdit::Line { target, input, .. }) = &self.pane_edit else {
            return Vec::new();
        };
        let text = input.text().trim().to_string();
        let Some(project) = self.library.selected().cloned() else {
            self.pane_edit = None;
            return Vec::new();
        };
        let action = match target {
            pane::EditTarget::Variable(slug) => Action::SetVariable {
                project: Box::new(project),
                slug: slug.clone(),
                value: text,
            },
            pane::EditTarget::Tag(from) => {
                if text == *from {
                    self.pane_edit = None;
                    return Vec::new();
                }
                let to = if text.is_empty() {
                    None
                } else {
                    match validators::tag(&text) {
                        Ok(tag) => Some(tag),
                        Err(error) => {
                            if let Some(edit) = &mut self.pane_edit {
                                edit.fail(error);
                            }
                            return Vec::new();
                        }
                    }
                };
                Action::ReplaceTag {
                    project: Box::new(project),
                    from: from.clone(),
                    to,
                }
            }
        };
        self.send_pane_edit(action)
    }

    /// `Ctrl-S` on the pane's note editor: the note, rewritten — or, emptied,
    /// removed. Unchanged text is a cancel.
    fn pane_edit_save(&mut self) -> Vec<Effect> {
        let Some(pane::PaneEdit::Note {
            area, ordinal, was, ..
        }) = &self.pane_edit
        else {
            return Vec::new();
        };
        let text = area.text();
        if text.trim() == was.trim() {
            self.pane_edit = None;
            return Vec::new();
        }
        let (ordinal, was) = (*ordinal, was.clone());
        let Some(project) = self.library.selected().cloned() else {
            self.pane_edit = None;
            return Vec::new();
        };
        self.send_pane_edit(Action::ReplaceNote {
            project: Box::new(project),
            ordinal,
            was,
            text,
        })
    }

    /// The edit stays open, marked pending, until the worker answers: an
    /// `Ok` closes it and pulses the row, an `Err` lands on it.
    fn send_pane_edit(&mut self, action: Action) -> Vec<Effect> {
        if let Some(edit) = &mut self.pane_edit {
            edit.set_pending(true);
        }
        let effects = self.run_action("writing…", action);
        if effects.is_empty()
            && let Some(edit) = &mut self.pane_edit
        {
            // Refused before it left — something else is still running.
            edit.set_pending(false);
        }
        effects
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

    /// The action menu: a verb's own key runs it and closes the menu, exactly
    /// as Enter on its row would; anything else — help, the palette, the
    /// arrows — runs over the menu and leaves it open.
    fn on_actions_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(id) = command::lookup(self.context(), key, self) else {
            return Vec::new();
        };
        let is_verb = crate::tui::app::actions::action_entries(self)
            .iter()
            .any(|(entry, _)| *entry == id);
        if is_verb {
            self.modals.pop();
        }
        self.run(id)
    }

    fn on_text_prompt_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Prompt, key, self)
        {
            return self.run(id);
        }
        let changed = match self.modals.top_mut() {
            Some(Modal::TextPrompt(prompt)) => prompt.input.apply(&key),
            _ => false,
        };
        if changed && let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
            prompt.error = None;
        }
        Vec::new()
    }

    fn submit_text_prompt(&mut self) -> Vec<Effect> {
        let (text, then) = match self.modals.top() {
            Some(Modal::TextPrompt(prompt)) => {
                (prompt.input.text().to_string(), prompt.then.clone())
            }
            _ => return Vec::new(),
        };
        match then {
            TextThen::Rename(path) => {
                if let Err(error) = validators::folder_name(&text) {
                    if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                        prompt.error = Some(error);
                    }
                    return Vec::new();
                }
                self.modals.pop();
                let Some(project) = self.project_at(&path) else {
                    return self.gone_from_the_library();
                };
                self.run_action(
                    "renaming…",
                    Action::Rename {
                        project: Box::new(project),
                        name: text,
                    },
                )
            }
            TextThen::AddTodo => {
                self.modals.pop();
                if text.trim().is_empty() {
                    return Vec::new();
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                let next = self
                    .details
                    .get(&project.path)
                    .map(|detail| detail.todos.len())
                    .unwrap_or(0);
                let effects = self.run_action(
                    "writing…",
                    Action::AddTodo {
                        project: Box::new(project),
                        text,
                    },
                );
                if !effects.is_empty() {
                    self.pane_pending = Some(pane::PaneTarget::Todo(next));
                }
                effects
            }
            TextThen::AddTag => {
                if text.trim().is_empty() {
                    self.modals.pop();
                    return Vec::new();
                }
                // Refused under the line, with the rule named, while the
                // text is still there to correct — the same shape a rename
                // takes.
                let tag = match validators::tag(&text) {
                    Ok(tag) => tag,
                    Err(error) => {
                        if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                            prompt.error = Some(error);
                        }
                        return Vec::new();
                    }
                };
                self.modals.pop();
                self.add_tag(tag)
            }
            TextThen::CopyTo => {
                let typed = text.trim().to_string();
                if typed.is_empty() {
                    return Vec::new();
                }
                self.modals.pop();
                // Expanded here, refused in the engine: `~/backups` has to mean
                // the same thing it means in `config set bases`, and the rule
                // about bases is stated once, in `copy_engine`.
                let destination = match crate::core::config::expand_base_path(&typed) {
                    Ok(path) => path,
                    Err(error) => {
                        self.warn(format!("{error:#}"));
                        return Vec::new();
                    }
                };
                if self.batching() {
                    return self.start_job(jobs::JobKind::CopyTo(destination), None);
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                self.run_action(
                    "copying…",
                    Action::CopyTo {
                        project: Box::new(project),
                        destination,
                    },
                )
            }
            TextThen::RaiseCounter => {
                self.modals.pop();
                match text.trim().parse::<u64>() {
                    Ok(value) => self.run_action(
                        settings::Job::RaiseCounter.busy(),
                        Action::RaiseCounter(value),
                    ),
                    Err(_) => {
                        self.warn(format!("expected a number, got '{}'", text.trim()));
                        Vec::new()
                    }
                }
            }
            TextThen::Delete(path) => {
                if !text.trim().eq_ignore_ascii_case(validators::DELETE_WORD) {
                    // The text stays: one Backspace fixes a typo.
                    if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                        prompt.error = Some(validators::DELETE_MISMATCH.to_string());
                    }
                    return Vec::new();
                }
                self.modals.pop();
                if self.batching() {
                    return self.start_job(jobs::JobKind::Delete, None);
                }
                let Some(project) = self.project_at(&path) else {
                    return self.gone_from_the_library();
                };
                self.run_action("deleting…", Action::Delete(Box::new(project)))
            }
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

    /// One tag, on the selection or on every mark.
    fn add_tag(&mut self, tag: String) -> Vec<Effect> {
        if self.batching() {
            return self.start_job(jobs::JobKind::AddTag(tag), None);
        }
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        self.run_action(
            "tagging…",
            Action::AddTag {
                project: Box::new(project),
                tag,
            },
        )
    }

    /// One note, on the selection or on every mark.
    fn add_note(&mut self, text: String) -> Vec<Effect> {
        if self.batching() {
            return self.start_job(jobs::JobKind::Note(text), None);
        }
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        // From the pane, the cursor follows the note to where it lands: the
        // end of the list. From the list, the pane's cursor is not in play.
        let next = self
            .details
            .get(&project.path)
            .map(|detail| detail.notes.len())
            .unwrap_or(0);
        let effects = self.run_action(
            "adding a note…",
            Action::AppendNote {
                project: Box::new(project),
                text,
            },
        );
        if !effects.is_empty() && self.focus == Focus::Detail {
            self.pane_pending = Some(pane::PaneTarget::Note(next));
        }
        effects
    }

    /// The quick note: Enter saves, Alt-Enter breaks a line, Esc cancels;
    /// everything else is the text area's.
    fn on_note_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Prompt, key, self)
        {
            return self.run(id);
        }
        if let Some(Modal::Note(note)) = self.modals.top_mut() {
            note.area.apply(&key);
        }
        Vec::new()
    }

    /// Enter on a quick note: keep it, or say nothing was written.
    fn save_note(&mut self) -> Vec<Effect> {
        let Some(Modal::Note(note)) = self.modals.pop() else {
            return Vec::new();
        };
        let text = note.area.text().trim().to_string();
        if text.is_empty() {
            self.info("no note written");
            return Vec::new();
        }
        self.add_note(text)
    }

    fn on_confirm_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if !key.ctrl => self.answer_confirm(true),
            KeyCode::Char('n') | KeyCode::Char('N') if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
            // Enter is the commonest reflex there is on a two-button dialog,
            // and nothing bound it in `Context::Modal` — so it fell through to
            // the registry and produced silence. It answers `y`, which is the
            // key line's first entry and the default every `confirm` in this
            // app already offers.
            KeyCode::Enter => self.answer_confirm(true),
            _ => self.lookup_and_run(key),
        }
    }

    fn answer_confirm(&mut self, yes: bool) -> Vec<Effect> {
        let then = match self.modals.top() {
            Some(Modal::Confirm(confirm)) => confirm.then.clone(),
            _ => return Vec::new(),
        };
        self.modals.pop();
        if !yes {
            return Vec::new();
        }
        match then {
            ConfirmThen::Unregister(path) => {
                let Some(project) = self.project_at(&path) else {
                    return self.gone_from_the_library();
                };
                self.run_action("unregistering…", Action::Unregister(Box::new(project)))
            }
            ConfirmThen::DeleteTemplate(slug) => {
                self.run_action("deleting the template…", Action::DeleteTemplate(slug))
            }
            ConfirmThen::DiscardTemplate { then_quit } => {
                // The confirm was popped above; the builder under it goes now.
                self.modals.pop();
                self.info("Discarded — the template was not written.");
                match then_quit {
                    Some(exit) => vec![Effect::Quit(exit)],
                    None => Vec::new(),
                }
            }
            ConfirmThen::DeleteBatch => self.start_job(jobs::JobKind::Delete, None),
            ConfirmThen::UnregisterBatch => self.start_job(jobs::JobKind::Unregister, None),
        }
    }

    /// A multi-pick holds no query, so every key is the registry's — Space
    /// included, which is why `PickToggle` is `Hidden` in the picker that does
    /// have one.
    fn on_multi_pick_key(&mut self, key: Key) -> Vec<Effect> {
        self.lookup_and_run(key)
    }

    /// Space on a multi-pick: put the row in the set, or take it out.
    fn toggle_multi_pick(&mut self) -> Vec<Effect> {
        if let Some(Modal::MultiPick(pick)) = self.modals.top_mut()
            && let Some(flag) = pick.picked.get_mut(pick.selected)
        {
            *flag = !*flag;
        }
        Vec::new()
    }

    fn submit_multi_pick(&mut self) -> Vec<Effect> {
        let (chosen, then) = match self.modals.top() {
            Some(Modal::MultiPick(pick)) => (pick.chosen(), pick.then),
            _ => return Vec::new(),
        };
        self.modals.pop();
        match then {
            MultiThen::RemoveTags => {
                if chosen.is_empty() {
                    return Vec::new();
                }
                if self.batching() {
                    return self.start_job(jobs::JobKind::RemoveTags(chosen), None);
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                self.run_action(
                    "removing tags…",
                    Action::RemoveTags {
                        project: Box::new(project),
                        tags: chosen,
                    },
                )
            }
        }
    }

    /// The palette is a query with a list under it, so the same rule the
    /// search bar follows applies: everything printable is the query, and only
    /// a key a field cannot hold reaches the registry. `Ctrl-p` is the
    /// previous entry here rather than "open the palette", which is why the
    /// opener is bound in every context **except** this one.
    fn on_palette_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Palette, key, self)
        {
            return self.run(id);
        }
        let changed = match self.modals.top_mut() {
            Some(Modal::Palette(palette)) => palette.input.apply(&key),
            _ => false,
        };
        if changed {
            self.refresh_palette();
        }
        Vec::new()
    }

    /// Move the palette's cursor, keeping its window around it.
    fn step_palette(&mut self, delta: isize) -> Vec<Effect> {
        let rows = self.palette_rows();
        if let Some(Modal::Palette(palette)) = self.modals.top_mut() {
            palette.step(delta);
            palette.clamp_viewport(rows);
        }
        Vec::new()
    }

    /// Run whatever is under the palette's cursor: a command dispatches
    /// exactly the `CommandId` its key would, a project selects its row, a
    /// template filters by it.
    fn run_palette_entry(&mut self) -> Vec<Effect> {
        let chosen = match self.modals.top() {
            Some(Modal::Palette(palette)) => palette.chosen().cloned(),
            _ => None,
        };
        self.modals.pop();
        let Some(entry) = chosen else {
            return Vec::new();
        };
        if !entry.enabled {
            self.warn(format!(
                "{}: {}",
                entry.title,
                entry.reason.unwrap_or("not available right now")
            ));
            return Vec::new();
        }
        match entry.target {
            PaletteTarget::Command(id) => self.run(id),
            PaletteTarget::Project(path) => {
                self.set_focus(Focus::Projects);
                if !self.library.select_path(&path) {
                    // Hidden by the query or the filter: show everything.
                    self.search.input.clear();
                    self.search.sync();
                    self.library.template_filter = None;
                    self.recompute();
                    self.library.select_path(&path);
                }
                self.after_selection_change()
            }
            PaletteTarget::Template(slug) => self.set_template_filter(Some(slug)),
        }
    }

    /// Rows the palette list has, for its viewport.
    fn palette_rows(&self) -> usize {
        layout::centered(self.area(), 70, 70)
            .height
            .saturating_sub(4) as usize
    }

    fn refresh_palette(&mut self) {
        let query = match self.modals.top() {
            Some(Modal::Palette(palette)) => palette.input.text().to_string(),
            _ => return,
        };
        let commands = command::palette_entries(self.focus_context(), self);
        let entries = palette::build(
            &query,
            commands,
            &self.library,
            &self.templates.cards,
            &mut self.fuzzy,
        );
        if let Some(Modal::Palette(palette)) = self.modals.top_mut() {
            palette.set_entries(entries);
        }
    }

    fn open_palette(&mut self) {
        self.modals.push(Modal::Palette(PaletteState::default()));
        self.refresh_palette();
    }

    /// Everything printable is the query, exactly as in the palette; the rows
    /// and the way out are commands.
    fn on_pick_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Pick, key, self)
        {
            return self.run(id);
        }
        if let Some(Modal::Pick(pick)) = self.modals.top_mut()
            && pick.query.apply(&key)
        {
            pick.rank(&mut self.fuzzy);
        }
        Vec::new()
    }

    /// Move a picker's cursor, whichever kind it is.
    fn step_pick(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        match self.modals.top_mut() {
            Some(Modal::Pick(pick)) => {
                pick.step(delta);
                pick.clamp_viewport(layout::list_rows(
                    layout::pick_box(area, pick.ranked.len()),
                    2,
                ));
            }
            // A multi-pick has no viewport of its own to keep — the box is
            // sized to its rows — so it is `step_top_modal`'s arm verbatim.
            Some(Modal::MultiPick(_)) => return self.step_top_modal(delta),
            _ => {}
        }
        Vec::new()
    }

    /// Take the row under a single picker's cursor and do what it was opened
    /// for.
    fn choose_pick(&mut self) -> Vec<Effect> {
        let Some(Modal::Pick(pick)) = self.modals.pop() else {
            return Vec::new();
        };
        let Some(item) = pick.chosen().cloned() else {
            return Vec::new();
        };
        match pick.then {
            Then::SortPick => {
                self.library.explicit_sort = Sort::from_label(&item.value);
                self.reordered()
            }
            Then::TemplateFilter => self.set_template_filter(Some(item.value.clone())),
            // A tag filter is `tag:x` in the bar and nothing else — one
            // mechanism, so clearing it is the same Esc rung as clearing any
            // other query and the bar goes on reporting what is filtering the
            // list.
            Then::TagFilter => {
                self.search.input.set_text(format!("tag:{}", item.value));
                self.search.sync();
                let effects = self.after_query_change();
                self.pulse_selected();
                effects
            }
            Then::BaseFilter => {
                let base = (!item.value.is_empty()).then(|| PathBuf::from(&item.value));
                self.set_base_filter(base)
            }
            Then::AddTag => {
                if item.value == crate::tui::app::actions::NEW_TAG {
                    self.modals.push(Modal::TextPrompt(TextPrompt::new(
                        validators::ADD_TAG_PROMPT,
                        TextThen::AddTag,
                    )));
                    return Vec::new();
                }
                self.add_tag(item.value.clone())
            }
            Then::MoveToBase => {
                let target = PathBuf::from(item.value.clone());
                // `batching()`, not `!marks.is_empty()`: marks are kept
                // by path and survive a filter change, so a marked row
                // can be off screen while the verb is aimed at it. Every
                // other verb asks this question the same way — the raw
                // mark set is what "batch tagging does nothing" was, and
                // this was the last caller still asking it.
                if self.batching() {
                    self.start_job(jobs::JobKind::Move, Some(target))
                } else {
                    self.run_move(target)
                }
            }
            Then::PaneVariable(slug) => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                // The picker was the edit; there is no line to keep open, so
                // the cursor's row is remembered for the pulse by the answer.
                self.pane_edit = Some(pane::PaneEdit::Line {
                    row: self.pane_cursor,
                    target: pane::EditTarget::Variable(slug.clone()),
                    input: crate::tui::widgets::input::LineEdit::with_text(item.value.clone()),
                    error: None,
                    pending: false,
                });
                self.send_pane_edit(Action::SetVariable {
                    project: Box::new(project),
                    slug,
                    value: item.value.clone(),
                })
            }
            Then::FormField(key) => {
                let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
                    return Vec::new();
                };
                let Some(field) = flow.form.field_mut(&key) else {
                    return Vec::new();
                };
                if !field.select(&item.value) {
                    return Vec::new();
                }
                flow.form.selected = flow
                    .form
                    .fields
                    .iter()
                    .position(|field| field.key == key)
                    .unwrap_or(flow.form.selected);
                self.on_form_changed()
            }
        }
    }

    /// Help and a message: a pager. Enter and `?` close it too — `?` because
    /// the key that opened it should put it away, and it must not open a
    /// second help over the first.
    fn on_scroll_modal_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter | KeyCode::Char('?') if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
            KeyCode::Char(' ') if !key.ctrl => self.scroll_top_modal(self.page_rows() as isize),
            _ => self.lookup_and_run(key),
        }
    }

    /// The guide's own keys.
    ///
    /// A reader owns its own left and right, which is why the guide has a
    /// `Context` to itself: `←` turns a page here and backs out of a dialog
    /// everywhere else, and both can be declared. Space is the pager's own key
    /// and stays here — `PageDown` cannot carry it, because Space is the mark
    /// on the list `PageDown` also answers on.
    fn on_guide_key(&mut self, key: Key) -> Vec<Effect> {
        if key == Key::ch(' ') {
            return self.scroll_top_modal(self.page_rows() as isize);
        }
        self.lookup_and_run(key)
    }

    /// Turn a page of the guide. Forward from the last page leaves it: a
    /// reader who keeps pressing the same key reaches the end and is let go,
    /// rather than pressing it against the last page.
    fn turn_guide(&mut self, delta: isize) -> Vec<Effect> {
        let last = matches!(self.modals.top(), Some(Modal::Guide(state)) if state.is_last());
        if delta > 0 && last {
            self.modals.pop();
            return Vec::new();
        }
        if let Some(Modal::Guide(state)) = self.modals.top_mut() {
            state.turn(delta);
        }
        Vec::new()
    }

    /// Scroll whatever dialog is on top by `delta` rows, clamped to its
    /// content. `isize::MIN` and `isize::MAX` are the ends.
    fn scroll_top_modal(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        let Some(top) = self.modals.top_mut() else {
            return Vec::new();
        };
        let (scroll, lines, rows) = match top {
            Modal::Help { ctx, scroll } => {
                let inner = layout::help_box(area);
                (
                    scroll,
                    command::help_line_count(*ctx, inner.width.saturating_sub(2) as usize),
                    inner.height.saturating_sub(2) as usize,
                )
            }

            Modal::Guide(state) => {
                let box_ = layout::guide_box(area);
                (
                    &mut state.scroll,
                    crate::tui::view::modals::guide_rows(state.page, box_),
                    // The border, the footer and the key line: what
                    // `view::builder::frame_parts` takes off the top and the
                    // bottom before the body is drawn.
                    box_.height.saturating_sub(4) as usize,
                )
            }
            Modal::Message { lines, scroll, .. } => (
                scroll,
                // Wrapped rows, not entries — the paragraph wraps, and
                // `Modal::Help` above has always counted them the same way.
                crate::tui::view::modals::message_rows(
                    lines,
                    crate::tui::view::modals::message_text_width(area),
                ),
                layout::message_box(area).height.saturating_sub(2) as usize,
            ),
            _ => return Vec::new(),
        };
        let max = lines.saturating_sub(rows);
        *scroll = scroll.saturating_add_signed(delta).min(max);
        Vec::new()
    }

    /// The pane's rows for the selected project — `pane::pane_rows` over what
    /// has been read of it. Empty with nothing selected.
    pub fn pane_rows(&self) -> Vec<pane::PaneRow> {
        let Some(project) = self.library.selected() else {
            return Vec::new();
        };
        pane::pane_rows(project, self.details.get(&project.path))
    }

    /// How many rows the pane shows at once: its height inside the border.
    fn pane_rows_on_screen(&self) -> usize {
        self.regions()
            .detail
            .map(|pane| pane.height.saturating_sub(2) as usize)
            .unwrap_or(0)
    }

    /// Put the pane's cursor back on the row an edit was about, pulse it, and
    /// keep it in view. Nothing happens when the row is not there (yet).
    fn settle_pane_cursor(&mut self, target: &pane::PaneTarget) {
        let rows = self.pane_rows();
        let Some(row) = pane::find_row(&rows, target) else {
            return;
        };
        self.pane_cursor = row;
        self.pane_pulses.start(row, self.elapsed_ms);
        self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
            self.detail_scroll,
            Some(row),
            rows.len(),
            self.pane_rows_on_screen(),
        );
    }

    /// Move the pane's cursor by `delta` selectable rows (`isize::MIN` and
    /// `isize::MAX` are the ends) and scroll the pane so it stays in view —
    /// the same bargain the table makes with its viewport.
    fn move_pane_cursor(&mut self, delta: isize) {
        let rows = self.pane_rows();
        self.pane_cursor = pane::step_cursor(&rows, self.pane_cursor, delta);
        self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
            self.detail_scroll,
            Some(self.pane_cursor),
            rows.len(),
            self.pane_rows_on_screen(),
        );
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
            // Flip the setting through `config set`, so the word on disk is
            // the word every surface reads, and switch the terminal the
            // moment the write is on its way — waiting for the settings
            // screen to re-read would leave the mouse as it was until `,`.
            CommandId::ToggleMouse => {
                let wanted = !self.mouse;
                let mut effects = self.run_action(
                    "saving…",
                    Action::SetConfig {
                        key: "mouse",
                        value: if wanted { "on" } else { "off" }.to_string(),
                    },
                );
                if !effects.is_empty() {
                    self.mouse = wanted;
                    effects.push(Effect::Mouse(wanted));
                }
                effects
            }
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

    // --- the flows: create, apply, register -------------------------------

    /// The templates on disk, by slug. Deliberately not `templates.cards`,
    /// which also carries a bare card for every slug the projects mention that
    /// no template answers to — `(registered)` is a slug, not a template.
    fn template_slugs(&self) -> Vec<String> {
        self.summary
            .as_ref()
            .map(|summary| {
                summary
                    .templates
                    .iter()
                    .map(|card| card.slug.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn prefs(&self) -> Prefs {
        self.summary
            .as_ref()
            .map(|summary| summary.prefs.clone())
            .unwrap_or_default()
    }

    /// The bases a new project could go in, the configured default first —
    /// which is what makes a plain Enter mean exactly what it always meant.
    fn base_options(&self) -> Vec<String> {
        let Some(summary) = &self.summary else {
            return Vec::new();
        };
        let mut bases: Vec<&data::BaseInfo> = summary
            .bases
            .iter()
            .filter(|base| base.probe.usable())
            .collect();
        bases.sort_by_key(|base| !base.is_default);
        bases
            .iter()
            .map(|base| crate::util::paths::display_path(&base.path))
            .collect()
    }

    /// `n`: the new-project wizard.
    fn open_create(&mut self) -> Vec<Effect> {
        let slugs = self.template_slugs();
        if slugs.is_empty() {
            self.warn(command::NO_TEMPLATES);
            return Vec::new();
        }
        let default = self.prefs().default_template;
        let at = slugs.iter().position(|slug| *slug == default).unwrap_or(0);
        let mut flow = Flow::new(
            FlowKind::Create,
            wizard::create_form(&slugs, at, &self.base_options()),
        );
        flow.auto_commit = !self.prefs().confirm_create;
        flow.pending = true;
        let slug = slugs[at].clone();
        self.modals.push(Modal::Flow(Box::new(flow)));
        vec![Effect::LoadTemplate { slug }]
    }

    /// The apply flow: a template over a folder that already exists.
    fn open_apply(&mut self) -> Vec<Effect> {
        let slugs = self.template_slugs();
        if slugs.is_empty() {
            self.warn(command::NO_TEMPLATES);
            return Vec::new();
        }
        let default = self.prefs().default_template;
        let at = slugs.iter().position(|slug| *slug == default).unwrap_or(0);
        let mut flow = Flow::new(FlowKind::Apply, wizard::apply_form(&slugs, at));
        flow.pending = true;
        let slug = slugs[at].clone();
        self.modals.push(Modal::Flow(Box::new(flow)));
        vec![Effect::LoadTemplate { slug }]
    }

    /// `e`: register a folder fastf did not create, or a whole base of them.
    fn open_register(&mut self) -> Vec<Effect> {
        let mut options = vec![wizard::NO_TEMPLATE.to_string()];
        options.extend(self.template_slugs());
        let mut flow = Flow::new(FlowKind::Register, register::register_form(&options));
        register::sync_visibility(&mut flow);
        self.modals.push(Modal::Flow(Box::new(flow)));
        Vec::new()
    }

    fn on_flow_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top() else {
            return Vec::new();
        };
        if flow.step == Step::Preview {
            return self.on_preview_key(key);
        }
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let event = flow.form.apply(&key);
        let rows = flow.form.rows();
        flow.form.clamp_viewport(rows.min(12));
        match event {
            FormEvent::Cancel => {
                let kind = flow.kind;
                self.modals.pop();
                self.info(kind.cancelled());
                Vec::new()
            }
            FormEvent::Submit => self.submit_flow(),
            FormEvent::Pick => self.open_field_picker(),
            FormEvent::Changed => self.on_form_changed(),
            // **A form does not fall through to the registry, and the preview
            // does.** The difference is that a form is a place you type into:
            // on a choice field every letter is `Ignored`, so handing those to
            // `lookup_and_run` would make `q` — `Close` in every context — throw
            // away a form somebody had filled in, with no question and no undo.
            // Esc is the form's own cancel and its key line says so. The
            // preview has nothing to type into, which is why `?` and `q` work
            // there.
            FormEvent::Moved | FormEvent::Ignored => Vec::new(),
        }
    }

    /// Keys on the preview half: Esc goes back to the answers (the app's Esc
    /// ladder — one step at a time, nothing typed is lost), Enter commits.
    fn on_preview_key(&mut self, key: Key) -> Vec<Effect> {
        let delta: isize = match key.code {
            KeyCode::Esc => {
                if let Some(Modal::Flow(flow)) = self.modals.top_mut() {
                    flow.step = Step::Form;
                    flow.scroll = 0;
                }
                return Vec::new();
            }
            KeyCode::Enter => return self.commit_flow(),
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 10,
            KeyCode::PageUp => -10,
            KeyCode::Home => isize::MIN / 2,
            KeyCode::End => isize::MAX / 2,
            // Anything the preview does not consume is whatever the registry
            // binds where the keys are: `?` for the help, `q` to close. There
            // is nothing to type into on this step, so swallowing them said
            // nothing and did nothing.
            _ => return self.lookup_and_run(key),
        };
        // Clamped **here**, against the geometry `view` draws with. Only the
        // view clamped before, so ten PgDns over a short preview drove `scroll`
        // to 100 with nothing moving on screen, and the next ten PgUps did
        // nothing either — a dialog that reads as frozen. `layout.rs`'s whole
        // job is that a cursor cannot leave the drawn window.
        let max = match self.modals.top() {
            Some(Modal::Flow(flow)) => {
                crate::tui::view::modals::preview_max_scroll(self, flow) as isize
            }
            _ => 0,
        };
        if let Some(Modal::Flow(flow)) = self.modals.top_mut() {
            flow.scroll = (flow.scroll as isize + delta).clamp(0, max) as usize;
        }
        Vec::new()
    }

    /// A value changed: the template field decides which variables are asked
    /// for, and register's scope decides which questions apply at all.
    fn on_form_changed(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if flow.kind == FlowKind::Register {
            register::sync_visibility(flow);
        }
        let focused = flow.form.focused().map(|field| field.key.clone());
        if focused.as_deref() != Some(wizard::FIELD_TEMPLATE) {
            return Vec::new();
        }
        self.load_flow_template()
    }

    /// Read the template the form now names, and rebuild its variable fields.
    fn load_flow_template(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match flow.template_slug() {
            Some(slug) => {
                flow.pending = true;
                vec![Effect::LoadTemplate { slug }]
            }
            None => {
                flow.pending = false;
                flow.set_template(None);
                Vec::new()
            }
        }
    }

    fn on_template_loaded(
        &mut self,
        slug: &str,
        result: Result<Box<data::TemplateInfo>, String>,
    ) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        // A slower read for a template the form has already moved off is an
        // answer to a question nobody is asking any more.
        if flow.template_slug().as_deref() != Some(slug) {
            return Vec::new();
        }
        flow.pending = false;
        match result {
            Ok(info) => {
                flow.set_template(Some(*info));
                if flow.kind == FlowKind::Register {
                    register::sync_visibility(flow);
                }
            }
            Err(error) => {
                flow.set_template(None);
                flow.form.fail(Some(wizard::FIELD_TEMPLATE), error);
            }
        }
        Vec::new()
    }

    fn on_previewed(&mut self, preview: wizard::Preview) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        flow.pending = false;
        flow.preview = Some(preview);
        // `confirm_create = false` is a standing answer to the question the
        // preview asks, so it is not asked: the plan was still built, by the
        // same code path, and every refusal it can produce still lands on the
        // field that caused it.
        if flow.auto_commit {
            return self.commit_flow();
        }
        flow.step = Step::Preview;
        flow.scroll = 0;
        Vec::new()
    }

    /// Space on a choice: the same options as a fuzzy-filtered picker.
    fn open_field_picker(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top() else {
            return Vec::new();
        };
        let Some(field) = flow.form.focused() else {
            return Vec::new();
        };
        let crate::tui::widgets::form::FieldKind::Choice { options, .. } = &field.kind else {
            return Vec::new();
        };
        let describe = field.key == wizard::FIELD_TEMPLATE;
        let cards = self.summary.as_ref().map(|s| s.templates.clone());
        let items: Vec<PickItem> = options
            .iter()
            .map(|option| PickItem {
                label: option.clone(),
                detail: if describe {
                    cards
                        .as_ref()
                        .and_then(|cards| cards.iter().find(|card| &card.slug == option))
                        .map(|card| card.description.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                },
                value: option.clone(),
            })
            .collect();
        let title = field.label.clone();
        let key = field.key.clone();
        self.modals.push(Modal::Pick(PickState::new(
            title,
            items,
            Then::FormField(key),
        )));
        Vec::new()
    }

    /// Enter on the form: check what `update` can check, then ask a worker for
    /// the preview — which is where a path that does not exist is refused,
    /// because looking is I/O and `update` does none.
    fn submit_flow(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if flow.pending {
            return Vec::new();
        }
        if let Some((key, message)) = flow.missing_required() {
            flow.form.fail(Some(&key), message);
            return Vec::new();
        }
        let Some(request) = self.flow_request() else {
            return Vec::new();
        };
        if let Some(Modal::Flow(flow)) = self.modals.top_mut() {
            flow.pending = true;
            flow.form.clear_errors();
        }
        vec![Effect::Preview(Box::new(request))]
    }

    /// The open flow's answers, as the request both the preview and the commit
    /// are built from.
    fn flow_request(&self) -> Option<Request> {
        let Some(Modal::Flow(flow)) = self.modals.top() else {
            return None;
        };
        match flow.kind {
            FlowKind::Create => Some(Request::Create(CreateRequest {
                template_slug: flow.template_slug()?,
                vars: flow.variables(),
                base_dir_override: self.chosen_base(flow),
            })),
            FlowKind::Apply => Some(Request::Apply(ApplyRequest {
                template_slug: flow.template_slug()?,
                target: PathBuf::from(flow.form.value(wizard::FIELD_TARGET).trim()),
                vars: flow.variables(),
            })),
            FlowKind::Register => Some(Request::Register(register::request(flow))),
            FlowKind::FromFolder => {
                Some(Request::FromFolder(crate::tui::effect::FromFolderRequest {
                    source: PathBuf::from(flow.form.value(wizard::FIELD_SOURCE).trim()),
                    slug: flow.form.value(wizard::FIELD_SLUG).trim().to_string(),
                    force: flow.form.is_on(wizard::FIELD_FORCE),
                    bundle_assets: flow.form.is_on(wizard::FIELD_BUNDLE),
                }))
            }
        }
    }

    /// The base the create form names, or `None` for the configured default —
    /// the same distinction `pick_base_interactively` drew by returning early
    /// when there was only one base to offer.
    fn chosen_base(&self, flow: &Flow) -> Option<String> {
        let chosen = flow.form.value(wizard::FIELD_BASE);
        if chosen.is_empty() {
            return None;
        }
        let default = self.base_options().into_iter().next();
        (Some(&chosen) != default.as_ref()).then_some(chosen)
    }

    /// Enter on the preview: run it.
    fn commit_flow(&mut self) -> Vec<Effect> {
        let Some(request) = self.flow_request() else {
            return Vec::new();
        };
        self.modals.pop();
        match request {
            Request::Create(request) => {
                self.run_action("creating…", Action::Create(Box::new(request)))
            }
            Request::Apply(request) => {
                self.run_action("applying…", Action::Apply(Box::new(request)))
            }
            Request::Register(request) => {
                self.run_action("registering…", Action::Register(Box::new(request)))
            }
            Request::FromFolder(request) => self.run_action(
                "generating the template…",
                Action::TemplateFromFolder(Box::new(request)),
            ),
        }
    }

    // --- the templates tab ------------------------------------------------

    /// `T`: to the templates tab, and `T` back. The search bar belongs to
    /// whichever tab is showing, so switching drops the query with it — the
    /// two searches are over different things and carrying one across would
    /// hide most of the other list for no reason anyone typed.
    fn toggle_templates(&mut self) -> Vec<Effect> {
        self.screen = match self.screen {
            Screen::Library => Screen::Templates,
            Screen::Templates => Screen::Library,
        };
        self.search.editing = false;
        self.search.input.clear();
        self.search.query = Default::default();
        self.recompute();
        let mut effects = self.after_rows_changed();
        if self.screen == Screen::Templates {
            let rows = self.studio.rows("");
            self.studio.reselect(&rows);
            self.studio
                .clamp_viewport(&rows, layout::template_rows(self.area()));
            if self.studio.shown.is_none()
                && let Some(slug) = self.studio.selected_slug()
            {
                effects.push(Effect::LoadTemplateView { slug });
            }
            self.offer_guide_once();
        }
        effects
    }

    /// The templates tab's arrows, over the rows its own query keeps.
    /// The ends of the templates list. `Studio::jump` keeps the selection on
    /// a row the current filter shows, exactly as `step` does.
    fn jump_templates(&mut self, first: bool) -> Vec<Effect> {
        let rows = self.studio.rows(self.search.input.text());
        self.studio.jump(first, &rows);
        self.studio
            .clamp_viewport(&rows, layout::template_rows(self.area()));
        self.studio
            .selected_slug()
            .map(|slug| vec![Effect::LoadTemplateView { slug }])
            .unwrap_or_default()
    }

    fn step_templates(&mut self, delta: isize) -> Vec<Effect> {
        let rows = self.studio.rows(self.search.input.text());
        self.studio.step(delta, &rows);
        self.studio
            .clamp_viewport(&rows, layout::template_rows(self.area()));
        self.studio
            .selected_slug()
            .map(|slug| vec![Effect::LoadTemplateView { slug }])
            .unwrap_or_default()
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

    /// The arrows on whatever dialog is on top.
    fn step_top_modal(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        let actions_len = crate::tui::app::actions::action_entries(self).len();
        match self.modals.top_mut() {
            Some(Modal::Actions(actions)) => {
                actions.step(actions_len, delta);
                actions.clamp_viewport(
                    actions_len,
                    layout::list_rows(layout::actions_box(area, actions_len), 0),
                );
                Vec::new()
            }
            Some(Modal::MultiPick(pick)) => {
                pick.selected =
                    crate::tui::widgets::nav::step(Some(pick.selected), pick.items.len(), delta)
                        .unwrap_or(0);
                Vec::new()
            }
            Some(Modal::Builder(builder)) => {
                match &mut builder.open {
                    None => builder.step(delta),
                    Some(Open::Variables(list)) => {
                        let count = builder.template.variables.len();
                        list.selected =
                            crate::tui::widgets::nav::step(Some(list.selected), count, delta)
                                .unwrap_or(0);
                    }
                    Some(Open::Files(list)) => {
                        let count = builder.template.files.len();
                        list.selected =
                            crate::tui::widgets::nav::step(Some(list.selected), count, delta)
                                .unwrap_or(0);
                    }
                    Some(_) => {}
                }
                Vec::new()
            }
            Some(Modal::Settings(state)) => {
                state.step(delta);
                state.clamp_viewport(layout::settings_rows(area));
                Vec::new()
            }
            Some(Modal::Help { .. }) | Some(Modal::Message { .. }) => self.scroll_top_modal(delta),
            _ => Vec::new(),
        }
    }

    /// A page, or the ends (`isize::MIN`/`isize::MAX`), on whatever dialog
    /// is on top.
    fn page_top_modal(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        let actions_len = crate::tui::app::actions::action_entries(self).len();
        let jump = |selected: usize, len: usize| -> usize {
            crate::tui::widgets::nav::step(Some(selected), len, delta).unwrap_or(0)
        };
        match self.modals.top_mut() {
            Some(Modal::Actions(actions)) => {
                actions.selected = jump(actions.selected, actions_len);
                actions.clamp_viewport(
                    actions_len,
                    layout::list_rows(layout::actions_box(area, actions_len), 0),
                );
                Vec::new()
            }
            Some(Modal::MultiPick(pick)) => {
                pick.selected = jump(pick.selected, pick.items.len());
                Vec::new()
            }
            Some(Modal::Settings(state)) => {
                state.jump(delta);
                state.clamp_viewport(layout::settings_rows(area));
                Vec::new()
            }
            Some(Modal::Help { .. }) | Some(Modal::Message { .. }) | Some(Modal::Guide(_)) => {
                self.scroll_top_modal(delta)
            }
            _ => Vec::new(),
        }
    }

    /// Enter on the builder: open the highlighted section (or save, or
    /// discard) from the section list; edit the highlighted entry on the
    /// variables and files lists.
    fn builder_open(&mut self) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match &mut builder.open {
            None => match builder.row() {
                Row::Section(section) => {
                    builder.open_section(section);
                    Vec::new()
                }
                Row::Save => self.save_template(),
                // Discard asks the same question Esc does, through the same
                // ladder — the row is the spelled-out spelling of the key.
                Row::Discard => self.close_top(),
            },
            Some(Open::Variables(list)) => {
                if let Some(variable) = builder.template.variables.get(list.selected) {
                    list.editing = Some((list.selected, studio::variable_form(Some(variable))));
                }
                Vec::new()
            }
            Some(Open::Files(list)) => {
                if let Some(file) = builder.template.files.get(list.selected) {
                    let body = if file.template.is_empty() {
                        file.content.clone()
                    } else {
                        file.template.clone()
                    };
                    list.editing = Some(studio::FileEdit {
                        index: list.selected,
                        path: crate::tui::widgets::input::LineEdit::with_text(file.path.clone()),
                        body: crate::tui::widgets::text_area::TextArea::with_text(&body),
                        in_body: false,
                        error: None,
                    });
                }
                Vec::new()
            }
            Some(_) => Vec::new(),
        }
    }

    /// `a` on the variables or files list: a new entry at the end.
    fn builder_add(&mut self) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match &mut builder.open {
            Some(Open::Variables(list)) => {
                let count = builder.template.variables.len();
                list.editing = Some((count, studio::variable_form(None)));
            }
            Some(Open::Files(list)) => {
                list.editing = Some(studio::FileEdit {
                    index: builder.template.files.len(),
                    path: crate::tui::widgets::input::LineEdit::new(),
                    body: crate::tui::widgets::text_area::TextArea::new(),
                    in_body: false,
                    error: None,
                });
            }
            _ => {}
        }
        Vec::new()
    }

    /// `d` on the variables or files list: the highlighted entry goes.
    fn builder_remove(&mut self) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match &mut builder.open {
            Some(Open::Variables(list)) => {
                let count = builder.template.variables.len();
                if list.selected < count {
                    builder.template.variables.remove(list.selected);
                    list.selected = list.selected.min(count.saturating_sub(2));
                    builder.error = None;
                }
            }
            Some(Open::Files(list)) => {
                let count = builder.template.files.len();
                if list.selected < count {
                    builder.template.files.remove(list.selected);
                    list.selected = list.selected.min(count.saturating_sub(2));
                    builder.error = None;
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// `K`/`J` on the variables list: reorder in place, which is what
    /// `prompt::sort` was for. Moving a row is one keystroke and shows the
    /// result immediately.
    fn builder_move(&mut self, up: bool) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let count = builder.template.variables.len();
        if let Some(Open::Variables(list)) = &mut builder.open {
            let at = list.selected;
            let other = if up {
                at.checked_sub(1)
            } else {
                (at + 1 < count).then_some(at + 1)
            };
            if let Some(other) = other {
                builder.template.variables.swap(at, other);
                list.selected = other;
            }
        }
        Vec::new()
    }

    /// Enter on the settings list. A yes/no and a two-way choice are written
    /// where they stand: opening a dialog to answer a question with two
    /// answers is a keystroke spent on nothing. A maintenance row runs;
    /// anything else opens on its own line.
    fn settings_change(&mut self) -> Vec<Effect> {
        let Some(Modal::Settings(state)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if let Some((key, value)) = state.immediate_write() {
            return self.write_setting(key, value);
        }
        if let Some(Kind::Run(job)) = state.row().map(|row| row.kind.clone()) {
            return self.run_job(job);
        }
        state.begin_edit();
        Vec::new()
    }

    /// Put the guide up, at `page`, and remember that it has been seen.
    ///
    /// Every door goes through here — the key, the palette and both automatic
    /// offers — so the flag is set wherever the guide is actually read, and a
    /// person who found it themselves is never offered it afterwards.
    fn open_guide(&mut self, page: usize) -> Vec<Effect> {
        self.guide_seen = true;
        self.modals
            .push(Modal::Guide(Box::new(GuideState::at(page))));
        Vec::new()
    }

    /// The guide, offered once, the first time templates come up at all.
    ///
    /// It goes **on top of** whatever asked for it rather than instead of it,
    /// so Esc leaves the reader exactly where they were going — on the tab, or
    /// in the editor with an untouched template under the dialog.
    fn offer_guide_once(&mut self) {
        if !self.guide_seen {
            self.open_guide(0);
        }
    }

    /// New (`slug` is `None`) or edit: the builder over a scratch template.
    fn open_builder(&mut self, slug: Option<String>) -> Vec<Effect> {
        let effects = self.open_builder_inner(slug);
        self.offer_guide_once();
        effects
    }

    fn open_builder_inner(&mut self, slug: Option<String>) -> Vec<Effect> {
        match slug {
            Some(slug) => {
                let mut builder = Builder::new(None);
                builder.pending = Some(slug.clone());
                self.modals.push(Modal::Builder(Box::new(builder)));
                vec![Effect::LoadTemplateSource { slug }]
            }
            None => {
                self.modals
                    .push(Modal::Builder(Box::new(Builder::new(None))));
                Vec::new()
            }
        }
    }

    // --- the builder ------------------------------------------------------

    fn on_builder_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if builder.pending.is_some() {
            if key.code == KeyCode::Esc {
                self.modals.pop();
            }
            return Vec::new();
        }
        // A save is in flight: it is one worker call away from an answer, and
        // every key until then would act on a template that may be about to
        // close. Esc included — cancelling a write already sent is not a
        // thing this can promise.
        if builder.saving {
            return Vec::new();
        }
        match &mut builder.open {
            None => self.on_builder_list_key(key),
            Some(Open::Metadata(_)) | Some(Open::Id(_)) => self.on_builder_form_key(key),
            Some(Open::Variables(_)) => self.on_variables_key(key),
            Some(Open::Structure(_)) => self.on_structure_key(key),
            Some(Open::Files(_)) => self.on_files_key(key),
        }
    }

    /// The section list: enter a section, save, or discard — every key of it
    /// declared under `Context::Builder`.
    fn on_builder_list_key(&mut self, key: Key) -> Vec<Effect> {
        self.lookup_and_run(key)
    }

    /// Save, or say what `Template::validate` refused — the check that used to
    /// print `Cannot save:` and drop back into the same menu.
    /// Save, or say what refused it.
    ///
    /// **The builder stays up until the write has actually landed.** It used
    /// to be popped the moment the effect was handed over, so a refusal from
    /// under the data lock — an occupied slug, a lock held by another
    /// terminal, a full disk — arrived with nothing to land on: the template
    /// was gone, and all that was left was one red line on the status bar.
    /// Everything typed into it was unrecoverable. `Modal::Builder` is popped
    /// in `on_action_done` now, on the success path only.
    fn save_template(&mut self) -> Vec<Effect> {
        // Read what the save needs first, so the borrow ends before the app
        // itself is needed.
        let (template, original_slug) = {
            let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
                return Vec::new();
            };
            if builder.saving {
                return Vec::new();
            }
            if let Err(error) = builder.template.validate() {
                builder.error = Some(format!("Cannot save: {error:#}"));
                return Vec::new();
            }
            (builder.template.clone(), builder.original_slug.clone())
        };

        // The slug must be free unless this very template was loaded from it.
        // `operations::save_template` enforces that under the lock and is the
        // authority; this is the same question asked of the cards already in
        // memory, so the answer lands on the list instead of arriving from a
        // worker. `update` reads no disk to do it.
        let occupied = self.studio.cards.iter().any(|card| {
            card.on_disk
                && card.slug == template.slug
                && original_slug.as_deref() != Some(card.slug.as_str())
        });
        if occupied {
            if let Some(Modal::Builder(builder)) = self.modals.top_mut() {
                builder.error = Some(format!(
                    "Cannot save: a template called '{}' already exists — \
                     give this one another slug, or edit that one instead",
                    template.slug
                ));
            }
            return Vec::new();
        }

        let effects = self.run_action(
            "saving the template…",
            Action::SaveTemplate {
                template: Box::new(template),
                original_slug,
            },
        );
        // `run_action` refuses while another one is running; only a save that
        // actually started may put the builder into its saving state.
        if !effects.is_empty()
            && let Some(Modal::Builder(builder)) = self.modals.top_mut()
        {
            builder.saving = true;
            builder.error = None;
        }
        effects
    }

    /// The metadata and ID sections: a form, checked here because every rule
    /// they enforce is a rule about the text and not about a disk.
    fn on_builder_form_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let is_metadata = matches!(builder.open, Some(Open::Metadata(_)));
        // Read off the template before the form is borrowed from the same
        // builder: the naming-pattern advice is a question about the
        // variables, and the form is a field of the thing that holds them.
        let declared: Vec<String> = builder
            .template
            .variables
            .iter()
            .map(|v| v.slug.clone())
            .collect();
        let (Some(Open::Metadata(form)) | Some(Open::Id(form))) = &mut builder.open else {
            return Vec::new();
        };
        match form.apply(&key) {
            FormEvent::Cancel => builder.open = None,
            FormEvent::Submit => {
                let refusal = if is_metadata {
                    studio::check_metadata(form)
                } else {
                    studio::check_id(form)
                };
                if let Some((field, message)) = refusal {
                    form.fail(Some(field), message);
                    return Vec::new();
                }
                let form = form.clone();
                if is_metadata {
                    builder.commit_metadata(&form);
                } else {
                    builder.commit_id(&form);
                }
                builder.open = None;
                builder.error = None;
            }
            // The slug follows the name until one is typed, and the naming
            // pattern says on this keystroke what it would do with the
            // variables this template declares.
            FormEvent::Changed if is_metadata => {
                let declared: Vec<&str> = declared.iter().map(String::as_str).collect();
                studio::sync_metadata_form(form, &declared);
            }
            _ => {}
        }
        Vec::new()
    }

    fn on_variables_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let count = builder.template.variables.len();
        let Some(Open::Variables(list)) = &mut builder.open else {
            return Vec::new();
        };
        // Editing one variable: the form owns every key until it answers.
        if let Some((index, form)) = &mut list.editing {
            match form.apply(&key) {
                FormEvent::Cancel => list.editing = None,
                FormEvent::Changed => studio::sync_variable_form(form, self.theme.glyphs),
                FormEvent::Submit => match studio::variable_from(form) {
                    Ok(variable) => {
                        let index = *index;
                        list.editing = None;
                        if index < count {
                            builder.template.variables[index] = variable;
                        } else {
                            builder.template.variables.push(variable);
                            list.selected = count;
                        }
                        builder.error = None;
                    }
                    Err((field, message)) => form.fail(Some(field), message),
                },
                _ => {}
            }
            return Vec::new();
        }
        // The list itself: every key it answers is in the registry.
        self.lookup_and_run(key)
    }

    /// The structure section: one folder path per line, the tree drawn beside
    /// it as it is typed. Enter is a newline here, so Ctrl-S commits.
    fn on_structure_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let Some(Open::Structure(area)) = &mut builder.open else {
            return Vec::new();
        };
        match (key.code, key.ctrl) {
            (KeyCode::Esc, false) => builder.open = None,
            (KeyCode::Char('s'), true) => {
                let area = area.clone();
                builder.commit_structure(&area);
                builder.open = None;
                builder.error = None;
            }
            _ => {
                area.apply(&key);
            }
        }
        Vec::new()
    }

    fn on_files_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let count = builder.template.files.len();
        let Some(Open::Files(list)) = &mut builder.open else {
            return Vec::new();
        };
        if let Some(edit) = &mut list.editing {
            match (key.code, key.ctrl) {
                (KeyCode::Esc, false) => list.editing = None,
                (KeyCode::Tab, false) | (KeyCode::BackTab, false) => edit.in_body = !edit.in_body,
                (KeyCode::Char('s'), true) => match studio::file_from(edit) {
                    Ok(entry) => {
                        let index = edit.index;
                        list.editing = None;
                        if index < count {
                            builder.template.files[index] = entry;
                        } else {
                            builder.template.files.push(entry);
                            list.selected = count;
                        }
                        builder.error = None;
                    }
                    Err(message) => edit.error = Some(message),
                },
                _ => {
                    if edit.in_body {
                        edit.body.apply(&key);
                    } else {
                        edit.path.apply(&key);
                    }
                    edit.error = None;
                }
            }
            return Vec::new();
        }
        // The list itself: every key it answers is in the registry.
        self.lookup_and_run(key)
    }

    // --- settings, the counter, maintenance ------------------------------

    /// `,`: every setting on one screen. The screen goes up at once, saying
    /// it is reading, and the rows are filled in when the read lands — so the
    /// key is seen to have worked, and no value shown is stale.
    fn open_settings(&mut self) -> Vec<Effect> {
        self.modals
            .push(Modal::Settings(Box::new(SettingsState::pending())));
        vec![Effect::LoadSettings]
    }

    fn on_settings_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Settings(state)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if state.editing.is_some() {
            return self.on_settings_edit_key(key);
        }
        // The list itself: every key it answers is in the registry.
        self.lookup_and_run(key)
    }

    fn on_settings_edit_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Settings(state)) = self.modals.top_mut() else {
            return Vec::new();
        };
        // Esc leaves the value alone, which is what "Esc in a settings field →
        // the value unchanged" has always meant. On the filter it means the
        // whole screen back, since a filter left behind is a screen missing
        // rows for a reason nobody can see.
        if key.code == KeyCode::Esc && !key.ctrl {
            if matches!(state.editing, Some(Editing::Filter)) {
                state.filter.clear();
                state.apply_filter();
            }
            state.editing = None;
            return Vec::new();
        }
        let commit = match &mut state.editing {
            // Enter keeps the filter and hands the keys back to the list; the
            // rows stay narrowed, and the title says so.
            Some(Editing::Filter) => {
                if key.code == KeyCode::Enter {
                    state.editing = None;
                } else if state.filter.apply(&key) {
                    state.apply_filter();
                }
                false
            }
            Some(Editing::Value { input, error, .. }) => {
                if key.code == KeyCode::Enter {
                    true
                } else {
                    if input.apply(&key) {
                        *error = None;
                    }
                    false
                }
            }
            // A list is a document: Enter is a newline, so Ctrl-S commits.
            Some(Editing::Bases { area, error }) => {
                if key.code == KeyCode::Char('s') && key.ctrl {
                    true
                } else {
                    if area.apply(&key) {
                        *error = None;
                    }
                    false
                }
            }
            None => false,
        };
        if !commit {
            return Vec::new();
        }
        let Some((key, value)) = state.pending_write() else {
            return Vec::new();
        };
        self.write_setting(key, value)
    }

    fn write_setting(&mut self, key: &'static str, value: String) -> Vec<Effect> {
        self.run_action("saving…", Action::SetConfig { key, value })
    }

    /// One of the settings screen's verbs.
    fn run_job(&mut self, job: settings::Job) -> Vec<Effect> {
        match job {
            settings::Job::RaiseCounter => {
                let floor = match self.modals.top() {
                    Some(Modal::Settings(state)) => state.settings.counter_floor,
                    _ => 0,
                };
                let mut prompt = TextPrompt::new(
                    validators::raise_counter_prompt(floor),
                    TextThen::RaiseCounter,
                );
                prompt.input = crate::tui::widgets::input::LineEdit::with_text(floor.to_string());
                self.modals.push(Modal::TextPrompt(prompt));
                Vec::new()
            }
            settings::Job::SyncCounters => self.run_action(job.busy(), Action::SyncCounters),
            settings::Job::Reindex => self.run_action(job.busy(), Action::Reindex),
            settings::Job::Reconcile => self.run_action(job.busy(), Action::Reconcile),
            settings::Job::DataLocations => self.load_view(
                "data locations".to_string(),
                PathBuf::new(),
                ViewKind::DataLocations,
            ),
        }
    }

    // --- first run --------------------------------------------------------

    /// Ask where projects should live, before the first frame.
    ///
    /// The old flow asked on the main screen before the app opened, because
    /// there was no app to ask in. This is a modal over the dashboard: the
    /// suggestion is editable, Enter creates the folder and records it, and an
    /// empty answer skips — the question returns next launch until a base is
    /// set.
    pub fn request_onboarding(&mut self, suggested: String) {
        self.modals
            .push(Modal::Onboarding(Onboarding::new(suggested)));
    }

    fn on_onboarding_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Prompt, key, self)
        {
            return self.run(id);
        }
        if let Some(Modal::Onboarding(state)) = self.modals.top_mut()
            && state.input.apply(&key)
        {
            state.error = None;
        }
        Vec::new()
    }

    /// Enter on the first-run question.
    fn submit_onboarding(&mut self) -> Vec<Effect> {
        let Some(Modal::Onboarding(state)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let answer = state.input.text().trim().to_string();
        if answer.is_empty() {
            self.modals.pop();
            self.info(validators::ONBOARDING_SKIPPED);
            return Vec::new();
        }
        // The dialog stays up until the folder exists: a path that cannot be
        // created is refused here, with the text still on the line, rather
        // than dropping a first-time user onto an empty dashboard with an
        // error and no question.
        state.pending = true;
        state.error = None;
        self.run_action("creating the base…", Action::InitBaseDir(answer))
    }

    /// Up/Down and the page keys on the templates tab: the card list, or the
    /// pane beside it when that is what Tab has the focus on.
    ///
    /// `Studio::scroll` existed, was clamped by the view, and was set to zero
    /// in four places and raised in none — so a template whose `template show`
    /// output was taller than the pane had its tail permanently unreachable,
    /// which on an 80×24 window is anything past about fifteen lines. Most
    /// real templates are longer than that.
    fn step_templates_or_pane(&mut self, delta: isize) -> Vec<Effect> {
        if self.focus == Focus::Detail {
            self.studio.scroll = self
                .studio
                .scroll
                .saturating_add_signed(delta)
                .min(self.studio_scroll_max());
            return Vec::new();
        }
        self.step_templates(delta)
    }

    /// The last row the pane can be scrolled to, from the geometry `view`
    /// draws with — so the cursor cannot leave the drawn window.
    ///
    /// **The templates tab's own split**, not the library's: it measured
    /// `regions().detail` before, which is the library pane — closed under a
    /// hundred columns while the template pane is always drawn — so on a
    /// narrow window Tab could not reach a pane that was right there.
    fn studio_scroll_max(&self) -> usize {
        let (_, pane) = layout::templates_panes(layout::templates_body(&self.regions()));
        let rows = pane.height.saturating_sub(2) as usize;
        self.studio.lines.len().saturating_sub(rows)
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

    /// `A`: pick a tag the library already uses, or type a new one. Over
    /// marks the list is every known tag not already on all of them, and the
    /// answer is asked once.
    fn open_add_tag(&mut self) -> Vec<Effect> {
        let targets = self.library.targets();
        if targets.is_empty() {
            return Vec::new();
        }
        let available: Vec<String> = self
            .library
            .known_tags
            .iter()
            .filter(|tag| !targets.iter().all(|project| project.tags.contains(tag)))
            .cloned()
            .collect();
        let title = if targets.len() > 1 {
            format!("Tag to add to {} projects", targets.len())
        } else {
            "Tag to add".to_string()
        };
        if available.is_empty() {
            self.modals.push(Modal::TextPrompt(TextPrompt::new(
                validators::ADD_TAG_PROMPT,
                TextThen::AddTag,
            )));
            return Vec::new();
        }

        let mut items: Vec<PickItem> = available
            .into_iter()
            .map(|tag| PickItem {
                label: tag.clone(),
                detail: String::new(),
                value: tag,
            })
            .collect();
        items.push(PickItem {
            label: crate::tui::app::actions::NEW_TAG.to_string(),
            detail: String::new(),
            value: crate::tui::app::actions::NEW_TAG.to_string(),
        });
        self.modals
            .push(Modal::Pick(PickState::new(title, items, Then::AddTag)));
        Vec::new()
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

    /// `m`: pick the mounted base to move into, then move.
    fn open_move_picker(&mut self) -> Vec<Effect> {
        let targets = crate::tui::app::actions::move_targets(self);
        if targets.is_empty() {
            return Vec::new();
        }
        let items: Vec<PickItem> = targets
            .into_iter()
            .map(|path| PickItem {
                label: crate::core::library::base_label(&path),
                detail: String::new(),
                value: path.display().to_string(),
            })
            .collect();
        self.modals.push(Modal::Pick(PickState::new(
            "Move to which base?",
            items,
            Then::MoveToBase,
        )));
        Vec::new()
    }

    /// A move as a one-item job, with the progress modal up while it runs.
    fn run_move(&mut self, target: PathBuf) -> Vec<Effect> {
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        self.move_progress = Some(Progress::new(&[]));
        self.run_action(
            "moving…",
            Action::Move {
                project: Box::new(project),
                target,
            },
        )
    }

    // --- batch jobs -------------------------------------------------------

    /// Run the verb over every marked project, one item at a time.
    fn start_job(&mut self, kind: jobs::JobKind, target: Option<PathBuf>) -> Vec<Effect> {
        let targets = self.library.targets();
        if targets.is_empty() {
            return Vec::new();
        }
        self.job = Some(jobs::Job::new(kind, targets, target));
        self.job_advance()
    }

    /// Begin the next item of the running job. When nothing is left — every
    /// item ran, or the job was cancelled — finish it.
    fn job_advance(&mut self) -> Vec<Effect> {
        let kind = match self.job.as_ref() {
            Some(job) => job.kind.clone(),
            None => return Vec::new(),
        };
        let Some(project) = self.job.as_mut().and_then(|job| job.begin_next().cloned()) else {
            return self.job_finish();
        };
        // The progress modal is for a move that is actually running: arming it
        // here — after an item began — means the final advance, which only
        // finishes the job, cannot leave a stale modal behind for every later
        // quit gesture to read as "a move is running".
        if kind == jobs::JobKind::Move {
            self.move_progress = Some(Progress::new(&[]));
        }
        let action = {
            let job = self.job.as_ref().expect("the job is running");
            job.action_for(&project)
        };
        self.run_action(kind.busy(), action)
    }

    /// One item's outcome landed: record it, patch the row, and move on.
    fn on_job_item_done(&mut self, outcome: Result<Box<ActionOutcome>, String>) -> Vec<Effect> {
        // The item that was running leaves `inflight`, whatever happened. Its
        // path is what the mark is keyed by.
        let finished = self.job.as_mut().and_then(|job| job.take_inflight());
        let (id, path) = match finished {
            Some(project) => (project.id, Some(project.path)),
            None => ("?".to_string(), None),
        };
        let mut effects = Vec::new();
        match outcome {
            Ok(outcome) => {
                let outcome = *outcome;
                if let Some(entry) = outcome.session {
                    crate::tui::frame::record(entry);
                    self.session = crate::tui::frame::recent_actions();
                }
                if let Some(warning) = outcome.warning
                    && let Some(job) = &mut self.job
                {
                    job.warnings.push(warning);
                }
                // **The effects a change asks for are the job's too.** They
                // were dropped here, where the single-action path returns them,
                // and `apply_change`'s `Reload` arm calls `discover`, which
                // sets `library.inflight` *before* handing back the effect that
                // would answer it. A dropped one left the app waiting on a
                // generation nothing would ever send, after which every patch
                // only set `dirty` and the list stopped changing: a batch
                // re-derive of tags rewrote every file and showed nothing, and
                // the list stayed frozen for the rest of the session.
                effects.extend(self.apply_change(outcome.change));
                // A mark is the retry list. An item that succeeded is not on
                // it any more, so "3 tagged" and the ✓ glyphs left on screen
                // cannot disagree — `jobs.rs` has always said so; nothing did
                // it, because `patch` only drops a mark when the path moved.
                if let Some(path) = &path {
                    self.library.marks.remove(path);
                }
                if let Some(job) = &mut self.job {
                    job.done += 1;
                }
            }
            Err(error) => {
                // A cancellation the user asked for is not a failure to list:
                // the report says how many were left instead. Either way the
                // mark stays: it is what a retry would act on.
                let cancelled = self.job.as_ref().is_some_and(|job| job.cancelled);
                if !cancelled && let Some(job) = &mut self.job {
                    job.failed.push((id, error));
                }
            }
        }
        effects.extend(self.job_advance());
        effects
    }

    /// The job has no items left to begin: report and clear it.
    fn job_finish(&mut self) -> Vec<Effect> {
        let Some(job) = self.job.take() else {
            return Vec::new();
        };
        self.move_progress = None;
        let mut headline = job.kind.done(job.done);
        if !job.failed.is_empty() {
            headline.push_str(&format!(", {} failed", job.failed.len()));
        }
        if job.cancelled {
            headline.push_str(" — cancelled");
        }
        if let Some((title, body)) = job.report() {
            // The rows the report names are the rows that still hold a mark,
            // so Esc closes the report straight back onto a consistent list.
            let level = if job.failed.is_empty() {
                MessageLevel::Warn
            } else {
                MessageLevel::Error
            };
            self.modals.push(Modal::message(title, body, level));
        }
        if job.failed.is_empty() && !job.cancelled {
            self.good(headline);
        } else {
            self.warn(headline);
        }
        Vec::new()
    }

    /// Stop after the current item: the in-flight move is told to cancel, and
    /// the job marks itself as cancelled so the rest stay marked. A bare
    /// single move (no job) just cancels at the runtime.
    fn request_cancel(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.move_progress.is_some() {
            effects.push(Effect::CancelMove);
        }
        match &mut self.job {
            Some(job) => job.cancelled = true,
            None => return effects,
        }
        // Between items nothing is in flight: finish now. Otherwise the
        // current item's ActionDone finishes the job when it lands.
        if self.busy.is_none() {
            effects.extend(self.job_finish());
        }
        effects
    }

    /// `M`/`J`: read the full metadata or journal on a worker, then show it.
    fn open_view(&mut self, id: CommandId) -> Vec<Effect> {
        let Some(project) = self.library.selected() else {
            return Vec::new();
        };
        let kind = if id == CommandId::ShowMetadata {
            ViewKind::Metadata
        } else {
            ViewKind::Journal
        };
        let title = format!(
            "{} · {}",
            project.id,
            if kind == ViewKind::Metadata {
                "metadata"
            } else {
                "notes"
            }
        );
        self.load_view(title, project.path.clone(), kind)
    }

    /// A read-only view: the dialog goes up at once saying it is reading, so
    /// the key is seen to have worked on a slow disk, and the worker's answer
    /// fills it in (`Msg::ViewLoaded`) if it is still the one on top.
    fn load_view(&mut self, title: String, path: PathBuf, kind: ViewKind) -> Vec<Effect> {
        self.modals.push(Modal::message(
            title.clone(),
            "reading…",
            MessageLevel::Info,
        ));
        vec![Effect::LoadView { title, path, kind }]
    }
}

/// Whether `(column, row)` lands inside `area`.
fn inside(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.x + area.width && row >= area.y && row < area.y + area.height
}

/// The state machine: the app and one message in, the effects out.
pub fn update(app: &mut App, msg: Msg) -> Vec<Effect> {
    app.handle(msg)
}
