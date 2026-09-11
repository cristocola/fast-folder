//! Tags, `PROJECT_INFO.md` frontmatter, the notes and the todos.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, project, project_info, template};

mod common;

use common::env::with_fresh_install;
use common::fixtures::{minimal_template_yaml, write_template};

/// This binary's lock over the process environment — see `common::env`.
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

/// A note lands under `## Notes` — the section every new file has — and the
/// next one after it. No `## Journal` is ever opened: that heading is read,
/// and appended to, only where a file written before v3.6.0 already has it.
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
        let fresh = fs::read_to_string(&pinfo).unwrap();
        assert!(fresh.ends_with("## Notes\n\n"), "{fresh:?}");

        project_info::append_journal_entry(&pinfo, "first note").unwrap();
        let after_first = fs::read_to_string(&pinfo).unwrap();
        assert!(
            after_first.starts_with(&fresh),
            "every byte before the note stays"
        );
        assert!(after_first.contains("## Notes\n\n- "), "{after_first:?}");
        assert!(after_first.ends_with(" — first note\n"), "{after_first:?}");
        assert!(!after_first.contains("## Journal"));

        project_info::append_journal_entry(&pinfo, "second note").unwrap();
        let after_second = fs::read_to_string(&pinfo).unwrap();
        assert!(after_second.starts_with(&after_first));
        assert!(
            after_second.contains(" — first note\n- "),
            "{after_second:?}"
        );
        assert!(
            after_second.ends_with(" — second note\n"),
            "{after_second:?}"
        );
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
        project_info::append_journal_entry(&pinfo, "beta\nwith a second line\n\nand a third")
            .unwrap();

        let entries = project_info::read_journal_entries(&plan.root_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "alpha");
        assert_eq!(entries[1].text, "beta\nwith a second line\n\nand a third");
        assert!(
            entries.iter().all(|note| note
                .timestamp
                .as_deref()
                .is_some_and(|ts| ts.ends_with('Z'))),
            "every note fastf writes is dated: {entries:?}"
        );
    });
}

/// A note written after a heading the user added is still a note.
///
/// The body below the frontmatter is theirs — `docs/projects.md` says "After
/// creation the file is yours" — and adding any `##` of their own underneath
/// the journal used to put every later entry past the point the reader stops
/// at. `append_journal_entry` wrote at the end of the *file*;
/// `read_journal_entries` stopped at the next `##`. So the write succeeded, the
/// CLI printed the entry it had just saved, and it was never seen again. Both
/// now go through one `body::notes_span` — and for a file written before
/// v3.6.0 that span is its `## Journal`, so the file keeps its shape.
#[test]
fn a_note_after_a_heading_the_user_added_is_still_readable() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "journal-owned-body".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        let pinfo = project_info::pinfo_path(&plan.root_path);

        project_info::append_journal_entry(&pinfo, "before").unwrap();

        // The user keeps their own section under the journal.
        let mut content = fs::read_to_string(&pinfo).unwrap();
        content.push_str("\n## Archive\n\nthings I want to keep\n");
        fs::write(&pinfo, &content).unwrap();

        project_info::append_journal_entry(&pinfo, "after").unwrap();

        let entries = project_info::read_journal_entries(&plan.root_path).unwrap();
        let messages: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            messages,
            ["before", "after"],
            "a note fastf says it wrote must be a note fastf can read back"
        );

        // And the user's own section is still there, still below the notes.
        let after = fs::read_to_string(&pinfo).unwrap();
        assert!(after.contains("things I want to keep"));
        assert!(
            after.find("after").unwrap() < after.find("## Archive").unwrap(),
            "the entry belongs in the notes section, not after somebody else's"
        );

        // A file written before v3.6.0: its `## Journal` keeps the notes, and
        // the new line is the only change.
        let legacy = after.replace("## Notes\n\n- ", "## Notes\n\n## Journal\n\n- ");
        fs::write(&pinfo, &legacy).unwrap();
        project_info::append_journal_entry(&pinfo, "later").unwrap();
        let entries = project_info::read_journal_entries(&plan.root_path).unwrap();
        let ts = entries[2].timestamp.clone().unwrap();
        assert_eq!(
            fs::read_to_string(&pinfo).unwrap(),
            legacy.replace("— after\n", &format!("— after\n- {ts} — later\n")),
            "one line added under the journal, nothing else moved"
        );
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

