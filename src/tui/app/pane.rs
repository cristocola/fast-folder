//! The detail pane as a list of rows.
//!
//! The pane was one `Paragraph` — a run of lines the view built as it went,
//! with `detail_scroll_max` hand-counting the same lines a second time to
//! know how far it could scroll. It is an editor now: some of its rows are
//! things you can change (the name, a tag, a variable, the notes), so *which*
//! rows exist and *which* of them the cursor may rest on has to be one answer
//! that `update` and `view` both read. `pane_rows` is that answer, and it is
//! pure: a project and what has been read of it in, rows out.
//!
//! **A row only says what it is**, and holds only what fits the pane, so an
//! edit reads the text it changes from the detail, never from a row. The
//! `impl App` below is `update`'s side of the pane — opening, sending and
//! landing an edit, and walking the cursor; what a change does to the file is
//! `core::operations`'.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::App;
use crate::core::library::Project;
use crate::core::template::VarType;
use crate::tui::app::actions::{TextPrompt, TextThen};
use crate::tui::app::data::{Entry, ProjectDetail};
use crate::tui::app::modal::{Modal, PickItem, PickState, Then};
use crate::tui::command::{self, CommandId, Context, Key};
use crate::tui::effect::{Action, Effect};
use crate::tui::validators;
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;
use crate::tui::widgets::text_area::TextArea;

/// How many entries of the folder listing the pane shows before `… n more`.
pub const LISTING_SHOWN: usize = 8;

/// How many notes the pane shows — the latest — under `… n earlier`.
pub const NOTES_SHOWN: usize = 5;

/// How many rows of one note the pane shows before `… n more lines`.
pub const NOTE_LINES_SHOWN: usize = 8;

/// The columns in front of a note's text: its date, ten wide, and a space.
pub const NOTE_INDENT: usize = 11;

/// The columns in front of a todo's text: `[ ] `.
pub const TODO_INDENT: usize = 4;

/// What kind of value a variable row holds, and so what Enter on it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VarKind {
    /// Free text, one line.
    Text,
    /// One of these, and nothing else.
    Select(Vec<String>),
}

/// One row of the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneRow {
    /// The folder name. Enter renames.
    Name,
    /// `template · base · created`.
    Facts,
    /// The size, the note count and the todo count.
    Figures,
    /// A section heading: `── label ───`. Never a row the cursor rests on:
    /// every section that can grow ends in a row that adds to it.
    Rule(&'static str),
    /// One tag. Enter edits it; emptied, it is removed.
    Tag(String),
    /// The row under the last tag that adds one.
    AddTag,
    /// The detail has not been read yet.
    Reading,
    /// A read that failed.
    Warning(String),
    /// One template variable. Enter edits its value.
    Variable {
        slug: String,
        label: String,
        kind: VarKind,
        value: String,
    },
    /// One entry of the folder listing.
    Entry(Entry),
    /// `… n more` under a listing longer than `LISTING_SHOWN`.
    More(usize),
    /// `… n earlier` above the notes shown. Enter shows them all.
    EarlierNotes(usize),
    /// A note's first row, with the day it was written (none for the
    /// undated note). Enter edits the note. `ordinal` is its index among
    /// the file's notes, which is what an edit is addressed by.
    Note {
        ordinal: usize,
        date: Option<String>,
        first: String,
    },
    /// A further row of the note above: the rest of a line too wide for the
    /// pane, or a line of its own. Not a row the cursor rests on: the note is
    /// one thing, and its first row is where Enter acts on it.
    NoteLine(String),
    /// `… n more lines` of the note above, counted in rows.
    NoteMore(usize),
    /// The row under the last note that adds one.
    AddNote,
    /// One todo's first row. Enter toggles it.
    Todo {
        ordinal: usize,
        done: bool,
        text: String,
    },
    /// The rest of a todo too wide for the pane. Not a row the cursor rests on.
    TodoLine { done: bool, text: String },
    /// The row under the last todo that adds one.
    AddTodo,
}

