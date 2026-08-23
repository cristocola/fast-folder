//! The project browser: paging, live sizes, filtering, and the action menu.
//!
//! Driven through a real terminal — `harness.rs` states why, and the rules
//! every suite in this binary follows.

use super::common::{self, Sandbox, pty};
use super::harness::*;
use std::fs;

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