// ---------------------------------------------------------------------------
// The pane's edits: set_variable, replace_tag, replace_note, the todos — and
// the tag rule
// ---------------------------------------------------------------------------

mod pane_edits {
    use super::*;
    use fastf::core::{library, operations, validated::Tag};

    /// A template with a text variable that drives a tag and a select, so an
    /// edit has a tag to keep honest and an option list to refuse against.
    const TEMPLATE: &str = r#"name: Client
slug: client
naming_pattern: "{id}_{name}"
id:
  prefix: C
  digits: 4
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: none
  - slug: tier
    label: Client tier
    type: select
    options: [Indie, Major]
    transform: none
  - slug: city
    label: City
    type: text
    transform: upper_underscore
tags: ["client"]
tag_from: ["tier"]
"#;

    /// One project from `TEMPLATE`, saved config, and the row that names it.
    fn planted(install: &Path) -> (Config, library::Project) {
        write_template(install, "client", TEMPLATE);
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();
        cfg.save().unwrap();
        let tmpl = template::find_by_slug("client").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "One".to_string());
        vars.insert("tier".to_string(), "Indie".to_string());
        vars.insert("city".to_string(), "new york".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        let project = library::resolve(&cfg, "C0001").unwrap();
        (cfg, project)
    }

    fn file(project: &library::Project) -> String {
        fs::read_to_string(project_info::pinfo_path(&project.path)).unwrap()
    }

    /// Setting a variable rewrites exactly three things — the value in the
    /// frontmatter, the tag derived from it, and the row of the body's table
    /// that mirrors it — and not one byte more.
    #[test]
    fn set_variable_rewrites_the_frontmatter_the_tag_and_the_table_and_nothing_else() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            operations::add_tags(&project, &["urgent".to_string()]).unwrap();
            let before = file(&project);
            assert!(before.contains("| Client tier | Indie    |"), "{before}");

            let meta = operations::set_variable(&project, "tier", "Major").unwrap();
            assert_eq!(meta.variables["tier"], "Major");
            assert!(
                meta.tags.contains(&"tier/Major".to_string()),
                "{:?}",
                meta.tags
            );
            assert!(
                !meta.tags.contains(&"tier/Indie".to_string()),
                "{:?}",
                meta.tags
            );
            assert!(meta.tags.contains(&"urgent".to_string()));
            assert!(meta.tags.contains(&"client".to_string()));
            assert_eq!(meta.auto_tags, vec!["tier/Major".to_string()]);

