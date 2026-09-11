//! What the app does besides moving: a range of marks, a sort in either
//! direction, a tag filter, a narrowed settings screen.

use crate::harness::*;
use fastf::tui::app::library::{Order, Sort};
use fastf::tui::app::modal::Then;

/// Space marks a row and steps on; `v` reaches from there to the cursor.
/// Twenty rows are three keystrokes instead of twenty.
#[test]
fn v_marks_from_the_last_mark_to_the_cursor() {
    let mut app = fixture(8, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0, cursor → 1
    assert_eq!(app.library.marks.len(), 1);
    for _ in 0..3 {
        press(&mut app, Key::plain(KeyCode::Down));
    }
    assert_eq!(app.library.selected, Some(4));
    press(&mut app, Key::ch('v'));
    assert_eq!(
        app.library.marks.len(),
        5,
        "rows 0 through 4, inclusive at both ends"
    );
    for row in 0..5 {
        let path = app.library.row(row).unwrap().path.clone();
        assert!(app.library.marks.contains(&path), "row {row} is marked");
    }
}

/// It reaches backwards too, and needs an anchor before it will do
/// anything at all.
#[test]
fn v_refuses_without_an_anchor_and_reaches_both_ways() {
    let mut app = fixture(8, 80, 24);
    assert!(!app.library.has_anchor());
    press(&mut app, Key::ch('v'));
    assert!(app.library.marks.is_empty(), "nothing to reach from");

    press(&mut app, Key::ch('G')); // last row
    press(&mut app, Key::ch(' ')); // mark it (and wrap to the first)
    press(&mut app, Key::ch('g')); // back to the top
    press(&mut app, Key::ch('v'));
    assert_eq!(app.library.marks.len(), app.library.len());
}

/// Every one-way order runs both ways, and the tie-break does not turn
/// round with it.
#[test]
fn a_sort_runs_both_ways() {
    let mut app = fixture(6, 80, 24);
    // `s` walks the cycle from newest: oldest, then name.
    for _ in 0..2 {
        press(&mut app, Key::ch('s'));
    }
    assert_eq!(
        app.library.effective_sort(&app.search.query),
        Sort::new(Order::Name)
    );
    let forwards = names(&app);
    app.library.explicit_sort = Some(Sort {
        order: Order::Name,
        reversed: true,
    });
    app.library.recompute(&app.search.query, &mut app.fuzzy);
    let mut expected = forwards.clone();
    expected.reverse();
    assert_eq!(names(&app), expected);

    // The picker offers both, and the label it writes is the label the
    // session file reads back.
    press(&mut app, Key::ch('S'));
    let labels: Vec<String> = match app.modals.top() {
        Some(Modal::Pick(pick)) => pick.items.iter().map(|i| i.label.clone()).collect(),
        other => panic!("the sort picker should be open, not {other:?}"),
    };
    assert!(labels.contains(&"size".to_string()));
    assert!(labels.contains(&"size reversed".to_string()));
    assert!(
        !labels.contains(&"newest reversed".to_string()),
        "newest and oldest are already the two directions of one order"
    );
}

/// The tag filter writes the search grammar's own clause, so clearing it
/// is the same Esc rung as clearing any other query.
#[test]
fn filter_by_tag_writes_the_query_the_grammar_already_had() {
    use fastf::tui::command::CommandId;

    let mut app = fixture(9, 100, 30);
    assert!(!app.library.known_tags.is_empty(), "the fixture has tags");
    let _ = app.run(CommandId::FilterTag);
    let tag = match app.modals.top() {
        Some(Modal::Pick(pick)) => {
            assert_eq!(pick.then, Then::TagFilter);
            pick.items[0].value.clone()
        }
        other => panic!("the tag picker should be open, not {other:?}"),
    };
    press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(app.search.input.text(), format!("tag:{tag}"));
    assert!(app.modals.is_empty());
}

/// `/` narrows the settings list and the title says what to; Esc gives the
/// whole screen back, because a filter left behind is a screen missing
/// rows for a reason nobody can see.
#[test]
fn slash_filters_the_settings_and_esc_gives_them_back() {
    let mut app = fixture(3, 100, 30);
    press(&mut app, Key::ch(','));
    let _ = update(&mut app, Msg::SettingsLoaded(Box::default()));
    let all = match app.modals.top() {
        Some(Modal::Settings(state)) => state.rows.len(),
        other => panic!("the settings should be open, not {other:?}"),
    };
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "base");
    match app.modals.top() {
        Some(Modal::Settings(state)) => {
            assert!(state.rows.len() < all, "the list narrowed");
            assert!(
                state.rows.iter().any(|row| row.label == "Base directory"),
                "and kept what was asked for"
            );
            assert!(
                state.rows.iter().any(|row| !row.selectable()),
                "a kept row keeps the heading it is under"
            );
        }
        other => panic!("the settings closed: {other:?}"),
    }
    press(&mut app, Key::plain(KeyCode::Esc));
    match app.modals.top() {
        Some(Modal::Settings(state)) => {
            assert_eq!(state.rows.len(), all, "Esc gave the screen back");
            assert!(state.filter.text().is_empty());
        }
        other => panic!("Esc closed the settings instead: {other:?}"),
    }
}
