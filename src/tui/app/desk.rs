//! The desk tab's own state: the library's rows grouped by who each
//! project's next open todo is assigned to, the cursor over them, the group
//! shown alone and the order within a group.
//!
//! The desk is a second reading of the library, not a second library: its
//! cards are the library's rows in view — the same bases' view, the same
//! query and filters the search bar reports — each with the record the pane's
//! cache (`App.details`) holds for it, grouped by `core::desk`. A row the
//! cache has no record for yet sits under a `reading` heading until its read
//! lands, so nothing is ever claimed about a project nobody has read.
//!
//! **The desk's cursor drives the library's selection** (`App::desk_moved`),
//! so every project verb, the pane and the action menu read the desk's row
//! without a second copy of "which project is selected".

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::core::desk::{self, Assignee, Card, Order};
use crate::tui::app::data::ProjectDetail;
use crate::tui::app::library::LibraryState;
use crate::tui::widgets::nav;

/// The least the next-todo column may have before the desk claims no room
/// beside its pane: a phase and a few words.
pub const NEXT_TODO_MIN: usize = 18;
/// The age cell: `12 d`, `3 mo`, right-aligned.
pub const AGE_CELL: usize = 6;
/// The cell after the age: `active`, or the warning glyph.
pub const ACTIVE_CELL: usize = 6;

/// One row of the desk: a group's heading, or a project's card under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeskRow {
    /// A group's heading with how many projects sit under it; `None` is the
    /// heading over the projects not read yet. Never selectable.
    Heading {
        group: Option<Assignee>,
        count: usize,
    },
    /// One project. `read` is false under the `reading` heading, where the
    /// card holds no todos because none have been read.
    Card { card: Box<Card>, read: bool },
}

impl DeskRow {
    pub fn card(&self) -> Option<&Card> {
        match self {
            DeskRow::Card { card, .. } => Some(card),
            DeskRow::Heading { .. } => None,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.card().map(|card| card.project.path.as_path())
    }
}

#[derive(Debug, Default)]
pub struct DeskState {
    /// The grouped, filtered rows, headings among them.
    pub rows: Vec<DeskRow>,
    /// The index into `rows` of the card under the cursor; never a heading.
    pub selected: Option<usize>,
    pub offset: usize,
    /// The group shown alone, if one is (`f`); every group otherwise.
    pub filter: Option<Assignee>,
    pub order: Order,
    /// Whether the pass over the processes has ever been asked for.
    pub active_asked: bool,
    /// Records asked for and not landed yet, so a second discovery of the
    /// same rows asks for none of them again.
    pub pending: BTreeSet<PathBuf>,
    /// How many cards sit under a group, and how many wait to be read:
    /// the bar's count and its `reading` chip.
    pub placed: usize,
    pub unread: usize,
    /// How many times `z` has been pressed: salt for the pick, so two
    /// presses in one millisecond are two picks.
    pub picks: u64,
}

impl DeskState {
    /// The rows from the library in view and the records read so far, the
    /// cursor kept on the card it was on. `hat` decides which group leads
    /// and is called `yours`.
    pub fn rebuild(
        &mut self,
        library: &LibraryState,
        details: &HashMap<PathBuf, ProjectDetail>,
        working: &HashMap<PathBuf, Vec<String>>,
        hat: Option<&str>,
    ) {
        let was = self.selected_path().map(Path::to_path_buf);
        let was_at = self.selected;
        let mut cards = Vec::new();
        let mut unread = Vec::new();
        for row in 0..library.len() {
            let Some(project) = library.row(row) else {
                continue;
            };
            let active = working.get(&project.path).cloned().unwrap_or_default();
            match details.get(&project.path) {
                Some(detail) => {
                    let newest = library
                        .sizes
                        .get(&project.path)
                        .and_then(|facts| facts.as_ref())
                        .and_then(|facts| facts.newest_stamp());
                    let quiet_since = quiet_since(detail.touched.as_deref(), newest);
                    cards.push(Card::of(
                        project.clone(),
                        &detail.todos,
                        active,
                        quiet_since,
                    ));
                }
                None => unread.push(Card::of(project.clone(), &[], active, None)),
            }
        }
        self.placed = cards.len();
        self.unread = unread.len();
        let mut groups = desk::group(cards, hat, self.order);
        if let Some(only) = &self.filter {
            groups.retain(|group| group.assignee == *only);
        }
        self.rows.clear();
        for group in groups {
            self.rows.push(DeskRow::Heading {
                group: Some(group.assignee),
                count: group.cards.len(),
            });
            for card in group.cards {
                self.rows.push(DeskRow::Card {
                    card: Box::new(card),
                    read: true,
                });
            }
        }
        // What is not read yet is listed only when every group is: a group
        // shown alone is a claim, and an unread project is not yet in one.
        if !unread.is_empty() && self.filter.is_none() {
            self.rows.push(DeskRow::Heading {
                group: None,
                count: unread.len(),
            });
            for card in unread {
                self.rows.push(DeskRow::Card {
                    card: Box::new(card),
                    read: false,
                });
            }
        }
        self.selected = match was.as_deref().and_then(|path| self.index_of(path)) {
            Some(at) => Some(at),
            // Its card is gone: the nearest card at or after where the cursor
            // was, else the last one. With no cursor yet, the first card.
            None if was.is_some() => was_at
                .and_then(|at| self.card_at_or_after(at))
                .or_else(|| self.card_indices().last().copied()),
            None => self.card_indices().first().copied(),
        };
    }

