//! The guided app's frames, rendered through ratatui's test backend and
//! compared against the snapshots under `tests/snapshots/`.
//!
//! Every frame is drawn in the mono theme at a fixed size from a fixture with
//! fixed dates and placeholder paths, so a snapshot depends on the code and
//! nothing else. Colour is not part of a snapshot — the test backend records
//! symbols — which is why the layout is what these guard: what is on screen,
//! where, at 80×24 and at 120×40.
//!
//! When a frame changes on purpose, review the new snapshot with
//! `INSTA_UPDATE=always cargo test --test tui_snapshots` (or `cargo insta
//! review`) and commit it.

use fastf::tui::app::update;
use fastf::tui::command::Key;
use fastf::tui::msg::Msg;
use fastf::tui::testing::{empty_fixture, fixture, render_to_string, sample_projects};
use ratatui::crossterm::event::KeyCode;

fn snap(name: &str, frame: String) {
    insta::assert_snapshot!(name, frame);
}

#[test]
fn dashboard_80x24() {
    let app = fixture(12, 80, 24);
    snap("dashboard_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn dashboard_120x40() {
    let mut app = fixture(12, 120, 40);
    let _ = update(
        &mut app,
        Msg::Sizes(vec![(sample_projects(1)[0].path.clone(), Some(3_355_443))]),
    );
    snap("dashboard_120x40", render_to_string(&app, 120, 40));
}

#[test]
fn too_small_40x10() {
    let app = fixture(3, 40, 10);
    snap("too_small_40x10", render_to_string(&app, 40, 10));
}

#[test]
fn loading_before_discovery_answers() {
    let app = empty_fixture(80, 24);
    snap("loading_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn palette_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('c')));
    for c in "open".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    snap("palette_open", render_to_string(&app, 100, 30));
}

#[test]
fn help_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('?')));
    snap("help_open", render_to_string(&app, 100, 30));
}

#[test]
fn search_with_a_fuzzy_term_and_a_tag() {
    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Key(Key::ch('/')));
    for c in "tag:draft lulla".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    snap("search_fuzzy_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn sort_picker_open() {
    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Key(Key::ch('S')));
    snap("sort_picker_80x24", render_to_string(&app, 80, 24));
}

/// The regression the column order exists for: at 80 columns the folder name
/// is still there in full, whatever else had to go.
#[test]
fn narrow_keeps_the_folder_name() {
    let app = fixture(6, 80, 24);
    let frame = render_to_string(&app, 80, 24);
    for project in sample_projects(6) {
        assert!(
            frame.contains(&project.name),
            "the folder name must survive an 80-column window:\n{frame}"
        );
    }
    assert!(!frame.contains("TAGS"), "tags are the first column to go");
    let _ = KeyCode::Null;
}
