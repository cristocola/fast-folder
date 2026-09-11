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

/// **The horizontal axis never quits, and never runs.** On the list `←`
/// has nothing to its left and is not bound; without a pane — the window
/// is under a hundred columns — `→` has nothing to its right either.
#[test]
fn the_arrows_are_unbound_where_there_is_nowhere_to_go() {
    let mut app = fixture(3, 80, 24);
    assert!(
        !app.detail_visible(),
        "the fixture is too narrow for a pane"
    );
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
