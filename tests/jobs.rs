//! Jobs: a move, a copy, a delete and a reconcile each in a process of its
//! own, which outlives whoever started it and any other fastf can follow,
//! cancel and leave alone. Real processes throughout — a job is a process.
//!
//! Every case slows a step down with a `delay-<ms>` failpoint, so they are
//! debug-only like the failpoints themselves.
#![cfg(debug_assertions)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

mod common;

use common::Sandbox;

/// A project of `files` small files, planted in `base`.
fn project(sb: &Sandbox, base: &Path, files: usize) -> PathBuf {
    let dir = sb.plant_project(base, "2026-01-01_Shoot_ID0001", "ID0001");
    fs::create_dir_all(dir.join("takes")).unwrap();
    for n in 0..files {
        fs::write(
            dir.join(format!("takes/take{n}.mov")),
            format!("frames {n}"),
        )
        .unwrap();
    }
    dir
}

/// `fastf <args>` with `fault` armed, started and not waited for.
fn start(sb: &Sandbox, args: &[&str], fault: &str) -> Child {
    sb.command()
        .args(args)
        .env("FASTF_FAULT", fault)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_until(what: &str, seconds: u64, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(seconds),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The newest job's state, read from its file.
fn newest(sb: &Sandbox) -> Option<(String, serde_json::Value)> {
    let root = sb.install.join("jobs");
    let mut ids: Vec<String> = fs::read_dir(&root)
        .ok()?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    let id = ids.pop()?;
    let text = fs::read_to_string(root.join(&id).join("state.json")).ok()?;
    Some((id, serde_json::from_str(&text).ok()?))
}

fn phase(state: &serde_json::Value) -> String {
    state["progress"]["phase"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn status(state: &serde_json::Value) -> String {
    state["status"].as_str().unwrap_or("").to_string()
}

fn in_phase(sb: &Sandbox, wanted: &str) -> bool {
    newest(sb).is_some_and(|(_, state)| phase(&state) == wanted)
}

fn ended(sb: &Sandbox) -> bool {
    newest(sb).is_some_and(|(_, state)| status(&state) != "running")
}

fn hidden_folders(base: &Path) -> Vec<String> {
    fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".fastf-moved-") || name.starts_with(".fastf-deleted-"))
        .collect()
}

/// **Killing the command line does not stop the move.** Its terminal was
/// only following; the job finishes, whole, and says so in its state.
#[test]
fn a_killed_command_line_leaves_its_move_to_finish() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    let original = project(&sb, &sb.base, 30);
    let mut cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,move:each-file:delay-40,remove:each-entry:delay-10",
    );
    wait_until("the copy", 20, || in_phase(&sb, "copying"));
    cli.kill().unwrap();
    let _ = cli.wait();

    wait_until("the job to end", 30, || ended(&sb));
    let (_, state) = newest(&sb).unwrap();
    assert_eq!(status(&state), "done", "{state}");
    assert!(!original.exists(), "the original is gone");
    assert_eq!(
        fs::read_dir(other.join("2026-01-01_Shoot_ID0001/takes"))
            .unwrap()
            .count(),
        30,
        "every file arrived"
    );
    assert!(hidden_folders(&sb.base).is_empty(), "no old copy is left");
}

/// A worker killed mid-copy is a job that stopped without saying how: its
/// state still says running and its lock is free. Reconcile rolls the move
/// back — the original was never touched.
#[cfg(unix)]
#[test]
fn a_killed_worker_is_read_as_interrupted_and_reconcile_rolls_it_back() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    let original = project(&sb, &sb.base, 30);
    let cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,move:each-file:delay-40",
    );
    wait_until("the copy", 20, || in_phase(&sb, "copying"));
    let (_, state) = newest(&sb).unwrap();
    let pid = state["pid"].as_u64().unwrap() as i32;
    // SAFETY: a signal to a process this test started, through its child.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    let out = cli.wait_with_output().unwrap();
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("stopped when its process ended"), "{said}");
    let listed = sb.ok(&["jobs"]);
    assert!(listed.contains("stopped"), "{listed}");

    let report = sb.ok(&["reconcile"]);
    assert!(report.contains("rolled back"), "{report}");
    assert_eq!(
        fs::read_dir(original.join("takes")).unwrap().count(),
        30,
        "the original is whole"
    );
    assert!(!other.join("2026-01-01_Shoot_ID0001").exists());
}

