//! Crash-recovery invariants, driven by fault injection.
//!
//! Every boundary that must survive an interruption carries a named failpoint
//! (see `util::faults`). These tests trip each one in turn and assert the same
//! invariants hold regardless of *where* the failure landed. That is the part
//! that matters: a hand-written test per boundary tests the boundaries someone
//! thought of, whereas iterating the list catches the one that gets added later
//! without a test.
//!
//! Two modes are exercised, because they prove different things:
//!
//! - **error** — in-process, the failpoint returns `Err`, so unwinding and
//!   rollback run. This is an interrupt or a full disk.
//! - **abort** — a real subprocess killed with `process::abort()`: no
//!   unwinding, no destructors, nothing cleaned up. This is a power cut, and it
//!   is the only honest way to test that *recovery* works rather than that
//!   *cleanup* works.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, library, project, provisioning, template};
use fastf::util::faults::FAULT_ENV;

/// `FASTF_INSTALL_DIR` and `FASTF_FAULT` are process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

struct Sandbox {
    _tmp: tempfile::TempDir,
    install: std::path::PathBuf,
    base: std::path::PathBuf,
}

/// Fresh install dir + base, with HOME redirected so an unconfigured base can
/// never reach the developer's real home directory.
fn sandbox<R>(body: impl FnOnce(&Sandbox) -> R) -> R {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = tmp.path().join("install");
    let base = tmp.path().join("base");
    fs::create_dir_all(install.join("templates")).unwrap();
    fs::create_dir_all(&base).unwrap();

    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let old_home = std::env::var_os(home_var);
    // SAFETY: SERIAL keeps other tests in this binary off these variables.
    unsafe {
        std::env::set_var("FASTF_INSTALL_DIR", &install);
        std::env::set_var(home_var, tmp.path());
    }

    let sb = Sandbox {
        _tmp: tmp,
        install,
        base,
    };
    let out = body(&sb);

    unsafe {
        std::env::remove_var("FASTF_INSTALL_DIR");
        std::env::remove_var(FAULT_ENV);
        match old_home {
            Some(v) => std::env::set_var(home_var, v),
            None => std::env::remove_var(home_var),
        }
    }
    out
}

/// A template with a handful of files, so `create:mid-copy` has something to
/// land in the middle of.
fn write_template(install: &Path, slug: &str) {
    let dir = install.join("templates").join(slug);
    fs::create_dir_all(dir.join("files/assets")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        format!(
            "name: Crash Test\nslug: {slug}\nnaming_pattern: \"{{id}}_proj\"\n\
             id:\n  prefix: C\n  digits: 3\nstructure:\n  - name: out\n"
        ),
    )
    .unwrap();
    for i in 0..4 {
        fs::write(
            dir.join(format!("files/assets/f{i}.txt")),
            format!("body {i}"),
        )
        .unwrap();
    }
}

fn config_for(base: &Path) -> Config {
    let mut cfg = Config::default();
    cfg.base_dir = base.display().to_string();
    cfg
}

fn arm(point: &str) {
    // SAFETY: the sandbox holds SERIAL for the duration of the test.
    unsafe { std::env::set_var(FAULT_ENV, point) };
}

fn disarm() {
    unsafe { std::env::remove_var(FAULT_ENV) };
}

/// Failpoints reachable from a plain `fastf new`.
const CREATE_POINTS: &[&str] = &[
    "create:after-root-dir",
    "create:after-pinfo",
    "create:mid-copy",
    "create:before-counter-save",
];

/// Whatever the failure point, an interrupted create must leave the base
/// exactly as it found it: no partial project, no scratch, no burned ID.
/// Debug-only: failpoints are compiled out of release builds, so these have
/// nothing to trip there.
#[cfg(debug_assertions)]
#[test]
fn interrupted_create_leaves_nothing_behind_at_every_failpoint() {
    for point in CREATE_POINTS {
        sandbox(|sb| {
            write_template(&sb.install, "crash");
            let cfg = config_for(&sb.base);
            let tmpl = template::find_by_slug("crash").unwrap();
            let mut counters = Counters::load().unwrap();
            let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();

            arm(point);
            let result = project::create(&plan, &tmpl, &mut counters, &cfg, false);
            disarm();

            assert!(result.is_err(), "[{point}] create should have failed");
            assert!(
                !plan.root_path.exists(),
                "[{point}] partial project left at {}",
                plan.root_path.display()
            );
            assert_eq!(
                Counters::load().unwrap().get(),
                0,
                "[{point}] a rolled-back create must not burn an ID"
            );
            assert!(
                library::discover(&cfg).is_empty(),
                "[{point}] a failed create must not be discoverable"
            );
            // Nothing but the base cache may remain.
            let leftovers: Vec<String> = fs::read_dir(&sb.base)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n != library::CACHE_FILENAME)
                .collect();
            assert!(leftovers.is_empty(), "[{point}] leftovers: {leftovers:?}");
        });
    }
}

