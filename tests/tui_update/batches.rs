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

#[test]
fn delete_over_marks_confirms_once_then_runs_each_item() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    let first = app.library.row(0).unwrap().path.clone();
    let second = app.library.row(1).unwrap().path.clone();

    // `D` over marks asks for the same word, naming every folder.
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
    let id1 = run_id(&effects);
    assert!(matches!(action_of(&effects), Action::Delete(project) if project.path == first));
    assert!(app.job.is_some(), "a job is running");
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);

    // The first item lands: its row goes, and the next item starts.
    let effects = update(
        &mut app,
        item_done(
            id1,
            ListChange::Removed {
                path: first.clone(),
            },
        ),
    );
    let id2 = run_id(&effects);
    assert!(
        !app.library.marks.contains(&first),
        "a deleted row loses its mark"
    );
    assert!(matches!(action_of(&effects), Action::Delete(project) if project.path == second));

    // The second lands: the job finishes clean — no report, just the status.
    let effects = update(
        &mut app,
        item_done(
            id2,
            ListChange::Removed {
                path: second.clone(),
            },
        ),
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Run(..))),
        "the job is over, nothing else may start: {effects:?}"
    );
    assert!(app.job.is_none());
    assert!(app.modals.is_empty(), "a clean job needs no report modal");
    assert!(
        app.status.text.contains("2 deleted"),
        "{:?}",
        app.status.text
    );
    assert!(app.library.marks.is_empty());
}

#[test]
fn a_failed_item_keeps_its_mark_and_opens_a_report() {
    let mut app = fixture(12, 80, 24);
    let doomed = app.library.selected().unwrap().path.clone();
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch('D'));
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id = run_id(&effects);

    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("injected fault at 'delete:mid-copy'".to_string()),
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
            assert_eq!(title, "delete report");
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
    press(&mut app, Key::ch('D'));
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id1 = run_id(&effects);
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);

    // Esc while an item runs asks the job to stop after it — no quitting, no
    // CancelMove (nothing is in flight to cancel for a delete).
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
        app.status.text.contains("1 deleted — cancelled"),
        "{:?}",
        app.status.text
    );
    match app.modals.top() {
        Some(Modal::Message { title, lines, .. }) => {
            assert_eq!(title, "delete report");
            assert!(
                lines.iter().any(|l| l.contains("1 project is left marked")),
                "{lines:?}"
            );
        }
        other => panic!("expected the cancel report, got {other:?}"),
    }
}

#[test]
fn move_over_marks_runs_a_move_job() {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let first = app.library.row(0).unwrap().clone();

    press(&mut app, Key::ch('m'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::Move { project, .. } if **project == first
    ));
    assert!(app.job.is_some(), "the marks started a move job");
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);
    assert!(app.move_progress.is_some(), "the progress modal is up");
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
