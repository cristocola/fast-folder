//! The browser-UI request layer: Templates: the bundled `files/` subtree, editing it, and generating a template from a folder.
//!
//! Exercises the pure router `ui::route_request` directly — no socket —
//! against a fresh `FASTF_INSTALL_DIR` tempdir. Split out of the single
//! 1300-line `ui_server.rs`, whose 45 tests all queued behind one mutex.

#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::Path;
use std::sync::Mutex;

mod common;

use common::env::with_fresh_install;
use common::fixtures::write_minimal_template;
use common::ui::{err, json, write_config};

/// This binary's own lock. `FASTF_INSTALL_DIR` and `HOME` are process-wide, so
/// every test in a binary shares one — and separate binaries are separate
/// processes, which is what lets these four run in parallel with each other.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

#[test]
fn create_small_assets_has_no_job() {
    // Only small/text files → nothing is deferred → no background job.
    sandboxed(|install| {
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
fn create_reproduces_bundled_files_via_ui() {
    // v0.8: the UI create path walks the template's files/ subtree. A binary
    // asset must land byte-identical; a text file must be interpolated.
    sandboxed(|install| {
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

#[test]
fn template_files_list_includes_bundled_readme() {
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
