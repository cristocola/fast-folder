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
#[cfg(debug_assertions)]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(25);

/// Main-menu indices: Create, Projects, Search, Register, Templates, Settings, Quit.
const MENU_PROJECTS: usize = 1;
const MENU_SEARCH: usize = 2;
const MENU_REGISTER: usize = 3;
const MENU_TEMPLATES: usize = 4;
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

/// `launch`, with `util::trace` writing to `trace`. Debug builds only — the
/// tracer is compiled out of release, like the failpoints, and so are its
/// callers.
#[cfg(debug_assertions)]
fn launch_traced(sb: &Sandbox, script: Vec<pty::Keystroke>, trace: &Path) -> (String, i32) {
    pty::run(
        common::FASTF,
        &[],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("FASTF_TRACE_FILE", trace),
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

/// The headline regression, now closed from the other side. Register used to ask
/// for the path *first* and reject it *last*, so a typo cost three more answered
/// prompts and the whole session. The path is now checked at the prompt that
/// collected it, and the text it rejected stays on the line to be corrected.
#[test]
fn a_bad_register_path_is_corrected_in_place() {
    let sb = Sandbox::new();
    let good = sb.base.join("Legacy");
    fs::create_dir_all(&good).unwrap();
    // Two characters longer than a real folder, so Backspace fixes it.
    let typo = format!("{}XX", good.display());

    let script = pty::Script::new()
        .down(MENU_REGISTER)
        .enter()
        .enter() // Register what? → One folder
        .key(&typo) // typed, not submitted
        .enter() // refused inline
        .pause(500)
        .backspace(2) // correct it without retyping the path
        .enter()
        .pause(500)
        .key("n") // attach a template?    (Confirm → keypress only)
        .key("n") // standardize the name? (Confirm → keypress only)
        .pause(500)
        .enter() // created date → the folder's own date
        .pause(900)
        .esc() // back at the main menu → quit
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(
        code, 0,
        "a mistyped path must not end the session (it exited {code}):\n{out}"
    );
    assert!(
        out.contains("no such folder"),
        "the path should be refused where it was typed:\n{out}"
    );
    assert!(
        good.join("PROJECT_INFO.md").exists(),
        "the corrected path should have been registered:\n{out}"
    );
}

/// Same shape, three submenus deep: an out-of-range setting is refused at the
/// field, the value stays there, and the correction is two keys rather than a
/// retype.
#[test]
fn an_invalid_setting_is_corrected_in_place() {
    let sb = Sandbox::new();
    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .down(3) // Settings → Project list (page size)
        .enter()
        .enter() // → the page-size field
        .key("0") // refused: must be at least 1
        .enter()
        .pause(500)
        .backspace(1)
        .key("5")
        .enter()
        .pause(700)
        .esc() // → Settings
        .esc() // → main menu
        .esc() // quit
        .build();
    let (out, code) = launch(&sb, script);

    assert!(
        out.contains("must be at least 1"),
        "the validation failure should be reported at the prompt:\n{out}"
    );
    assert_eq!(
        code, 0,
        "an invalid setting must not end the session (it exited {code}):\n{out}"
    );
    let shown = sb.ok(&["config", "show"]);
    assert!(
        shown.contains('5'),
        "the corrected value should have been saved:\n{shown}"
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

/// A tag changes one row, and only that row.
///
/// The browser used to answer every mutation by re-running `library::discover`
/// across every configured base, re-reading every `PROJECT_INFO.md` in the
/// library to put one word into one cell. The trace file is how that is
/// observable at all: the rendered list looks the same either way, and the cost
/// is seconds on a network share and nothing on a local disk.
// Debug-only, like the failpoint suites: `util::trace` compiles to nothing in
// release, so there would be no counts to compare.
#[cfg(debug_assertions)]
#[test]
fn a_tag_patches_its_row_without_rescanning_the_library() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "1"]);
    // Short name on purpose: the row is clamped to the terminal width, and the
    // tag cell is the last thing on it.
    let root = plant_dated_project(&sb, "Mut", "ID0001", "2026-01-01T00:00:00Z", 256);
    let trace = sb.tmp.path().join("trace");

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        .enter() // select the project
        .down(3) // → Tags
        .enter()
        .enter() // → Add a tag (no known tags yet, so it asks for one)
        .line("draft")
        .pause(700)
        // Still a one-project page: project, Back.
        .down(1)
        .enter()
        .down(MENU_QUIT)
        .enter()
        .build();
    let (out, code) = launch_traced(&sb, script, &trace);

    assert_eq!(
        code, 0,
        "mutation should return safely to the browser:\n{out}"
    );
    assert!(
        out.contains("Added 1 tag"),
        "tag action did not complete:\n{out}"
    );
    assert!(
        out.contains("[draft]"),
        "the patched row should show the new tag:\n{out}"
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

    let counts = fs::read_to_string(&trace).unwrap_or_default();
    let discoveries = counts.lines().filter(|line| *line == "discover").count();
    assert_eq!(
        discoveries, 1,
        "the browser opened once and must not have rescanned the library \
         to add a tag (traced {discoveries} discoveries):\n{counts}"
    );
    assert!(out.contains("Goodbye."));
}

/// Deleting a project takes its row out of the list, also without a rescan.
#[cfg(debug_assertions)]
#[test]
fn a_delete_drops_its_row_without_rescanning_the_library() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "9"]);
    plant_dated_project(&sb, "Doomed_Project", "ID0002", "2026-02-02T00:00:00Z", 128);
    plant_dated_project(&sb, "Kept_Project", "ID0001", "2026-01-01T00:00:00Z", 128);
    let trace = sb.tmp.path().join("trace");

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        .enter() // the newest row: Doomed_Project
        .down(7) // → Delete folder permanently
        .enter()
        .line("Doomed_Project") // typed confirmation
        .pause(800)
        .esc() // leave the browser
        .pause(400)
        .esc() // quit
        .build();
    let (out, code) = launch_traced(&sb, script, &trace);

    assert_eq!(code, 0, "a delete should return to the browser:\n{out}");
    assert!(out.contains("Deleted"), "the delete did not run:\n{out}");
    assert!(
        !sb.base.join("Doomed_Project").exists(),
        "the folder should be gone:\n{out}"
    );
    assert!(
        out.contains("Page 1/1"),
        "the list should have shrunk to a single page:\n{out}"
    );

    let counts = fs::read_to_string(&trace).unwrap_or_default();
    let discoveries = counts.lines().filter(|line| *line == "discover").count();
    assert_eq!(
        discoveries, 1,
        "dropping a row must not rescan the library (traced {discoveries}):\n{counts}"
    );
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

