//! The pane's rows as the app asks for them, and its cursor: moved, paged, and
//! found again by what it was on.

use super::App;
use super::data::ProjectDetail;
use super::pane::*;
use crate::tui::effect::Effect;

/// Whether a hold made on the todo at `ordinal`, which read `text`, is
/// still about that todo: an ordinal names whatever todo sits there now.
fn still_reads(detail: &ProjectDetail, ordinal: usize, text: &str) -> bool {
    detail
        .todos
        .get(ordinal)
        .is_some_and(|todo| todo.text == text)
}

impl App {
    /// The pane's rows for the selected project — `pane_rows` over what
    /// has been read of it, wrapped to the pane as it is drawn. Empty with
    /// nothing selected.
    pub fn pane_rows(&self) -> Vec<PaneRow> {
        let Some(project) = self.library.selected() else {
            return Vec::new();
        };
        let width = self
            .regions()
            .detail
            .map(|pane| crate::tui::layout::pane_text(pane).width as usize)
            .unwrap_or(0);
        let glance = Glance {
            newest: self
                .library
                .sizes
                .get(&project.path)
                .and_then(|facts| facts.as_ref())
                .and_then(|facts| facts.newest.as_ref())
                .map(|(at, path)| {
                    (
                        crate::util::time::iso8601_of(*at),
                        crate::util::paths::display_path(path),
                    )
                }),
            working: self.working.get(&project.path).cloned(),
        };
        let detail = self.details.get(&project.path);
        let fold = TodoFold {
            shown: self.done_shown.contains(&project.path),
            // The holds are this visit's, and each only while its ordinal
            // still reads the text it was toggled with.
            held: match detail {
                Some(detail) if self.pane_for.as_ref() == Some(&project.path) => self
                    .pane_held
                    .iter()
                    .filter(|(ordinal, (_, text))| still_reads(detail, **ordinal, text))
                    .map(|(ordinal, (at, _))| (*ordinal, *at))
                    .collect(),
                _ => Default::default(),
            },
        };
        let rows = pane_rows(project, detail, &glance, &fold, width);
        match &self.pane_edit {
            Some(PaneEdit::Line {
                target: EditTarget::NewTodo { place, .. },
                ..
            }) => with_adding(rows, place),
            Some(PaneEdit::Line {
                target: EditTarget::NewPhase { .. },
                ..
            }) => with_naming(rows),
            _ => rows,
        }
    }

    /// Enter on a fold: show the selected project's done todos under every
    /// fold, or hide them, for the rest of the session. The cursor stays on
    /// the fold. Hiding drops the holds under the folds — an untick held
    /// there would otherwise vanish with them — and the pulses go, since
    /// they are kept by row and the rows below have moved.
    pub(super) fn toggle_done_shown(&mut self, group: usize) {
        let Some(path) = self.library.selected().map(|p| p.path.clone()) else {
            return;
        };
        if !self.done_shown.remove(&path) {
            self.done_shown.insert(path);
        } else {
            self.pane_held.retain(|_, (at, _)| *at == HeldAt::OpenRun);
        }
        self.pane_pulses.clear();
        self.pane_anchor = Some(PaneTarget::Fold(group));
        self.refind_pane();
    }

    /// Remember where the todo at `ordinal` was drawn when it was toggled
    /// from row `at`, with the text it read: its row stays there until the
    /// selection leaves the project.
    pub(super) fn hold_todo(&mut self, rows: &[PaneRow], at: usize, ordinal: usize, text: String) {
        self.pane_held
            .entry(ordinal)
            .or_insert_with(|| (held_at(rows, at), text));
    }

    /// A re-read of the pane's project has landed: drop every hold whose
    /// ordinal no longer reads the text it was toggled with — an add or a
    /// removal renumbered the list, by this pane or anybody — since a hold
    /// on an ordinal pins whatever todo is there now.
    pub(super) fn release_moved_holds(&mut self, detail: &ProjectDetail) {
        self.pane_held
            .retain(|ordinal, (_, text)| still_reads(detail, *ordinal, text));
    }

    /// How many rows the pane shows at once: the height of its text.
    pub(super) fn pane_rows_on_screen(&self) -> usize {
        self.regions()
            .detail
            .map(|pane| crate::tui::layout::pane_text(pane).height as usize)
            .unwrap_or(0)
    }

