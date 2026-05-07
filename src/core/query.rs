//! Simple query parser for `fastf search`.
//!
//! Grammar (all clauses AND-together; no OR, no parentheses):
//!
//! | Syntax       | Meaning                                         |
//! |---|---|
//! | `key=value`  | exact match on frontmatter field or variable    |
//! | `key=pat*`   | prefix match (wildcard only at end)             |
//! | `key>date`   | ISO-date comparison: field is after date        |
//! | `key<date`   | ISO-date comparison: field is before date       |
//! | `tag:value`  | exact tag match                                 |
//! | `tag:pat*`   | tag prefix match (wildcard at end)              |
//!
//! Unknown keys produce zero matches — not an error — which keeps forward
//! compatibility as new frontmatter fields are added.

use crate::core::index::ProjectRecord;
use crate::core::project_info::Metadata;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One parsed search clause.
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    /// `key=value` or `key=prefix*`
    Field { key: String, pattern: Pattern },
    /// `key>date`
    After { key: String, value: String },
    /// `key<date`
    Before { key: String, value: String },
    /// `tag:value` or `tag:prefix*`
    Tag(Pattern),
}

/// Exact or prefix match pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Exact(String),
    Prefix(String),
}

impl Pattern {
    fn matches(&self, candidate: &str) -> bool {
        match self {
            Pattern::Exact(v) => candidate.eq_ignore_ascii_case(v),
            Pattern::Prefix(p) => candidate
                .to_ascii_lowercase()
                .starts_with(&p.to_ascii_lowercase()),
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a slice of raw query terms into predicates.
///
/// Unrecognised or empty terms are silently skipped so callers can pass
/// `args.collect::<Vec<_>>()` without pre-validation.
pub fn parse(terms: &[String]) -> Vec<Predicate> {
    terms.iter().filter_map(|t| parse_term(t.trim())).collect()
}

fn parse_term(term: &str) -> Option<Predicate> {
    if term.is_empty() {
        return None;
    }

    // tag:value  or  tag:prefix*
    if let Some(rest) = term.strip_prefix("tag:") {
        return Some(Predicate::Tag(to_pattern(rest)));
    }

    // key>value
    if let Some((key, val)) = term.split_once('>')
        && !key.is_empty()
        && !val.is_empty()
    {
        return Some(Predicate::After {
            key: key.to_string(),
            value: val.to_string(),
        });
    }

    // key<value
    if let Some((key, val)) = term.split_once('<')
        && !key.is_empty()
        && !val.is_empty()
    {
        return Some(Predicate::Before {
            key: key.to_string(),
            value: val.to_string(),
        });
    }

    // key=value or key=prefix*
    if let Some((key, val)) = term.split_once('=')
        && !key.is_empty()
    {
        return Some(Predicate::Field {
            key: key.to_string(),
            pattern: to_pattern(val),
        });
    }

    None
}

fn to_pattern(s: &str) -> Pattern {
    if let Some(prefix) = s.strip_suffix('*') {
        Pattern::Prefix(prefix.to_string())
    } else {
        Pattern::Exact(s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Return `true` when every predicate in `predicates` matches the given
/// `(record, metadata)` pair.
///
/// Fields are resolved in this order:
/// 1. Top-level `Metadata` scalar fields (`id`, `template`, `template_name`,
///    `created`, `folder`, `path`).
/// 2. `variables.<slug>` — looked up in `meta.variables`.
/// 3. `ProjectRecord` fields for `name` (alias for `folder`).
///
/// Unknown field keys never match (returns false for that predicate).
pub fn evaluate(predicates: &[Predicate], record: &ProjectRecord, meta: &Metadata) -> bool {
    predicates.iter().all(|p| eval_one(p, record, meta))
}

fn eval_one(pred: &Predicate, record: &ProjectRecord, meta: &Metadata) -> bool {
    match pred {
        Predicate::Tag(pat) => meta.tags.iter().any(|t| pat.matches(t)),

        Predicate::Field { key, pattern } => {
            let val = resolve_field(key, record, meta);
            val.map(|v| pattern.matches(&v)).unwrap_or(false)
        }

        Predicate::After { key, value } => {
            let val = resolve_field(key, record, meta);
            val.map(|v| v.as_str() > value.as_str()).unwrap_or(false)
        }

        Predicate::Before { key, value } => {
            let val = resolve_field(key, record, meta);
            val.map(|v| v.as_str() < value.as_str()).unwrap_or(false)
        }
    }
}

/// Look up a field value from the combined record + metadata view.
fn resolve_field(key: &str, record: &ProjectRecord, meta: &Metadata) -> Option<String> {
    match key {
        "id" => Some(meta.id.clone()),
        "template" => Some(meta.template.clone()),
        "template_name" => Some(meta.template_name.clone()),
        "created" => Some(meta.created.clone()),
        "folder" | "name" => Some(meta.folder.clone()),
        "path" => Some(meta.path.clone()),
        // Check user-defined template variables
        other => meta
            .variables
            .get(other)
            .cloned()
            // Fall back to ProjectRecord fields for convenience
            .or_else(|| match other {
                "id" => Some(record.id.clone()),
                "name" => Some(record.name.clone()),
                _ => None,
            }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn make_meta(id: &str, template: &str, tags: &[&str]) -> Metadata {
        Metadata {
            id: id.to_string(),
            template: template.to_string(),
            template_name: template.to_string(),
            created: "2026-01-15T10:00:00Z".to_string(),
            folder: "ID0001_My_Project".to_string(),
            path: "/projects/ID0001_My_Project".to_string(),
            variables: BTreeMap::new(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn make_record(id: &str) -> ProjectRecord {
        ProjectRecord {
            id: id.to_string(),
            template: "test".to_string(),
            path: "/projects/test".to_string(),
            name: "test".to_string(),
            created_at: "2026-01-15T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn parse_exact_field() {
        let terms: Vec<String> = vec!["template=music-video".to_string()];
        let preds = parse(&terms);
        assert_eq!(preds.len(), 1);
        assert!(matches!(
            &preds[0],
            Predicate::Field { key, pattern: Pattern::Exact(v) }
            if key == "template" && v == "music-video"
        ));
    }

    #[test]
    fn parse_prefix_field() {
        let terms: Vec<String> = vec!["artist=Aria*".to_string()];
        let preds = parse(&terms);
        assert!(matches!(
            &preds[0],
            Predicate::Field { key, pattern: Pattern::Prefix(p) }
            if key == "artist" && p == "Aria"
        ));
    }

    #[test]
    fn parse_date_after() {
        let terms: Vec<String> = vec!["created>2026-01-01".to_string()];
        let preds = parse(&terms);
        assert!(matches!(
            &preds[0],
            Predicate::After { key, value } if key == "created" && value == "2026-01-01"
        ));
    }

    #[test]
    fn parse_tag() {
        let terms: Vec<String> = vec!["tag:draft".to_string()];
        let preds = parse(&terms);
        assert!(matches!(&preds[0], Predicate::Tag(Pattern::Exact(v)) if v == "draft"));
    }

    #[test]
    fn parse_tag_glob() {
        let terms: Vec<String> = vec!["tag:client/*".to_string()];
        let preds = parse(&terms);
        assert!(matches!(&preds[0], Predicate::Tag(Pattern::Prefix(p)) if p == "client/"));
    }

    #[test]
    fn evaluate_exact_tag_match() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "music-video", &["draft", "client/Acme"]);
        let preds = parse(&["tag:draft".to_string()]);
        assert!(evaluate(&preds, &rec, &meta));
    }

    #[test]
    fn evaluate_tag_glob_match() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "music-video", &["client/Acme_Corp"]);
        let preds = parse(&["tag:client/*".to_string()]);
        assert!(evaluate(&preds, &rec, &meta));
    }

    #[test]
    fn evaluate_tag_no_match() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "music-video", &["draft"]);
        let preds = parse(&["tag:urgent".to_string()]);
        assert!(!evaluate(&preds, &rec, &meta));
    }

    #[test]
    fn evaluate_field_exact() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "music-video", &[]);
        let preds = parse(&["template=music-video".to_string()]);
        assert!(evaluate(&preds, &rec, &meta));
    }

    #[test]
    fn evaluate_date_after() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "music-video", &[]);
        let preds = parse(&["created>2026-01-01".to_string()]);
        assert!(evaluate(&preds, &rec, &meta));
    }

    #[test]
    fn evaluate_date_before() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "music-video", &[]);
        let preds = parse(&["created<2027-01-01".to_string()]);
        assert!(evaluate(&preds, &rec, &meta));
    }

    #[test]
    fn evaluate_multi_clause_and() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "music-video", &["draft"]);
        let terms: Vec<String> = vec!["template=music-video".to_string(), "tag:draft".to_string()];
        let preds = parse(&terms);
        assert!(evaluate(&preds, &rec, &meta));
    }

    #[test]
    fn evaluate_unknown_key_is_false() {
        let rec = make_record("ID0001");
        let meta = make_meta("ID0001", "test", &[]);
        let preds = parse(&["nonexistent_field=anything".to_string()]);
        assert!(!evaluate(&preds, &rec, &meta));
    }
}
