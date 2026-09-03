//! The bridged flows: bulk register, maintenance, and the template builder —
//! reached from the dashboard, run on the main screen, returned from.
//!
//! Driven through a real terminal — `harness.rs` states why, and the rules
//! every suite in this binary follows.

use super::common::{self, Sandbox, pty};
use super::harness::*;
use std::fs;

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
        .key(KEY_REGISTER)
        .pause(600)
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
        .enter() // press Enter to return to fastf
        .pause(500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let out = pty::plain(&out);

    assert_eq!(
        code, 0,
        "bulk register should return to the dashboard:\n{out}"
    );
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
        .key(KEY_REGISTER)
        .pause(600)
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
        .enter() // press Enter to return to fastf
        .pause(500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let out = pty::plain(&out);

    assert_eq!(code, 0, "declining should return to the dashboard:\n{out}");
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
        .key(KEY_REGISTER)
        .pause(600)
        .enter() // One folder
        .line(&folder.display().to_string())
        .pause(400)
        .key("n") // attach a template?
        .key("n") // standardize the name?
        .pause(400)
        .down(1) // created date → Today
        .enter()
        .pause(900)
        .enter() // press Enter to return to fastf
        .pause(500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let out = pty::plain(&out);

    assert_eq!(code, 0, "register should return to the dashboard:\n{out}");
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
        .key(KEY_SETTINGS)
        .pause(600)
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
        .esc() // → the dashboard
        .pause(500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let out = pty::plain(&out);

    assert_eq!(
        code, 0,
        "maintenance should return to the dashboard:\n{out}"
    );
    assert!(
        out.contains("Reindexed") || out.contains("project"),
        "reindex should report what it found:\n{out}"
    );
    assert!(
        out.contains("data dir") || out.contains("templates"),
        "the data locations should be printed:\n{out}"
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
        .key(KEY_TEMPLATES)
        .pause(600)
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
        .esc() // Templates → the dashboard
        .pause(500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let out = pty::plain(&out);

    assert_eq!(
        code, 0,
        "the builder should return to the dashboard:\n{out}"
    );
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
        .key(KEY_TEMPLATES)
        .pause(600)
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
        .esc() // Templates → the dashboard
        .pause(500)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let out = pty::plain(&out);

    assert_eq!(
        code, 0,
        "the builder should return to the dashboard:\n{out}"
    );
    let file = sb.install.join("templates/marker/files/.gitkeep");
    assert!(file.exists(), "the empty file should exist:\n{out}");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "",
        "and it should be empty"
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
