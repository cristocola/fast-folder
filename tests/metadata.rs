//! Tags, `PROJECT_INFO.md` frontmatter, and the journal.
//!
//! Split out of the single 2700-line `integration.rs`, whose 67 tests all
//! queued behind one mutex in one binary.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, project, project_info, template};

mod common;

use common::env::with_fresh_install;
use common::fixtures::{minimal_template_yaml, write_template};

/// This binary's own lock. `FASTF_INSTALL_DIR` and `HOME` are process-wide, so
/// every test in a binary shares one — and separate binaries are separate
/// processes, which is what lets these suites run in parallel with each other.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

// ---------------------------------------------------------------------------
// Tags — write_frontmatter + auto-tag + tag CLI
// ---------------------------------------------------------------------------

/// Template with `tags` and `tag_from` should produce combined tags in frontmatter.
#[test]
fn auto_tag_from_template_tag_from() {
    sandboxed(|install| {
        let yaml = r#"name: Tagged
slug: tagged
naming_pattern: "{id}_{name}"
id:
  prefix: T
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: title_underscore
  - slug: client_type
    label: Client type
    type: text
tags:
  - creative
tag_from:
  - client_type
"#;
        write_template(install, "tagged", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("tagged").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "My Project".to_string());
        vars.insert("client_type".to_string(), "Indie".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();

        assert!(
            meta.tags.contains(&"creative".to_string()),
            "literal tag should be present: {:?}",
            meta.tags
        );
        assert!(
            meta.tags.contains(&"client_type/Indie".to_string()),
            "derived tag should be present: {:?}",
            meta.tags
        );
        assert_eq!(meta.tags.len(), 2);
    });
}

/// Empty tag_from value should not produce an orphan `slug/` tag.
#[test]
fn auto_tag_skips_empty_variable_value() {
    sandboxed(|install| {
        let yaml = r#"name: Tagged2
slug: tagged2
naming_pattern: "{id}"
variables:
  - slug: client_type
    label: Client type
    type: text
tag_from:
  - client_type
"#;
        write_template(install, "tagged2", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("tagged2").unwrap();
        // leave client_type empty
        let vars = HashMap::new();
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        assert!(
            meta.tags.is_empty(),
            "should have no tags when variable is empty: {:?}",
            meta.tags
        );
    });
}

/// write_frontmatter preserves the body bytes unchanged when only tags are mutated.
#[test]
fn write_frontmatter_body_bytes_preserved() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "roundtrip".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        // Record the body section before mutation.
        let before = fs::read_to_string(&pinfo).unwrap();
        let (_, body_before) = project_info::split_frontmatter_body(&before).unwrap();
        let body_before = body_before.to_string();

        // Mutate via write_frontmatter.
        project_info::write_frontmatter(&pinfo, |meta| {
            meta.tags.push("draft".to_string());
        })
        .unwrap();

        // Read back and compare body.
        let after = fs::read_to_string(&pinfo).unwrap();
        let (_, body_after) = project_info::split_frontmatter_body(&after).unwrap();

        assert_eq!(
            body_before, body_after,
            "body bytes must be identical after frontmatter mutation"
        );

        // Tag must be present.
        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        assert!(meta.tags.contains(&"draft".to_string()));
    });
}

/// write_frontmatter returns a structured error when frontmatter is missing.
#[test]
fn write_frontmatter_errors_on_missing_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("plain.md");
    fs::write(&path, "# No frontmatter here\n\nJust text.\n").unwrap();

    let result = project_info::write_frontmatter(&path, |_| {});
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("no YAML frontmatter"),
        "error should mention missing frontmatter: {msg}"
    );
}

/// append_journal_entry creates the section when it doesn't exist and appends
/// additional entries chronologically.
#[test]
fn append_journal_entry_creates_and_appends() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "journal-test".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        // No journal section yet.
        let content = fs::read_to_string(&pinfo).unwrap();
        assert!(
            !content.contains("## Journal"),
            "no journal before first append"
        );

        // First entry — should create the section.
        project_info::append_journal_entry(&pinfo, "first note").unwrap();
        let after_first = fs::read_to_string(&pinfo).unwrap();
        assert!(after_first.contains("## Journal"));
        assert!(after_first.contains("first note"));

        // Second entry — appended after first.
        project_info::append_journal_entry(&pinfo, "second note").unwrap();
        let after_second = fs::read_to_string(&pinfo).unwrap();
        assert!(after_second.contains("first note"));
        assert!(after_second.contains("second note"));
        // Chronological: first appears before second.
        let pos_first = after_second.find("first note").unwrap();
        let pos_second = after_second.find("second note").unwrap();
        assert!(pos_first < pos_second, "entries should be chronological");
    });
}

/// read_journal_entries parses entries from the file correctly.
#[test]
fn journal_entries_round_trip() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "journal-rtrip".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        project_info::append_journal_entry(&pinfo, "alpha").unwrap();
        project_info::append_journal_entry(&pinfo, "beta").unwrap();

        let entries = project_info::read_journal_entries(&plan.root_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "alpha");
        assert_eq!(entries[1].message, "beta");
    });
}

/// Tags added via write_frontmatter persist across a restart (re-parse).
#[test]
fn tag_add_persists() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "tag-persist".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        // Add twice — should be idempotent.
        project_info::write_frontmatter(&pinfo, |m| {
            if !m.tags.contains(&"draft".to_string()) {
                m.tags.push("draft".to_string());
            }
        })
        .unwrap();
        project_info::write_frontmatter(&pinfo, |m| {
            if !m.tags.contains(&"draft".to_string()) {
                m.tags.push("draft".to_string());
            }
        })
        .unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        let count = meta.tags.iter().filter(|t| t.as_str() == "draft").count();
        assert_eq!(
            count, 1,
            "idempotent add should not duplicate: {:?}",
            meta.tags
        );

        // Remove.
        project_info::write_frontmatter(&pinfo, |m| m.tags.retain(|t| t != "draft")).unwrap();
        let meta2 = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        assert!(!meta2.tags.contains(&"draft".to_string()));
    });
}

/// Older PROJECT_INFO.md without tags: field loads with an empty tags vec.
#[test]
fn legacy_metadata_without_tags_loads_cleanly() {
    // Simulate a file written before tagging was introduced.
    let legacy_yaml = r#"---
id: ID0001
template: old-template
template_name: Old Template
created: "2025-01-01T00:00:00Z"
folder: ID0001_Project
path: /projects/ID0001_Project
variables:
  name: hello
---

# Project Info

## Notes

"#;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let file = dir.join("PROJECT_INFO.md");
    fs::write(&file, legacy_yaml).unwrap();

    let meta = project_info::read_metadata(dir).unwrap().unwrap();
    assert_eq!(meta.id, "ID0001");
    assert!(
        meta.tags.is_empty(),
        "legacy file should deserialize with empty tags"
    );
}
