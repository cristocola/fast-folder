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
use crate::core::body::TodoPlace;
use crate::core::library::Project;
use crate::core::template::VarType;
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

/// How far the tasks under a phase sit in from its heading, so they read as
/// its own.
pub const PHASE_INDENT: usize = 2;

/// What kind of value a variable row holds, and so what Enter on it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VarKind {
    /// Free text, one line.
    Text,
    /// One of these, and nothing else.
    Select(Vec<String>),
}

/// The parts of the pane, in the order they are drawn. The header is the
/// name, the facts and the figures; every other part opens with its rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneSection {
    Header,
    Tags,
    Todo,
    Notes,
    Variables,
    Inside,
}

impl PaneSection {
    /// The word on the section's rule.
    pub fn label(self) -> &'static str {
        match self {
            PaneSection::Header => "",
            PaneSection::Tags => "tags",
            PaneSection::Variables => "variables",
            PaneSection::Inside => "inside",
            PaneSection::Notes => "notes",
            PaneSection::Todo => "todo",
        }
    }
}

/// The section row `at` belongs to: the nearest rule above it, or the header
/// when there is none.
pub fn section_at(rows: &[PaneRow], at: usize) -> PaneSection {
    rows.iter()
        .take(at.saturating_add(1))
        .rev()
        .find_map(|row| match row {
            PaneRow::Rule(section) => Some(*section),
            _ => None,
        })
        .unwrap_or(PaneSection::Header)
}

/// The first row of `section` the cursor can rest on — its first item, or
/// the row that adds one when it has none. `None` when the section is not in
/// the rows (yet: the detail is still being read).
pub fn first_in_section(rows: &[PaneRow], section: PaneSection) -> Option<usize> {
    let start = match section {
        PaneSection::Header => 0,
        _ => rows.iter().position(|row| *row == PaneRow::Rule(section))? + 1,
    };
    rows.iter()
        .enumerate()
        .skip(start)
        .take_while(|(index, row)| *index == start || !matches!(row, PaneRow::Rule(_)))
        .find(|(_, row)| row.selectable())
        .map(|(index, _)| index)
}

/// One fact the pane's header states about a project. The header is two
/// runs of them — what the project is (its template, its base, the day it was
/// made) and what it holds (its size, its notes, its todos) — each flowed
/// across as many rows as the pane's width needs, whole facts to a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    Template,
    Base,
    Created,
    Size,
    Notes(usize),
    Todos { done: usize, total: usize },
}

/// What stands between two facts on a row: three spaces, the separator, three
/// spaces — the house rule for facts on a line.
pub const FACT_GAP: usize = 7;

/// A note count as the pane and the list's peek say it.
pub fn notes_label(notes: usize) -> String {
    match notes {
        0 => "no notes".to_string(),
        1 => "1 note".to_string(),
        n => format!("{n} notes"),
    }
}

/// A todo count as the pane and the list's peek say it.
pub fn todos_label(done: usize, total: usize) -> String {
    if total == 0 {
        "no todos".to_string()
    } else {
        format!("{done}/{total} todos done")
    }
}

/// How wide `fact` is drawn — the size at the widest a size cell gets, so a
/// size landing (`scanning…` becoming `41.0 GB`) can never move a fact onto
/// another row, and with it every row under the cursor.
fn fact_width(fact: &Fact, project: &Project) -> usize {
    match fact {
        Fact::Template => project.template.width(),
        Fact::Base => crate::core::library::base_label(&project.base).width(),
        Fact::Created => "created ".len() + crate::tui::rows::date_cell(&project.created).width(),
        Fact::Size => crate::tui::rows::SIZE_CELL,
        Fact::Notes(notes) => notes_label(*notes).width(),
        Fact::Todos { done, total } => todos_label(*done, *total).width(),
    }
}

