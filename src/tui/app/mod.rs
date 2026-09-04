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
pub mod register;
pub mod search;
pub mod settings;
pub mod studio;
pub mod wizard;

use std::collections::HashMap;
use std::path::PathBuf;

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
use crate::tui::msg::{Mouse, MouseKind, Msg, Resumed};
use crate::tui::theme::Theme;
use crate::tui::validators;
use crate::tui::widgets::form::FormEvent;
use crate::util::diag::Level;
use crate::util::size_scan::SizeCell;
use data::{Prefs, ProjectDetail, Summary, TemplateCard};
use library::{LibraryState, Order};
use modal::{MessageLevel, Modal, ModalStack, PickItem, PickState, Then};
use palette::{PaletteState, PaletteTarget};
use search::SearchState;
use settings::{Editing, Kind, Onboarding, SettingsState};
use studio::{Builder, Open, Row, Studio};
use wizard::{Flow, FlowKind, Step};

/// How long a status message stays, in ticks of 200 ms.
const STATUS_TICKS: u64 = 30;
/// How many project details the pane remembers.
const DETAIL_CACHE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Projects,
    Detail,
    Templates,
}

/// The template strip: every template known, with how many projects use it.
#[derive(Debug, Default)]
pub struct TemplatesState {
    pub cards: Vec<TemplateCard>,
    pub counts: HashMap<String, usize>,
    pub selected: usize,
    /// How many of the cards are templates on disk — what the header counts.
    pub on_disk: usize,
}

