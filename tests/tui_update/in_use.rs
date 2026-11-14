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
    let from = picker(app).selected.expect("the picker has a cursor");
    let key = if at < from {
        KeyCode::Up
    } else {
        KeyCode::Down
    };
    for _ in 0..at.abs_diff(from) {
        press(app, Key::plain(key));
    }
    press(app, Key::plain(KeyCode::Enter))
}

/// The label of the row under the picker's cursor.
fn under_cursor(app: &App) -> String {
    picker(app)
        .chosen()
        .map(|item| item.label.clone())
        .unwrap_or_default()
}

/// Ask `verb` about the selected project and answer that a program works
/// in it: the picker is up.
fn busy_question(app: &mut App, verb: &str) {
    let selected = app.library.selected().unwrap().clone();
    let effects = match verb {
        "move" => ask_to_move(app),
        "delete" => {
            press(app, Key::ch('D'));
            type_text(app, "delete");
            press(app, Key::plain(KeyCode::Enter))
        }
        _ => {
            press(app, Key::ch('r'));
            press(app, Key::ctrl('u'));
            type_text(app, "New_Name");
            press(app, Key::plain(KeyCode::Enter))
        }
    };
    answer_in_use(app, &effects, &[(&selected.path, vec![sitter(9)])]);
}

/// Whether the effects start the verb: a job, or the rename's write.
fn starts_something(effects: &[Effect]) -> bool {
    job_started(effects).is_some()
        || effects
            .iter()
            .any(|effect| matches!(effect, Effect::Run(..)))
}

/// **One Enter on the question goes nowhere.** The cursor starts on
/// Cancel, as the command line's same question defaults to no, so going
/// past a use is a deliberate move of the cursor; the rows keep their
/// order.
#[test]
fn one_enter_on_the_question_moves_deletes_and_renames_nothing() {
    for (verb, past) in [
        ("move", "moved"),
        ("delete", "deleted"),
        ("rename", "renamed"),
    ] {
        let mut app = fixture(12, 100, 30);
        busy_question(&mut app, verb);
        assert_eq!(choices(&app).last().map(String::as_str), Some("Cancel"));
        assert_eq!(under_cursor(&app), "Cancel", "{verb}");
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(!starts_something(&effects), "{verb}: {effects:?}");
        assert!(app.modals.is_empty(), "{verb}");
        assert!(
            app.status.text.contains(&format!("nothing was {past}")),
            "{verb}: {}",
            app.status.text
        );
    }
}

/// **Typing into the picker keeps the cursor off the verb.** Keys meant for
/// something else land in its filter when the question comes up under
/// them; while Cancel is among the matches the cursor stays on it, and with
/// nothing matching, Enter still says nothing was moved.
#[test]
fn typing_into_the_question_leaves_the_cursor_on_cancel() {
    let mut app = fixture(12, 100, 30);
    busy_question(&mut app, "move");
    type_text(&mut app, "node");
    assert_eq!(choices(&app), ["Move anyway", "Cancel"], "both still match");
    assert_eq!(under_cursor(&app), "Cancel");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(!starts_something(&effects), "{effects:?}");
    assert!(
        app.status.text.contains("nothing was moved"),
        "{}",
        app.status.text
    );

    let mut app = fixture(12, 100, 30);
    busy_question(&mut app, "move");
    type_text(&mut app, "zqzq");
    assert!(picker(&app).ranked.is_empty(), "nothing matches");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(!starts_something(&effects), "{effects:?}");
    assert!(app.modals.is_empty());
    assert!(
        app.status.text.contains("nothing was moved"),
        "{}",
        app.status.text
    );
}

