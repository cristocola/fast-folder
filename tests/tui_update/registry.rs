//! One registry: every dialog's keys are declared, and help is everywhere.

use crate::harness::*;

#[test]
fn a_verb_key_in_the_action_menu_runs_it_and_closes_the_menu() {
    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('a'));
    assert!(matches!(app.modals.top(), Some(Modal::Actions(_))));
    let effects = press(&mut app, Key::ch('o'));
    assert!(
        matches!(effects.first(), Some(Effect::Spawn(SpawnKind::Reveal(_)))),
        "o in the menu reveals the folder: {effects:?}"
    );
    assert!(app.modals.is_empty(), "the menu closes once its verb ran");

    let _ = press(&mut app, Key::ch('a'));
    let _ = press(&mut app, Key::ch('D'));
    assert!(
        matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
        "D in the menu opens the delete confirmation"
    );
}

#[test]
fn help_opens_over_any_dialog_for_that_dialogs_context() {
    use fastf::tui::command::Context;

    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('a'));
    let _ = press(&mut app, Key::ch('?'));
    assert!(
        matches!(
            app.modals.top(),
            Some(Modal::Help {
                ctx: Context::Actions,
                ..
            })
        ),
        "? over the action menu is the action menu's help"
    );
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        matches!(app.modals.top(), Some(Modal::Actions(_))),
        "closing the help lands back on the menu"
    );

    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    let _ = press(&mut app, Key::ch(','));
    let _ = update(&mut app, Msg::SettingsLoaded(Box::default()));
    assert!(matches!(app.modals.top(), Some(Modal::Settings(_))));
    let _ = press(&mut app, Key::plain(KeyCode::F(1)));
    assert!(matches!(
        app.modals.top(),
        Some(Modal::Help {
            ctx: Context::Settings,
            ..
        })
    ));
    let _ = press(&mut app, Key::ch('q'));
    let effects = press(&mut app, Key::ch('q'));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Quit(_))),
        "q closes the settings, it does not quit: {effects:?}"
    );
    assert!(app.modals.is_empty());
}

#[test]
fn the_templates_tab_and_the_builder_answer_their_declared_keys() {
    use fastf::tui::app::Screen;

    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('T'));
    assert_eq!(app.screen, Screen::Templates);
    let _ = press(&mut app, Key::ch('D'));
    assert!(
        matches!(app.modals.top(), Some(Modal::Confirm(_))),
        "D asks before deleting a template"
    );
    let _ = press(&mut app, Key::ch('x'));
    assert!(
        matches!(app.modals.top(), Some(Modal::Confirm(_))),
        "a confirmation takes only y, n, Esc and the global keys"
    );
    let _ = press(&mut app, Key::ch('n'));
    assert!(
        app.modals.is_empty(),
        "n answers no and closes the confirmation"
    );
    let _ = press(&mut app, Key::ch('n'));
    assert!(
        matches!(app.modals.top(), Some(Modal::Builder(_))),
        "n opens the builder on a new template"
    );
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        app.modals.is_empty(),
        "Esc on an untouched section list returns to the tab without asking"
    );
    assert_eq!(app.screen, Screen::Templates);
    assert!(app.status.text.contains("Closed"));
    // Esc again is one more level out: the tab you came from.
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Library);
}

#[test]
fn a_pager_stops_at_its_last_line() {
    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('?'));
    let _ = press(&mut app, Key::plain(KeyCode::End));
    let Some(Modal::Help { scroll, .. }) = app.modals.top() else {
        panic!("expected the help");
    };
    let at_end = *scroll;
    assert!(at_end > 0, "End scrolls a long help");
    let _ = press(&mut app, Key::plain(KeyCode::PageDown));
    let Some(Modal::Help { scroll, .. }) = app.modals.top() else {
        panic!("expected the help");
    };
    assert_eq!(*scroll, at_end, "nothing past the last line");
    let _ = press(&mut app, Key::plain(KeyCode::Home));
    let Some(Modal::Help { scroll, .. }) = app.modals.top() else {
        panic!("expected the help");
    };
    assert_eq!(*scroll, 0);
}