/// A `config.toml` that exists but does not parse changes which directory is
/// the library, so the menu that used to open on the home directory — with the
/// real projects nowhere in sight — has to refuse instead.
#[test]
fn a_corrupt_config_stops_the_menu() {
    let sb = Sandbox::new();
    let config_path = sb.install.join("config.toml");
    let mut raw = fs::read_to_string(&config_path).unwrap();
    raw.push_str("\nthis is = not [valid toml\n");
    fs::write(&config_path, raw).unwrap();

    let (out, code) = launch(&sb, pty::Script::new().pause(600).build());

    assert_eq!(code, 1, "an unreadable config must stop the menu:\n{out}");
    assert!(
        out.contains("config.toml") && out.contains("hint:"),
        "the failure must name the file and say how to recover:\n{out}"
    );
    assert!(
        !out.contains("What would you like to do?"),
        "no menu may open over a library fastf could not resolve:\n{out}"
    );
}

/// Run the menu on its own thread, so the test can drive a second fastf process
/// while one of its prompts is open.
fn launch_detached(
    sb: &Sandbox,
    script: Vec<pty::Keystroke>,
) -> std::thread::JoinHandle<(String, i32)> {
    let install = sb.install.clone();
    let home = sb.tmp.path().to_path_buf();
    std::thread::spawn(move || {
        pty::run(
            common::FASTF,
            &[],
            &[("FASTF_INSTALL_DIR", install.as_path()), ("HOME", &home)],
            &script,
            DEADLINE,
        )
    })
}

/// Settings held a loaded `Config` across the prompt and then wrote the whole
/// `bases` list back from that stale copy, silently reverting anything the
/// browser UI or another `fastf config set` had written meanwhile. The rule is
/// the one `edit_postcreate_commands` already follows: prompt first, then lock,
/// then reload.
///
/// The pty runs on its own thread so the test can write the config from a
/// second process while the "Base directory to add" prompt is open.
#[test]
fn adding_a_base_does_not_revert_a_concurrent_edit() {
    let sb = Sandbox::new();
    let typed = sb.tmp.path().join("typed_base");
    let concurrent = sb.tmp.path().join("concurrent_base");
    fs::create_dir_all(&typed).unwrap();
    fs::create_dir_all(&concurrent).unwrap();

    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .down(2) // Settings → Library bases
        .enter()
        .enter() // → Add a base directory
        // The prompt is open from here. Everything the menu writes after this
        // point must be computed from a config reloaded under the lock.
        .pause(2500)
        .line(&typed.display().to_string())
        .pause(1200)
        .ctrl_c()
        .build();
    let driver = launch_detached(&sb, script);

    // Well inside the window: the menu snapshots its config on entering the
    // submenu (~3.4s) and writes after the answer (~6.5s).
    std::thread::sleep(Duration::from_millis(5000));
    let out = sb.run(&["config", "set", "bases", &concurrent.display().to_string()]);
    assert!(
        out.status.success(),
        "concurrent config set failed: {out:?}"
    );

    let (transcript, _code) = driver.join().expect("pty thread");
    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    assert!(
        config.contains(&typed.display().to_string()),
        "the base typed into the menu is missing:\n{config}\n--- transcript ---\n{transcript}"
    );
    assert!(
        config.contains(&concurrent.display().to_string()),
        "the concurrently added base was reverted:\n{config}\n--- transcript ---\n{transcript}"
    );
}