impl PaneRow {
    /// Whether the cursor may rest on this row — that is, whether Enter on
    /// it does something.
    pub fn selectable(&self) -> bool {
        matches!(
            self,
            PaneRow::Name
                | PaneRow::Tag(_)
                | PaneRow::AddTag
                | PaneRow::Variable { .. }
                | PaneRow::EarlierNotes(_)
                | PaneRow::Note { .. }
                | PaneRow::AddNote
                | PaneRow::Todo { .. }
                | PaneRow::AddTodo
        )
    }
}

/// The pane's rows for `project`, given what has been read of it so far.
///
/// The order is the order the pane always drew: the name, the facts, the
/// figures, the tags, then everything the detail read added — a warning if a
/// read failed, the variables, the folder's top level, the notes, the todos.
/// Tags, notes and todos are one row each so a cursor can rest on one, with
/// the row that adds one under them; their rules are drawn whenever there is
/// something to add to, which is always.
///
/// **A note is several rows, never one wrapped row.** `width` is the pane's
/// inside, and every line of a note is wrapped to the columns after its date
/// (`wrap_columns`): the first row is the one the cursor rests on and Enter
/// edits, and every further row is its own under it, up to `NOTE_LINES_SHOWN`
/// rows, then `… n more lines`. A todo wraps the same way into `TodoLine`s.
/// The cursor is an index into the rows and the scroll counts rows, so a row
/// that drew two lines would put everything under it off by one. A `width` of
/// zero wraps nothing. The latest `NOTES_SHOWN` notes are shown, under
/// `… n earlier` when there are more.
///
/// Variables follow the template's own order and carry its `type`, so a
/// `select` offers its options and nothing else; a variable the metadata
/// holds that the template no longer declares — or every variable of a
/// registered project, which has no template — is free text after them.
pub fn pane_rows(project: &Project, detail: Option<&ProjectDetail>, width: usize) -> Vec<PaneRow> {
    let mut rows = vec![PaneRow::Name, PaneRow::Facts, PaneRow::Figures];

    rows.push(PaneRow::Rule("tags"));
    rows.extend(project.tags.iter().cloned().map(PaneRow::Tag));
    rows.push(PaneRow::AddTag);

    let Some(detail) = detail else {
        rows.push(PaneRow::Reading);
        return rows;
    };
    if let Some(error) = &detail.error {
        rows.push(PaneRow::Warning(error.clone()));
    }

    if let Some(meta) = &detail.meta {
        let mut variables = Vec::new();
        for variable in &detail.variables {
            let value = meta
                .variables
                .get(&variable.slug)
                .cloned()
                .unwrap_or_default();
            let kind = match variable.var_type {
                VarType::Select => VarKind::Select(variable.options.clone()),
                VarType::Text => VarKind::Text,
            };
            variables.push(PaneRow::Variable {
                slug: variable.slug.clone(),
                label: variable.label.clone(),
                kind,
                value,
            });
        }
        for (slug, value) in &meta.variables {
            if detail.variables.iter().any(|v| &v.slug == slug) {
                continue;
            }
            variables.push(PaneRow::Variable {
                slug: slug.clone(),
                label: slug.clone(),
                kind: VarKind::Text,
                value: value.clone(),
            });
        }
        if !variables.is_empty() {
            rows.push(PaneRow::Rule("variables"));
            rows.extend(variables);
        }
    }

    if !detail.listing.is_empty() {
        rows.push(PaneRow::Rule("inside"));
        rows.extend(
            detail
                .listing
                .iter()
                .take(LISTING_SHOWN)
                .cloned()
                .map(PaneRow::Entry),
        );
        if detail.listing.len() > LISTING_SHOWN {
            rows.push(PaneRow::More(detail.listing.len() - LISTING_SHOWN));
        }
    }

    rows.push(PaneRow::Rule("notes"));
    let earlier = detail.notes.len().saturating_sub(NOTES_SHOWN);
    if earlier > 0 {
        rows.push(PaneRow::EarlierNotes(earlier));
    }
    let note_width = width.saturating_sub(NOTE_INDENT);
    for (ordinal, note) in detail.notes.iter().enumerate().skip(earlier) {
        let mut lines = note
            .text
            .lines()
            .flat_map(|line| wrap_columns(line, note_width));
        rows.push(PaneRow::Note {
            ordinal,
            date: note.day().map(str::to_string),
            first: lines.next().unwrap_or_default(),
        });
        let rest: Vec<String> = lines.collect();
        let shown = NOTE_LINES_SHOWN.saturating_sub(1);
        rows.extend(rest.iter().take(shown).cloned().map(PaneRow::NoteLine));
        if rest.len() > shown {
            rows.push(PaneRow::NoteMore(rest.len() - shown));
        }
    }
    rows.push(PaneRow::AddNote);

    rows.push(PaneRow::Rule("todo"));
    let todo_width = width.saturating_sub(TODO_INDENT);
    for (ordinal, todo) in detail.todos.iter().enumerate() {
        let mut lines = wrap_columns(&todo.text, todo_width).into_iter();
        rows.push(PaneRow::Todo {
            ordinal,
            done: todo.done,
            text: lines.next().unwrap_or_default(),
        });
        rows.extend(lines.map(|text| PaneRow::TodoLine {
            done: todo.done,
            text,
        }));
    }
    rows.push(PaneRow::AddTodo);
    rows
}