    /// Put the pane's cursor back on the row an edit was about, pulse it, and
    /// keep it in view. Nothing happens when the row is not there (yet).
    pub(super) fn settle_pane_cursor(&mut self, target: &PaneTarget) {
        let rows = self.pane_rows();
        let Some(row) = find_row(&rows, target) else {
            return;
        };
        self.pane_cursor = row;
        self.pane_anchor = Some(target.clone());
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
    pub(super) fn move_pane_cursor(&mut self, delta: isize) {
        let rows = self.pane_rows();
        self.pane_cursor = step_cursor(&rows, self.pane_cursor, delta);
        self.pane_anchor = target_at(&rows, self.pane_cursor);
        self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
            self.detail_scroll,
            Some(self.pane_cursor),
            rows.len(),
            self.pane_rows_on_screen(),
        );
    }

    /// Page the pane's cursor by `delta_rows` drawn rows (`page_cursor`) and
    /// keep it in view.
    pub(super) fn page_pane_cursor(&mut self, delta_rows: isize) {
        let rows = self.pane_rows();
        self.pane_cursor = page_cursor(&rows, self.pane_cursor, delta_rows);
        self.pane_anchor = target_at(&rows, self.pane_cursor);
        self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
            self.detail_scroll,
            Some(self.pane_cursor),
            rows.len(),
            self.pane_rows_on_screen(),
        );
    }

    /// `<` and `>`: the project above or below, shown in the pane with the
    /// focus still there and the cursor in the section it was in — so a run
    /// of projects' todos can be read one after another. Moving is not a
    /// change, so nothing pulses; at the ends of the list nothing moves.
    pub(super) fn step_project_from_pane(&mut self, delta: isize) -> Vec<Effect> {
        let section = section_at(&self.pane_rows(), self.pane_cursor);
        let effects = if self.screen == super::Screen::Desk {
            let before = self.desk.selected;
            self.desk.step(delta);
            if self.desk.selected == before {
                return Vec::new();
            }
            self.desk_moved()
        } else {
            let before = self.library.selected_index();
            self.library.step(delta);
            if self.library.selected_index() == before {
                return Vec::new();
            }
            self.after_selection_change()
        };
        self.pane_seek = Some(section);
        self.land_pane_seek();
        effects
    }

    /// Put the cursor in the section `<` or `>` asked for, once that section
    /// is among the rows — at once when the next project's detail is cached,
    /// or when its read lands. Its first item, or the row that adds one.
    pub(super) fn land_pane_seek(&mut self) {
        let Some(section) = self.pane_seek else {
            return;
        };
        let rows = self.pane_rows();
        let Some(row) = first_in_section(&rows, section) else {
            // Read, and the project has no such section: the seek is over,
            // so a later refresh that brings one in moves nothing unasked.
            if !rows.contains(&PaneRow::Reading) {
                self.pane_seek = None;
            }
            return;
        };
        self.pane_seek = None;
        self.pane_cursor = row;
        self.pane_anchor = target_at(&rows, row);
        self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
            self.detail_scroll,
            Some(row),
            rows.len(),
            self.pane_rows_on_screen(),
        );
    }

    /// Find the cursor and an open edit again after the rows were rebuilt
    /// under them — a re-read, a re-wrap at a new width, a tag that changed
    /// above — by what they are on, and keep them in view. No pulse: nothing
    /// the cursor is on changed.
    ///
    /// The anchor is kept when its row is not there (yet): while the detail
    /// is being read a todo has no row, and the cursor waits on the nearest
    /// one rather than forgetting where it was going.
    pub(super) fn refind_pane(&mut self) {
        let rows = self.pane_rows();
        if rows.is_empty() {
            self.pane_cursor = 0;
            self.detail_scroll = 0;
            return;
        }
        let found = self
            .pane_anchor
            .as_ref()
            .and_then(|anchor| find_row(&rows, anchor));
        self.pane_cursor = match found {
            Some(row) => row,
            None => step_cursor(&rows, self.pane_cursor.min(rows.len() - 1), 0),
        };
        if let Some(edit) = &self.pane_edit
            && let Some(row) = find_row(&rows, &edit.anchor())
            && let Some(edit) = &mut self.pane_edit
        {
            edit.set_row(row);
            // The cursor is where the typing is.
            self.pane_cursor = row;
        }
        // An open edit is what must stay in view; otherwise the cursor.
        let keep = self
            .pane_edit
            .as_ref()
            .map_or(self.pane_cursor, PaneEdit::row);
        self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
            self.detail_scroll,
            Some(keep),
            rows.len(),
            self.pane_rows_on_screen(),
        );
    }
}
