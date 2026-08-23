//! The query parser and evaluator.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, project, project_info, query, template};

mod common;

use common::env::with_fresh_install;
use common::fixtures::write_template;

/// This binary's lock over the process environment — see `common::env`.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

// ---------------------------------------------------------------------------
// Search — query parser + evaluator
// ---------------------------------------------------------------------------

/// Each query operator returns correct matches on a synthesised metadata set.
#[test]
fn query_predicates_each_operator() {
    use std::collections::BTreeMap;

    let make_meta = |id: &str, tmpl: &str, created: &str, tags: &[&str], vars: &[(&str, &str)]| {
        project_info::Metadata {
            id: id.to_string(),
            template: tmpl.to_string(),
            template_name: tmpl.to_string(),
            created: created.to_string(),
            folder: format!("{id}_proj"),
            path: format!("/projects/{id}_proj"),
            variables: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            provisioning: false,
        }
    };

    // exact field
    let meta = make_meta(
        "ID0001",
        "music-video",
        "2026-03-01T00:00:00Z",
        &["draft"],
        &[("artist", "Ariana")],
    );
    assert!(query::evaluate(
        &query::parse(&["template=music-video".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["template=other".to_string()]),
        &meta
    ));

    // prefix glob
    assert!(query::evaluate(
        &query::parse(&["template=music*".to_string()]),
        &meta
    ));

    // date after
    assert!(query::evaluate(
        &query::parse(&["created>2026-01-01".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["created>2027-01-01".to_string()]),
        &meta
    ));

    // date before
    assert!(query::evaluate(
        &query::parse(&["created<2027-01-01".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["created<2025-01-01".to_string()]),
        &meta
    ));

    // exact tag
    assert!(query::evaluate(
        &query::parse(&["tag:draft".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["tag:urgent".to_string()]),
        &meta
    ));

    // tag glob
    assert!(query::evaluate(
        &query::parse(&["tag:dra*".to_string()]),
        &meta
    ));

    // variable field
    assert!(query::evaluate(
        &query::parse(&["artist=Ariana".to_string()]),
        &meta
    ));
    assert!(query::evaluate(
        &query::parse(&["artist=Aria*".to_string()]),
        &meta
    ));

    // multi-clause AND
    assert!(query::evaluate(
        &query::parse(&["template=music-video".to_string(), "tag:draft".to_string()]),
        &meta,
    ));
    assert!(!query::evaluate(
        &query::parse(&["template=music-video".to_string(), "tag:urgent".to_string()]),
        &meta,
    ));

    // unknown key → false, not error
    assert!(!query::evaluate(
        &query::parse(&["nonexistent=anything".to_string()]),
        &meta
    ));
}

/// Bare-term default mode searches across vars, tags, folder, template, and id
/// — and explicitly excludes `path`.  Drives the end-to-end create→read→evaluate
/// path so we know the predicate works against real frontmatter on disk.
#[test]
fn query_free_term_searches_across_fields() {
    sandboxed(|install| {
        // Template with one variable so we can verify variable-value matching.
        let yaml = r#"name: Free Search
slug: free-search
naming_pattern: "{id}_{title}"
id:
  prefix: F
  digits: 3
variables:
  - slug: title
    label: Title
    type: text
    required: true
    transform: title_underscore
  - slug: artist
    label: Artist
    type: text
    transform: title_underscore
tags:
  - creative
"#;
        write_template(install, "free-search", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("free-search").unwrap();
        let mut vars = HashMap::new();
        vars.insert("title".to_string(), "Lullaby".to_string());
        vars.insert("artist".to_string(), "Ariana Grande".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Read the on-disk metadata for predicate evaluation.
        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();

        // Variable value match (case-insensitive)
        assert!(
            query::evaluate(&query::parse(&["ariana".to_string()]), &meta),
            "should match variable value 'Ariana_Grande'"
        );

        // Tag match
        assert!(
            query::evaluate(&query::parse(&["creative".to_string()]), &meta),
            "should match tag 'creative'"
        );

        // Folder name (the resolved naming pattern)
        assert!(
            query::evaluate(&query::parse(&["lullaby".to_string()]), &meta),
            "should match folder name '{}'",
            plan.folder_name
        );

        // Template slug
        assert!(
            query::evaluate(&query::parse(&["free-search".to_string()]), &meta),
            "should match template slug 'free-search'"
        );

        // ID
        assert!(
            query::evaluate(&query::parse(std::slice::from_ref(&plan.id_str)), &meta),
            "should match ID '{}'",
            plan.id_str
        );

        // Multi-term AND: both must appear somewhere
        assert!(
            query::evaluate(
                &query::parse(&["ariana".to_string(), "lullaby".to_string()]),
                &meta
            ),
            "two bare terms should AND across different fields"
        );

        // Free + explicit clause AND
        assert!(
            query::evaluate(
                &query::parse(&["ariana".to_string(), "tag:creative".to_string()]),
                &meta
            ),
            "free term should AND with explicit tag clause"
        );

        // No match
        assert!(
            !query::evaluate(&query::parse(&["xyzzy".to_string()]), &meta),
            "unmatched bare term should return false"
        );

        // Path is excluded — find a substring that exists ONLY in the path,
        // not in folder name / vars / tags / template / id / template_name.
        // The base_dir component "projects" appears in the path but should
        // NOT be searchable as a free term.
        // (Defensive: only assert this if "projects" is genuinely absent
        // from the other fields, which it is for this fixture.)
        assert!(!plan.folder_name.to_lowercase().contains("projects"));
        assert!(!meta.template.to_lowercase().contains("projects"));
        assert!(!meta.template_name.to_lowercase().contains("projects"));
        assert!(
            !query::evaluate(&query::parse(&["projects".to_string()]), &meta),
            "bare term that lives only in path must NOT match"
        );
    });
}

/// Every template in `examples/templates/` must parse, validate **and plan** —
/// it is the public gallery users copy from, so a broken one is very visible.
///
/// Named individually rather than counted. The old version asserted
/// `seen >= 5` against eight templates, so three could rot away without the
/// suite noticing; and it stopped at `validate`, so the thing the name promised
/// — that each one can actually plan a project — was never checked.
#[test]
fn every_gallery_template_parses_validates_and_plans() {
    const GALLERY: [&str; 8] = [
        "finance-monthly",
        "music-video",
        "photography",
        "python-project",
        "research-note",
        "rust-project",
        "video-production",
        "web-project",
    ];

    let gallery = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("templates");

    let on_disk: Vec<String> = fs::read_dir(&gallery)
        .unwrap_or_else(|e| panic!("missing gallery at {}: {}", gallery.display(), e))
        .flatten()
        .filter(|entry| entry.path().join("template.yaml").exists())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let mut sorted = on_disk.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        GALLERY.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "the gallery on disk and the list this test checks have drifted apart"
    );

    for slug in GALLERY {
        let manifest = gallery.join(slug).join("template.yaml");
        let tmpl = template::Template::load_from_file(&manifest)
            .unwrap_or_else(|e| panic!("failed to parse {slug}: {e}"));
        tmpl.validate()
            .unwrap_or_else(|e| panic!("failed to validate {slug}: {e}"));

        // Plan one, with a sample answer for every declared variable: a select
        // takes its first option, and everything else takes a word. Planning is
        // where a naming pattern that references a variable nobody declared
        // shows up, and it writes nothing.
        sandboxed(|_install| {
            let vars: HashMap<String, String> = tmpl
                .variables
                .iter()
                .map(|var| {
                    let value = if var.var_type == template::VarType::Select {
                        var.options.first().cloned().unwrap_or_default()
                    } else if !var.default.is_empty() {
                        var.default.clone()
                    } else {
                        "Sample".to_string()
                    };
                    (var.slug.clone(), value)
                })
                .collect();
            let plan = project::plan(&tmpl, &vars, &Config::default(), &Counters::default())
                .unwrap_or_else(|e| panic!("failed to plan {slug}: {e}"));
            assert!(
                !plan.folder_name.is_empty(),
                "{slug} planned an empty folder name"
            );
            assert!(
                !plan.folder_name.contains('{'),
                "{slug} left an unresolved token in {}",
                plan.folder_name
            );
        });
    }
}

/// Two gallery templates declare structure and no `files/` subtree. That is
/// legal and deliberate — they scaffold folders, not documents — and this pins
/// it so a future "every template has files" assumption breaks here rather than
/// in somebody's project.
#[test]
fn a_gallery_template_may_declare_no_files_at_all() {
    let gallery = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("templates");

    for slug in ["photography", "video-production"] {
        let dir = gallery.join(slug);
        assert!(
            !dir.join("files").exists(),
            "{slug} is documented as structure-only"
        );
        let tmpl = template::Template::load_from_file(&dir.join("template.yaml")).unwrap();
        assert!(
            !tmpl.structure.is_empty(),
            "{slug} must at least declare folders"
        );
    }
}
