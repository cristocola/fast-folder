//! Batch jobs over the marks.

use crate::harness::*;

/// A success is marked with the theme's tick, not with a literal one.
///
/// `runtime::run_action` runs on a worker with no theme to ask, so its
/// messages carry no glyph; `App::good` is the one place all of them pass
/// through, and it asks the theme. A literal `✓` draws a replacement box on a
/// legacy Windows console, or anywhere with `FASTF_ASCII=1`, where
/// `Glyphs::ascii` spells the tick `+`.
#[test]
fn a_success_message_wears_the_theme_glyph_not_a_hardcoded_one() {
    use fastf::tui::theme::{Glyphs, Theme};

    let mut app = fixture(4, 100, 30);
    app.theme = Theme::mono().with_glyphs(Glyphs::ascii());

    let effects = press(&mut app, Key::ch('A'));
    let effects = if effects.iter().any(|e| matches!(e, Effect::Run(_, _))) {
        effects
    } else {
        // `A` opens a tag picker first; answer it.
        press(&mut app, Key::plain(KeyCode::Enter))
    };
    let id = run_id(&effects);
    let _ = update(&mut app, item_done(id, ListChange::SummaryOnly));

    let shown = &app.status.text;
    assert!(
        !shown.contains(Glyphs::unicode().check),
        "the ASCII theme must not draw a unicode tick: {shown:?}"
    );
    assert!(
        shown.starts_with(Glyphs::ascii().check),
        "and it must draw its own: {shown:?}"
    );
}

/// **A delete acts on the folders its question named, and on no other.** The
/// list can change under an open prompt: a discovery lands, and the marked
/// rows are gone from it. The word typed then confirms nothing about the row
/// the cursor happens to be on.
#[test]
fn a_delete_never_reaches_a_project_its_question_did_not_name() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1; the cursor is on row 2
    let named = [
        app.library.row(0).unwrap().clone(),
        app.library.row(1).unwrap().clone(),
    ];
    let under_the_cursor = app.library.selected().unwrap().clone();
    assert!(
        named
            .iter()
            .all(|project| project.path != under_the_cursor.path)
    );

    press(&mut app, Key::plain(KeyCode::F(5)));
    let generation = app.library.inflight.expect("a discovery is in flight");
    press(&mut app, Key::ch('D'));
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("delete asks in a text prompt");
    };
    assert!(
        prompt.title.contains("these 2 projects"),
        "{}",
        prompt.title
    );

    // The discovery lands without the two the question named.
    let left: Vec<_> = sample_projects(12)
        .into_iter()
        .filter(|project| named.iter().all(|gone| gone.path != project.path))
        .collect();
    update(
        &mut app,
        Msg::Discovered {
            generation,
            projects: left,
        },
    );

    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        job_started(&effects).is_none(),
        "nothing the question named is left, so nothing is deleted: {effects:?}"
    );
    assert!(app.modals.is_empty() || !matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
}

/// Two marks with the cursor on a third row, `open` pressed, and then a
/// discovery that holds neither marked project: what the answer starts.
fn answered_once_its_projects_left(
    open: Key,
    answer: impl Fn(&mut App) -> Vec<Effect>,
) -> (App, Vec<Effect>) {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let named = [
        app.library.row(0).unwrap().path.clone(),
        app.library.row(1).unwrap().path.clone(),
    ];

    press(&mut app, Key::plain(KeyCode::F(5)));
    let generation = app.library.inflight.expect("a discovery is in flight");
    press(&mut app, open);
    assert!(!app.modals.is_empty(), "{open:?} asks first");
    let left: Vec<_> = sample_projects(12)
        .into_iter()
        .filter(|project| !named.contains(&project.path))
        .collect();
    update(
        &mut app,
        Msg::Discovered {
            generation,
            projects: left,
        },
    );
    assert!(app.library.selected().is_some(), "the cursor is on a row");
    let effects = answer(&mut app);
    (app, effects)
}

