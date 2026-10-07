//! The detail pane as a list of rows.
//!
//! The pane is an editor: some of its rows are things you can change (the
//! name, a tag, a variable, the notes), so *which* rows exist and *which* of
//! them the cursor may rest on has to be one answer that `update` and `view`
//! both read, never counted a second time. `pane_rows` is that answer, and it
//! is pure: a project and what has been read of it in, rows out.
//!
//! **A row only says what it is**, and holds only what fits the pane, so an
//! edit reads the text it changes from the detail, never from a row.
//! `update`'s side of the pane is beside this module, in `pane_edit` (opening,
//! sending and landing an edit), `pane_add` and `pane_cursor`; what a change
//! does to the file is `core::operations`'.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::core::body::TodoPlace;
use crate::core::library::Project;
use crate::core::template::VarType;
use crate::tui::app::data::{Entry, ProjectDetail};
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
    /// When the project was last written, as `loaders::detail` read it:
    /// carried as text, since the fact comes from the detail and
    /// `fact_width` sees only the project.
    Touched(String),
    Size,
    Notes(usize),
    Todos {
        done: usize,
        total: usize,
    },
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
        Fact::Touched(at) => "touched ".len() + crate::tui::rows::date_cell(at).width(),
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
pub(super) fn place_at(rows: &[PaneRow], at: usize) -> TodoPlace {
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

/// The list's `###` labels as the file has them — or, for a detail that
/// carries only its todos, the labels they name, one where the phase changes.
pub fn labels_of(detail: &ProjectDetail) -> Vec<crate::core::body::PhaseLabel> {
    if !detail.phases.is_empty() {
        return detail.phases.clone();
    }
    let mut labels = Vec::new();
    let mut phase: Option<&str> = None;
    for (ordinal, todo) in detail.todos.iter().enumerate() {
        if todo.phase.as_deref() != phase {
            phase = todo.phase.as_deref();
            if let Some(name) = phase {
                labels.push(crate::core::body::PhaseLabel {
                    name: name.to_string(),
                    before: ordinal,
                });
            }
        }
    }
    labels
}

/// Where `body::add_todos_at` opens a label the list does not have: above
/// the first `### Other`, which is where a list keeps what belongs to no
/// phase; otherwise under the last task — above any empty labels after it —
/// and in a list with no task, at its end, over "add a todo".
fn new_phase_at(rows: &[PaneRow]) -> Option<usize> {
    let other = rows.iter().position(
        |row| matches!(row, PaneRow::Phase { name, .. } if name.to_lowercase() == "other"),
    );
    let after_last_task = rows
        .iter()
        .rposition(|row| matches!(row, PaneRow::Todo { .. } | PaneRow::TodoLine { .. }))
        .map(|last| last + 1);
    other
        .or(after_last_task)
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
    /// The one line that says what the project is, or as much of it as fits
    /// the row; empty when there is none, drawn as `(no description)`. Enter
    /// edits it; emptied, it is removed.
    Description(String),
    /// The rest of a description too wide for one row. Not a row the cursor
    /// rests on.
    DescriptionLine(String),
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
                | PaneRow::Description(_)
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
/// sections without a scroll. A tag, a note and a todo each have one row a
/// cursor can rest on, with the row that adds one under them; their rules
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
    let mut rows = header_rows(project, detail, width);

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

    rows.extend(todo_rows(detail, width));
    rows.extend(note_rows(detail, width));
    rows.extend(variable_rows(detail));
    rows.extend(listing_rows(detail));
    rows
}

fn header_rows(project: &Project, detail: Option<&ProjectDetail>, width: usize) -> Vec<PaneRow> {
    let mut rows = Vec::new();
    let mut name = wrap_name(&project.name, width).into_iter();
    rows.push(PaneRow::Name(name.next().unwrap_or_default()));
    rows.extend(name.map(PaneRow::NameLine));
    // The file's line once it has been read, the index's until then: the
    // index is what an older fastf may have rewritten without it.
    let description = detail
        .and_then(|detail| detail.meta.as_ref())
        .map(|meta| crate::core::validated::Description::one_line(&meta.description))
        .unwrap_or_else(|| project.description.clone());
    let mut description = wrap_columns(&description, width).into_iter();
    rows.push(PaneRow::Description(description.next().unwrap_or_default()));
    rows.extend(
        description
            .filter(|line| !line.is_empty())
            .map(PaneRow::DescriptionLine),
    );
    let mut identity = vec![Fact::Template, Fact::Base, Fact::Created];
    if let Some(at) = detail.and_then(|detail| detail.touched.clone()) {
        identity.push(Fact::Touched(at));
    }
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
    rows
}

fn todo_rows(detail: &ProjectDetail, width: usize) -> Vec<PaneRow> {
    let mut rows = Vec::new();
    rows.push(PaneRow::Rule(PaneSection::Todo));
    // Each label is drawn where the file has it, over the run of tasks up to
    // the next one, with how much of the run is done — a label with nothing
    // under it too, since that is where a todo added to its phase will go.
    let labels = labels_of(detail);
    let push_labels = |rows: &mut Vec<PaneRow>, at: usize| {
        for (index, label) in labels.iter().enumerate().filter(|(_, l)| l.before == at) {
            let end = labels
                .get(index + 1)
                .map_or(detail.todos.len(), |next| next.before);
            let run = detail.todos.get(at..end).unwrap_or_default();
            rows.push(PaneRow::Phase {
                name: label.name.clone(),
                done: run.iter().filter(|t| t.done).count(),
                total: run.len(),
            });
        }
    };
    for (ordinal, todo) in detail.todos.iter().enumerate() {
        push_labels(&mut rows, ordinal);
        let phased = todo.phase.is_some();
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
    push_labels(&mut rows, detail.todos.len());
    rows.push(PaneRow::AddTodo);
    rows.push(PaneRow::AddPhase);
    rows
}

fn note_rows(detail: &ProjectDetail, width: usize) -> Vec<PaneRow> {
    let mut rows = Vec::new();
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
    rows
}

fn variable_rows(detail: &ProjectDetail) -> Vec<PaneRow> {
    let mut rows = Vec::new();
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
    rows
}

fn listing_rows(detail: &ProjectDetail) -> Vec<PaneRow> {
    let mut rows = Vec::new();
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
    /// The one-line description; emptied, it is removed.
    Description,
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
    /// One line: a variable's value, a tag's or a todo's text, or the line a
    /// new todo or phase is typed on.
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
    Description,
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
        PaneRow::Description(_) => PaneTarget::Description,
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
                target: EditTarget::Description,
                ..
            } => PaneTarget::Description,
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
    /// row at all until the write lands, and an edit re-anchored by it would
    /// move to "add a tag" on every refresh.
    pub fn anchor(&self) -> PaneTarget {
        match self {
            PaneEdit::Line {
                target: EditTarget::Variable(slug),
                ..
            } => PaneTarget::Variable(slug.clone()),
            PaneEdit::Line {
                target: EditTarget::Description,
                ..
            } => PaneTarget::Description,
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
        PaneTarget::Description => find(&|row| matches!(row, PaneRow::Description(_))),
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

#[cfg(test)]
mod tests;
