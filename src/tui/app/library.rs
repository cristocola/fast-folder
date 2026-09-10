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
use unicode_width::UnicodeWidthStr;

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

    /// The order a label names — one of the cycle's, never `Relevance`,
    /// which is chosen by the query and not by a person. A trailing
    /// `reversed` is the direction and belongs to `Sort`, so it is ignored
    /// here; `Sort::from_label` reads both halves.
    pub fn from_label(label: &str) -> Option<Order> {
        let label = label.trim().trim_end_matches(Sort::REVERSED).trim();
        Self::CYCLE
            .iter()
            .copied()
            .find(|order| order.label() == label)
    }

    /// Whether running this order backwards says anything the cycle does not.
    ///
    /// `newest` and `oldest` are already the two directions of one order, and
    /// relevance is the query's answer rather than a person's choice — so
    /// offering "newest reversed" beside "oldest" would be one list with two
    /// names for the same row.
    pub fn reversible(self) -> bool {
        matches!(
            self,
            Order::Name | Order::Id | Order::Template | Order::Base | Order::Size
        )
    }

    /// What running it backwards actually does, for the picker's second
    /// column — "reversed" says which way, not what you get.
    pub fn reversed_detail(self) -> &'static str {
        match self {
            Order::Name => "z to a",
            Order::Id => "the highest ID first",
            Order::Template => "z to a",
            Order::Base => "z to a",
            Order::Size => "the smallest first",
            _ => "",
        }
    }
}

/// An order and the direction it runs in.
///
/// The direction is not a second `Order` variant because every order that has
/// one has the *same* one, and five more variants is five more rows in a
/// picker, five more labels to persist and five more arms in `compare`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub order: Order,
    pub reversed: bool,
}

impl Sort {
    /// The word a reversed label ends in — one spelling, read by the session
    /// file, the picker and the search bar alike.
    pub const REVERSED: &'static str = "reversed";

    pub fn new(order: Order) -> Self {
        Self {
            order,
            reversed: false,
        }
    }

    pub fn label(self) -> String {
        if self.reversed {
            format!("{} {}", self.order.label(), Self::REVERSED)
        } else {
            self.order.label().to_string()
        }
    }

