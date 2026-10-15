//! A program the user named is found where the user's shell finds it.
//!
//! A desktop launcher starts fastf with the session's `PATH`, which lacks the
//! folders an rc file adds (`~/.local/bin`). Each test here starts fastf with a
//! bare `PATH` and a display, and with a fake `$SHELL` whose "rc" adds a hidden
//! folder; the program the test configures lives only in that folder, so it is
//! found only if fastf asked the shell. The fake shell logs each time it is
//! asked, and whether it could open a terminal (it must not).
//!
//! **No test can open a real window**: `WAYLAND_DISPLAY` is removed and
//! `DISPLAY` names a server that is not there, and every terminal is a
//! recorder.

#![cfg(unix)]

mod common;

use common::{Recorder, Sandbox, recorder};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// fastf's own `PATH` in these tests: what a desktop session has.
const SESSION_PATH: &str = "/usr/bin:/bin";

struct Shell {
    program: PathBuf,
    hidden: PathBuf,
    calls: PathBuf,
}

impl Shell {
    /// A shell whose rc puts `hidden` first on its `PATH` and prints a banner,
    /// as rc files do. `printenv` is a function here, so the shell needs no
    /// folder of its own on `PATH` to answer, and a real emulator in
    /// `/usr/bin` cannot be found through it.
    fn new(sb: &Sandbox) -> Shell {
        Shell::with_rc(sb, "")
    }

    fn with_rc(sb: &Sandbox, rc: &str) -> Shell {
        use std::os::unix::fs::PermissionsExt;
        let hidden = sb.tmp.path().join("hidden");
        std::fs::create_dir_all(&hidden).unwrap();
        let calls = sb.tmp.path().join("shell.calls");
        let program = sb.tmp.path().join("fake-shell");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\n\
                 echo asked >> '{calls}'\n\
                 ( : < /dev/tty ) 2>/dev/null && echo tty >> '{calls}'\n\
                 {rc}\n\
                 echo 'Welcome back, a banner the rc prints'\n\
                 PATH='{hidden}':\"$PATH\"; export PATH\n\
                 printenv() {{ echo \"$PATH\"; }}\n\
                 for line; do :; done\n\
                 eval \"$line\"\n",
                calls = calls.display(),
                hidden = hidden.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        Shell {
            program,
            hidden,
            calls,
        }
    }

    fn times_asked(&self) -> usize {
        std::fs::read_to_string(&self.calls)
            .map(|calls| calls.lines().filter(|line| *line == "asked").count())
            .unwrap_or(0)
    }

    fn had_a_terminal(&self) -> bool {
        std::fs::read_to_string(&self.calls).is_ok_and(|calls| calls.contains("tty"))
    }
}

/// fastf as a desktop launcher's terminal starts it, but spawned by the test
/// rather than by an emulator: a bare `PATH`, a display, the fake shell.
fn launched(sb: &Sandbox, shell: &Shell) -> Command {
    let mut command = sb.command();
    command
        .env("PATH", SESSION_PATH)
        .env("DISPLAY", ":99")
        .env_remove("WAYLAND_DISPLAY")
        .env("SHELL", &shell.program);
    command
}

fn agent_only_the_shell_finds(sb: &Sandbox, shell: &Shell) -> Recorder {
    let rec = recorder(&shell.hidden, "fake-agent");
    sb.ok(&["config", "set", "agent", "fake-agent --resume"]);
    sb.plant_project(&sb.base, "proj", "ID0001");
    rec
}

fn first_on_path(rec: &Recorder) -> String {
    let path = rec
        .env_var("PATH")
        .expect("the program's environment has a PATH");
    path.split(':').next().unwrap_or_default().to_string()
}

