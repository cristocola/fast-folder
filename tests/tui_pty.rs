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
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(25);

/// Main-menu indices: Create, Projects, Search, Register, Templates, Settings, Quit.
const MENU_PROJECTS: usize = 1;
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

fn plant_dated_project(
    sb: &Sandbox,
    folder: &str,
    id: &str,
    created: &str,
    payload_bytes: usize,
) -> PathBuf {
    let root = sb.plant_project(&sb.base, folder, id);
    let pinfo = root.join("PROJECT_INFO.md");
    let raw = fs::read_to_string(&pinfo).unwrap();
    fs::write(&pinfo, raw.replace("2026-01-01T00:00:00Z", created)).unwrap();
    fs::write(root.join("payload.bin"), vec![7_u8; payload_bytes]).unwrap();
    root
}

#[test]
fn projects_browser_is_newest_first_sized_and_paged() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "1"]);
    plant_dated_project(
        &sb,
        "Newest_Project",
        "ID0003",
        "2026-03-03T00:00:00Z",
        2048,
    );
    plant_dated_project(
        &sb,
        "Middle_Project",
        "ID0002",
        "2026-02-02T00:00:00Z",
        1024,
    );
    plant_dated_project(&sb, "Oldest_Project", "ID0001", "2026-01-01T00:00:00Z", 512);

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        // Page 1: one project, Next, Back.
        .down(1)
        .enter()
        // Page 2: one project, Previous, Next, Back.
        .down(1)
        .enter()
        // Back on page 1: one project, Next, Back.
        .down(2)
        .enter()
        .down(MENU_QUIT)
        .enter()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(
        code, 0,
        "paged browser should return and quit cleanly:\n{out}"
    );
    assert!(
        out.contains("Projects — Page 1/3"),
        "page prompt missing:\n{out}"
    );
    assert!(
        out.contains("Projects — Page 2/3"),
        "next page missing:\n{out}"
    );
    assert!(
        out.contains("Previous page"),
        "previous control missing:\n{out}"
    );
    assert!(out.contains("Next page"), "next control missing:\n{out}");
    assert!(out.contains("Back"), "back control missing:\n{out}");
    assert!(
        out.contains("scanning…"),
        "the list should draw before any size is known:\n{out}"
    );
    assert!(
        out.contains("Size") && out.contains("KB"),
        "human size missing:\n{out}"
    );

    let newest = out.find("Newest_Project").expect("newest project label");
    let middle = out.find("Middle_Project").expect("middle project label");
    assert!(
        newest < middle,
        "projects were not shown newest first:\n{out}"
    );
    assert!(
        !out.contains("Oldest_Project"),
        "only visited pages should be scanned/rendered:\n{out}"
    );
    assert!(out.contains("Goodbye."));
}

#[test]
fn projects_browser_reloads_after_a_project_mutation() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "1"]);
    let root = plant_dated_project(
        &sb,
        "Mutable_Project",
        "ID0001",
        "2026-01-01T00:00:00Z",
        256,
    );

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        .enter() // select the project
        .down(2) // Add tag
        .enter()
        .line("draft")
        .pause(500)
        // Reloaded one-project page: project, Back.
        .down(1)
        .enter()
        .down(MENU_QUIT)
        .enter()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(
        code, 0,
        "mutation should return safely to the browser:\n{out}"
    );
    assert!(
        out.contains("Added 1 tag"),
        "tag action did not complete:\n{out}"
    );
    assert!(
        out.matches("scanning…").count() >= 2,
        "the changed project size should be invalidated and rescanned:\n{out}"
    );
    assert!(
        fs::read_to_string(root.join("PROJECT_INFO.md"))
            .unwrap()
            .contains("draft")
    );
    assert!(out.contains("Goodbye."));
}

/// The headline behaviour: the list is drawn before a single folder has been
/// walked, and the sizes arrive on their own while nothing is being typed.
///
/// The proof is structural rather than a timing race. `ProjectRowTheme` prefixes
/// only the *highlighted* row with `> `, and the one arrow key in this script
/// moves that highlight off the project for good. So a row rendered as
/// `> ID0001 … 3.0 MB` can only have been drawn while the selection was still
/// untouched — by a repaint the browser drove itself. A keypress-driven design
/// can produce the first half of this test but never the second.
#[test]
fn projects_browser_fills_in_sizes_without_any_input() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "1"]);
    // Three whole mebibytes, so the rendered figure is stable: PROJECT_INFO.md
    // adds a few hundred bytes, which cannot move a one-decimal MB reading.
    plant_dated_project(
        &sb,
        "Measured_Project",
        "ID0001",
        "2026-01-01T00:00:00Z",
        3 * 1024 * 1024,
    );

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        // Nothing at all is typed here. Any size that appears in this window was
        // drawn by the browser on its own.
        .pause(2500)
        .down(1) // → Back, the first keystroke since the list opened
        .enter()
        .down(MENU_QUIT)
        .enter()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "the browser should quit cleanly:\n{out}");
    let selected_row = |contains: &str| {
        out.lines()
            .any(|line| line.contains("> ID0001") && line.contains(contains))
    };

    assert!(
        selected_row("scanning…"),
        "the list must draw before the folder has been walked:\n{out}"
    );
    assert!(
        selected_row("3.0 MB"),
        "the size never reached the untouched list — the repaint is not live:\n{out}"
    );
    assert!(
        out.contains("> Back"),
        "the arrow key was never processed, so the anchor proves nothing:\n{out}"
    );
    assert!(out.contains("Goodbye."));
}
