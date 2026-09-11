//! Marks: what a batch verb will act on.

use crate::harness::*;

#[test]
fn space_toggles_the_selected_row_and_steps_on() {
    let mut app = fixture(12, 80, 24);
    let first = app.library.selected().unwrap().path.clone();

    press(&mut app, Key::ch(' '));
    assert!(app.library.marks.contains(&first), "Space marks the row");
    assert_eq!(app.library.selected_index(), Some(1), "and moves on");

    let second = app.library.selected().unwrap().path.clone();
    press(&mut app, Key::ch(' '));
    assert!(app.library.marks.contains(&second));

    // Back to the first row: Space unmarks it and steps on again.
    press(&mut app, Key::plain(KeyCode::Home));
    press(&mut app, Key::ch(' '));
    assert!(!app.library.marks.contains(&first));
    assert_eq!(app.library.selected_index(), Some(1));
}

#[test]
fn mark_all_marks_what_the_current_view_shows() {
    let mut app = fixture(12, 80, 24);
    // Narrow the view to the two Lullaby rows.
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "lullaby");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(app.library.len(), 2, "the query narrows the view");
    let visible: Vec<PathBuf> = (0..app.library.len())
        .filter_map(|row| app.library.row(row).map(|p| p.path.clone()))
        .collect();
    let hidden = app
        .library
        .snapshot
        .iter()
        .find(|p| !visible.contains(&p.path))
        .unwrap()
        .path
        .clone();

    press(&mut app, Key::ch('*'));
    assert_eq!(
        app.library.marks.len(),
        2,
        "only what the view shows is marked"
    );
    assert!(visible.iter().all(|p| app.library.marks.contains(p)));
    assert!(!app.library.marks.contains(&hidden));
    assert!(
        app.status.text.contains("2 marked"),
        "{:?}",
        app.status.text
    );
}

#[test]
fn minus_clears_the_marks_and_says_so() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    assert_eq!(app.library.marks.len(), 2);

    press(&mut app, Key::ch('-'));
    assert!(app.library.marks.is_empty());
    assert!(
        app.status.text.contains("2 marks cleared"),
        "{:?}",
        app.status.text
    );
}

#[test]
fn esc_clears_marks_before_it_quits() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' '));
    assert_eq!(app.library.marks.len(), 1);

    assert!(press(&mut app, Key::plain(KeyCode::Esc)).is_empty());
    assert!(
        app.library.marks.is_empty(),
        "the first Esc clears the marks"
    );
    assert!(app.status.text.contains("marks cleared"));
    assert_eq!(
        press(&mut app, Key::plain(KeyCode::Esc)),
        vec![Effect::Quit(Exit::Normal)],
        "the second Esc quits"
    );
}

#[test]
fn targets_are_the_marks_in_display_order() {
    let mut app = fixture(12, 80, 24);
    // Marks are a set; the order a job runs in comes from the view.
    let last = app.library.row(app.library.len() - 1).unwrap().path.clone();
    let first = app.library.row(0).unwrap().path.clone();
    app.library.marks.insert(last.clone());
    app.library.marks.insert(first.clone());

    let targets = app.library.targets();
    let paths: Vec<PathBuf> = targets.iter().map(|p| p.path.clone()).collect();
    assert_eq!(paths, vec![first, last], "marks run in display order");
}

#[test]
fn removing_a_row_drops_its_mark() {
    let mut app = fixture(12, 80, 24);
    let doomed = app.library.selected().unwrap().path.clone();
    press(&mut app, Key::ch(' '));
    assert!(app.library.marks.contains(&doomed));

    // What the engine does when a delete completes: the row goes, and with it
    // its mark — a deleted project cannot stay a batch target.
    app.library.remove(&doomed);
    assert!(!app.library.marks.contains(&doomed));
}
