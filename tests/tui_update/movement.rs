//! The movement grammar and the horizontal axis: one set of movement keys in
//! every list, and `→`/`←` moving the focus between the list and the pane.

use crate::harness::*;
use fastf::tui::app::Screen;

/// Half a page is half of what a page key moves, and it stops at the ends
/// rather than wrapping — the same bargain the page keys make.
#[test]
fn ctrl_d_and_ctrl_u_move_half_a_page_and_clamp() {
    let mut app = fixture(40, 80, 24);
    let page = app.rows_on_screen();
    assert!(page >= 4, "the fixture needs a page worth of rows");
    press(&mut app, Key::ctrl('d'));
    assert_eq!(app.library.selected, Some(page / 2));
    press(&mut app, Key::ctrl('u'));
    assert_eq!(app.library.selected, Some(0));
    press(&mut app, Key::ctrl('u'));
    assert_eq!(
        app.library.selected,
        Some(0),
        "a half page clamps at the top"
    );
    for _ in 0..40 {
        press(&mut app, Key::ctrl('d'));
    }
    assert_eq!(app.library.selected, Some(app.library.len() - 1));
}

/// The action menu is a list of eighteen verbs that cannot be searched.
/// Before this it was the one list that could only be walked a row at a
/// time: neither the page keys nor the jumps reached it.
#[test]
fn the_action_menu_pages_and_jumps() {
    let mut app = fixture(6, 80, 24);
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(app.modals.top(), Some(Modal::Actions(_))),
        "the action menu should be open"
    );
    let rows = fastf::tui::app::actions::action_entries(&app).len();
    assert!(rows > 3, "the menu has more rows than a step");
    let at = |app: &App| match app.modals.top() {
        Some(Modal::Actions(state)) => state.selected,
        _ => panic!("the action menu closed"),
    };
    press(&mut app, Key::ch('G'));
    assert_eq!(at(&app), rows - 1, "G is the last row here too");
    press(&mut app, Key::ch('g'));
    assert_eq!(at(&app), 0);
    press(&mut app, Key::plain(KeyCode::PageDown));
    assert!(at(&app) > 0, "PgDn moves in the action menu");
    press(&mut app, Key::plain(KeyCode::Home));
    assert_eq!(at(&app), 0);
}

/// `g` and `G` used to mean "template from a folder" and "the guide" on the
/// templates tab, so the one list of arbitrary length had no jump keys at
/// all. The two verbs moved to `I` and `H`.
#[test]
fn the_templates_tab_jumps_to_its_ends() {
    let mut app = fixture(3, 100, 30);
    press(&mut app, Key::ch('T'));
    assert_eq!(app.screen, Screen::Templates);
    press(&mut app, Key::ch('G'));
    let last = app.studio.selected;
    press(&mut app, Key::ch('g'));
    assert_eq!(app.studio.selected, 0);
    assert!(last > 0, "the fixture has more than one template");
    assert!(app.modals.is_empty(), "neither key opened a dialog");
}

/// The templates tab's page keys route through the same step as its
/// arrows, so this is where a page used to come round to the top.
#[test]
fn the_templates_tab_pages_without_wrapping() {
    let mut app = fixture(3, 100, 30);
    press(&mut app, Key::ch('T'));
    assert_eq!(app.screen, Screen::Templates);
    press(&mut app, Key::plain(KeyCode::PageDown));
    let last = app.studio.selected;
    assert!(last > 0, "a page moved the cursor");
    press(&mut app, Key::plain(KeyCode::PageDown));
    assert_eq!(
        app.studio.selected, last,
        "and a second page stays at the end"
    );
    press(&mut app, Key::plain(KeyCode::PageUp));
    press(&mut app, Key::plain(KeyCode::PageUp));
    assert_eq!(app.studio.selected, 0, "the top is the top");
}

/// **The horizontal axis is focus.** `→` puts the cursor in the pane
/// beside the list, `←` puts it back — and neither runs anything. `→`
/// used to open the action menu, which is what Enter is for; an arrow
/// that executes a verb is an arrow you cannot lean on.
#[test]
fn the_right_arrow_focuses_the_pane_and_the_left_arrow_the_list() {
    let mut app = fixture(3, 120, 40);
    assert!(app.detail_visible());
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail, "→ moves into the pane");
    assert!(app.modals.is_empty(), "and opens nothing");
    press(&mut app, Key::plain(KeyCode::Left));
    assert_eq!(app.focus, Focus::Projects, "← comes back to the list");
    press(&mut app, Key::ch('l'));
    assert_eq!(app.focus, Focus::Detail);
    press(&mut app, Key::ch('h'));
    assert_eq!(app.focus, Focus::Projects);
    assert!(app.modals.is_empty());
}