/// `text` as lines at most `width` display columns wide: broken at a space
/// where one fits, inside a word only when the word alone is wider than a
/// line. The spaces a break lands on are dropped and every other character is
/// kept, the indent a line starts with included. A `width` of zero is no limit.
fn wrap_columns(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.width() <= width {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut rest = text;
    while rest.width() > width {
        let (mut columns, mut fits, mut space, mut word) = (0, 0, None, false);
        for (at, c) in rest.char_indices() {
            let w = c.width().unwrap_or(0);
            if columns + w > width {
                break;
            }
            columns += w;
            fits = at + c.len_utf8();
            if c == ' ' {
                if word {
                    space = Some(at);
                }
            } else {
                word = true;
            }
        }
        let split = if rest[fits..].starts_with(' ') {
            fits
        } else {
            space.unwrap_or_else(|| fits.max(rest.chars().next().map_or(0, char::len_utf8)))
        };
        lines.push(rest[..split].trim_end().to_string());
        rest = rest[split..].trim_start_matches(' ');
    }
    if !rest.is_empty() {
        lines.push(rest.to_string());
    }
    lines
}

/// What a line edit in the pane is changing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// A template variable, by slug.
    Variable(String),
    /// A tag, by the text it had; emptied, it is removed.
    Tag(String),
}

/// An edit open on one row of the pane.
///
/// **Nothing edits until Enter, and Esc leaves the row as it was.** The edit
/// lives beside the rows rather than in a dialog over them so what is being
/// changed stays in view with everything around it; `pending` is set once the
/// write is on its way and cleared by the answer — an `Ok` closes the edit, an
/// `Err` lands on it as `error`, with the text still there to correct, the
/// way the builder's `saving` and a settings row's edit already work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneEdit {
    /// One line: a variable's value or a tag's text.
    Line {
        row: usize,
        target: EditTarget,
        input: LineEdit,
        error: Option<String>,
        pending: bool,
    },
    /// One note, as a text area over its rows. `ordinal` and `was` are how
    /// the write names the note: its index in the file, and the text it had
    /// when the edit opened, so a note that changed meanwhile is refused
    /// rather than overwritten. Boxed: a text area is the larger payload by
    /// a distance, and the enum travels in `App`.
    Note {
        row: usize,
        ordinal: usize,
        was: String,
        area: Box<TextArea>,
        error: Option<String>,
        pending: bool,
    },
}

impl PaneEdit {
    /// The row the edit is on.
    pub fn row(&self) -> usize {
        match self {
            PaneEdit::Line { row, .. } | PaneEdit::Note { row, .. } => *row,
        }
    }

    pub fn is_note(&self) -> bool {
        matches!(self, PaneEdit::Note { .. })
    }

    /// Move the edit to the row its target is on now — the rows were rebuilt
    /// under it.
    pub fn set_row(&mut self, at: usize) {
        match self {
            PaneEdit::Line { row, .. } | PaneEdit::Note { row, .. } => *row = at,
        }
    }

