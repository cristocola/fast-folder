//! The browser-UI request layer: Background copy jobs, and what recovery reports about interrupted ones.
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
use fastf::ui::{self};

mod common;

use common::env::with_fresh_install;
use common::fixtures::write_minimal_template;
use common::ui::{json, write_config};

/// This binary's own lock. `FASTF_INSTALL_DIR` and `HOME` are process-wide, so
/// every test in a binary shares one — and separate binaries are separate
/// processes, which is what lets these four run in parallel with each other.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

#[test]
fn create_large_asset_returns_job_and_completes() {
    // A bundled file over the defer threshold is copied in the background: the
    // create returns a job_id immediately (project already usable), and polling
    // /api/job/<id> reaches "done" with the file copied byte-for-byte.
    sandboxed(|install| {
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

/// The frontend distinguishes "this job finished and was evicted" from "we
/// could not reach the server" purely by HTTP status: only a 404 counts as
/// finished, everything else is reported as unknown. `handle_connection` derives
/// 404 from the `not found:` message prefix, so a missing job MUST keep it.
///
/// This replaced a data-safety bug: `app.js` used to treat *any* poll failure as
/// success, so one dropped request during a slow copy reported a still-running
/// move as "Moved — verified", and a user deletes a source on that word.
#[test]
fn a_missing_job_is_reported_as_not_found() {
    sandboxed(|_install| {
        let err = ui::route_request("GET", "/api/job/job-does-not-exist", b"").unwrap_err();
        assert!(
            err.to_string().starts_with("not found:"),
            "a missing job must map to 404, got: {err}"
        );

        let err =
            ui::route_request("POST", "/api/job/job-does-not-exist/cancel", b"{}").unwrap_err();
        assert!(
            err.to_string().starts_with("not found:"),
            "cancelling a missing job must map to 404, got: {err}"
        );
    });
}

#[test]
fn reconcile_route_reports_pre_v2_copy_without_following_it() {
    sandboxed(|install| {
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
        // Planted as bytes: fastf no longer writes this format, and the point of
        // the test is that reconcile reports it without reading the absolute
        // paths inside it.
        fs::write(
            root.join(".fastf-provisioning.json"),
            format!(
                r#"{{"version":1,"started_at":"2026-01-01T00:00:00Z","jobs":[{{"src":{src},"dest":{dest},"bytes":{bytes},"done":false}}]}}"#,
                src = serde_json::to_string(&src.display().to_string()).unwrap(),
                dest = serde_json::to_string(&dest.display().to_string()).unwrap(),
                bytes = data.len(),
            ),
        )
        .unwrap();

        // /api/state surfaces the incomplete provisioning for the banner.
        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["provisioning"].as_array().unwrap().len(), 1);

        // /api/reconcile reports the obsolete marker but never parses its
        // absolute paths or copies through it.
        let value = json("POST", "/api/reconcile", serde_json::json!({}));
        assert_eq!(value["ok"], true);
        assert_eq!(value["report"]["resumed"], 0);
        assert_eq!(value["report"]["obsolete"].as_array().unwrap().len(), 1);
        assert!(!dest.exists());

        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["provisioning"].as_array().unwrap().len(), 1);
    });
}
