//! The model, and `update`: one function of the app and one message, with no
//! I/O of its own. Everything it wants done comes back as an `Effect`.
//!
//! That split is what makes the guided app testable without a terminal
//! (`tests/tui_update.rs` builds an `App`, feeds it messages and asserts on the
//! effects) and what keeps a slow filesystem out of the key handler: nothing in
//! here blocks, because nothing in here reads a disk.

pub mod data;
pub mod library;
pub mod modal;
pub mod palette;
pub mod search;

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;

use crate::core::library::Project;
use crate::tui::command::{self, Availability, CommandId, Context, Key};
use crate::tui::effect::{
    Action, ActionId, ActionOutcome, Effect, Exit, LegacyFlow, ListChange, SpawnKind, Suspended,
};
use crate::tui::entry::Entry;
use crate::tui::fuzzy::Fuzzy;
use crate::tui::layout;
use crate::tui::msg::{Msg, Resumed};
use crate::tui::theme::Theme;
use crate::util::diag::Level;
use crate::util::size_scan::SizeCell;
use data::{ProjectDetail, Summary, TemplateCard};
use library::{LibraryState, Order};
use modal::{MessageLevel, Modal, ModalStack, PickItem, PickState, Then};
use palette::{PaletteState, PaletteTarget};
use search::SearchState;

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
}

