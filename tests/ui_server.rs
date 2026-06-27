//! Integration tests for the browser-UI request layer (`fastf::ui`).
//!
//! These exercise the pure router `ui::route_request` directly — no socket —
//! against a fresh `FASTF_INSTALL_DIR` tempdir, mirroring the harness in
//! `integration.rs`. They prove the UI shares the real Fast Folder creation
//! path (folder on disk + index append) and that embedded frontend assets are
//! served.

#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::{config::Config, index};
use fastf::ui::{self, Response};

static SERIAL: Mutex<()> = Mutex::new(());

fn with_fresh_install<R>(body: impl FnOnce(&Path) -> R) -> R {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    // Safe: SERIAL guarantees no other test thread races on this process-wide var.
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

fn write_minimal_template(install: &Path, slug: &str) {
    let yaml = format!(
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
files:
  - path: README.md
    template: |
      # {{name}}
      id: {{id}}
"#
    );
    fs::write(install.join("templates").join(format!("{slug}.yaml")), yaml).unwrap();
}

/// Write a config.toml whose base_dir points inside the sandbox, so the UI's
/// `Config::load()` resolves projects into the tempdir.
fn write_config(install: &Path) -> String {
    let mut cfg = Config::default();
    let base = install.join("projects");
    fs::create_dir_all(&base).unwrap();
    cfg.base_dir = base.display().to_string();
    cfg.save().unwrap();
    cfg.base_dir
}

fn json(method: &str, route: &str, body: serde_json::Value) -> serde_json::Value {
    match ui::route_request(method, route, body.to_string().as_bytes()).unwrap() {
        Response::Json(value) => value,
        Response::Static(..) => panic!("expected JSON response for {method} {route}"),
    }
}

#[test]
fn health_route_responds_ok() {
    with_fresh_install(|_install| {
        let value = json("GET", "/api/health", serde_json::Value::Null);
        assert_eq!(value["ok"], true);
    });
}

#[test]
fn preview_produces_plan_without_writing() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        let base = write_config(install);

        let value = json(
            "POST",
            "/api/preview",
            serde_json::json!({"template": "test", "variables": {"name": "hello world"}}),
        );
        assert_eq!(value["folder_name"], "T001_Hello_World");
        assert_eq!(value["id"], "T001");

        // Preview must not create anything on disk.
        assert!(
            fs::read_dir(&base).unwrap().next().is_none(),
            "preview should not write any project folder"
        );
        // Counter untouched.
        assert!(index::load_all().unwrap().is_empty());
    });
}

#[test]
fn create_makes_folder_and_appends_index() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        let base = write_config(install);

        let value = json(
            "POST",
            "/api/create",
            serde_json::json!({"template": "test", "variables": {"name": "hello world"}}),
        );
        assert_eq!(value["ok"], true);

        // Real folder + interpolated file on disk.
        let root = Path::new(&base).join("T001_Hello_World");
        assert!(root.join("src").is_dir(), "structure folder missing");
        let readme = fs::read_to_string(root.join("README.md")).unwrap();
        assert!(readme.contains("# Hello_World"), "readme was: {readme}");
        assert!(readme.contains("id: T001"));

        // Index got exactly one record for it.
        let records = index::load_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "T001");
    });
}

#[test]
fn serves_embedded_frontend_asset() {
    with_fresh_install(
        |_install| match ui::route_request("GET", "/app.js", b"").unwrap() {
            Response::Static(content_type, bytes) => {
                assert!(content_type.contains("javascript"));
                assert!(!bytes.is_empty(), "embedded app.js should be non-empty");
            }
            Response::Json(_) => panic!("expected static asset for /app.js"),
        },
    );
}

#[test]
fn unknown_route_is_not_found() {
    with_fresh_install(|_install| {
        let err = ui::route_request("GET", "/nope", b"").unwrap_err();
        assert!(err.to_string().starts_with("not found:"), "got: {err}");
    });
}

// ---------------------------------------------------------------------------
// v0.7 — search, project detail, tags, journal, register, apply, maintenance
// ---------------------------------------------------------------------------

/// Create a project through the UI and return its root path on disk.
fn create_project(name: &str) -> String {
    let value = json(
        "POST",
        "/api/create",
        serde_json::json!({"template": "test", "variables": {"name": name}}),
    );
    value["project"]["root_path"]
        .as_str()
        .expect("root_path")
        .to_string()
}

#[test]
fn search_route_respects_query() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);

        let alpha = create_project("alpha unique");
        create_project("beta project");

        // Free-text term matches the folder/variable of exactly one project.
        let value = json(
            "POST",
            "/api/search",
            serde_json::json!({"terms": ["alpha"]}),
        );
        let hits = value["projects"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "expected one alpha match");
        assert_eq!(hits[0]["name"], "T001_Alpha_Unique");

        // Empty query returns everything.
        let all = json("POST", "/api/search", serde_json::json!({"terms": []}));
        assert_eq!(all["projects"].as_array().unwrap().len(), 2);

        // `projects` only appears in the base path — path is NOT free-text searched.
        let none = json(
            "POST",
            "/api/search",
            serde_json::json!({"terms": ["projects"]}),
        );
        assert!(
            none["projects"].as_array().unwrap().is_empty(),
            "path must be excluded from free-text search"
        );

        // Tag a project, then filter by it.
        let _ = json(
            "POST",
            "/api/project/tag",
            serde_json::json!({"path": alpha, "action": "add", "tag": "special"}),
        );
        let tagged = json(
            "POST",
            "/api/search",
            serde_json::json!({"terms": ["tag:special"]}),
        );
        let tagged_hits = tagged["projects"].as_array().unwrap();
        assert_eq!(tagged_hits.len(), 1);
        assert_eq!(tagged_hits[0]["name"], "T001_Alpha_Unique");
    });
}

