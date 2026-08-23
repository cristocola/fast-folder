//! The browser-UI request layer: Path authorization, base initialization, and the status code an error gets.
//!
//! Exercises the pure router `ui::route_request` directly — no socket —
//! against a fresh `FASTF_INSTALL_DIR` tempdir. Split out of the single
//! 1300-line `ui_server.rs`, whose 45 tests all queued behind one mutex.

#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::config::Config;
use fastf::ui::{self};

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

/// `POST /api/open` handed its path straight to the system file manager after
/// nothing but an `exists()` check, so any page that could reach the loopback
/// port could make fastf open an arbitrary local folder. Every sibling route
/// resolves the path through discovery first.
#[test]
fn open_refuses_a_path_outside_the_library() {
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        create_project("real");

        let stray = install.join("stray");
        fs::create_dir_all(&stray).unwrap();
        let error = ui::route_request(
            "POST",
            "/api/open",
            serde_json::json!({"path": stray.display().to_string()})
                .to_string()
                .as_bytes(),
        )
        .unwrap_err();

        assert!(
            error.to_string().starts_with("forbidden:"),
            "an unauthorized path must be refused before anything is spawned, got: {error}"
        );
    });
}

/// `GET /api/project?path=` read `PROJECT_INFO.md` and reported existence for
/// any absolute path on the machine.
#[test]
fn project_detail_refuses_a_path_outside_the_library() {
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        create_project("real");

        let stray = install.join("stray");
        fs::create_dir_all(&stray).unwrap();
        fs::write(stray.join("PROJECT_INFO.md"), "not in a configured base").unwrap();
        let error = ui::route_request(
            "GET",
            &format!("/api/project?path={}", stray.display()),
            b"",
        )
        .unwrap_err();

        assert!(error.to_string().starts_with("forbidden:"), "got: {error}");
    });
}

#[test]
fn project_size_rejects_paths_outside_the_discovered_library() {
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);
        create_project("real");

        let stray = install.join("stray");
        fs::create_dir_all(&stray).unwrap();
        fs::write(stray.join("PROJECT_INFO.md"), "not in a configured base").unwrap();
        let error = ui::route_request(
            "GET",
            &format!("/api/project/size?path={}", stray.display()),
            b"",
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("no project found"),
            "got: {error}"
        );
    });
}

