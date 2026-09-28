//! Without a terminal: every command that would ask refuses, and names the
//! way through.

use super::*;

/// The cursor restore (`interrupt::restore_terminal`) writes its escape only
/// to a stream that is a terminal; unguarded, a literal `\x1b[?25h` lands in
/// the output a script reads.
#[test]
fn a_piped_failure_leaks_no_terminal_escapes() {
    let sb = Sandbox::new();
    corrupt_the_config(&sb);

    let out = sb.run(&["recent", "--plain"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        !stdout.contains("\x1b[?25h") && !stderr.contains("\x1b[?25h"),
        "a piped failure must not emit terminal escapes:\nstdout: {stdout:?}\nstderr: {stderr:?}"
    );
}

#[test]
fn every_prompt_refuses_with_a_way_through() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let target = sb.tmp.path().join("existing");
    fs::create_dir_all(&target).unwrap();
    let target = target.display().to_string();
    let legacy = sb.base.join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    let legacy = legacy.display().to_string();

    // apply's confirmation
    refuses_without_a_terminal(&sb, &["apply", "race", &target, "--name=x"], "--yes");
    // register's rename confirmation
    refuses_without_a_terminal(&sb, &["register", &legacy, "--rename"], "--yes");
    // the template picker `fastf new` falls back to with no slug
    refuses_without_a_terminal(&sb, &["new"], "fastf new <slug>");
    // the interactive menu itself
    refuses_without_a_terminal(&sb, &[], "--help");
}

/// The menu prints a banner before it asks anything. Failing after the banner
/// puts decoration on stdout for a session that never existed.
#[test]
fn the_menu_refuses_before_it_draws_anything() {
    let sb = Sandbox::new();
    let out = sb.run_headless(&[]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "a menu that cannot run must not draw its banner: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `fastf move` with no terminal to confirm on refuses rather than moving: it
/// is the one prompt whose absence changes what happens on disk.
#[test]
fn a_move_without_a_terminal_refuses_instead_of_moving() {
    let sb = Sandbox::new();
    let archive = sb.with_bases(&["archive"]).remove(0);
    let project = sb.plant_project(&sb.base, "proj", "ID0001");

    let err = sb.fails_headless(&["move", "ID0001", "archive"]);
    assert!(
        err.contains("no terminal") && err.contains("--yes"),
        "a move that cannot confirm must refuse and say how:\n{err}"
    );
    assert!(project.is_dir(), "the project must still be where it was");
    assert!(
        !archive.join("proj").exists(),
        "nothing may be moved by a confirmation that never happened"
    );

    // With --yes there is nothing to confirm, so it goes through.
    let out = sb.run_headless(&["move", "ID0001", "archive", "--yes"]);
    assert!(out.status.success(), "move --yes failed: {out:?}");
    assert!(archive.join("proj").is_dir(), "--yes must still move it");

    // No base and no terminal: the picker cannot run, and the usage line is the answer.
    let err = sb.fails_headless(&["move", "ID0001"]);
    assert!(
        err.contains("no terminal") && err.contains("fastf move"),
        "the base picker must refuse with the noninteractive form:\n{err}"
    );
}

/// A terminal is on stderr and stdin; stdout is the output. `fastf new t >
/// out.txt` still prompts, because the guard asks stderr, not stdout.
#[cfg(unix)]
#[test]
fn a_redirected_stdout_still_has_a_terminal_to_prompt_on() {
    use common::pty;
    use std::time::Duration;

    let sb = Sandbox::new();
    sb.write_template("race");
    let captured = sb.tmp.path().join("out.txt");

    let (transcript, code) = pty::run_stdout_to(
        common::FASTF,
        &["new", "race", "--name=Redirected"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        // Confirm answers on the keypress itself — no Enter, or the newline
        // survives into the next prompt.
        &pty::Script::new().key("y").pause(400).key("n").build(),
        Duration::from_secs(20),
        &captured,
    );
    assert_eq!(
        code, 0,
        "a redirected stdout must not stop the prompt:\n{transcript}"
    );
    assert!(
        sb.base.join("R0001_Redirected").is_dir(),
        "the project should exist: {:?}",
        common::project_dirs(&sb.base)
    );
    let captured = fs::read_to_string(&captured).unwrap();
    assert!(
        captured.contains("R0001_Redirected"),
        "the redirected file is where the output went:\n{captured}"
    );
}

/// The first-run banner is on stderr, so `$(fastf path …)` is a path.
///
/// `ensure_bootstrapped` runs for every command but `completions` and
/// `mangen`. On a machine whose data directory does not exist yet but whose
/// base already holds projects — a second computer, a portable base, a
/// scripted `FASTF_INSTALL_DIR` — a banner on stdout lands in front of the
/// path `cd "$(fastf path lullaby)"` reads. `docs/cli.md` states the contract:
/// "prints the path followed by a newline — no colour, no decoration, nothing
/// else on stdout".
#[test]
fn the_first_run_banner_never_lands_in_a_command_substitution() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    // Throw the data directory away, keeping the base and its projects: the
    // next command bootstraps from scratch.
    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    fs::remove_dir_all(&sb.install).unwrap();
    fs::create_dir_all(&sb.install).unwrap();
    fs::write(sb.install.join("config.toml"), config).unwrap();

    let out = sb.run(&["path", "ID0001"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end(),
        shown_path(&dir),
        "stdout is the path and nothing else:\n{stdout}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("initialized in"),
        "and the banner is still said, on the stream for saying things"
    );
}

/// Without a desktop session there is nothing to open a window on: `open` and
/// `term` say so and exit non-zero, rather than starting a program that dies
/// at once and reporting it opened.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn open_and_term_refuse_without_a_display() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");
    for verb in ["open", "term"] {
        let out = sb
            .command()
            .args([verb, "ID0001"])
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .expect("running fastf");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success(),
            "{verb} must not claim success:\n{stderr}"
        );
        assert!(
            stderr.contains("no display"),
            "{verb} should say what is missing:\n{stderr}"
        );
    }
}
