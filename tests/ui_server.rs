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
use std::time::{Duration, Instant};

use fastf::core::config::Config;
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
"#
    );
    // v0.8 folder form: manifest + a files/ subtree holding the bundled README.
    let dir = install.join("templates").join(slug);
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(dir.join("template.yaml"), yaml).unwrap();
    fs::write(dir.join("files").join("README.md"), "# {name}\nid: {id}\n").unwrap();
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
        // Preview writes nothing at all — no cache either.
        assert!(!install.join("projects.jsonl").exists());
    });
}

#[test]
fn create_makes_folder_and_is_discovered() {
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

        // Filesystem-as-truth: no projects.jsonl is written; the project is
        // discovered from its PROJECT_INFO.md and surfaced via /api/state.
        assert!(
            !install.join("projects.jsonl").exists(),
            "create must not write projects.jsonl"
        );
        let state = json("GET", "/api/state", serde_json::Value::Null);
        let projects = state["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"], "T001");
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
        let base = write_config(install);
        // Place the folder under the base so discovery surfaces it afterwards.
        let folder = Path::new(&base).join("legacy_project");
        fs::create_dir_all(&folder).unwrap();

        let value = json(
            "POST",
            "/api/register",
            serde_json::json!({"path": folder.display().to_string()}),
        );
        assert_eq!(value["ok"], true);
        assert_eq!(value["project"]["pinfo_written"], true);

        // The folder is now a project: PROJECT_INFO.md was written and it shows
        // up in discovery via /api/state.
        assert!(folder.join("PROJECT_INFO.md").exists());
        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["projects"].as_array().unwrap().len(), 1);
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
fn create_small_assets_has_no_job() {
    // Only small/text files → nothing is deferred → no background job.
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let value = json(
            "POST",
            "/api/create",
            serde_json::json!({"template": "test", "variables": {"name": "small"}}),
        );
        assert!(value["job_id"].is_null(), "unexpected job: {value}");
    });
}

#[test]
fn create_large_asset_returns_job_and_completes() {
    // A bundled file over the defer threshold is copied in the background: the
    // create returns a job_id immediately (project already usable), and polling
    // /api/job/<id> reaches "done" with the file copied byte-for-byte.
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        let files_dir = install.join("templates").join("test").join("files");
        // 5 MiB > JOB_DEFER_BYTES (4 MiB).
        let big = vec![0xABu8; 5 * 1024 * 1024];
        fs::write(files_dir.join("delivery.bin"), &big).unwrap();
        write_config(install);

        let value = json(
            "POST",
            "/api/create",
            serde_json::json!({"template": "test", "variables": {"name": "heavy"}}),
        );
        let job_id = value["job_id"]
            .as_str()
            .expect("expected a job_id")
            .to_string();
        let root = Path::new(value["project"]["root_path"].as_str().unwrap()).to_path_buf();
        // Project is immediately usable (structure exists) even mid-copy.
        assert!(root.join("src").is_dir());

        // Poll to completion.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut status = String::new();
        while Instant::now() < deadline {
            let job = json(
                "GET",
                &format!("/api/job/{job_id}"),
                serde_json::Value::Null,
            );
            status = job["job"]["status"].as_str().unwrap_or("").to_string();
            assert_ne!(status, "failed", "copy job failed: {job}");
            if status == "done" {
                assert_eq!(
                    job["job"]["total_bytes"].as_u64().unwrap(),
                    big.len() as u64
                );
                assert_eq!(job["job"]["done_files"].as_u64().unwrap(), 1);
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        assert_eq!(status, "done", "job did not complete in time");
        assert_eq!(fs::read(root.join("delivery.bin")).unwrap(), big);
        assert!(!root.join("delivery.bin.part").exists());
    });
}

#[test]
fn create_reproduces_bundled_files_via_ui() {
    // v0.8: the UI create path walks the template's files/ subtree. A binary
    // asset must land byte-identical; a text file must be interpolated.
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        let files_dir = install.join("templates").join("test").join("files");
        let blob: [u8; 4] = [0x00, 0xFF, 0x10, 0x80];
        fs::write(files_dir.join("logo.bin"), blob).unwrap();
        write_config(install);

        let created = json(
            "POST",
            "/api/create",
            serde_json::json!({"template": "test", "variables": {"name": "bundled"}}),
        );
        let root = Path::new(created["project"]["root_path"].as_str().unwrap());
        assert_eq!(fs::read(root.join("logo.bin")).unwrap(), blob);
        let readme = fs::read_to_string(root.join("README.md")).unwrap();
        assert!(readme.contains("# Bundled"), "readme was: {readme}");
    });
}

// ---------------------------------------------------------------------------
// v0.8 phase 3 — template file ingestion / editor
// ---------------------------------------------------------------------------

/// Send a request expected to fail and return the error string.
fn err(method: &str, route: &str, body: serde_json::Value) -> String {
    ui::route_request(method, route, body.to_string().as_bytes())
        .unwrap_err()
        .to_string()
}

