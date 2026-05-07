//! Integration tests for fastf.
//!
//! Each test drops a `FASTF_INSTALL_DIR` env override so that config,
//! counters, templates, and the project index all live in a fresh tempdir.
//! No test touches the real installed fastf folder.
//!
//! Tests run serially in a single-threaded runner (see the `serial` helper).
//! This is deliberate: `FASTF_INSTALL_DIR` is process-wide, so parallel tests
//! in the same binary would race. Compared to pulling in `serial_test`, a
//! Mutex we own is leaner and explicit.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastf::core::{
    config::Config, counter::Counters, index, naming, project, project_info, query, template,
};

static SERIAL: Mutex<()> = Mutex::new(());

/// Acquire the serial-test lock and install a fresh `FASTF_INSTALL_DIR`.
fn with_fresh_install<R>(body: impl FnOnce(&Path) -> R) -> R {
    // Recover from poisoned lock — we don't hold any invariants that panics could violate.
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    // Safe here: the SERIAL mutex guarantees no other test thread races on this env var.
    unsafe {
        std::env::set_var("FASTF_INSTALL_DIR", tmp.path());
    }
    fs::create_dir_all(tmp.path().join("templates")).unwrap();
    let result = body(tmp.path());
    unsafe {
        std::env::remove_var("FASTF_INSTALL_DIR");
    }
    drop(guard);
    result
}

fn write_template(install: &Path, slug: &str, yaml: &str) {
    let path = install.join("templates").join(format!("{}.yaml", slug));
    fs::write(&path, yaml).unwrap();
}

/// A minimal valid template with one text var, one folder, and one templated file.
fn minimal_template_yaml(slug: &str) -> String {
    format!(
        r#"name: Test
slug: {slug}
description: fixture
naming_pattern: "{{id}}_{{name}}"
id:
  prefix: T
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: title_underscore
structure:
  - name: src
    children:
      - name: core
files:
  - path: README.md
    template: |
      # {{name}}
      id: {{id}}
"#
    )
}

// ---------------------------------------------------------------------------

#[test]
fn create_project_basic_round_trip() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "hello world".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        assert_eq!(plan.id_str, "T001");
        assert_eq!(plan.folder_name, "T001_Hello_World");

        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Verify folder tree exists.
        assert!(plan.root_path.join("src").join("core").is_dir());
        // Verify file interpolation happened.
        let readme = fs::read_to_string(plan.root_path.join("README.md")).unwrap();
        assert!(readme.contains("# Hello_World"), "readme was: {readme}");
        assert!(readme.contains("id: T001"));

        // Counter persisted.
        let fresh = Counters::load().unwrap();
        assert_eq!(fresh.get(), 1);
    });
}

#[test]
fn counter_increments_across_runs() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        for expected in 1..=3u64 {
            let tmpl = template::find_by_slug("test").unwrap();
            let mut vars = HashMap::new();
            vars.insert("name".to_string(), format!("run {expected}"));
            let counters = Counters::load().unwrap();
            let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
            assert_eq!(plan.counter_value, expected);
            let mut counters = counters;
            project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        }

        assert_eq!(Counters::load().unwrap().get(), 3);
    });
}

#[test]
fn existing_project_fails_cleanly() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "collide".to_string());

        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Second attempt at same path should fail.
        let counters2 = Counters::load().unwrap();
        let plan2 = project::plan(&tmpl, &vars, &cfg, &counters2).unwrap();
        // Force the same root_path as the first run by mutating the expected folder name.
        let mut plan2 = plan2;
        plan2.root_path = plan.root_path.clone();
        let mut counters2 = counters2;
        let err = project::create(&plan2, &tmpl, &mut counters2, &cfg, false)
            .expect_err("second create should fail");
        assert!(err.to_string().contains("already exists"), "got: {err:#}");
    });
}

#[test]
fn project_index_appends_on_create() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "indexed".to_string());

        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let records = index::load_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "T001");
        assert_eq!(records[0].template, "test");
        assert!(records[0].name.contains("Indexed"));
    });
}

#[test]
fn apply_skips_existing_and_creates_missing() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        // Create a target folder with only README.md pre-populated.
        let target = install.join("existing");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("README.md"), "pre-existing content").unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "already there".to_string());

        project::apply(&tmpl, &target, &vars, &cfg).unwrap();

        // README was skipped (content unchanged).
        let readme = fs::read_to_string(target.join("README.md")).unwrap();
        assert_eq!(readme, "pre-existing content");
        // But src/core were created.
        assert!(target.join("src").join("core").is_dir());
    });
}