    pub fn pending(&self) -> bool {
        match self {
            PaneEdit::Line { pending, .. } | PaneEdit::Note { pending, .. } => *pending,
        }
    }

    pub fn set_pending(&mut self, on: bool) {
        match self {
            PaneEdit::Line { pending, .. } | PaneEdit::Note { pending, .. } => *pending = on,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            PaneEdit::Line { error, .. } | PaneEdit::Note { error, .. } => error.as_deref(),
        }
    }

    /// A refusal, under the field that earned it; the write is no longer
    /// pending, so the field can be corrected and sent again.
    pub fn fail(&mut self, message: String) {
        match self {
            PaneEdit::Line { error, pending, .. } | PaneEdit::Note { error, pending, .. } => {
                *error = Some(message);
                *pending = false;
            }
        }
    }

    pub fn clear_error(&mut self) {
        match self {
            PaneEdit::Line { error, .. } | PaneEdit::Note { error, .. } => *error = None,
        }
    }
}

/// What an edit was about, for finding its row again after the rows changed.
///
/// A landed edit patches the project's row and drops the cached detail, so
/// the pane's rows are rebuilt — with a tag more or less above the variable
/// that changed, or with the variables gone until the re-read lands. The
/// cursor follows the *thing*, not its old index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneTarget {
    /// A tag by its text; gone, the cursor settles on the row that adds one.
    Tag(String),
    Variable(String),
    /// A note by its ordinal; gone (removed), the row that adds one.
    Note(usize),
    /// A todo by its ordinal; gone, the row that adds one.
    Todo(usize),
    AddNote,
    AddTodo,
}

impl PaneEdit {
    /// What this edit is about, with a tag named by the text it is being
    /// given — which is where it will be once the write lands.
    pub fn target(&self) -> PaneTarget {
        match self {
            PaneEdit::Line {
                target: EditTarget::Variable(slug),
                ..
            } => PaneTarget::Variable(slug.clone()),
            PaneEdit::Line {
                target: EditTarget::Tag(_),
                input,
                ..
            } => PaneTarget::Tag(input.text().trim().to_string()),
            PaneEdit::Note { ordinal, .. } => PaneTarget::Note(*ordinal),
        }
    }
}

/// Where `target` is in `rows`, if it is: the row to put the cursor back on.
pub fn find_row(rows: &[PaneRow], target: &PaneTarget) -> Option<usize> {
    let find = |wanted: &dyn Fn(&PaneRow) -> bool| rows.iter().position(wanted);
    match target {
        PaneTarget::Tag(text) => find(&|row| matches!(row, PaneRow::Tag(tag) if tag == text))
            .or_else(|| find(&|row| matches!(row, PaneRow::AddTag))),
        PaneTarget::Variable(slug) => {
            find(&|row| matches!(row, PaneRow::Variable { slug: s, .. } if s == slug))
        }
        PaneTarget::Note(ordinal) => {
            find(&|row| matches!(row, PaneRow::Note { ordinal: o, .. } if o == ordinal))
                .or_else(|| find(&|row| matches!(row, PaneRow::AddNote)))
        }
        PaneTarget::Todo(ordinal) => {
            find(&|row| matches!(row, PaneRow::Todo { ordinal: o, .. } if o == ordinal))
                .or_else(|| find(&|row| matches!(row, PaneRow::AddTodo)))
        }
        PaneTarget::AddNote => find(&|row| matches!(row, PaneRow::AddNote)),
        PaneTarget::AddTodo => find(&|row| matches!(row, PaneRow::AddTodo)),
    }
}

/// The row the cursor lands on after moving `delta` selectable rows from
/// `from`, stopping at the ends like every list. `from` on a row that is not
/// selectable counts as the next selectable one below it, so a cursor left on
/// a row that stopped existing finds its footing rather than vanishing.
pub fn step_cursor(rows: &[PaneRow], from: usize, delta: isize) -> usize {
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(index, _)| index)
        .collect();
    if selectable.is_empty() {
        return 0;
    }
    let at = selectable
        .iter()
        .position(|index| *index >= from)
        .unwrap_or(selectable.len() - 1);
    let next = nav::step(Some(at), selectable.len(), delta).unwrap_or(0);
    selectable[next]
}