/// The second Ctrl-C exits straight from the signal handler, bypassing `main`'s
/// error path — so the shell got its terminal back with the cursor still
/// hidden. The template's post-create command ignores SIGINT (and `SIG_IGN` is
/// inherited across exec), which keeps fastf blocked long enough for the second
/// interrupt to land on the path under test.
#[test]
fn a_second_ctrl_c_restores_the_cursor() {
    let sb = Sandbox::new();
    let dir = sb.install.join("templates").join("slow");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: Slow\nslug: slow\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: S\n  digits: 4\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n\
         \x20   required: true\n    transform: none\n\
         post_create:\n  commands:\n    - \"trap '' INT; sleep 5\"\n",
    )
    .unwrap();

    let script = pty::Script::new()
        .pause(700) // the project is created; the command is now sleeping
        .ctrl_c() // first: cooperative, sets the flag
        .ctrl_c() // second: stop being polite and exit
        .build();
    let (out, code) = pty::run(
        common::FASTF,
        &["new", "slow", "--name=Slow", "--yes"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &script,
        DEADLINE,
    );

    assert_eq!(code, 130, "the second interrupt should exit 130:\n{out}");
    assert!(
        out.contains("\x1b[?25h"),
        "the cursor must be shown again, or the user's shell is left blind:\n{out}"
    );
}

/// The other half of the same rule: the item says "Remove <base>", so that base
/// is what gets removed. Removing by the position it held in the list the user
/// saw would, once anything else had edited the list, both delete the wrong
/// entry and write the rest of the stale snapshot back over it.
#[test]
fn removing_a_base_leaves_the_rest_of_a_concurrent_edit_alone() {
    let sb = Sandbox::new();
    let (gone, kept, added) = (
        sb.tmp.path().join("base_a"),
        sb.tmp.path().join("base_b"),
        sb.tmp.path().join("base_c"),
    );
    for dir in [&gone, &kept, &added] {
        fs::create_dir_all(dir).unwrap();
    }
    let list = |dirs: [&PathBuf; 2]| {
        dirs.iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    sb.ok(&["config", "set", "bases", &list([&gone, &kept])]);

    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .down(2) // Settings → Library bases
        .enter() // the list the user sees is snapshotted here
        .down(1) // → Remove <base_a>
        .pause(2500)
        .enter()
        .pause(1200)
        .ctrl_c()
        .build();
    let driver = launch_detached(&sb, script);

    // Meanwhile, elsewhere: base_a goes away and base_c arrives, so every
    // position in the menu's snapshot now means something different.
    std::thread::sleep(Duration::from_millis(5000));
    let out = sb.run(&["config", "set", "bases", &list([&kept, &added])]);
    assert!(
        out.status.success(),
        "concurrent config set failed: {out:?}"
    );

    let (transcript, _code) = driver.join().expect("pty thread");
    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    let shows = |dir: &PathBuf| config.contains(&dir.display().to_string());
    assert!(
        !shows(&gone),
        "the base the user pointed at is still there:\n{config}\n--- transcript ---\n{transcript}"
    );
    assert!(
        shows(&kept) && shows(&added),
        "removing one base rewrote the list from the stale snapshot:\n{config}\n--- transcript ---\n{transcript}"
    );
}

// ---------------------------------------------------------------------------
// The cancel contract (v1.7.0)
//
// One case per row of the semantics table in `docs/cli.md`. Before this, Esc
// was ignored by every `dialoguer::Select` and swallowed outright by every
// `Input`, so the only way out of a menu — or out of a required variable that
// would not accept an empty answer — was Ctrl-C, which ends the session at
// exit 130 and throws away everything already typed.
// ---------------------------------------------------------------------------

/// Esc at the top level is Quit: there is no parent to go back to.
#[test]
fn esc_at_the_main_menu_quits() {
    let sb = Sandbox::new();
    let (out, code) = launch(&sb, pty::Script::new().esc().build());

    assert_eq!(code, 0, "Esc at the main menu should exit cleanly:\n{out}");
    assert!(out.contains("Goodbye."), "expected a clean exit:\n{out}");
}

/// Esc in a submenu goes to its parent, one level per press. Three levels down,
/// three presses reach the shell — and no press may skip a level.
#[test]
fn esc_backs_out_one_level_at_a_time() {
    let sb = Sandbox::new();
    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter() // Settings
        .enter() // → Project basics
        .esc() // → Settings
        .esc() // → main menu
        .esc() // → quit
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(
        code, 0,
        "Esc must not end the session with a failure:\n{out}"
    );
    assert!(
        out.contains("Goodbye."),
        "three levels, three presses, then the main menu's own exit:\n{out}"
    );
    // The parent reappeared. Anchored on rows that belong to exactly one of the
    // two menus: "Set date format" is only in Project basics, "Library bases" is
    // only in Settings, and the second must be drawn after the last of the first.
    let last_child_row = out.rfind("Set date format").expect("the submenu was drawn");
    assert!(
        out[last_child_row..].contains("Library bases"),
        "escaping the submenu should redraw its parent:\n{out}"
    );
}

/// Esc anywhere in the create wizard cancels the whole create. Nothing on disk,
/// and the ID counter is exactly where it was.
#[test]
fn esc_in_the_create_wizard_creates_nothing() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let before = sb.local_counter();

    let script = pty::Script::new()
        .enter() // Create new project → template picker
        .esc() // cancel at the picker
        .pause(500)
        .esc() // main menu → quit
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "a cancelled create is not a failure:\n{out}");
    assert!(
        out.contains("Cancelled"),
        "the cancel should say so, not fail silently:\n{out}"
    );
    assert!(
        common::project_dirs(&sb.base).is_empty(),
        "a cancelled create must leave no folder behind"
    );
    assert_eq!(
        sb.local_counter(),
        before,
        "a cancelled create must not consume an ID"
    );
}