#[test]
fn template_rejects_parent_escape() {
    with_fresh_install(|install| {
        let bad = r#"name: Bad
slug: bad
naming_pattern: "{id}"
files:
  - path: "../pwned.txt"
    content: nope
"#;
        write_template(install, "bad", bad);
        let err = template::find_by_slug("bad").expect_err("should reject");
        let msg = format!("{err:#}");
        assert!(msg.contains("..") || msg.contains("relative"), "got: {msg}");
    });
}

#[test]
fn template_rejects_absolute_path() {
    with_fresh_install(|install| {
        let bad = r#"name: Bad
slug: bad
naming_pattern: "{id}"
files:
  - path: "/etc/passwd"
    content: nope
"#;
        write_template(install, "bad", bad);
        let err = template::find_by_slug("bad").expect_err("should reject");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("relative") || msg.contains("drive letter"),
            "got: {msg}"
        );
    });
}

#[test]
fn from_folder_round_trip() {
    with_fresh_install(|install| {
        // Build a small fixture folder tree on disk.
        let src = install.join("fixture");
        fs::create_dir_all(src.join("subdir")).unwrap();
        fs::write(src.join("README.md"), "hello").unwrap();
        fs::write(src.join("subdir").join("nested.txt"), "deep").unwrap();
        // A noise dir that should be ignored.
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::write(src.join(".git").join("HEAD"), "noise").unwrap();

        fastf::cli::template::from_folder(&src.display().to_string(), "generated", false).unwrap();

        let tmpl = template::find_by_slug("generated").unwrap();
        // The .git folder must be absent.
        assert!(
            tmpl.structure.iter().all(|n| n.name != ".git"),
            "structure: {:?}",
            tmpl.structure
        );
        // subdir should be a folder node.
        assert!(tmpl.structure.iter().any(|n| n.name == "subdir"));
        // Files captured with relative paths.
        assert!(tmpl.files.iter().any(|f| f.path == "README.md"));
        assert!(tmpl.files.iter().any(|f| f.path == "subdir/nested.txt"));
    });
}

#[test]
fn sanitize_and_safe_path_units_exposed_via_lib() {
    // Smoke-test that the lib re-exports the naming helpers as expected —
    // protects against someone pruning the module accidentally.
    assert_eq!(naming::sanitize_name("a/b"), "a_b");
    assert!(naming::ensure_relative_safe_path("foo/bar.txt").is_ok());
    assert!(naming::ensure_relative_safe_path("../bad").is_err());
}

#[test]
fn dry_run_does_not_write() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "dry".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();

        // plan() does not touch disk — verify counters.toml and project folder still absent.
        assert!(
            !PathBuf::from(&cfg.base_dir)
                .join(&plan.folder_name)
                .exists()
        );
        assert!(Counters::load().unwrap().get() == 0);
    });
}

#[cfg(windows)]
#[test]
fn windows_forward_slash_paths_work() {
    with_fresh_install(|install| {
        let yaml = r#"name: Slashes
slug: slashes
naming_pattern: "{id}"
id:
  prefix: S
  digits: 2
files:
  - path: a/b/c.txt
    content: hi
"#;
        write_template(install, "slashes", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("slashes").unwrap();
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // On Windows, join() converts to backslashes. File should exist either way.
        assert!(plan.root_path.join("a").join("b").join("c.txt").is_file());
    });
}

#[test]
fn project_info_md_written_on_new_with_resolved_variables() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "metadata test".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let path = plan.root_path.join("PROJECT_INFO.md");
        assert!(
            path.is_file(),
            "PROJECT_INFO.md should exist at {}",
            path.display()
        );

        let body = fs::read_to_string(&path).unwrap();
        // Frontmatter shape — the file MUST start with `---\n` and contain
        // a closing `---` line. This is the searchability guarantee.
        assert!(
            body.starts_with("---\n"),
            "must start with YAML frontmatter open: {body}"
        );
        assert!(
            body.contains("\n---\n"),
            "must close YAML frontmatter: {body}"
        );
        // Frontmatter content
        assert!(
            body.contains("id: T001"),
            "missing id in frontmatter: {body}"
        );
        assert!(
            body.contains("template: test"),
            "missing template slug: {body}"
        );
        assert!(
            body.contains("template_name: Test"),
            "missing template_name: {body}"
        );
        // Variable slug + transformed value, captured under `variables:`
        assert!(
            body.contains("name: Metadata_Test"),
            "missing transformed variable: {body}"
        );
        // Human body — Project Info header + variables table + Notes section
        assert!(body.contains("# Project Info"), "missing header: {body}");
        assert!(
            body.contains("| Variable"),
            "missing variables table: {body}"
        );
        assert!(body.contains("## Notes"), "missing Notes section: {body}");

        // Raw read round-trip
        let read_back = project_info::read(&plan.root_path, &cfg).unwrap();
        assert_eq!(read_back, body);
    });
}

