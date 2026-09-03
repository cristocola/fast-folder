//! The project list: order, live sizes, the search bar, the bridged action
//! menu, and what a mutation does to the rows.
//!
//! Driven through a real terminal — `harness.rs` states why, and the rules
//! every suite in this binary follows.

use super::common::{self, Sandbox, pty};
use super::harness::*;
use std::fs;

/// The list is drawn newest first, before a single folder has been walked, and
/// the sizes fill in afterwards.
#[test]
fn the_list_is_newest_first_and_sizes_fill_in() {
    let sb = Sandbox::new();
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

    let script = pty::Script::new().pause(2500).key(KEY_QUIT).build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "the app should quit cleanly:\n{screen}");
    assert!(
        pty::plain(&out).contains("scanning…"),
        "the list should draw before any size is known:\n{out}"
    );
    // The payloads plus a `PROJECT_INFO.md` each: a few kilobytes, measured.
    assert_eq!(
        screen.matches(" KB").count(),
        3,
        "every row should have been measured by now:\n{screen}"
    );
    assert!(
        !screen.contains("scanning…"),
        "no row should still be pending:\n{screen}"
    );

    let newest = screen.find("Newest_Project").expect("newest project row");
    let middle = screen.find("Middle_Project").expect("middle project row");
    let oldest = screen.find("Oldest_Project").expect("oldest project row");
    assert!(
        newest < middle && middle < oldest,
        "projects were not shown newest first:\n{screen}"
    );
    assert!(pty::plain(&out).contains("Goodbye."));
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
    let root = plant_dated_project(&sb, "Mut", "ID0001", "2026-01-01T00:00:00Z", 256);
    let trace = sb.tmp.path().join("trace");

    let script = pty::Script::new()
        .pause(800)
        .enter() // the selected project's action menu
        .pause(600)
        .down(4) // → Tags
        .enter()
        .enter() // → Add a tag (no known tags yet, so it asks for one)
        .line("draft")
        .pause(700)
        .esc() // Back to list → the dashboard
        .pause(900)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch_traced(&sb, script, &trace);
    let text = pty::plain(&out);

    assert_eq!(
        code, 0,
        "a mutation should return safely to the dashboard:\n{text}"
    );
    assert!(
        text.contains("Added 1 tag"),
        "tag action did not complete:\n{text}"
    );
    let added = text.find("Added 1 tag").unwrap();
    assert!(
        text[added..].contains("draft"),
        "the patched row should show the new tag once the dashboard is back:\n{text}"
    );
    assert!(
        fs::read_to_string(root.join("PROJECT_INFO.md"))
            .unwrap()
            .contains("draft")
    );
    assert_eq!(
        traced(&trace, "discover"),
        1,
        "the app opened once and must not have rescanned the library to add a tag"
    );
    assert!(text.contains("Goodbye."));
}

/// Deleting a project takes its row out of the list, also without a rescan.
#[cfg(debug_assertions)]
#[test]
fn a_delete_drops_its_row_without_rescanning_the_library() {
    let sb = Sandbox::new();
    plant_dated_project(&sb, "Doomed_Project", "ID0002", "2026-02-02T00:00:00Z", 128);
    plant_dated_project(&sb, "Kept_Project", "ID0001", "2026-01-01T00:00:00Z", 128);
    let trace = sb.tmp.path().join("trace");

    let script = pty::Script::new()
        .pause(800)
        .enter() // the newest row: Doomed_Project
        .pause(600)
        .down(8) // → Delete folder permanently
        .enter()
        .line("Doomed_Project") // typed confirmation
        .pause(1200)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch_traced(&sb, script, &trace);
    let text = pty::plain(&out);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "a delete should return to the dashboard:\n{text}");
    assert!(text.contains("Deleted"), "the delete did not run:\n{text}");
    assert!(
        !sb.base.join("Doomed_Project").exists(),
        "the folder should be gone:\n{text}"
    );
    assert!(
        screen.contains("1 of 1 projects") && !screen.contains("Doomed_Project"),
        "the list should have shrunk to the one project left:\n{screen}"
    );
    assert_eq!(
        traced(&trace, "discover"),
        1,
        "dropping a row must not rescan the library"
    );
}