            let after = file(&project);
            assert!(after.contains("| Client tier | Major    |"), "{after}");
            // Everything that is not the frontmatter's tags/variables and the
            // table's one row is untouched: compare with those lines swapped.
            let expected = before
                .replace("  tier: Indie", "  tier: Major")
                .replace("- tier/Indie", "- tier/Major")
                .replace("| Client tier | Indie    |", "| Client tier | Major    |");
            assert_eq!(
                after, expected,
                "only the value, its tag and its table row moved"
            );
        });
    }

    /// A text variable lands the way a create would have stored it: through
    /// the template's transform and the filesystem sanitizer.
    #[test]
    fn set_variable_applies_the_templates_transform() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            let meta = operations::set_variable(&project, "city", " san francisco ").unwrap();
            assert_eq!(meta.variables["city"], "SAN_FRANCISCO");
            assert!(
                file(&project).contains("| City        | SAN_FRANCISCO |"),
                "{}",
                file(&project)
            );
        });
    }

    /// A `select` holds one of its options and nothing else, and the refusal
    /// names them — the same sentence a create would have given.
    #[test]
    fn set_variable_refuses_a_select_value_outside_its_options() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            let before = file(&project);
            let err = operations::set_variable(&project, "tier", "Boutique")
                .unwrap_err()
                .to_string();
            assert!(err.contains("Indie, Major"), "{err}");
            assert_eq!(file(&project), before, "a refused edit writes nothing");
            let err = operations::set_variable(&project, "name", "two\nlines")
                .unwrap_err()
                .to_string();
            assert!(err.contains("one line"), "{err}");
            let err = operations::set_variable(&project, "name", "  ")
                .unwrap_err()
                .to_string();
            assert!(err.contains("required"), "{err}");
        });
    }

    /// A table the user reshaped is theirs: the frontmatter still changes, the
    /// body does not.
    #[test]
    fn set_variable_leaves_a_table_that_is_not_fastfs_alone() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            let pinfo = project_info::pinfo_path(&project.path);
            let reshaped =
                file(&project).replace("| Variable    | Value    |", "| Field       | Value    |");
            fs::write(&pinfo, &reshaped).unwrap();
            let meta = operations::set_variable(&project, "tier", "Major").unwrap();
            assert_eq!(meta.variables["tier"], "Major");
            let after = file(&project);
            let (_, body) = project_info::split_frontmatter_body(&after).unwrap();
            let (_, body_before) = project_info::split_frontmatter_body(&reshaped).unwrap();
            assert_eq!(
                body, body_before,
                "a table that is not fastf's is not rewritten"
            );
        });
    }

    /// A tag edited on its row: renamed in place, or removed when emptied. The
    /// record of derived tags follows what is actually there.
    #[test]
    fn replace_tag_renames_in_place_and_empty_removes() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            operations::add_tags(&project, &["urgent".to_string(), "later".to_string()]).unwrap();
            let tags =
                operations::replace_tag(&project, "urgent", Some(&Tag::parse("soon").unwrap()))
                    .unwrap();
            assert_eq!(tags, vec!["client", "tier/Indie", "soon", "later"]);
            let tags = operations::replace_tag(&project, "later", None).unwrap();
            assert_eq!(tags, vec!["client", "tier/Indie", "soon"]);
            let tags =
                operations::replace_tag(&project, "soon", Some(&Tag::parse("client").unwrap()))
                    .unwrap();
            assert_eq!(
                tags,
                vec!["client", "tier/Indie"],
                "renaming onto a tag already there merges"
            );
            let tags = operations::replace_tag(&project, "tier/Indie", None).unwrap();
            assert_eq!(tags, vec!["client"]);
            let meta = project_info::read_metadata(&project.path).unwrap().unwrap();
            assert!(
                meta.auto_tags.is_empty(),
                "the record follows the tags: {:?}",
                meta.auto_tags
            );
            let err = operations::replace_tag(&project, "gone", None)
                .unwrap_err()
                .to_string();
            assert!(err.contains("no tag 'gone'"), "{err}");
        });
    }

    /// The undated note — the free text a file written before v3.6.0 kept
    /// under `## Notes` — is edited in place like any other note, with the
    /// entries under it untouched, and a heading in it is refused by name.
    #[test]
    fn replace_note_edits_the_undated_note_in_place_and_refuses_a_heading() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            operations::append_note(&project, "began").unwrap();
            let pinfo = project_info::pinfo_path(&project.path);
            let legacy = file(&project).replace(
                "## Notes\n\n- ",
                "## Notes\n\nfirst cut due Friday\n\n## Journal\n\n- ",
            );
            fs::write(&pinfo, &legacy).unwrap();
            let notes = project_info::read_journal_entries(&project.path).unwrap();
            assert_eq!(notes.len(), 2);
            assert_eq!(
                (notes[0].timestamp.as_deref(), notes[0].text.as_str()),
                (None, "first cut due Friday")
            );

            operations::replace_note(
                &project,
                0,
                "first cut due Friday",
                "first cut due Friday\nthen colour",
            )
            .unwrap();
            assert_eq!(
                file(&project),
                legacy.replace(
                    "first cut due Friday\n",
                    "first cut due Friday\nthen colour\n"
                ),
                "the note and only the note"
            );
            let err = operations::replace_note(
                &project,
                0,
                "first cut due Friday\nthen colour",
                "fine\n## Journal\nnot fine",
            )
            .unwrap_err()
            .to_string();
            assert!(
                err.contains("## Journal") && err.contains("end the notes"),
                "{err}"
            );
            operations::replace_note(&project, 0, "first cut due Friday\nthen colour", "").unwrap();
            assert_eq!(
                file(&project),
                legacy.replace("first cut due Friday\n\n", ""),
                "emptied reads as never written: {}",
                file(&project)
            );
        });
    }

    /// A note is one thing however many lines it has: written as one, read
    /// back as one, rewritten over its own lines and nothing else — and
    /// refused, untouched, when it is no longer the note the caller read.
    #[test]
    fn replace_note_rewrites_one_note_and_keeps_every_other_byte() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            operations::append_note(&project, "began").unwrap();
            operations::append_note(
                &project,
                "Timeline v02 has a render problem when h264\nrender with quicktime",
            )
            .unwrap();
            operations::append_note(
                &project,
                "client made a poem for me\n\noh you who edit my videos\nroad is long",
            )
            .unwrap();
            let notes = project_info::read_journal_entries(&project.path).unwrap();
            let texts: Vec<&str> = notes.iter().map(|n| n.text.as_str()).collect();
            assert_eq!(
                texts,
                [
                    "began",
                    "Timeline v02 has a render problem when h264\nrender with quicktime",
                    "client made a poem for me\n\noh you who edit my videos\nroad is long",
                ]
            );
            let before = file(&project);

            operations::replace_note(&project, 1, &notes[1].text, "Timeline v02 renders fine now")
                .unwrap();
            let after = file(&project);
            assert_eq!(
                after,
                before.replace(
                    " — Timeline v02 has a render problem when h264\n  render with quicktime\n",
                    " — Timeline v02 renders fine now\n"
                ),
                "one note rewritten, every other byte kept"
            );

            // A stale expectation is refused and writes nothing.
            let err = operations::replace_note(&project, 1, &notes[1].text, "again")
                .unwrap_err()
                .to_string();
            assert!(err.contains("changed meanwhile"), "{err}");
            assert_eq!(file(&project), after);

            // Empty text takes the note out.
            operations::replace_note(&project, 0, "began", "").unwrap();
            let notes = project_info::read_journal_entries(&project.path).unwrap();
            assert_eq!(notes.len(), 2);
            assert_eq!(notes[0].text, "Timeline v02 renders fine now");
            assert!(
                file(&project).contains("## Notes\n\n- "),
                "{}",
                file(&project)
            );
        });
    }

    /// A todo is added at the end of `## Todo` — opened after the notes when
    /// there is none — and toggled by the one character in its brackets.
    #[test]
    fn a_todo_is_added_and_toggled_under_the_lock() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            operations::append_note(&project, "began").unwrap();
            let before = file(&project);
            operations::add_todo(&project, "ingested the videos").unwrap();
            operations::add_todo(&project, "delivered the video").unwrap();
            let after = file(&project);
            assert_eq!(
                after,
                format!(
                    "{before}\n## Todo\n\n- [ ] ingested the videos\n- [ ] delivered the video\n"
                )
            );
            let todos = fastf::core::body::read_todos(&project.path).unwrap();
            assert_eq!(
                todos
                    .iter()
                    .map(|t| (t.done, t.text.as_str()))
                    .collect::<Vec<_>>(),
                [
                    (false, "ingested the videos"),
                    (false, "delivered the video")
                ]
            );

            assert!(operations::toggle_todo(&project, 0, "ingested the videos").unwrap());
            assert_eq!(
                file(&project),
                after.replace("- [ ] ingested", "- [x] ingested"),
                "one byte"
            );
            assert!(!operations::toggle_todo(&project, 0, "ingested the videos").unwrap());
            assert_eq!(file(&project), after);

            let err = operations::toggle_todo(&project, 0, "something else")
                .unwrap_err()
                .to_string();
            assert!(err.contains("changed meanwhile"), "{err}");
            for bad in ["", "   ", "two\nlines"] {
                assert!(operations::add_todo(&project, bad).is_err(), "{bad:?}");
            }
            assert_eq!(file(&project), after);
            // The notes are untouched by any of it.
            assert_eq!(
                project_info::read_journal_entries(&project.path)
                    .unwrap()
                    .len(),
                1
            );
        });
    }

    /// Every tag comes through one door, and a poem does not fit through it.
    #[test]
    fn add_tags_refuses_what_is_not_a_tag() {
        sandboxed(|install| {
            let (_cfg, project) = planted(install);
            for bad in [
                "a poem about tags",
                "",
                "client/",
                "two\nlines",
                "no,commas",
            ] {
                let err = operations::add_tags(&project, &[bad.to_string()])
                    .unwrap_err()
                    .to_string();
                assert!(
                    err.contains("a tag") || err.contains("a '/'"),
                    "{bad:?}: {err}"
                );
            }
            let tags = operations::add_tags(&project, &["  ok-tag.v2  ".to_string()]).unwrap();
            assert!(
                tags.contains(&"ok-tag.v2".to_string()),
                "trimmed, then kept: {tags:?}"
            );
        });
    }
}