/// Killed while removing the old copy, the move is done — the project is in
/// its new place — and the next reconcile finishes the removal.
#[cfg(unix)]
#[test]
fn a_worker_killed_mid_removal_leaves_the_rest_to_reconcile() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    project(&sb, &sb.base, 30);
    let mut cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,remove:each-entry:delay-60",
    );
    wait_until("the removal", 20, || in_phase(&sb, "removing"));
    let (_, state) = newest(&sb).unwrap();
    // SAFETY: as above.
    unsafe {
        libc::kill(state["pid"].as_u64().unwrap() as i32, libc::SIGKILL);
    }
    let _ = cli.wait();
    assert!(
        other
            .join("2026-01-01_Shoot_ID0001/PROJECT_INFO.md")
            .is_file()
    );
    assert_eq!(
        hidden_folders(&sb.base).len(),
        1,
        "part of the old copy is left"
    );

    let report = sb.ok(&["reconcile"]);
    assert!(report.contains("finished"), "{report}");
    assert!(
        hidden_folders(&sb.base).is_empty(),
        "and reconcile finished it"
    );
}

/// Another process can stop a job: before the publish the move is undone;
/// after it the answer is "too late", and the move finishes.
#[test]
fn another_process_cancels_before_the_publish_and_is_told_too_late_after() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    let original = project(&sb, &sb.base, 30);
    let cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,move:each-file:delay-40",
    );
    wait_until("the copy", 20, || in_phase(&sb, "copying"));
    let asked = sb.ok(&["jobs", "cancel"]);
    assert!(asked.contains("Asked"), "{asked}");
    let out = cli.wait_with_output().unwrap();
    assert!(!out.status.success(), "a cancelled move fails");
    let (_, state) = newest(&sb).unwrap();
    assert_eq!(status(&state), "cancelled", "{state}");
    assert!(original.is_dir());
    assert!(!other.join("2026-01-01_Shoot_ID0001").exists());

    let cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,remove:each-entry:delay-60",
    );
    wait_until("the removal", 20, || in_phase(&sb, "removing"));
    let late = sb.ok(&["jobs", "cancel"]);
    assert!(late.contains("Too late"), "{late}");
    let out = cli.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(!original.exists());
}

/// While a job removes an old copy, a reconcile from another process leaves
/// that record alone and does not count it as needing attention; the data
/// lock is free, so a change elsewhere lands at once.
#[test]
fn a_live_jobs_records_are_its_own_and_the_lock_is_free_while_it_removes() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    project(&sb, &sb.base, 40);
    sb.plant_project(&sb.base, "2026-01-02_Other_ID0002", "ID0002");
    let cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,remove:each-entry:delay-60",
    );
    wait_until("the removal", 20, || in_phase(&sb, "removing"));

    let report = sb.ok(&["reconcile"]);
    assert!(report.contains("Nothing to reconcile"), "{report}");
    let started = Instant::now();
    sb.ok(&["tag", "add", "ID0002", "client"]);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a tag does not wait for a removal"
    );

    let out = cli.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(hidden_folders(&sb.base).is_empty());
}

/// While a job copies it holds the data lock, and a change elsewhere waits —
/// saying for which job.
#[test]
fn a_change_made_during_a_copy_waits_and_names_the_job() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    project(&sb, &sb.base, 30);
    sb.plant_project(&sb.base, "2026-01-02_Other_ID0002", "ID0002");
    let cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,move:each-file:delay-60",
    );
    wait_until("the copy", 20, || in_phase(&sb, "copying"));
    let out = sb.run(&["tag", "add", "ID0002", "client"]);
    assert!(out.status.success(), "{out:?}");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("waiting for the move ID0001"),
        "the wait names the job:\n{said}"
    );
    assert!(cli.wait_with_output().unwrap().status.success());
}