/// **A rename's answer leaves a job started meanwhile its own Ctrl-C.** A
/// rename puts no dialog up, so a copy can start while it is asked about
/// and be cancelled on its dialog before it has a worker: the rename's
/// answer still renames, and the copy is cancelled the moment it starts.
#[test]
fn a_renames_answer_leaves_a_copys_cancel_to_the_copy() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('r'));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "New_Name");
    let rename = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(in_use_asked(&rename).is_some());
    press(&mut app, Key::ch('C'));
    // An absolute path where the test runs: the prompt refuses `/mnt/backup`
    // on Windows.
    type_text(
        &mut app,
        if cfg!(windows) {
            r"C:\backup"
        } else {
            "/mnt/backup"
        },
    );
    let copy = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(job_started(&copy), Some((JobKind::Copy, [_]))));
    press(&mut app, Key::ctrl('c'));

    let answered = nothing_uses_them(&mut app, &rename);
    assert!(
        matches!(action_of(&answered), Action::Rename { name, .. } if name == "New_Name"),
        "the rename goes ahead: {answered:?} ({})",
        app.status.text
    );
    let started = update(&mut app, Msg::JobStarted(Ok("18d8-1-0".to_string())));
    assert!(
        started.contains(&Effect::CancelJob("18d8-1-0".to_string())),
        "and the copy is cancelled as it starts: {started:?}"
    );
}

/// **A refusal says what it waits for**: a question still out is named as
/// one, and a job still starting by its own word.
#[test]
fn a_refused_question_names_what_it_waits_for() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('r'));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "New_Name");
    let rename = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(in_use_asked(&rename).is_some());
    let again = ask_to_move(&mut app);
    assert!(in_use_asked(&again).is_none(), "{again:?}");
    assert_eq!(
        app.status.text,
        "still checking what uses the last folders asked about"
    );

    // The rename prompt is up when a job starts: no key starts one under a
    // prompt, so the state is set as the job's start would leave it.
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('r'));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "New_Name");
    app.background.starting = Some("copying…");
    let refused = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(in_use_asked(&refused).is_none(), "{refused:?}");
    assert_eq!(app.status.text, "still copying…");
}

/// **A rename's question leaves a hidden job hidden, and Ctrl-C on the
/// in-use picker closes the picker**, saying nothing was renamed, rather
/// than cancelling the job the dialog follows.
#[test]
fn ctrl_c_on_a_renames_picker_closes_it_and_leaves_the_hidden_job_alone() {
    use fastf::core::assets::Progress;
    let mut app = fixture(12, 100, 30);
    let id = fastf::tui::testing::follow_job(&mut app, JobKind::Move, Progress::new(&[]));
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(!app.job_dialog_up());

    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('r'));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "New_Name");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(!app.job_dialog_up(), "the rename un-hid the move's dialog");
    answer_in_use(&mut app, &effects, &[(&selected.path, vec![sitter(3)])]);
    assert_eq!(choices(&app), ["Rename anyway", "Cancel"]);
    assert!(!app.job_dialog_up());

    let effects = press(&mut app, Key::ctrl('c'));
    assert!(
        !effects.contains(&Effect::CancelJob(id.clone())),
        "Ctrl-C cancelled the move: {effects:?}"
    );
    assert!(app.modals.is_empty(), "the picker is closed");
    assert!(
        app.status.text.contains("nothing was renamed"),
        "{}",
        app.status.text
    );
}

/// **A move's question gives the dialog back as it found it**: the hidden
/// job it borrowed the dialog from stays hidden behind the picker, and
/// Ctrl-C there closes the picker, not the job.
#[test]
fn a_moves_question_leaves_a_hidden_job_hidden_behind_its_picker() {
    use fastf::core::assets::Progress;
    let mut app = fixture(12, 100, 30);
    let id = fastf::tui::testing::follow_job(&mut app, JobKind::Move, Progress::new(&[]));
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(!app.job_dialog_up());

    busy_question(&mut app, "move");
    assert_eq!(choices(&app), ["Move anyway", "Cancel"]);
    assert!(
        !app.job_dialog_up(),
        "the hidden job's dialog came back over the question"
    );
    let effects = press(&mut app, Key::ctrl('c'));
    assert!(
        !effects.contains(&Effect::CancelJob(id.clone())),
        "Ctrl-C cancelled the other job: {effects:?}"
    );
    assert!(app.modals.is_empty(), "the picker is closed");
    assert!(
        app.status.text.contains("nothing was moved"),
        "{}",
        app.status.text
    );
    assert!(!app.job_dialog_up(), "and the other job is still hidden");
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
    assert_eq!(app.status.text, "checking what uses it…");
    answer_in_use(&mut app, &effects, &[(&selected.path, vec![sitter(3)])]);
    assert_eq!(
        app.status.text, "",
        "the answer is on screen, so the status line no longer says it is coming"
    );
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
