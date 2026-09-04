//! The flows that build something — create, register, apply, the template
//! builder — and the command line's own prompts, driven through the runtime.
//!
//! Driven through a real terminal — `harness.rs` states why, and the rules
//! every suite in this binary follows.

use super::common::{self, Sandbox, pty};
use super::harness::*;
use std::fs;

/// Bulk register: the preview first, then the commit, both in the app.
///
/// `--recursive` was command-line only, so onboarding a folder of legacy
/// projects meant leaving the tool the whole flow was designed for.
#[test]
fn the_app_can_register_a_whole_base_after_previewing_it() {
    let sb = Sandbox::new();
    // Inside the configured base: registration refuses a folder that is not a
    // direct child of one, which is what keeps a registered project findable.
    let legacy = sb.base.clone();
    for name in ["One", "Two"] {
        fs::create_dir_all(legacy.join(name)).unwrap();
    }

    let script = pty::Script::new()
        .key(KEY_REGISTER)
        .pause(600)
        .right(1) // Register → every unregistered folder in a base
        .tab()
        .key(&legacy.display().to_string())
        .enter() // → the preview
        .pause(900)
        .enter() // → commit
        .pause(1500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(
        code, 0,
        "bulk register should return to the dashboard:\n{screen}"
    );
    assert!(
        legacy.join("One/PROJECT_INFO.md").exists() && legacy.join("Two/PROJECT_INFO.md").exists(),
        "both folders should have been registered:\n{screen}"
    );
    assert!(
        pty::plain(&out).contains("would be registered"),
        "the preview should have been shown before anything was written:\n{}",
        pty::plain(&out)
    );
}

/// Esc at the preview goes back to the answers, and Esc again abandons the
/// whole thing — the app's Esc ladder, one step at a time, nothing typed lost.
#[test]
fn escaping_the_bulk_register_preview_writes_nothing() {
    let sb = Sandbox::new();
    let legacy = sb.base.clone();
    fs::create_dir_all(legacy.join("One")).unwrap();

    let script = pty::Script::new()
        .key(KEY_REGISTER)
        .pause(600)
        .right(1)
        .tab()
        .key(&legacy.display().to_string())
        .enter() // → the preview
        .pause(900)
        .esc() // → back to the answers, with the path still typed
        .pause(400)
        .esc() // → cancelled
        .pause(500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(
        code, 0,
        "escaping should return to the dashboard:\n{screen}"
    );
    assert!(
        !legacy.join("One/PROJECT_INFO.md").exists(),
        "an abandoned preview must write nothing:\n{screen}"
    );
    assert!(
        pty::plain(&out).contains("nothing was registered"),
        "the cancel should say so:\n{}",
        pty::plain(&out)
    );
}

/// Registering with "today" as the created date, which the old menu could not
/// say at all.
#[test]
fn the_app_can_register_with_todays_date() {
    let sb = Sandbox::new();
    let folder = sb.base.join("Legacy");
    fs::create_dir_all(&folder).unwrap();

    let script = pty::Script::new()
        .key(KEY_REGISTER)
        .pause(600)
        .tab() // → Folder
        .key(&folder.display().to_string())
        .tab() // → Template
        .tab() // → Standardize name
        .tab() // → Created
        .right(1) // → today
        .enter() // → the preview
        .pause(900)
        .enter() // → commit
        .pause(1200)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(
        code, 0,
        "register should return to the dashboard:\n{screen}"
    );
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

/// The create wizard end to end: the answers, the plan, the folder — and the
/// new project selected in the list it comes back to.
#[test]
fn the_wizard_creates_a_project_and_selects_it() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["config", "set", "default-template", "race"]);

    let script = pty::Script::new()
        .key(KEY_CREATE)
        .pause(800)
        .tab() // → the template's first variable
        .key("Lullaby")
        .enter() // → the preview
        .pause(1000)
        .enter() // → create
        .pause(1800)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "the wizard should return cleanly:\n{screen}");
    let created = common::project_dirs(&sb.base);
    assert_eq!(created.len(), 1, "exactly one project:\n{screen}");
    let name = created[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        name.contains("Lullaby"),
        "the answer should be in the folder name, got {name}"
    );
    assert!(
        screen.contains("Created"),
        "the app should say what it made:\n{screen}"
    );
    assert!(
        screen.contains(&name),
        "the new project should be in the list:\n{screen}"
    );
}