/// One change to how fastf is started.
type Adjust = fn(&mut Command);

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The defect: an agent installed where only the user's shell looks
/// (`~/.local/bin`) was `command not found` (127) from a desktop launcher.
#[test]
fn an_agent_only_the_shell_finds_is_started() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let rec = agent_only_the_shell_finds(&sb, &shell);

    let out = launched(&sb, &shell)
        .args(["agent", "ID0001"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{out:?}");
    assert_eq!(rec.argv().as_deref(), Some(&["--resume".to_string()][..]));
    assert_eq!(first_on_path(&rec), shell.hidden.display().to_string());
    assert_eq!(shell.times_asked(), 1, "asked once");
    assert!(
        !shell.had_a_terminal(),
        "the shell is asked without the terminal"
    );
}

/// A program fastf's own `PATH` finds, from a shell (fastf does not lead its
/// session): nothing to ask, and the program gets fastf's `PATH` unchanged.
/// A guard on the design: 3.18.0 never asked either.
#[test]
fn a_program_fastf_finds_is_started_without_asking() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let bin = sb.tmp.path().join("bin");
    let rec = recorder(&bin, "fake-agent");
    sb.ok(&["config", "set", "agent", "fake-agent"]);
    sb.plant_project(&sb.base, "proj", "ID0001");
    let path = format!("{}:{SESSION_PATH}", bin.display());

    let out = launched(&sb, &shell)
        .env("PATH", &path)
        .args(["agent", "ID0001"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{out:?}");
    assert!(rec.argv().is_some(), "the agent should have run");
    assert_eq!(rec.env_var("PATH").as_deref(), Some(path.as_str()));
    assert_eq!(shell.times_asked(), 0);
}

/// Started the way a terminal emulator starts it — the leader of its own
/// session, which a shell never makes it — every program gets the shell's
/// `PATH`, found or not: the agent fastf finds may run tools of its own from
/// the folders the shell adds.
#[test]
fn a_fastf_leading_its_session_gives_every_program_the_shells_path() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let bin = sb.tmp.path().join("bin");
    let rec = recorder(&bin, "fake-agent");
    sb.ok(&["config", "set", "agent", "fake-agent"]);
    sb.plant_project(&sb.base, "proj", "ID0001");
    let path = PathBuf::from(format!("{}:{SESSION_PATH}", bin.display()));

    let (out, _) = common::pty::run(
        common::FASTF,
        &["agent", "ID0001"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("PATH", &path),
            ("DISPLAY", Path::new(":99")),
            ("SHELL", &shell.program),
        ],
        &common::pty::Script::new().build(),
        std::time::Duration::from_secs(20),
    );

    assert!(rec.argv().is_some(), "the agent should have run:\n{out}");
    assert_eq!(first_on_path(&rec), shell.hidden.display().to_string());
    assert!(
        !shell.had_a_terminal(),
        "the shell is asked without the terminal"
    );
}

/// The shell is asked only where a desktop started fastf: never without a
/// display (a console login's profile may start one), over ssh (the login
/// already ran the profile), or from inside the shell being asked. A guard
/// on the design, as are the builtin and emulator cases below.
#[test]
fn the_shell_is_not_asked_without_a_display_over_ssh_or_from_itself() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let rec = agent_only_the_shell_finds(&sb, &shell);

    let cases: [(&str, Adjust); 3] = [
        ("no display", |c| {
            c.env_remove("DISPLAY");
        }),
        ("ssh", |c| {
            c.env("SSH_CONNECTION", "10.0.0.1 22 10.0.0.2 22");
        }),
        ("inside the asking", |c| {
            c.env("FASTF_ASKING_SHELL", "1");
        }),
    ];
    for (case, adjust) in cases {
        let mut command = launched(&sb, &shell);
        adjust(&mut command);
        let out = command.args(["agent", "ID0001"]).output().unwrap();
        assert!(!out.status.success(), "{case}: {out:?}");
        assert!(stderr(&out).contains("127"), "{case}: {}", stderr(&out));
    }
    assert_eq!(shell.times_asked(), 0);
    assert!(!rec.was_called());
}

/// A line that starts with something the shell runs itself names no program
/// fastf could look for, so from a shell it is run as it is.
#[test]
fn a_line_that_starts_with_a_builtin_is_run_as_it_is() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let rec = recorder(&shell.hidden, "fake-agent");
    sb.ok(&["config", "set", "agent", "cd . && fake-agent"]);
    sb.plant_project(&sb.base, "proj", "ID0001");

    let out = launched(&sb, &shell)
        .args(["agent", "ID0001"])
        .output()
        .unwrap();

    assert!(!out.status.success(), "{out:?}");
    assert_eq!(shell.times_asked(), 0);
    assert!(!rec.was_called());
}

