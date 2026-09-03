//! The project list: the snapshot discovery handed over, the rows the query
//! keeps, the selection, the marks, and what has been measured so far.
//!
//! Indices, not clones: `filtered` is a projection over `snapshot`, so a
//! keystroke in the search bar re-filters without copying a project. The
//! selection survives a re-filter, a re-sort and a reload by **path**, which is
//! the one identity that does not move.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use nucleo_matcher::Utf32String;

use crate::core::library::{self, Project};
use crate::core::project_info::Metadata;
use crate::core::query;
use crate::tui::app::search::{Query, row_meta};
use crate::tui::entry::Preset;
use crate::tui::fuzzy::{Fuzzy, Word};
use crate::tui::widgets::nav;

/// Which text of a row a word was matched against. Only the two the table
/// draws keep their hit characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Id,
    Other,
}

/// A row's searchable texts, each on its own: a word matches inside one of
/// them, never across two.
type Fields = Vec<(Field, Utf32String)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    Newest,
    Oldest,
    Name,
    Id,
    Template,
    Base,
    Size,
    /// Chosen automatically while the query has bare words; never in the cycle.
    Relevance,
}

impl Order {
    /// What `s` walks through.
    pub const CYCLE: [Order; 7] = [
        Order::Newest,
        Order::Oldest,
        Order::Name,
        Order::Id,
        Order::Template,
        Order::Base,
        Order::Size,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Order::Newest => "newest",
            Order::Oldest => "oldest",
            Order::Name => "name",
            Order::Id => "id",
            Order::Template => "template",
            Order::Base => "base",
            Order::Size => "size",
            Order::Relevance => "relevance",
        }
    }

    pub fn next(self) -> Order {
        let at = Self::CYCLE
            .iter()
            .position(|s| *s == self)
            .unwrap_or(Self::CYCLE.len() - 1);
        Self::CYCLE[(at + 1) % Self::CYCLE.len()]
    }
}

/// Where a fuzzy query hit a row, as char offsets into the id and the name.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MatchInfo {
    pub score: u32,
    pub id_hits: Vec<usize>,
    pub name_hits: Vec<usize>,
}

#[derive(Debug)]
pub struct LibraryState {
    /// Discovery order: newest first.
    pub snapshot: Vec<Project>,
    /// What the free words are matched against, one row of fields per
    /// snapshot row.
    haystacks: Vec<Fields>,
    pub generation: u64,
    /// A discovery that has not answered yet.
    pub inflight: Option<u64>,
    /// A row was patched or removed while a discovery was in flight, so its
    /// answer may predate the change: discover once more when it lands.
    pub dirty: bool,
    pub loaded: bool,
    pub error: Option<String>,
    /// Indices into `snapshot`, in display order.
    pub filtered: Vec<usize>,
    /// Parallel to `filtered`: the fuzzy hits, when the query has bare words.
    pub scores: Vec<Option<MatchInfo>>,
    /// Index into `filtered`.
    pub selected: Option<usize>,
    /// First visible row of `filtered`.
    pub offset: usize,
    pub marks: BTreeSet<PathBuf>,
    /// What the user chose with `s`/`S`; `None` follows the query.
    pub explicit_sort: Option<Order>,
    pub template_filter: Option<String>,
    pub preset: Option<Preset>,
    /// Landed size cells; a missing key is still pending.
    pub sizes: HashMap<PathBuf, Option<u64>>,
    /// Metadata read on demand; `Some(None)` is a project whose file could not
    /// be parsed.
    pub meta: HashMap<PathBuf, Option<Metadata>>,
    /// Every tag the snapshot carries, sorted and distinct.
    pub known_tags: Vec<String>,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self::new()
    }
}

impl LibraryState {
    pub fn new() -> Self {
        Self {
            snapshot: Vec::new(),
            haystacks: Vec::new(),
            generation: 0,
            inflight: None,
            dirty: false,
            loaded: false,
            error: None,
            filtered: Vec::new(),
            scores: Vec::new(),
            selected: None,
            offset: 0,
            marks: BTreeSet::new(),
            explicit_sort: None,
            template_filter: None,
            preset: None,
            sizes: HashMap::new(),
            meta: HashMap::new(),
            known_tags: Vec::new(),
        }
    }