impl TemplatesState {
    /// Cards from the summary — the templates on disk, busiest first — then a
    /// bare card for any slug the projects still name that no template
    /// answers to, so the strip can filter by it and say what it is. The
    /// first card is always a real template, so the app never opens on
    /// `(registered)`.
    pub fn rebuild(&mut self, summary: Option<&Summary>, counts: HashMap<String, usize>) {
        // The cursor stays on its card across a rebuild — unless it was on an
        // orphan only because nothing on disk was known yet (discovery can
        // land before the summary), in which case the first real template is
        // where it belongs.
        let keep = self
            .selected_card()
            .filter(|card| card.on_disk || self.on_disk == 0 && summary.is_none())
            .map(|c| c.slug.clone());

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
        cards.sort_by(|a, b| {
            let ca = counts.get(&a.slug).copied().unwrap_or(0);
            let cb = counts.get(&b.slug).copied().unwrap_or(0);
            b.on_disk
                .cmp(&a.on_disk)
                .then_with(|| cb.cmp(&ca))
                .then_with(|| a.slug.cmp(&b.slug))
        });
        self.on_disk = cards.iter().filter(|c| c.on_disk).count();
        self.cards = cards;
        self.counts = counts;
        self.selected = keep
            .and_then(|slug| self.cards.iter().position(|c| c.slug == slug))
            .unwrap_or(0)
            .min(self.cards.len().saturating_sub(1));
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

    pub fn selected_card(&self) -> Option<&TemplateCard> {
        self.cards.get(self.selected)
    }

    pub fn count(&self, slug: &str) -> usize {
        self.counts.get(slug).copied().unwrap_or(0)
    }

    pub fn step(&mut self, delta: isize) {
        if let Some(next) =
            crate::tui::widgets::nav::wrap_step(Some(self.selected), self.cards.len(), delta)
        {
            self.selected = next;
        }
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
    pub focus: Focus,
    pub templates: TemplatesState,
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
    pub ticks: u64,
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
            focus: Focus::Projects,
            templates: TemplatesState::default(),
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
            ticks: 0,
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
        // before the first frame so the command lands where it was aimed.
        match self.studio_entry.take() {
            Some(crate::tui::entry::StudioEntry::List) => effects.extend(self.open_studio()),
            Some(crate::tui::entry::StudioEntry::New) => effects.extend(self.open_builder(None)),
            Some(crate::tui::entry::StudioEntry::Edit(slug)) => {
                effects.extend(self.open_builder(Some(slug)))
            }
            None => {}
        }
        if self.library.loaded {
            self.templates
                .rebuild(self.summary.as_ref(), self.library.per_template());
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
    /// cell, each followed by a space, inside the borders.
    pub fn table_min_width(&self) -> u16 {
        let (id_w, name_w) = self.library.widths;
        // `choose_columns` adds a column only while it fits with its spacing.
        (2 + 1 + id_w + 1 + name_w + 1 + crate::tui::rows::SIZE_CELL + 1).min(u16::MAX as usize)
            as u16
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
        self.focus_context()
    }

    fn focus_context(&self) -> Context {
        match self.focus {
            Focus::Projects => Context::Projects,
            Focus::Detail => Context::Detail,
            Focus::Templates => Context::Templates,
        }
    }

    /// Whether something on screen is moving, so the runtime should wake the
    /// app without input: a spinner, a toast about to expire, a size cell
    /// still being measured.
    pub fn needs_tick(&self) -> bool {
        self.busy.is_some()
            || self.status.expires_at.is_some()
            || (self.library.loaded && self.library.sizes_pending(self.rows_on_screen()))
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
            expires_at: Some(self.ticks + STATUS_TICKS),
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

    fn good(&mut self, text: impl Into<String>) {
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
        if self.focus == Focus::Detail && !self.detail_visible() {
            self.focus = Focus::Projects;
        }
        let rows = self.rows_on_screen();
        self.library.clamp_viewport(rows);
        self.detail_scroll = 0;

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
            && !self.details.contains_key(&project.path)
        {
            effects.push(Effect::LoadDetail(project.path.clone()));
        }
        effects
    }

    fn after_query_change(&mut self) -> Vec<Effect> {
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

    fn set_template_filter(&mut self, slug: Option<String>) -> Vec<Effect> {
        self.library.template_filter = slug;
        self.recompute();
        self.after_rows_changed()
    }

    fn apply_change(&mut self, change: ListChange) -> Vec<Effect> {
        let mut effects = Vec::new();
        match change {
            ListChange::Patched { project, stale } => {
                if !self.library.patch(*project) {
                    effects.push(self.discover());
                }
                for path in &stale {
                    self.library.sizes.remove(path);
                    self.details.remove(path);
                }
                effects.push(Effect::ForgetSizes(stale));
            }
            ListChange::Removed { path } => {
                self.library.remove(&path);
                self.details.remove(&path);
                effects.push(Effect::ForgetSizes(vec![path]));
            }
            ListChange::Reload => {
                effects.push(self.discover());
                effects.push(Effect::LoadSummary);
            }
            ListChange::SummaryOnly => effects.push(Effect::LoadSummary),
            ListChange::None => {}
        }
        self.recompute();
        self.templates
            .rebuild(self.summary.as_ref(), self.library.per_template());
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
                self.ticks += 1;
                if self.status.expires_at.is_some_and(|at| at <= self.ticks) {
                    self.status = Status::default();
                }
                Vec::new()
            }
            Msg::Sizes(cells) => {
                for (path, size) in cells {
                    self.library.sizes.insert(path, size);
                }
                if self.library.effective_sort(&self.search.query) == Order::Size {
                    self.recompute();
                    let rows = self.rows_on_screen();
                    self.library.clamp_viewport(rows);
                }
                Vec::new()
            }
            Msg::Summary(summary) => {
                self.summary = Some(*summary);
                self.summary_error = None;
                self.templates
                    .rebuild(self.summary.as_ref(), self.library.per_template());
                // A template written or deleted while the studio is open is a
                // change to the list it is showing.
                if let Some(Modal::Studio(studio)) = self.modals.top_mut() {
                    let cards = self
                        .summary
                        .as_ref()
                        .map(|summary| summary.templates.clone())
                        .unwrap_or_default();
                    let keep = studio.selected_slug();
                    studio.cards = cards;
                    studio.selected = keep
                        .and_then(|slug| studio.cards.iter().position(|card| card.slug == slug))
                        .unwrap_or(0)
                        .min(studio.cards.len().saturating_sub(1));
                    if studio.shown.as_deref() != studio.selected_slug().as_deref() {
                        studio.lines.clear();
                        studio.shown = None;
                    }
                    return studio
                        .selected_slug()
                        .map(|slug| vec![Effect::LoadTemplateView { slug }])
                        .unwrap_or_default();
                }
                Vec::new()
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
                self.templates
                    .rebuild(self.summary.as_ref(), self.library.per_template());
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
                let mut effects = self.after_rows_changed();

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
                        format!("{error}\n\nfix the configuration (`fastf config show`), then reload with F5."),
                        MessageLevel::Error,
                    ));
                }
                Vec::new()
            }
            Msg::Detail { path, detail } => {
                if self.details.len() >= DETAIL_CACHE {
                    self.details.clear();
                }
                self.details.insert(path, *detail);
                Vec::new()
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
                builder.pending = false;
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
                if let Some(Modal::Studio(studio)) = self.modals.top_mut()
                    && studio.selected_slug().as_deref() == Some(slug.as_str())
                {
                    studio.shown = Some(slug);
                    studio.lines = lines;
                    studio.scroll = 0;
                }
                Vec::new()
            }
            Msg::SettingsLoaded(loaded) => {
                // The screen went up when `,` was pressed, saying it was
                // reading; a read that lands after it was closed has nothing
                // to fill in and is dropped.
                let theme = loaded.theme.clone();
                if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                    state.refresh(*loaded);
                }
                // A theme written on this screen takes effect on the frame
                // that shows it was written.
                vec![Effect::Retheme(theme)]
            }
            Msg::Themed(theme) => {
                self.theme = *theme;
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
                let mut effects = self.apply_change(outcome.change);
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
                match self.modals.top_mut() {
                    Some(Modal::Settings(state)) if state.editing.is_some() => {
                        state.pending = false;
                        state.fail(error);
                    }
                    Some(Modal::Onboarding(state)) => {
                        state.pending = false;
                        state.error = Some(error);
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
                self.good(format!(
                    "{}  Opened {} in the file manager",
                    self.theme.glyphs.check, project.name
                ));
            }
            (SpawnKind::Reveal(_), Err(error)) => {
                self.error(format!("could not open the folder: {error}"))
            }
            (SpawnKind::Terminal(_), Ok(_)) => {
                self.good(format!("{}  Terminal opened", self.theme.glyphs.check));
            }
            (SpawnKind::Terminal(_), Err(error)) => {
                self.error(format!("could not open a terminal: {error}"));
            }
            (SpawnKind::Clipboard(_), Ok(tool)) => {
                self.good(format!("{}  Copied with {tool}", self.theme.glyphs.check));
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
                self.info("pasted text ignored — press / to search, or open a field first");
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
            self.focus = Focus::Projects;
            return Vec::new();
        }
        if let Some(detail) = regions.detail
            && inside(detail, column, row)
        {
            self.focus = Focus::Detail;
            return Vec::new();
        }
        if let Some(strip) = regions.strip
            && inside(strip, column, row)
        {
            self.focus = Focus::Templates;
            return Vec::new();
        }
        if !inside(regions.table, column, row) {
            return Vec::new();
        }
        self.focus = Focus::Projects;
        // The table's border and its header row: the first project sits two
        // rows below the top of the region.
        let Some(offset_row) = row.checked_sub(regions.table.y + 2) else {
            return Vec::new();
        };
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
            if self.job.is_some() || self.move_progress.is_some() {
                return self.request_cancel();
            }
            return vec![Effect::Quit(if key == Key::ctrl('c') {
                Exit::Interrupted
            } else {
                Exit::Normal
            })];
        }
        if key == Key::ctrl('c') {
            // A job or a move is running: Ctrl-C cancels it rather than
            // quitting under a worker that is still mutating the filesystem.
            if self.job.is_some() || self.move_progress.is_some() {
                return self.request_cancel();
            }
            return if self.modals.pop().is_some() {
                Vec::new()
            } else {
                vec![Effect::Quit(Exit::Interrupted)]
            };
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
        match command::lookup(self.context(), key, self) {
            Some(id) => self.run(id),
            None => Vec::new(),
        }
    }

    fn on_search_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Esc if !key.ctrl => {
                // The first Esc clears, the second leaves: a query can hide the
                // row the user was looking for, so clearing comes first.
                if self.search.input.is_empty() {
                    self.search.editing = false;
                    Vec::new()
                } else {
                    self.search.input.clear();
                    self.after_query_change()
                }
            }
            KeyCode::Enter => {
                self.search.editing = false;
                Vec::new()
            }
            KeyCode::Up | KeyCode::Down if !key.ctrl => {
                self.library
                    .step(if key.code == KeyCode::Down { 1 } else { -1 });
                self.after_selection_change()
            }
            KeyCode::PageUp | KeyCode::PageDown if !key.ctrl => {
                let rows = self.rows_on_screen() as isize;
                self.library.jump(if key.code == KeyCode::PageDown {
                    rows
                } else {
                    -rows
                });
                self.after_selection_change()
            }
            _ => {
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
            Some(Modal::Studio(_)) => self.on_studio_key(key),
            Some(Modal::Builder(_)) => self.on_builder_key(key),
            Some(Modal::Settings(_)) => self.on_settings_key(key),
            Some(Modal::Onboarding(_)) => self.on_onboarding_key(key),
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
        match key.code {
            KeyCode::Esc if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
            KeyCode::Enter => self.submit_text_prompt(),
            _ => {
                let changed = match self.modals.top_mut() {
                    Some(Modal::TextPrompt(prompt)) => prompt.input.apply(&key),
                    _ => false,
                };
                if changed && let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                    prompt.error = None;
                }
                Vec::new()
            }
        }
    }

    fn submit_text_prompt(&mut self) -> Vec<Effect> {
        let (text, then) = match self.modals.top() {
            Some(Modal::TextPrompt(prompt)) => {
                (prompt.input.text().to_string(), prompt.then.clone())
            }
            _ => return Vec::new(),
        };
        let project = self.library.selected().cloned();
        match then {
            TextThen::Rename => {
                if let Err(error) = validators::folder_name(&text) {
                    if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                        prompt.error = Some(error);
                    }
                    return Vec::new();
                }
                self.modals.pop();
                let Some(project) = project else {
                    return Vec::new();
                };
                self.run_action(
                    "renaming…",
                    Action::Rename {
                        project: Box::new(project),
                        name: text,
                    },
                )
            }
            TextThen::AddTag => {
                self.modals.pop();
                let tag = text.trim().to_string();
                if tag.is_empty() {
                    return Vec::new();
                }
                self.add_tag(tag)
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
            TextThen::Delete => {
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
                let Some(project) = project else {
                    return Vec::new();
                };
                self.run_action("deleting…", Action::Delete(Box::new(project)))
            }
        }
    }

    /// Whether a verb acts on the marks rather than the selection.
    fn batching(&self) -> bool {
        !self.library.marks.is_empty()
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
        self.run_action(
            "adding a note…",
            Action::AppendNote {
                project: Box::new(project),
                text,
            },
        )
    }

    /// The quick note: Enter saves, Alt-Enter breaks a line, Esc cancels;
    /// everything else is the text area's.
    fn on_note_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Esc if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
            KeyCode::Enter if !key.alt && !key.ctrl => {
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
            KeyCode::Enter => {
                if let Some(Modal::Note(note)) = self.modals.top_mut() {
                    note.area.apply(&Key::plain(KeyCode::Enter));
                }
                Vec::new()
            }
            _ => {
                if let Some(Modal::Note(note)) = self.modals.top_mut() {
                    note.area.apply(&key);
                }
                Vec::new()
            }
        }
    }

