//! The interactive menu, driven through a real terminal.
//!
//! The TUI had no tests at all, and the defect that mattered most could only be
//! seen from a terminal: **any recoverable error ended the session**. A mistyped
//! path in Register — after three answered prompts — or an out-of-range value in
//! Settings unwound all the way to `main` and exited 1, dropping the user back
//! to the shell with everything they had typed gone.
//!
//! These drive the real binary under a pty (`common::pty`), because `dialoguer`
//! refuses to prompt without one. Unix only by construction; the logic behind
//! containment is covered cross-platform by the `is_fatal` unit tests in
//! `tui::menu`.
//!
//! Two rules keep them from being flaky:
//! - keystrokes are **spaced**, never burst — `dialoguer` redraws between them,
//!   and a burst of six arrows loses most of them (`pty::Script` handles this);
//! - assertions match **stable text only**, never cursor-positioning escapes.

#![cfg(unix)]

mod common;

use common::{Sandbox, pty};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(25);

/// Main-menu indices: Create, Recent, Search, Register, Templates, Settings, Quit.
const MENU_REGISTER: usize = 3;
const MENU_SETTINGS: usize = 5;
const MENU_QUIT: usize = 6;

fn launch(sb: &Sandbox, script: Vec<pty::Keystroke>) -> (String, i32) {
    pty::run(
        common::FASTF,
        &[],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &script,
        DEADLINE,
    )
}

/// Baseline: the menu opens and Quit leaves cleanly. If this breaks, the
/// navigation the other tests rely on is wrong, not the behaviour under test.
#[test]
fn the_menu_opens_and_quits_cleanly() {
    let sb = Sandbox::new();
    let (out, code) = launch(&sb, pty::Script::new().down(MENU_QUIT).enter().build());

    assert_eq!(code, 0, "quitting should succeed:\n{out}");
    assert!(out.contains("Goodbye."), "expected a clean exit:\n{out}");
}

/// The headline regression. Register asks for a path *last*, so a typo used to
/// cost three answered prompts and the whole session.
#[test]
fn a_bad_register_path_returns_to_the_menu() {
    let sb = Sandbox::new();
    let script = pty::Script::new()
        .down(MENU_REGISTER)
        .enter()
        .line("/nope/does/not/exist") // folder to register (Input → Enter)
        .key("n") // attach a template?      (Confirm → keypress only)
        .key("n") // standardize the name?   (Confirm → keypress only)
        .pause(900)
        // Back at the main menu — prove it by quitting from there.
        .down(MENU_QUIT)
        .enter()
        .build();
    let (out, code) = launch(&sb, script);

    assert!(
        out.contains("does not exist or is not accessible"),
        "the failure should be reported:\n{out}"
    );
    assert_eq!(
        code, 0,
        "a mistyped path must not end the session (it exited {code}):\n{out}"
    );
    assert!(
        out.contains("Goodbye."),
        "expected to reach the menu and quit from it:\n{out}"
    );
}

/// Same shape, three submenus deep: an out-of-range setting is a correction to
/// make, not a reason to close the tool.
#[test]
fn an_invalid_setting_returns_to_the_menu() {
    let sb = Sandbox::new();
    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .down(3) // Settings → Recent projects
        .enter()
        .enter() // → Default list limit
        .line("0") // refused: must be at least 1
        .pause(700)
        .down(1) // Recent projects → Back
        .enter()
        .down(6) // Settings → Back
        .enter()
        .pause(400)
        .down(MENU_QUIT)
        .enter()
        .build();
    let (out, code) = launch(&sb, script);

    assert!(
        out.contains("must be at least 1"),
        "the validation failure should be reported:\n{out}"
    );
    assert_eq!(
        code, 0,
        "an invalid setting must not end the session (it exited {code}):\n{out}"
    );
}

/// Ctrl-C at the main menu, where nothing is in flight.
///
/// It used to announce "the partial project was removed" — untrue anywhere but
/// mid-create — and leave the terminal's cursor hidden, because `dialoguer`
/// balances hide/show only on the success path.
#[test]
fn ctrl_c_at_the_menu_is_honest_and_restores_the_cursor() {
    let sb = Sandbox::new();
    let (out, code) = launch(&sb, pty::Script::new().ctrl_c().build());

    assert_eq!(code, 130, "SIGINT should exit 130:\n{out}");
    assert!(
        !out.contains("partial project"),
        "nothing was being created, so nothing was removed:\n{out}"
    );
    assert!(
        out.contains("aborted"),
        "the exit should be announced:\n{out}"
    );
    assert!(
        out.contains("\x1b[?25h"),
        "the cursor must be shown again, or the user's shell is left blind"
    );
}