/// A shell that never answers is given up on, said so, and the program
/// starts with fastf's own `PATH`, as it did before fastf asked.
#[cfg(debug_assertions)]
#[test]
fn a_shell_that_does_not_answer_is_given_up_on() {
    let sb = Sandbox::new();
    let shell = Shell::with_rc(&sb, "sleep 30");
    let rec = agent_only_the_shell_finds(&sb, &shell);

    let started = std::time::Instant::now();
    let out = launched(&sb, &shell)
        .env("FASTF_FAULT", "shell:short-wait")
        .args(["agent", "ID0001"])
        .output()
        .unwrap();

    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let err = stderr(&out);
    assert!(
        err.contains("warning:")
            && err.contains("fake-shell")
            && err.contains("did not say its PATH"),
        "{err}"
    );
    assert!(!out.status.success(), "{out:?}");
    assert!(!rec.was_called());
}

/// Everything a create starts — a template's command, the editor — is found
/// where the shell finds it.
#[test]
fn what_a_create_starts_is_found_where_the_shell_finds_it() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let tool = recorder(&shell.hidden, "fake-tool");
    let editor = recorder(&shell.hidden, "fake-editor");
    sb.write_template("race");
    let manifest = sb.install.join("templates/race/template.yaml");
    let mut yaml = std::fs::read_to_string(&manifest).unwrap();
    yaml.push_str(
        "post_create:\n  open_in_editor: true\n  commands:\n    - \"fake-tool --made\"\n",
    );
    std::fs::write(&manifest, yaml).unwrap();
    sb.ok(&["config", "set", "editor", "fake-editor --wait"]);

    let out = launched(&sb, &shell)
        .args(["new", "race", "--name=One", "--yes", "--no-preview"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{out:?}");
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    assert!(
        !all.contains("could not") && !all.contains("exited"),
        "{all}"
    );
    assert_eq!(tool.argv().as_deref(), Some(&["--made".to_string()][..]));
    let argv = editor.argv().expect("the editor should have run");
    assert_eq!(argv[0], "--wait");
    assert_eq!(shell.times_asked(), 1, "asked once for the whole create");
}

/// The editor a note is written in is found where the shell finds it.
#[test]
fn the_note_editor_is_found_where_the_shell_finds_it() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let editor = recorder(&shell.hidden, "fake-editor");
    sb.ok(&["config", "set", "editor", "fake-editor"]);
    sb.plant_project(&sb.base, "proj", "ID0001");

    let _ = launched(&sb, &shell)
        .args(["note", "add", "ID0001"])
        .output()
        .unwrap();

    let argv = editor.argv().expect("the editor should have run");
    assert!(argv[0].contains("fastf-note-"), "{argv:?}");
}

/// A terminal named by the user and installed only where the shell looks is
/// the one that opens, before any emulator fastf's own `PATH` holds.
#[test]
fn a_terminal_only_the_shell_finds_is_the_one_opened() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let terminal = recorder(&shell.hidden, "fake-terminal");
    sb.ok(&["config", "set", "terminal", "fake-terminal"]);
    sb.plant_project(&sb.base, "proj", "ID0001");

    let out = launched(&sb, &shell)
        .args(["term", "ID0001"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{out:?}");
    assert!(
        terminal.argv().is_some(),
        "the named terminal should have opened"
    );
}

/// With no terminal named, an emulator on fastf's own `PATH` opens without
/// the shell being asked about the ones that are missing.
#[test]
fn an_emulator_fastf_finds_opens_without_asking() {
    let sb = Sandbox::new();
    let shell = Shell::new(&sb);
    let bin = sb.tmp.path().join("bin");
    let konsole = recorder(&bin, "konsole");
    sb.ok(&["config", "set", "agent", "true"]);
    sb.plant_project(&sb.base, "proj", "ID0001");

    let run = sb.run_like_a_launcher(
        &["agent", "ID0001"],
        &[
            ("PATH", bin.to_str().unwrap()),
            ("DISPLAY", ":99"),
            ("SHELL", shell.program.to_str().unwrap()),
        ],
    );

    assert_eq!(run.code, 0, "{}", run.output);
    assert!(konsole.argv().is_some(), "konsole should have opened");
    assert_eq!(shell.times_asked(), 0);
}
