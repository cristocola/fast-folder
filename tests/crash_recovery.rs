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
//!   unwinding, no destructors, nothing cleaned up. This models hard process
//!   termination and proves that *recovery* works rather than only testing
//!   ordinary unwind cleanup.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
#[cfg(debug_assertions)]
use std::process::Command;
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, library, project, provisioning, template};
// Every use of this is inside `#[cfg(debug_assertions)]` — failpoints are
// compiled out of release builds, so in a release test build the import itself
// is dead. The AUR source package's `check()` is a release test build, which is
// where the warning showed up.
#[cfg(debug_assertions)]
use fastf::util::faults::FAULT_ENV;

mod common;

/// This binary's lock over the process environment — see `common::env`.
static SERIAL: Mutex<()> = Mutex::new(());

use common::env::{EnvGuard, Sandbox, with_sandbox};

/// Fresh install dir + base, with HOME redirected — see `common::env`.
///
/// The guard comes through so a test can arm a failpoint with it. `FASTF_FAULT`
/// is process-wide like `FASTF_INSTALL_DIR`, and `common::env` is the one place
/// under `tests/` allowed to mutate the environment (`tests/layering.rs`
/// enforces that) — a second `set_var` behind a second lock is not isolation.
fn sandbox<R>(body: impl FnOnce(&Sandbox, &mut EnvGuard<'_>) -> R) -> R {
    with_sandbox(&SERIAL, body)
}

/// A template with a handful of files, so `create:mid-copy` has something to
/// land in the middle of.
fn write_crash_template(install: &Path, slug: &str) {
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

#[cfg(debug_assertions)]
fn arm(guard: &mut EnvGuard<'_>, point: &str) {
    guard.set(FAULT_ENV, Path::new(point));
}

#[cfg(debug_assertions)]
fn disarm(guard: &mut EnvGuard<'_>) {
    guard.remove(FAULT_ENV);
}

/// Failpoints reachable from a plain `fastf new`.
#[cfg(debug_assertions)]
const CREATE_POINTS: &[&str] = &[
    "create:after-root-dir",
    "create:after-pinfo",
    "create:mid-copy",
    "create:before-counter-save",
];

#[cfg(debug_assertions)]
const MOVE_ABORT_POINTS: &[&str] = &[
    "move:after-transaction-create",
    "move:mid-copy",
    "move:post-verification",
    "move:after-publication",
    "move:before-source-cleanup",
    "move:after-source-cleanup",
];

#[cfg(debug_assertions)]
const MOVE_CHILD_ENV: &str = "FASTF_MOVE_ABORT_CHILD";
#[cfg(debug_assertions)]
const MOVE_TARGET_ENV: &str = "FASTF_MOVE_TARGET_BASE";

/// Whatever the failure point, an interrupted create must leave the base
/// exactly as it found it: no partial project, no scratch, no burned ID.
/// Debug-only: failpoints are compiled out of release builds, so these have
/// nothing to trip there.
#[cfg(debug_assertions)]
#[test]
fn interrupted_create_leaves_nothing_behind_at_every_failpoint() {
    for point in CREATE_POINTS {
        sandbox(|sb, guard| {
            write_crash_template(&sb.install, "crash");
            let cfg = config_for(&sb.base);
            let tmpl = template::find_by_slug("crash").unwrap();
            let mut counters = Counters::load().unwrap();
            let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();

            arm(guard, point);
            let result = project::create(&plan, &tmpl, &mut counters, &cfg, false);
            disarm(guard);

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
    sandbox(|sb, _guard| {
        write_crash_template(&sb.install, "crash");
        let cfg = config_for(&sb.base);
        let tmpl = template::find_by_slug("crash").unwrap();
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let report = provisioning::reconcile_unlocked(&cfg);
        assert!(
            report.is_empty(),
            "clean project reported as needing work: {report:?}"
        );
        assert!(provisioning::list_incomplete(&cfg).is_empty());
        assert_eq!(library::discover(&cfg).len(), 1);
    });
}

#[test]
fn create_v2_recovery_resumes_scoped_copies_and_is_idempotent() {
    sandbox(|sb, _guard| {
        write_crash_template(&sb.install, "crash");
        let cfg = config_for(&sb.base);
        let tmpl = template::find_by_slug("crash").unwrap();
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let source = tmpl.files_dir().join("assets/f0.txt");
        let destination = plan.root_path.join("recovered.txt");
        let job = fastf::core::assets::CopyJob {
            bytes: fs::metadata(&source).unwrap().len(),
            src: source.clone(),
            dest: destination.clone(),
        };
        fastf::core::project_info::mark_provisioning(&plan.root_path).unwrap();
        provisioning::write_create_journal(
            &plan.root_path,
            "crash",
            &tmpl.files_dir(),
            std::slice::from_ref(&job),
        )
        .unwrap();

        let first = provisioning::reconcile_unlocked(&cfg);
        assert_eq!(first.resumed, 1, "{first:?}");
        assert_eq!(fs::read(&destination).unwrap(), fs::read(&source).unwrap());
        assert!(!fastf::core::project_info::is_provisioning(&plan.root_path));
        assert!(
            !plan
                .root_path
                .join(provisioning::CREATE_JOURNAL_V2)
                .exists()
        );

        let second = provisioning::reconcile_unlocked(&cfg);
        assert!(second.is_empty(), "recovery must be idempotent: {second:?}");
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

/// Subprocess entry point for hard-abort move tests. A normal test run executes
/// this as a no-op; the parent test selects it explicitly and supplies a
/// configured sandbox plus an armed failpoint.
#[cfg(debug_assertions)]
#[test]
fn move_abort_child_driver() {
    if std::env::var_os(MOVE_CHILD_ENV).is_none() {
        return;
    }
    let cfg = Config::load().expect("child config");
    let project = library::discover(&cfg)
        .into_iter()
        .next()
        .expect("child source project");
    let target =
        std::path::PathBuf::from(std::env::var_os(MOVE_TARGET_ENV).expect("child target base"));
    let result = library::move_project_staged_for_test(&project, &target);
    panic!("armed move returned instead of aborting: {result:?}");
}

#[cfg(debug_assertions)]
fn transaction_count(target: &Path) -> usize {
    let root = target.join(fastf::core::transactions::TRANSACTIONS_DIR);
    fs::read_dir(root)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}

/// A real process abort at every durable move boundary must reconcile to one
/// complete authoritative copy. The deliberately journal-less transaction
/// created by the earliest abort is malformed by definition, so recovery
/// reports it and leaves both it and the untouched source alone.
#[cfg(debug_assertions)]
#[test]
fn hard_killed_staged_moves_reconcile_without_data_loss() {
    for point in MOVE_ABORT_POINTS {
        sandbox(|sb, _guard| {
            write_crash_template(&sb.install, "crash");
            let target = sb.install.parent().unwrap().join("target");
            fs::create_dir_all(&target).unwrap();
            let mut cfg = config_for(&sb.base);
            cfg.bases = vec![target.display().to_string()];
            cfg.save().unwrap();

            let tmpl = template::find_by_slug("crash").unwrap();
            let mut counters = Counters::load().unwrap();
            let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();
            project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
            fs::write(plan.root_path.join("payload.tmp"), b"real tmp payload").unwrap();
            fs::write(plan.root_path.join("payload.part"), b"real part payload").unwrap();
            fs::write(plan.root_path.join("empty.bin"), []).unwrap();
            fs::write(plan.root_path.join("binary.bin"), [0_u8, 255, 17, 128]).unwrap();
            fs::create_dir(plan.root_path.join("empty-dir")).unwrap();

            let home = sb.install.parent().unwrap();
            let killed = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "move_abort_child_driver", "--nocapture"])
                .env("FASTF_INSTALL_DIR", &sb.install)
                .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
                .env(MOVE_CHILD_ENV, "1")
                .env(MOVE_TARGET_ENV, &target)
                .env(FAULT_ENV, format!("{point}:abort"))
                .output()
                .expect("running move child");
            assert!(
                !killed.status.success(),
                "[{point}] child should abort: {killed:?}"
            );

            let source = plan.root_path.clone();
            let final_path = target.join(&plan.folder_name);
            assert!(
                source.join("payload.tmp").is_file() || final_path.join("payload.tmp").is_file(),
                "[{point}] payload disappeared before reconcile"
            );

            let first = provisioning::reconcile_unlocked(&cfg);
            let source_after = source.exists();
            let final_after = final_path.exists();
            let state_after = (source_after, final_after, transaction_count(&target));
            let second = provisioning::reconcile_unlocked(&cfg);
            assert_eq!(
                state_after,
                (
                    source.exists(),
                    final_path.exists(),
                    transaction_count(&target)
                ),
                "[{point}] repeated reconcile changed settled state: {second:?}"
            );

            if *point == "move:after-transaction-create" {
                assert!(
                    source_after && !final_after,
                    "[{point}] source is authoritative"
                );
                assert_eq!(
                    state_after.2, 1,
                    "[{point}] malformed transaction is retained"
                );
                assert!(
                    !first.unrecoverable.is_empty(),
                    "[{point}] must be reported"
                );
            } else if matches!(
                *point,
                "move:after-publication"
                    | "move:before-source-cleanup"
                    | "move:after-source-cleanup"
            ) {
                assert!(
                    !source_after && final_after,
                    "[{point}] recovery must finish commit"
                );
                assert_eq!(state_after.2, 0, "[{point}] transaction should clear");
            } else {
                assert!(
                    source_after && !final_after,
                    "[{point}] recovery must roll back"
                );
                assert_eq!(state_after.2, 0, "[{point}] transaction should clear");
            }

            let authoritative = if final_after { &final_path } else { &source };
            assert_eq!(
                fs::read(authoritative.join("payload.tmp")).unwrap(),
                b"real tmp payload"
            );
            assert_eq!(
                fs::read(authoritative.join("payload.part")).unwrap(),
                b"real part payload"
            );
            assert_eq!(
                fs::read(authoritative.join("empty.bin")).unwrap(),
                Vec::<u8>::new()
            );
            assert_eq!(
                fs::read(authoritative.join("binary.bin")).unwrap(),
                [0_u8, 255, 17, 128]
            );
            assert!(authoritative.join("empty-dir").is_dir());
        });
    }
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
    sandbox(|sb, _guard| {
        write_crash_template(&sb.install, "crash");
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
        let report = provisioning::reconcile_unlocked(&cfg);
        assert!(
            !report.is_empty(),
            "reconcile must not report a clean library here"
        );
        assert_eq!(
            report.incomplete.len(),
            1,
            "expected exactly one safely scoped incomplete create, got {report:?}"
        );
        assert!(report.obsolete.is_empty(), "active creates use v2 journals");
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
    sandbox(|sb, _guard| {
        write_crash_template(&sb.install, "crash");
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

/// A failpoint that cannot fire is a boundary nobody is testing.
///
/// `reconcile`'s source-cleanup boundary called `faults::check(...).ok()`, which
/// throws the injected error away — so every other `check` in the file could be
/// trusted and this one silently could not. The failure it models is real: the
/// source is gone, the destination is published, and the transaction that records
/// the remaining bookkeeping cannot be settled yet. Recovery must keep that
/// transaction and report it, not declare the move complete.
#[cfg(debug_assertions)]
#[test]
fn a_fault_after_source_cleanup_retains_the_transaction_for_the_next_pass() {
    sandbox(|sb, guard| {
        write_crash_template(&sb.install, "crash");
        let target = sb.install.parent().unwrap().join("target");
        fs::create_dir_all(&target).unwrap();
        let mut cfg = config_for(&sb.base);
        cfg.bases = vec![target.display().to_string()];
        cfg.save().unwrap();

        let tmpl = template::find_by_slug("crash").unwrap();
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        fs::write(plan.root_path.join("payload.bin"), b"irreplaceable").unwrap();

        // Abort just before source cleanup: published destination, source still
        // present, transaction sitting in CleanupPending — the state reconcile's
        // cleanup branch exists to finish.
        let home = sb.install.parent().unwrap();
        let killed = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "move_abort_child_driver", "--nocapture"])
            .env("FASTF_INSTALL_DIR", &sb.install)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
            .env(MOVE_CHILD_ENV, "1")
            .env(MOVE_TARGET_ENV, &target)
            .env(FAULT_ENV, "move:before-source-cleanup:abort")
            .output()
            .expect("running move child");
        assert!(!killed.status.success(), "child should abort: {killed:?}");

        let source = plan.root_path.clone();
        let final_path = target.join(&plan.folder_name);
        assert!(
            source.is_dir() && final_path.is_dir(),
            "expected both halves"
        );

        arm(guard, "move:after-source-cleanup");
        let interrupted = provisioning::reconcile_unlocked(&cfg);
        disarm(guard);

        assert_eq!(
            interrupted.completed, 0,
            "an interrupted cleanup must not be reported as a completed move: {interrupted:?}"
        );
        assert!(
            !interrupted.unrecoverable.is_empty(),
            "the interruption must be reported: {interrupted:?}"
        );
        assert_eq!(
            transaction_count(&target),
            1,
            "the transaction must be retained for the next pass"
        );
        assert!(
            !source.exists(),
            "source removal itself had already succeeded"
        );

        // The next pass finds the source gone and settles the bookkeeping.
        let settled = provisioning::reconcile_unlocked(&cfg);
        assert_eq!(settled.completed, 1, "{settled:?}");
        assert_eq!(transaction_count(&target), 0, "transaction must clear");
        assert_eq!(
            fs::read(final_path.join("payload.bin")).unwrap(),
            b"irreplaceable"
        );
        assert_eq!(library::discover(&cfg).len(), 1);
    });
}

/// A template save killed outright must leave a manifest that still loads.
///
/// `template.yaml` is what every create reads, so a truncated one takes the
/// template out of service entirely — the same class of damage the bare
/// `fs::write` on `config.toml` and `counters.toml` caused before `util::atomic`
/// existed. The manifest write now goes through the same atomic writer, and the
/// scratch file it uses is a uniquely named sibling that no loader ever reads.
///
/// **Design guard, not a regression test** (see `tests/CLAUDE.md`): it passes
/// against the pre-fix build too, because a failpoint can only be placed
/// *around* `fs::write`, never inside the window where it had truncated the file
/// and not yet written the bytes. What is genuinely pinned here is that a hard
/// kill at this boundary leaves a loadable template and no scaffolding behind.
#[cfg(debug_assertions)]
#[test]
fn a_hard_killed_template_save_leaves_a_loadable_manifest() {
    sandbox(|sb, _guard| {
        write_crash_template(&sb.install, "crash");
        let manifest = sb.install.join("templates/crash/template.yaml");
        let before = fs::read_to_string(&manifest).unwrap();

        let source = sb.install.parent().unwrap().join("source-tree");
        fs::create_dir_all(source.join("docs")).unwrap();
        fs::write(source.join("docs/readme.md"), "# regenerated\n").unwrap();

        let home = sb.install.parent().unwrap();
        let killed = run_fastf_aborting(
            &sb.install,
            home,
            "template:mid-save",
            &[
                "template",
                "from-folder",
                &source.display().to_string(),
                "crash",
                "--force",
                "--yes",
            ],
        );
        assert!(!killed.status.success(), "child should abort: {killed:?}");
        // Prove the boundary was actually reached: a save that never happened
        // would pass every assertion below without testing anything.
        assert!(
            String::from_utf8_lossy(&killed.stderr).contains("template:mid-save"),
            "the failpoint must have fired: {killed:?}"
        );

        // Either generation is acceptable; a manifest that no longer parses is not.
        let after = fs::read_to_string(&manifest).expect("manifest must still exist");
        assert!(
            after == before || template::find_by_slug("crash").is_ok(),
            "manifest is neither the old one nor a loadable new one:\n{after}"
        );
        template::find_by_slug("crash").expect("template must still load");

        // No scratch siblings left where a future reader could trip over them.
        let strays: Vec<String> = fs::read_dir(sb.install.join("templates/crash"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(
            strays.is_empty(),
            "temp scaffolding left behind: {strays:?}"
        );
    });
}

/// `ALL_FAULT_POINTS` must list exactly the names the source actually trips.
///
/// The list's own documentation says an invariant test iterates it and asserts
/// agreement with the call sites; until now nothing referenced the list at all,
/// so a boundary could gain a failpoint that no list, and therefore no reader,
/// knew about — and a name could be deleted from the code while the list went on
/// advertising it. Scanning the source is the only way to check this, since a
/// failpoint's name is a string literal by design.
#[test]
fn every_failpoint_in_the_source_is_declared_and_vice_versa() {
    fn collect(dir: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, found);
                continue;
            }
            // The module's own unit tests name points to prove the matcher works;
            // they are not boundaries in the code.
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("util/faults.rs") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            // A failpoint is armed at the boundary it guards with `check`, or —
            // `move:force-staged` — asked about as a decision with `is_armed`.
            for (_, rest) in text
                .match_indices("faults::check(\"")
                .chain(text.match_indices("faults::is_armed(\""))
                .map(|(i, m)| (i, &text[i + m.len()..]))
            {
                if let Some(name) = rest.split('"').next() {
                    found.push(name.to_string());
                }
            }
        }
    }

    let mut found = Vec::new();
    collect(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found.sort();
    found.dedup();

    let mut declared: Vec<String> = fastf::util::faults::ALL_FAULT_POINTS
        .iter()
        .map(|s| s.to_string())
        .collect();
    declared.sort();
    declared.dedup();

    assert_eq!(
        found, declared,
        "ALL_FAULT_POINTS and the `faults::check` call sites have drifted apart"
    );
}

/// `FASTF_FAULT=move:force-staged` puts a same-volume move onto the staged
/// copy path — the code a same-filesystem rename would never reach.
///
/// Proving it on one volume: arm `move:after-staging` alone and the move
/// succeeds, because the rename path never passes that boundary; arm
/// `move:force-staged` alongside it and the move fails *after staging* — the
/// injected error lands, the transaction is rolled back, and the source is
/// untouched.
#[cfg(debug_assertions)]
#[test]
fn force_staged_reaches_the_staged_path_on_one_volume() {
    fn create_one(cfg: &Config) -> std::path::PathBuf {
        let tmpl = template::find_by_slug("crash").unwrap();
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, cfg, false).unwrap();
        plan.root_path
    }
    fn find_project(cfg: &Config, root: &std::path::Path) -> library::Project {
        library::discover(cfg)
            .into_iter()
            .find(|p| p.path == root)
            .unwrap_or_else(|| panic!("no project at {}", root.display()))
    }
    let folder = |root: &std::path::Path| root.file_name().unwrap().to_os_string();

    sandbox(|sb, guard| {
        write_crash_template(&sb.install, "crash");
        let target = sb.install.parent().unwrap().join("target");
        fs::create_dir_all(&target).unwrap();
        let mut cfg = config_for(&sb.base);
        cfg.bases = vec![target.display().to_string()];
        cfg.save().unwrap();

        // Control: an after-staging arm does not stop a same-volume move,
        // because the rename path never reaches the staged boundary.
        let first_root = create_one(&cfg);
        arm(guard, "move:after-staging");
        let first = find_project(&cfg, &first_root);
        let moved = library::move_project(&first, &target);
        disarm(guard);
        assert!(
            moved.is_ok(),
            "a rename move must not pass move:after-staging: {moved:?}"
        );
        assert!(
            !first_root.exists() && target.join(folder(&first_root)).is_dir(),
            "the control move should have landed in the target base"
        );

        // Forced: the same arm plus move:force-staged reaches the staged path
        // and fails there, source intact, no transaction left behind.
        let second_root = create_one(&cfg);
        arm(guard, "move:force-staged,move:after-staging");
        let second = find_project(&cfg, &second_root);
        let forced = library::move_project(&second, &target);
        disarm(guard);
        let message = format!("{forced:?}");
        assert!(
            forced.is_err() && message.contains("move:after-staging"),
            "the forced move should fail at the staged boundary: {message}"
        );
        assert!(
            second_root.is_dir(),
            "the failed move must leave the source untouched"
        );
        assert!(
            !target.join(folder(&second_root)).exists(),
            "the failed move must not publish a destination"
        );
        assert_eq!(
            transaction_count(&target),
            0,
            "the rolled-back transaction must be removed"
        );
    });
}