/// The same, one prompt deeper: Esc at a required variable. This is the dead end
/// the old build had no exit from at all — an empty answer re-prompted forever
/// and Esc did nothing.
#[test]
fn esc_at_a_required_variable_creates_nothing() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let before = sb.local_counter();

    let script = pty::Script::new()
        .enter() // Create new project
        .enter() // pick the only template
        .pause(400)
        .esc() // at the required "Name" variable
        .pause(500)
        .esc() // main menu → quit
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "a cancelled create is not a failure:\n{out}");
    assert!(
        common::project_dirs(&sb.base).is_empty(),
        "a cancelled create must leave no folder behind:\n{out}"
    );
    assert_eq!(sb.local_counter(), before, "no ID may be consumed:\n{out}");
}

/// Esc in a settings field leaves the value alone and returns to the submenu.
#[test]
fn esc_in_a_settings_field_leaves_the_value_unchanged() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "7"]);

    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .down(3) // Settings → Project list (page size)
        .enter()
        .enter() // → the page-size field
        .pause(400)
        .esc() // abandon the edit
        .pause(400)
        .esc() // → Settings
        .esc() // → main menu
        .esc() // quit
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "Esc in a field is not a failure:\n{out}");
    let shown = sb.ok(&["config", "show"]);
    assert!(
        shown.contains('7'),
        "the setting must be untouched by a cancelled edit:\n{shown}"
    );
}

/// Esc in the project list returns to the main menu; Esc in the action menu
/// returns to the list it was opened from.
#[test]
fn esc_walks_back_out_of_the_project_browser() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter() // the browser
        .pause(600)
        .enter() // open the row's action menu
        .pause(600)
        .esc() // → back to the list
        .pause(600)
        .esc() // → main menu
        .pause(400)
        .esc() // quit
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "Esc must not end the session:\n{out}");
    assert!(
        out.contains("Goodbye."),
        "two presses should reach the main menu:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// Keep what you typed (v1.7.0)
//
// Every one of these flows used to ask its dependent questions first and reject
// the value they depended on afterwards, so a single typo cost every answer
// given since. The rule now: a value with a local validity rule is checked at
// the prompt that collected it.
// ---------------------------------------------------------------------------

/// `template from-folder` asked path, slug and force, then rejected the slug.
#[test]
fn a_bad_template_slug_is_refused_at_its_own_prompt() {
    let sb = Sandbox::new();
    let source = sb.base.join("Source");
    fs::create_dir_all(source.join("01_Assets")).unwrap();

    let script = pty::Script::new()
        .down(MENU_TEMPLATES)
        .enter()
        .down(1) // → Generate template from existing folder
        .enter()
        .line(&source.display().to_string())
        .pause(400)
        .key("not a slug") // spaces are not allowed in a slug
        .enter()
        .pause(500)
        // Correct it in place: drop " a slug", leaving "not".
        .backspace(7)
        .key("-a-slug")
        .enter()
        .pause(500)
        .key("n") // overwrite if it exists?
        .key("n") // bundle binary/large files?
        .pause(900)
        .esc() // Templates → main menu
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "a bad slug must not end the session:\n{out}");
    assert!(
        sb.install
            .join("templates/not-a-slug/template.yaml")
            .exists(),
        "the corrected slug should have produced the template:\n{out}"
    );
}

