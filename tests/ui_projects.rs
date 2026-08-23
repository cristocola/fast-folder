//! The browser-UI request layer: Projects: preview, create, search, detail, size, tags, notes, register, apply, and the destructive routes.
//!
//! Exercises the pure router `ui::route_request` directly — no socket —
//! against a fresh `FASTF_INSTALL_DIR` tempdir. Split out of the single
//! 1300-line `ui_server.rs`, whose 45 tests all queued behind one mutex.

#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use fastf::core::config::Config;
use fastf::ui::{self, Response};

mod common;

use common::env::with_fresh_install;
use common::fixtures::write_minimal_template;
use common::ui::{create_fixture_project, create_project, json, write_config};

/// This binary's own lock. `FASTF_INSTALL_DIR` and `HOME` are process-wide, so
/// every test in a binary shares one — and separate binaries are separate
/// processes, which is what lets these four run in parallel with each other.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

#[test]
fn health_route_responds_ok() {
    sandboxed(|_install| {
        let value = json("GET", "/api/health", serde_json::Value::Null);
        assert_eq!(value["ok"], true);
    });
}

#[test]
fn preview_produces_plan_without_writing() {
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(
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
    sandboxed(|_install| {
        let err = ui::route_request("GET", "/nope", b"").unwrap_err();
        assert!(err.to_string().starts_with("not found:"), "got: {err}");
    });
}

#[test]
fn search_route_respects_query() {
    sandboxed(|install| {
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
    sandboxed(|install| {
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
fn project_size_returns_exact_live_logical_bytes() {
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("sized");
        let root = Path::new(&path);
        fs::create_dir_all(root.join("nested/empty")).unwrap();
        fs::write(root.join("nested/data.bin"), [0_u8; 137]).unwrap();
        fs::write(root.join(".hidden"), b"hidden bytes").unwrap();

        let expected = fs::metadata(root.join("PROJECT_INFO.md")).unwrap().len()
            + fs::metadata(root.join("README.md")).unwrap().len()
            + 137
            + b"hidden bytes".len() as u64;
        let value = json(
            "GET",
            &format!("/api/project/size?path={path}"),
            serde_json::Value::Null,
        );

        assert_eq!(value["ok"], true);
        let returned_path = value["path"].as_str().expect("size response path");
        assert_eq!(
            Path::new(returned_path).canonicalize().unwrap(),
            root.canonicalize().unwrap(),
            "the response path should identify the requested project",
        );
        assert_eq!(value["size_bytes"], expected);
    });
}

#[test]
fn project_size_rejects_a_project_that_disappeared() {
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("gone");

        // Prime discovery/cache, then remove the folder externally. Discovery
        // existence-checks cached rows, so a later size request cannot be used
        // to probe a path that is no longer a project.
        let _ = json("GET", "/api/state", serde_json::Value::Null);
        fs::remove_dir_all(&path).unwrap();
        let error =
            ui::route_request("GET", &format!("/api/project/size?path={path}"), b"").unwrap_err();

        assert!(
            error.to_string().contains("no project found"),
            "got: {error}"
        );
    });
}

#[cfg(unix)]
#[test]
fn project_size_reports_unavailable_instead_of_a_partial_total() {
    use std::os::unix::fs::PermissionsExt;

    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("unreadable");
        let blocked = Path::new(&path).join("blocked");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("secret.bin"), [9_u8; 211]).unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

        let value = json(
            "GET",
            &format!("/api/project/size?path={path}"),
            serde_json::Value::Null,
        );
        assert_eq!(value["ok"], true);
        assert!(value["size_bytes"].is_null());

        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
    });
}

#[test]
fn state_neither_embeds_nor_requires_project_size_scans() {
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        let path = create_project("state stays fast");
        fs::create_dir_all(Path::new(&path).join("deep/tree")).unwrap();
        fs::write(Path::new(&path).join("deep/tree/payload.bin"), [4_u8; 4096]).unwrap();

        let state = json("GET", "/api/state", serde_json::Value::Null);
        let project = &state["projects"][0];
        assert!(project.get("size_bytes").is_none());
        assert!(project.get("size").is_none());
    });
}

#[test]
fn tag_add_then_remove_roundtrip() {
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
fn reindex_route_rescans_bases() {
    sandboxed(|install| {
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
    sandboxed(|_install| {
        let err = ui::route_request("POST", "/api/projects/prune", b"{}").unwrap_err();
        assert!(err.to_string().starts_with("not found:"), "got: {err}");
    });
}

#[test]
fn discovery_self_heals_missing_folder() {
    sandboxed(|install| {
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

#[test]
fn project_json_includes_base() {
    sandboxed(|install| {
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
    sandboxed(|install| {
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
fn unregister_keeps_files_but_hides_project() {
    sandboxed(|install| {
        let root = create_fixture_project(install);

        let value = json(
            "POST",
            "/api/project/unregister",
            serde_json::json!({"path": root.display().to_string()}),
        );
        assert_eq!(value["ok"], true);

        // Files stay; only PROJECT_INFO.md is gone; discovery forgets it.
        assert!(root.join("src").is_dir(), "project files must survive");
        assert!(root.join("README.md").is_file());
        assert!(!root.join("PROJECT_INFO.md").exists());
        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["projects"].as_array().unwrap().len(), 0);
    });
}

#[test]
fn delete_removes_folder_after_typed_confirmation() {
    sandboxed(|install| {
        let root = create_fixture_project(install);

        // Wrong confirmation name → error, nothing deleted.
        let err = ui::route_request(
            "POST",
            "/api/project/delete",
            serde_json::json!({"path": root.display().to_string(), "confirm_name": "nope"})
                .to_string()
                .as_bytes(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("confirmation"), "err was: {err}");
        assert!(root.is_dir(), "folder must survive a failed confirmation");

        // Correct confirmation → recursive delete + gone from discovery.
        let value = json(
            "POST",
            "/api/project/delete",
            serde_json::json!({
                "path": root.display().to_string(),
                "confirm_name": "T001_Hello_World"
            }),
        );
        assert_eq!(value["ok"], true);
        assert!(!root.exists(), "folder must be deleted");
        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["projects"].as_array().unwrap().len(), 0);
    });
}

#[test]
fn rename_round_trips_and_rejects_collisions() {
    sandboxed(|install| {
        let root = create_fixture_project(install);
        let base = root.parent().unwrap().to_path_buf();

        let value = json(
            "POST",
            "/api/project/rename",
            serde_json::json!({
                "path": root.display().to_string(),
                "folder": "T001_Renamed_Project"
            }),
        );
        assert_eq!(value["ok"], true);
        let new_root = base.join("T001_Renamed_Project");
        assert!(!root.exists());
        assert!(
            new_root.join("src").is_dir(),
            "files travel with the rename"
        );

        // Metadata folder/path patched; identity (id) unchanged; discoverable.
        let meta = fastf::core::project_info::read_metadata(&new_root)
            .unwrap()
            .unwrap();
        assert_eq!(meta.folder, "T001_Renamed_Project");
        assert_eq!(meta.id, "T001");
        let state = json("GET", "/api/state", serde_json::Value::Null);
        let projects = state["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["name"], "T001_Renamed_Project");

        // Renaming onto an existing folder is rejected.
        fs::create_dir_all(base.join("occupied")).unwrap();
        let err = ui::route_request(
            "POST",
            "/api/project/rename",
            serde_json::json!({
                "path": new_root.display().to_string(),
                "folder": "occupied"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already exists"), "err: {err}");
        assert!(new_root.is_dir(), "failed rename must leave project intact");
    });
}
