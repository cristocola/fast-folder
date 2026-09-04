//! A terminal when there is no terminal.
//!
//! From a desktop launcher fastf has nowhere to write: stdin is `/dev/null`,
//! stdout and stderr are journald sockets, and every line a command prints is
//! read by nobody. These pin the rule that decides when fastf re-runs itself in
//! a terminal emulator — and, much more importantly, when it must not.
//!
//! **Every test here pins `config set terminal <recorder>`**, so nothing in this
//! suite can open a real window on a developer's desktop or on a CI runner that
//! happens to have a display. See `tests/CLAUDE.md`.

#![cfg(unix)]

mod common;

use common::{Sandbox, recorder, shown_path};
use std::fs;

/// A sandbox whose configured terminal is a recorder, plus that recorder.
fn sandbox_with_recorder() -> (Sandbox, common::Recorder) {
    let sb = Sandbox::new();
    let rec = recorder(&sb.tmp.path().join("bin"), "fake-terminal");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &rec.program.display().to_string(),
    ]);
    (sb, rec)
}

/// The single most important negative: a pipe means somebody is reading, and a
/// pipe is what cron, CI and `| grep` all hand their children. A display being
/// set changes nothing about that.
#[test]
fn a_piped_stdout_never_relaunches_even_with_a_display() {
    let (sb, rec) = sandbox_with_recorder();
    sb.plant_project(&sb.base, "proj", "ID0001");

    let out = sb
        .command()
        .args(["recent", "--limit", "5"])
        .env("DISPLAY", ":99")
        .env("WAYLAND_DISPLAY", "wayland-99")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("running fastf");

    assert!(out.status.success(), "{out:?}");
    assert!(
        !rec.was_called(),
        "a piped run must never open a window — cron and CI depend on it"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ID0001"),
        "and it must print the plain list as it always has"
    );
}

/// The positive: with nothing to write to and a display present, fastf hands the
/// command to a terminal — the same command, argument for argument — and says
/// nothing itself.
#[test]
fn a_null_stdio_gui_session_relaunches_through_the_configured_terminal() {
    let (sb, rec) = sandbox_with_recorder();
    sb.plant_project(&sb.base, "proj", "ID0001");

    let run = sb.run_like_a_launcher(&["search", "rust project"], &[("DISPLAY", ":99")]);

    assert_eq!(run.code, 0, "the parent hands off and exits cleanly");
    assert_eq!(
        run.output, "",
        "the parent must print nothing: there is nowhere for it to go"
    );

    let argv = rec.argv().expect("the terminal should have been started");
    // The configured terminal is not one fastf knows, so it gets the
    // xterm-compatible `-e`, then the executable, then `--relaunched` and the
    // original argv — `rust project` still one argument.
    assert_eq!(argv[0], "-e", "an unknown emulator gets the -e convention");
    assert!(
        argv[1].ends_with("fastf"),
        "the second argument should be fastf itself, got {argv:?}"
    );
    assert_eq!(
        &argv[2..],
        ["--relaunched", "search", "rust project"],
        "the rerun carries the flag that says it is the rerun, then the original \
         argv verbatim, got {argv:?}"
    );
}

/// Both documented off switches, and both must work with everything else in
/// place — otherwise the escape hatch in the docs is a lie.
#[test]
fn no_relaunch_env_and_terminal_none_both_suppress_it() {
    let (sb, rec) = sandbox_with_recorder();

    let run = sb.run_like_a_launcher(
        &["search", "anything"],
        &[("DISPLAY", ":99"), ("FASTF_NO_RELAUNCH", "1")],
    );
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(
        !rec.was_called(),
        "FASTF_NO_RELAUNCH=1 must suppress the relaunch"
    );

    sb.ok(&["config", "set", "terminal", "none"]);
    let run = sb.run_like_a_launcher(&["search", "anything"], &[("DISPLAY", ":99")]);
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(
        !rec.was_called(),
        "terminal = \"none\" must suppress the relaunch"
    );
}