/// After a clean create, reconcile must find nothing — otherwise every healthy
/// project would look broken and the signal would be worthless.
#[test]
fn successful_create_is_reported_clean_by_reconcile() {
    sandbox(|sb| {
        write_template(&sb.install, "crash");
        let cfg = config_for(&sb.base);
        let tmpl = template::find_by_slug("crash").unwrap();
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let report = provisioning::reconcile(&cfg);
        assert!(
            report.is_empty(),
            "clean project reported as needing work: {report:?}"
        );
        assert!(provisioning::list_incomplete(&cfg).is_empty());
        assert_eq!(library::discover(&cfg).len(), 1);
    });
}

// ---------------------------------------------------------------------------
// Hard-kill (abort) — a real subprocess, no unwinding
// ---------------------------------------------------------------------------

/// Run the built `fastf` binary with a failpoint armed in abort mode.
#[cfg(debug_assertions)]
fn run_fastf_aborting(
    install: &Path,
    home: &Path,
    point: &str,
    args: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fastf"))
        .args(args)
        .env("FASTF_INSTALL_DIR", install)
        .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
        .env(FAULT_ENV, format!("{point}:abort"))
        .output()
        .expect("running fastf")
}

/// The scenario that motivated all of this: a create killed outright, mid-copy,
/// with no chance to clean up.
///
/// Before, this stranded a folder with no metadata — invisible to `recent`,
/// `search` and `reindex`, while `reconcile` reported "all projects fully
/// provisioned". The folder must now be *visible* and *honestly reported*.
/// Debug-only: failpoints are compiled out of release builds, so these have
/// nothing to trip there.
#[cfg(debug_assertions)]
#[test]
fn hard_killed_create_is_visible_and_reported() {
    sandbox(|sb| {
        write_template(&sb.install, "crash");
        let home = sb.install.parent().unwrap();

        // Point the real binary at our sandbox base.
        let out = Command::new(env!("CARGO_BIN_EXE_fastf"))
            .args(["config", "set", "base-dir", &sb.base.display().to_string()])
            .env("FASTF_INSTALL_DIR", &sb.install)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
            .output()
            .expect("configuring base-dir");
        assert!(out.status.success(), "config set failed: {out:?}");

        let killed = run_fastf_aborting(
            &sb.install,
            home,
            "create:mid-copy",
            &["new", "crash", "--yes", "--no-preview"],
        );
        assert!(
            !killed.status.success(),
            "the process was supposed to abort, got {:?}",
            killed.status
        );

        // A folder survived the kill, and it carries metadata — so it is a
        // project fastf can see, not an invisible orphan.
        let cfg = config_for(&sb.base);
        let found = library::discover(&cfg);
        assert_eq!(
            found.len(),
            1,
            "the killed create should still be discoverable"
        );
        assert!(
            fastf::core::project_info::is_provisioning(&found[0].path),
            "it must be flagged as never finished"
        );

        // And reconcile says so, rather than claiming everything is fine.
        let report = provisioning::reconcile(&cfg);
        assert!(
            !report.is_empty(),
            "reconcile must not report a clean library here"
        );
        assert_eq!(
            report.incomplete.len(),
            1,
            "expected exactly one incomplete project, got {report:?}"
        );
        assert!(!provisioning::list_incomplete(&cfg).is_empty());
    });
}

/// Killed before the metadata is written, the folder is empty and nameless — but
/// it must still never be mistaken for a finished project.
/// Debug-only: failpoints are compiled out of release builds, so these have
/// nothing to trip there.
#[cfg(debug_assertions)]
#[test]
fn hard_kill_before_metadata_does_not_produce_a_phantom_project() {
    sandbox(|sb| {
        write_template(&sb.install, "crash");
        let home = sb.install.parent().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_fastf"))
            .args(["config", "set", "base-dir", &sb.base.display().to_string()])
            .env("FASTF_INSTALL_DIR", &sb.install)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
            .output()
            .expect("configuring base-dir");
        assert!(out.status.success());

        run_fastf_aborting(
            &sb.install,
            home,
            "create:after-root-dir",
            &["new", "crash", "--yes", "--no-preview"],
        );

        let cfg = config_for(&sb.base);
        // No metadata → not a project. An empty leftover directory is untidy,
        // never wrong: discovery is defined by PROJECT_INFO.md.
        assert!(
            library::discover(&cfg).is_empty(),
            "a metadata-less folder must not surface as a project"
        );
        assert_eq!(
            Counters::load().unwrap().get(),
            0,
            "the counter must not have advanced"
        );
    });
}
