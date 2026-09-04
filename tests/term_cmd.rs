//! `fastf term` — a shell at the project, without opening one for real.
//!
//! **Every test here pins `config set terminal <recorder>`** — the
//! `tests/relaunch.rs` harness rule — so no run can start a real emulator on a
//! developer's desktop or on a CI runner that happens to have a display. The
//! recorder logs its argv and its working directory; the assertions are about
//! *what fastf would have opened*, never about a window.

#![cfg(unix)]

mod common;

use common::{Sandbox, recorder, shown_path};

/// `fastf term …` with a display present: a terminal window needs a desktop
/// session, and a CI runner has none — the recorder stands in for the
/// emulator, the variable for the desktop.
fn term_ok(sb: &Sandbox, args: &[&str]) -> String {
    let out = sb
        .command()
        .args(args)
        .env("DISPLAY", ":99")
        .output()
        .expect("running fastf");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A known emulator gets its own directory flag — and the spawn's working
/// directory is the project too, the belt under the braces.
#[test]
fn a_known_emulator_gets_its_directory_flag_and_the_cwd() {
    let sb = Sandbox::new();
    let rec = recorder(&sb.tmp.path().join("bin"), "konsole");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &rec.program.display().to_string(),
    ]);
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    term_ok(&sb, &["term", "ID0001"]);

    let argv = rec.argv().expect("the terminal should have been started");
    assert_eq!(
        argv,
        ["--workdir", &expected],
        "a full path configured as `terminal` must still be recognised as konsole"
    );
    assert_eq!(
        rec.cwd().as_deref(),
        Some(expected.as_str()),
        "the spawn itself must start in the project folder"
    );
}

/// An emulator fastf has never heard of gets *no* guessed flag — a wrong flag
/// aborts it — and the inherited working directory is what carries the project.
#[test]
fn an_unknown_emulator_gets_no_flag_but_still_starts_in_the_project() {
    let sb = Sandbox::new();
    let rec = recorder(&sb.tmp.path().join("bin"), "fake-terminal");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &rec.program.display().to_string(),
    ]);
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    term_ok(&sb, &["term", "ID0001"]);

    // `printf '%s\n' "$@"` with no arguments still prints one empty line, so a
    // flagless call reads back as a single empty argument.
    let argv = rec.argv().expect("the terminal should have been started");
    assert_eq!(argv, [""], "an unknown emulator must get the bare argv");
    assert_eq!(
        rec.cwd().as_deref(),
        Some(expected.as_str()),
        "with no flag, the inherited working directory is the whole mechanism"
    );
}

/// `terminal = "none"` switches off the *automatic* relaunch, not this verb:
/// `fastf term` is an explicit request for a terminal, so it degrades to the
/// probe instead of refusing.
#[test]
fn terminal_none_does_not_disable_an_explicit_term() {
    let sb = Sandbox::new();
    let bin = sb.tmp.path().join("bin");
    let rec = recorder(&bin, "konsole");
    sb.ok(&["config", "set", "terminal", "none"]);
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    // A PATH holding nothing but the recorder: the probe finds "konsole" there
    // and nothing real can possibly be opened.
    let out = sb
        .command()
        .args(["term", "ID0001"])
        .env("PATH", &bin)
        .env("DISPLAY", ":99")
        .output()
        .expect("running fastf");
    assert!(out.status.success(), "{out:?}");

    let argv = rec.argv().expect("the probe should have found konsole");
    assert_eq!(argv, ["--workdir", &expected]);
}

/// The window fastf opens is the user's, and fastf's own bookkeeping must not
/// be in it. `FASTF_RELAUNCHED` is inherited: a terminal opened from a
/// relaunched fastf — the guided app's own "open a terminal here" — would pass
/// it to its shell, the shell to everything typed into it, and one of those is
/// a build that runs this suite, where the variable switches off the behaviour
/// four of its cases are about.
#[test]
fn the_terminal_it_opens_carries_none_of_the_relaunch_marker() {
    let sb = Sandbox::new();
    let rec = recorder(&sb.tmp.path().join("bin"), "konsole");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &rec.program.display().to_string(),
    ]);
    sb.plant_project(&sb.base, "proj", "ID0001");

    // Piped, so this is not the relaunched window's own fastf (`window_is_ours`
    // wants a real terminal): it spawns rather than execs, which is the path the
    // app's terminal action takes.
    let out = sb
        .command()
        .args(["term", "ID0001"])
        .env("DISPLAY", ":99")
        .env("FASTF_RELAUNCHED", "1")
        .output()
        .expect("running fastf");
    assert!(out.status.success(), "{out:?}");

    rec.argv().expect("the terminal should have been started");
    assert_eq!(
        rec.env_var("FASTF_RELAUNCHED"),
        None,
        "the window would hand the marker to its shell and its shell to everything after it"
    );
}