/// Apply asked template, target, dry-run and every variable, then rejected the
/// target. The target now comes second, and is checked there.
#[test]
fn apply_refuses_a_missing_target_before_asking_anything_else() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let script = pty::Script::new()
        .down(MENU_TEMPLATES)
        .enter()
        .down(3) // → Apply template to existing folder
        .enter()
        .enter() // pick the only template
        .pause(400)
        .line("/nope/does/not/exist")
        .pause(600)
        .esc() // give up on the target
        .pause(400)
        .esc() // Templates → main menu
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "a missing target must not end the session:\n{out}");
    assert!(
        out.contains("no such folder"),
        "the target should be refused at its own prompt:\n{out}"
    );
    assert!(
        !out.contains("Dry run first"),
        "nothing that depends on the target may be asked before it is valid:\n{out}"
    );
}

/// A base directory that is not absolute is refused where it is typed, and the
/// text stays there — so making it absolute is a Home and a prefix, not a
/// retype. Pre-Phase-8 the message appeared too, but only after the field had
/// closed and thrown the value away.
#[test]
fn a_relative_base_directory_is_corrected_in_place() {
    let sb = Sandbox::new();
    let prefix = sb.tmp.path().display().to_string();

    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .enter() // Project basics
        .enter() // → Set base directory
        .key("relative/path")
        .enter() // refused inline
        .pause(600)
        .home()
        .key(&format!("{prefix}/"))
        .enter()
        .pause(800)
        .esc() // → Settings
        .esc() // → main menu
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "a rejected value must not end the session:\n{out}");
    assert!(
        out.contains("absolute path"),
        "the reason should appear at the prompt:\n{out}"
    );
    let shown = sb.ok(&["config", "show"]);
    assert!(
        shown.contains(&format!("{prefix}/relative/path")),
        "the text the prompt rejected should still have been there to fix:\n{shown}"
    );
}

/// A search that matches nothing comes back with the query still in the field.
#[test]
fn a_search_that_matches_nothing_keeps_the_query() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    let script = pty::Script::new()
        .down(MENU_SEARCH)
        .enter()
        .key("Alphaa") // one letter too many
        .enter()
        .pause(700)
        // The query is back in the field: one Backspace makes it match.
        .backspace(1)
        .enter()
        .pause(900)
        .esc() // leave the browser
        .pause(400)
        .esc() // quit
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "a search miss must not end the session:\n{out}");
    assert!(
        out.contains("No projects match that query."),
        "the miss should be reported:\n{out}"
    );
    // Anchored on the browser's own prompt, not on the project name: "Alpha" is
    // a substring of the "Alphaa" that was typed, so matching it would pass
    // against a build that never re-ran the search at all.
    assert!(
        out.contains("Projects — Page 1/1"),
        "the corrected query should have opened the browser:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// The frame, the keys, and one browser (v1.7.0)
// ---------------------------------------------------------------------------

/// The main menu says what the library looks like, and it costs no scan.
///
/// The counts come from each base's own `.fastf-index.json` and are labelled as
/// such, so opening the menu does not get slower the more it has to say. A base
/// that is configured but not mounted is named rather than silently dropped.
#[cfg(debug_assertions)]
#[test]
fn the_frame_reports_the_library_from_the_index_without_scanning() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    sb.plant_project(&sb.base, "2026-02-02_Beta_ID0002", "ID0002");
    // Build the cache once, up front, the way ordinary use would.
    sb.ok(&["reindex"]);

    // A base that is configured and then taken away.
    let gone = sb.tmp.path().join("unplugged");
    fs::create_dir_all(&gone).unwrap();
    sb.ok(&["config", "set", "bases", &gone.display().to_string()]);
    fs::remove_dir_all(&gone).unwrap();

    let trace = sb.tmp.path().join("trace");
    let (out, code) = launch_traced(&sb, pty::Script::new().esc().build(), &trace);

    assert_eq!(code, 0, "the menu should open and quit:\n{out}");
    assert!(
        out.contains("2 projects") && out.contains("(from index)"),
        "the frame should report the indexed count, and say it is from the index:\n{out}"
    );
    assert!(
        out.contains("highest ID0002"),
        "the frame should report the highest ID:\n{out}"
    );
    assert!(
        out.contains("(not mounted)"),
        "a configured base that is gone should be named:\n{out}"
    );

    let counts = fs::read_to_string(&trace).unwrap_or_default();
    let scans = counts.lines().filter(|line| *line == "scan_base").count();
    assert_eq!(
        scans, 0,
        "opening the menu must not scan a single base (traced {scans}):\n{counts}"
    );
}