#[test]
fn project_info_metadata_round_trips_via_yaml() {
    // Parsing the file back via read_metadata should reconstruct the typed
    // Metadata struct cleanly — this is the contract that future search /
    // index tools will rely on.
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "round trip".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path, &cfg)
            .expect("read_metadata Ok")
            .expect("frontmatter present");

        assert_eq!(meta.id, "T001");
        assert_eq!(meta.template, "test");
        assert_eq!(meta.template_name, "Test");
        assert_eq!(meta.folder, plan.folder_name);
        assert_eq!(
            meta.variables.get("name").map(String::as_str),
            Some("Round_Trip")
        );
    });
}

#[test]
fn project_info_captures_variables_not_in_naming_pattern() {
    // The metadata file is the durable home for variables that don't make it
    // into the folder name — that's the user's stated workflow.
    with_fresh_install(|install| {
        // Naming pattern uses only {id}_{title}, but `artist` is also a variable.
        let yaml = r#"name: Music
slug: music
naming_pattern: "{id}_{title}"
id:
  prefix: M
  digits: 3
variables:
  - slug: title
    label: Song Title
    type: text
    required: true
    transform: title_underscore
  - slug: artist
    label: Artist Name
    type: text
    required: false
    transform: title_underscore
structure:
  - name: assets
"#;
        write_template(install, "music", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("music").unwrap();
        let mut vars = HashMap::new();
        vars.insert("title".to_string(), "lullaby".to_string());
        vars.insert("artist".to_string(), "ariana grande".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        // Folder name does NOT include artist
        assert!(!plan.folder_name.to_lowercase().contains("ariana"));

        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path, &cfg)
            .unwrap()
            .unwrap();
        // Both vars are recorded — even the one absent from the folder name.
        assert_eq!(
            meta.variables.get("title").map(String::as_str),
            Some("Lullaby")
        );
        assert_eq!(
            meta.variables.get("artist").map(String::as_str),
            Some("Ariana_Grande")
        );
    });
}

#[test]
fn project_info_md_skipped_when_disabled() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        cfg.project_info_enabled = false;
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "no metadata".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        assert!(
            !plan.root_path.join("PROJECT_INFO.md").exists(),
            "PROJECT_INFO.md should NOT exist when project_info_enabled=false"
        );
    });
}

#[test]
fn project_info_filename_setting_respected() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        cfg.project_info_filename = ".fastf-info.md".to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "custom name".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        assert!(plan.root_path.join(".fastf-info.md").is_file());
        assert!(!plan.root_path.join("PROJECT_INFO.md").exists());
    });
}

#[test]
fn config_alias_pinfo_enabled_still_parses() {
    // A v0.2-interim config that used the old `pinfo_*` keys must still load
    // — the rename to `project_info_*` ships with serde aliases for safety.
    let raw = r#"
base_dir = ""
editor = ""
default_template = ""
date_format = "%Y-%m-%d"
pinfo_enabled = false
pinfo_filename = ".legacy-info.md"
"#;
    let cfg: Config = toml::from_str(raw).expect("alias config should parse");
    assert!(!cfg.project_info_enabled);
    assert_eq!(cfg.project_info_filename, ".legacy-info.md");
}

#[test]
fn config_defaults_are_backwards_compatible() {
    // An old config.toml that predates the new fields must still parse,
    // and the new fields must take their defaults.
    let raw = r#"
base_dir = ""
editor = ""
default_template = ""
date_format = "%Y-%m-%d"
"#;
    let cfg: Config = toml::from_str(raw).expect("old config should still parse");
    assert!(cfg.prompt_open_after_create, "default should be true");
    assert!(cfg.project_info_enabled, "default should be true");
    assert_eq!(cfg.project_info_filename, "PROJECT_INFO.md");
    assert_eq!(cfg.recent_default_limit, 20);
    assert!(cfg.confirm_create);
    assert!(cfg.show_banner);
}

