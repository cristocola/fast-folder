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
//! **Nothing here edits.** A row says what it is; Enter on it is `update`'s
//! business, and what a change does to the file is `core::operations`'.

use crate::core::library::Project;
use crate::core::template::VarType;
use crate::tui::app::data::{Entry, ProjectDetail};
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;
use crate::tui::widgets::text_area::TextArea;

/// How many entries of the folder listing the pane shows before `… n more`.
pub const LISTING_SHOWN: usize = 8;

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
    /// The size and the journal count.
    Figures,
    /// A section heading: `── label ───`. The notes' and the journal's are
    /// where Enter edits the notes and adds an entry.
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
    /// One line of the notes.
    Note(String),
    /// One journal entry: `(date, message)`.
    Journal(String, String),
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
                | PaneRow::Rule("notes")
                | PaneRow::Rule("journal")
        )
    }
}

/// The pane's rows for `project`, given what has been read of it so far.
///
/// The order is the order the pane always drew: the name, the facts, the
/// figures, the tags, then everything the detail read added — a warning if a
/// read failed, the variables, the folder's top level, the notes, the
/// journal. Tags are one row each so a cursor can rest on one, with the row
/// that adds one under them; the tag rule is drawn whenever the tags are
/// there to add to, which is always.
///
/// Variables follow the template's own order and carry its `type`, so a
/// `select` offers its options and nothing else; a variable the metadata
/// holds that the template no longer declares — or every variable of a
/// registered project, which has no template — is free text after them.
pub fn pane_rows(project: &Project, detail: Option<&ProjectDetail>) -> Vec<PaneRow> {
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
    rows.extend(detail.notes.iter().cloned().map(PaneRow::Note));

    rows.push(PaneRow::Rule("journal"));
    rows.extend(
        detail
            .journal
            .iter()
            .map(|(date, message)| PaneRow::Journal(date.clone(), message.clone())),
    );
    rows
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
    /// The notes, as a text area over the section. Boxed: a text area is
    /// the larger payload by a distance, and the enum travels in `App`.
    Notes {
        row: usize,
        area: Box<TextArea>,
        error: Option<String>,
        pending: bool,
    },
}

impl PaneEdit {
    /// The row the edit is on.
    pub fn row(&self) -> usize {
        match self {
            PaneEdit::Line { row, .. } | PaneEdit::Notes { row, .. } => *row,
        }
    }

    pub fn is_notes(&self) -> bool {
        matches!(self, PaneEdit::Notes { .. })
    }

    pub fn pending(&self) -> bool {
        match self {
            PaneEdit::Line { pending, .. } | PaneEdit::Notes { pending, .. } => *pending,
        }
    }

    pub fn set_pending(&mut self, on: bool) {
        match self {
            PaneEdit::Line { pending, .. } | PaneEdit::Notes { pending, .. } => *pending = on,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            PaneEdit::Line { error, .. } | PaneEdit::Notes { error, .. } => error.as_deref(),
        }
    }

    /// A refusal, under the field that earned it; the write is no longer
    /// pending, so the field can be corrected and sent again.
    pub fn fail(&mut self, message: String) {
        match self {
            PaneEdit::Line { error, pending, .. } | PaneEdit::Notes { error, pending, .. } => {
                *error = Some(message);
                *pending = false;
            }
        }
    }

    pub fn clear_error(&mut self) {
        match self {
            PaneEdit::Line { error, .. } | PaneEdit::Notes { error, .. } => *error = None,
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
    Notes,
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
            PaneEdit::Notes { .. } => PaneTarget::Notes,
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
        PaneTarget::Notes => find(&|row| matches!(row, PaneRow::Rule("notes"))),
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
        let rows = pane_rows(&project(&["draft", "client/Acme"], "client"), None);
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

    #[test]
    fn selectable_rows_skip_the_facts_the_rules_and_the_listing() {
        let detail = ProjectDetail {
            listing: vec![Entry {
                name: "src".to_string(),
                is_dir: true,
            }],
            notes: vec!["a note".to_string()],
            journal: vec![("2026-01-01".to_string(), "began".to_string())],
            ..Default::default()
        };
        let rows = pane_rows(&project(&["draft"], "client"), Some(&detail));
        let selectable: Vec<&PaneRow> = rows.iter().filter(|r| r.selectable()).collect();
        assert_eq!(
            selectable,
            vec![
                &PaneRow::Name,
                &PaneRow::Tag("draft".to_string()),
                &PaneRow::AddTag,
                &PaneRow::Rule("notes"),
                &PaneRow::Rule("journal"),
            ]
        );
        assert!(rows.contains(&PaneRow::Rule("inside")));
        assert!(!rows.contains(&PaneRow::Reading));
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
        let rows = pane_rows(&project(&[], "client"), Some(&detail));
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
        let rows = pane_rows(&project(&[], "(registered)"), Some(&detail));
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
        let rows = pane_rows(&project(&["draft"], "client"), None);
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
}
