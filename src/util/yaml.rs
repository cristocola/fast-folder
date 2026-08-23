//! Rewriting a YAML document without destroying the parts we do not understand.
//!
//! `PROJECT_INFO.md`'s frontmatter and `template.yaml` are both user-owned files
//! that fastf rewrites in place: tagging, moving, renaming, and every template
//! editor round-trips them through a typed struct. Anything the struct has no
//! field for used to vanish at that point — a key written by a newer fastf, or
//! by the user's own tooling, deleted by an older build running `tag add`.
//!
//! The obvious fix is a `#[serde(flatten)]` catch-all, and it is the wrong one.
//! `flatten` routes *every* field through serde's `Content` buffer, so a plain
//! unquoted scalar (`year: 2026` in a hand-edited file) arrives as an integer and
//! is then rejected by the `String` field it belongs to. Metadata that fails to
//! parse is metadata that does not exist: `library::read_project_meta` drops the
//! error and the project disappears from discovery. Preserving unknown keys must
//! not cost a new way to lose a project.
//!
//! So the merge happens one level down, on the parsed document rather than on
//! the type. Deserialization is left exactly as it was, and the surviving keys
//! keep their original positions because [`serde_yaml_ng::Mapping`] is insertion
//! ordered.

use anyhow::{Context, Result};
use serde::Serialize;
use serde_yaml_ng::Value;

// ---------------------------------------------------------------------------
// The YAML crate, named once
// ---------------------------------------------------------------------------

/// Serialize a value to a YAML document.
///
/// Every YAML call in fastf goes through this module, so the crate underneath is
/// named in one file. That matters because `serde_yaml 0.9` is archived
/// upstream: replacing it is a change here rather than at nine call sites across
/// `template`, `project_info` and `library`, every one of which sits under a
/// byte-identity test.
pub fn to_string<T: Serialize>(value: &T) -> Result<String, serde_yaml_ng::Error> {
    serde_yaml_ng::to_string(value)
}

/// Parse a YAML document.
pub fn from_str<'a, T: serde::Deserialize<'a>>(text: &'a str) -> Result<T, serde_yaml_ng::Error> {
    serde_yaml_ng::from_str(text)
}

/// Convert a value into the crate's dynamic `Value`.
pub fn to_value<T: Serialize>(value: T) -> Result<Value, serde_yaml_ng::Error> {
    serde_yaml_ng::to_value(value)
}
/// Serialize `value`, keeping every top-level key `original` had that `value`'s
/// type does not own — each in its original position.
///
/// `owned` names the keys the type is authoritative for. A key listed there that
/// `value` does not currently emit is **removed**: that is how `provisioning:
/// false` disappears from a finished project, and how a pre-v0.8 flat `files:`
/// block keeps being dropped from a template rather than being resurrected as an
/// "unknown" key. Every key outside that list is left alone.
///
/// Falls back to a plain serialization whenever the original cannot be treated
/// as a mapping — absent, unparseable, or a scalar. There is nothing to preserve
/// in those cases, and refusing to write would turn a cosmetic problem into a
/// failed command.
pub fn to_string_preserving_unknown<T: Serialize>(
    value: &T,
    original: &str,
    owned: &[&str],
) -> Result<String> {
    let fresh = serde_yaml_ng::to_value(value).context("serializing")?;
    let Value::Mapping(fresh) = fresh else {
        return serde_yaml_ng::to_string(value).context("serializing");
    };

    // A BOM here is routine: Notepad and PowerShell's `Out-File -Encoding utf8`
    // both add one, and `Template::load_from_file` has stripped it for years.
    let original = original.strip_prefix('\u{feff}').unwrap_or(original);
    let Ok(Value::Mapping(mut merged)) = serde_yaml_ng::from_str::<Value>(original) else {
        return serde_yaml_ng::to_string(&Value::Mapping(fresh)).context("serializing");
    };

    for key in owned {
        let key = Value::String((*key).to_string());
        if !fresh.contains_key(&key) {
            // `shift_remove`, not `remove`: the latter swaps the last entry into
            // the hole and would shuffle an unrelated key across the document.
            merged.shift_remove(&key);
        }
    }
    for (key, value) in fresh {
        // Insertion keeps an existing key where it is and appends a new one,
        // which is what makes a mutation look like an edit rather than a rewrite.
        merged.insert(key, value);
    }

    serde_yaml_ng::to_string(&Value::Mapping(merged)).context("serializing")
}