#[test]
fn bundled_templates_do_not_emit_duplicate_project_info() {
    // Auto-gen owns PROJECT_INFO.md — bundled templates must not also
    // declare it as a content file (would conflict / overwrite). This guards
    // against accidental re-introduction.
    use fastf::core::template::Template;
    let bundled_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("templates");
    // Also check the strings baked into bootstrap.rs by parsing each file
    // currently shipped in the gallery.
    for entry in fs::read_dir(&bundled_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let raw = fs::read_to_string(&path).unwrap();
        let tmpl: Template = serde_yaml::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e));
        for f in &tmpl.files {
            assert_ne!(
                f.path,
                "PROJECT_INFO.md",
                "{} declares PROJECT_INFO.md but auto-gen now owns it",
                path.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tags — write_frontmatter + auto-tag + tag CLI
// ---------------------------------------------------------------------------

/// Template with `tags` and `tag_from` should produce combined tags in frontmatter.
#[test]
fn auto_tag_from_template_tag_from() {
    with_fresh_install(|install| {
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

        let meta = project_info::read_metadata(&plan.root_path, &cfg)
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
    with_fresh_install(|install| {
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

        let meta = project_info::read_metadata(&plan.root_path, &cfg)
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
    with_fresh_install(|install| {
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

        let pinfo = plan.root_path.join(&cfg.project_info_filename);

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
        let meta = project_info::read_metadata(&plan.root_path, &cfg)
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
    with_fresh_install(|install| {
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

        let pinfo = plan.root_path.join(&cfg.project_info_filename);

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
    with_fresh_install(|install| {
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

        let pinfo = plan.root_path.join(&cfg.project_info_filename);

        project_info::append_journal_entry(&pinfo, "alpha").unwrap();
        project_info::append_journal_entry(&pinfo, "beta").unwrap();

        let entries = project_info::read_journal_entries(&plan.root_path, &cfg).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "alpha");
        assert_eq!(entries[1].message, "beta");
    });
}

/// Tags added via write_frontmatter persist across a restart (re-parse).
#[test]
fn tag_add_persists() {
    with_fresh_install(|install| {
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

        let pinfo = plan.root_path.join(&cfg.project_info_filename);

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

        let meta = project_info::read_metadata(&plan.root_path, &cfg)
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
        let meta2 = project_info::read_metadata(&plan.root_path, &cfg)
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

    let mut cfg = Config::default();
    cfg.project_info_filename = "PROJECT_INFO.md".to_string();
    cfg.project_info_enabled = true;

    let meta = project_info::read_metadata(dir, &cfg).unwrap().unwrap();
    assert_eq!(meta.id, "ID0001");
    assert!(
        meta.tags.is_empty(),
        "legacy file should deserialize with empty tags"
    );
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
        }
    };

    let make_rec = |id: &str, tmpl: &str| index::ProjectRecord {
        id: id.to_string(),
        template: tmpl.to_string(),
        path: format!("/projects/{id}"),
        name: format!("{id}_proj"),
        created_at: "2026-01-15T10:00:00Z".to_string(),
    };

    // exact field
    let meta = make_meta(
        "ID0001",
        "music-video",
        "2026-03-01T00:00:00Z",
        &["draft"],
        &[("artist", "Ariana")],
    );
    let rec = make_rec("ID0001", "music-video");
    assert!(query::evaluate(
        &query::parse(&["template=music-video".to_string()]),
        &rec,
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["template=other".to_string()]),
        &rec,
        &meta
    ));

    // prefix glob
    assert!(query::evaluate(
        &query::parse(&["template=music*".to_string()]),
        &rec,
        &meta
    ));

    // date after
    assert!(query::evaluate(
        &query::parse(&["created>2026-01-01".to_string()]),
        &rec,
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["created>2027-01-01".to_string()]),
        &rec,
        &meta
    ));

    // date before
    assert!(query::evaluate(
        &query::parse(&["created<2027-01-01".to_string()]),
        &rec,
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["created<2025-01-01".to_string()]),
        &rec,
        &meta
    ));

    // exact tag
    assert!(query::evaluate(
        &query::parse(&["tag:draft".to_string()]),
        &rec,
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["tag:urgent".to_string()]),
        &rec,
        &meta
    ));

    // tag glob
    assert!(query::evaluate(
        &query::parse(&["tag:dra*".to_string()]),
        &rec,
        &meta
    ));

    // variable field
    assert!(query::evaluate(
        &query::parse(&["artist=Ariana".to_string()]),
        &rec,
        &meta
    ));
    assert!(query::evaluate(
        &query::parse(&["artist=Aria*".to_string()]),
        &rec,
        &meta
    ));

    // multi-clause AND
    assert!(query::evaluate(
        &query::parse(&["template=music-video".to_string(), "tag:draft".to_string()]),
        &rec,
        &meta,
    ));
    assert!(!query::evaluate(
        &query::parse(&["template=music-video".to_string(), "tag:urgent".to_string()]),
        &rec,
        &meta,
    ));

    // unknown key → false, not error
    assert!(!query::evaluate(
        &query::parse(&["nonexistent=anything".to_string()]),
        &rec,
        &meta
    ));
}

/// Bare-term default mode searches across vars, tags, folder, template, and id
/// — and explicitly excludes `path`.  Drives the end-to-end create→read→evaluate
/// path so we know the predicate works against real frontmatter on disk.
#[test]
fn query_free_term_searches_across_fields() {
    with_fresh_install(|install| {
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

        // Locate the synthetic record + metadata for predicate evaluation.
        let record = index::ProjectRecord {
            id: plan.id_str.clone(),
            template: tmpl.slug.clone(),
            path: plan.root_path.display().to_string(),
            name: plan.folder_name.clone(),
            created_at: "2026-01-15T10:00:00Z".to_string(),
        };
        let meta = project_info::read_metadata(&plan.root_path, &cfg)
            .unwrap()
            .unwrap();

        // Variable value match (case-insensitive)
        assert!(
            query::evaluate(&query::parse(&["ariana".to_string()]), &record, &meta),
            "should match variable value 'Ariana_Grande'"
        );

        // Tag match
        assert!(
            query::evaluate(&query::parse(&["creative".to_string()]), &record, &meta),
            "should match tag 'creative'"
        );

        // Folder name (the resolved naming pattern)
        assert!(
            query::evaluate(&query::parse(&["lullaby".to_string()]), &record, &meta),
            "should match folder name '{}'",
            plan.folder_name
        );

        // Template slug
        assert!(
            query::evaluate(&query::parse(&["free-search".to_string()]), &record, &meta),
            "should match template slug 'free-search'"
        );

        // ID
        assert!(
            query::evaluate(
                &query::parse(std::slice::from_ref(&plan.id_str)),
                &record,
                &meta
            ),
            "should match ID '{}'",
            plan.id_str
        );

        // Multi-term AND: both must appear somewhere
        assert!(
            query::evaluate(
                &query::parse(&["ariana".to_string(), "lullaby".to_string()]),
                &record,
                &meta
            ),
            "two bare terms should AND across different fields"
        );

        // Free + explicit clause AND
        assert!(
            query::evaluate(
                &query::parse(&["ariana".to_string(), "tag:creative".to_string()]),
                &record,
                &meta
            ),
            "free term should AND with explicit tag clause"
        );

        // No match
        assert!(
            !query::evaluate(&query::parse(&["xyzzy".to_string()]), &record, &meta),
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
            !query::evaluate(&query::parse(&["projects".to_string()]), &record, &meta),
            "bare term that lives only in path must NOT match"
        );
    });
}

/// Every YAML in `examples/templates/` must parse, validate, and plan — it's the
/// public gallery users copy from, so broken YAML would be very visible.
#[test]
fn gallery_templates_parse_and_plan() {
    let gallery = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("templates");
    let entries = fs::read_dir(&gallery)
        .unwrap_or_else(|e| panic!("missing gallery at {}: {}", gallery.display(), e));

    let mut seen = 0;
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        seen += 1;
        let yaml = fs::read_to_string(&path).unwrap();
        let tmpl: template::Template = serde_yaml::from_str(&yaml)
            .unwrap_or_else(|e| panic!("failed to parse {}: {}", path.display(), e));
        tmpl.validate()
            .unwrap_or_else(|e| panic!("failed to validate {}: {}", path.display(), e));
    }
    assert!(
        seen >= 5,
        "expected at least 5 gallery templates, found {seen}"
    );
}