/// **On a small window the pane takes the list's place.** There is no room
/// beside the list or under it at 80×24, so `→` puts the pane where the list
/// was, and `←` brings the list back — the same keys, the same focus, the
/// pane drawn instead of beside.
#[test]
fn on_a_small_window_the_pane_takes_the_lists_place() {
    use fastf::tui::layout::Placement;

    let mut app = fixture(3, 80, 24);
    assert_eq!(app.regions().placement, Some(Placement::Over));
    assert!(app.pane_behind_list());
    assert!(!app.detail_visible(), "the list is what is drawn");
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail);
    assert!(app.detail_visible(), "and now the pane is");
    let frame = fastf::tui::testing::render_to_string(&app, 80, 24);
    let selected = app.library.selected().unwrap().name.clone();
    assert!(
        frame.contains(&selected),
        "the pane names its project:\n{frame}"
    );
    assert!(
        !frame.contains("PROJECT"),
        "and the table is not drawn under it:\n{frame}"
    );
    press(&mut app, Key::plain(KeyCode::Left));
    assert_eq!(app.focus, Focus::Projects);
    assert!(!app.detail_visible());
    assert!(app.modals.is_empty());
}

/// **The horizontal axis never quits, and never runs.** On the list `←` has
/// nothing to its left and is not bound; with the pane switched off, `→` has
/// nothing to its right either.
#[test]
fn the_arrows_are_unbound_with_the_pane_closed() {
    // Switched off where it sits beside the list — at 80×24 `i` only goes in
    // and out of it — and the window made small after.
    let mut app = fixture(3, 120, 40);
    press(&mut app, Key::ch('i'));
    update(&mut app, Msg::Resize(80, 24));
    assert!(!app.detail_open, "the fixture switches the pane off");
    for key in [
        Key::plain(KeyCode::Left),
        Key::ch('h'),
        Key::plain(KeyCode::Right),
        Key::ch('l'),
    ] {
        assert!(
            press(&mut app, key).is_empty(),
            "{} did something",
            key.label()
        );
        assert!(app.modals.is_empty());
        assert_eq!(app.focus, Focus::Projects);
    }
}

/// **`i` shows or hides the pane, wherever it is.** Beside the list it closes
/// and opens it; where it takes the list's place, showing it is going into it
/// and hiding it is coming back out.
#[test]
fn i_shows_or_hides_the_pane_where_it_is() {
    let mut wide = fixture(3, 120, 40);
    assert!(wide.detail_visible());
    press(&mut wide, Key::ch('i'));
    assert!(
        !wide.detail_open && !wide.detail_visible(),
        "beside: i closes it"
    );
    press(&mut wide, Key::ch('i'));
    assert!(
        wide.detail_open && wide.detail_visible(),
        "and opens it again"
    );
    assert_eq!(wide.focus, Focus::Projects, "without taking the focus");

    let mut small = fixture(3, 80, 24);
    press(&mut small, Key::ch('i'));
    assert_eq!(
        small.focus,
        Focus::Detail,
        "in the list's place: i goes into it"
    );
    assert!(small.detail_visible());
    press(&mut small, Key::ch('i'));
    assert_eq!(small.focus, Focus::Projects, "and i again comes back out");
    assert!(small.detail_open, "still on, one key away");
}

/// The templates tab has a pane too, and it is always drawn — so `→`
/// reaches it at any width, and `←` from it is the card list, never the
/// library: leaving a tab is Esc's ladder and `T`, not an arrow.
#[test]
fn the_left_arrow_leaves_the_pane_but_never_the_tab() {
    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('T'));
    assert_eq!(app.screen, Screen::Templates);
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail, "→ reaches the template pane");
    press(&mut app, Key::ch('h'));
    assert_eq!(app.focus, Focus::Projects);
    assert_eq!(app.screen, Screen::Templates, "← is not the way off a tab");
    assert!(press(&mut app, Key::ch('h')).is_empty());
    assert_eq!(
        app.screen,
        Screen::Templates,
        "and a second ← is not either"
    );
    press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Library, "Esc is");
}

/// Tab reaches the template pane on a window too narrow for the library's
/// pane. It measured the library's geometry before, so on an 80-column
/// window the ring had one member and the template pane's tail — a
/// `template show` taller than the box — was unreachable.
#[test]
fn tab_reaches_the_template_pane_on_a_narrow_window() {
    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('T'));
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Detail);
    app.studio.lines = (0..60).map(|n| format!("line {n}")).collect();
    press(&mut app, Key::ch('j'));
    assert_eq!(app.studio.scroll, 1, "and the arrows scroll the pane");
    press(&mut app, Key::ch('G'));
    assert!(app.studio.scroll > 1, "G reaches the end of the pane");
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Projects, "Tab comes round");
}