/// **Every verb that asks first acts on what its question was about**: the
/// same rule as the delete's, for the verbs that lose nothing. The row under
/// the cursor was never asked about, so it is not moved, copied, unregistered,
/// tagged or noted.
#[test]
fn no_verb_reaches_a_project_its_question_was_not_about() {
    type Answer = fn(&mut App) -> Vec<Effect>;
    let verbs: [(&str, Key, Answer); 6] = [
        ("unregister", Key::ch('u'), |app| press(app, Key::ch('y'))),
        ("move", Key::ch('m'), |app| {
            press(app, Key::plain(KeyCode::Enter))
        }),
        ("copy", Key::ch('C'), |app| {
            type_text(app, "/mnt/backups");
            press(app, Key::plain(KeyCode::Enter))
        }),
        ("add a tag", Key::ch('A'), |app| {
            press(app, Key::plain(KeyCode::Enter))
        }),
        ("remove tags", Key::ctrl('t'), |app| {
            press(app, Key::ch(' '));
            press(app, Key::plain(KeyCode::Enter))
        }),
        ("note", Key::ctrl('n'), |app| {
            type_text(app, "graded");
            press(app, Key::plain(KeyCode::Enter))
        }),
    ];
    for (verb, open, answer) in verbs {
        let (app, effects) = answered_once_its_projects_left(open, answer);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Run(..) | Effect::StartJob { .. })),
            "{verb}: nothing it asked about is left, so nothing runs: {effects:?}"
        );
        assert!(app.job.is_none(), "{verb}: and no batch is begun");
        assert!(
            app.status.text.contains("no longer in the library"),
            "{verb}: and it says why: {:?}",
            app.status.text
        );
    }
}

/// And when only some of them are left, those are what is deleted.
#[test]
fn a_delete_takes_what_is_left_of_what_its_question_named() {
    use fastf::core::jobs::JobKind;

    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let first = app.library.row(0).unwrap().clone();
    let second = app.library.row(1).unwrap().clone();

    press(&mut app, Key::plain(KeyCode::F(5)));
    let generation = app.library.inflight.expect("a discovery is in flight");
    press(&mut app, Key::ch('D'));
    let left: Vec<_> = sample_projects(12)
        .into_iter()
        .filter(|project| project.path != first.path)
        .collect();
    update(
        &mut app,
        Msg::Discovered {
            generation,
            projects: left,
        },
    );

    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let (kind, items) = job_started(&effects).expect("one job");
    assert_eq!(kind, JobKind::Delete);
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, [second.id.as_str()]);
}

/// Delete over marks asks for the word once, naming the count, and starts
/// one job over every marked project — a job of its own, which goes on if
/// the app is closed. The marks go as its items land.
#[test]
fn delete_over_marks_confirms_once_then_starts_one_job() {
    use fastf::core::assets::{JobStatus, Progress};
    use fastf::core::jobs::JobKind;

    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    let first = app.library.row(0).unwrap().clone();
    let second = app.library.row(1).unwrap().clone();

    press(&mut app, Key::ch('D'));
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("delete asks in a text prompt");
    };
    assert!(
        prompt.title.contains("these 2 projects"),
        "the question counts the marks: {}",
        prompt.title
    );
    type_text(&mut app, "DELETE");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let (kind, items) = job_started(&effects).expect("one job");
    assert_eq!(kind, JobKind::Delete);
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, vec![first.id.as_str(), second.id.as_str()]);
    assert!(app.job.is_none(), "not the app's own batch");

    // The job ends with one item through and one failed: that one keeps its
    // mark, for a retry.
    let _ = update(&mut app, Msg::JobStarted(Ok("18d8-1-0".to_string())));
    let mut ended = fastf::tui::testing::job_view(
        "18d8-1-0",
        JobKind::Delete,
        &[(&first.id, &first.name), (&second.id, &second.name)],
        Progress::new(&[]),
        false,
        JobStatus::Failed,
    );
    if let Some(request) = ended.request.as_mut() {
        request.items[0].path = first.path.display().to_string();
        request.items[1].path = second.path.display().to_string();
    }
    let state = ended.state.as_mut().unwrap();
    state.summary = "deleted 1 of 2, 1 failed".to_string();
    state.items[0].done = true;
    state.items[1].done = true;
    state.items[1].headline = format!("The delete of {} failed", second.id);
    state.items[1].error = Some("a program has a file in it open".to_string());
    let _ = update(&mut app, Msg::Jobs(vec![ended]));
    assert!(
        !app.library.marks.contains(&first.path),
        "through: unmarked"
    );
    assert!(
        app.library.marks.contains(&second.path),
        "failed: still marked"
    );
    assert!(app.status.text.contains("1 failed"), "{}", app.status.text);
    assert!(matches!(
        app.modals.top(),
        Some(Modal::Message { title, .. }) if title == "error"
    ));
}

