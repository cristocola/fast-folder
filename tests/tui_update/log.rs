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

    let effects = press(&mut app, Key::ch('L'));
    assert_eq!(
        effects,
        vec![Effect::LoadActivity, Effect::WatchJobs],
        "and asks for every session's, and the jobs"
    );
    let Some(Modal::Activity(activity)) = app.modals.top() else {
        panic!("L opens the messages");
    };
    let lines = &activity.messages;
    assert!(
        lines[0].starts_with("10:00:00") && lines[0].contains("disk is full"),
        "this session's at once, newest first, stamped: {:?}",
        lines.first()
    );
    assert_eq!(app.unseen_warnings, 0, "reading the log clears the count");
}

/// Every status line is handed to the runtime to keep, in the order it was
/// said — `update` writes nothing itself.
#[test]
fn every_status_line_is_handed_over_to_be_kept() {
    use fastf::util::messages::Level;

    let mut app = fixture(3, 120, 40);
    app.outbox.clear();
    let _ = update(
        &mut app,
        Msg::Diag(fastf::util::diag::Level::Warn, "disk is full".to_string()),
    );
    let kept = app.outbox.last().expect("the warning was handed over");
    assert_eq!(kept.level, Level::Warn);
    assert_eq!(kept.source, "app");
    assert!(kept.text.contains("disk is full"));
}

/// Tab turns the activity screen's page, and each page keeps its own place;
/// a read landing while it is open fills both pages in.
#[test]
fn the_activity_screen_turns_its_page_and_fills_in_when_read() {
    use fastf::tui::app::modal::ActivityPage;
    use fastf::util::messages::{Level, Message};

    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('L'));
    let _ = update(
        &mut app,
        Msg::ActivityLoaded {
            messages: vec![Message {
                at: "then".to_string(),
                level: Level::Good,
                source: "cli".to_string(),
                text: "moved ID0001".to_string(),
            }],
            log: (0..300).map(|n| format!("T INFO  - 1 event {n}")).collect(),
        },
    );
    let _ = press(&mut app, Key::plain(KeyCode::Tab));
    let _ = press(&mut app, Key::plain(KeyCode::Tab));
    let _ = press(&mut app, Key::plain(KeyCode::PageDown));
    let Some(Modal::Activity(activity)) = app.modals.top() else {
        panic!("still open");
    };
    assert_eq!(activity.page, ActivityPage::Log);
    assert!(activity.log[0].ends_with("event 299"), "newest first");
    assert!(activity.scroll[2] > 0 && activity.scroll[0] == 0);
    assert!(
        activity.messages[0].contains("moved ID0001") && activity.messages[0].contains("(cli)")
    );
    let _ = press(&mut app, Key::plain(KeyCode::BackTab));
    let Some(Modal::Activity(activity)) = app.modals.top() else {
        panic!("still open");
    };
    assert_eq!(activity.page, ActivityPage::Jobs);
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