/// Apply asked template, target, dry-run and every variable, then rejected the
/// target. The target is a field now, and it is checked where it was typed.
#[test]
fn apply_refuses_a_missing_target_where_it_was_typed() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["config", "set", "default-template", "race"]);

    let script = pty::Script::new()
        .key("E")
        .pause(700)
        .tab() // → Target folder
        .key("/nope/does/not/exist")
        .tab() // → the template's own variable, answered so it is not what refuses
        .key("Anything")
        .enter()
        .pause(900)
        .build();
    let (out, _) = launch(&sb, script);
    let screen = app_screen(&out);

    assert!(
        screen.contains("/nope/does/not/exist"),
        "the text must stay on the line to be corrected:\n{screen}"
    );
    assert!(
        screen.contains("no such folder"),
        "the target should be refused at its own field:\n{screen}"
    );
}

/// Apply, for real: an existing folder gains the template's missing folders.
#[test]
fn apply_fills_in_a_folder_from_the_preview() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["config", "set", "default-template", "race"]);
    let target = sb.base.join("Existing");
    fs::create_dir_all(&target).unwrap();

    let script = pty::Script::new()
        .key("E")
        .pause(700)
        .tab()
        .key(&target.display().to_string())
        .tab()
        .key("Anything")
        .enter() // → the preview
        .pause(1000)
        .enter() // → apply
        .pause(1200)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "apply should return to the dashboard:\n{screen}");
    assert!(
        target.join("README.md").is_file(),
        "the template's file should have been created:\n{screen}"
    );
}

/// Maintenance: the three commands the menu could only reach by leaving it —
/// reindex, check and recover, and where fastf keeps its things.
#[test]
fn maintenance_runs_reindex_recover_and_data_locations() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    let script = pty::Script::new()
        .key(KEY_SETTINGS)
        .pause(900)
        // The settings list has 22 selectable rows; Reindex is the twentieth.
        .down(19) // → Reindex
        .enter()
        .pause(1200)
        .down(1) // → Check and recover
        .enter()
        .pause(1200)
        .down(1) // → Data locations
        .enter()
        .pause(1000)
        .build();
    let (out, _) = launch(&sb, script);
    let screen = app_screen(&out);
    let text = pty::plain(&out);

    assert!(
        text.contains("Reindexed"),
        "reindex should report what it found:\n{text}"
    );
    assert!(
        text.contains("Nothing to reconcile") || text.contains("Reconciled"),
        "recovery should say what it did:\n{text}"
    );
    assert!(
        screen.contains("Templates") && screen.contains("Data dir"),
        "the data locations should be on screen:\n{screen}"
    );
}

/// The template builder, end to end through the menu: build one, notice the
/// folder is wrong on the review summary, fix it there, and save.
///
/// New mode used to end at a bare "Save template?" — noticing anything wrong
/// meant answering no and starting the six steps again. This is also the first
/// The builder is one list of sections, entered in any order, and Save says
/// what `Template::validate` refuses. The six-step linear pass is gone: it
/// made noticing a wrong folder name on the summary mean starting again.
#[test]
fn the_builder_saves_a_template_built_section_by_section() {
    let sb = Sandbox::new();

    let script = pty::Script::new()
        .key(KEY_TEMPLATES)
        .pause(700)
        .key("n") // → a new template
        .pause(400)
        .enter() // → Metadata
        .pause(300)
        .key("Demo") // the name, and the slug follows it
        .enter() // → back to the sections
        .pause(300)
        .down(3) // → Structure
        .enter()
        .pause(300)
        .key("01_Assetz") // deliberately wrong
        .backspace(1)
        .key("s") // corrected in place, before it is ever written
        .key("\x13") // Ctrl-S → keep
        .pause(300)
        .down(2) // → Save
        .enter()
        .pause(1200)
        .esc() // the studio → the dashboard
        .pause(400)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "the builder should return cleanly:\n{screen}");
    let manifest = sb.install.join("templates/demo/template.yaml");
    let text =
        fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("no manifest: {e}\n{screen}"));
    assert!(
        text.contains("01_Assets") && !text.contains("01_Assetz"),
        "the corrected folder is what was saved:\n{text}"
    );
    assert!(text.contains("Demo"), "the name is what was typed:\n{text}");
}

/// Save refuses an incomplete template and says so where the reader is, rather
/// than writing something `Template::validate` would reject on load.
#[test]
fn the_builder_refuses_to_save_a_template_that_would_not_load() {
    let sb = Sandbox::new();

    let script = pty::Script::new()
        .key(KEY_TEMPLATES)
        .pause(700)
        .key("n")
        .pause(400)
        .down(5) // → Save, with nothing filled in
        .enter()
        .pause(600)
        .build();
    let (out, _) = launch(&sb, script);
    let screen = app_screen(&out);

    assert!(
        screen.contains("Cannot save:"),
        "an invalid template must be refused, not written:\n{screen}"
    );
    assert!(
        !sb.install.join("templates/template.yaml").exists(),
        "nothing was written"
    );
}