#[test]
fn a_failed_item_keeps_its_mark_and_opens_a_report() {
    let mut app = fixture(12, 80, 24);
    let doomed = app.library.selected().unwrap().path.clone();
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch('u'));
    let effects = press(&mut app, Key::ch('y'));
    let id = run_id(&effects);

    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("injected fault at 'unregister:mid-copy'".to_string()),
        },
    );
    assert!(effects.is_empty());
    assert!(app.job.is_none(), "the job is over");
    assert!(
        app.library.marks.contains(&doomed),
        "the failed row stays marked for a retry"
    );
    assert!(
        app.status.text.contains("1 failed"),
        "{:?}",
        app.status.text
    );
    match app.modals.top() {
        Some(Modal::Message { title, lines, .. }) => {
            assert_eq!(title, "unregister report");
            assert!(lines.iter().any(|l| l.contains("mid-copy")), "{lines:?}");
        }
        other => panic!("expected the failure report, got {other:?}"),
    }
}

#[test]
fn esc_cancels_a_job_and_the_rest_stay_marked() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    let first = app.library.row(0).unwrap().path.clone();
    let second = app.library.row(1).unwrap().path.clone();
    press(&mut app, Key::ch('u'));
    let effects = press(&mut app, Key::ch('y'));
    let id1 = run_id(&effects);
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);

    // Esc while an item runs asks the job to stop after it — no quitting.
    assert!(press(&mut app, Key::plain(KeyCode::Esc)).is_empty());
    assert!(app.job.as_ref().unwrap().cancelled);

    // The running item still lands: its row goes, its mark with it. Nothing
    // new starts, and the unrun row keeps its mark.
    let effects = update(
        &mut app,
        item_done(
            id1,
            ListChange::Removed {
                path: first.clone(),
            },
        ),
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Run(..))),
        "no further item may start: {effects:?}"
    );
    assert!(app.job.is_none(), "the cancelled job is over");
    assert!(!app.library.marks.contains(&first));
    assert!(
        app.library.marks.contains(&second),
        "the unrun row stays marked"
    );
    assert!(app.library.snapshot.iter().any(|p| p.path == second));
    assert!(
        app.status.text.contains("1 unregistered — cancelled"),
        "{:?}",
        app.status.text
    );
    match app.modals.top() {
        Some(Modal::Message { title, lines, .. }) => {
            assert_eq!(title, "unregister report");
            assert!(
                lines.iter().any(|l| l.contains("1 project is left marked")),
                "{lines:?}"
            );
        }
        other => panic!("expected the cancel report, got {other:?}"),
    }
}

/// A move over marks is one job over every marked project: moves first,
/// every old copy removed after.
#[test]
fn move_over_marks_starts_one_move_job() {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let first = app.library.row(0).unwrap().clone();

    press(&mut app, Key::ch('m'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let (kind, items) = job_started(&effects).expect("one job");
    assert_eq!(kind, fastf::core::jobs::JobKind::Move);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, first.id);
    assert!(app.job.is_none(), "not the app's own batch");
    assert!(app.shown_progress().is_some(), "the dialog is up");
}

#[test]
fn unregister_over_marks_confirms_the_count_then_runs() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch('u'));
    let prompt = match app.modals.top() {
        Some(Modal::Confirm(confirm)) => confirm.prompt.clone(),
        other => panic!("expected a batch confirm, got {other:?}"),
    };
    assert!(prompt.contains("2 projects"), "{prompt}");
    let effects = press(&mut app, Key::ch('y'));
    assert!(matches!(action_of(&effects), Action::Unregister(_)));
    assert!(app.job.is_some());
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);
}
