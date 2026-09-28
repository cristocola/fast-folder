//! The list as the app keeps it: what discovery brings, what a change does to
//! the rows, and the selection and pane that follow.

use super::*;

impl App {
    /// Rows landed — a whole library, or one base's.
    pub(super) fn rows_arrived(&mut self) -> Vec<Effect> {
        self.recompute();
        let mut effects = self.refresh_templates();
        // A create or a register asked for its new project to be selected;
        // it exists only once discovery has seen it.
        if let Some(path) = self.select_when_found.clone()
            && self.library.select_path(&path)
        {
            self.select_when_found = None;
        }
        // The remembered row, as soon as the base that holds it answers.
        if let Some(id) = self.select_id_when_found.clone()
            && self.library.select_id(&id)
        {
            self.select_id_when_found = None;
        }
        effects.extend(self.after_rows_changed());
        effects
    }

    /// A discovery ended. The remembered row is looked for no longer: a
    /// later discovery is a reload, and the cursor is wherever the user has
    /// since put it. A change made meanwhile asks for one more.
    pub(super) fn discovery_over(&mut self) -> Vec<Effect> {
        self.select_id_when_found = None;
        if self.library.dirty {
            self.library.dirty = false;
            return vec![self.discover()];
        }
        Vec::new()
    }

    /// **One part of the summary** (`SummaryPart`), unless a later read's
    /// part is already in: a slow read answering after a newer one must not
    /// put an older template list or base list back.
    pub(super) fn install_summary_part(
        &mut self,
        generation: u64,
        part: SummaryPart,
    ) -> Vec<Effect> {
        let slot = part.slot();
        if generation < self.summary_seen[slot] {
            return Vec::new();
        }
        self.summary_seen[slot] = generation;
        self.summary_error = None;
        let summary = self.summary.get_or_insert_with(|| Summary {
            probing: true,
            ..Summary::default()
        });
        match part {
            SummaryPart::Local { templates, prefs } => {
                summary.templates = templates;
                summary.prefs = prefs;
                // A template written or deleted is a change to the templates
                // tab's list, whether or not that tab is the one on screen.
                self.refresh_templates()
            }
            SummaryPart::Bases {
                bases,
                projects,
                max_id,
                newest,
            } => {
                summary.bases = bases;
                summary.projects = projects;
                summary.max_id = max_id;
                summary.newest = newest;
                summary.probing = false;
                Vec::new()
            }
            SummaryPart::Attention(attention) => {
                summary.attention = attention;
                self.maybe_finish_leftovers()
            }
        }
    }

    pub(super) fn discover(&mut self) -> Effect {
        self.next_generation += 1;
        self.library.inflight = Some(self.next_generation);
        Effect::Discover {
            generation: self.next_generation,
        }
    }

    pub(super) fn recompute(&mut self) {
        self.library.recompute(&self.search.query, &mut self.fuzzy);
    }