/// A marker file: a path and no contents at all. The old builder could not
/// declare one — its content loop only ended once a line had been typed.
#[test]
fn the_builder_can_declare_an_empty_file() {
    let sb = Sandbox::new();

    let script = pty::Script::new()
        .key(KEY_TEMPLATES)
        .pause(700)
        .key("n")
        .pause(400)
        .enter() // → Metadata
        .pause(300)
        .key("Marker")
        .enter()
        .pause(300)
        .down(4) // → Files
        .enter()
        .pause(300)
        .key("a") // → a new file
        .pause(300)
        .key(".gitkeep")
        .key("\x13") // Ctrl-S → keep, with no contents
        .pause(400)
        .esc() // → the sections, back on Files
        .pause(300)
        .down(1) // → Save
        .enter()
        .pause(1200)
        .esc()
        .pause(400)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "the builder should return cleanly:\n{screen}");
    let file = sb.install.join("templates/marker/files/.gitkeep");
    assert!(file.exists(), "the empty file should exist:\n{screen}");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "",
        "and it should be empty"
    );
}

/// The studio deletes a template, and asks first.
#[test]
fn deleting_a_template_asks_first() {
    let sb = Sandbox::new();
    sb.write_template("doomed");

    let script = pty::Script::new()
        .key(KEY_TEMPLATES)
        .pause(700)
        .key("D") // → the confirmation
        .pause(500)
        .key("n") // → no
        .pause(500)
        .key("D")
        .pause(500)
        .key("y") // → yes
        .pause(900)
        .esc()
        .pause(400)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);
    let text = pty::plain(&out);

    assert_eq!(code, 0, "deleting should return cleanly:\n{screen}");
    assert!(
        text.contains("Deleted template"),
        "the second answer should have deleted one:\n{text}"
    );
    // `n` answered the first confirmation, so the first `D` deleted nothing:
    // three templates went in and exactly one came out.
    let left = fs::read_dir(sb.install.join("templates"))
        .unwrap()
        .flatten()
        .count();
    assert_eq!(
        left, 2,
        "the refused delete must not have run as well:\n{screen}"
    );
}

// ---------------------------------------------------------------------------
// The ambiguity picker
// ---------------------------------------------------------------------------

/// Driven through `fastf path`, never `fastf open`: `open` ends in
/// `reveal_folder`, which would spawn the real file manager on whatever machine
/// runs the suite. `path` takes the identical route through
/// `cli::target::one_project` and stops at a printed line.
///
/// Stdout goes to a file while the picker draws on the pty, which is the shape
/// `cd "$(fastf path lullaby)"` has: a terminal is right there and only the
/// output is redirected. Both halves are asserted, because the whole point is
/// that the picker never contaminates stdout.
#[test]
fn an_ambiguous_path_query_opens_a_picker_and_prints_the_choice() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "shared_one", "ID0011");
    let newer = sb.plant_project(&sb.base, "shared_two", "ID0012");
    // Both fixtures carry the same creation date, and rows are ordered
    // newest-first — so without this the tie is broken by directory order and
    // "one Down" lands on whichever the filesystem felt like listing second.
    let pinfo = newer.join("PROJECT_INFO.md");
    let text = fs::read_to_string(&pinfo).unwrap().replace(
        "created: 2026-01-01T00:00:00Z",
        "created: 2026-02-01T00:00:00Z",
    );
    fs::write(&pinfo, text).unwrap();

    let captured = sb.tmp.path().join("chosen.txt");
    // shared_two heads the list; one Down selects shared_one.
    let script = pty::Script::new().down(1).enter().pause(600).build();
    let (out, code) = pty::run_stdout_to(
        common::FASTF,
        &["path", "shared"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &script,
        DEADLINE,
        &captured,
    );

    assert_eq!(code, 0, "the picker should have resolved the query:\n{out}");
    assert!(
        out.contains("Which project's path?") && out.contains("ID0011") && out.contains("ID0012"),
        "the picker should offer both candidates:\n{out}"
    );
    assert!(
        !out.contains("is ambiguous"),
        "a terminal gets the picker, not the error:\n{out}"
    );

    let printed = fs::read_to_string(&captured).unwrap();
    let chosen = common::shown_path(&sb.base.join("shared_one"));
    assert_eq!(
        printed,
        format!("{chosen}\n"),
        "stdout must carry the chosen path and nothing the picker drew"
    );
}

