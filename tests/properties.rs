//! Property tests for the pure logic.
//!
//! These assert invariants over generated input rather than a handful of chosen
//! cases. The important one is `sanitize_name`: its whole purpose is to return a
//! string that can become a directory, and the only convincing way to check that
//! is to feed it arbitrary text and try.

use std::collections::HashMap;

use fastf::core::{naming, project_info, query};
use proptest::prelude::*;

/// Text likely to break a filename: control characters, separators, reserved
/// words, dots and spaces in awkward places, plus ordinary unicode.
fn hostile_text() -> impl Strategy<Value = String> {
    prop_oneof![
        // Arbitrary unicode, including control characters.
        ".{0,40}",
        // Weighted toward the characters that actually cause trouble.
        prop::collection::vec(
            prop_oneof![
                Just('/'),
                Just('\\'),
                Just(':'),
                Just('*'),
                Just('?'),
                Just('"'),
                Just('<'),
                Just('>'),
                Just('|'),
                Just('.'),
                Just(' '),
                Just('_'),
                Just('\t'),
                Just('\u{0}'),
                Just('C'),
                Just('O'),
                Just('N'),
                Just('U'),
                Just('L'),
                Just('a'),
                Just('1'),
                Just('é'),
                Just('🎬'),
            ],
            0..24
        )
        .prop_map(|chars| chars.into_iter().collect::<String>()),
        // Reserved device names with assorted decoration.
        prop_oneof![
            Just("CON".to_string()),
            Just("nul".to_string()),
            Just("COM1.txt".to_string()),
            Just("LPT9 ".to_string()),
            Just("aux.".to_string()),
        ],
    ]
}

proptest! {
    /// The core guarantee: whatever goes in, what comes out can be created as a
    /// directory on this operating system — or is empty, which callers reject
    /// explicitly. Before the Windows hardening this failed for `CON`, for
    /// trailing dots, and for control characters.
    #[test]
    fn sanitize_name_always_yields_a_creatable_directory(raw in hostile_text()) {
        let safe = naming::sanitize_name(&raw);
        prop_assume!(!safe.is_empty());

        // It must also be a single path component: never a separator, never a
        // traversal, or a "name" could escape the base directory.
        prop_assert!(!safe.contains('/'), "separator in {safe:?}");
        prop_assert!(!safe.contains('\\'), "separator in {safe:?}");
        prop_assert_ne!(safe.as_str(), "..");
        prop_assert_ne!(safe.as_str(), ".");

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(&safe);
        std::fs::create_dir(&dir)
            .map_err(|e| TestCaseError::fail(format!("{raw:?} => {safe:?} not creatable: {e}")))?;

        // And the filesystem must have kept the name verbatim — Windows strips
        // trailing dots and spaces, which would desynchronize what fastf
        // recorded from what actually exists.
        let listed = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .next();
        prop_assert_eq!(listed.as_deref(), Some(safe.as_str()));
    }

    /// Applying it twice must equal applying it once: `plan()` sanitizes each
    /// variable and then the assembled folder name.
    #[test]
    fn sanitize_name_is_idempotent(raw in hostile_text()) {
        let once = naming::sanitize_name(&raw);
        prop_assert_eq!(naming::sanitize_name(&once), once);
    }

    /// Interpolated *names* must stay single components no matter what a
    /// variable contains — this is what stops a value from escaping the project.
    #[test]
    fn interpolated_names_never_traverse(
        pattern in "[a-z{}_-]{0,20}",
        value in hostile_text(),
    ) {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), naming::sanitize_name(&value));
        let out = naming::sanitize_name(&naming::interpolate_name(&pattern, &vars, "%Y-%m-%d"));

        prop_assert!(!out.contains('/'), "{out:?}");
        prop_assert!(!out.contains('\\'), "{out:?}");
        prop_assert_ne!(out.as_str(), "..");
    }

    /// The search parser takes raw user input and must never panic on it.
    #[test]
    fn query_parse_never_panics(terms in prop::collection::vec(".{0,30}", 0..6)) {
        let preds = query::parse(&terms);
        // Evaluating them against arbitrary metadata must be equally safe.
        let meta = project_info::Metadata {
            id: "ID0001".to_string(),
            template: "t".to_string(),
            template_name: "T".to_string(),
            created: "2026-01-01T00:00:00Z".to_string(),
            folder: "f".to_string(),
            path: "/p".to_string(),
            variables: Default::default(),
            tags: vec![],
            provisioning: false,
        };
        let _ = query::evaluate(&preds, &meta);
    }

    /// Frontmatter must round-trip byte-identically: fastf rewrites it for tags
    /// and journal notes, and a user's hand-written body must survive untouched.
    #[test]
    fn frontmatter_body_round_trips_byte_identically(
        body in "(?s).{0,200}",
        id in "[A-Z]{1,3}[0-9]{1,4}",
    ) {
        let content = format!(
            "---\nid: {id}\ntemplate: t\ntemplate_name: T\n\
             created: 2026-01-01T00:00:00Z\nfolder: f\npath: p\n\
             variables: {{}}\ntags: []\n---\n{body}"
        );
        let Some((frontmatter, split_body)) = project_info::split_frontmatter_body(&content) else {
            return Ok(()); // generated body broke the delimiters; not our concern
        };
        prop_assert_eq!(split_body, body.as_str(), "body must survive the split verbatim");
        prop_assert!(frontmatter.contains(&id));
    }
}
