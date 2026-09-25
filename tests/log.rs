//! The log and the messages: kept on disk, by every process at once, and read
//! back by `fastf log` and `fastf messages`.

use std::fs;
use std::process::Command;

mod common;

use common::Sandbox;

/// Set on a child this binary starts, to make it the writer rather than the
/// test.
const CHILD: &str = "FASTF_LOG_TEST_CHILD";
const LINES_EACH: usize = 1000;

/// Not a test of its own: the body a child process runs when the test below
/// starts this binary again. It writes its share of the log and leaves.
#[test]
fn log_writer_child() {
    let Some(name) = std::env::var_os(CHILD) else {
        return;
    };
    let name = name.to_string_lossy().into_owned();
    for n in 0..LINES_EACH {
        fastf::util::log::info(format!("{name} line {n} {}", "x".repeat(120)));
    }
}

/// Two processes writing at once leave every line whole: one `write` per
/// event on a file opened for appending, so neither can tear the other's.
#[test]
fn two_processes_writing_at_once_leave_every_line_whole() {
    let sb = Sandbox::new();
    let writers: Vec<_> = ["first", "second"]
        .iter()
        .map(|name| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "log_writer_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, name)
                .env("FASTF_INSTALL_DIR", &sb.install)
                .env(common::env::home_var(), sb.tmp.path())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut writer in writers {
        assert!(writer.wait().unwrap().success());
    }
    let text = fs::read_to_string(sb.install.join("logs/fastf.log")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2 * LINES_EACH, "every line, and no more");
    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 8, "a whole line: {line}");
        assert_eq!(fields[1], "INFO");
        assert_eq!(fields[7].len(), 120, "not torn: {line}");
    }
}

/// After a move the log has its steps with their counts and the messages
/// have its outcome — read back through the two verbs, in another process.
#[cfg(debug_assertions)]
#[test]
fn a_move_leaves_its_steps_in_the_log_and_its_outcome_in_the_messages() {
    let sb = Sandbox::new();
    let bases = sb.with_bases(&["other"]);
    sb.plant_project(&sb.base, "2026-01-01_Shoot_ID0001", "ID0001");
    fs::write(sb.base.join("2026-01-01_Shoot_ID0001/take.mov"), "frames").unwrap();

    let out = sb
        .command()
        .args(["move", "ID0001", &bases[0].display().to_string(), "-y"])
        .env("FASTF_FAULT", "move:force-staged")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");

    let log = sb.ok(&["log", "-n", "200"]);
    for step in [
        "move ID0001 2026-01-01_Shoot_ID0001",
        "scanned 2 entries",
        "copied 1 file",
        "removed the old copy 2 entries",
        ": done",
    ] {
        assert!(log.contains(step), "`{step}` is in the log:\n{log}");
    }
    let five = sb.ok(&["log", "-n", "5"]);
    assert_eq!(five.lines().count(), 5, "{five}");

    let messages = sb.ok(&["messages"]);
    assert!(
        messages.contains("moved ID0001 2026-01-01_Shoot_ID0001 to"),
        "{messages}"
    );
    assert!(messages.contains("cli"), "who said it: {messages}");
}

/// Nothing written yet reads as an empty log, not an error.
#[test]
fn an_empty_log_says_so() {
    let sb = Sandbox::new();
    let _ = fs::remove_dir_all(sb.install.join("logs"));
    let out = sb.ok(&["log"]);
    assert!(out.contains("The log is empty"), "{out}");
    let out = sb.ok(&["messages"]);
    assert!(out.contains("No messages yet"), "{out}");
}

/// `log-level` is a key like any other, and `off` keeps the central log
/// quiet.
#[test]
fn the_log_level_is_a_config_key() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "log-level", "off"]);
    let show = sb.ok(&["config", "show"]);
    assert!(
        show.contains("log_level:") && show.contains("off"),
        "{show}"
    );
    let refused = sb.fails(&["config", "set", "log-level", "loud"]);
    assert!(
        refused.contains("debug, info, warn, error, off"),
        "{refused}"
    );
}