    fn card_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, DeskRow::Card { .. }))
            .map(|(index, _)| index)
            .collect()
    }

    fn card_at_or_after(&self, at: usize) -> Option<usize> {
        (at..self.rows.len()).find(|&index| matches!(self.rows[index], DeskRow::Card { .. }))
    }

    fn index_of(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|row| row.path() == Some(path))
    }

    pub fn selected_row(&self) -> Option<&DeskRow> {
        self.selected.and_then(|at| self.rows.get(at))
    }

    pub fn selected_card(&self) -> Option<&Card> {
        self.selected_row().and_then(DeskRow::card)
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_row().and_then(DeskRow::path)
    }

    /// The group the cursor's card sits in: the heading above it.
    pub fn selected_group(&self) -> Option<Assignee> {
        let at = self.selected?;
        self.rows[..at].iter().rev().find_map(|row| match row {
            DeskRow::Heading { group, .. } => Some(group.clone()),
            DeskRow::Card { .. } => None,
        })?
    }

    /// Whether there is a card under the cursor that has been read.
    pub fn selected_is_read(&self) -> bool {
        matches!(self.selected_row(), Some(DeskRow::Card { read: true, .. }))
    }

    /// `delta` cards up or down, headings skipped, stopping at the ends.
    pub fn step(&mut self, delta: isize) {
        let cards = self.card_indices();
        if cards.is_empty() {
            self.selected = None;
            return;
        }
        let at = self
            .selected
            .and_then(|selected| cards.iter().position(|&index| index == selected));
        let next = nav::step(at, cards.len(), delta).unwrap_or(0);
        self.selected = Some(cards[next]);
    }

    pub fn select_first(&mut self) {
        self.selected = self.card_indices().first().copied();
    }

    pub fn select_last(&mut self) {
        self.selected = self.card_indices().last().copied();
    }

    pub fn select_path(&mut self, path: &Path) -> bool {
        match self.index_of(path) {
            Some(at) => {
                self.selected = Some(at);
                true
            }
            None => false,
        }
    }

    /// A card picked from the rows by `seed`, for `z`: the cursor lands on
    /// it. The seed is the app's clock in milliseconds, which is as random
    /// as a pick needs and still a number a test can set.
    pub fn select_random(&mut self, seed: u64) -> bool {
        let cards = self.card_indices();
        if cards.is_empty() {
            return false;
        }
        let at = (seed.wrapping_mul(2_654_435_761) % cards.len() as u64) as usize;
        self.selected = Some(cards[at]);
        true
    }

    /// Keep the cursor on screen for a list `rows` high — with the heading
    /// above it when the cursor is the first card under one, so a group is
    /// never scrolled to without its name.
    pub fn clamp_viewport(&mut self, rows: usize) {
        self.offset = nav::viewport_offset(self.offset, self.selected, self.rows.len(), rows);
        if let Some(at) = self.selected
            && at == self.offset
            && at > 0
            && matches!(self.rows[at - 1], DeskRow::Heading { .. })
        {
            self.offset = at - 1;
        }
    }

    /// The projects a list `rows` high shows, the cursor's first.
    pub fn visible_paths(&self, rows: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(path) = self.selected_path() {
            out.push(path.to_path_buf());
        }
        for row in self.offset..(self.offset + rows).min(self.rows.len()) {
            if let Some(path) = self.rows[row].path()
                && !out.iter().any(|seen| seen == path)
            {
                out.push(path.to_path_buf());
            }
        }
        out
    }

    /// The groups the rows hold, in their order, for a picker.
    pub fn groups(&self) -> Vec<Assignee> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                DeskRow::Heading {
                    group: Some(group), ..
                } => Some(group.clone()),
                _ => None,
            })
            .collect()
    }
}

