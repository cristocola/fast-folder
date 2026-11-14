//! `move`, `rename` and `delete` on a project something is using: refused
//! under `--yes` with the list and the command that goes past it, done under
//! `--yes --force` with what it went past — and never because of the shell
//! that asked.

use super::*;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// A child the test kills and waits for however it ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Running {
    fn pid(&self) -> u32 {
        self.0.id()
    }
}

/// `program` with its working folder in `folder`, given a moment to be there.
fn working_in(folder: &Path, program: &str, args: &[&str]) -> Running {
    let child = Command::new(program)
        .args(args)
        .current_dir(folder)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    Running(child)
}

/// A project in the base, and an `archive` base to move it to.
fn project_and_archive(sb: &Sandbox) -> (std::path::PathBuf, std::path::PathBuf) {
    let archive = sb.with_bases(&["archive"]).remove(0);
    let project = sb.plant_project(&sb.base, "proj", "ID0001");
    (project, archive)
}

fn jobs_started(sb: &Sandbox) -> usize {
    fs::read_dir(sb.install.join("jobs"))
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}

#[test]
fn a_move_under_yes_is_refused_with_what_uses_it_and_the_way_past() {
    let sb = Sandbox::new();
    let (project, archive) = project_and_archive(&sb);
    let sitter = working_in(&project, "sleep", &["30"]);

    let err = sb.fails(&["move", "ID0001", "archive", "--yes"]);
    assert!(
        err.contains(&format!(
            "sleep (pid {}) is working in the project folder",
            sitter.pid()
        )),
        "{err}"
    );
    assert!(
        err.contains("hint: --force moves it anyway")
            && err.contains("`fastf move ID0001 archive --yes --force`"),
        "{err}"
    );
    assert!(project.is_dir(), "not moved");
    assert!(!archive.join("proj").exists());
    assert_eq!(jobs_started(&sb), 0, "no job was started");
}

#[test]
fn a_forced_move_goes_and_says_and_logs_what_it_went_past() {
    let sb = Sandbox::new();
    let (project, archive) = project_and_archive(&sb);
    let sitter = working_in(&project, "sleep", &["30"]);

    let out = sb.run(&["move", "ID0001", "archive", "--yes", "--force"]);
    drop(sitter);
    assert!(out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("note:") && err.contains("sleep (pid") && err.contains("restart it"),
        "{err}"
    );
    assert!(archive.join("proj").is_dir() && !project.exists());
    let log = fs::read_to_string(sb.install.join("logs/fastf.log")).unwrap();
    assert!(
        log.contains("forced the move of ID0001 past what used it: sleep (pid"),
        "{log}"
    );
}

/// A shell sitting in the project only has its prompt there: the move goes,
/// and says to `cd`.
#[test]
fn a_shell_at_its_prompt_does_not_stop_a_move() {
    let sb = Sandbox::new();
    let (project, archive) = project_and_archive(&sb);
    let shell = working_in(&project, "sh", &[]);

    let out = sb.run(&["move", "ID0001", "archive", "--yes"]);
    drop(shell);
    assert!(out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("sh (pid") && err.contains("has its prompt in the old folder"),
        "{err}"
    );
    assert!(archive.join("proj").is_dir());
}

/// A user unit that names the project counts whether or not it runs.
#[test]
fn a_unit_naming_the_project_refuses_until_forced() {
    let sb = Sandbox::new();
    let (project, archive) = project_and_archive(&sb);
    let config = sb.tmp.path().join("xdg-config");
    let units = config.join("systemd").join("user");
    fs::create_dir_all(&units).unwrap();
    let real = fs::canonicalize(&project).unwrap();
    let unit = format!("fastf-test-{}.service", std::process::id());
    fs::write(
        units.join(&unit),
        format!(
            "[Service]\nWorkingDirectory={}\nExecStart=/bin/true\n",
            real.display()
        ),
    )
    .unwrap();
    let run = |args: &[&str]| {
        sb.command()
            .args(args)
            .env("XDG_CONFIG_HOME", &config)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };

    let refused = run(&["move", "ID0001", "archive", "--yes"]);
    assert!(!refused.status.success());
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(
        err.contains(&unit) && err.contains("names it: WorkingDirectory="),
        "{err}"
    );
    assert!(project.is_dir());

    let forced = run(&["move", "ID0001", "archive", "--yes", "--force"]);
    assert!(forced.status.success(), "{forced:?}");
    let err = String::from_utf8_lossy(&forced.stderr);
    assert!(
        err.contains(&unit) && err.contains("still names the old folder"),
        "{err}"
    );
    assert!(archive.join("proj").is_dir());
}