/// Esc is the one cancel key everywhere else in fastf, and declining to choose
/// is not a failure: it says so and exits 0, so `fastf copy x && something`
/// does not treat "I changed my mind" as an error.
#[test]
fn esc_on_the_ambiguity_picker_cancels_with_exit_0_and_says_so() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "shared_one", "ID0011");
    sb.plant_project(&sb.base, "shared_two", "ID0012");

    let script = pty::Script::new().esc().pause(600).build();
    let (out, code) = pty::run(
        common::FASTF,
        &["path", "shared"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &script,
        DEADLINE,
    );

    assert_eq!(code, 0, "cancelling is not a failure:\n{out}");
    assert!(
        out.contains("Cancelled") && out.contains("no path printed"),
        "cancelling should say what did not happen:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// The relaunched window
// ---------------------------------------------------------------------------

/// A window fastf opened for itself must not close on the last line of output:
/// the text would flash past exactly as it does in the journal, which is the
/// problem the whole mechanism exists to solve.
///
/// `FASTF_RELAUNCHED` is what the relaunch sets on the child, so setting it here
/// *is* being that child — there is no window to open on a CI runner.
#[test]
fn a_relaunched_run_with_nothing_interactive_waits_for_enter() {
    let sb = Sandbox::new();

    // Nothing to browse, so `recent` prints one line and is done: no prompt, no
    // picker, nothing that waited for the user.
    let script = pty::Script::new().pause(600).key("\r").build();
    let (out, code) = pty::run(
        common::FASTF,
        &["recent"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("FASTF_RELAUNCHED", std::path::Path::new("1")),
        ],
        &script,
        DEADLINE,
    );

    assert_eq!(code, 0, "the pause is not a failure:\n{out}");
    assert!(
        out.contains("press Enter to close"),
        "a relaunched window that only printed must hold itself open:\n{out}"
    );
}

/// The other half: a window that ran a picker or a menu already had the user's
/// attention for as long as they wanted it, so demanding one more keypress from
/// somebody who has just pressed a key is noise.
#[test]
fn a_relaunched_run_that_showed_a_picker_does_not_wait() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");

    // With a project to show, `recent` opens the dashboard — an interactive
    // surface. Esc leaves it, and that must be the end of the process.
    let script = pty::Script::new().pause(1200).esc().pause(600).build();
    let (out, code) = pty::run(
        common::FASTF,
        &["recent"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("FASTF_RELAUNCHED", std::path::Path::new("1")),
        ],
        &script,
        DEADLINE,
    );

    assert_eq!(code, 0, "leaving the browser is not a failure:\n{out}");
    assert!(
        !out.contains("press Enter to close"),
        "a window that already waited for the user must not wait again:\n{out}"
    );
}

/// First run: the app asks where projects should live, creates the folder and
/// records it — before it draws anything else, because there is nothing else
/// to draw.
#[test]
fn first_run_asks_for_a_base_and_creates_it() {
    let sb = Sandbox::unconfigured();
    let wanted = sb.tmp.path().join("Projects");

    let script = pty::Script::new()
        .pause(700)
        .key("\x15") // clear the suggestion
        .key(&wanted.display().to_string())
        .enter()
        .pause(1500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "the first run should end cleanly:\n{screen}");
    assert!(
        wanted.is_dir(),
        "the folder should have been created:\n{screen}"
    );
    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    assert!(
        config.contains(&wanted.display().to_string()),
        "the base should have been recorded:\n{config}"
    );
}

/// Skipping leaves the configuration alone, and the question comes back.
#[test]
fn first_run_can_be_skipped_and_writes_nothing() {
    let sb = Sandbox::unconfigured();

    let script = pty::Script::new()
        .pause(700)
        .esc() // skip
        .pause(600)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "skipping is not a failure:\n{screen}");
    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap_or_default();
    assert!(
        !config.contains("base_dir = \"/"),
        "skipping must write no base:\n{config}"
    );
}

/// The ID counter is raised from the settings screen, and never lowered.
#[test]
fn the_counter_is_raised_from_the_settings_screen() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0007", "ID0007");

    let script = pty::Script::new()
        .key(KEY_SETTINGS)
        .pause(900)
        .down(17) // → Counter
        .enter() // → the number
        .pause(500)
        .key("\x15")
        .key("3") // below the floor: refused
        .enter()
        .pause(900)
        // The cursor never left the Counter row, so the second try is one key.
        .enter()
        .pause(400)
        .key("\x15")
        .key("40")
        .enter()
        .pause(1200)
        .esc()
        .pause(400)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);
    let text = pty::plain(&out);

    assert_eq!(code, 0, "a refused number is not a failure:\n{screen}");
    assert!(
        text.contains("cannot") || text.contains("below"),
        "lowering the counter must be refused and say why:\n{text}"
    );
    let shown = sb.ok(&["id", "show"]);
    assert!(
        shown.contains("40"),
        "the raise should have taken:\n{shown}"
    );
}

