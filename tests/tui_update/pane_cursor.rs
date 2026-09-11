//! The detail pane's own cursor: it rests only on rows Enter can act on, it
//! stops at the ends, and the pane scrolls to keep it in view.

use crate::harness::*;
use fastf::tui::app::data::ProjectDetail;
use fastf::tui::app::pane::PaneRow;

fn with_detail(app: &mut App, detail: ProjectDetail) {
    let path = app.library.selected().unwrap().path.clone();
    update(
        app,
        Msg::Detail {
            path,
            detail: Box::new(detail),
        },
    );
}

#[test]
fn the_cursor_walks_selectable_rows_and_stops_at_the_ends() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('j'));
    // Row 1 of the fixture carries two tags.
    assert_eq!(app.library.selected().unwrap().tags.len(), 2);
    with_detail(&mut app, ProjectDetail::default());
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail);
    assert_eq!(app.pane_cursor, 0, "the cursor starts on the name");

    let rows = app.pane_rows();
    press(&mut app, Key::ch('j'));
    assert!(
        matches!(rows[app.pane_cursor], PaneRow::Tag(_)),
        "down from the name is the first tag, over the facts: {:?}",
        rows[app.pane_cursor]
    );
    press(&mut app, Key::ch('G'));
    assert_eq!(rows[app.pane_cursor], PaneRow::AddTodo);
    press(&mut app, Key::ch('j'));
    assert_eq!(
        rows[app.pane_cursor],
        PaneRow::AddTodo,
        "the last row is the last row"
    );
    press(&mut app, Key::ch('g'));
    assert_eq!(app.pane_cursor, 0);
    press(&mut app, Key::ch('k'));
    assert_eq!(app.pane_cursor, 0, "and the first is the first");
    assert!(
        rows.iter().all(|row| !matches!(row, PaneRow::Reading)),
        "a read detail leaves no reading row"
    );
}

#[test]
fn the_pane_scrolls_to_keep_its_cursor_in_view_and_a_new_row_resets_it() {
    let mut app = fixture(6, 120, 24);
    // Five notes of six lines each: thirty rows, over a pane of twenty.
    let detail = ProjectDetail {
        notes: (0..5)
            .map(|n| fastf::core::body::Note {
                timestamp: Some(format!("2026-01-0{}T00:00:00Z", n + 1)),
                text: (0..6)
                    .map(|l| format!("note {n} line {l}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect(),
        ..Default::default()
    };
    with_detail(&mut app, detail);
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.detail_scroll, 0);
    press(&mut app, Key::ch('G'));
    let rows = app.pane_rows();
    assert_eq!(rows[app.pane_cursor], PaneRow::AddTodo);
    assert!(
        app.detail_scroll > 0,
        "the todo rows sit under thirty rows of notes, so the pane scrolled"
    );
    assert!(
        app.pane_cursor >= app.detail_scroll,
        "and the cursor is inside the window it scrolled to"
    );
    press(&mut app, Key::plain(KeyCode::Left));
    press(&mut app, Key::ch('j'));
    assert_eq!(
        app.pane_cursor, 0,
        "another project, the cursor starts again"
    );
    assert_eq!(app.detail_scroll, 0);
}

#[test]
fn the_cursor_is_drawn_only_while_the_pane_has_the_focus() {
    let mut app = fixture(6, 120, 40);
    with_detail(&mut app, ProjectDetail::default());
    press(&mut app, Key::plain(KeyCode::Right));
    // The cursor is a style, not a glyph — the pane's text does not move
    // when the focus arrives — so it is the buffer that shows it: mono
    // draws the selection reversed.
    let lit = fastf::tui::testing::render_to_buffer(&app, 120, 40);
    let pane = app.regions().detail.expect("a pane at 120 columns");
    let name_row = &lit[(pane.x + 1, pane.y + 1)];
    assert!(
        name_row
            .modifier
            .contains(ratatui::style::Modifier::REVERSED),
        "the name row wears the selection while the pane has the focus"
    );
    press(&mut app, Key::plain(KeyCode::Left));
    let dark = fastf::tui::testing::render_to_buffer(&app, 120, 40);
    assert!(
        !dark[(pane.x + 1, pane.y + 1)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED),
        "and not while the list has it"
    );
}
