//! `update`'s side of the desk tab: switching to it, what it asks the
//! runtime to read, its cursor, its cuts, and the journal page it shares
//! its records with.
//!
//! The desk reads the library's rows in view and the records in the pane's
//! cache; what it adds is the reading of every record, in chunks on a
//! worker per base (`Effect::LoadDetails`), one pass over the processes for
//! who is working where (`Effect::LoadWorking`), and a refresh the runtime
//! asks for every few seconds while the desk or the journal is on screen
//! (`App::desk_refresh_effects`): a stat per record, a read only where the
//! disk disagrees.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{App, Focus, Screen};
use crate::core::desk::{Assignee, Order};
use crate::tui::app::modal::{Modal, PickItem, PickState, Then};
use crate::tui::effect::Effect;
use crate::tui::layout;

use crate::tui::app::desk::{ACTIVE_CELL, AGE_CELL, NEXT_TODO_MIN};

/// How many journal entries the page holds at most.
const JOURNAL_ROWS: usize = 500;

impl App {
    /// Switch tabs. The search bar belongs to whichever tab is showing, so
    /// switching drops the query with it — the templates' search is over
    /// different things, and the desk reads the library's own query, so a
    /// query carried over would hide most of the desk for no reason anyone
    /// typed. Leaving the desk for the library keeps the project under the
    /// cursor: the library's selection is already the desk's.
    pub(super) fn show_screen(&mut self, screen: Screen) -> Vec<Effect> {
        if self.screen == screen {
            return Vec::new();
        }
        self.screen = screen;
        self.search.editing = false;
        self.search.input.clear();
        self.search.query = Default::default();
        self.recompute();
        if screen == Screen::Desk {
            self.desk_visited = true;
        }
        let mut effects = self.after_rows_changed();
        if screen == Screen::Templates {
            let rows = self.studio.rows("");
            self.studio.reselect(&rows);
            self.studio.clamp_viewport(&rows, self.template_rows());
            if self.studio.shown.is_none()
                && let Some(slug) = self.studio.selected_slug()
            {
                effects.push(Effect::LoadTemplateView { slug });
            }
            self.offer_guide_once();
        }
        effects
    }

    /// `w`: to the desk, and `w` back to the library.
    pub(super) fn toggle_desk(&mut self) -> Vec<Effect> {
        self.show_screen(if self.screen == Screen::Desk {
            Screen::Library
        } else {
            Screen::Desk
        })
    }

    /// The desk's rows from the library in view and the records read, the
    /// cursor kept on its card — and the library's selection moved to it,
    /// since every verb and the pane read the library's.
    pub(super) fn rebuild_desk(&mut self) {
        let hat = self.hat();
        self.desk
            .rebuild(&self.library, &self.details, &self.working, hat.as_deref());
        self.sync_selection_to_desk();
        if self.desk_random_once && self.desk.placed > 0 {
            self.desk_random_once = false;
            let seed = self.elapsed_ms;
            self.desk.select_random(seed);
            self.sync_selection_to_desk();
        }
    }

    fn sync_selection_to_desk(&mut self) {
        if let Some(path) = self.desk.selected_path().map(Path::to_path_buf) {
            self.library.select_path(&path);
        }
    }

    /// After the desk's cursor moved: the library's selection follows, the
    /// viewport keeps the cursor, and the pane and the sizes follow the
    /// selection as they do on the library.
    pub(super) fn desk_moved(&mut self) -> Vec<Effect> {
        self.sync_selection_to_desk();
        let rows = self.rows_on_screen();
        self.desk.clamp_viewport(rows);
        self.after_selection_change()
    }

    /// The reads the desk is still owed: every project in view whose record
    /// is not in the cache and not already asked for, one request per base,
    /// so a base that answers slowly holds up only its own cards; and, once,
    /// who is working where.
    pub(super) fn desk_read_effects(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut by_base: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        for row in 0..self.library.len() {
            let Some(project) = self.library.row(row) else {
                continue;
            };
            if self.details.contains_key(&project.path) || self.desk.pending.contains(&project.path)
            {
                continue;
            }
            by_base
                .entry(project.base.clone())
                .or_default()
                .push(project.path.clone());
        }
        for paths in by_base.into_values() {
            self.desk.pending.extend(paths.iter().cloned());
            effects.push(Effect::LoadDetails(paths));
        }
        if !self.desk.active_asked && !self.library.is_empty() {
            self.desk.active_asked = true;
            effects.push(Effect::LoadWorking(self.paths_in_view()));
        }
        effects
    }

