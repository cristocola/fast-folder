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

/// **A read lands in the dialog that asked for it.** Two projects can carry
/// one id — `copy-to` keeps it, and the copy's folder can become a base — so
/// their dialogs carry one title, and a slow read of the first must not fill
/// the second's.
#[test]
fn a_late_read_never_fills_the_dialog_of_another_project() {
    let mut app = fixture(3, 120, 40);
    let mut projects = sample_projects(3);
    let original = projects[0].clone();
    let mut copy = original.clone();
    copy.base = std::path::PathBuf::from("/mnt/archive");
    copy.path = copy.base.join(&copy.name);
    projects.push(copy.clone());
    press(&mut app, Key::plain(KeyCode::F(5)));
    let generation = app.library.inflight.expect("a discovery is in flight");
    update(
        &mut app,
        Msg::Discovered {
            generation,
            projects,
        },
    );

    assert!(app.library.select_path(&original.path));
    let effects = press(&mut app, Key::ch('M'));
    let Some(Effect::LoadView {
        request: first,
        title,
        ..
    }) = effects.first()
    else {
        panic!("a read is asked for: {effects:?}");
    };
    let (first, title) = (*first, title.clone());
    press(&mut app, Key::plain(KeyCode::Esc));

    assert!(app.library.select_path(&copy.path));
    let effects = press(&mut app, Key::ch('M'));
    let Some(Effect::LoadView {
        request: second,
        title: same,
        ..
    }) = effects.first()
    else {
        panic!("a read is asked for: {effects:?}");
    };
    let second = *second;
    assert_eq!(&title, same, "one id, one title: the case this is about");
    assert_ne!(first, second);

    update(
        &mut app,
        Msg::ViewLoaded {
            request: first,
            title: title.clone(),
            lines: vec!["base             /mnt/projects".to_string()],
        },
    );
    let Some(Modal::Message { lines, .. }) = app.modals.top() else {
        panic!("the dialog is up");
    };
    assert_eq!(
        lines,
        &vec!["reading…".to_string()],
        "the first project's read is not this dialog's"
    );

    update(
        &mut app,
        Msg::ViewLoaded {
            request: second,
            title,
            lines: vec!["base             /mnt/archive".to_string()],
        },
    );
    let Some(Modal::Message { lines, .. }) = app.modals.top() else {
        panic!("the dialog is up");
    };
    assert_eq!(lines[0], "base             /mnt/archive");
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
    let request = app.view_request;
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            request,
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
            request,
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