#[test]
fn project_detail_returns_metadata_and_journal() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("detail demo");

        let value = json(
            "GET",
            &format!("/api/project?path={path}"),
            serde_json::Value::Null,
        );
        assert_eq!(value["ok"], true);
        assert_eq!(value["has_metadata"], true);
        assert_eq!(value["metadata"]["id"], "T001");
        assert_eq!(value["metadata"]["variables"]["name"], "Detail_Demo");
        assert!(value["journal"].as_array().unwrap().is_empty());
    });
}

#[test]
fn tag_add_then_remove_roundtrip() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("tagme");

        let added = json(
            "POST",
            "/api/project/tag",
            serde_json::json!({"path": path, "action": "add", "tag": "draft"}),
        );
        let tags = added["tags"].as_array().unwrap();
        assert!(tags.iter().any(|t| t == "draft"));

        let pinfo = fs::read_to_string(Path::new(&path).join("PROJECT_INFO.md")).unwrap();
        assert!(pinfo.contains("draft"), "frontmatter should hold the tag");

        let removed = json(
            "POST",
            "/api/project/tag",
            serde_json::json!({"path": path, "action": "remove", "tag": "draft"}),
        );
        assert!(removed["tags"].as_array().unwrap().is_empty());
    });
}

#[test]
fn note_appends_journal_entry() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("journal demo");

        let value = json(
            "POST",
            "/api/project/note",
            serde_json::json!({"path": path, "message": "first milestone"}),
        );
        let journal = value["journal"].as_array().unwrap();
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0]["message"], "first milestone");

        let pinfo = fs::read_to_string(Path::new(&path).join("PROJECT_INFO.md")).unwrap();
        assert!(pinfo.contains("## Journal"));
        assert!(pinfo.contains("first milestone"));
    });
}

#[test]
fn register_onboards_existing_folder() {
    with_fresh_install(|install| {
        write_config(install);
        let folder = install.join("legacy_project");
        fs::create_dir_all(&folder).unwrap();

        let value = json(
            "POST",
            "/api/register",
            serde_json::json!({"path": folder.display().to_string()}),
        );
        assert_eq!(value["ok"], true);
        assert_eq!(value["project"]["pinfo_written"], true);

        // Index got the record and the counter advanced.
        let records = index::load_all().unwrap();
        assert_eq!(records.len(), 1);
        assert!(folder.join("PROJECT_INFO.md").exists());
    });
}

#[test]
fn apply_preview_lists_actions_without_writing() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let target = install.join("apply_target");
        fs::create_dir_all(&target).unwrap();

        let value = json(
            "POST",
            "/api/apply/preview",
            serde_json::json!({"template": "test", "variables": {"name": "x"}, "target": target.display().to_string()}),
        );
        let actions = value["actions"].as_array().unwrap();
        assert!(actions.iter().any(|a| a["action"] == "create"));
        // Preview writes nothing.
        assert!(!target.join("src").exists());
    });
}

#[test]
fn apply_creates_missing() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let target = install.join("apply_real");
        fs::create_dir_all(&target).unwrap();

        let _ = json(
            "POST",
            "/api/apply",
            serde_json::json!({"template": "test", "variables": {"name": "x"}, "target": target.display().to_string()}),
        );
        assert!(
            target.join("src").is_dir(),
            "structure folder should be created"
        );
        assert!(target.join("README.md").exists(), "file should be created");
    });
}

#[test]
fn template_import_then_export_roundtrip() {
    with_fresh_install(|install| {
        write_config(install);
        let yaml = r#"name: Imported
slug: imported
description: via UI
naming_pattern: "{id}_{name}"
id:
  prefix: I
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: none
structure: []
files: []
"#;
        let imported = json(
            "POST",
            "/api/templates/import",
            serde_json::json!({"yaml": yaml}),
        );
        assert_eq!(imported["template"]["slug"], "imported");
        assert!(install.join("templates").join("imported.yaml").exists());

        match ui::route_request("GET", "/api/templates/export?slug=imported", b"").unwrap() {
            Response::Static(content_type, bytes) => {
                assert!(content_type.contains("yaml"));
                let text = String::from_utf8(bytes).unwrap();
                assert!(text.contains("slug: imported"));
            }
            Response::Json(_) => panic!("expected YAML download"),
        }
    });
}

#[test]
fn prune_drops_missing_records() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("doomed");
        assert_eq!(index::load_all().unwrap().len(), 1);

        fs::remove_dir_all(&path).unwrap();
        let value = json("POST", "/api/projects/prune", serde_json::json!({}));
        assert_eq!(value["removed"], 1);
        assert!(index::load_all().unwrap().is_empty());
    });
}