    /// The refresh the runtime asks for while the desk or the journal is on
    /// screen: every cached record checked against its stamp, and who is
    /// working where read again. Pure — the runtime carries it out.
    pub fn desk_refresh_effects(&self) -> Vec<Effect> {
        let mut stamps = Vec::new();
        for row in 0..self.library.len() {
            if let Some(project) = self.library.row(row)
                && let Some(detail) = self.details.get(&project.path)
            {
                stamps.push((project.path.clone(), detail.stamp));
            }
        }
        let mut effects = Vec::new();
        if !stamps.is_empty() {
            effects.push(Effect::CheckDetails(stamps));
        }
        let paths = self.paths_in_view();
        if !paths.is_empty() {
            effects.push(Effect::LoadWorking(paths));
        }
        effects
    }

    /// Whether the desk's records are wanted fresh: the desk is on screen,
    /// or the journal page is.
    pub fn desk_watching(&self) -> bool {
        self.screen == Screen::Desk || self.activity_on_journal()
    }

    fn paths_in_view(&self) -> Vec<PathBuf> {
        (0..self.library.len())
            .filter_map(|row| self.library.row(row).map(|p| p.path.clone()))
            .collect()
    }

    /// Records landed for the desk: into the cache, each the way the pane's
    /// own read lands, then the desk and the journal read again.
    pub(super) fn on_details(
        &mut self,
        read: Vec<(PathBuf, crate::tui::app::data::ProjectDetail)>,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        for (path, detail) in read {
            self.desk.pending.remove(&path);
            effects.extend(self.on_detail(path, Box::new(detail)));
        }
        self.after_records_changed();
        effects
    }

    /// Who is working where, from one pass over the processes, for the
    /// projects `asked` about: each gets its answer, an empty one included,
    /// and nothing is recorded where the platform cannot answer at all.
    pub(super) fn on_working(
        &mut self,
        asked: Vec<PathBuf>,
        active: std::collections::HashMap<PathBuf, Vec<String>>,
    ) -> Vec<Effect> {
        if !crate::core::holders::WORKING_KNOWN {
            return Vec::new();
        }
        let mut changed = false;
        for path in asked {
            let names = active.get(&path).cloned().unwrap_or_default();
            if self.working.get(&path) != Some(&names) {
                self.working.insert(path, names);
                changed = true;
            }
        }
        if changed {
            self.after_records_changed();
            // The pane's glance names who is working: its rows changed
            // under the cursor, which finds its row again.
            self.refind_pane();
        }
        Vec::new()
    }

    /// Something a record said changed: the desk's rows and the journal
    /// page follow, wherever they are on screen.
    pub(super) fn after_records_changed(&mut self) {
        if self.screen == Screen::Desk {
            self.rebuild_desk();
            let rows = self.rows_on_screen();
            self.desk.clamp_viewport(rows);
        }
        self.refresh_activity_journal();
    }

    /// `f`: the group the cursor's card sits in, alone; `F` and Esc take it
    /// off again.
    pub(super) fn show_group_alone(&mut self) -> Vec<Effect> {
        let Some(group) = self.desk.selected_group() else {
            return Vec::new();
        };
        self.set_group_filter(Some(group))
    }

    pub(super) fn set_group_filter(&mut self, group: Option<Assignee>) -> Vec<Effect> {
        self.desk.filter = group;
        self.reordered()
    }

    /// `s`: the next order within each group.
    pub(super) fn cycle_desk_order(&mut self) -> Vec<Effect> {
        self.desk.order = self.desk.order.next();
        self.info(format!("desk order: {}", self.desk.order.label()));
        self.reordered()
    }

    /// `S`: pick the order from a list.
    pub(super) fn open_desk_order_picker(&mut self) -> Vec<Effect> {
        let items = Order::ALL
            .iter()
            .map(|order| PickItem {
                label: order.label().to_string(),
                detail: order.detail().to_string(),
                value: order.label().to_string(),
            })
            .collect();
        self.modals.push(Modal::Pick(PickState::new(
            "Desk order",
            items,
            Then::DeskOrder,
        )));
        Vec::new()
    }

    /// `z`: the cursor on a card picked at random from what the cuts leave,
    /// and the row lit so the eye finds it.
    pub(super) fn desk_random(&mut self) -> Vec<Effect> {
        self.desk.picks += 1;
        let seed = self
            .elapsed_ms
            .wrapping_add(self.desk.picks.wrapping_mul(7919));
        if !self.desk.select_random(seed) {
            return Vec::new();
        }
        let effects = self.desk_moved();
        self.pulse_selected();
        effects
    }