    /// Reads what `label` writes — **and every label written before there was
    /// a direction to write**, so a `state.toml` from an earlier version still
    /// names an order.
    pub fn from_label(label: &str) -> Option<Self> {
        let order = Order::from_label(label)?;
        let reversed = label.trim().ends_with(Self::REVERSED) && order.reversible();
        Some(Self { order, reversed })
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
    /// The row Space last acted on, so `v` has somewhere to reach from. Kept
    /// by path like the marks themselves, and dropped when that row leaves.
    pub last_mark: Option<PathBuf>,
    /// What the user chose with `s`/`S`; `None` follows the query.
    pub explicit_sort: Option<Sort>,
    pub template_filter: Option<String>,
    /// The one base the list is restricted to, by its full path. A path rather
    /// than a label, because two bases can share a basename and the label is
    /// what `base_label` shortens for display, not what identifies a base.
    pub base_filter: Option<PathBuf>,
    pub preset: Option<Preset>,
    /// Landed size cells; a missing key is still pending.
    pub sizes: HashMap<PathBuf, Option<u64>>,
    /// Metadata read on demand; `Some(None)` is a project whose file could not
    /// be parsed.
    pub meta: HashMap<PathBuf, Option<Metadata>>,
    /// Every tag the snapshot carries, sorted and distinct.
    pub known_tags: Vec<String>,
    /// The widest id and the widest name among the rows shown — measured once
    /// per `recompute`, so the table and the layout agree on them.
    pub widths: (usize, usize),
    /// The widest base label among the rows shown, and whether they come from
    /// more than one base. Measured beside `widths` for the same reason: the
    /// layout has to reserve the base column the table is about to elect, or
    /// the table asks for a width it will not use and the column never appears.
    pub base_width: usize,
    pub many_bases: bool,
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
            last_mark: None,
            explicit_sort: None,
            template_filter: None,
            base_filter: None,
            preset: None,
            sizes: HashMap::new(),
            meta: HashMap::new(),
            known_tags: Vec::new(),
            widths: (4, 8),
            base_width: 4,
            many_bases: false,
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
        self.last_mark = self.last_mark.take().filter(|p| present.contains(p));
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

    /// Replace one row after a content mutation.
    ///
    /// **Found by the path it had, and only then by id.** The id is the
    /// identity the frontmatter carries and it survives a rename and a move,
    /// which is why it was the only key — but `copy-to` can put a second
    /// project with the same id on a backup drive, and once that drive is a
    /// base, patching by id alone tags one row and shows it on the other.
    /// The old path is unique whatever else is true. `false` when neither
    /// finds it (the row was removed meanwhile).
    pub fn patch(&mut self, was: &Path, project: Project) -> bool {
        let found = self
            .snapshot
            .iter()
            .position(|p| p.path == was)
            .or_else(|| self.snapshot.iter().position(|p| p.id == project.id));
        let Some(index) = found else {
            return false;
        };
        if self.snapshot[index].path != project.path {
            // The row moved or was renamed: the old path's bookkeeping is no
            // longer this project's.
            let old_path = self.snapshot[index].path.clone();
            self.marks.remove(&old_path);
            if self.last_mark.as_deref() == Some(was) {
                self.last_mark = Some(project.path.clone());
            }
            self.meta.remove(&old_path);
        }
        self.snapshot[index] = project;
        self.rebuild_haystack(index);
        self.known_tags = known_tags(&self.snapshot);
        if self.inflight.is_some() {
            self.dirty = true;
        }
        true
    }

    pub fn remove(&mut self, path: &Path) {
        if let Some(index) = self.snapshot.iter().position(|p| p.path == path) {
            self.snapshot.remove(index);
            self.haystacks.remove(index);
            self.marks.remove(path);
            if self.last_mark.as_deref() == Some(path) {
                self.last_mark = None;
            }
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
    pub fn effective_sort(&self, query: &Query) -> Sort {
        match self.explicit_sort {
            Some(sort) => sort,
            None if !query.free.is_empty() => Sort::new(Order::Relevance),
            None => Sort::new(Order::Newest),
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
            if let Some(base) = &self.base_filter
                && &project.base != base
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
        self.widths = self.filtered.iter().fold((4, 8), |(id_w, name_w), &index| {
            let project = &self.snapshot[index];
            (
                id_w.max(UnicodeWidthStr::width(project.id.as_str())),
                name_w.max(UnicodeWidthStr::width(project.name.as_str())),
            )
        });
        let mut base_width = 4usize;
        let mut first_base: Option<&Path> = None;
        let mut many = false;
        for &index in &self.filtered {
            let base = self.snapshot[index].base.as_path();
            base_width = base_width.max(UnicodeWidthStr::width(library::base_label(base).as_str()));
            match first_base {
                None => first_base = Some(base),
                Some(seen) if seen != base => many = true,
                Some(_) => {}
            }
        }
        self.base_width = base_width;
        self.many_bases = many;

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
        sort: Sort,
        a: &(usize, Option<MatchInfo>),
        b: &(usize, Option<MatchInfo>),
    ) -> std::cmp::Ordering {
        // **The tie-break never turns round.** Two rows that the order cannot
        // tell apart are settled by date, and running that backwards too would
        // shuffle every group of equals as well as the groups themselves.
        let out = self.compare_by(sort.order, a, b);
        if sort.reversed && sort.order.reversible() {
            out.reverse()
        } else {
            out
        }
    }

    fn compare_by(
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
            Order::Id => pa
                .number()
                .cmp(&pb.number())
                .then_with(|| pa.id.cmp(&pb.id)),
            Order::Template => pa.template.cmp(&pb.template).then_with(|| newest(pa, pb)),
            // The label first, because that is the word the column shows and
            // the order has to look sorted; the full path second, because two
            // bases can share a basename (`/mnt/a/PROJECTS`, `/mnt/b/PROJECTS`)
            // and grouping them into one run is not sorting by base.
            Order::Base => library::base_label(&pa.base)
                .cmp(&library::base_label(&pb.base))
                .then_with(|| pa.base.cmp(&pb.base))
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

    /// Put the cursor on the row whose frontmatter id is `id`, if it is shown.
    pub fn select_id(&mut self, id: &str) -> bool {
        match self
            .filtered
            .iter()
            .position(|&index| self.snapshot[index].id == id)
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

    /// Mark every row between the last one Space touched and the cursor,
    /// inclusive, in the order the list is showing. A run of twenty is three
    /// keystrokes instead of twenty.
    ///
    /// **In view order, not in the snapshot's.** The rows between two rows are
    /// the rows a person can see between them; sorting by size and reaching
    /// from the first to the fourth means those four, whatever order they were
    /// discovered in.
    pub fn mark_to_here(&mut self) -> usize {
        let (Some(anchor), Some(cursor)) = (self.last_mark.clone(), self.selected) else {
            return 0;
        };
        let Some(from) = self
            .filtered
            .iter()
            .position(|&index| self.snapshot[index].path == anchor)
        else {
            return 0;
        };
        let (lo, hi) = if from <= cursor {
            (from, cursor)
        } else {
            (cursor, from)
        };
        let mut added = 0;
        for row in lo..=hi {
            let Some(project) = self.row(row) else {
                continue;
            };
            if self.marks.insert(project.path.clone()) {
                added += 1;
            }
        }
        self.last_mark = self.selected().map(|p| p.path.clone());
        added
    }

    /// Whether `v` has an anchor to reach from.
    pub fn has_anchor(&self) -> bool {
        self.last_mark.as_ref().is_some_and(|path| {
            self.filtered
                .iter()
                .any(|&i| self.snapshot[i].path == *path)
        })
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