/// When a project last moved: the newer of `touched` (its record and its
/// folder) and the newest file a walk found under it, as stamps.
fn quiet_since(touched: Option<&str>, newest: Option<String>) -> Option<String> {
    match (touched, newest) {
        (Some(a), Some(b)) => Some(if b.as_str() > a { b } else { a.to_string() }),
        (Some(a), None) => Some(a.to_string()),
        (None, b) => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(n: usize) -> LibraryState {
        let mut library = LibraryState::new();
        library.install_initial(crate::tui::testing::sample_projects(n));
        library.recompute(&Default::default(), &mut crate::tui::fuzzy::Fuzzy::new());
        library
    }

    fn record(todos: &[(bool, &str, Option<&str>)]) -> ProjectDetail {
        ProjectDetail {
            todos: todos
                .iter()
                .map(|(done, text, owner)| crate::core::body::Todo {
                    done: *done,
                    text: text.to_string(),
                    owner: owner.map(str::to_string),
                    ..Default::default()
                })
                .collect(),
            touched: Some("2026-01-10T10:00:00Z".to_string()),
            ..Default::default()
        }
    }

    /// Projects not read yet wait under their own heading at the end, and a
    /// group shown alone lists none of them.
    #[test]
    fn unread_projects_wait_under_their_own_heading() {
        let library = library(3);
        let mut details = HashMap::new();
        details.insert(
            library.row(0).unwrap().path.clone(),
            record(&[(false, "send the invoice", None)]),
        );
        let mut desk = DeskState::default();
        desk.rebuild(&library, &details, &HashMap::new(), None);
        let headings: Vec<Option<Assignee>> = desk
            .rows
            .iter()
            .filter_map(|row| match row {
                DeskRow::Heading { group, .. } => Some(group.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(headings, [Some(Assignee::NoOwner), None]);
        assert_eq!((desk.placed, desk.unread), (1, 2));

        desk.filter = Some(Assignee::NoOwner);
        desk.rebuild(&library, &details, &HashMap::new(), None);
        assert_eq!(desk.rows.len(), 2, "the heading and its one card");
    }

    /// The cursor walks cards only, keeps its card across a rebuild, and
    /// lands on the nearest card when its own is gone.
    #[test]
    fn the_cursor_skips_headings_and_follows_its_card() {
        let library = library(4);
        let mut details = HashMap::new();
        for (index, owner) in [
            (0, Some("claude")),
            (1, None),
            (2, Some("claude")),
            (3, None),
        ] {
            details.insert(
                library.row(index).unwrap().path.clone(),
                record(&[(false, "do it", owner)]),
            );
        }
        let mut desk = DeskState::default();
        desk.rebuild(&library, &details, &HashMap::new(), None);
        assert!(matches!(desk.rows[0], DeskRow::Heading { .. }));
        assert_eq!(desk.selected, Some(1), "the first card, not its heading");
        desk.step(1);
        assert_eq!(desk.selected, Some(2));
        desk.step(1);
        assert_eq!(desk.selected, Some(4), "over the next heading");
        let on = desk.selected_path().unwrap().to_path_buf();

        // With the hat set to claude, @claude's group comes first; the cursor
        // stays on its card.
        desk.rebuild(&library, &details, &HashMap::new(), Some("claude"));
        assert_eq!(desk.selected_path(), Some(on.as_path()));
        assert_eq!(
            desk.selected_group(),
            Some(Assignee::Owner("claude".into()))
        );

        // Its card gone, the cursor is on the nearest card that is left.
        details.remove(&on);
        let mut without = LibraryState::new();
        without.install_initial(
            crate::tui::testing::sample_projects(4)
                .into_iter()
                .filter(|p| p.path != on)
                .collect(),
        );
        without.recompute(&Default::default(), &mut crate::tui::fuzzy::Fuzzy::new());
        desk.rebuild(&without, &details, &HashMap::new(), Some("claude"));
        assert!(desk.selected_card().is_some());
        assert_ne!(desk.selected_path(), Some(on.as_path()));
    }

    /// The viewport keeps a group's heading with its first card.
    #[test]
    fn a_group_is_never_scrolled_to_without_its_heading() {
        let library = library(6);
        let mut details = HashMap::new();
        for index in 0..6 {
            let owner = if index < 3 { Some("sales") } else { None };
            details.insert(
                library.row(index).unwrap().path.clone(),
                record(&[(false, "do it", owner)]),
            );
        }
        let mut desk = DeskState::default();
        desk.rebuild(&library, &details, &HashMap::new(), None);
        // Rows: heading, 3 cards, heading, 3 cards. Move onto the first card
        // of the second group with a window of three rows.
        desk.select_last();
        desk.step(-2);
        let at = desk.selected.unwrap();
        assert!(matches!(desk.rows[at - 1], DeskRow::Heading { .. }));
        desk.clamp_viewport(3);
        assert!(
            (desk.offset..desk.offset + 3).contains(&(at - 1)),
            "the heading at {} is on screen from {}",
            at - 1,
            desk.offset
        );
        // Scrolled down past it and back up onto the first card of the
        // group, the heading comes back with the card.
        desk.select_last();
        desk.clamp_viewport(3);
        desk.step(-2);
        desk.clamp_viewport(3);
        assert_eq!(desk.offset, at - 1);
    }

    #[test]
    fn the_newer_of_touched_and_the_newest_file_is_when_it_was_quiet_since() {
        assert_eq!(
            quiet_since(
                Some("2026-01-01T00:00:00Z"),
                Some("2026-02-01T00:00:00Z".into())
            ),
            Some("2026-02-01T00:00:00Z".to_string())
        );
        assert_eq!(
            quiet_since(
                Some("2026-03-01T00:00:00Z"),
                Some("2026-02-01T00:00:00Z".into())
            ),
            Some("2026-03-01T00:00:00Z".to_string())
        );
        assert_eq!(quiet_since(None, None), None);
    }
}
