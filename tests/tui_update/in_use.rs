//! A move, a delete and a rename ask what uses their folders first, and a
//! folder something uses gets a picker before the verb starts.

use crate::harness::*;
use fastf::core::jobs::JobKind;
use fastf::tui::app::modal::PickState;

/// The labels of the picker on top.
fn choices(app: &App) -> Vec<String> {
    match app.modals.top() {
        Some(Modal::Pick(pick)) => pick.items.iter().map(|item| item.label.clone()).collect(),
        other => panic!("the in-use picker is up, not {other:?}"),
    }
}

fn picker(app: &App) -> &PickState {
    match app.modals.top() {
        Some(Modal::Pick(pick)) => pick,
        other => panic!("the in-use picker is up, not {other:?}"),
    }
}

/// Move the picker's cursor to the row labelled `label` and take it.
fn choose(app: &mut App, label: &str) -> Vec<Effect> {
    let at = choices(app)
        .iter()
        .position(|choice| choice == label)
        .unwrap_or_else(|| panic!("no {label} in {:?}", choices(app)));
    for _ in 0..at {
        press(app, Key::plain(KeyCode::Down));
    }
    press(app, Key::plain(KeyCode::Enter))
}

/// `m` and the first base, answered up to the question.
fn ask_to_move(app: &mut App) -> Vec<Effect> {
    app.summary = Some(sample_summary_moveable(12));
    press(app, Key::ch('m'));
    press(app, Key::plain(KeyCode::Enter))
}

#[test]
fn a_busy_project_asks_and_anyway_forces_the_move() {
    let mut app = fixture(12, 100, 30);
    let selected = app.library.selected().unwrap().clone();
    let effects = ask_to_move(&mut app);
    assert!(
        job_started(&effects).is_none(),
        "nothing starts before the answer"
    );
    let (_, asked) = in_use_asked(&effects).expect("what uses it is asked");
    assert_eq!(asked, std::slice::from_ref(&selected.path));

    let effects = answer_in_use(&mut app, &effects, &[(&selected.path, vec![sitter(4242)])]);
    assert!(job_started(&effects).is_none());
    assert!(
        !app.job_dialog_up(),
        "the progress dialog gives way to the question"
    );
    assert_eq!(picker(&app).title, format!("{} is in use", selected.id));
    assert_eq!(choices(&app), ["Move anyway", "Cancel"]);
    let detail = picker(&app).selected_detail().unwrap().to_string();
    assert!(
        detail.contains("node (pid 4242) is working in src"),
        "{detail}"
    );

    let effects = choose(&mut app, "Move anyway");
    let (kind, items) = job_started(&effects).expect("the move starts");
    assert_eq!(kind, JobKind::Move);
    assert_eq!(items[0].id, selected.id);
    assert_eq!(job_forced(&effects), Some(true));
}

#[test]
fn a_batch_with_one_busy_project_can_go_on_with_the_others() {
    let mut app = fixture(12, 100, 30);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let busy = app.library.row(1).unwrap().clone();
    let effects = ask_to_move(&mut app);
    let effects = answer_in_use(&mut app, &effects, &[(&busy.path, vec![sitter(7)])]);
    assert!(job_started(&effects).is_none());
    assert_eq!(picker(&app).title, "1 of 3 projects is in use");
    assert_eq!(
        choices(&app),
        ["Move anyway", "Move the 2 others", "Cancel"]
    );
    assert!(
        picker(&app)
            .selected_detail()
            .unwrap()
            .starts_with(&format!("{}: node (pid 7)", busy.id)),
        "{:?}",
        picker(&app).selected_detail()
    );

    let effects = choose(&mut app, "Move the 2 others");
    let (_, items) = job_started(&effects).expect("the others move");
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|item| item.id != busy.id), "{items:?}");
    assert_eq!(job_forced(&effects), Some(false));
}

#[test]
fn cancel_and_esc_move_nothing_and_say_so() {
    for how in ["cancel", "esc"] {
        let mut app = fixture(12, 100, 30);
        let selected = app.library.selected().unwrap().clone();
        let effects = ask_to_move(&mut app);
        answer_in_use(&mut app, &effects, &[(&selected.path, vec![sitter(1)])]);
        let effects = if how == "cancel" {
            choose(&mut app, "Cancel")
        } else {
            press(&mut app, Key::plain(KeyCode::Esc))
        };
        assert!(job_started(&effects).is_none(), "{how}: {effects:?}");
        assert!(app.modals.is_empty(), "{how}");
        assert!(
            app.status.text.contains("nothing was moved"),
            "{how}: {}",
            app.status.text
        );
    }
}

