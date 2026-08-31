//! The menu itself: it opens, it contains errors, it can be left.
//!
//! Driven through a real terminal — `harness.rs` states why, and the rules
//! every suite in this binary follows.

use super::common::{self, Sandbox, pty};
use super::harness::*;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

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

/// Settings held a loaded `Config` across the prompt and then wrote the whole
/// `bases` list back from that stale copy, silently reverting anything the
/// another `fastf config set` had written meanwhile. The rule is
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

/// The register naming pattern is the one config key the menu could not edit,
/// and `{id}` is the rule that stops two folders renaming onto each other.
#[test]
fn the_register_naming_pattern_is_editable_and_refuses_a_pattern_without_id() {
    let sb = Sandbox::new();
    let script = pty::Script::new()
        .down(MENU_SETTINGS)
        .enter()
        .enter() // Project basics
        // base dir, default template, date format, editor, terminal, pattern.
        .down(5) // → Set register naming pattern
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