/// Two moves started at once take turns at the data lock, the second saying
/// it waits; both finish.
#[test]
fn two_moves_at_once_take_turns() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    project(&sb, &sb.base, 20);
    let second = sb.plant_project(&sb.base, "2026-01-02_Other_ID0002", "ID0002");
    let target = other.display().to_string();
    let first = start(
        &sb,
        &["move", "ID0001", &target, "--yes"],
        "move:force-staged,move:each-file:delay-50",
    );
    wait_until("the first copy", 20, || in_phase(&sb, "copying"));
    let out = sb
        .command()
        .args(["move", "ID0002", &target, "--yes", "--detach"])
        .env("FASTF_FAULT", "move:force-staged")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    wait_until("the second to wait", 20, || in_phase(&sb, "waiting"));
    assert!(first.wait_with_output().unwrap().status.success());
    wait_until("the second to end", 30, || ended(&sb));
    assert!(!second.exists());
    assert!(other.join("2026-01-02_Other_ID0002").is_dir());
}

/// `--detach` starts the job and returns; `fastf jobs watch` follows it to
/// the end and prints what the move would have printed.
#[test]
fn a_detached_move_is_followed_by_watch() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    project(&sb, &sb.base, 10);
    let out = sb
        .command()
        .args([
            "move",
            "ID0001",
            &other.display().to_string(),
            "--yes",
            "--detach",
        ])
        .env("FASTF_FAULT", "move:force-staged,move:each-file:delay-30")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("started job"), "{said}");
    let watched = sb.ok(&["jobs", "watch"]);
    assert!(watched.contains("Moved"), "{watched}");
    assert!(watched.contains("copied"), "{watched}");
}

/// A job's worker is detached from the terminal that started it: its own
/// session, so a hang-up of that terminal never reaches it.
#[cfg(unix)]
#[test]
fn a_worker_is_in_a_session_of_its_own() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    project(&sb, &sb.base, 20);
    let cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,move:each-file:delay-40",
    );
    wait_until("the copy", 20, || in_phase(&sb, "copying"));
    let (_, state) = newest(&sb).unwrap();
    let worker = state["pid"].as_u64().unwrap() as i32;
    // SAFETY: querying a process's session id has no effect on it.
    let (worker_session, our_session) = unsafe { (libc::getsid(worker), libc::getsid(0)) };
    assert_eq!(worker_session, worker, "the worker leads its own session");
    assert_ne!(worker_session, our_session);
    assert!(cli.wait_with_output().unwrap().status.success());
}

/// **Closing the terminal is not a cancel.** A hang-up reaches the command
/// following the move; it stops following, and the move goes on. Only Ctrl-C
/// asks the job to stop.
#[cfg(unix)]
#[test]
fn a_hang_up_leaves_the_move_running() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    let original = project(&sb, &sb.base, 30);
    let cli = start(
        &sb,
        &["move", "ID0001", &other.display().to_string(), "--yes"],
        "move:force-staged,move:each-file:delay-40",
    );
    wait_until("the copy", 20, || in_phase(&sb, "copying"));
    // SAFETY: a signal to a process this test started.
    unsafe {
        libc::kill(cli.id() as i32, libc::SIGHUP);
    }
    let out = cli.wait_with_output().unwrap();
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("cancelling"),
        "a hang-up is not a cancel: {out:?}"
    );
    wait_until("the job to end", 30, || ended(&sb));
    let (_, state) = newest(&sb).unwrap();
    assert_eq!(status(&state), "done", "{state}");
    assert!(!original.exists());
}