impl App {
    /// A pane edit is open: the field has first refusal on anything typed,
    /// the registry answers the rest (`Enter`, `Esc`, `Ctrl-S`), and what
    /// neither takes goes to the field — which is how Enter in the notes is a
    /// new line: `PaneEditConfirm` is hidden there, so the text area gets it.
    pub(super) fn on_pane_edit_key(&mut self, key: Key) -> Vec<Effect> {
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
            PaneEdit::Line { input, .. } => input.apply(&key),
            PaneEdit::Note { area, .. } => area.apply(&key),
        };
        if changed {
            edit.clear_error();
        }
        Vec::new()
    }

    /// Enter on a pane row: what the row is decides what opens — or, for a
    /// todo, what is written at once, since a toggle has nothing to type.
    pub(super) fn pane_edit_start(&mut self) -> Vec<Effect> {
        let rows = self.pane_rows();
        let Some(row) = rows.get(self.pane_cursor).cloned() else {
            return Vec::new();
        };
        let at = self.pane_cursor;
        match row {
            PaneRow::Name => self.run(CommandId::Rename),
            PaneRow::AddTag => self.open_add_tag(),
            PaneRow::AddNote => self.run(CommandId::NoteInline),
            PaneRow::EarlierNotes(_) => self.run(CommandId::ShowJournal),
            PaneRow::AddTodo => {
                self.modals.push(Modal::TextPrompt(TextPrompt::new(
                    validators::ADD_TODO_PROMPT,
                    TextThen::AddTodo,
                )));
                Vec::new()
            }
            PaneRow::Todo { ordinal, .. } => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                // The whole todo as it was read — the row holds only what fits.
                let Some(was) = self
                    .details
                    .get(&project.path)
                    .and_then(|detail| detail.todos.get(ordinal))
                    .map(|todo| todo.text.clone())
                else {
                    return Vec::new();
                };
                let effects = self.run_action(
                    "writing…",
                    Action::ToggleTodo {
                        project: Box::new(project),
                        ordinal,
                        was,
                    },
                );
                if !effects.is_empty() {
                    self.pane_pending = Some(PaneTarget::Todo(ordinal));
                }
                effects
            }
            PaneRow::Tag(tag) => {
                self.pane_edit = Some(PaneEdit::Line {
                    row: at,
                    input: crate::tui::widgets::input::LineEdit::with_text(tag.clone()),
                    target: EditTarget::Tag(tag),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            PaneRow::Variable {
                slug,
                label,
                kind: VarKind::Select(options),
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
            PaneRow::Variable {
                slug,
                kind: VarKind::Text,
                value,
                ..
            } => {
                self.pane_edit = Some(PaneEdit::Line {
                    row: at,
                    input: crate::tui::widgets::input::LineEdit::with_text(value),
                    target: EditTarget::Variable(slug),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            PaneRow::Note { ordinal, .. } => {
                let text = self
                    .library
                    .selected()
                    .and_then(|project| self.details.get(&project.path))
                    .and_then(|detail| detail.notes.get(ordinal))
                    .map(|note| note.text.clone())
                    .unwrap_or_default();
                self.pane_edit = Some(PaneEdit::Note {
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
    pub(super) fn pane_edit_confirm(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Line { target, input, .. }) = &self.pane_edit else {
            return Vec::new();
        };
        let text = input.text().trim().to_string();
        let Some(project) = self.library.selected().cloned() else {
            self.pane_edit = None;
            return Vec::new();
        };
        let action = match target {
            EditTarget::Variable(slug) => Action::SetVariable {
                project: Box::new(project),
                slug: slug.clone(),
                value: text,
            },
            EditTarget::Tag(from) => {
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
    pub(super) fn pane_edit_save(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Note {
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
    pub(super) fn send_pane_edit(&mut self, action: Action) -> Vec<Effect> {
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
            .map(|pane| pane.width.saturating_sub(2) as usize)
            .unwrap_or(0);
        pane_rows(project, self.details.get(&project.path), width)
    }

    /// How many rows the pane shows at once: its height inside the border.
    pub(super) fn pane_rows_on_screen(&self) -> usize {
        self.regions()
            .detail
            .map(|pane| pane.height.saturating_sub(2) as usize)
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
        self.detail_scroll = crate::tui::widgets::nav::viewport_offset(
            self.detail_scroll,
            Some(self.pane_cursor),
            rows.len(),
            self.pane_rows_on_screen(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project_info::Metadata;
    use crate::core::template::{Transform, Variable};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn project(tags: &[&str], template: &str) -> Project {
        Project {
            id: "ID0001".to_string(),
            id_number: Some(1),
            template: template.to_string(),
            template_name: template.to_string(),
            path: PathBuf::from("/mnt/projects/ID0001_One"),
            name: "ID0001_One".to_string(),
            base: PathBuf::from("/mnt/projects"),
            created: "2026-01-01T00:00:00Z".to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            exists: true,
        }
    }

    fn variable(slug: &str, var_type: VarType, options: &[&str]) -> Variable {
        Variable {
            slug: slug.to_string(),
            label: slug.to_uppercase(),
            var_type,
            required: false,
            options: options.iter().map(|o| o.to_string()).collect(),
            default: String::new(),
            transform: Transform::None,
        }
    }

    fn metadata(variables: &[(&str, &str)]) -> Metadata {
        Metadata {
            id: "ID0001".to_string(),
            id_number: Some(1),
            template: "client".to_string(),
            template_name: "Client".to_string(),
            created: String::new(),
            folder: String::new(),
            path: String::new(),
            variables: variables
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
            tags: Vec::new(),
            auto_tags: Vec::new(),
            provisioning: false,
        }
    }

    #[test]
    fn rows_are_one_per_tag_then_add_tag_and_reading_until_the_detail_lands() {
        let rows = pane_rows(&project(&["draft", "client/Acme"], "client"), None, 0);
        assert_eq!(
            rows,
            vec![
                PaneRow::Name,
                PaneRow::Facts,
                PaneRow::Figures,
                PaneRow::Rule("tags"),
                PaneRow::Tag("draft".to_string()),
                PaneRow::Tag("client/Acme".to_string()),
                PaneRow::AddTag,
                PaneRow::Reading,
            ]
        );
    }

    fn note(timestamp: Option<&str>, text: &str) -> crate::core::body::Note {
        crate::core::body::Note {
            timestamp: timestamp.map(str::to_string),
            text: text.to_string(),
        }
    }

    #[test]
    fn selectable_rows_skip_the_facts_the_rules_the_listing_and_a_notes_other_lines() {
        let detail = ProjectDetail {
            listing: vec![Entry {
                name: "src".to_string(),
                is_dir: true,
            }],
            notes: vec![
                note(None, "free text\nsecond line"),
                note(Some("2026-01-01T10:00:00Z"), "began"),
            ],
            todos: vec![crate::core::body::Todo {
                done: true,
                text: "ingested".to_string(),
            }],
            ..Default::default()
        };
        let rows = pane_rows(&project(&["draft"], "client"), Some(&detail), 0);
        let selectable: Vec<&PaneRow> = rows.iter().filter(|r| r.selectable()).collect();
        assert_eq!(
            selectable,
            vec![
                &PaneRow::Name,
                &PaneRow::Tag("draft".to_string()),
                &PaneRow::AddTag,
                &PaneRow::Note {
                    ordinal: 0,
                    date: None,
                    first: "free text".to_string(),
                },
                &PaneRow::Note {
                    ordinal: 1,
                    date: Some("2026-01-01".to_string()),
                    first: "began".to_string(),
                },
                &PaneRow::AddNote,
                &PaneRow::Todo {
                    ordinal: 0,
                    done: true,
                    text: "ingested".to_string(),
                },
                &PaneRow::AddTodo,
            ]
        );
        assert!(rows.contains(&PaneRow::Rule("inside")));
        assert!(rows.contains(&PaneRow::NoteLine("second line".to_string())));
        assert!(!rows.contains(&PaneRow::Reading));
        // The rows of a note sit together, under one rule, over the todos.
        let at = |wanted: &PaneRow| rows.iter().position(|r| r == wanted).unwrap();
        assert_eq!(
            at(&PaneRow::NoteLine("second line".to_string())),
            at(&PaneRow::Note {
                ordinal: 0,
                date: None,
                first: "free text".to_string()
            }) + 1
        );
        assert!(at(&PaneRow::Rule("notes")) < at(&PaneRow::AddNote));
        assert!(at(&PaneRow::AddNote) < at(&PaneRow::Rule("todo")));
        assert!(at(&PaneRow::Rule("todo")) < at(&PaneRow::AddTodo));
    }

    #[test]
    fn the_latest_notes_are_shown_under_earlier_and_a_long_note_is_cut_with_more() {
        let long = (0..12)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let detail = ProjectDetail {
            notes: (0..8)
                .map(|n| note(Some("2026-01-01"), if n == 7 { &long } else { "short" }))
                .collect(),
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
        assert!(rows.contains(&PaneRow::EarlierNotes(3)));
        let shown: Vec<usize> = rows
            .iter()
            .filter_map(|r| match r {
                PaneRow::Note { ordinal, .. } => Some(*ordinal),
                _ => None,
            })
            .collect();
        assert_eq!(shown, vec![3, 4, 5, 6, 7], "the latest, in file order");
        let lines = rows
            .iter()
            .filter(|r| matches!(r, PaneRow::NoteLine(_)))
            .count();
        assert_eq!(lines, NOTE_LINES_SHOWN - 1);
        assert!(rows.contains(&PaneRow::NoteMore(12 - NOTE_LINES_SHOWN)));
        assert_eq!(
            rows.iter()
                .position(|r| matches!(r, PaneRow::EarlierNotes(_))),
            Some(
                rows.iter()
                    .position(|r| r == &PaneRow::Rule("notes"))
                    .unwrap()
                    + 1
            )
        );
    }

    #[test]
    fn variables_follow_the_template_and_a_select_offers_only_its_options() {
        let detail = ProjectDetail {
            meta: Some(metadata(&[
                ("tier", "Indie"),
                ("name", "One"),
                ("extra", "x"),
            ])),
            variables: vec![
                variable("name", VarType::Text, &[]),
                variable("tier", VarType::Select, &["Indie", "Major"]),
            ],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
        let variables: Vec<&PaneRow> = rows
            .iter()
            .filter(|r| matches!(r, PaneRow::Variable { .. }))
            .collect();
        assert_eq!(variables.len(), 3);
        assert_eq!(
            variables[0],
            &PaneRow::Variable {
                slug: "name".to_string(),
                label: "NAME".to_string(),
                kind: VarKind::Text,
                value: "One".to_string(),
            },
            "the template's order, not the alphabet's"
        );
        assert_eq!(
            variables[1],
            &PaneRow::Variable {
                slug: "tier".to_string(),
                label: "TIER".to_string(),
                kind: VarKind::Select(vec!["Indie".to_string(), "Major".to_string()]),
                value: "Indie".to_string(),
            }
        );
        assert_eq!(
            variables[2],
            &PaneRow::Variable {
                slug: "extra".to_string(),
                label: "extra".to_string(),
                kind: VarKind::Text,
                value: "x".to_string(),
            },
            "a variable the template no longer declares is free text, after the others"
        );
    }

    #[test]
    fn a_registered_project_offers_every_variable_as_text() {
        let detail = ProjectDetail {
            meta: Some(metadata(&[("b", "2"), ("a", "1")])),
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "(registered)"), Some(&detail), 0);
        let kinds: Vec<(&str, &VarKind)> = rows
            .iter()
            .filter_map(|r| match r {
                PaneRow::Variable { slug, kind, .. } => Some((slug.as_str(), kind)),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec![("a", &VarKind::Text), ("b", &VarKind::Text)]);
    }

    #[test]
    fn the_cursor_walks_selectable_rows_and_stops() {
        let rows = pane_rows(&project(&["draft"], "client"), None, 0);
        // Name(0) Facts Figures Rule Tag(4) AddTag(5) Reading
        assert_eq!(step_cursor(&rows, 0, 1), 4, "over the facts and the rule");
        assert_eq!(step_cursor(&rows, 4, 1), 5);
        assert_eq!(step_cursor(&rows, 5, 1), 5, "the end is the end");
        assert_eq!(step_cursor(&rows, 5, -1), 4);
        assert_eq!(step_cursor(&rows, 0, -1), 0);
        assert_eq!(step_cursor(&rows, 0, isize::MAX), 5);
        assert_eq!(step_cursor(&rows, 5, isize::MIN), 0);
        assert_eq!(
            step_cursor(&rows, 2, 0),
            4,
            "a cursor on a row it may not rest on settles below"
        );
        assert_eq!(step_cursor(&rows, 99, 0), 5, "or on the last, past the end");
    }

    #[test]
    fn a_line_wider_than_the_pane_breaks_at_a_space_and_keeps_every_word() {
        assert_eq!(wrap_columns("one two three", 0), ["one two three"]);
        assert_eq!(wrap_columns("one two three", 13), ["one two three"]);
        assert_eq!(wrap_columns("one two three", 8), ["one two", "three"]);
        assert_eq!(wrap_columns("one two three", 7), ["one two", "three"]);
        assert_eq!(
            wrap_columns("abcdefghij", 4),
            ["abcd", "efgh", "ij"],
            "a word wider than a line is broken where it must be"
        );
        assert_eq!(
            wrap_columns("  indented words here", 12),
            ["  indented", "words here"],
            "the indent a line starts with stays, and is never a row of its own"
        );
        assert_eq!(
            wrap_columns("日本語のメモ", 6),
            ["日本語", "のメモ"],
            "measured in display columns, not characters"
        );
    }

    #[test]
    fn a_long_note_and_a_long_todo_continue_on_the_rows_under_them() {
        let detail = ProjectDetail {
            notes: vec![note(
                Some("2026-01-01T10:00:00Z"),
                "recut the second verse around the new take\nthen colour",
            )],
            todos: vec![crate::core::body::Todo {
                done: false,
                text: "send the rough cut to the label".to_string(),
            }],
            ..Default::default()
        };
        // 31 columns inside the border: 20 for a note's text, 27 for a todo's.
        let rows = pane_rows(&project(&[], "client"), Some(&detail), 31);
        let at = rows
            .iter()
            .position(|r| matches!(r, PaneRow::Note { .. }))
            .unwrap();
        assert_eq!(
            rows[at..at + 4],
            [
                PaneRow::Note {
                    ordinal: 0,
                    date: Some("2026-01-01".to_string()),
                    first: "recut the second".to_string(),
                },
                PaneRow::NoteLine("verse around the new".to_string()),
                PaneRow::NoteLine("take".to_string()),
                PaneRow::NoteLine("then colour".to_string()),
            ]
        );
        let todo = rows
            .iter()
            .position(|r| matches!(r, PaneRow::Todo { .. }))
            .unwrap();
        assert_eq!(
            rows[todo..todo + 2],
            [
                PaneRow::Todo {
                    ordinal: 0,
                    done: false,
                    text: "send the rough cut to the".to_string(),
                },
                PaneRow::TodoLine {
                    done: false,
                    text: "label".to_string(),
                },
            ]
        );
        assert!(
            !rows[todo + 1].selectable(),
            "the cursor rests on the todo, not on the rest of it"
        );

        let long = ProjectDetail {
            notes: vec![note(None, &"word ".repeat(200))],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&long), 31);
        let lines = rows
            .iter()
            .filter(|r| matches!(r, PaneRow::NoteLine(_)))
            .count();
        assert_eq!(lines, NOTE_LINES_SHOWN - 1, "wrapped rows count as lines");
        assert!(rows.iter().any(|r| matches!(r, PaneRow::NoteMore(_))));
    }
}