#[test]
fn template_files_list_includes_bundled_readme() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let value = json(
            "GET",
            "/api/template-files?slug=test",
            serde_json::Value::Null,
        );
        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["path"], "README.md");
        assert_eq!(files[0]["is_text"], true);
        assert!(files[0]["content"].as_str().unwrap().contains("{name}"));
    });
}

#[test]
fn state_reports_template_file_count() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let value = json("GET", "/api/state", serde_json::Value::Null);
        let tmpl = value["templates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["slug"] == "test")
            .unwrap();
        assert_eq!(tmpl["file_count"], 1);
    });
}

#[test]
fn template_file_save_creates_and_updates() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let on_disk = install.join("templates/test/files/docs/NOTES.md");

        let _ = json(
            "POST",
            "/api/templates/file-save",
            serde_json::json!({"slug": "test", "path": "docs/NOTES.md", "content": "hello {name}"}),
        );
        assert_eq!(fs::read_to_string(&on_disk).unwrap(), "hello {name}");

        // The list now reports both files.
        let listed = json(
            "GET",
            "/api/template-files?slug=test",
            serde_json::Value::Null,
        );
        assert_eq!(listed["files"].as_array().unwrap().len(), 2);

        // Saving again overwrites in place.
        let _ = json(
            "POST",
            "/api/templates/file-save",
            serde_json::json!({"slug": "test", "path": "docs/NOTES.md", "content": "changed"}),
        );
        assert_eq!(fs::read_to_string(&on_disk).unwrap(), "changed");
    });
}

#[test]
fn template_file_add_from_path_copies_bytes() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let src = install.join("logo_source.bin");
        let blob = vec![0x00u8, 0xFF, 0x7F, 0x80, 0x01];
        fs::write(&src, &blob).unwrap();

        let value = json(
            "POST",
            "/api/templates/file-add",
            serde_json::json!({"slug": "test", "src": src.display().to_string(), "dest": "assets/logo.bin"}),
        );
        assert_eq!(value["is_text"], false);
        let landed = install.join("templates/test/files/assets/logo.bin");
        assert_eq!(fs::read(&landed).unwrap(), blob);
        assert!(!landed.with_extension("bin.part").exists());
    });
}

#[test]
fn template_file_delete_removes_it() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let _ = json(
            "POST",
            "/api/templates/file-delete",
            serde_json::json!({"slug": "test", "path": "README.md"}),
        );
        assert!(!install.join("templates/test/files/README.md").exists());
        let listed = json(
            "GET",
            "/api/template-files?slug=test",
            serde_json::Value::Null,
        );
        assert!(listed["files"].as_array().unwrap().is_empty());
    });
}

#[test]
fn template_file_rejects_reserved_traversal_and_missing() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);

        // The reserved auto-gen filename is refused.
        let reserved = err(
            "POST",
            "/api/templates/file-save",
            serde_json::json!({"slug": "test", "path": "PROJECT_INFO.md", "content": "x"}),
        );
        assert!(
            reserved.contains("generated automatically"),
            "got: {reserved}"
        );

        // Path traversal is rejected and writes nothing outside files/.
        let _ = err(
            "POST",
            "/api/templates/file-save",
            serde_json::json!({"slug": "test", "path": "../pwned.txt", "content": "x"}),
        );
        assert!(!install.join("templates/test/pwned.txt").exists());

        // Adding to a template that isn't saved yet fails clearly (the
        // existence check runs before the source is even inspected).
        let missing = err(
            "POST",
            "/api/templates/file-add",
            serde_json::json!({"slug": "ghost", "src": "/nonexistent/src", "dest": "x"}),
        );
        assert!(missing.contains("does not exist"), "got: {missing}");
    });
}

#[test]
fn from_folder_bundles_assets_and_reports_counts() {
    with_fresh_install(|install| {
        write_config(install);
        let src = install.join("kit");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("brief.md"), "# Brief").unwrap();
        let blob = vec![0x00u8, 0xFF, 0x10, 0x80];
        fs::write(src.join("logo.bin"), &blob).unwrap();

        let value = json(
            "POST",
            "/api/templates/from-folder",
            serde_json::json!({"source": src.display().to_string(), "slug": "kit", "bundle_assets": true}),
        );
        assert_eq!(value["ok"], true);
        assert_eq!(value["report"]["text_files"], 1);
        assert_eq!(value["report"]["bundled"], 1);
        assert_eq!(value["report"]["bundled_bytes"], blob.len() as u64);

        // The binary landed byte-for-byte in the new template's files/.
        let landed = install.join("templates/kit/files/logo.bin");
        assert_eq!(fs::read(&landed).unwrap(), blob);
    });
}

#[test]
fn reindex_route_rescans_bases() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        create_project("one");
        create_project("two");

        let value = json("POST", "/api/reindex", serde_json::json!({}));
        assert_eq!(value["ok"], true);
        assert_eq!(value["projects"], 2);
    });
}

