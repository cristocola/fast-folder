//! The edges of the dashboard: a preset as a filter, a dialog that waits on its
//! worker, a malformed query, a verb with nowhere to go, the too-small guard.

use crate::harness::*;

#[test]
fn a_recent_preset_is_a_filter_esc_takes_off() {
    let mut app = App::new(
        Entry::Recent {
            preset: Preset {
                template: Some("general".to_string()),
                ..Default::default()
            },
            initial: sample_projects(6),
        },
        Theme::mono(),
        (120, 40),
    );
    assert_eq!(app.library.len(), 2, "the preset narrows the rows");
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.library.preset.is_none());
    assert_eq!(app.library.len(), 6, "Esc shows every project again");
    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        matches!(effects.first(), Some(Effect::Quit(Exit::Normal))),
        "with nothing left to clear, Esc quits: {effects:?}"
    );
}

#[test]
fn a_dialog_that_reads_from_a_worker_goes_up_at_once() {
    let mut app = fixture(3, 120, 40);
    let effects = press(&mut app, Key::ch('M'));
    assert!(matches!(effects.first(), Some(Effect::LoadView { .. })));
    let Some(Modal::Message { title, lines, .. }) = app.modals.top() else {
        panic!("the metadata dialog goes up before the read lands");
    };
    assert!(title.ends_with("metadata"));
    assert_eq!(lines, &vec!["reading…".to_string()]);
    let title = title.clone();
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title: title.clone(),
            lines: vec!["id             ID0248".to_string()],
        },
    );
    let Some(Modal::Message { lines, .. }) = app.modals.top() else {
        panic!("still the same dialog");
    };
    assert_eq!(lines[0], "id             ID0248");
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title,
            lines: vec!["late".to_string()],
        },
    );
    assert!(
        app.modals.is_empty(),
        "a read that lands after Esc is dropped"
    );

    let effects = press(&mut app, Key::ch(','));
    assert!(matches!(effects.first(), Some(Effect::LoadSettings)));
    assert!(
        matches!(app.modals.top(), Some(Modal::Settings(state)) if state.rows.is_empty() && state.pending),
        "the settings screen goes up empty and pending"
    );
}

#[test]
fn a_malformed_query_is_named_while_it_is_typed() {
    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('/'));
    let _ = type_text(&mut app, "created>notadate");
    assert!(
        app.status.text.contains("needs a date like 2026-01-01"),
        "{:?}",
        app.status
    );
    let _ = press(&mut app, Key::ctrl('u'));
    assert!(
        app.status.text.is_empty(),
        "a good query clears the warning"
    );
}

#[test]
fn move_with_one_base_says_why() {
    let mut app = fixture(3, 120, 40);
    let effects = press(&mut app, Key::ch('m'));
    assert!(effects.is_empty());
    assert!(
        app.status.text.contains("base"),
        "m explains itself: {:?}",
        app.status
    );
    assert!(app.modals.is_empty());
}

#[test]
fn the_too_small_guard_keeps_the_quit_gestures_honest() {
    let mut app = fixture(3, 40, 10);
    let effects = press(&mut app, Key::ch('?'));
    assert!(effects.is_empty() && app.modals.is_empty());
    let effects = press(&mut app, Key::ctrl('c'));
    assert!(
        matches!(effects.first(), Some(Effect::Quit(Exit::Interrupted))),
        "Ctrl-C is still an interrupt: {effects:?}"
    );
    let effects = press(&mut app, Key::ch('q'));
    assert!(matches!(effects.first(), Some(Effect::Quit(Exit::Normal))));
}