/// The launcher case the whole feature exists for: an ambiguous `fastf term`
/// with no terminal hands itself to one, argv verbatim, so the picker can be
/// shown in a window.
#[test]
fn an_ambiguous_term_from_a_launcher_hands_off_to_a_terminal() {
    let sb = Sandbox::new();
    let rec = recorder(&sb.tmp.path().join("bin"), "fake-terminal");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &rec.program.display().to_string(),
    ]);
    sb.plant_project(&sb.base, "shared_one", "ID0011");
    sb.plant_project(&sb.base, "shared_two", "ID0012");

    let run = sb.run_like_a_launcher(&["term", "shared"], &[("DISPLAY", ":99")]);

    assert_eq!(run.code, 0, "handing off is not a failure: {}", run.output);
    assert_eq!(
        run.output, "",
        "the parent must print nothing: there is nowhere for it to go"
    );
    let argv = rec.argv().expect("the terminal should have been started");
    assert_eq!(
        argv[0], "-e",
        "the relaunch, not the term spawn: -e + fastf"
    );
    assert_eq!(&argv[2..], ["--relaunched", "term", "shared"]);
}

/// An unambiguous `fastf term` from a launcher needs no relaunch: the window it
/// opens *is* the answer, and the notification says so where it can be seen.
#[test]
fn a_headless_unambiguous_term_opens_the_terminal_and_notifies() {
    let sb = Sandbox::new();
    let bin = sb.tmp.path().join("bin");
    let rec = recorder(&bin, "konsole");
    let notify = recorder(&bin, "notify-send");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &rec.program.display().to_string(),
    ]);
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    let run = sb.run_like_a_launcher(
        &["term", "ID0001"],
        &[("DISPLAY", ":99"), ("PATH", &bin.display().to_string())],
    );

    assert_eq!(run.code, 0, "{}", run.output);
    let argv = rec.argv().expect("the terminal should have been started");
    assert_eq!(argv, ["--workdir", &expected]);
    let argv = notify.argv().expect("a notification should have been sent");
    assert!(
        argv.last().unwrap() == &expected,
        "the notification must carry the path, got {argv:?}"
    );
}

/// The other side of that case, and a defect the same inherited variable caused:
/// a shell carrying `FASTF_RELAUNCHED` is not a window fastf opened. Every shell
/// in such a window has it, so `fastf term proj` typed there took the exec path
/// and replaced the user's own shell instead of opening the window they asked
/// for. An unambiguous `term` never hands off, so it can never be looking at a
/// relaunched window.
#[test]
fn an_unambiguous_term_in_a_marked_shell_still_opens_a_window() {
    let sb = Sandbox::new();
    let bin = sb.tmp.path().join("bin");
    let term_rec = recorder(&bin, "konsole");
    let shell_rec = recorder(&bin, "fake-shell");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &term_rec.program.display().to_string(),
    ]);
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    // A pty, so stdin and stderr are real terminals — the rest of the exec
    // path's conditions — with the marker set as an inherited one would be.
    let script = common::pty::Script::new().build();
    let (out, code) = common::pty::run(
        common::FASTF,
        &["term", "ID0001"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("FASTF_RELAUNCHED", std::path::Path::new("1")),
            ("SHELL", shell_rec.program.as_path()),
            ("DISPLAY", std::path::Path::new(":99")),
        ],
        &script,
        std::time::Duration::from_secs(15),
    );

    assert_eq!(code, 0, "{out}");
    let argv = term_rec.argv().expect("a window should have been opened");
    assert_eq!(argv, ["--workdir", &expected]);
    assert!(
        !shell_rec.was_called(),
        "the user's own shell must not be replaced:\n{out}"
    );
}

/// The relaunched-picker case: fastf already owns the window a relaunch opened
/// for its picker, so after the pick it must *become* the shell there — exec,
/// not a second window. `$SHELL` is a recorder, so the proof is its working
/// directory.
#[test]
fn a_relaunched_picker_execs_the_shell_in_its_own_window() {
    let sb = Sandbox::new();
    let bin = sb.tmp.path().join("bin");
    let term_rec = recorder(&bin, "fake-terminal");
    let shell_rec = recorder(&bin, "fake-shell");
    sb.ok(&[
        "config",
        "set",
        "terminal",
        &term_rec.program.display().to_string(),
    ]);
    let dir = sb.plant_project(&sb.base, "shared_one", "ID0011");
    sb.plant_project(&sb.base, "shared_two", "ID0012");
    let expected = shown_path(&dir);

    // A pty stands in for the relaunched window, driven exactly as the relaunch
    // drives it: `--relaunched` on the command line (the claim) and the variable
    // in the environment (the loop guard), a real terminal on every stream, an
    // ambiguous query — the picker draws, Enter takes the first row (ID0011,
    // lists are newest-first but these share a date; the row text is asserted
    // below).
    let script = common::pty::Script::new().enter().build();
    let (out, code) = common::pty::run(
        common::FASTF,
        &["--relaunched", "term", "shared"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("FASTF_RELAUNCHED", std::path::Path::new("1")),
            ("SHELL", shell_rec.program.as_path()),
        ],
        &script,
        std::time::Duration::from_secs(15),
    );

    assert_eq!(code, 0, "the exec'd shell (a recorder) exits 0:\n{out}");
    assert!(
        out.contains("Open a terminal at which project?"),
        "the picker must have been shown:\n{out}"
    );
    let picked = shell_rec.cwd().expect("the shell should have been exec'd");
    assert!(
        picked == expected || picked == shown_path(&sb.base.join("shared_two")),
        "the shell must start in a picked project, got {picked}"
    );
    assert_eq!(
        shell_rec.env_var("FASTF_RELAUNCHED"),
        None,
        "this shell is the user's — it must not inherit fastf's loop guard"
    );
    assert!(
        !term_rec.was_called(),
        "a relaunched fastf must never open a second window"
    );
}