    /// Take a discovery's answer. `false` when it is not the one in flight —
    /// an older request answering after a newer one was sent.
    pub fn install(&mut self, generation: u64, projects: Vec<Project>) -> bool {
        if self.inflight != Some(generation) {
            return false;
        }
        self.inflight = None;
        self.generation = generation;
        self.loaded = true;
        self.error = None;
        self.replace_snapshot(projects);
        true
    }

    /// Install rows that were read before the app opened, with no discovery
    /// in flight.
    pub fn install_initial(&mut self, projects: Vec<Project>) {
        self.loaded = true;
        self.replace_snapshot(projects);
    }

    fn replace_snapshot(&mut self, projects: Vec<Project>) {
        let present: BTreeSet<PathBuf> = projects.iter().map(|p| p.path.clone()).collect();
        self.marks.retain(|path| present.contains(path));
        self.meta.retain(|path, _| present.contains(path));
        self.snapshot = projects;
        self.rebuild_haystacks();
        self.known_tags = known_tags(&self.snapshot);
    }

    fn rebuild_haystacks(&mut self) {
        self.haystacks = self
            .snapshot
            .iter()
            .map(|p| row_fields(p, self.meta.get(&p.path)))
            .collect();
    }

    fn rebuild_haystack(&mut self, index: usize) {
        if let Some(p) = self.snapshot.get(index) {
            self.haystacks[index] = row_fields(p, self.meta.get(&p.path));
        }
    }

    /// Replace one row after a content mutation. `false` when the row is not
    /// in the snapshot (it was removed meanwhile).
    pub fn patch(&mut self, project: Project) -> bool {
        let Some(index) = self.snapshot.iter().position(|p| p.path == project.path) else {
            return false;
        };
        self.snapshot[index] = project;
        self.rebuild_haystack(index);
        self.known_tags = known_tags(&self.snapshot);
        if self.inflight.is_some() {
            self.dirty = true;
        }
        true
    }

    /// Replace a row whose path changed (a rename, a move): the old row goes,
    /// the new one takes its place.
    pub fn replace(&mut self, old_path: &Path, project: Project) {
        if let Some(index) = self.snapshot.iter().position(|p| p.path == old_path) {
            self.marks.remove(old_path);
            self.meta.remove(old_path);
            self.snapshot[index] = project;
            self.rebuild_haystack(index);
        } else {
            self.snapshot.insert(0, project);
            self.rebuild_haystacks();
        }
        self.known_tags = known_tags(&self.snapshot);
        if self.inflight.is_some() {
            self.dirty = true;
        }
    }

    pub fn remove(&mut self, path: &Path) {
        if let Some(index) = self.snapshot.iter().position(|p| p.path == path) {
            self.snapshot.remove(index);
            self.haystacks.remove(index);
            self.marks.remove(path);
            self.meta.remove(path);
            self.sizes.remove(path);
            self.known_tags = known_tags(&self.snapshot);
            if self.inflight.is_some() {
                self.dirty = true;
            }
        }
    }

    /// Metadata that a query asked for landed: the fuzzy haystack grows by the
    /// variable values.
    pub fn absorb_meta(&mut self, loaded: Vec<(PathBuf, Option<Metadata>)>) {
        for (path, meta) in loaded {
            self.meta.insert(path, meta);
        }
        self.rebuild_haystacks();
    }

    /// Rows whose metadata has not been read.
    pub fn paths_without_meta(&self) -> Vec<PathBuf> {
        self.snapshot
            .iter()
            .filter(|p| !self.meta.contains_key(&p.path))
            .map(|p| p.path.clone())
            .collect()
    }

    /// The order the rows are in right now.
    pub fn effective_sort(&self, query: &Query) -> Order {
        match self.explicit_sort {
            Some(sort) => sort,
            None if !query.free.is_empty() => Order::Relevance,
            None => Order::Newest,
        }
    }

