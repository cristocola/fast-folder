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
    assert_eq!(rows[app.pane_cursor], PaneRow::AddNote);
    press(&mut app, Key::ch('j'));
    assert_eq!(
        rows[app.pane_cursor],
        PaneRow::AddNote,
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
    assert_eq!(rows[app.pane_cursor], PaneRow::AddNote);
    assert!(
        app.detail_scroll > 0,
        "the last row sits under thirty rows of notes, so the pane scrolled"
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

/// **A page in the pane is the pane's height, in drawn rows.** It used to be
/// the table's height counted in *selectable* rows, so one PageDown walked
/// past every wrapped line of every note and landed several screens down.
#[test]
fn page_down_in_the_pane_moves_by_the_panes_height() {
    let mut app = fixture(6, 120, 24);
    // Five notes of six lines each: thirty rows, over a pane of seventeen.
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
    let pane = app.regions().detail.expect("a pane at 120 columns");
    let page = (pane.height - 2) as usize;
    let rows = app.pane_rows();
    assert!(rows.len() > 2 * page, "the fixture is several pages tall");

    press(&mut app, Key::plain(KeyCode::PageDown));
    let first = app.pane_cursor;
    assert!(
        first > 0 && first <= page,
        "one page down stays within a page: row {first} of a {page}-row pane"
    );
    assert!(rows[first].selectable());

    press(&mut app, Key::plain(KeyCode::PageDown));
    assert!(app.pane_cursor > first, "a second page goes further");
    assert!(app.pane_cursor <= first + page);
    assert!(
        app.pane_cursor >= app.detail_scroll && app.pane_cursor < app.detail_scroll + page,
        "and the cursor is in view"
    );

    press(&mut app, Key::plain(KeyCode::PageUp));
    press(&mut app, Key::plain(KeyCode::PageUp));
    assert_eq!(app.pane_cursor, 0, "two pages back up is the top");
}

fn with_todos(app: &mut App, n: usize) {
    with_detail(
        app,
        ProjectDetail {
            todos: (0..n)
                .map(|t| fastf::core::body::Todo {
                    done: false,
                    text: format!("task {t}"),
                    phase: None,
                })
                .collect(),
            ..Default::default()
        },
    );
}

/// **A size landing never moves the pane's cursor.** The size shares a header
/// row with the counts; measured at the widest a size gets, the row count
/// cannot change when `scanning…` becomes `41.0 GB`, so the cursor stays on
/// the row it was on — and no other row slides under it.
#[test]
fn a_size_landing_never_moves_the_pane_cursor() {
    let mut app = fixture(6, 120, 40);
    with_todos(&mut app, 3);
    press(&mut app, Key::plain(KeyCode::Right));
    press(&mut app, Key::ch('j'));
    press(&mut app, Key::ch('j'));
    press(&mut app, Key::ch('j'));
    let before = app.pane_rows();
    let at = app.pane_cursor;
    let path = app.library.selected().unwrap().path.clone();
    update(
        &mut app,
        Msg::Sizes(vec![(path, Some(41 * 1024 * 1024 * 1024))]),
    );
    assert_eq!(app.pane_rows(), before, "the rows are the same rows");
    assert_eq!(app.pane_cursor, at, "and the cursor is where it was");
}

/// **The pane's text is drawn where `pane_rows` measured it**: a column in
/// from each border. The first letter of the name sits at that column, and a
/// wrapped todo's rows end inside it.
#[test]
fn the_pane_text_is_where_pane_rows_measured_it() {
    let mut app = fixture(6, 120, 40);
    with_todos(&mut app, 1);
    let pane = app.regions().detail.expect("a pane at 120 columns");
    let text = fastf::tui::layout::pane_text(pane);
    assert_eq!((text.x, text.width), (pane.x + 2, pane.width - 4));
    let buffer = fastf::tui::testing::render_to_buffer(&app, 120, 40);
    let name = app.library.selected().unwrap().name.clone();
    assert_eq!(
        buffer[(pane.x + 1, pane.y + 1)].symbol(),
        " ",
        "the padding column is blank"
    );
    assert_eq!(
        buffer[(text.x, text.y)].symbol(),
        name.chars().next().unwrap().to_string(),
        "the name starts at the text column"
    );
}

/// A pane with more rows than it shows has a scrollbar on its border, drawn
/// in the theme's alphabet; one that shows everything has none.
#[test]
fn a_pane_taller_than_its_box_has_a_scrollbar() {
    use fastf::tui::theme::{Glyphs, Theme};

    let mut app = fixture(6, 120, 40);
    with_todos(&mut app, 3);
    let pane = app.regions().detail.unwrap();
    let edge = |app: &App| {
        let buffer = fastf::tui::testing::render_to_buffer(app, 120, 40);
        (pane.y + 1..pane.y + pane.height - 1)
            .map(|y| buffer[(pane.x + pane.width - 1, y)].symbol().to_string())
            .collect::<String>()
    };
    assert!(
        edge(&app).chars().all(|c| c == '│'),
        "everything fits: a plain border"
    );

    with_todos(&mut app, 60);
    assert!(
        edge(&app).contains('█'),
        "sixty todos: a thumb on the border"
    );
    app.theme = Theme::mono().with_glyphs(Glyphs::ascii());
    let ascii = edge(&app);
    assert!(
        ascii.contains('#') && !ascii.contains('█'),
        "in the ASCII alphabet, too: {ascii}"
    );
}