/// An answer to an older question is dropped; the current one is waited for.
#[test]
fn an_answer_to_another_question_is_dropped() {
    let mut app = fixture(12, 100, 30);
    let selected = app.library.selected().unwrap().clone();
    let effects = ask_to_move(&mut app);
    let (request, _) = in_use_asked(&effects).unwrap();
    let stale = update(
        &mut app,
        Msg::InUse {
            request: request + 7,
            found: Vec::new(),
        },
    );
    assert!(stale.is_empty(), "{stale:?}");
    assert!(app.job_dialog_up(), "still asking");
    let effects = nothing_uses_them(&mut app, &effects);
    let (_, items) = job_started(&effects).expect("the current answer starts it");
    assert_eq!(items[0].id, selected.id);
}

/// Ctrl-C while the folder is asked about: the answer starts nothing, and a
/// later job is not cancelled by a flag left behind.
#[test]
fn ctrl_c_during_the_question_starts_nothing() {
    let mut app = fixture(12, 100, 30);
    let effects = ask_to_move(&mut app);
    assert_eq!(press(&mut app, Key::ctrl('c')), vec![]);
    let answered = nothing_uses_them(&mut app, &effects);
    assert!(job_started(&answered).is_none(), "{answered:?}");
    assert!(!app.job_dialog_up());
    assert!(
        app.status.text.contains("nothing was moved"),
        "{}",
        app.status.text
    );

    let effects = ask_to_move(&mut app);
    let effects = nothing_uses_them(&mut app, &effects);
    assert!(job_started(&effects).is_some(), "the next move starts");
    let effects = update(&mut app, Msg::JobStarted(Ok("18d8-1-0".to_string())));
    assert!(
        !effects.contains(&Effect::CancelJob("18d8-1-0".to_string())),
        "and is not cancelled: {effects:?}"
    );
}

/// One question at a time: a second verb while the first is being asked
/// about is refused, and the first's answer still lands.
#[test]
fn a_second_question_waits_for_the_first() {
    let mut app = fixture(12, 100, 30);
    let effects = ask_to_move(&mut app);
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(!app.job_dialog_up(), "Esc hides the dialog while it asks");
    let again = ask_to_move(&mut app);
    assert!(in_use_asked(&again).is_none(), "{again:?}");
    let effects = nothing_uses_them(&mut app, &effects);
    assert!(job_started(&effects).is_some());
    assert!(app.job_dialog_up(), "the answer shows the dialog again");
}

#[test]
fn a_delete_asks_the_same_way() {
    let mut app = fixture(12, 100, 30);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('D'));
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(app.shown_title(), "deleting…");
    let effects = answer_in_use(&mut app, &effects, &[(&selected.path, vec![sitter(9)])]);
    assert!(job_started(&effects).is_none());
    assert_eq!(choices(&app), ["Delete anyway", "Cancel"]);
    let effects = choose(&mut app, "Delete anyway");
    assert!(matches!(
        job_started(&effects),
        Some((JobKind::Delete, [_]))
    ));
    assert_eq!(job_forced(&effects), Some(true));
}

#[test]
fn a_rename_asks_and_has_no_others() {
    let mut app = fixture(12, 100, 30);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('r'));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "New_Name");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Run(..))),
        "{effects:?}"
    );
    answer_in_use(&mut app, &effects, &[(&selected.path, vec![sitter(3)])]);
    assert_eq!(choices(&app), ["Rename anyway", "Cancel"]);
    let effects = choose(&mut app, "Rename anyway");
    assert!(matches!(
        action_of(&effects),
        Action::Rename { project, name, force: true }
            if **project == selected && name == "New_Name"
    ));
}

/// What nothing goes past is offered no "anyway".
#[test]
fn a_physical_use_offers_no_anyway() {
    let mut app = fixture(12, 100, 30);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('D'));
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let mounted = fastf::core::holders::Use {
        by: fastf::core::holders::By::Nobody,
        how: fastf::core::holders::How::MountedInside(PathBuf::from("remote")),
    };
    answer_in_use(&mut app, &effects, &[(&selected.path, vec![mounted])]);
    assert_eq!(choices(&app), ["Cancel"]);
    assert!(
        picker(&app)
            .selected_detail()
            .unwrap()
            .contains("remote is another filesystem"),
        "{:?}",
        picker(&app).selected_detail()
    );
}