    fn on_confirm_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if !key.ctrl => self.answer_confirm(true),
            KeyCode::Char('n') | KeyCode::Char('N') if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
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
            ConfirmThen::Unregister => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                self.run_action("unregistering…", Action::Unregister(Box::new(project)))
            }
            ConfirmThen::DeleteTemplate(slug) => {
                self.run_action("deleting the template…", Action::DeleteTemplate(slug))
            }
            ConfirmThen::DeleteBatch => self.start_job(jobs::JobKind::Delete, None),
            ConfirmThen::UnregisterBatch => self.start_job(jobs::JobKind::Unregister, None),
        }
    }

    fn on_multi_pick_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter => self.submit_multi_pick(),
            KeyCode::Char(' ') => {
                if let Some(Modal::MultiPick(pick)) = self.modals.top_mut()
                    && let Some(flag) = pick.picked.get_mut(pick.selected)
                {
                    *flag = !*flag;
                }
                Vec::new()
            }
            _ => self.lookup_and_run(key),
        }
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

    fn on_palette_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Esc => {
                self.modals.pop();
                Vec::new()
            }
            KeyCode::Enter => {
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
                        self.focus = Focus::Projects;
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
            KeyCode::Up | KeyCode::Down if !key.ctrl => {
                let rows = self.palette_rows();
                if let Some(Modal::Palette(palette)) = self.modals.top_mut() {
                    palette.step(if key.code == KeyCode::Down { 1 } else { -1 });
                    palette.clamp_viewport(rows);
                }
                Vec::new()
            }
            KeyCode::Char('n') | KeyCode::Char('p') if key.ctrl => {
                let rows = self.palette_rows();
                if let Some(Modal::Palette(palette)) = self.modals.top_mut() {
                    palette.step(if key.code == KeyCode::Char('n') {
                        1
                    } else {
                        -1
                    });
                    palette.clamp_viewport(rows);
                }
                Vec::new()
            }
            _ => {
                let changed = match self.modals.top_mut() {
                    Some(Modal::Palette(palette)) => palette.input.apply(&key),
                    _ => false,
                };
                if changed {
                    self.refresh_palette();
                }
                Vec::new()
            }
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

    fn on_pick_key(&mut self, key: Key) -> Vec<Effect> {
        let area = self.area();
        match key.code {
            KeyCode::Esc => {
                self.modals.pop();
                Vec::new()
            }
            KeyCode::Enter => {
                let Some(Modal::Pick(pick)) = self.modals.pop() else {
                    return Vec::new();
                };
                let Some(item) = pick.chosen().cloned() else {
                    return Vec::new();
                };
                match pick.then {
                    Then::SortPick => {
                        self.library.explicit_sort = Order::CYCLE
                            .iter()
                            .copied()
                            .find(|s| s.label() == item.value);
                        self.recompute();
                        self.after_rows_changed()
                    }
                    Then::TemplateFilter => self.set_template_filter(Some(item.value.clone())),
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
                        if !self.library.marks.is_empty() {
                            self.start_job(jobs::JobKind::Move, Some(target))
                        } else {
                            self.run_move(target)
                        }
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
            KeyCode::Up | KeyCode::Down if !key.ctrl => {
                if let Some(Modal::Pick(pick)) = self.modals.top_mut() {
                    pick.step(if key.code == KeyCode::Down { 1 } else { -1 });
                    pick.clamp_viewport(layout::list_rows(
                        layout::pick_box(area, pick.ranked.len()),
                        2,
                    ));
                }
                Vec::new()
            }
            _ => {
                if let Some(Modal::Pick(pick)) = self.modals.top_mut()
                    && pick.query.apply(&key)
                {
                    pick.rank(&mut self.fuzzy);
                }
                Vec::new()
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

            Modal::Message { lines, scroll, .. } => (
                scroll,
                lines.len(),
                layout::message_box(area).height.saturating_sub(2) as usize,
            ),
            Modal::Studio(studio) => {
                studio.scroll = studio.scroll.saturating_add_signed(delta);
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        let max = lines.saturating_sub(rows);
        *scroll = scroll.saturating_add_signed(delta).min(max);
        Vec::new()
    }

    /// An upper bound on how far the detail pane can scroll: the lines it
    /// draws for the selected row, less the rows it has — so scrolling stops
    /// about where the text does, a blank line or two past the end at worst
    /// and never a screenful of nothing.
    fn detail_scroll_max(&self) -> usize {
        let Some(project) = self.library.selected() else {
            return 0;
        };
        let mut lines = 4 + usize::from(!project.tags.is_empty());
        if let Some(detail) = self.details.get(&project.path) {
            lines += usize::from(detail.error.is_some());
            if let Some(meta) = &detail.meta
                && !meta.variables.is_empty()
            {
                lines += 1 + meta.variables.len();
            }
            if !detail.listing.is_empty() {
                lines += 2 + detail.listing.len();
            }
            if !detail.journal.is_empty() {
                lines += 1 + detail.journal.len();
            }
            if !detail.notes.is_empty() {
                lines += 1 + detail.notes.len();
            }
        }
        let rows = self
            .regions()
            .detail
            .map(|pane| pane.height.saturating_sub(2) as usize)
            .unwrap_or(0);
        lines.saturating_sub(rows)
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
            CommandId::Quit => {
                // Quitting under a running move would abandon it mid-write.
                if self.job.is_some() || self.move_progress.is_some() {
                    return self.request_cancel();
                }
                vec![Effect::Quit(Exit::Normal)]
            }
            CommandId::Back => {
                // Anything running is cancelled first: a batch job and a
                // single move alike, before a keystroke can clear something
                // the user was looking at.
                if self.job.is_some() || self.move_progress.is_some() {
                    return self.request_cancel();
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
            CommandId::StudioNew => self.open_builder(None),
            CommandId::StudioEdit => {
                let slug = match self.modals.top() {
                    Some(Modal::Studio(studio)) => studio.selected_slug(),
                    _ => None,
                };
                match slug {
                    Some(slug) => self.open_builder(Some(slug)),
                    None => Vec::new(),
                }
            }
            CommandId::StudioFromFolder => {
                self.modals.push(Modal::Flow(Box::new(Flow::new(
                    FlowKind::FromFolder,
                    wizard::from_folder_form(),
                ))));
                Vec::new()
            }
            CommandId::StudioDelete => {
                let slug = match self.modals.top() {
                    Some(Modal::Studio(studio)) => studio.selected_slug(),
                    _ => None,
                };
                let Some(slug) = slug else {
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
            CommandId::SettingsChange => self.settings_change(),
            CommandId::Palette => {
                self.open_palette();
                Vec::new()
            }
            CommandId::Reload => vec![self.discover(), Effect::LoadSummary],
            CommandId::Reindex => self.run_action("reindexing…", Action::Reindex),
            CommandId::FocusNext | CommandId::FocusPrevious => {
                let forward = id == CommandId::FocusNext;
                self.focus = self.next_focus(forward);
                Vec::new()
            }
            CommandId::Down | CommandId::Up => {
                let delta = if id == CommandId::Down { 1 } else { -1 };
                if !self.modals.is_empty() {
                    return self.step_top_modal(delta);
                }
                match self.focus {
                    Focus::Projects => {
                        self.library.step(delta);
                        self.after_selection_change()
                    }
                    Focus::Detail => {
                        self.detail_scroll = self
                            .detail_scroll
                            .saturating_add_signed(delta)
                            .min(self.detail_scroll_max());
                        Vec::new()
                    }
                    Focus::Templates => {
                        self.templates.step(delta);
                        Vec::new()
                    }
                }
            }
            CommandId::PageDown | CommandId::PageUp => {
                let rows = self.page_rows() as isize;
                let delta = if id == CommandId::PageDown {
                    rows
                } else {
                    -rows
                };
                if !self.modals.is_empty() {
                    return self.page_top_modal(delta);
                }
                match self.focus {
                    Focus::Detail => {
                        self.detail_scroll = self
                            .detail_scroll
                            .saturating_add_signed(delta)
                            .min(self.detail_scroll_max());
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
                match self.focus {
                    Focus::Detail => {
                        self.detail_scroll = if first { 0 } else { self.detail_scroll_max() };
                        Vec::new()
                    }
                    Focus::Templates => {
                        self.templates.selected = if first {
                            0
                        } else {
                            self.templates.cards.len().saturating_sub(1)
                        };
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

            CommandId::Search => {
                self.search.editing = true;
                self.focus = Focus::Projects;
                Vec::new()
            }
            CommandId::ClearSearch => {
                self.search.input.clear();
                self.after_query_change()
            }
            CommandId::SortCycle => {
                let current = self.library.effective_sort(&self.search.query);
                self.library.explicit_sort = Some(current.next());
                self.recompute();
                let sort = self.library.effective_sort(&self.search.query);
                self.info(format!("sorted by {}", sort.label()));
                self.after_rows_changed()
            }
            CommandId::SortPick => {
                let items = Order::CYCLE
                    .iter()
                    .map(|sort| PickItem {
                        label: sort.label().to_string(),
                        detail: String::new(),
                        value: sort.label().to_string(),
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
            CommandId::ClearTemplateFilter => self.set_template_filter(None),
            CommandId::Actions => {
                self.modals.push(Modal::Actions(
                    crate::tui::app::actions::ActionsState::default(),
                ));
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
                if !self.detail_visible() && self.focus == Focus::Detail {
                    self.focus = Focus::Projects;
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
                    self.library.marks.insert(path);
                }
                self.library.step(1);
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
                let mut prompt = TextPrompt::new(validators::RENAME_PROMPT, TextThen::Rename);
                prompt.input =
                    crate::tui::widgets::input::LineEdit::with_text(project.name.clone());
                self.modals.push(Modal::TextPrompt(prompt));
                Vec::new()
            }
            CommandId::Move => self.open_move_picker(),
            CommandId::Unregister => {
                // Nothing is lost by unregistering, so a yes/no is enough —
                // but the question names the folders it is about.
                let names = self.target_names();
                if names.is_empty() {
                    return Vec::new();
                }
                let then = if self.batching() {
                    ConfirmThen::UnregisterBatch
                } else {
                    ConfirmThen::Unregister
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
                self.modals.push(Modal::TextPrompt(TextPrompt::new(
                    validators::delete_prompt(&names),
                    TextThen::Delete,
                )));
                Vec::new()
            }

            CommandId::ShowMetadata | CommandId::ShowJournal => self.open_view(id),
            CommandId::NewProject => self.open_create(),
            CommandId::Register => self.open_register(),
            CommandId::ApplyTemplate => self.open_apply(),
            CommandId::Templates => self.open_studio(),
            CommandId::Settings => self.open_settings(),
            CommandId::Reconcile => self.run_job(settings::Job::Reconcile),
            CommandId::StripFilter => {
                let slug = self.templates.selected_card().map(|c| c.slug.clone());
                let next = if slug == self.library.template_filter {
                    None
                } else {
                    slug
                };
                self.set_template_filter(next)
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
            self.warn("no templates yet — press T to make one");
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
            self.warn("no templates yet — press T to make one");
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
            _ => return Vec::new(),
        };
        if let Some(Modal::Flow(flow)) = self.modals.top_mut() {
            flow.scroll = (flow.scroll as isize + delta).max(0) as usize;
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

    // --- the template studio ----------------------------------------------

    /// `T`: every template, with the selected one's details beside it.
    fn open_studio(&mut self) -> Vec<Effect> {
        let cards = self
            .summary
            .as_ref()
            .map(|summary| summary.templates.clone())
            .unwrap_or_default();
        let studio = Studio::new(cards);
        let slug = studio.selected_slug();
        self.modals.push(Modal::Studio(studio));
        slug.map(|slug| vec![Effect::LoadTemplateView { slug }])
            .unwrap_or_default()
    }

    /// The studio is a list: every key it answers is declared in the
    /// registry under `Context::Studio`.
    fn on_studio_key(&mut self, key: Key) -> Vec<Effect> {
        self.lookup_and_run(key)
    }

    /// Esc on a dialog: one level at a time. A builder section goes back to
    /// the section list; the section list discards the template; everything
    /// else simply closes.
    fn close_top(&mut self) -> Vec<Effect> {
        match self.modals.top_mut() {
            Some(Modal::Builder(builder)) => {
                if builder.open.is_some() {
                    builder.open = None;
                } else {
                    self.modals.pop();
                    self.info("Discarded — the template was not written.");
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
                pick.selected = crate::tui::widgets::nav::wrap_step(
                    Some(pick.selected),
                    pick.items.len(),
                    delta,
                )
                .unwrap_or(0);
                Vec::new()
            }
            Some(Modal::Studio(studio)) => {
                studio.step(delta);
                studio.clamp_viewport(layout::studio_rows(
                    area,
                    studio.cards.len(),
                    studio.lines.len(),
                ));
                studio
                    .selected_slug()
                    .map(|slug| vec![Effect::LoadTemplateView { slug }])
                    .unwrap_or_default()
            }
            Some(Modal::Builder(builder)) => {
                match &mut builder.open {
                    None => builder.step(delta),
                    Some(Open::Variables(list)) => {
                        let count = builder.template.variables.len();
                        list.selected = list
                            .selected
                            .saturating_add_signed(delta)
                            .min(count.saturating_sub(1));
                    }
                    Some(Open::Files(list)) => {
                        let count = builder.template.files.len();
                        list.selected = list
                            .selected
                            .saturating_add_signed(delta)
                            .min(count.saturating_sub(1));
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
            crate::tui::widgets::nav::clamp_jump(Some(selected), len, delta).unwrap_or(0)
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
            Some(Modal::Studio(_)) | Some(Modal::Help { .. }) | Some(Modal::Message { .. }) => {
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
                Row::Discard => {
                    self.modals.pop();
                    self.info("Discarded — the template was not written.");
                    Vec::new()
                }
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

    /// New (`slug` is `None`) or edit: the builder over a scratch template.
    fn open_builder(&mut self, slug: Option<String>) -> Vec<Effect> {
        match slug {
            Some(slug) => {
                let mut builder = Builder::new(None);
                builder.pending = true;
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
        if builder.pending {
            if key.code == KeyCode::Esc {
                self.modals.pop();
            }
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
    fn save_template(&mut self) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if let Err(error) = builder.template.validate() {
            builder.error = Some(format!("Cannot save: {error:#}"));
            return Vec::new();
        }
        let Some(Modal::Builder(builder)) = self.modals.pop() else {
            return Vec::new();
        };
        self.run_action(
            "saving the template…",
            Action::SaveTemplate {
                template: Box::new(builder.template),
                original_slug: builder.original_slug,
            },
        )
    }

    /// The metadata and ID sections: a form, checked here because every rule
    /// they enforce is a rule about the text and not about a disk.
    fn on_builder_form_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let is_metadata = matches!(builder.open, Some(Open::Metadata(_)));
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
            FormEvent::Changed if is_metadata => studio::suggest_slug(form),
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
                FormEvent::Changed => studio::sync_variable_form(form),
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
        // the value unchanged" has always meant.
        if key.code == KeyCode::Esc && !key.ctrl {
            state.editing = None;
            return Vec::new();
        }
        let commit = match &mut state.editing {
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
        let Some(Modal::Onboarding(state)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => {
                self.modals.pop();
                self.info(validators::ONBOARDING_SKIPPED);
                Vec::new()
            }
            KeyCode::Enter => {
                let answer = state.input.text().trim().to_string();
                if answer.is_empty() {
                    self.modals.pop();
                    self.info(validators::ONBOARDING_SKIPPED);
                    return Vec::new();
                }
                // The dialog stays up until the folder exists: a path that
                // cannot be created is refused here, with the text still on the
                // line, rather than dropping a first-time user onto an empty
                // dashboard with an error and no question.
                state.pending = true;
                state.error = None;
                self.run_action("creating the base…", Action::InitBaseDir(answer))
            }
            _ => {
                if state.input.apply(&key) {
                    state.error = None;
                }
                Vec::new()
            }
        }
    }

    fn next_focus(&self, forward: bool) -> Focus {
        let regions = self.regions();
        let mut ring = vec![Focus::Projects];
        if regions.detail.is_some() {
            ring.push(Focus::Detail);
        }
        if regions.strip.is_some() && !self.templates.cards.is_empty() {
            ring.push(Focus::Templates);
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
        // The item that was running leaves `inflight`, whatever happened.
        let id = self
            .job
            .as_mut()
            .and_then(|job| job.clear_inflight())
            .unwrap_or_else(|| "?".to_string());
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
                self.apply_change(outcome.change);
                if let Some(job) = &mut self.job {
                    job.done += 1;
                }
            }
            Err(error) => {
                // A cancellation the user asked for is not a failure to list:
                // the report says how many were left instead.
                let cancelled = self.job.as_ref().is_some_and(|job| job.cancelled);
                if !cancelled && let Some(job) = &mut self.job {
                    job.failed.push((id, error));
                }
            }
        }
        self.job_advance()
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
                "journal"
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