/// `/` filters the list, and Enter opens the row that is left.
#[test]
fn the_browser_filter_narrows_to_one_row() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "9"]);
    plant_dated_project(&sb, "Aardvark", "ID0001", "2026-01-01T00:00:00Z", 64);
    plant_dated_project(&sb, "Buffalo", "ID0002", "2026-02-02T00:00:00Z", 64);
    plant_dated_project(&sb, "Capybara", "ID0003", "2026-03-03T00:00:00Z", 64);

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        .pause(600)
        .key("/")
        .key("buff") // lower case: the filter is case-insensitive
        .pause(600)
        .enter() // opens the only row left
        .pause(600)
        .esc() // action menu → list
        .pause(400)
        .esc() // list → main menu
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "filtering should not end the session:\n{out}");
    assert!(
        out.contains("filter: buff"),
        "the filter line should be drawn under the prompt:\n{out}"
    );
    // What the filtered list itself contained: from the last time the filter
    // line was drawn to the action menu it opened, Buffalo is the only project
    // on screen.
    let filtered_at = out.rfind("filter: buff").expect("the filter was drawn");
    // The *next* action menu after the filter, not the last one in the stream:
    // the main menu asks the same question, and it is drawn again at the end.
    let opened = filtered_at
        + out[filtered_at..]
            .find("What would you like to do?")
            .expect("an action menu was opened");
    let filtered_view = &out[filtered_at..opened];
    assert!(
        filtered_view.contains("Buffalo"),
        "the matching row should stay:\n{filtered_view}"
    );
    assert!(
        !filtered_view.contains("Aardvark") && !filtered_view.contains("Capybara"),
        "the rows that do not match should be gone:\n{filtered_view}"
    );
}

