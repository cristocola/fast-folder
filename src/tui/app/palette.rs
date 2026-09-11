//! The command palette: one fuzzy list over every command that applies right
//! now, the projects, and the templates.
//!
//! Enter on a command dispatches exactly the `CommandId` its key would; Enter
//! on a project selects its row; Enter on a template filters by it. A `#` or
//! `@` prefix restricts the list to projects — the jump-to-project gesture.

use std::path::PathBuf;

use super::{App, Focus};
use crate::tui::app::data::TemplateCard;
use crate::tui::app::library::LibraryState;
use crate::tui::app::modal::Modal;
use crate::tui::command::{self, Availability, Command, CommandId, Context, Key};
use crate::tui::effect::Effect;
use crate::tui::fuzzy::Fuzzy;
use crate::tui::layout;
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;

/// How many projects and templates the palette offers under a query.
const PROJECT_LIMIT: usize = 8;
const TEMPLATE_LIMIT: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteTarget {
    Command(CommandId),
    Project(PathBuf),
    Template(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteEntry {
    pub target: PaletteTarget,
    pub title: String,
    pub detail: String,
    /// The key label, for a command; empty otherwise.
    pub key: String,
    pub enabled: bool,
    pub reason: Option<&'static str>,
    /// Characters of `title` the query hit.
    pub hits: Vec<u32>,
    pub group: &'static str,
}

#[derive(Debug, Default)]
pub struct PaletteState {
    pub input: LineEdit,
    pub entries: Vec<PaletteEntry>,
    pub selected: Option<usize>,
    pub offset: usize,
}

impl PaletteState {
    pub fn chosen(&self) -> Option<&PaletteEntry> {
        self.selected.and_then(|at| self.entries.get(at))
    }

    pub fn step(&mut self, delta: isize) {
        self.selected = nav::step(self.selected, self.entries.len(), delta);
    }

    pub fn clamp_viewport(&mut self, rows: usize) {
        self.offset = nav::viewport_offset(self.offset, self.selected, self.entries.len(), rows);
    }

    pub fn set_entries(&mut self, entries: Vec<PaletteEntry>) {
        self.entries = entries;
        self.selected = (!self.entries.is_empty()).then_some(0);
        self.offset = 0;
    }
}

/// One scored candidate, before the groups are merged.
struct Candidate {
    entry: PaletteEntry,
    score: u32,
    order: usize,
}

/// Rank everything against `query`.
pub fn build(
    query: &str,
    commands: Vec<(&'static Command, Availability)>,
    library: &LibraryState,
    cards: &[TemplateCard],
    fuzzy: &mut Fuzzy,
) -> Vec<PaletteEntry> {
    let (projects_only, query) = match query.trim_start().strip_prefix(['#', '@']) {
        Some(rest) => (true, rest.trim()),
        None => (false, query.trim()),
    };
    let empty = query.is_empty();

    let mut candidates: Vec<Candidate> = Vec::new();

    if !projects_only {
        let words = Fuzzy::words(query);
        for (i, (command, availability)) in commands.iter().enumerate() {
            // A hit in the title outranks any hit in the description: `open`
            // is Open project folder before it is "open the action menu".
            let (score, hits) = if words.is_empty() {
                (0, Vec::new())
            } else {
                let title = fuzzy.match_all(&words, &Fuzzy::haystack(command.title));
                let description = fuzzy.match_all(&words, &Fuzzy::haystack(command.description));
                match (title, description) {
                    (Some(hit), _) => (1_000_000 + hit.score, hit.indices),
                    (None, Some(hit)) => (hit.score, Vec::new()),
                    (None, None) => continue,
                }
            };
            candidates.push(Candidate {
                entry: PaletteEntry {
                    target: PaletteTarget::Command(command.id),
                    title: command.title.to_string(),
                    detail: command.description.to_string(),
                    key: command.keys.first().map(|k| k.label()).unwrap_or_default(),
                    enabled: *availability == Availability::Enabled,
                    reason: match availability {
                        Availability::Disabled(reason) => Some(reason),
                        _ => None,
                    },
                    hits,
                    group: "commands",
                },
                // Commands outrank a project of equal score: they are what the
                // palette is for, and a project has its own search bar.
                score: score + 1,
                order: i,
            });
        }
    }

    let projects: Vec<(usize, String)> = library
        .snapshot
        .iter()
        .enumerate()
        .map(|(i, p)| (i, format!("{} {} {}", p.id, p.name, p.template)))
        .collect();
    let ranked = fuzzy.rank(query, projects);
    let take = if projects_only && !empty {
        usize::MAX
    } else {
        PROJECT_LIMIT
    };
    for (i, hit) in ranked.into_iter().take(take) {
        let p = &library.snapshot[i];
        let id_len = p.id.chars().count() as u32;
        candidates.push(Candidate {
            entry: PaletteEntry {
                target: PaletteTarget::Project(p.path.clone()),
                title: format!("{}  {}", p.id, p.name),
                detail: format!(
                    "go to · {} · {}",
                    p.template,
                    p.created.get(..10).unwrap_or(&p.created)
                ),
                key: String::new(),
                enabled: true,
                reason: None,
                // The title is `id␣␣name`; the haystack is `id␣name␣template`.
                hits: hit
                    .indices
                    .into_iter()
                    .filter(|&h| h < id_len + 1 + p.name.chars().count() as u32)
                    .map(|h| if h > id_len { h + 1 } else { h })
                    .collect(),
                group: "projects",
            },
            score: hit.score,
            order: usize::MAX / 2 + i,
        });
    }

    if !projects_only {
        let items: Vec<(usize, String)> = cards
            .iter()
            .enumerate()
            .map(|(i, c)| (i, format!("{} {}", c.slug, c.name)))
            .collect();
        for (i, hit) in fuzzy.rank(query, items).into_iter().take(TEMPLATE_LIMIT) {
            let card = &cards[i];
            let slug_len = card.slug.chars().count() as u32;
            candidates.push(Candidate {
                entry: PaletteEntry {
                    target: PaletteTarget::Template(card.slug.clone()),
                    title: format!("filter by template {}", card.slug),
                    detail: card.name.clone(),
                    key: String::new(),
                    enabled: true,
                    reason: None,
                    hits: hit
                        .indices
                        .into_iter()
                        .filter(|&h| h < slug_len)
                        .map(|h| h + "filter by template ".len() as u32)
                        .collect(),
                    group: "templates",
                },
                score: hit.score,
                order: usize::MAX / 2 + i,
            });
        }
    }

    if !empty {
        candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.order.cmp(&b.order)));
    }
    candidates.into_iter().map(|c| c.entry).collect()
}

impl App {
    /// The palette is a query with a list under it, so the same rule the
    /// search bar follows applies: everything printable is the query, and only
    /// a key a field cannot hold reaches the registry. `Ctrl-p` is the
    /// previous entry here rather than "open the palette", which is why the
    /// opener is bound in every context **except** this one.
    pub(super) fn on_palette_key(&mut self, key: Key) -> Vec<Effect> {
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
    pub(super) fn step_palette(&mut self, delta: isize) -> Vec<Effect> {
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
    pub(super) fn run_palette_entry(&mut self) -> Vec<Effect> {
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

    pub(super) fn refresh_palette(&mut self) {
        let query = match self.modals.top() {
            Some(Modal::Palette(palette)) => palette.input.text().to_string(),
            _ => return,
        };
        let commands = command::palette_entries(self.focus_context(), self);
        let entries = build(
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

    pub(super) fn open_palette(&mut self) {
        self.modals.push(Modal::Palette(PaletteState::default()));
        self.refresh_palette();
    }
}