/// Ctrl-C is a declared command now, so it is in the help — and it still
/// answers before anything else, from under a dialog that takes every key.
#[test]
fn ctrl_c_is_a_command_and_still_answers_first() {
    use fastf::tui::command::{CommandId, Context, find};
    assert!(
        find(CommandId::Interrupt)
            .contexts
            .contains(&Context::Global)
    );
    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('?'));
    assert!(
        press(&mut app, Key::ctrl('c')).is_empty(),
        "it closed the help"
    );
    assert!(app.modals.is_empty());
    assert_eq!(
        press(&mut app, Key::ctrl('c')),
        vec![Effect::Quit(Exit::Interrupted)]
    );
}

/// Everything printable in the search bar is the query — `c` types a `c`
/// rather than opening the palette — and everything else is offered to the
/// registry, which is what makes `Context::SearchEdit`'s help true.
#[test]
fn the_search_bar_types_letters_and_lets_chords_through() {
    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "cq?");
    assert_eq!(app.search.input.text(), "cq?");
    assert!(app.modals.is_empty(), "not one of those opened a dialog");

    press(&mut app, Key::ctrl('p'));
    assert!(
        matches!(app.modals.top(), Some(Modal::Palette(_))),
        "a chord still reaches the registry from the bar"
    );
    press(&mut app, Key::plain(KeyCode::Esc));

    // Esc's ladder: the first clears the query, the second leaves the bar.
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.search.input.is_empty());
    assert!(app.search.editing, "still in the bar, ready to retype");
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(!app.search.editing);
}

/// The palette's own keys are declared too, so `Ctrl-p` means the previous
/// entry there and the opener is bound in every context but this one.
#[test]
fn the_palette_moves_with_its_own_keys() {
    let mut app = fixture(6, 100, 30);
    press(&mut app, Key::ch('c'));
    let at = |app: &App| match app.modals.top() {
        Some(Modal::Palette(state)) => state.selected,
        _ => panic!("the palette closed"),
    };
    assert_eq!(at(&app), Some(0));
    press(&mut app, Key::ctrl('n'));
    assert_eq!(at(&app), Some(1));
    press(&mut app, Key::ctrl('p'));
    assert_eq!(at(&app), Some(0), "Ctrl-p is back up, not a second palette");
    type_text(&mut app, "q");
    assert!(
        matches!(app.modals.top(), Some(Modal::Palette(_))),
        "`q` is a letter of the query, not the quit key"
    );
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.modals.is_empty());
}

/// **Esc leaves the pane before it does anything else.** The pane is a level,
/// like a tab; Esc used to fall through it to the list's ladder, and with no
/// search, filter or marks to clear, that ladder quits — so leaving the pane
/// the way every dialog is left closed the app.
#[test]
fn esc_in_the_pane_goes_back_to_the_list_and_never_quits() {
    let mut app = fixture(3, 120, 40);
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail);

    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Quit(_))),
        "Esc in the pane quit the app: {effects:?}"
    );
    assert_eq!(app.focus, Focus::Projects, "it goes back to the list");
    assert_eq!(app.screen, Screen::Library);

    // From the list, the ladder is what it always was.
    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Quit(Exit::Normal))),
        "the list's Esc still quits once there is nothing to clear: {effects:?}"
    );
}

/// The template pane is a level of its tab: Esc goes to the card list first,
/// then off the tab.
#[test]
fn esc_from_the_template_pane_goes_to_its_list_first() {
    let mut app = fixture(3, 120, 40);
    press(&mut app, Key::ch('T'));
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail);

    press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.focus, Focus::Projects, "the card list first");
    assert_eq!(app.screen, Screen::Templates, "still on the tab");
    press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Library, "then off it");
}

/// **A window that shrinks keeps the pane focused**, in its new place: from
/// beside the list at 120 columns to the list's place at 80, the focus stays
/// in the pane and the pane is what is drawn.
#[test]
fn a_window_that_shrinks_keeps_the_pane_focused_in_its_new_place() {
    use fastf::tui::layout::Placement;

    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.regions().placement, Some(Placement::Beside));
    update(&mut app, Msg::Resize(80, 24));
    assert_eq!(app.regions().placement, Some(Placement::Over));
    assert_eq!(app.focus, Focus::Detail, "still in the pane");
    assert!(app.detail_visible(), "which now has the list's place");
    update(&mut app, Msg::Resize(120, 40));
    assert_eq!(app.focus, Focus::Detail);
    assert_eq!(app.regions().placement, Some(Placement::Beside));
}