#[test]
fn delete_refuses_paths_that_are_not_projects() {
    sandboxed(|install| {
        create_fixture_project(install);
        // A random dir outside any base is never discovered → clean error.
        let stray = install.join("stray");
        fs::create_dir_all(&stray).unwrap();
        let err = ui::route_request(
            "POST",
            "/api/project/delete",
            serde_json::json!({"path": stray.display().to_string(), "confirm_name": "stray"})
                .to_string()
                .as_bytes(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no project found"), "err: {err}");
        assert!(stray.is_dir());
    });
}

#[test]
fn destructive_route_rechecks_cached_project_identity_before_deleting() {
    sandboxed(|install| {
        let root = create_fixture_project(install);
        fs::write(root.join("sentinel.bin"), b"must survive").unwrap();

        // Prime the per-base cache with T001, then change the authoritative
        // metadata without touching the base directory mtime. Discovery may
        // still return the cached record; the destructive operation must not
        // trust it.
        let _ = json("GET", "/api/state", serde_json::Value::Null);
        fastf::core::project_info::write_frontmatter(&root.join("PROJECT_INFO.md"), |metadata| {
            metadata.id = "T999".to_string()
        })
        .unwrap();
        // Keep discovery on the deliberately stale cache entry. Windows may
        // timestamp the parent directory after the atomic cache rename, which
        // otherwise makes the cache look stale and causes an authoritative
        // rescan before this test reaches the destructive-operation guard.
        fastf::core::library::touch_cache(root.parent().unwrap());

        let error = ui::route_request(
            "POST",
            "/api/project/delete",
            serde_json::json!({
                "path": root.display().to_string(),
                "confirm_name": "T001_Hello_World"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("identity changed"),
            "got: {error}"
        );
        assert_eq!(
            fs::read(root.join("sentinel.bin")).unwrap(),
            b"must survive"
        );
    });
}

#[test]
fn base_init_onboards_first_run() {
    sandboxed(|install| {
        // Fresh install: nothing configured, a conventional folder suggested.
        // (The harness redirects home into the sandbox, so the `~` shorthand
        // and the unconfigured-base fallback both resolve to `install`.)
        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["base_configured"], false);
        let suggested = state["suggested_base"].as_str().unwrap();
        assert!(suggested.ends_with("Projects"), "suggested: {suggested}");

        // Init with the ~ shorthand: folder created, config points at it.
        let value = json(
            "POST",
            "/api/base/init",
            serde_json::json!({"path": "~/My Projects"}),
        );
        assert_eq!(value["ok"], true);
        let created = install.join("My Projects");
        assert!(created.is_dir(), "base folder should be created");
        let cfg = Config::load().unwrap();
        assert_eq!(
            cfg.base_dir,
            created.canonicalize().unwrap().display().to_string()
        );
        let state = json("GET", "/api/state", serde_json::Value::Null);
        assert_eq!(state["base_configured"], true);

        // Relative paths are rejected — the base must never depend on the
        // server's working directory.
        let err =
            ui::route_request("POST", "/api/base/init", br#"{"path":"relative/dir"}"#).unwrap_err();
        assert!(err.to_string().contains("absolute"), "err: {err}");
    });
}

#[test]
fn ui_base_overrides_use_the_shared_absolute_path_resolver() {
    sandboxed(|install| {
        write_minimal_template(install, "test");
        write_config(install);

        for route in ["/api/preview", "/api/create"] {
            let error = ui::route_request(
                "POST",
                route,
                serde_json::json!({
                    "template": "test",
                    "variables": {"name": "contained"},
                    "base_dir": "relative/projects"
                })
                .to_string()
                .as_bytes(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("absolute"), "got: {error}");
        }

        let value = json(
            "POST",
            "/api/preview",
            serde_json::json!({
                "template": "test",
                "variables": {"name": "contained"},
                "base_dir": "~/override"
            }),
        );
        let actual_base = Path::new(value["root_path"].as_str().unwrap())
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        let expected_base = install.join("override").canonicalize().unwrap();
        assert_eq!(actual_base, expected_base);

        let error = ui::route_request("POST", "/api/settings", br#"{"base_dir":"also/relative"}"#)
            .unwrap_err();
        assert!(error.to_string().contains("absolute"), "got: {error}");
    });
}

#[test]
fn pick_path_rejects_unknown_kind() {
    sandboxed(|_install| {
        // Validation happens before any dialog could spawn, so this is safe
        // headless. The happy path needs a desktop and stays a manual test.
        let err = ui::route_request("POST", "/api/pick-path", br#"{"kind":"bogus"}"#).unwrap_err();
        assert!(
            err.to_string().contains("unknown picker kind"),
            "err: {err}"
        );
    });
}

/// A `config.toml` that cannot be parsed is the server's problem, not the
/// browser's: it must answer 5xx with the file named, never fall back to
/// defaults and describe a different library. `status_for` keys the status off
/// the message prefix, so that prefix is what this asserts (the same way the
/// job routes pin `not found:` to 404).
#[test]
fn an_unreadable_config_is_a_server_error() {
    sandboxed(|install| {
        write_config(install);
        let path = install.join("config.toml");
        let mut raw = fs::read_to_string(&path).unwrap();
        raw.push_str("\nthis is = not [valid toml\n");
        fs::write(&path, raw).unwrap();

        let err = ui::route_request("GET", "/api/state", b"").unwrap_err();
        assert!(
            err.to_string().starts_with("server error:"),
            "an unreadable config must map to 500, got: {err}"
        );
        assert!(
            format!("{err:#}").contains("config.toml"),
            "the client must be told which file to fix, got: {err:#}"
        );
    });
}