/// The top-level keys of a value's own serialization, for the tests that keep an
/// `OWNED_KEYS` list honest. A field added to the struct and forgotten in the
/// list would otherwise be silently preserved from the old file instead of
/// updated — the mutation would appear to do nothing.
#[cfg(test)]
pub fn serialized_keys<T: Serialize>(value: &T) -> Vec<String> {
    let Ok(Value::Mapping(map)) = serde_yaml_ng::to_value(value) else {
        return Vec::new();
    };
    map.keys()
        .filter_map(|key| key.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Doc {
        first: String,
        second: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        optional: Option<String>,
    }

    const OWNED: &[&str] = &["first", "second", "optional"];

    fn doc(first: &str, second: &str, optional: Option<&str>) -> Doc {
        Doc {
            first: first.to_string(),
            second: second.to_string(),
            optional: optional.map(str::to_string),
        }
    }

    #[test]
    fn unknown_key_keeps_its_position() {
        let original = "first: a\nunknown: keep me\nsecond: b\n";
        let out =
            to_string_preserving_unknown(&doc("changed", "b", None), original, OWNED).unwrap();
        assert_eq!(out, "first: changed\nunknown: keep me\nsecond: b\n");
    }

    #[test]
    fn an_owned_key_the_value_no_longer_emits_is_removed() {
        let original = "first: a\noptional: gone soon\nsecond: b\n";
        let out = to_string_preserving_unknown(&doc("a", "b", None), original, OWNED).unwrap();
        assert!(!out.contains("optional"), "{out}");
        assert!(out.contains("first: a"), "{out}");
    }

    #[test]
    fn a_new_owned_key_is_appended() {
        let original = "first: a\nunknown: keep me\nsecond: b\n";
        let out = to_string_preserving_unknown(&doc("a", "b", Some("now here")), original, OWNED)
            .unwrap();
        assert_eq!(
            out,
            "first: a\nunknown: keep me\nsecond: b\noptional: now here\n"
        );
    }

    /// The value shape `#[serde(flatten)]` would have rejected. Nothing here
    /// parses it as anything but an opaque node, which is the entire point.
    #[test]
    fn an_unquoted_number_under_an_unknown_key_survives_verbatim() {
        let original = "first: a\nyear: 2026\nsecond: b\n";
        let out = to_string_preserving_unknown(&doc("a", "b", None), original, OWNED).unwrap();
        assert!(out.contains("year: 2026"), "{out}");
    }

    #[test]
    fn output_matches_a_plain_serialization_when_there_is_nothing_extra() {
        let value = doc("a", "b", None);
        let plain = serde_yaml_ng::to_string(&value).unwrap();
        let out = to_string_preserving_unknown(&value, &plain, OWNED).unwrap();
        assert_eq!(out, plain, "a document with no extras must not be reshaped");
    }

    #[test]
    fn a_non_mapping_original_falls_back_instead_of_failing() {
        let value = doc("a", "b", None);
        for original in ["", "just a scalar", "- a\n- list\n", "{{{ not yaml"] {
            let out = to_string_preserving_unknown(&value, original, OWNED).unwrap();
            assert_eq!(
                out,
                serde_yaml_ng::to_string(&value).unwrap(),
                "{original:?}"
            );
        }
    }

    #[test]
    fn a_leading_bom_does_not_hide_the_unknown_keys() {
        let original = "\u{feff}first: a\nunknown: keep me\nsecond: b\n";
        let out = to_string_preserving_unknown(&doc("a", "b", None), original, OWNED).unwrap();
        assert!(out.contains("unknown: keep me"), "{out}");
    }

    #[test]
    fn serialized_keys_reports_what_is_actually_emitted() {
        assert_eq!(serialized_keys(&doc("a", "b", None)), ["first", "second"]);
        assert_eq!(
            serialized_keys(&doc("a", "b", Some("x"))),
            ["first", "second", "optional"]
        );
    }

    /// The exact bytes the YAML crate emits, for a value that exercises
    /// everything a `PROJECT_INFO.md` can hold: a colon, a quote, a `#`, a
    /// multi-line value, unicode, an empty string, and a nested map.
    ///
    /// Captured from `serde_yaml 0.9.34` before the crate was replaced. This is
    /// what makes the swap checkable: every existing round-trip test proves
    /// fastf can read what it wrote, and only this one proves the bytes did not
    /// move — which they must not, because these files are diffed, committed and
    /// hand-edited by users.
    #[test]
    fn the_emitted_bytes_are_the_ones_we_have_always_emitted() {
        use serde::Serialize;
        use std::collections::BTreeMap;

        #[derive(Serialize)]
        struct Sample {
            id: String,
            title: String,
            quoted: String,
            hashed: String,
            multiline: String,
            unicode: String,
            empty: String,
            flag: bool,
            count: u32,
            list: Vec<String>,
            nested: BTreeMap<String, String>,
        }

        let sample = Sample {
            id: "ID0048".to_string(),
            title: "Aria: a study".to_string(),
            quoted: "she said \"hello\"".to_string(),
            hashed: "#1 pick".to_string(),
            multiline: "first\nsecond\n".to_string(),
            unicode: "日本語 · émigré".to_string(),
            empty: String::new(),
            flag: true,
            count: 7,
            list: vec!["one".to_string(), "two: three".to_string()],
            nested: BTreeMap::from([
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), String::new()),
            ]),
        };

        let expected = "\
id: ID0048
title: 'Aria: a study'
quoted: she said \"hello\"
hashed: '#1 pick'
multiline: |
  first
  second
unicode: 日本語 · émigré
empty: ''
flag: true
count: 7
list:
- one
- 'two: three'
nested:
  a: '1'
  b: ''
";
        assert_eq!(to_string(&sample).unwrap(), expected);
    }
}
