//! The terminal in every situation.

use crate::harness::*;

#[test]
fn a_window_verb_says_no_display_where_there_is_none() {
    let mut app = fixture(3, 120, 40);
    app.has_display = false;
    let effects = press(&mut app, Key::ch('o'));
    assert!(effects.is_empty(), "nothing is spawned: {effects:?}");
    assert!(
        app.status.text.contains("no display"),
        "o says why: {}",
        app.status.text
    );
    let effects = press(&mut app, Key::ch('t'));
    assert!(effects.is_empty());
    // Copying the path still works — it is what the reason points at.
    let effects = press(&mut app, Key::ch('y'));
    assert!(matches!(
        effects.first(),
        Some(Effect::Spawn(SpawnKind::Clipboard(_)))
    ));
}

#[cfg(unix)]
#[test]
fn ctrl_z_suspends_to_the_shell_and_the_size_is_reread_on_return() {
    let mut app = fixture(3, 120, 40);
    let effects = press(&mut app, Key::ctrl('z'));
    assert_eq!(effects, vec![Effect::Suspend(Suspended::Shell)]);
    let _ = update(&mut app, Msg::Resumed(fastf::tui::msg::Resumed::Shell));
    let _ = update(&mut app, Msg::Resize(100, 30));
    assert_eq!(app.size, (100, 30));
}

/// `$` hands this terminal to the user's shell at the selected project, and
/// needs no display — it is the terminal that works over ssh, where `t` has
/// no desktop to open a window on. Back from the shell, the pane reads the
/// project's file again, since whatever was done there is in the folder.
#[test]
fn a_shell_here_needs_no_display_and_rereads_the_pane_on_return() {
    let mut app = fixture(3, 120, 40);
    app.has_display = false;
    let project = app.library.selected().unwrap().clone();
    assert!(
        press(&mut app, Key::ch('t')).is_empty(),
        "no window without a display"
    );
    let effects = press(&mut app, Key::ch('$'));
    assert!(
        matches!(&effects[..], [Effect::Suspend(Suspended::ProjectShell(p))] if p.path == project.path),
        "{effects:?}"
    );

    let effects = update(
        &mut app,
        Msg::Resumed(fastf::tui::msg::Resumed::ProjectShell {
            project: Box::new(project.clone()),
        }),
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDetail(path) if *path == project.path)),
        "{effects:?}"
    );
}

/// With no project there is no folder to open a shell in.
#[test]
fn a_shell_here_needs_a_project() {
    let mut app = empty_fixture(120, 40);
    assert!(press(&mut app, Key::ch('$')).is_empty());
}