/// `facts` in rows no wider than `width`, whole facts to a row: one that does
/// not fit after the last starts the next row, and one wider than a row has a
/// row of its own. A `width` of zero is no limit.
fn flow_facts(facts: Vec<Fact>, project: &Project, width: usize) -> Vec<Vec<Fact>> {
    let mut rows: Vec<Vec<Fact>> = Vec::new();
    let mut row: Vec<Fact> = Vec::new();
    let mut used = 0;
    for fact in facts {
        let w = fact_width(&fact, project);
        if !row.is_empty() && width > 0 && used + FACT_GAP + w > width {
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        used += if row.is_empty() { w } else { FACT_GAP + w };
        row.push(fact);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// Where `+` on row `at` puts a new todo, as the file will have it: under the
/// phase the cursor is in; with the tasks that belong to no phase when the
/// cursor is on one of them and the list has phases below; at the end of the
/// list otherwise — which, for a list ending in a phase, is that phase.
fn place_at(rows: &[PaneRow], at: usize) -> TodoPlace {
    if !matches!(
        rows.get(at),
        Some(PaneRow::Todo { .. } | PaneRow::TodoLine { .. } | PaneRow::Phase { .. })
    ) {
        return TodoPlace::End;
    }
    let heading = rows
        .iter()
        .take(at + 1)
        .rev()
        .find_map(|row| match row {
            PaneRow::Phase { name, .. } => Some(Some(name.clone())),
            PaneRow::Rule(_) => Some(None),
            _ => None,
        })
        .flatten();
    match heading {
        Some(name) => TodoPlace::Phase(name),
        None if rows.iter().any(|row| matches!(row, PaneRow::Phase { .. })) => TodoPlace::Loose,
        None => TodoPlace::End,
    }
}

/// One pasted line as the todo it lists: the checklist or list marker it was
/// copied with — `- [ ]`, `- [x]`, `* `, `+ `, `1.`, `2)` — taken off.
pub fn todo_text_of(line: &str) -> String {
    let mut rest = line.trim();
    for marker in ["- ", "* ", "+ "] {
        if let Some(after) = rest.strip_prefix(marker) {
            rest = after.trim_start();
            break;
        }
    }
    let numbered = rest.find(['.', ')']).filter(|&at| {
        at > 0 && rest[..at].chars().all(|c| c.is_ascii_digit()) && rest[at + 1..].starts_with(' ')
    });
    if let Some(at) = numbered {
        rest = rest[at + 1..].trim_start();
    }
    for bracket in ["[ ]", "[x]", "[X]", "[]"] {
        if let Some(after) = rest.strip_prefix(bracket) {
            rest = after.trim_start();
            break;
        }
    }
    rest.trim_end().to_string()
}

/// Where `body::add_todos_at` opens a label the list does not have: above
/// the first `### Other`, which is where a list keeps what belongs to no
/// phase, and otherwise after the last task, over "add a todo".
fn new_phase_at(rows: &[PaneRow]) -> Option<usize> {
    rows.iter()
        .position(
            |row| matches!(row, PaneRow::Phase { name, .. } if name.eq_ignore_ascii_case("other")),
        )
        .or_else(|| rows.iter().position(|row| *row == PaneRow::AddTodo))
}

/// `rows` with the line a new phase is named on, where its heading will be
/// written.
pub fn with_naming(mut rows: Vec<PaneRow>) -> Vec<PaneRow> {
    if let Some(at) = new_phase_at(&rows) {
        rows.insert(at, PaneRow::Adding);
    }
    rows
}

/// `rows` with the line a new todo is typed on, where `body::add_todos_at`
/// will write it: after the last run of the phase — the last heading of that
/// name, ignoring case, as the writer matches it — after the tasks that belong
/// to no phase, or at the end of the list, over "add a todo". A phase the list
/// does not have yet is drawn where the writer will open it, empty, over the
/// line: it is written with its first todo.
pub fn with_adding(mut rows: Vec<PaneRow>, place: &TodoPlace) -> Vec<PaneRow> {
    let Some(add) = rows.iter().position(|row| *row == PaneRow::AddTodo) else {
        return rows;
    };
    if let TodoPlace::Phase(name) = place {
        let wanted = name.to_lowercase();
        let known = rows
            .iter()
            .any(|row| matches!(row, PaneRow::Phase { name, .. } if name.to_lowercase() == wanted));
        if !known {
            let at = new_phase_at(&rows).unwrap_or(add);
            rows.insert(
                at,
                PaneRow::Phase {
                    name: name.clone(),
                    done: 0,
                    total: 0,
                },
            );
            rows.insert(at + 1, PaneRow::Adding);
            return rows;
        }
    }
    // The next heading after `from`, or "add a todo": where a run ends.
    let run_end = |from: usize| {
        rows.iter()
            .enumerate()
            .skip(from)
            .find(|(_, row)| matches!(row, PaneRow::Phase { .. } | PaneRow::AddTodo))
            .map_or(add, |(index, _)| index)
    };
    let at = match place {
        TodoPlace::Phase(wanted) => {
            let wanted = wanted.to_lowercase();
            rows.iter()
                .rposition(|row| matches!(row, PaneRow::Phase { name, .. } if name.to_lowercase() == wanted))
                .map_or(add, |heading| run_end(heading + 1))
        }
        TodoPlace::Loose => rows
            .iter()
            .position(|row| *row == PaneRow::Rule(PaneSection::Todo))
            .map_or(add, |rule| run_end(rule + 1)),
        TodoPlace::End => add,
    };
    rows.insert(at, PaneRow::Adding);
    rows
}

/// A folder name as rows no wider than `width`, broken **after** a `_`, `-`,
/// `.` or space where one falls inside the row — the places a fastf name joins
/// its parts, so a row ends on a whole word of it — and inside a run only when
/// the run alone is wider than a row. Nothing is dropped. A `width` of zero is
/// no limit.
fn wrap_name(name: &str, width: usize) -> Vec<String> {
    if width == 0 || name.width() <= width {
        return vec![name.to_string()];
    }
    let mut rows = Vec::new();
    let mut rest = name;
    while rest.width() > width {
        let mut used = 0;
        let mut fits = 0;
        let mut after_joint = None;
        for (at, c) in rest.char_indices() {
            let w = c.width().unwrap_or(0);
            if used + w > width {
                break;
            }
            used += w;
            fits = at + c.len_utf8();
            if matches!(c, '_' | '-' | '.' | ' ') {
                after_joint = Some(fits);
            }
        }
        // Always forward: a first character wider than the row is a row.
        let first = rest.chars().next().map_or(rest.len(), char::len_utf8);
        let cut = after_joint.unwrap_or(fits).max(first);
        rows.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        rows.push(rest.to_string());
    }
    rows
}

/// One row of the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneRow {
    /// The folder name, or as much of it as fits the first row. Enter renames.
    Name(String),
    /// The rest of a name too wide for one row. Not a row the cursor rests on.
    NameLine(String),
    /// One row of the header's facts (`Fact`).
    Facts(Vec<Fact>),
    /// A section heading: `── label ───`. Never a row the cursor rests on:
    /// every section that can grow ends in a row that adds to it.
    Rule(PaneSection),
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
    /// The `###` label a run of todos sits under, with how many of the run
    /// are done. Not a row the cursor rests on: a phase is a line of the
    /// user's own file, not something to toggle.
    Phase {
        name: String,
        done: usize,
        total: usize,
    },
    /// One todo's first row. Enter toggles it; F2 edits its text. `phased`
    /// when it sits under a phase heading, and is drawn in from it.
    Todo {
        ordinal: usize,
        done: bool,
        text: String,
        phased: bool,
    },
    /// The rest of a todo too wide for the pane. Not a row the cursor rests on.
    TodoLine {
        done: bool,
        text: String,
        phased: bool,
    },
    /// The line a new todo is being typed on, where it will land (`with_adding`).
    Adding,
    /// The row under the last todo that adds one.
    AddTodo,
    /// The row under "add a todo" that opens a new phase — a `###` label,
    /// written with the first todo typed under it.
    AddPhase,
}

impl PaneRow {
    /// Whether the cursor may rest on this row — that is, whether Enter on
    /// it does something.
    pub fn selectable(&self) -> bool {
        matches!(
            self,
            PaneRow::Name(_)
                | PaneRow::Tag(_)
                | PaneRow::AddTag
                | PaneRow::Variable { .. }
                | PaneRow::EarlierNotes(_)
                | PaneRow::Note { .. }
                | PaneRow::AddNote
                | PaneRow::Todo { .. }
                | PaneRow::Adding
                | PaneRow::AddTodo
                | PaneRow::AddPhase
        )
    }
}

/// The pane's rows for `project`, given what has been read of it so far.
///
/// **What you act on first, what you look up last**: the header (the name,
/// what the project is, what it holds), the tags, then everything the detail
/// read added — a warning if a read failed, the todos, the notes, and only
/// then the reference material, the template's variables and the folder's top
/// level. A short pane — under the list, or a small window — shows the living
/// sections without a scroll. Tags, notes and todos are one row each so a
/// cursor can rest on one, with the row that adds one under them; their rules
/// are drawn whenever there is something to add to, which is always.
///
/// **Nothing in the header is cut.** The name wraps after the joints of a
/// fastf name (`wrap_name`) and the facts flow whole (`flow_facts`), so a
/// narrow pane shows more rows rather than half a date.
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
    let mut rows = Vec::new();
    let mut name = wrap_name(&project.name, width).into_iter();
    rows.push(PaneRow::Name(name.next().unwrap_or_default()));
    rows.extend(name.map(PaneRow::NameLine));
    let identity = vec![Fact::Template, Fact::Base, Fact::Created];
    rows.extend(
        flow_facts(identity, project, width)
            .into_iter()
            .map(PaneRow::Facts),
    );
    // Until the record is read the pane knows the size and nothing else: a
    // count of zero there would be a claim, not a fact.
    let mut figures = vec![Fact::Size];
    if let Some(detail) = detail {
        figures.push(Fact::Notes(detail.notes.len()));
        figures.push(Fact::Todos {
            done: detail.todos.iter().filter(|todo| todo.done).count(),
            total: detail.todos.len(),
        });
    }
    rows.extend(
        flow_facts(figures, project, width)
            .into_iter()
            .map(PaneRow::Facts),
    );

    rows.push(PaneRow::Rule(PaneSection::Tags));
    rows.extend(project.tags.iter().cloned().map(PaneRow::Tag));
    rows.push(PaneRow::AddTag);

    let Some(detail) = detail else {
        rows.push(PaneRow::Reading);
        return rows;
    };
    if let Some(error) = &detail.error {
        rows.push(PaneRow::Warning(error.clone()));
    }

    rows.push(PaneRow::Rule(PaneSection::Todo));
    // A label is drawn where it changes, so an ungrouped list draws none and
    // a grouped one draws each label once, over the run it names, with how
    // much of the run is done.
    let mut phase: Option<&str> = None;
    for (ordinal, todo) in detail.todos.iter().enumerate() {
        if todo.phase.as_deref() != phase {
            phase = todo.phase.as_deref();
            if let Some(name) = phase {
                let (done, total) = detail
                    .todos
                    .iter()
                    .skip(ordinal)
                    .take_while(|t| t.phase.as_deref() == Some(name))
                    .fold((0, 0), |(done, total), t| {
                        (done + usize::from(t.done), total + 1)
                    });
                rows.push(PaneRow::Phase {
                    name: name.to_string(),
                    done,
                    total,
                });
            }
        }
        let phased = phase.is_some();
        let indent = TODO_INDENT + if phased { PHASE_INDENT } else { 0 };
        let mut lines = wrap_columns(&todo.text, width.saturating_sub(indent)).into_iter();
        rows.push(PaneRow::Todo {
            ordinal,
            done: todo.done,
            text: lines.next().unwrap_or_default(),
            phased,
        });
        rows.extend(lines.map(|text| PaneRow::TodoLine {
            done: todo.done,
            text,
            phased,
        }));
    }
    rows.push(PaneRow::AddTodo);
    rows.push(PaneRow::AddPhase);

    rows.push(PaneRow::Rule(PaneSection::Notes));
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
            rows.push(PaneRow::Rule(PaneSection::Variables));
            rows.extend(variables);
        }
    }

    if !detail.listing.is_empty() {
        rows.push(PaneRow::Rule(PaneSection::Inside));
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
    /// A todo's text: `ordinal` in the file's todos, whose text was `was`
    /// when the edit opened, so a todo that changed meanwhile is refused
    /// rather than rewritten; emptied, it is removed.
    Todo { ordinal: usize, was: String },
    /// The line new todos are typed on, where they will land (`place`),
    /// drawn in from the heading when they will sit under one (`phased`).
    ///
    /// **It takes keys while a write is on its way**, so a person typing a
    /// list never loses the first letters of the next todo to the write of
    /// the last: `sending` holds what is on its way, `queued` what was entered
    /// after it and goes next, and `closing` a close asked for while either
    /// is waiting — the line goes once they have landed. `from` is where the
    /// cursor goes when the line closes: the row `+` was pressed on, then the
    /// last todo that landed. `landed` once any write from it has.
    NewTodo {
        place: TodoPlace,
        phased: bool,
        from: PaneTarget,
        sending: Vec<String>,
        queued: Vec<String>,
        closing: bool,
        landed: bool,
    },
    /// The line a new phase is named on, where its heading will be written.
    /// Enter turns it into the line its todos are typed on (`NewTodo`, in
    /// that phase): nothing is written until the first of them is, because
    /// a label with no task under it is not a phase to anybody reading the
    /// list. `from` is where the cursor goes when the line closes.
    NewPhase { from: PaneTarget },
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

/// A row by what it is about, for finding it again after the rows changed.
///
/// A landed edit patches the project's row and drops the cached detail, so
/// the pane's rows are rebuilt — with a tag more or less above the variable
/// that changed, or with the variables gone until the re-read lands. A resize
/// re-wraps every note and todo, and a discovery landing can change the tags
/// above them. The cursor follows the *thing*, not its old index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneTarget {
    Name,
    /// A tag by its text; gone, the cursor settles on the row that adds one.
    Tag(String),
    AddTag,
    Variable(String),
    EarlierNotes,
    /// A note by its ordinal; gone (removed), the row that adds one.
    Note(usize),
    /// A todo by its ordinal; gone, the row that adds one.
    Todo(usize),
    /// The line a new todo is being typed on.
    Adding,
    AddNote,
    AddTodo,
    AddPhase,
}

