//! Batch jobs over the marks.

use crate::harness::*;

/// A success is marked with the theme's tick, not with a literal one.
///
/// Twelve of `runtime::run_action`'s messages carried a hardcoded `✓`, which
/// `Glyphs::ascii` maps to `+` — so on a legacy Windows console, or anywhere
/// with `FASTF_ASCII=1`, they drew a replacement box beside the app's own
/// correctly-themed messages. `run_action` runs on a worker with no theme to
/// ask; `App::good` is the one place all of them pass through.
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