/// The templates tab draws in the whole body, wherever the library's pane
/// would be: it used to rebuild its band from the table's width plus the
/// pane's, which is the window twice over once the pane sits under the table.
#[test]
fn the_templates_tab_takes_the_whole_body_whatever_the_library_pane_does() {
    for (width, height) in [(120, 40), (60, 45), (80, 24)] {
        let mut app = fixture(6, width, height);
        press(&mut app, Key::ch('T'));
        let frame = fastf::tui::testing::render_to_string(&app, width, height);
        let regions = app.regions();
        let bottom = frame
            .lines()
            .nth((regions.body.y + regions.body.height - 1) as usize)
            .unwrap_or_default();
        assert!(
            bottom.contains('└') && bottom.contains('┘'),
            "{width}×{height}: the tab's boxes close on the body's last row:\n{frame}"
        );
    }
}

/// **`<` and `>` walk the projects from inside the pane**, keeping the focus
/// there and the cursor in the section it was in, so one project's todos
/// after another's is one key each. A walk is not a change: nothing pulses.
/// A project whose detail is still being read takes the cursor there when its
/// read lands.
#[test]
fn angle_brackets_walk_the_projects_from_the_pane_and_keep_the_section() {
    use fastf::tui::app::data::ProjectDetail;
    use fastf::tui::app::pane::{PaneRow, PaneSection, section_at};

    let record = || ProjectDetail {
        todos: vec![
            fastf::core::body::Todo {
                done: false,
                text: "grade".to_string(),
                phase: None,
            },
            fastf::core::body::Todo {
                done: false,
                text: "deliver".to_string(),
                phase: None,
            },
        ],
        ..Default::default()
    };
    let deliver = |app: &mut App| {
        let path = app.library.selected().unwrap().path.clone();
        update(
            app,
            Msg::Detail {
                path,
                detail: Box::new(record()),
            },
        );
    };
    let mut app = fixture(6, 80, 24);
    deliver(&mut app);
    press(&mut app, Key::plain(KeyCode::Right));
    // Onto the second todo.
    let second = app
        .pane_rows()
        .iter()
        .position(|row| matches!(row, PaneRow::Todo { ordinal: 1, .. }))
        .unwrap();
    while app.pane_cursor < second {
        press(&mut app, Key::ch('j'));
    }

    // The next project has not been read: the cursor waits, then lands.
    press(&mut app, Key::ch('>'));
    assert_eq!(app.library.selected_index(), Some(1));
    assert_eq!(app.focus, Focus::Detail, "still in the pane");
    deliver(&mut app);
    let rows = app.pane_rows();
    assert_eq!(section_at(&rows, app.pane_cursor), PaneSection::Todo);
    assert!(
        matches!(rows[app.pane_cursor], PaneRow::Todo { ordinal: 0, .. }),
        "the first todo of the next project: {:?}",
        rows[app.pane_cursor]
    );
    assert!(app.pane_pulses.is_empty(), "a walk is not a change");

    // Back: the first project is cached, so the cursor lands at once.
    press(&mut app, Key::ch('<'));
    assert_eq!(app.library.selected_index(), Some(0));
    let rows = app.pane_rows();
    assert!(matches!(
        rows[app.pane_cursor],
        PaneRow::Todo { ordinal: 0, .. }
    ));

    // At the top of the list there is nowhere above to go.
    let effects = press(&mut app, Key::ch('<'));
    assert!(effects.is_empty());
    assert_eq!(app.library.selected_index(), Some(0));
}

/// **The template pane pages by its own height.** It paged by the library
/// table's, which is a different box once each tab places its pane by its own
/// rule — at 60×45 the library's table is eleven rows under a pane, while the
/// templates tab's pane is beside a short card list.
#[test]
fn the_template_pane_pages_by_its_own_height() {
    let mut app = fixture(8, 60, 45);
    press(&mut app, Key::ch('T'));
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Detail);
    app.studio.lines = (0..200).map(|n| format!("line {n}")).collect();
    let pane = app.template_panes().1;
    press(&mut app, Key::plain(KeyCode::PageDown));
    assert_eq!(
        app.studio.scroll,
        (pane.height - 2) as usize,
        "one page is the template pane's text height"
    );
    assert_ne!(
        app.studio.scroll,
        app.regions().table_rows(),
        "not the library table's"
    );
}