/// A remote session may have a forwarded display. Opening a window over X11
/// forwarding for a command somebody piped is not a favour.
#[test]
fn ssh_connection_suppresses_it() {
    let (sb, rec) = sandbox_with_recorder();

    let run = sb.run_like_a_launcher(
        &["search", "anything"],
        &[
            ("DISPLAY", ":99"),
            ("SSH_CONNECTION", "10.0.0.2 51000 10.0.0.1 22"),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(!rec.was_called(), "an ssh session must not open a window");
}

/// The loop guard, and the one job `FASTF_RELAUNCHED` still has. Inside the
/// relaunched process the terminal may still not be a terminal — a `terminal`
/// command that is not an emulator at all, say — and a second relaunch would
/// fork a window per attempt forever. The variable is inherited, so a descendant
/// that wrongly reads it opens no window: suppression is the safe direction, and
/// it is why this half stayed an environment variable when the claim "I am the
/// rerun" moved to argv.
#[test]
fn a_relaunched_child_with_no_tty_falls_through_to_plain_output() {
    let (sb, rec) = sandbox_with_recorder();
    sb.plant_project(&sb.base, "proj", "ID0001");

    let run = sb.run_like_a_launcher(
        &["recent", "--limit", "5"],
        &[("DISPLAY", ":99"), ("FASTF_RELAUNCHED", "1")],
    );

    assert_eq!(run.code, 0, "{}", run.output);
    assert!(
        !rec.was_called(),
        "a relaunched run must never relaunch again"
    );
    assert!(
        run.output.contains("ID0001"),
        "it must fall through to the plain list:\n{}",
        run.output
    );
}

/// A single match needs no terminal — the work is done and the answer is one
/// line. `path` still prints it (a journal trace costs nothing), and degrades
/// to what `copy` does so the answer reaches somewhere a person is looking.
#[test]
fn path_headless_gui_copies_and_notifies_but_still_prints() {
    let (sb, _rec) = sandbox_with_recorder();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    // A PATH holding nothing but a fake `notify-send`: no clipboard tool is
    // found either, which is the honest state of a machine like this.
    let bin = sb.tmp.path().join("notify-bin");
    let notify = recorder(&bin, "notify-send");

    let run = sb.run_like_a_launcher(
        &["path", "ID0001"],
        &[("DISPLAY", ":99"), ("PATH", &bin.display().to_string())],
    );

    assert_eq!(run.code, 0, "{}", run.output);
    assert_eq!(
        run.output,
        format!("{expected}\n"),
        "the path is still printed, and it is still the only thing on stdout"
    );
    let argv = notify.argv().expect("a notification should have been sent");
    assert_eq!(argv[0], "-a", "notifications are attributed to fastf");
    assert_eq!(argv[1], "fastf");
    assert!(
        argv.last().unwrap() == &expected,
        "the notification must carry the path, got {argv:?}"
    );
}

/// An ambiguous query from a launcher is the case that produced this phase: the
/// candidate list went to the journal and the command looked like it did
/// nothing at all. It must hand off, so the picker can be shown in a window.
#[test]
fn an_ambiguous_query_from_a_launcher_opens_a_terminal_instead_of_erroring() {
    let (sb, rec) = sandbox_with_recorder();
    sb.plant_project(&sb.base, "shared_one", "ID0011");
    sb.plant_project(&sb.base, "shared_two", "ID0012");

    let run = sb.run_like_a_launcher(&["copy", "shared"], &[("DISPLAY", ":99")]);

    assert_eq!(run.code, 0, "handing off is not a failure: {}", run.output);
    assert!(
        !run.output.contains("is ambiguous"),
        "the error belongs to readers, not to the journal:\n{}",
        run.output
    );
    let argv = rec.argv().expect("the terminal should have been started");
    assert_eq!(&argv[2..], ["--relaunched", "copy", "shared"]);
}

/// The redirect case, spelled out because it is the contract: `fastf path x > f`
/// writes the path to the file and opens nothing, display or no display. A
/// regular file is somebody keeping the bytes.
#[test]
fn a_redirected_path_writes_the_file_and_opens_nothing() {
    let (sb, rec) = sandbox_with_recorder();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    let target = sb.tmp.path().join("captured.txt");
    let file = fs::File::create(&target).unwrap();
    let out = sb
        .command()
        .args(["path", "ID0001"])
        .env("DISPLAY", ":99")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .output()
        .expect("running fastf");

    assert!(out.status.success(), "{out:?}");
    assert!(!rec.was_called(), "a redirect must never open a window");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        format!("{expected}\n"),
        "the file must hold the path and nothing else"
    );
}

/// Nothing about any of this may reach a run that has a terminal: the relaunch
/// machinery is invisible from a shell, which is where fastf is normally used.
#[test]
fn a_missing_display_is_enough_to_suppress_it() {
    let (sb, rec) = sandbox_with_recorder();

    let run = sb.run_like_a_launcher(
        &["search", "anything"],
        &[("DISPLAY", ""), ("WAYLAND_DISPLAY", "")],
    );
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(
        !rec.was_called(),
        "no display means there is no window to open"
    );

    // And a stale index file is not what decides it either — with a display it
    // does hand off, from the very same sandbox.
    let run = sb.run_like_a_launcher(&["search", "anything"], &[("DISPLAY", ":99")]);
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(rec.was_called(), "with a display it hands off");
    let _ = fs::metadata(&rec.log);
}
