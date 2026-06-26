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
