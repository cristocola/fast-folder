//! The browser-UI request layer, driven through its pure router.
//!
//! Shared by the four `ui_*` suites, which were one 1300-line binary whose tests
//! all queued behind a single mutex.

use std::fs;
use std::path::Path;

use fastf::core::config::Config;
use fastf::ui::{self, Response};

/// Write a `config.toml` whose `base_dir` points inside the sandbox, so
/// `Config::load()` resolves projects into the tempdir. Returns the base.
pub fn write_config(install: &Path) -> String {
    let mut cfg = Config::default();
    let base = install.join("projects");
    fs::create_dir_all(&base).unwrap();
    cfg.base_dir = base.display().to_string();
    cfg.save().unwrap();
    cfg.base_dir
}

/// One JSON request/response through the router — no socket.
pub fn json(method: &str, route: &str, body: serde_json::Value) -> serde_json::Value {
    match ui::route_request(method, route, body.to_string().as_bytes()).unwrap() {
        Response::Json(value) => value,
        Response::Static(..) => panic!("expected JSON response for {method} {route}"),
    }
}

/// The error text a route returns, for the cases where refusal is the point.
pub fn err(method: &str, route: &str, body: serde_json::Value) -> String {
    ui::route_request(method, route, body.to_string().as_bytes())
        .unwrap_err()
        .to_string()
}

/// Create a project through the UI and return its root path.
///
/// Assumes a `test` template and a written config are already in place.
pub fn create_project(name: &str) -> String {
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

/// Template, config, and one project — the starting point for the routes that
/// act on an existing project.
pub fn create_fixture_project(install: &Path) -> std::path::PathBuf {
    super::fixtures::write_minimal_template(install, "test");
    let base = write_config(install);
    let value = json(
        "POST",
        "/api/create",
        serde_json::json!({"template": "test", "variables": {"name": "hello world"}}),
    );
    assert_eq!(value["ok"], true);
    Path::new(&base).join("T001_Hello_World")
}