/// The headline behaviour: the list is drawn before a single folder has been
/// walked, and the size arrives on its own while nothing is being typed. The
/// only key in this script is the `q` that ends it, so a size in the transcript
/// can only have been drawn by a repaint the app drove itself.
#[test]
fn sizes_land_without_any_input() {
    let sb = Sandbox::new();
    // Three whole mebibytes, so the rendered figure is stable: PROJECT_INFO.md
    // adds a few hundred bytes, which cannot move a one-decimal MB reading.
    plant_dated_project(
        &sb,
        "Measured_Project",
        "ID0001",
        "2026-01-01T00:00:00Z",
        3 * 1024 * 1024,
    );

    let script = pty::Script::new().pause(2500).key(KEY_QUIT).build();
    let (out, code) = launch(&sb, script);
    let text = pty::plain(&out);

    assert_eq!(code, 0, "the app should quit cleanly:\n{text}");
    let pending = text
        .find("scanning…")
        .expect("the list must draw before the folder has been walked");
    let measured = text
        .find("3.0 MB")
        .expect("the size never reached the list — the repaint is not live");
    assert!(
        pending < measured,
        "the size must land after the list was drawn:\n{text}"
    );
    assert!(text.contains("Goodbye."));
}

/// Esc in the bridged action menu returns to the dashboard, not the shell.
#[test]
fn esc_walks_back_out_of_the_action_menu() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    let script = pty::Script::new()
        .pause(800)
        .enter() // open the row's action menu
        .pause(600)
        .esc() // → back to the dashboard
        .pause(600)
        .esc() // → quit
        .build();
    let (out, code) = launch(&sb, script);
    let text = pty::plain(&out);

    assert_eq!(code, 0, "Esc must not end the session:\n{text}");
    assert!(
        text.contains("What would you like to do?"),
        "the action menu should have opened:\n{text}"
    );
    assert!(
        text.contains("Goodbye."),
        "one Esc back, one Esc out:\n{text}"
    );
}

/// A search that matches nothing keeps the query in the bar, one keystroke
/// from being fixed — and says so instead of showing an empty list.
#[test]
fn a_search_that_matches_nothing_keeps_the_query() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    let script = pty::Script::new()
        .pause(800)
        .key(KEY_SEARCH)
        .key("Alphaz") // one letter no name has
        .pause(700)
        // The query is still in the bar: one Backspace makes it match.
        .backspace(1)
        .pause(700)
        .enter() // keep the query, leave the bar
        .pause(400)
        .esc() // clears the query
        .esc() // quits
        .build();
    let (out, code) = launch(&sb, script);
    let text = pty::plain(&out);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "a search miss must not end the session:\n{text}");
    assert!(
        text.contains("no matches"),
        "the miss should be reported in the status line:\n{text}"
    );
    assert!(
        screen.contains("2026-01-01_Alpha_ID0001"),
        "the corrected query should have brought the row back:\n{screen}"
    );
    assert!(text.contains("Goodbye."));
}

/// The header says what the library looks like, and it costs no scan.
///
/// The counts come from each base's own `.fastf-index.json`, so opening the
/// app does not get slower the more it has to say. A base that is configured
/// but not mounted is named rather than silently dropped. The one discovery
/// the list needs reads the same fresh index.
#[cfg(debug_assertions)]
#[test]
fn the_header_reports_the_library_from_the_index_without_scanning() {
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
    let (out, code) = launch_traced(&sb, pty::Script::new().pause(1500).esc().build(), &trace);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "the app should open and quit:\n{screen}");
    assert!(
        screen.contains("2 projects"),
        "the header should report the count:\n{screen}"
    );
    assert!(
        screen.contains("highest ID0002"),
        "the header should report the highest ID:\n{screen}"
    );
    assert!(
        screen.contains("not mounted"),
        "a configured base that is gone should be named:\n{screen}"
    );
    assert_eq!(
        traced(&trace, "scan_base"),
        0,
        "opening the app over a fresh index must not scan a single base"
    );
    assert_eq!(traced(&trace, "discover"), 1);
}