    /// Re-filter and re-sort after anything that changes what is shown. The
    /// selection follows its row; when that row is gone the same position is
    /// kept, clamped.
    pub fn recompute(&mut self, query: &Query, fuzzy: &mut Fuzzy) {
        let keep_path = self.selected().map(|p| p.path.clone());
        let words = Fuzzy::words(&query.free_text());

        let mut rows: Vec<(usize, Option<MatchInfo>)> = Vec::new();
        for (index, project) in self.snapshot.iter().enumerate() {
            if let Some(slug) = &self.template_filter
                && &project.template != slug
            {
                continue;
            }
            if let Some(preset) = &self.preset
                && !preset.keeps(project)
            {
                continue;
            }
            if !query.structured.is_empty() {
                let passes = match self.meta.get(&project.path) {
                    Some(Some(meta)) => query::evaluate(&query.structured, meta),
                    _ => query::evaluate(&query.structured, &row_meta(project)),
                };
                if !passes {
                    continue;
                }
            }
            let info = if words.is_empty() {
                None
            } else {
                match match_fields(fuzzy, &words, &self.haystacks[index]) {
                    Some(info) => Some(info),
                    None => continue,
                }
            };
            rows.push((index, info));
        }

        let sort = self.effective_sort(query);
        rows.sort_by(|a, b| self.compare(sort, a, b));
        if let Some(limit) = self.preset.as_ref().and_then(|p| p.limit) {
            rows.truncate(limit);
        }

        self.filtered = rows.iter().map(|(index, _)| *index).collect();
        self.scores = rows.into_iter().map(|(_, info)| info).collect();

        self.selected = match keep_path {
            Some(path) => self
                .filtered
                .iter()
                .position(|&index| self.snapshot[index].path == path)
                .or_else(|| self.clamped_selection()),
            None => self.clamped_selection(),
        };
    }

    fn clamped_selection(&self) -> Option<usize> {
        if self.filtered.is_empty() {
            None
        } else {
            Some(self.selected.unwrap_or(0).min(self.filtered.len() - 1))
        }
    }

    fn compare(
        &self,
        sort: Order,
        a: &(usize, Option<MatchInfo>),
        b: &(usize, Option<MatchInfo>),
    ) -> std::cmp::Ordering {
        let pa = &self.snapshot[a.0];
        let pb = &self.snapshot[b.0];
        let newest =
            |x: &Project, y: &Project| y.created.cmp(&x.created).then_with(|| x.name.cmp(&y.name));
        match sort {
            Order::Newest => newest(pa, pb),
            Order::Oldest => newest(pb, pa),
            Order::Name => pa
                .name
                .to_lowercase()
                .cmp(&pb.name.to_lowercase())
                .then_with(|| newest(pa, pb)),
            Order::Id => crate::core::naming::id_value(&pa.id)
                .cmp(&crate::core::naming::id_value(&pb.id))
                .then_with(|| pa.id.cmp(&pb.id)),
            Order::Template => pa.template.cmp(&pb.template).then_with(|| newest(pa, pb)),
            Order::Base => library::base_label(&pa.base)
                .cmp(&library::base_label(&pb.base))
                .then_with(|| newest(pa, pb)),
            Order::Size => {
                // Biggest first; unmeasured and unmeasurable rows last.
                let size = |p: &Project| self.sizes.get(&p.path).copied().flatten();
                size(pb).cmp(&size(pa)).then_with(|| newest(pa, pb))
            }
            Order::Relevance => {
                let score = |info: &Option<MatchInfo>| info.as_ref().map_or(0, |i| i.score);
                score(&b.1).cmp(&score(&a.1)).then_with(|| newest(pa, pb))
            }
        }
    }

    pub fn selected(&self) -> Option<&Project> {
        self.selected
            .and_then(|at| self.filtered.get(at))
            .and_then(|&index| self.snapshot.get(index))
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// The project at display row `row`.
    pub fn row(&self, row: usize) -> Option<&Project> {
        self.filtered
            .get(row)
            .and_then(|&index| self.snapshot.get(index))
    }

    pub fn match_info(&self, row: usize) -> Option<&MatchInfo> {
        self.scores.get(row).and_then(|info| info.as_ref())
    }

    pub fn len(&self) -> usize {
        self.filtered.len()
    }

    pub fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }

    /// Arrow keys wrap.
    pub fn step(&mut self, delta: isize) {
        self.selected = nav::wrap_step(self.selected, self.filtered.len(), delta);
    }

    /// Page keys clamp.
    pub fn jump(&mut self, delta: isize) {
        self.selected = nav::clamp_jump(self.selected, self.filtered.len(), delta);
    }