    /// What the desk's list asks of the body: the id and the name whole,
    /// as the table measures them over the library in view, the next todo's
    /// minimum, the age and the active cell, and how many rows there are.
    pub fn desk_needs(&self) -> layout::TableNeeds {
        let (id_w, name_w) = self.library.widths;
        layout::TableNeeds {
            min_width: (2
                + 1
                + id_w
                + 1
                + name_w
                + 1
                + NEXT_TODO_MIN
                + 1
                + AGE_CELL
                + 1
                + ACTIVE_CELL
                + 1)
            .min(u16::MAX as usize) as u16,
            rows: self.desk.rows.len(),
        }
    }

    /// The desk's tab-specific keys are the library's when the pane has the
    /// focus: the pane is the library's pane.
    pub(super) fn desk_list_focused(&self) -> bool {
        self.screen == Screen::Desk && self.focus == Focus::Projects
    }

    /// The journal page: every dated note and every todo's `done:` and
    /// `added:` event across the records read, newest first, each row the
    /// day and time, the project's id, what it is and its first line — with
    /// a rule where a previous run last had the desk on screen, so what
    /// happened since you last looked reads above it.
    pub fn journal_rows(&self) -> Vec<String> {
        struct Event {
            stamp: String,
            id: String,
            kind: &'static str,
            text: String,
        }
        let mut events: Vec<Event> = Vec::new();
        for project in &self.library.snapshot {
            let Some(detail) = self.details.get(&project.path) else {
                continue;
            };
            for note in &detail.notes {
                if let Some(stamp) = note.timestamp.as_ref().filter(|_| note.is_dated()) {
                    events.push(Event {
                        stamp: stamp.clone(),
                        id: project.id.clone(),
                        kind: "note",
                        text: note.text.lines().next().unwrap_or("").to_string(),
                    });
                }
            }
            for todo in &detail.todos {
                let text = match &todo.owner {
                    Some(owner) => format!("{} @{owner}", todo.text),
                    None => todo.text.clone(),
                };
                if todo.done
                    && let Some(done) = &todo.done_at
                {
                    events.push(Event {
                        stamp: done.clone(),
                        id: project.id.clone(),
                        kind: "done",
                        text: text.clone(),
                    });
                }
                if let Some(added) = &todo.added {
                    events.push(Event {
                        stamp: added.clone(),
                        id: project.id.clone(),
                        kind: "added",
                        text,
                    });
                }
            }
        }
        if events.is_empty() {
            return vec![if self.details.is_empty() && !self.library.is_empty() {
                "reading…".to_string()
            } else {
                "Nothing dated yet: a note or a todo ticked by fastf is an entry here.".to_string()
            }];
        }
        events.sort_by(|a, b| b.stamp.cmp(&a.stamp));
        events.truncate(JOURNAL_ROWS);
        let id_w = events
            .iter()
            .map(|e| e.id.chars().count())
            .max()
            .unwrap_or(0);
        let g = self.theme.glyphs;
        let when = |stamp: &str| {
            format!(
                "{} {}",
                stamp.get(..10).unwrap_or(stamp),
                stamp.get(11..16).unwrap_or("")
            )
        };
        let mut rows = Vec::with_capacity(events.len() + 1);
        let mut rule_drawn = self.desk_seen_at.is_none();
        for event in &events {
            if !rule_drawn
                && let Some(seen) = &self.desk_seen_at
                && event.stamp.as_str() <= seen.as_str()
            {
                rows.push(format!(
                    "{} the desk was last open {} {}",
                    g.rule.repeat(4),
                    when(seen),
                    g.rule.repeat(4)
                ));
                rule_drawn = true;
            }
            rows.push(format!(
                "{}  {:<id_w$}  {:<5}  {}",
                when(&event.stamp),
                event.id,
                event.kind,
                event.text
            ));
        }
        if !rule_drawn && let Some(seen) = &self.desk_seen_at {
            rows.push(format!(
                "{} the desk was last open {} {}",
                g.rule.repeat(4),
                when(seen),
                g.rule.repeat(4)
            ));
        }
        rows
    }

    /// Put the journal on the activity screen, if it is open.
    pub(super) fn refresh_activity_journal(&mut self) {
        if !self.activity_open() {
            return;
        }
        let rows = self.journal_rows();
        if let Some(Modal::Activity(activity)) = self.modals.top_mut() {
            activity.journal = rows;
        }
    }

    /// Whether the activity screen is open on its journal page.
    pub fn activity_on_journal(&self) -> bool {
        matches!(
            self.modals.top(),
            Some(Modal::Activity(activity))
                if activity.page == crate::tui::app::modal::ActivityPage::Journal
        )
    }
}