/// The search bar narrows the list, and Enter on what is left opens it.
#[test]
fn the_search_narrows_to_one_row() {
    let sb = Sandbox::new();
    plant_dated_project(&sb, "Aardvark", "ID0001", "2026-01-01T00:00:00Z", 64);
    plant_dated_project(&sb, "Buffalo", "ID0002", "2026-02-02T00:00:00Z", 64);
    plant_dated_project(&sb, "Capybara", "ID0003", "2026-03-03T00:00:00Z", 64);

    let script = pty::Script::new()
        .pause(800)
        .key(KEY_SEARCH)
        .key("buff") // lower case: matching ignores case
        .pause(600)
        .enter() // keep the query, leave the bar
        .enter() // opens the only row left
        .pause(600)
        .esc() // action menu → the dashboard, the query still set
        .pause(600)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let text = pty::plain(&out);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "searching should not end the session:\n{text}");
    // The action menu is printed by the bridged flow, on the main screen.
    let opened = text
        .find("What would you like to do?")
        .unwrap_or_else(|| panic!("an action menu should have opened:\n{text}"));
    assert!(
        text[..opened].contains("Buffalo"),
        "Enter should have opened the matching row:\n{text}"
    );
    // Back on the dashboard, the query is still narrowing the list.
    assert!(
        screen.contains("1/3") && screen.contains("Buffalo"),
        "one of three rows should be left:\n{screen}"
    );
    // Table rows only: the header's `newest` line names Capybara regardless.
    let rows: Vec<&str> = screen.lines().filter(|l| l.starts_with('│')).collect();
    assert!(
        !rows
            .iter()
            .any(|l| l.contains("Aardvark") || l.contains("Capybara")),
        "the rows that do not match should be gone:\n{screen}"
    );
}

/// PageDown moves the highlight by a viewport rather than a row.
#[test]
fn page_keys_move_by_a_viewport() {
    let sb = Sandbox::new();
    // Forty projects with distinct dates, newest first: P40 heads the list and
    // P01 ends it, well beyond the rows a 40-line terminal shows.
    for n in 1..=40 {
        let (month, day) = if n <= 28 { (1, n) } else { (2, n - 28) };
        plant_dated_project(
            &sb,
            &format!("P{n:02}"),
            &format!("ID{n:04}"),
            &format!("2026-{month:02}-{day:02}T00:00:00Z"),
            32,
        );
    }

    let script = pty::Script::new()
        .pause(1200)
        .page_down()
        .page_down()
        .pause(600)
        .key(KEY_QUIT)
        .build();
    let (out, code) = launch(&sb, script);
    let screen = app_screen(&out);

    assert_eq!(code, 0, "page keys should not end the session:\n{screen}");
    assert!(
        !screen.contains("P40"),
        "the first screen's rows should have scrolled away:\n{screen}"
    );
    assert!(
        screen.contains("P01"),
        "two PageDowns reach the end of a forty-row list, which the first \
         screen cannot show:\n{screen}"
    );
    let selected = screen
        .lines()
        .find(|line| line.contains("▸ ID0001"))
        .unwrap_or_else(|| panic!("the last row should be selected:\n{screen}"));
    assert!(selected.contains("P01"));
}

/// `fastf recent` on a terminal opens the same app the bare command opens.
#[test]
fn fastf_recent_opens_the_same_app() {
    let sb = Sandbox::new();
    plant_dated_project(&sb, "Solo", "ID0001", "2026-01-01T00:00:00Z", 4096);

    let script = pty::Script::new().pause(1500).key(KEY_QUIT).build();
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
    let screen = app_screen(&out);

    assert_eq!(code, 0, "`fastf recent` should exit cleanly:\n{screen}");
    assert!(
        screen.contains("Solo") && screen.contains(" KB"),
        "`fastf recent` should show the dashboard with sizes:\n{screen}"
    );
}

/// Copy path always says what it did. With no clipboard tool on PATH it shows
/// the path instead — a Copy that silently does nothing is the worst version.
#[test]
fn copy_path_falls_back_to_showing_the_path() {
    let sb = Sandbox::new();
    let root = plant_dated_project(&sb, "Copied", "ID0001", "2026-01-01T00:00:00Z", 64);
    // An empty PATH: no wl-copy, no xclip, no pbcopy.
    let empty = sb.tmp.path().join("empty-path");
    fs::create_dir_all(&empty).unwrap();

    let script = pty::Script::new()
        .pause(800)
        .key(KEY_COPY_PATH)
        .pause(800)
        .esc() // close the dialog
        .pause(400)
        .key(KEY_QUIT)
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
    let text = pty::plain(&out);

    assert_eq!(code, 0, "Copy path should not end the session:\n{text}");
    assert!(
        text.contains("no clipboard tool found"),
        "with nothing on PATH it should say so:\n{text}"
    );
    assert!(
        text.contains(&root.display().to_string()),
        "and show the path it could not copy:\n{text}"
    );
}