/// PageDown moves the highlight by a viewport rather than a row.
#[test]
fn page_keys_move_by_a_viewport() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-default-limit", "40"]);
    for n in 1..=40 {
        plant_dated_project(
            &sb,
            &format!("P{n:02}"),
            &format!("ID{n:04}"),
            &format!("2026-01-{:02}T00:00:00Z", (n % 28) + 1),
            32,
        );
    }

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        .pause(700)
        .page_down()
        .pause(500)
        .page_up()
        .pause(500)
        .esc()
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "page keys should not end the session:\n{out}");
    // A 40-row list cannot fit a 24-row terminal, so the window hint is drawn on
    // every repaint — which is why counting hints proves nothing. What proves it
    // is a hint that does *not* start at row 1: only a scroll produces one, and
    // one arrow key cannot reach it in this script.
    let starts: Vec<usize> = out
        .match_indices("(rows ")
        .filter_map(|(at, marker)| {
            out[at + marker.len()..]
                .split('–')
                .next()?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .collect();
    assert!(
        !starts.is_empty(),
        "the viewport hint should be drawn for a list taller than the terminal:\n{out}"
    );
    assert!(
        starts.iter().any(|start| *start > 1),
        "PageDown should have scrolled the window past the first row \
         (window starts seen: {starts:?}):\n{out}"
    );
}

/// `fastf recent` on a terminal opens the same browser the menu opens, with the
/// Size column and the same action menu. It used to have a second, size-less
/// picker of its own.
#[test]
fn fastf_recent_opens_the_same_browser() {
    let sb = Sandbox::new();
    plant_dated_project(&sb, "Solo", "ID0001", "2026-01-01T00:00:00Z", 4096);

    let script = pty::Script::new()
        .pause(700)
        .enter()
        .pause(700)
        .esc()
        .esc()
        .build();
    let (out, code) = pty::run(
        common::FASTF,
        &["recent"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &script,
        DEADLINE,
    );

    assert_eq!(code, 0, "`fastf recent` should exit cleanly:\n{out}");
    assert!(
        out.contains("Size") && out.contains("Projects — Page 1/1"),
        "`fastf recent` should show the guided browser with sizes:\n{out}"
    );
    assert!(
        out.contains("Quit"),
        "started from a shell, the last row says Quit, not Back to main menu:\n{out}"
    );
}

/// Copy path always says what it did. With no clipboard tool on PATH it prints
/// the path instead — a Copy that silently does nothing is the worst version.
#[test]
fn copy_path_falls_back_to_printing_the_path() {
    let sb = Sandbox::new();
    let root = plant_dated_project(&sb, "Copied", "ID0001", "2026-01-01T00:00:00Z", 64);
    // An empty PATH: no wl-copy, no xclip, no pbcopy.
    let empty = sb.tmp.path().join("empty-path");
    fs::create_dir_all(&empty).unwrap();

    let script = pty::Script::new()
        .down(MENU_PROJECTS)
        .enter()
        .pause(600)
        .enter() // open the row
        .down(1) // → Copy path
        .enter()
        .pause(600)
        .esc()
        .esc()
        .esc()
        .build();
    let (out, code) = pty::run(
        common::FASTF,
        &[],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("PATH", empty.as_path()),
        ],
        &script,
        DEADLINE,
    );

    assert_eq!(code, 0, "Copy path should not end the session:\n{out}");
    assert!(
        out.contains("no clipboard tool found"),
        "with nothing on PATH it should say so:\n{out}"
    );
    assert!(
        out.contains(&root.display().to_string()),
        "and print the path it could not copy:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// Parity with the CLI, and a builder that lets you change your mind (v1.7.0)
// ---------------------------------------------------------------------------

/// Bulk register: the preview first, then the commit, both from the menu.
///
/// `--recursive` was command-line only, so onboarding a folder of legacy
/// projects meant leaving the tool the whole flow was designed for.
#[test]
fn the_menu_can_register_a_whole_base_after_previewing_it() {
    let sb = Sandbox::new();
    // Inside the configured base: registration refuses a folder that is not a
    // direct child of one, which is what keeps a registered project findable.
    let legacy = sb.base.clone();
    for name in ["One", "Two"] {
        fs::create_dir_all(legacy.join(name)).unwrap();
    }

    let script = pty::Script::new()
        .down(MENU_REGISTER)
        .enter()
        .down(1) // → Every unregistered folder in a base
        .enter()
        .line(&legacy.display().to_string())
        .pause(400)
        .key("n") // attach a template?
        .pause(400)
        .enter() // created date → the folder's own date
        .pause(900)
        .key("y") // register these folders now?
        .pause(1200)
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "bulk register should return to the menu:\n{out}");
    assert!(
        out.contains("dry run") || out.contains("Preview"),
        "the preview should be shown before anything is written:\n{out}"
    );
    assert!(
        legacy.join("One/PROJECT_INFO.md").exists() && legacy.join("Two/PROJECT_INFO.md").exists(),
        "both folders should have been registered:\n{out}"
    );
}

/// Answering no to the preview writes nothing.
#[test]
fn declining_the_bulk_register_preview_writes_nothing() {
    let sb = Sandbox::new();
    let legacy = sb.base.clone();
    fs::create_dir_all(legacy.join("One")).unwrap();

    let script = pty::Script::new()
        .down(MENU_REGISTER)
        .enter()
        .down(1)
        .enter()
        .line(&legacy.display().to_string())
        .pause(400)
        .key("n") // attach a template?
        .pause(400)
        .enter() // created date
        .pause(900)
        .key("n") // register these folders now? → no
        .pause(700)
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "declining should return to the menu:\n{out}");
    assert!(
        !legacy.join("One/PROJECT_INFO.md").exists(),
        "a declined preview must write nothing:\n{out}"
    );
}

/// Registering with "Today" as the created date, which the menu could not say.
#[test]
fn the_menu_can_register_with_todays_date() {
    let sb = Sandbox::new();
    let folder = sb.base.join("Legacy");
    fs::create_dir_all(&folder).unwrap();

    let script = pty::Script::new()
        .down(MENU_REGISTER)
        .enter()
        .enter() // One folder
        .line(&folder.display().to_string())
        .pause(400)
        .key("n") // attach a template?
        .key("n") // standardize the name?
        .pause(400)
        .down(1) // created date → Today
        .enter()
        .pause(900)
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "register should return to the menu:\n{out}");
    let meta = fs::read_to_string(folder.join("PROJECT_INFO.md")).expect("registered");
    // Not the folder's own timestamp: the current year, from `now`.
    let year = meta
        .lines()
        .find_map(|line| line.strip_prefix("created: "))
        .map(|value| value[..4].to_string())
        .expect("a created date");
    assert!(
        year.starts_with("20"),
        "expected an ISO created date, got {year}:\n{meta}"
    );
}

