//! The field-first rule, and the dialogs that declare their own keys.

use crate::harness::*;

/// `Ctrl-u` is `HalfUp` on a list and kill-to-start inside the bar. The
/// field has first refusal, so the same physical key one keystroke apart
/// keeps meaning what the surface it is on says it means.
#[test]
fn the_field_takes_its_chords_before_the_registry_does() {
    let mut app = fixture(40, 80, 24);
    press(&mut app, Key::ctrl('d'));
    assert!(app.library.selected.unwrap() > 0, "Ctrl-d paged the list");

    press(&mut app, Key::ch('/'));
    type_text(&mut app, "lull");
    press(&mut app, Key::ctrl('u'));
    assert!(app.search.input.is_empty(), "Ctrl-u killed the line");
    assert!(app.search.editing, "and did not page anything");
}

/// The arrows in the bar are `Down` and `Up` themselves, moving the list
/// under the query rather than a second pair written out in the handler.
#[test]
fn the_arrows_move_the_library_under_the_query() {
    let mut app = fixture(6, 80, 24);
    press(&mut app, Key::ch('/'));
    let first = selected_name(&app);
    press(&mut app, Key::plain(KeyCode::Down));
    assert_ne!(selected_name(&app), first);
    assert!(app.search.editing, "still typing");
    // …and `j` is a letter of the query, not a step.
    let at = app.library.selected;
    type_text(&mut app, "j");
    assert_eq!(app.library.selected, at);
    assert_eq!(app.search.input.text(), "j");
}

/// Space ticks a row in a multi-pick and types a space in a picker with a
/// query — one command, `Hidden` where it does not apply.
#[test]
fn space_ticks_a_multi_pick_and_types_in_a_picker() {
    let mut app = fixture(3, 100, 30);
    press(&mut app, Key::ch('A')); // add a tag → a picker with a query
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    press(&mut app, Key::ch(' '));
    match app.modals.top() {
        Some(Modal::Pick(pick)) => assert_eq!(pick.query.text(), " "),
        other => panic!("the picker closed: {other:?}"),
    }
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.modals.is_empty());
}

/// Enter and Esc on a one-line prompt are commands now, which is what lets
/// `Context::Prompt` have a help at all.
#[test]
fn a_prompt_confirms_and_cancels_through_the_registry() {
    use fastf::tui::command::{Context, keys_in};

    let mut app = fixture(3, 100, 30);
    press(&mut app, Key::ch('r')); // rename → a text prompt
    assert!(matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
    assert_eq!(app.context(), Context::Prompt);

    // `?` is a letter here, so the bar must not offer it.
    let help = fastf::tui::command::find(fastf::tui::command::CommandId::Help);
    let offered = keys_in(Context::Prompt, help);
    assert!(offered.iter().all(|k| k.typed().is_none()));
    assert!(offered.contains(&Key::plain(KeyCode::F(1))));

    type_text(&mut app, "?");
    assert!(
        matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
        "`?` typed a question mark rather than opening the help"
    );
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.modals.is_empty());
}

/// **The doors lead the bar.** Where the pane takes the list's place, the one
/// of the two out of sight is a key away and nothing on screen says so — so
/// its key comes first, and a narrow bar can never cut it: `→ details` from
/// the list, `← list` from the pane.
#[test]
fn the_hint_bar_leads_with_the_door_when_the_pane_is_hidden() {
    use fastf::tui::command::{Context, hints};

    let mut app = fixture(6, 60, 20);
    assert!(app.pane_behind_list());
    let bar = hints(Context::Projects, &app, 58);
    assert_eq!(
        bar.first().map(|(k, t)| (k.as_str(), *t)),
        Some(("→", "details")),
        "{bar:?}"
    );
    press(&mut app, Key::plain(KeyCode::Right));
    let bar = hints(Context::Detail, &app, 58);
    assert_eq!(
        bar.first().map(|(k, t)| (k.as_str(), *t)),
        Some(("←", "list")),
        "{bar:?}"
    );

    // Beside the list both are in view, and the bar keeps its usual order.
    let wide = fixture(6, 120, 40);
    let bar = hints(Context::Projects, &wide, 118);
    assert_ne!(bar.first().map(|(k, _)| k.as_str()), Some("→"), "{bar:?}");
    assert!(bar.iter().any(|(k, t)| k == "→" && *t == "pane"), "{bar:?}");
}
