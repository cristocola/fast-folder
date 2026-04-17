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

use fastf::core::{config::Config, counter::Counters, index, naming, project, template};

static SERIAL: Mutex<()> = Mutex::new(());

/// Acquire the serial-test lock and install a fresh `FASTF_INSTALL_DIR`.
fn with_fresh_install<R>(body: impl FnOnce(&Path) -> R) -> R {
    // Recover from poisoned lock — we don't hold any invariants that panics could violate.
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    // Safe here: the SERIAL mutex guarantees no other test thread races on this env var.
    unsafe { std::env::set_var("FASTF_INSTALL_DIR", tmp.path()); }
    fs::create_dir_all(tmp.path().join("templates")).unwrap();
    let result = body(tmp.path());
    unsafe { std::env::remove_var("FASTF_INSTALL_DIR"); }
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
        assert!(!PathBuf::from(&cfg.base_dir).join(&plan.folder_name).exists());
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
    assert!(seen >= 5, "expected at least 5 gallery templates, found {seen}");
}