/// Maintenance: the three commands that were command-line only.
#[test]
fn the_maintenance_menu_runs_reindex_recover_and_paths() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .down(6) // → Maintenance
        .enter()
        .enter() // → Reindex
        .pause(900)
        .down(1) // → Check and recover
        .enter()
        .pause(900)
        .down(2) // → Show data locations
        .enter()
        .pause(900)
        .esc() // Maintenance → Settings
        .esc() // → main menu
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "maintenance should return to the menu:\n{out}");
    assert!(
        out.contains("Reindexed") || out.contains("project"),
        "reindex should report what it found:\n{out}"
    );
    assert!(
        out.contains("data dir") || out.contains("templates"),
        "the data locations should be printed:\n{out}"
    );
}

/// The register naming pattern is the one config key the menu could not edit,
/// and `{id}` is the rule that stops two folders renaming onto each other.
#[test]
fn the_register_naming_pattern_is_editable_and_refuses_a_pattern_without_id() {
    let sb = Sandbox::new();
    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .enter() // Project basics
        .down(4) // → Set register naming pattern
        .enter()
        .key("{date}_{name}") // no {id}
        .enter()
        .pause(600)
        .key("_{id}") // corrected in place
        .enter()
        .pause(700)
        .esc()
        .esc()
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(
        code, 0,
        "a refused pattern must not end the session:\n{out}"
    );
    assert!(
        out.contains("must contain {id}"),
        "the rule should be stated at the prompt:\n{out}"
    );
    let shown = sb.ok(&["config", "show"]);
    assert!(
        shown.contains("{date}_{name}_{id}"),
        "the corrected pattern should have been saved:\n{shown}"
    );
}

/// The template builder, end to end through the menu: build one, notice the
/// folder is wrong on the review summary, fix it there, and save.
///
/// New mode used to end at a bare "Save template?" — noticing anything wrong
/// meant answering no and starting the six steps again. This is also the first
/// coverage `template_builder.rs` has ever had.
#[test]
fn the_builder_new_mode_ends_in_the_review_menu() {
    let sb = Sandbox::new();

    let script = pty::Script::new()
        .down(MENU_TEMPLATES)
        .enter()
        .enter() // → Create new template
        // Step 1: metadata
        .line("Demo") // name
        .line("demo") // slug (suggested)
        .line("") // description
        .line("{date}_{id}") // naming pattern
        // Step 2: ID
        .line("D") // prefix
        .line("4") // digits
        // Step 3: variables — none
        .key("n")
        // Step 4: folder structure
        .line("01_Assetz") // deliberately wrong
        .line("")
        // Step 5: files — none
        .key("n")
        .pause(700)
        // Review menu: Folder structure
        .down(3)
        .enter()
        .pause(800)
        // Add / Edit a folder path / Remove / Replace all / Done
        .down(1)
        .enter()
        .pause(800)
        .enter() // "Which folder?" — the only one
        .pause(800)
        .backspace(1)
        .key("s")
        .enter()
        .pause(800)
        .down(4) // → Done
        .enter()
        .pause(900)
        // Save
        .down(5)
        .enter()
        .pause(900)
        .esc()
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "the builder should return to the menu:\n{out}");
    let manifest = sb.install.join("templates/demo/template.yaml");
    let text = fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("no manifest: {e}\n{out}"));
    assert!(
        text.contains("01_Assets"),
        "the folder corrected in the review menu should be what was saved:\n{text}"
    );
    assert!(
        !text.contains("01_Assetz"),
        "the wrong name should be gone, not kept alongside:\n{text}"
    );
}

/// A template file with no contents. `.gitkeep` and every other marker file was
/// unreachable: the content loop only ended on an empty line once at least one
/// line had been typed.
#[test]
fn the_builder_can_declare_an_empty_file() {
    let sb = Sandbox::new();

    let script = pty::Script::new()
        .down(MENU_TEMPLATES)
        .enter()
        .enter() // → Create new template
        .line("Marker")
        .line("marker")
        .line("")
        .line("{date}_{id}")
        .line("M")
        .line("4")
        .key("n") // no variables
        .line("") // no folders
        .key("y") // add a placeholder file
        .line(".gitkeep")
        .pause(500)
        .down(1) // → Empty file
        .enter()
        .pause(500)
        .key("n") // no more files
        .pause(700)
        .down(5) // review → Save
        .enter()
        .pause(900)
        .esc()
        .esc()
        .build();
    let (out, code) = launch(&sb, script);

    assert_eq!(code, 0, "the builder should return to the menu:\n{out}");
    let file = sb.install.join("templates/marker/files/.gitkeep");
    assert!(file.exists(), "the empty file should exist:\n{out}");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "",
        "and it should be empty"
    );
}
