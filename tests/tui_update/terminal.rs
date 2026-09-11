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