/// A job asked for a moment ago whose worker has not answered yet is
/// starting, not stopped — a reader must not report it killed.
#[test]
fn a_job_whose_worker_has_not_answered_yet_is_not_stopped() {
    let sb = Sandbox::new();
    let dir = sb.install.join("jobs").join("18d8983cdf094ce9-1-0");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("request.json"),
        r#"{"version":1,"kind":"reconcile","items":[]}"#,
    )
    .unwrap();
    let listed = sb.ok(&["jobs"]);
    assert!(listed.contains("running"), "{listed}");
    assert!(!listed.contains("stopped"), "{listed}");
}

/// Two reconciles at once: the first claims the removals it defers past its
/// lock, and the second leaves them to it — no half-finished record, no
/// removal run twice over one tree.
#[test]
fn a_second_reconcile_leaves_the_first_ones_removals_alone() {
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    project(&sb, &sb.base, 30);
    let killed = sb
        .command()
        .args(["move", "ID0001", &other.display().to_string(), "--yes"])
        .env("FASTF_FAULT", "move:force-staged,move:after-retire:abort")
        .output()
        .unwrap();
    assert!(!killed.status.success());
    assert_eq!(
        hidden_folders(&sb.base).len(),
        1,
        "the retired original is left"
    );

    let first = start(&sb, &["reconcile"], "remove:each-entry:delay-60");
    wait_until("the first reconcile's removal", 20, || {
        in_phase(&sb, "removing")
    });
    let second = sb.ok(&["reconcile"]);
    assert!(second.contains("Nothing to reconcile"), "{second}");
    let out = first.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert!(said.contains("1 move(s) finished"), "{said}");
    assert!(hidden_folders(&sb.base).is_empty());
}

/// **A job is nobody's unit's.** Started from a desktop launcher, the app runs
/// in a systemd service; with systemd's default exit type, its main process
/// ending stops the unit and SIGTERMs everything in its cgroup. The worker is
/// started in a scope of its own, so the move finishes anyway. Only where
/// there is a systemd user manager to ask — never on CI's runners.
#[cfg(target_os = "linux")]
#[test]
fn a_move_outlives_the_systemd_unit_that_started_it() {
    let bus = std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("bus"));
    let usable = bus.is_some_and(|bus| bus.exists())
        && std::process::Command::new("systemd-run")
            .args(["--user", "--quiet", "--wait", "--collect", "true"])
            .status()
            .is_ok_and(|status| status.success());
    if !usable {
        eprintln!("no systemd user manager here; nothing to show");
        return;
    }
    let sb = Sandbox::new();
    let other = sb.with_bases(&["other"]).remove(0);
    let original = project(&sb, &sb.base, 20);
    let unit = format!("fastf-test-{}", std::process::id());
    let set = |name: &str, value: &Path| format!("--setenv={name}={}", value.display());
    let started = std::process::Command::new("systemd-run")
        .args([
            "--user",
            "--quiet",
            "-p",
            "ExitType=main",
            "-p",
            "KillMode=control-group",
        ])
        .arg(format!("--unit={unit}"))
        .arg(set("FASTF_INSTALL_DIR", &sb.install))
        .arg(set("HOME", sb.tmp.path()))
        .arg("--setenv=FASTF_NO_RELAUNCH=1")
        .arg("--setenv=FASTF_FAULT=move:force-staged,move:each-file:delay-100")
        .arg(format!(
            "--setenv=XDG_RUNTIME_DIR={}",
            std::env::var("XDG_RUNTIME_DIR").unwrap()
        ))
        .arg(format!("--setenv=PATH={}", std::env::var("PATH").unwrap()))
        .arg(common::FASTF)
        .args([
            "move",
            "ID0001",
            &other.display().to_string(),
            "--yes",
            "--detach",
        ])
        .status()
        .unwrap();
    assert!(started.success());
    wait_until("the job to end", 30, || ended(&sb));
    let (_, state) = newest(&sb).unwrap();
    assert_eq!(status(&state), "done", "{state}");
    assert!(!original.exists());
}