#[test]
fn prune_route_is_gone() {
    with_fresh_install(|_install| {
        let err = ui::route_request("POST", "/api/projects/prune", b"{}").unwrap_err();
        assert!(err.to_string().starts_with("not found:"), "got: {err}");
    });
}

#[test]
fn discovery_self_heals_missing_folder() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("doomed");

        // State lists the project, discovered from its PROJECT_INFO.md.
        let before = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(before["projects"].as_array().unwrap().len(), 1);

        // Delete the folder — no manual prune. The next discovery drops it.
        fs::remove_dir_all(&path).unwrap();
        let after = json("GET", "/api/state", serde_json::Value::Null);
        assert!(after["projects"].as_array().unwrap().is_empty());
    });
}

// ---------------------------------------------------------------------------
// v0.10: base display + move between bases
// ---------------------------------------------------------------------------

#[test]
fn project_json_includes_base() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        create_project("based");

        let state = json("GET", "/api/state", serde_json::Value::Null);
        let project = &state["projects"][0];
        assert_eq!(project["base_label"], "projects");
        let base = project["base"].as_str().unwrap();
        assert!(
            base.ends_with("projects"),
            "base should be the configured base dir, got: {base}"
        );
    });
}

#[test]
fn move_route_moves_between_configured_bases() {
    with_fresh_install(|install| {
        write_minimal_template(install, "test");

        let base_a = install.join("projects");
        let base_b = install.join("projects_b");
        fs::create_dir_all(&base_a).unwrap();
        fs::create_dir_all(&base_b).unwrap();
        let mut cfg = Config::default();
        cfg.base_dir = base_a.display().to_string();
        cfg.bases = vec![base_b.display().to_string()];
        cfg.save().unwrap();

        let value = json(
            "POST",
            "/api/create",
            serde_json::json!({"template": "test", "variables": {"name": "mover"}}),
        );
        assert_eq!(value["ok"], true);
        let state = json("GET", "/api/state", serde_json::Value::Null);
        let path = state["projects"][0]["path"].as_str().unwrap().to_string();

        // A non-configured base is rejected — targets are effective_bases only.
        let err = ui::route_request(
            "POST",
            "/api/project/move",
            serde_json::json!({
                "path": path,
                "base": install.join("elsewhere").display().to_string(),
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not a configured base"),
            "got: {err}"
        );

        // Moving into the second configured base returns a background job id.
        let value = json(
            "POST",
            "/api/project/move",
            serde_json::json!({"path": path, "base": base_b.display().to_string()}),
        );
        assert_eq!(value["ok"], true);
        let job_id = value["job_id"]
            .as_str()
            .expect("expected a job_id")
            .to_string();

        // Poll the move job to completion (copy → verify → finalize → done).
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut status = String::new();
        while Instant::now() < deadline {
            let job = json(
                "GET",
                &format!("/api/job/{job_id}"),
                serde_json::Value::Null,
            );
            status = job["job"]["status"].as_str().unwrap_or("").to_string();
            assert_ne!(status, "failed", "move job failed: {job}");
            if status == "done" {
                assert_eq!(job["job"]["phase"], "done");
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        assert_eq!(status, "done", "move job did not complete in time");
        assert!(!Path::new(&path).exists(), "source folder should be gone");

        // State now shows the project under the new base, with its files intact.
        let state = json("GET", "/api/state", serde_json::Value::Null);
        let projects = state["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["base_label"], "projects_b");
        let shown = projects[0]["path"].as_str().unwrap();
        assert!(
            Path::new(shown).join("README.md").is_file(),
            "files must travel with the move"
        );
    });
}

#[test]
fn reconcile_route_resumes_pending_copy_and_state_surfaces_it() {
    with_fresh_install(|install| {
        let base = install.join("projects");
        fs::create_dir_all(&base).unwrap();
        let mut cfg = Config::default();
        cfg.base_dir = base.display().to_string();
        cfg.save().unwrap();

        // A project folder left mid-provisioning: a durable create marker plus an
        // asset that never finished copying (simulating a crash).
        let root = base.join("proj");
        fs::create_dir_all(&root).unwrap();
        let src = install.join("asset.bin");
        let data = vec![3u8; 6000];
        fs::write(&src, &data).unwrap();
        let dest = root.join("asset.bin");
        let job = fastf::core::assets::CopyJob {
            src,
            dest: dest.clone(),
            bytes: data.len() as u64,
        };
        fastf::core::provisioning::write_create_marker(&root, std::slice::from_ref(&job)).unwrap();

        // /api/state surfaces the incomplete provisioning for the banner.
        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["provisioning"].as_array().unwrap().len(), 1);

        // /api/reconcile resumes the copy and clears the marker.
        let value = json("POST", "/api/reconcile", serde_json::json!({}));
        assert_eq!(value["ok"], true);
        assert_eq!(value["report"]["resumed"], 1);
        assert_eq!(fs::read(&dest).unwrap(), data);

        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["provisioning"].as_array().unwrap().len(), 0);
    });
}