// ---------------------------------------------------------------------------
// The command line's own prompts, on the same ratatui
// ---------------------------------------------------------------------------

/// Run one subcommand under a pty with `script`, and give back the transcript.
fn run_cli(sb: &Sandbox, args: &[&str], script: Vec<pty::Keystroke>) -> (String, i32) {
    pty::run(
        common::FASTF,
        args,
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &script,
        DEADLINE,
    )
}

/// `q` cancels the ambiguity picker, the same as Esc. Both were dialoguer's
/// contract and both are kept: the picker interrupted a verb, and getting out
/// of it must not need a key anyone has to look up.
#[test]
fn q_cancels_the_ambiguity_picker_like_esc() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "shared_one", "ID0011");
    sb.plant_project(&sb.base, "shared_two", "ID0012");

    let script = pty::Script::new().key("q").pause(600).build();
    let (out, code) = run_cli(&sb, &["path", "shared"], script);

    assert_eq!(code, 0, "cancelling is not a failure:\n{out}");
    assert!(
        pty::plain(&out).contains("no path printed"),
        "cancelling should say what did not happen:\n{}",
        pty::plain(&out)
    );
}

/// A yes/no answers on the keypress, with no Enter. That is what makes
/// `fastf new`'s confirmation one keystroke, and a trailing `\r` would survive
/// into whatever asked next.
#[test]
fn a_command_line_confirm_answers_a_bare_y() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let script = pty::Script::new()
        .pause(500)
        .key("Lullaby")
        .enter() // the template's one variable
        .pause(900)
        .key("y") // "Create this project?" — no Enter
        .pause(1500)
        .key("n") // "Open project folder?" — likewise
        .pause(900)
        .build();
    let (out, code) = run_cli(&sb, &["new", "race"], script);
    let text = pty::plain(&out);

    assert_eq!(code, 0, "the create should have finished:\n{text}");
    let created = common::project_dirs(&sb.base);
    assert_eq!(created.len(), 1, "exactly one project:\n{text}");
    assert!(
        created[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("Lullaby"),
        "the answer should be in the folder name:\n{text}"
    );
}

/// A command-line text prompt shows where you are typing.
///
/// The same guarantee the app's fields have, on the other surface: the caret is
/// parked in the line being edited, at the cursor's offset *within the visible
/// window*, not at the start of the prompt.
#[test]
fn a_command_line_text_prompt_parks_a_visible_caret_after_the_text() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let script = pty::Script::new()
        .pause(600)
        .key("Lulla")
        .pause(700)
        .build();
    let (out, _) = run_cli(&sb, &["new", "race"], script);
    let screen = app_screen(&out);
    let (row, column) = app_cursor(&out);

    let (text_row, text_line) = screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("Lulla"))
        .map(|(index, line)| (index as u16, line.to_string()))
        .unwrap_or_else(|| panic!("the typed text never drew:\n{screen}"));
    let at = text_line.find("Lulla").expect("just found");
    let after_text = (text_line[..at].chars().count() + "Lulla".chars().count()) as u16;

    assert_eq!(
        row, text_row,
        "the caret belongs on the line being edited:\n{screen}"
    );
    assert_eq!(
        column, after_text,
        "the caret belongs after the text, not at the start of the prompt:\n{screen}"
    );
}

/// A refused answer says why, under the line, and leaves the prompt where it
/// was — the difference between "correct this" and "start again".
#[test]
fn a_command_line_prompt_refuses_an_empty_required_answer_in_place() {
    let sb = Sandbox::new();
    sb.write_template("race"); // its one variable is required

    let script = pty::Script::new()
        .pause(600)
        .enter() // submit it empty
        .pause(700)
        .key("Lullaby")
        .pause(500)
        .build();
    let (out, _) = run_cli(&sb, &["new", "race"], script);
    let screen = app_screen(&out);

    assert!(
        pty::plain(&out).contains("a value is required"),
        "the refusal belongs under the line:\n{}",
        pty::plain(&out)
    );
    // And typing clears it: the last frame is the prompt with the answer on it,
    // not a stale complaint about the answer before it.
    assert!(
        screen.contains("Lullaby") && !screen.contains("a value is required"),
        "the prompt is still there to answer, and the refusal is gone:\n{screen}"
    );
}