impl TemplatesState {
    /// Cards from the summary, plus a bare card for any slug the projects use
    /// that no template on disk answers to; busiest first.
    pub fn rebuild(&mut self, summary: Option<&Summary>, counts: HashMap<String, usize>) {
        let keep = self.selected_card().map(|c| c.slug.clone());
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
                });
            }
        }
        cards.sort_by(|a, b| {
            let ca = counts.get(&a.slug).copied().unwrap_or(0);
            let cb = counts.get(&b.slug).copied().unwrap_or(0);
            cb.cmp(&ca).then_with(|| a.slug.cmp(&b.slug))
        });
        self.cards = cards;
        self.counts = counts;
        self.selected = keep
            .and_then(|slug| self.cards.iter().position(|c| c.slug == slug))
            .unwrap_or(0)
            .min(self.cards.len().saturating_sub(1));
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
    pub status: Status,
    /// The last few things this session did, oldest first.
    pub session: Vec<String>,
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
            status: Status::default(),
            session: crate::tui::frame::recent_actions(),
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
        }
        app.recompute();
        app
    }

    /// The first effects: the header's summary, and a discovery unless the
    /// rows were handed in.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut effects = vec![Effect::LoadSummary];
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
        layout::regions(self.area(), self.detail_open)
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
        self.status = Status {
            text: text.into(),
            level,
            expires_at: Some(self.ticks + STATUS_TICKS),
        };
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
            Msg::ActionDone { id, outcome } => self.on_action_done(id, outcome),
            Msg::Spawned { what, outcome } => self.on_spawned(what, outcome),
            Msg::Resumed(Resumed::Legacy { change, quit }) => {
                self.session = crate::tui::frame::recent_actions();
                if quit {
                    return vec![Effect::Quit(Exit::Normal)];
                }
                let mut effects = self.apply_change(change);
                if !effects.contains(&Effect::LoadSummary) {
                    effects.push(Effect::LoadSummary);
                }
                effects
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
                self.apply_change(outcome.change)
            }
            Err(error) => {
                self.error(format!("error: {error}"));
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

    fn on_paste(&mut self, text: &str) -> Vec<Effect> {
        match self.modals.top_mut() {
            Some(Modal::Palette(palette)) => {
                palette.input.paste(text);
                self.refresh_palette();
                Vec::new()
            }
            Some(Modal::Pick(pick)) => {
                pick.query.paste(text);
                pick.rank(&mut self.fuzzy);
                Vec::new()
            }
            Some(_) => Vec::new(),
            None if self.search.editing => {
                self.search.input.paste(text);
                self.after_query_change()
            }
            None => Vec::new(),
        }
    }

    // --- keys -------------------------------------------------------------

    fn on_key(&mut self, key: Key) -> Vec<Effect> {
        if layout::too_small(self.area()) {
            return if key == Key::ch('q') || key == Key::ctrl('c') {
                vec![Effect::Quit(Exit::Normal)]
            } else {
                Vec::new()
            };
        }
        if key == Key::ctrl('c') {
            return if self.modals.pop().is_some() {
                Vec::new()
            } else {
                vec![Effect::Quit(Exit::Interrupted)]
            };
        }
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
            Some(Modal::Help { .. }) | Some(Modal::Message { .. }) => self.on_scroll_modal_key(key),
            None => Vec::new(),
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
        match key.code {
            KeyCode::Esc => {
                self.modals.pop();
                Vec::new()
            }
            KeyCode::Enter => {
                let Some(Modal::Pick(pick)) = self.modals.pop() else {
                    return Vec::new();
                };
                let Some(item) = pick.chosen() else {
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
                }
            }
            KeyCode::Up | KeyCode::Down if !key.ctrl => {
                if let Some(Modal::Pick(pick)) = self.modals.top_mut() {
                    pick.step(if key.code == KeyCode::Down { 1 } else { -1 });
                    pick.clamp_viewport(12);
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

    fn on_scroll_modal_key(&mut self, key: Key) -> Vec<Effect> {
        let delta: isize = match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.modals.pop();
                return Vec::new();
            }
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 10,
            KeyCode::PageUp => -10,
            KeyCode::Home => isize::MIN / 2,
            _ => return Vec::new(),
        };
        match self.modals.top_mut() {
            Some(Modal::Help { scroll, .. }) | Some(Modal::Message { scroll, .. }) => {
                *scroll = (*scroll as isize + delta).max(0) as usize;
            }
            _ => {}
        }
        Vec::new()
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
            CommandId::Quit => vec![Effect::Quit(Exit::Normal)],
            CommandId::Back => {
                if !self.search.input.is_empty() {
                    self.search.input.clear();
                    return self.after_query_change();
                }
                if self.library.template_filter.is_some() {
                    return self.set_template_filter(None);
                }
                vec![Effect::Quit(Exit::Normal)]
            }
            CommandId::Help => {
                let ctx = self.focus_context();
                self.modals.push(Modal::Help { ctx, scroll: 0 });
                Vec::new()
            }
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
                match self.focus {
                    Focus::Projects => {
                        self.library.step(delta);
                        self.after_selection_change()
                    }
                    Focus::Detail => {
                        self.detail_scroll = (self.detail_scroll as isize + delta).max(0) as usize;
                        Vec::new()
                    }
                    Focus::Templates => {
                        self.templates.step(delta);
                        Vec::new()
                    }
                }
            }
            CommandId::PageDown | CommandId::PageUp => {
                let rows = self.rows_on_screen() as isize;
                let delta = if id == CommandId::PageDown {
                    rows
                } else {
                    -rows
                };
                match self.focus {
                    Focus::Detail => {
                        self.detail_scroll = (self.detail_scroll as isize + delta).max(0) as usize;
                        Vec::new()
                    }
                    _ => {
                        self.library.jump(delta);
                        self.after_selection_change()
                    }
                }
            }
            CommandId::First => {
                self.library.select_first();
                self.after_selection_change()
            }
            CommandId::Last => {
                self.library.select_last();
                self.after_selection_change()
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
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                let size = Some(self.size_cell(&project.path));
                vec![Effect::Suspend(Suspended::Legacy(LegacyFlow::ActionMenu {
                    project: Box::new(project),
                    size,
                    known_tags: self.library.known_tags.clone(),
                }))]
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
            CommandId::NewProject => vec![Effect::Suspend(Suspended::Legacy(LegacyFlow::Create))],
            CommandId::Register => vec![Effect::Suspend(Suspended::Legacy(LegacyFlow::Register))],
            CommandId::Templates => vec![Effect::Suspend(Suspended::Legacy(LegacyFlow::Templates))],
            CommandId::Settings => vec![Effect::Suspend(Suspended::Legacy(LegacyFlow::Settings))],
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

    fn run_action(&mut self, what: &'static str, action: Action) -> Vec<Effect> {
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
}

/// The state machine: the app and one message in, the effects out.
pub fn update(app: &mut App, msg: Msg) -> Vec<Effect> {
    app.handle(msg)
}
