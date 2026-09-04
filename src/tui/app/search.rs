//! The search bar's query: the `fastf search` grammar for anything with an
//! operator, fuzzy matching for the bare words.
//!
//! `core::query` decides what `tag:draft`, `template=music-video` and
//! `created>2026-01-01` mean, so the bar and the command agree. Bare terms are
//! where the two surfaces differ on purpose: the command's substring match is a
//! scripting contract, and a person typing wants `lulrmx` to find
//! `Lullaby_Remix`.

use std::collections::BTreeMap;

use crate::core::library::Project;
use crate::core::project_info::Metadata;
use crate::core::query::{self, Predicate};
use crate::tui::widgets::input::LineEdit;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub raw: String,
    /// Everything with an operator, evaluated by `core::query`.
    pub structured: Vec<Predicate>,
    /// The bare words, matched fuzzily against the row.
    pub free: Vec<String>,
}

impl Query {
    pub fn parse(raw: &str) -> Self {
        let terms: Vec<String> = raw.split_whitespace().map(str::to_string).collect();
        let mut structured = Vec::new();
        let mut free = Vec::new();
        for predicate in query::parse(&terms) {
            match predicate {
                Predicate::Free(term) => free.push(term),
                other => structured.push(other),
            }
        }
        Self {
            raw: raw.to_string(),
            structured,
            free,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.structured.is_empty() && self.free.is_empty()
    }

    /// What is wrong with the query as typed, if anything — the first term
    /// the grammar cannot mean anything by.
    pub fn diagnose(&self) -> Option<String> {
        self.raw.split_whitespace().find_map(query::diagnose)
    }

    /// The fuzzy pattern text: every bare word, space-joined.
    pub fn free_text(&self) -> String {
        self.free.join(" ")
    }

    /// Whether any predicate reads a field only `PROJECT_INFO.md` holds — a
    /// template variable — so the rows' metadata has to be read to answer it.
    pub fn needs_metadata(&self) -> bool {
        self.structured.iter().any(needs_metadata)
    }
}

/// Everything a `Project` row already knows, as `core::query` wants it. Rows
/// answer `tag:`, `template=`, `created>` and friends without a file read.
pub fn row_meta(project: &Project) -> Metadata {
    Metadata {
        id: project.id.clone(),
        template: project.template.clone(),
        template_name: project.template_name.clone(),
        created: project.created.clone(),
        folder: project.name.clone(),
        path: project.path.display().to_string(),
        variables: BTreeMap::new(),
        tags: project.tags.clone(),
        provisioning: false,
    }
}

/// The fields a row carries; anything else is a template variable.
const ROW_FIELDS: [&str; 8] = [
    "id",
    "template",
    "template_name",
    "created",
    "folder",
    "name",
    "path",
    "tags",
];

fn needs_metadata(predicate: &Predicate) -> bool {
    match predicate {
        Predicate::Tag(_) | Predicate::Free(_) => false,
        Predicate::Field { key, .. }
        | Predicate::After { key, .. }
        | Predicate::Before { key, .. } => !ROW_FIELDS.contains(&key.as_str()),
    }
}

/// The search bar itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchState {
    pub input: LineEdit,
    /// Keys go to the input rather than the lists.
    pub editing: bool,
    pub query: Query,
}

impl SearchState {
    pub fn with_text(text: &str) -> Self {
        Self {
            input: LineEdit::with_text(text),
            editing: false,
            query: Query::parse(text),
        }
    }

    /// Re-parse after an edit. `true` when the query changed.
    pub fn sync(&mut self) -> bool {
        let query = Query::parse(self.input.text());
        if query == self.query {
            return false;
        }
        self.query = query;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_go_to_core_and_bare_words_stay_free() {
        let q = Query::parse("tag:draft lulla template=music-video remix");
        assert_eq!(q.free, vec!["lulla".to_string(), "remix".to_string()]);
        assert_eq!(q.structured.len(), 2);
        assert!(!q.needs_metadata(), "tag and template live on the row");
        assert!(Query::parse("artist=Aria*").needs_metadata());
        assert!(Query::parse("created>2026-01-01").structured.len() == 1);
        assert!(Query::parse("  ").is_empty());
    }
}
