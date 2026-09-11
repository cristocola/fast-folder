//! The message log.

use crate::harness::*;

#[test]
fn every_status_line_is_logged_and_l_reads_them_back_newest_first() {
    use fastf::tui::app::{LOG_CAP, StatusLevel};

    let mut app = fixture(3, 120, 40);
    for i in 0..(LOG_CAP + 50) {
        let _ = update(
            &mut app,
            Msg::Diag(fastf::util::diag::Level::Note, format!("note {i}")),
        );
    }
    assert_eq!(app.log.len(), LOG_CAP, "the log is bounded");
    assert!(
        app.log.front().unwrap().text.ends_with("note 50"),
        "the oldest lines go first: {:?}",
        app.log.front()
    );
    assert_eq!(app.unseen_warnings, 0, "a note on the dashboard was seen");

    // A warning that lands under a dialog is counted until the log is read.
    let _ = press(&mut app, Key::ch('a'));
    let _ = update(
        &mut app,
        Msg::Diag(fastf::util::diag::Level::Warn, "disk is full".to_string()),
    );
    assert_eq!(app.unseen_warnings, 1);
    assert!(matches!(app.log.back(), Some(entry) if entry.level == StatusLevel::Warn));
    let _ = press(&mut app, Key::plain(KeyCode::Esc));

    let _ = press(&mut app, Key::ch('L'));
    let Some(Modal::Message { title, lines, .. }) = app.modals.top() else {
        panic!("L opens the messages");
    };
    assert_eq!(title, "messages");
    assert!(
        lines[0].starts_with("10:00:00") && lines[0].contains("disk is full"),
        "newest first, stamped: {:?}",
        lines.first()
    );
    assert_eq!(app.unseen_warnings, 0, "reading the log clears the count");
}

#[test]
fn register_can_take_a_typed_date_like_the_command_line() {
    use fastf::tui::app::register::{CREATED_TYPED, FIELD_CREATED, FIELD_CREATED_DATE};

    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('e'));
    let Some(Modal::Flow(flow)) = app.modals.top() else {
        panic!("e opens the register form");
    };
    assert!(
        flow.form.field(FIELD_CREATED_DATE).unwrap().hidden,
        "the date field waits for its choice"
    );
    // Walk to the Created choice and pick "a date I type".
    while app.modals.top().is_some_and(|m| matches!(m, Modal::Flow(flow) if flow.form.focused().map(|f| f.key.as_str()) != Some(FIELD_CREATED))) {
        let _ = press(&mut app, Key::plain(KeyCode::Tab));
    }
    let _ = press(&mut app, Key::plain(KeyCode::Right));
    let _ = press(&mut app, Key::plain(KeyCode::Right));
    let Some(Modal::Flow(flow)) = app.modals.top() else {
        panic!("still the form");
    };
    assert_eq!(flow.form.value(FIELD_CREATED), CREATED_TYPED);
    assert!(!flow.form.field(FIELD_CREATED_DATE).unwrap().hidden);
    let _ = press(&mut app, Key::plain(KeyCode::Tab));
    type_text(&mut app, "2024-05-06");
    let Some(Modal::Flow(flow)) = app.modals.top() else {
        panic!("still the form");
    };
    let request = fastf::tui::app::register::request(flow);
    assert_eq!(request.created_override.as_deref(), Some("2024-05-06"));
    assert!(!request.use_today);
}