    pub fn select_first(&mut self) {
        self.selected = (!self.filtered.is_empty()).then_some(0);
    }

    pub fn select_last(&mut self) {
        self.selected = self.filtered.len().checked_sub(1);
    }

    pub fn select_path(&mut self, path: &Path) -> bool {
        match self
            .filtered
            .iter()
            .position(|&index| self.snapshot[index].path == path)
        {
            Some(at) => {
                self.selected = Some(at);
                true
            }
            None => false,
        }
    }

    /// Keep the selection on screen for a table `rows` high.
    pub fn clamp_viewport(&mut self, rows: usize) {
        self.offset = nav::viewport_offset(self.offset, self.selected, self.filtered.len(), rows);
    }

    /// The paths a table `rows` high shows, selected row first — the one the
    /// user is pointing at is measured next.
    pub fn visible_paths(&self, rows: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(selected) = self.selected() {
            out.push(selected.path.clone());
        }
        for row in self.offset..(self.offset + rows).min(self.filtered.len()) {
            if let Some(project) = self.row(row)
                && !out.contains(&project.path)
            {
                out.push(project.path.clone());
            }
        }
        out
    }

    /// Whether any visible row is still waiting for its size.
    pub fn sizes_pending(&self, rows: usize) -> bool {
        self.visible_paths(rows)
            .iter()
            .any(|path| !self.sizes.contains_key(path))
    }

    /// What a verb acts on: the marks when there are any, else the selection.
    pub fn targets(&self) -> Vec<Project> {
        if self.marks.is_empty() {
            return self.selected().cloned().into_iter().collect();
        }
        self.filtered
            .iter()
            .map(|&index| &self.snapshot[index])
            .filter(|p| self.marks.contains(&p.path))
            .cloned()
            .collect()
    }

    /// Projects per template slug, over the whole snapshot.
    pub fn per_template(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for project in &self.snapshot {
            *counts.entry(project.template.clone()).or_insert(0) += 1;
        }
        counts
    }
}

/// A row's texts, each its own haystack: the name and the id first, because
/// they are what the table shows and what a hit is highlighted in; then the
/// template slug and name, every tag, and the variable values once metadata
/// is loaded.
fn row_fields(project: &Project, meta: Option<&Option<Metadata>>) -> Fields {
    let mut fields: Fields = vec![
        (Field::Name, Fuzzy::haystack(&project.name)),
        (Field::Id, Fuzzy::haystack(&project.id)),
        (Field::Other, Fuzzy::haystack(&project.template)),
        (Field::Other, Fuzzy::haystack(&project.template_name)),
    ];
    for tag in &project.tags {
        fields.push((Field::Other, Fuzzy::haystack(tag)));
    }
    if let Some(Some(meta)) = meta {
        for value in meta.variables.values() {
            fields.push((Field::Other, Fuzzy::haystack(value)));
        }
    }
    fields
}

/// Every word must match one of the row's fields — any field, but the whole
/// word inside it. The best field per word counts, and its hit characters are
/// kept when they land in a column the table draws.
fn match_fields(fuzzy: &mut Fuzzy, words: &[Word], fields: &Fields) -> Option<MatchInfo> {
    let mut info = MatchInfo::default();
    for word in words {
        let mut best: Option<(Field, crate::tui::fuzzy::Hit)> = None;
        for (field, haystack) in fields {
            if let Some(hit) = fuzzy.match_word(word, haystack)
                && best.as_ref().is_none_or(|(_, held)| hit.score > held.score)
            {
                best = Some((*field, hit));
            }
        }
        let (field, hit) = best?;
        info.score += hit.score;
        let hits = hit.indices.into_iter().map(|i| i as usize);
        match field {
            Field::Name => info.name_hits.extend(hits),
            Field::Id => info.id_hits.extend(hits),
            Field::Other => {}
        }
    }
    info.name_hits.sort_unstable();
    info.name_hits.dedup();
    info.id_hits.sort_unstable();
    info.id_hits.dedup();
    Some(info)
}

/// The distinct tags across a loaded list, sorted for a stable picker.
fn known_tags(projects: &[Project]) -> Vec<String> {
    let mut tags: Vec<String> = projects
        .iter()
        .flat_map(|project| project.tags.iter().cloned())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}
