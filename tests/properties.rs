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

    /// Whatever a user types into a variable, the metadata fastf writes for it
    /// must read back — with the same values.
    ///
    /// This is the guarantee behind making `render` return a `Result`. It used to
    /// swallow a serialization failure and write `# yaml-serialize-error: ...`
    /// between valid `---` delimiters, which parses as an empty document: the
    /// project was unreadable, and therefore undiscoverable, from birth. Colons,
    /// quotes, leading dashes, newlines and unicode are all ordinary things to
    /// type into a client name.
    #[test]
    fn rendered_metadata_always_reads_back(
        values in prop::collection::vec(hostile_text(), 0..4),
    ) {
        use fastf::core::template::{Template, Variable, VarType, Transform};

        let mut tmpl = Template {
            name: "T".to_string(),
            slug: "t".to_string(),
            naming_pattern: "{id}".to_string(),
            ..Template::default()
        };
        let mut vars = HashMap::new();
        for (index, value) in values.iter().enumerate() {
            let slug = format!("v{index}");
            tmpl.variables.push(Variable {
                slug: slug.clone(),
                label: format!("Var {index}"),
                var_type: VarType::Text,
                required: false,
                options: vec![],
                default: String::new(),
                transform: Transform::None,
            });
            vars.insert(slug, value.clone());
        }

        let plan = fastf::core::project::ProjectPlan {
            folder_name: "ID0001".to_string(),
            root_path: std::path::PathBuf::from("/tmp/ID0001"),
            vars: vars.clone(),
            id_str: "ID0001".to_string(),
            counter_value: 1,
            ctx: fastf::core::naming::RenderContext::now("%Y-%m-%d"),
        };

        let rendered = project_info::render(&plan, &tmpl, &[]).expect("render must not fail");
        let (frontmatter, _) = project_info::split_frontmatter_body(&rendered)
            .ok_or_else(|| TestCaseError::fail("rendered file has no frontmatter"))?;
        let meta: project_info::Metadata = serde_yaml::from_str(frontmatter)
            .map_err(|e| TestCaseError::fail(format!("frontmatter unreadable: {e}\n{frontmatter}")))?;

        prop_assert_eq!(&meta.id, "ID0001");
        for (slug, value) in &vars {
            prop_assert_eq!(meta.variables.get(slug), Some(value), "variable {}", slug);
        }
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

// ---------------------------------------------------------------------------
// Interpolation is deterministic (v1.7.1)
// ---------------------------------------------------------------------------

proptest! {
    /// The rendered result does not depend on `HashMap` iteration order.
    ///
    /// The old implementation ran `String::replace` once per variable, in
    /// whatever order the map iterated, so a value that happened to contain
    /// `{another_token}` expanded or did not depending on hashing — two runs of
    /// the same create could produce different names.
    #[test]
    fn interpolation_does_not_depend_on_map_order(
        values in prop::collection::vec("[a-zA-Z0-9{}_-]{0,12}", 1..6)
    ) {
        use std::collections::HashMap;

        let slugs: Vec<String> = (0..values.len()).map(|n| format!("v{n}")).collect();
        let pattern: String = slugs
            .iter()
            .map(|slug| format!("{{{slug}}}"))
            .collect::<Vec<_>>()
            .join("_");

        // The same pairs, inserted in two different orders. A `HashMap` does not
        // preserve insertion order, but with a value that itself contains a
        // token the old code's result depended on which one it replaced first.
        let mut forward: HashMap<String, String> = HashMap::new();
        for (slug, value) in slugs.iter().zip(values.iter()) {
            forward.insert(slug.clone(), value.clone());
        }
        let mut backward: HashMap<String, String> = HashMap::new();
        for (slug, value) in slugs.iter().zip(values.iter()).rev() {
            backward.insert(slug.clone(), value.clone());
        }

        let ctx = fastf::core::naming::RenderContext::now("%Y-%m-%d");
        prop_assert_eq!(
            fastf::core::naming::interpolate_with(&pattern, &forward, &ctx),
            fastf::core::naming::interpolate_with(&pattern, &backward, &ctx),
        );
    }

    /// A substituted value is never re-scanned, so a variable holding `{v0}`
    /// comes out as the literal text `{v0}`.
    #[test]
    fn a_substituted_value_is_never_rescanned(inner in "[a-z]{1,6}") {
        use std::collections::HashMap;

        let vars: HashMap<String, String> = [
            ("v0".to_string(), inner.clone()),
            ("v1".to_string(), "{v0}".to_string()),
        ]
        .into_iter()
        .collect();

        let ctx = fastf::core::naming::RenderContext::now("%Y-%m-%d");
        prop_assert_eq!(
            fastf::core::naming::interpolate_with("{v1}", &vars, &ctx),
            "{v0}".to_string(),
        );
    }

    /// An unknown token passes through exactly as written, whatever is around it.
    #[test]
    fn an_unknown_token_is_left_alone(head in "[a-z ]{0,8}", tail in "[a-z ]{0,8}") {
        use std::collections::HashMap;

        let pattern = format!("{head}{{no_such_variable}}{tail}");
        let ctx = fastf::core::naming::RenderContext::now("%Y-%m-%d");
        prop_assert_eq!(
            fastf::core::naming::interpolate_with(&pattern, &HashMap::new(), &ctx),
            pattern.clone(),
        );
    }
}