    /// After anything that changed which rows are shown.
    pub(super) fn after_rows_changed(&mut self) -> Vec<Effect> {
        // Long names can close the pane (`layout::regions`); the focus cannot
        // stay on a pane that is not drawn.
        if self.focus == Focus::Detail && !self.pane_present() {
            self.set_focus(Focus::Projects);
        }
        let rows = self.rows_on_screen();
        self.library.clamp_viewport(rows);
        // Another project under the cursor starts the pane afresh; the same
        // one keeps its place, found again among rows that may have changed.
        if !self.sync_pane() {
            self.refind_pane();
        }

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
    pub(super) fn after_selection_change(&mut self) -> Vec<Effect> {
        let rows = self.rows_on_screen();
        self.library.clamp_viewport(rows);
        if !self.sync_pane() {
            self.refind_pane();
        }
        self.selection_effects()
    }

    /// Start the pane afresh when the selection is another project than the
    /// one its cursor, scroll, pulses and open edit belong to. Returns
    /// whether it did. An edit belongs to its project: moving off it leaves
    /// the row as it was.
    fn sync_pane(&mut self) -> bool {
        let now = self.library.selected().map(|p| p.path.clone());
        if now == self.pane_for {
            return false;
        }
        self.pane_for = now;
        if let Some(pane::PaneEdit::Line {
            target: pane::EditTarget::NewTodo { queued, .. },
            ..
        }) = &self.pane_edit
            && !queued.is_empty()
        {
            let unsent = queued.join(", ");
            self.warn(format!("not added: {unsent}"));
        }
        self.detail_scroll = 0;
        self.pane_cursor = 0;
        self.pane_anchor = None;
        self.pane_edit = None;
        self.pane_return = None;
        self.pane_pending = None;
        self.pane_seek = None;
        self.pane_pulses.clear();
        true
    }

    /// The window changed size. Everything that is measured moves with it —
    /// the table's viewport, the templates list's, the pane's wrapping — and
    /// nothing that was being done is lost: the pane's cursor and an open edit
    /// are found again by what they are on. The focus leaves the pane only if
    /// there is no pane left to be in.
    ///
    /// A resize to the size the app already has is nothing at all: one comes
    /// back from every `$EDITOR` note and every `fg`, and it used to close
    /// whatever edit was open in the pane.
    pub(super) fn on_resize(&mut self, width: u16, height: u16) -> Vec<Effect> {
        if (width, height) == self.size {
            return Vec::new();
        }
        self.size = (width, height);
        if self.focus == Focus::Detail && !self.pane_present() {
            self.set_focus(Focus::Projects);
        }
        // Pulses are keyed by row index, and the rows are about to re-wrap.
        self.pane_pulses.clear();
        let rows = self.rows_on_screen();
        self.library.clamp_viewport(rows);
        if self.screen == Screen::Templates {
            let rows = self.studio.rows(self.search.input.text());
            self.studio.clamp_viewport(&rows, self.template_rows());
            self.studio.scroll = self.studio.scroll.min(self.studio_scroll_max());
        }
        self.refind_pane();
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
        if self.pane_live()
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
    pub(super) fn detail_effect(&self, path: &Path) -> Effect {
        match self.details.get(path) {
            Some(detail) => Effect::RefreshDetail {
                path: path.to_path_buf(),
                stamp: detail.stamp,
            },
            None => Effect::LoadDetail(path.to_path_buf()),
        }
    }

    pub(super) fn after_query_change(&mut self) -> Vec<Effect> {
        // On the templates tab the bar filters the template list, which is a
        // plain substring over a handful of slugs — no grammar, no metadata
        // reads, and the cursor kept on a row the query still keeps.
        if self.screen == Screen::Templates {
            let rows = self.studio.rows(self.search.input.text());
            self.studio.reselect(&rows);
            self.studio.clamp_viewport(&rows, self.template_rows());
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
    pub(super) fn refresh_templates(&mut self) -> Vec<Effect> {
        self.templates
            .rebuild(self.summary.as_ref(), self.library.per_template());
        let cards = self.templates.cards.clone();
        self.studio.install(cards)
    }

    pub(super) fn set_template_filter(&mut self, slug: Option<String>) -> Vec<Effect> {
        self.library.template_filter = slug;
        self.reordered()
    }

    pub(super) fn set_base_filter(&mut self, base: Option<PathBuf>) -> Vec<Effect> {
        self.library.base_filter = base;
        self.reordered()
    }

    /// Both row filters off. `F` is one key because they are one question —
    /// "why am I not seeing everything" — and answering half of it leaves the
    /// list still short with no hint which half is left.
    pub(super) fn clear_filters(&mut self) -> Vec<Effect> {
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
    pub(super) fn reordered(&mut self) -> Vec<Effect> {
        self.recompute();
        self.pulse_selected();
        self.after_rows_changed()
    }

    pub(super) fn pulse_selected(&mut self) {
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
                if self.pane_live() && self.library.selected().is_some_and(|p| p.path == path) {
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
}