#[test]
fn a_rename_and_a_delete_refuse_until_forced() {
    let sb = Sandbox::new();
    let project = sb.plant_project(&sb.base, "proj", "ID0001");
    let sitter = working_in(&project, "sleep", &["30"]);

    let err = sb.fails(&["rename", "ID0001", "renamed", "--yes"]);
    assert!(
        err.contains("so ID0001 was not renamed")
            && err.contains("`fastf rename ID0001 renamed --yes --force`"),
        "{err}"
    );
    assert!(project.is_dir());

    let err = sb.fails(&["delete", "ID0001", "--yes"]);
    assert!(
        err.contains("so ID0001 was not deleted")
            && err.contains("`fastf delete ID0001 --yes --force`"),
        "{err}"
    );
    assert!(project.join("PROJECT_INFO.md").is_file());
    assert_eq!(jobs_started(&sb), 0);

    let out = sb.run(&["rename", "ID0001", "renamed", "--yes", "--force"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("note: sleep (pid"));
    let renamed = sb.base.join("renamed");
    assert!(renamed.is_dir());

    let out = sb.run(&["delete", "ID0001", "--yes", "--force"]);
    drop(sitter);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("the deleted folder"),
        "{out:?}"
    );
    assert!(!renamed.join("PROJECT_INFO.md").exists());
}

/// Who asked for `job`'s verb, by pid, as its request carries them.
fn asking_of(job: &Path) -> Vec<u64> {
    let request: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(job.join("request.json")).unwrap()).unwrap();
    request["asking"]
        .as_array()
        .expect("the request carries who asked")
        .iter()
        .filter_map(|asked| asked["pid"].as_u64())
        .collect()
}

/// `program args…` run as the parent of the fastf it starts, working in
/// `folder`, in the sandbox; its pid, once it has ended well.
fn parent_of_fastf(sb: &Sandbox, folder: &Path, program: &str, args: &[&str]) -> u32 {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(folder)
        .env("FASTF", common::FASTF)
        .env("FASTF_INSTALL_DIR", &sb.install)
        .env("HOME", sb.tmp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in common::NOT_INHERITED {
        command.env_remove(name);
    }
    let child = command.spawn().unwrap();
    let pid = child.id();
    let done = child.wait_with_output().unwrap();
    assert!(done.status.success(), "{done:?}");
    pid
}

/// **Whoever asked never counts**: what runs `fastf move` from inside the
/// project and stays alive while it moves is left out — by the command
/// line's own look and by the job's, through the chain the request carries.
/// A shell is the usual parent; `timeout`, which works in the project and is
/// no shell, would stop the move if the chain were not left out.
#[test]
fn whoever_asked_is_left_out() {
    let sb = Sandbox::new();
    let archive = sb.with_bases(&["archive"]).remove(0);
    let first = sb.plant_project(&sb.base, "first", "ID0001");
    let second = sb.plant_project(&sb.base, "second", "ID0002");

    // Not `exec`: the shell has to stay, the parent of the fastf it ran.
    let shell = parent_of_fastf(
        &sb,
        &first,
        "sh",
        &[
            "-c",
            "\"$FASTF\" move ID0001 archive --yes; status=$?; exit $status",
        ],
    );
    assert!(archive.join("first").is_dir() && !first.exists());
    let job = fs::read_dir(sb.install.join("jobs"))
        .unwrap()
        .flatten()
        .find(|entry| entry.path().join("request.json").is_file())
        .expect("the move was a job")
        .path();
    let asking = asking_of(&job);
    assert!(
        asking.contains(&u64::from(shell)),
        "the shell that ran the command is in the chain the job leaves out: {asking:?}"
    );
    assert!(
        asking.contains(&u64::from(std::process::id())),
        "and so is everything above it: {asking:?}"
    );

    parent_of_fastf(
        &sb,
        &second,
        "timeout",
        &["60", common::FASTF, "move", "ID0002", "archive", "--yes"],
    );
    assert!(archive.join("second").is_dir() && !second.exists());
}

/// **Neither does what the command is piped to or from.** `tail`, `grep` or a
/// `sleep` feeding its input work in the project only because the shell that
/// ran the pipeline does; an agent in the project that pipes `fastf move` to
/// `tail` is not refused by its own `tail`, by the command line's look or by
/// the job's.
#[test]
fn what_the_command_is_piped_to_or_from_is_left_out() {
    let sb = Sandbox::new();
    let archive = sb.with_bases(&["archive"]).remove(0);
    let first = sb.plant_project(&sb.base, "first", "ID0001");
    let second = sb.plant_project(&sb.base, "second", "ID0002");

    parent_of_fastf(
        &sb,
        &first,
        "bash",
        &[
            "-c",
            "set -o pipefail; \"$FASTF\" move ID0001 archive --yes 2>&1 | cat | tail -n 50",
        ],
    );
    assert!(archive.join("first").is_dir() && !first.exists());

    parent_of_fastf(
        &sb,
        &second,
        "bash",
        &[
            "-c",
            "set -o pipefail; sleep 20 | \"$FASTF\" move ID0002 archive --yes",
        ],
    );
    assert!(archive.join("second").is_dir() && !second.exists());
}