/// What the selectable row at `at` is about — `None` for a row the cursor
/// never rests on, or past the end.
pub fn target_at(rows: &[PaneRow], at: usize) -> Option<PaneTarget> {
    Some(match rows.get(at)? {
        PaneRow::Name(_) => PaneTarget::Name,
        PaneRow::Tag(tag) => PaneTarget::Tag(tag.clone()),
        PaneRow::AddTag => PaneTarget::AddTag,
        PaneRow::Variable { slug, .. } => PaneTarget::Variable(slug.clone()),
        PaneRow::EarlierNotes(_) => PaneTarget::EarlierNotes,
        PaneRow::Note { ordinal, .. } => PaneTarget::Note(*ordinal),
        PaneRow::AddNote => PaneTarget::AddNote,
        PaneRow::Todo { ordinal, .. } => PaneTarget::Todo(*ordinal),
        PaneRow::Adding => PaneTarget::Adding,
        PaneRow::AddTodo => PaneTarget::AddTodo,
        PaneRow::AddPhase => PaneTarget::AddPhase,
        _ => return None,
    })
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
            PaneEdit::Line {
                target: EditTarget::Todo { ordinal, .. },
                ..
            } => PaneTarget::Todo(*ordinal),
            // Where an add lands is the writer's answer, not a guess
            // (`ActionOutcome::todo_ordinal`); until then, the line.
            PaneEdit::Line {
                target: EditTarget::NewTodo { .. } | EditTarget::NewPhase { .. },
                ..
            } => PaneTarget::Adding,
            PaneEdit::Note { ordinal, .. } => PaneTarget::Note(*ordinal),
        }
    }

    /// Whether this is the line a new todo is typed on, with a write of it
    /// on its way.
    pub fn adding_in_flight(&self) -> bool {
        matches!(
            self,
            PaneEdit::Line {
                target: EditTarget::NewTodo { sending, .. },
                ..
            } if !sending.is_empty()
        )
    }

    /// Whether this is the line a new todo is typed on.
    pub fn is_adding(&self) -> bool {
        matches!(
            self,
            PaneEdit::Line {
                target: EditTarget::NewTodo { .. },
                ..
            }
        )
    }

    /// The row this edit was opened on, as it still is in the file — where
    /// to find the edit again when the rows are rebuilt under it. Not
    /// `target`: a tag is named there by the text being typed, which is no
    /// row at all until the write lands, and an edit re-anchored by it moved
    /// to "add a tag" on every refresh.
    pub fn anchor(&self) -> PaneTarget {
        match self {
            PaneEdit::Line {
                target: EditTarget::Variable(slug),
                ..
            } => PaneTarget::Variable(slug.clone()),
            PaneEdit::Line {
                target: EditTarget::Tag(from),
                ..
            } => PaneTarget::Tag(from.clone()),
            PaneEdit::Line {
                target: EditTarget::Todo { ordinal, .. },
                ..
            } => PaneTarget::Todo(*ordinal),
            PaneEdit::Line {
                target: EditTarget::NewTodo { .. } | EditTarget::NewPhase { .. },
                ..
            } => PaneTarget::Adding,
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
        PaneTarget::Name => find(&|row| matches!(row, PaneRow::Name(_))),
        PaneTarget::AddTag => find(&|row| matches!(row, PaneRow::AddTag)),
        PaneTarget::EarlierNotes => find(&|row| matches!(row, PaneRow::EarlierNotes(_)))
            .or_else(|| find(&|row| matches!(row, PaneRow::Note { .. }))),
        PaneTarget::Adding => find(&|row| matches!(row, PaneRow::Adding))
            .or_else(|| find(&|row| matches!(row, PaneRow::AddTodo))),
        PaneTarget::AddNote => find(&|row| matches!(row, PaneRow::AddNote)),
        PaneTarget::AddTodo => find(&|row| matches!(row, PaneRow::AddTodo)),
        PaneTarget::AddPhase => find(&|row| matches!(row, PaneRow::AddPhase)),
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

/// The row the cursor lands on after paging `delta_rows` *drawn* rows from
/// `from`: the farthest selectable row the page reaches, or the next one past
/// it when the page holds none, stopping at the ends.
///
/// Paging by selectable rows instead — `step_cursor` with a page's worth —
/// would skip every wrapped line of a note and every rule between sections,
/// and a PageDown would jump several screens at once.
pub fn page_cursor(rows: &[PaneRow], from: usize, delta_rows: isize) -> usize {
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(index, _)| index)
        .collect();
    let (Some(&first), Some(&last)) = (selectable.first(), selectable.last()) else {
        return 0;
    };
    let target = (from as isize).saturating_add(delta_rows);
    if delta_rows >= 0 {
        let within = selectable
            .iter()
            .rev()
            .find(|&&index| index > from && index as isize <= target);
        let beyond = selectable.iter().find(|&&index| index as isize > target);
        *within.or(beyond).unwrap_or(&last)
    } else {
        let within = selectable
            .iter()
            .find(|&&index| index < from && index as isize >= target);
        let beyond = selectable
            .iter()
            .rev()
            .find(|&&index| (index as isize) < target);
        *within.or(beyond).unwrap_or(&first)
    }
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
            PaneRow::Name(_) => self.run(CommandId::Rename),
            PaneRow::AddTag => self.open_add_tag(),
            PaneRow::AddNote => self.run(CommandId::NoteInline),
            PaneRow::EarlierNotes(_) => self.run(CommandId::ShowJournal),
            PaneRow::AddTodo => self.run(CommandId::AddTodo),
            PaneRow::AddPhase => self.run(CommandId::AddPhase),
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
        if self.pane_edit.as_ref().is_some_and(PaneEdit::is_adding) {
            return self.add_line_enter();
        }
        if matches!(
            self.pane_edit,
            Some(PaneEdit::Line {
                target: EditTarget::NewPhase { .. },
                ..
            })
        ) {
            return self.phase_line_enter();
        }
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
            EditTarget::Todo { ordinal, was } => {
                // Unchanged is a cancel — except a todo with no words, where
                // the empty line kept is how it is removed.
                if text == *was && !text.is_empty() {
                    self.close_pane_edit();
                    return Vec::new();
                }
                Action::ReplaceTodo {
                    project: Box::new(project),
                    ordinal: *ordinal,
                    was: was.clone(),
                    text,
                }
            }
            EditTarget::NewTodo { .. } | EditTarget::NewPhase { .. } => return Vec::new(),
            EditTarget::Tag(from) => {
                if text == *from {
                    self.close_pane_edit();
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

    /// F2 on a pane row: its text, opened in place. On a todo that is the
    /// rewording Enter never does — Enter ticks it — and emptied, the todo
    /// goes; on the name, a tag, a variable or a note it is what Enter opens.
    pub(super) fn pane_edit_text(&mut self) -> Vec<Effect> {
        let rows = self.pane_rows();
        let Some(PaneRow::Todo { ordinal, .. }) = rows.get(self.pane_cursor) else {
            return self.pane_edit_start();
        };
        let ordinal = *ordinal;
        // The whole todo as it was read — the row holds only what fits.
        let Some(was) = self
            .library
            .selected()
            .and_then(|project| self.details.get(&project.path))
            .and_then(|detail| detail.todos.get(ordinal))
            .map(|todo| todo.text.clone())
        else {
            return Vec::new();
        };
        self.pane_edit = Some(PaneEdit::Line {
            row: self.pane_cursor,
            input: crate::tui::widgets::input::LineEdit::with_text(was.clone()),
            target: EditTarget::Todo { ordinal, was },
            error: None,
            pending: false,
        });
        Vec::new()
    }

    /// `+` in the pane: one more of whatever the cursor is among — a tag, a
    /// note, or a todo, typed on a line where it will land: in the phase the
    /// cursor is in, with the loose tasks when it is on one, else at the end.
    pub(super) fn pane_add(&mut self) -> Vec<Effect> {
        let rows = self.pane_rows();
        match section_at(&rows, self.pane_cursor) {
            PaneSection::Tags => self.open_add_tag(),
            PaneSection::Notes => self.run(CommandId::NoteInline),
            _ => {
                // A todo goes to one list: with marks, which one is a guess,
                // and the verb says so rather than choosing.
                if let command::Availability::Disabled(reason) =
                    (command::find(CommandId::AddTodo).available)(self)
                {
                    self.warn(format!("Add a todo: {reason}"));
                    return Vec::new();
                }
                let place = place_at(&rows, self.pane_cursor);
                self.start_adding(place)
            }
        }
    }

    /// Open the line a new todo is typed on, in the pane, with the focus
    /// there. Falls back to the prompt when the pane cannot show the list —
    /// switched off, or its record not read yet.
    pub(super) fn start_adding(&mut self, place: TodoPlace) -> Vec<Effect> {
        let read = self
            .library
            .selected()
            .and_then(|p| self.details.get(&p.path));
        let Some(detail) = read else {
            return self.prompt_for_a_todo(place);
        };
        if !self.pane_live() || self.screen != super::Screen::Library {
            return self.prompt_for_a_todo(place);
        }
        // Drawn in from a heading when the todo will sit under one — at the
        // end of a list whose last run is a phase, too.
        let phased = match &place {
            TodoPlace::Phase(_) => true,
            TodoPlace::Loose => false,
            TodoPlace::End => detail.todos.last().is_some_and(|todo| todo.phase.is_some()),
        };
        // Closing the line comes back to the row `+` was pressed on — or, from
        // the list, to the row that adds a todo.
        let from = if self.focus == super::Focus::Detail {
            target_at(&self.pane_rows(), self.pane_cursor).unwrap_or(PaneTarget::AddTodo)
        } else {
            PaneTarget::AddTodo
        };
        self.set_focus(super::Focus::Detail);
        self.close_pane_edit();
        self.pane_edit = Some(PaneEdit::Line {
            row: 0,
            input: crate::tui::widgets::input::LineEdit::new(),
            target: EditTarget::NewTodo {
                place,
                phased,
                from,
                sending: Vec::new(),
                queued: Vec::new(),
                closing: false,
                landed: false,
            },
            error: None,
            pending: false,
        });
        self.pane_anchor = Some(PaneTarget::Adding);
        self.refind_pane();
        Vec::new()
    }

    /// The one-line prompt for a todo — what adding is where the pane cannot
    /// show the list it goes into.
    pub(super) fn prompt_for_a_todo(&mut self, place: TodoPlace) -> Vec<Effect> {
        let title = match &place {
            TodoPlace::Phase(name) => validators::add_todo_in_prompt(name),
            TodoPlace::End | TodoPlace::Loose => validators::ADD_TODO_PROMPT.to_string(),
        };
        self.modals
            .push(Modal::TextPrompt(super::actions::TextPrompt::new(
                title,
                super::actions::TextThen::AddTodo(place),
            )));
        Vec::new()
    }

    /// A new phase: its name typed on a line where its heading will be
    /// written, then its todos on the line under it. Falls back to a prompt
    /// for the name when the pane cannot show the list.
    pub(super) fn start_phase(&mut self) -> Vec<Effect> {
        let read = self
            .library
            .selected()
            .is_some_and(|p| self.details.contains_key(&p.path));
        if !read || !self.pane_live() || self.screen != super::Screen::Library {
            self.modals
                .push(Modal::TextPrompt(super::actions::TextPrompt::new(
                    validators::ADD_PHASE_PROMPT,
                    super::actions::TextThen::AddPhase,
                )));
            return Vec::new();
        }
        let from = if self.focus == super::Focus::Detail {
            target_at(&self.pane_rows(), self.pane_cursor).unwrap_or(PaneTarget::AddPhase)
        } else {
            PaneTarget::AddPhase
        };
        self.set_focus(super::Focus::Detail);
        self.close_pane_edit();
        self.pane_edit = Some(PaneEdit::Line {
            row: 0,
            input: crate::tui::widgets::input::LineEdit::new(),
            target: EditTarget::NewPhase { from },
            error: None,
            pending: false,
        });
        self.pane_anchor = Some(PaneTarget::Adding);
        self.refind_pane();
        Vec::new()
    }

    /// Enter on the line a phase is named on: the line becomes the one its
    /// todos are typed on, under the heading. Nothing is written yet. An
    /// empty Enter is a cancel.
    fn phase_line_enter(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Line {
            input,
            error,
            target: EditTarget::NewPhase { from },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        if input.text().trim().is_empty() {
            self.close_pane_edit();
            return Vec::new();
        }
        let Some(name) = crate::core::body::phase_label(input.text()) else {
            *error = Some(validators::PHASE_NAMELESS.to_string());
            return Vec::new();
        };
        let from = from.clone();
        self.pane_edit = Some(PaneEdit::Line {
            row: 0,
            input: crate::tui::widgets::input::LineEdit::new(),
            target: EditTarget::NewTodo {
                place: TodoPlace::Phase(name),
                phased: true,
                from,
                sending: Vec::new(),
                queued: Vec::new(),
                closing: false,
                landed: false,
            },
            error: None,
            pending: false,
        });
        self.pane_anchor = Some(PaneTarget::Adding);
        self.refind_pane();
        Vec::new()
    }

    /// Enter on the add line: what was typed goes to the file, and the line
    /// empties for the next — at once, so the keys that follow land in it
    /// while the write is on its way. Entered while one is, it waits its turn.
    /// An empty Enter is done: now, or once what is waiting has landed.
    pub(super) fn add_line_enter(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Line {
            input,
            error,
            target:
                EditTarget::NewTodo {
                    sending,
                    queued,
                    closing,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        let text = input.text().trim().to_string();
        if text.is_empty() {
            if sending.is_empty() && queued.is_empty() {
                self.close_pane_edit();
            } else {
                *closing = true;
            }
            return Vec::new();
        }
        *error = None;
        input.set_text("");
        if !sending.is_empty() {
            queued.push(text);
            return Vec::new();
        }
        self.send_adds(vec![text])
    }

    /// A paste onto the add line goes in at the caret, as into any field; a
    /// paste of several lines makes that many todos, in one write, the list
    /// markers a checklist is copied with taken off — and what was typed on
    /// the line before it is the first of them. A single line is the field's,
    /// markers taken off when the line was empty. `None` when this is not the
    /// add line's to take.
    pub(super) fn paste_todos(&mut self, text: &str) -> Option<Vec<Effect>> {
        let Some(PaneEdit::Line {
            input,
            error,
            target: EditTarget::NewTodo {
                sending, queued, ..
            },
            ..
        }) = &mut self.pane_edit
        else {
            return None;
        };
        if !text.contains('\n') {
            if input.is_empty() {
                input.set_text(todo_text_of(text));
                *error = None;
                return Some(Vec::new());
            }
            return None;
        }
        let typed = input.text().to_string();
        let at = typed
            .char_indices()
            .nth(input.cursor())
            .map_or(typed.len(), |(byte, _)| byte);
        let whole = format!("{}{text}{}", &typed[..at], &typed[at..]);
        let texts: Vec<String> = whole
            .lines()
            .map(todo_text_of)
            .filter(|line| !line.is_empty())
            .collect();
        input.set_text("");
        *error = None;
        if texts.is_empty() {
            return Some(Vec::new());
        }
        if !sending.is_empty() {
            queued.extend(texts);
            return Some(Vec::new());
        }
        Some(self.send_adds(texts))
    }

    /// Send `texts` from the add line. Refused before it left — something
    /// else is being written — they wait their turn instead, and go when that
    /// has landed (`flush_adds`).
    fn send_adds(&mut self, texts: Vec<String>) -> Vec<Effect> {
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        let Some(PaneEdit::Line {
            target: EditTarget::NewTodo { place, sending, .. },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        let place = place.clone();
        sending.clone_from(&texts);
        let effects = self.run_action(
            "writing…",
            Action::AddTodos {
                project: Box::new(project),
                texts: texts.clone(),
                place,
            },
        );
        if effects.is_empty()
            && let Some(PaneEdit::Line {
                target:
                    EditTarget::NewTodo {
                        sending, queued, ..
                    },
                ..
            }) = &mut self.pane_edit
        {
            sending.clear();
            let later = std::mem::take(queued);
            *queued = texts;
            queued.extend(later);
        }
        effects
    }

    /// After a write has landed: send what was entered while it was on its
    /// way, or close the line if that was asked for meanwhile.
    pub(super) fn flush_adds(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Line {
            target:
                EditTarget::NewTodo {
                    sending,
                    queued,
                    closing,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        if !sending.is_empty() || self.busy.is_some() {
            return Vec::new();
        }
        if !queued.is_empty() {
            let texts = std::mem::take(queued);
            return self.send_adds(texts);
        }
        if *closing {
            self.close_pane_edit();
        }
        Vec::new()
    }

    /// The add line's write has landed at `ordinal` (the last todo written):
    /// the line stays open, and closing it now comes back to that todo.
    pub(super) fn adds_landed(&mut self, ordinal: Option<usize>) {
        if let Some(PaneEdit::Line {
            target:
                EditTarget::NewTodo {
                    sending,
                    from,
                    landed,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        {
            sending.clear();
            *landed = true;
            if let Some(ordinal) = ordinal {
                *from = PaneTarget::Todo(ordinal);
            }
        }
    }

    /// The add line's write was refused. What was sent comes back to the
    /// field when nothing was typed after it; otherwise the refusal names what
    /// was not written, so nothing entered is lost without a word. What was
    /// waiting behind it is not sent: the refusal is the person's to read.
    pub(super) fn adds_refused(&mut self, message: String) {
        let Some(PaneEdit::Line {
            input,
            error,
            target:
                EditTarget::NewTodo {
                    sending,
                    queued,
                    closing,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        else {
            return;
        };
        let unsent: Vec<String> = sending.drain(..).chain(queued.drain(..)).collect();
        *closing = false;
        if unsent.len() == 1 && input.is_empty() {
            input.set_text(unsent[0].clone());
            *error = Some(message);
        } else {
            *error = Some(format!("{message} — not added: {}", unsent.join(", ")));
        }
    }

    /// Close the edit open in the pane, leaving its row as it was. The add
    /// line gives the cursor back to where it came from — the row `+` was
    /// pressed on, or the last todo it added — rather than to its own row,
    /// which goes with it; a write of it still on its way settles the cursor
    /// when it lands; and anything entered that could not be sent yet is
    /// named rather than dropped in silence.
    pub(super) fn close_pane_edit(&mut self) {
        let Some(edit) = self.pane_edit.take() else {
            return;
        };
        match edit {
            PaneEdit::Line {
                target:
                    EditTarget::NewTodo {
                        place,
                        from,
                        sending,
                        queued,
                        landed,
                        ..
                    },
                ..
            } => {
                if !sending.is_empty() {
                    self.pane_pending = Some(PaneTarget::Adding);
                }
                if !queued.is_empty() {
                    self.warn(format!("not added: {}", queued.join(", ")));
                } else if sending.is_empty()
                    && !landed
                    && let TodoPlace::Phase(name) = &place
                    && !self.has_phase(name)
                {
                    // The heading was only drawn: a phase is written with
                    // its first todo, and none was typed.
                    self.info(validators::PHASE_NOT_WRITTEN);
                }
                self.pane_anchor = Some(from);
            }
            PaneEdit::Line {
                target: EditTarget::NewPhase { from },
                ..
            } => self.pane_anchor = Some(from),
            _ => {}
        }
        self.refind_pane();
    }

    /// Whether the selected project's list, as last read, has a phase of this
    /// name — ignoring case, as the writer matches one.
    fn has_phase(&self, name: &str) -> bool {
        let wanted = name.to_lowercase();
        self.library
            .selected()
            .and_then(|project| self.details.get(&project.path))
            .is_some_and(|detail| {
                detail.todos.iter().any(|todo| {
                    todo.phase
                        .as_ref()
                        .is_some_and(|p| p.to_lowercase() == wanted)
                })
            })
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
            .map(|pane| crate::tui::layout::pane_text(pane).width as usize)
            .unwrap_or(0);
        let rows = pane_rows(project, self.details.get(&project.path), width);
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
        let before = self.library.selected_index();
        self.library.step(delta);
        if self.library.selected_index() == before {
            return Vec::new();
        }
        let effects = self.after_selection_change();
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
                PaneRow::Name("ID0001_One".to_string()),
                PaneRow::Facts(vec![Fact::Template, Fact::Base, Fact::Created]),
                // Before the read the size is all the pane knows.
                PaneRow::Facts(vec![Fact::Size]),
                PaneRow::Rule(PaneSection::Tags),
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
    fn a_new_phase_is_drawn_where_the_writer_will_open_it() {
        let todo = |text: &str, phase: Option<&str>| crate::core::body::Todo {
            done: false,
            text: text.to_string(),
            phase: phase.map(str::to_string),
        };
        let heading = |name: &str, total: usize| PaneRow::Phase {
            name: name.to_string(),
            done: 0,
            total,
        };
        let todo_rows = |rows: Vec<PaneRow>| -> Vec<PaneRow> {
            rows.into_iter()
                .skip_while(|r| *r != PaneRow::Rule(PaneSection::Todo))
                .skip(1)
                .take_while(|r| *r != PaneRow::Rule(PaneSection::Notes))
                .filter(|r| !matches!(r, PaneRow::Todo { .. }))
                .collect()
        };
        let plain = ProjectDetail {
            todos: vec![todo("read the order", None)],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&plain), 0);

        // The name is typed at the end of the list, where the label will go.
        assert_eq!(
            todo_rows(with_naming(rows.clone())),
            vec![PaneRow::Adding, PaneRow::AddTodo, PaneRow::AddPhase]
        );
        // Named, the heading is drawn over the line its todos are typed on.
        assert_eq!(
            todo_rows(with_adding(rows, &TodoPlace::Phase("Grade".to_string()))),
            vec![
                heading("Grade", 0),
                PaneRow::Adding,
                PaneRow::AddTodo,
                PaneRow::AddPhase
            ]
        );

        // A list that keeps an `### Other` opens a new label above it.
        let other = ProjectDetail {
            todos: vec![
                todo("download", Some("Setup")),
                todo("chase the invoice", Some("Other")),
            ],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&other), 0);
        assert_eq!(
            todo_rows(with_naming(rows.clone())),
            vec![
                heading("Setup", 1),
                PaneRow::Adding,
                heading("Other", 1),
                PaneRow::AddTodo,
                PaneRow::AddPhase
            ]
        );
        assert_eq!(
            todo_rows(with_adding(
                rows.clone(),
                &TodoPlace::Phase("Grade".to_string())
            )),
            vec![
                heading("Setup", 1),
                heading("Grade", 0),
                PaneRow::Adding,
                heading("Other", 1),
                PaneRow::AddTodo,
                PaneRow::AddPhase
            ]
        );
        // A name the list has, in any case, is that phase: no second label.
        assert_eq!(
            todo_rows(with_adding(rows, &TodoPlace::Phase("setup".to_string()))),
            vec![
                heading("Setup", 1),
                PaneRow::Adding,
                heading("Other", 1),
                PaneRow::AddTodo,
                PaneRow::AddPhase
            ]
        );
    }

    #[test]
    fn a_phase_label_is_drawn_where_it_changes_and_the_cursor_steps_over_it() {
        let todo = |done: bool, text: &str, phase: Option<&str>| crate::core::body::Todo {
            done,
            text: text.to_string(),
            phase: phase.map(str::to_string),
        };
        let detail = ProjectDetail {
            todos: vec![
                todo(true, "read the order", None),
                todo(false, "download the files", Some("Setup")),
                todo(false, "copy the audio", Some("Setup")),
                todo(false, "listen to the song", Some("Creative Plan")),
            ],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
        let todo_rows: Vec<&PaneRow> = rows
            .iter()
            .skip_while(|r| **r != PaneRow::Rule(PaneSection::Todo))
            .take_while(|r| **r != PaneRow::Rule(PaneSection::Notes))
            .collect();
        assert_eq!(
            todo_rows,
            vec![
                &PaneRow::Rule(PaneSection::Todo),
                &PaneRow::Todo {
                    ordinal: 0,
                    done: true,
                    text: "read the order".to_string(),
                    phased: false,
                },
                &PaneRow::Phase {
                    name: "Setup".to_string(),
                    done: 0,
                    total: 2,
                },
                &PaneRow::Todo {
                    ordinal: 1,
                    done: false,
                    text: "download the files".to_string(),
                    phased: true,
                },
                // The second task of a run draws no second label.
                &PaneRow::Todo {
                    ordinal: 2,
                    done: false,
                    text: "copy the audio".to_string(),
                    phased: true,
                },
                &PaneRow::Phase {
                    name: "Creative Plan".to_string(),
                    done: 0,
                    total: 1,
                },
                &PaneRow::Todo {
                    ordinal: 3,
                    done: false,
                    text: "listen to the song".to_string(),
                    phased: true,
                },
                &PaneRow::AddTodo,
                &PaneRow::AddPhase,
            ],
            "a label where the phase changes, and the ordinals still count tasks only"
        );
        assert!(
            !PaneRow::Phase {
                name: "Setup".to_string(),
                done: 0,
                total: 2
            }
            .selectable(),
            "Enter on a phase would have nothing to toggle"
        );
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
                phase: None,
            }],
            ..Default::default()
        };
        let rows = pane_rows(&project(&["draft"], "client"), Some(&detail), 0);
        let selectable: Vec<&PaneRow> = rows.iter().filter(|r| r.selectable()).collect();
        assert_eq!(
            selectable,
            vec![
                &PaneRow::Name("ID0001_One".to_string()),
                &PaneRow::Tag("draft".to_string()),
                &PaneRow::AddTag,
                &PaneRow::Todo {
                    ordinal: 0,
                    done: true,
                    text: "ingested".to_string(),
                    phased: false,
                },
                &PaneRow::AddTodo,
                &PaneRow::AddPhase,
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
            ]
        );
        assert!(rows.contains(&PaneRow::Rule(PaneSection::Inside)));
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
        assert!(at(&PaneRow::Rule(PaneSection::Todo)) < at(&PaneRow::AddTodo));
        assert!(at(&PaneRow::AddTodo) < at(&PaneRow::Rule(PaneSection::Notes)));
        assert!(at(&PaneRow::Rule(PaneSection::Notes)) < at(&PaneRow::AddNote));
        assert!(at(&PaneRow::AddNote) < at(&PaneRow::Rule(PaneSection::Inside)));
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
                    .position(|r| r == &PaneRow::Rule(PaneSection::Notes))
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
                phase: None,
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
                    phased: false,
                },
                PaneRow::TodoLine {
                    done: false,
                    text: "label".to_string(),
                    phased: false,
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

    /// A page is counted in drawn rows and lands on the farthest row Enter can
    /// act on inside it — or, when the page holds none, the next one past it.
    #[test]
    fn a_page_moves_by_drawn_rows_and_lands_on_a_row_that_can_be_acted_on() {
        let note = |ordinal| PaneRow::Note {
            ordinal,
            date: None,
            first: String::new(),
        };
        let line = || PaneRow::NoteLine(String::new());
        let mut rows = vec![
            PaneRow::Name(String::new()),
            PaneRow::Facts(Vec::new()),
            PaneRow::Facts(Vec::new()),
            PaneRow::Rule(PaneSection::Tags),
            PaneRow::Tag("draft".to_string()),
            PaneRow::AddTag,
            PaneRow::Rule(PaneSection::Notes),
            note(0),
        ];
        rows.extend((0..5).map(|_| line()));
        rows.push(note(1));
        rows.extend((0..5).map(|_| line()));
        rows.extend([
            PaneRow::AddNote,
            PaneRow::Rule(PaneSection::Todo),
            PaneRow::AddTodo,
        ]);
        let last = rows.len() - 1;

        assert_eq!(page_cursor(&rows, 0, 6), 5, "the farthest inside the page");
        assert_eq!(page_cursor(&rows, 5, 6), 7);
        assert_eq!(
            page_cursor(&rows, 7, 3),
            13,
            "a page of wrapped lines only reaches the next note"
        );
        assert_eq!(page_cursor(&rows, 13, 100), last, "and stops at the end");
        assert_eq!(page_cursor(&rows, last, 100), last);

        assert_eq!(page_cursor(&rows, 13, -6), 7, "up, the farthest back");
        assert_eq!(page_cursor(&rows, 7, -1), 5, "or the next one before it");
        assert_eq!(page_cursor(&rows, 0, -10), 0, "and stops at the top");
        assert_eq!(page_cursor(&[], 3, 5), 0, "no rows, no cursor");
    }

    /// A name wider than the pane wraps after the joints of a fastf name —
    /// `_`, `-`, `.`, a space — so a row ends on a whole part of it, and
    /// inside a part only when the part alone is wider than a row. Nothing is
    /// lost: the rows are the name.
    #[test]
    fn a_long_name_wraps_after_its_separators() {
        let name = "2026-01-02_acme_studio_Spring_Campaign-Extended_Directors_Cut_ID0201";
        let rows = wrap_name(name, 24);
        assert_eq!(rows.concat(), name, "nothing dropped: {rows:?}");
        assert!(rows.iter().all(|row| row.width() <= 24), "{rows:?}");
        for row in &rows[..rows.len() - 1] {
            assert!(
                row.ends_with(['_', '-', '.', ' ']),
                "a row ends on a joint: {rows:?}"
            );
        }
        assert_eq!(wrap_name("short", 24), vec!["short"]);
        assert_eq!(wrap_name(name, 0), vec![name], "no width, no wrap");
        // A part wider than a row is cut inside it, and still goes forward.
        let rows = wrap_name("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(rows, vec!["abcdefghij", "klmnopqrst", "uvwxyz"]);
        let rows = wrap_name("日本語のフォルダ", 3);
        assert_eq!(rows.concat(), "日本語のフォルダ");
        assert!(rows.iter().all(|row| !row.is_empty()));
    }

    /// Facts flow whole: as many to a row as fit with the gap between them,
    /// the next on the row under — never half a date on one row and half on
    /// the next.
    #[test]
    fn facts_flow_whole_and_wrap_between_them() {
        let project = project(&[], "client-project");
        let wide = pane_rows(&project, None, 200);
        assert_eq!(
            wide[1],
            PaneRow::Facts(vec![Fact::Template, Fact::Base, Fact::Created]),
            "a wide pane has what the project is on one row"
        );
        // "client-project" (14) + gap (7) + "projects" (8) = 29; the date's 18
        // more does not fit 40, so it starts the next row.
        let narrow = pane_rows(&project, None, 40);
        assert_eq!(narrow[1], PaneRow::Facts(vec![Fact::Template, Fact::Base]));
        assert_eq!(narrow[2], PaneRow::Facts(vec![Fact::Created]));
        // Narrower than any two: one fact to a row.
        let tight = pane_rows(&project, None, 15);
        let facts: Vec<&PaneRow> = tight
            .iter()
            .filter(|row| matches!(row, PaneRow::Facts(_)))
            .collect();
        assert!(
            facts
                .iter()
                .all(|row| matches!(row, PaneRow::Facts(f) if f.len() == 1))
        );
    }

    /// The size is measured at the widest a size cell gets, so the rows the
    /// figures take cannot change when a size lands — which would move every
    /// row under the cursor by one.
    #[test]
    fn the_figures_row_count_does_not_depend_on_the_size() {
        let project = project(&[], "client");
        let detail = ProjectDetail::default();
        // Size (11) + gap (7) + "no notes" (8) = 26: at 26 they share a row,
        // whatever the size turns out to read.
        let rows = pane_rows(&project, Some(&detail), 26);
        assert!(rows.contains(&PaneRow::Facts(vec![Fact::Size, Fact::Notes(0)])));
        let rows = pane_rows(&project, Some(&detail), 25);
        assert!(rows.contains(&PaneRow::Facts(vec![Fact::Size])));
    }

    /// The sections you act on come before the ones you look things up in.
    #[test]
    fn the_living_sections_come_first() {
        let meta = metadata(&[("client", "Acme")]);
        let detail = ProjectDetail {
            meta: Some(meta),
            variables: vec![variable("client", VarType::Text, &[])],
            listing: vec![Entry {
                name: "src".to_string(),
                is_dir: true,
            }],
            ..Default::default()
        };
        let rows = pane_rows(&project(&["draft"], "client"), Some(&detail), 60);
        let rules: Vec<PaneSection> = rows
            .iter()
            .filter_map(|row| match row {
                PaneRow::Rule(section) => Some(*section),
                _ => None,
            })
            .collect();
        assert_eq!(
            rules,
            vec![
                PaneSection::Tags,
                PaneSection::Todo,
                PaneSection::Notes,
                PaneSection::Variables,
                PaneSection::Inside,
            ]
        );
    }

    /// A pasted line loses the marker a checklist was copied with — a box, a
    /// bullet, a number — and nothing else.
    #[test]
    fn a_pasted_line_loses_its_list_marker_and_nothing_else() {
        for (line, todo) in [
            ("- [ ] colour", "colour"),
            ("- [x] shot the stills", "shot the stills"),
            ("* [X] graded", "graded"),
            ("* sound mix", "sound mix"),
            ("+ export", "export"),
            ("1. deliver", "deliver"),
            ("12) invoice", "invoice"),
            ("[ ] bare box", "bare box"),
            ("  plain words  ", "plain words"),
            (
                "2026-01-02 is a date, not a number",
                "2026-01-02 is a date, not a number",
            ),
            ("3.5 GB to upload", "3.5 GB to upload"),
            ("-dash without a space", "-dash without a space"),
        ] {
            assert_eq!(todo_text_of(line), todo, "{line:?}");
        }
    }

    /// The add line opens where `body::add_todos_at` will write: the last run
    /// of the phase, whatever the case of its name; after the loose tasks;
    /// or at the end.
    #[test]
    fn the_add_line_opens_where_the_todo_will_land() {
        let todo = |text: &str, phase: Option<&str>| crate::core::body::Todo {
            done: false,
            text: text.to_string(),
            phase: phase.map(str::to_string),
        };
        let detail = ProjectDetail {
            todos: vec![
                todo("loose", None),
                todo("brief", Some("Setup")),
                todo("room", Some("Setup")),
                todo("cut", Some("Edit")),
                todo("again", Some("setup")),
            ],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
        let around = |place: TodoPlace| {
            let rows = with_adding(rows.clone(), &place);
            let at = rows.iter().position(|r| *r == PaneRow::Adding).unwrap();
            (rows[at - 1].clone(), rows[at + 1].clone())
        };
        let (above, below) = around(TodoPlace::Phase("Setup".to_string()));
        assert!(
            matches!(above, PaneRow::Todo { ordinal: 4, .. }) && below == PaneRow::AddTodo,
            "the last run named setup, in any case: {above:?} / {below:?}"
        );
        let (above, below) = around(TodoPlace::Loose);
        assert!(matches!(above, PaneRow::Todo { ordinal: 0, .. }));
        assert!(matches!(below, PaneRow::Phase { .. }), "{below:?}");
        let (_, below) = around(TodoPlace::End);
        assert_eq!(below, PaneRow::AddTodo);
        let (_, below) = around(TodoPlace::Phase("Deliver".to_string()));
        assert_eq!(below, PaneRow::AddTodo, "a phase not there yet: the end");

        // `+` chooses the place from the row it is pressed on.
        let on = |wanted: &dyn Fn(&PaneRow) -> bool| {
            place_at(&rows, rows.iter().position(wanted).unwrap())
        };
        assert_eq!(
            on(&|r| matches!(r, PaneRow::Todo { ordinal: 0, .. })),
            TodoPlace::Loose
        );
        assert_eq!(
            on(&|r| matches!(r, PaneRow::Todo { ordinal: 3, .. })),
            TodoPlace::Phase("Edit".to_string())
        );
        assert_eq!(on(&|r| *r == PaneRow::AddTodo), TodoPlace::End);
    }

    /// A phase heading counts what is done under it, and its tasks sit in from
    /// it; a list with no phases is not indented at all.
    #[test]
    fn a_phase_heading_counts_its_tasks_and_indents_them() {
        let todo = |done: bool, phase: Option<&str>| crate::core::body::Todo {
            done,
            text: "task".to_string(),
            phase: phase.map(str::to_string),
        };
        let detail = ProjectDetail {
            todos: vec![
                todo(true, Some("Shoot")),
                todo(true, Some("Shoot")),
                todo(false, Some("Cut")),
            ],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&detail), 30);
        let headings: Vec<(&str, usize, usize)> = rows
            .iter()
            .filter_map(|row| match row {
                PaneRow::Phase { name, done, total } => Some((name.as_str(), *done, *total)),
                _ => None,
            })
            .collect();
        assert_eq!(headings, vec![("Shoot", 2, 2), ("Cut", 0, 1)]);
        assert!(
            rows.iter()
                .all(|row| !matches!(row, PaneRow::Todo { phased: false, .. }))
        );

        let flat = ProjectDetail {
            todos: vec![todo(false, None)],
            ..Default::default()
        };
        let rows = pane_rows(&project(&[], "client"), Some(&flat), 30);
        assert!(
            rows.iter()
                .any(|row| matches!(row, PaneRow::Todo { phased: false, .. }))
        );
    }
}
