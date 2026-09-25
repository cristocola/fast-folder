//! Single-project actions.

use crate::harness::*;

#[test]
fn rederive_rename_and_move_each_run_their_action() {
    use fastf::tui::command::CommandId;

    // Re-derive tags: no prompt, straight to the worker.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    let effects = app.run(CommandId::ReautoTags);
    assert!(matches!(
        action_of(&effects),
        Action::ReautoTags(p) if **p == selected
    ));

    // Rename: a text prompt pre-filled with the current name.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('r'));
    assert!(matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "New_Name");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::Rename { project, name } if **project == selected && name == "New_Name"
    ));
}

#[test]
fn add_and_remove_tags_run_their_actions() {
    // Add: the library already knows `client/Acme`, which the selected project
    // lacks, so `A` offers it in a picker.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('A'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::AddTag { project, tag } if **project == selected && tag == "client/Acme"
    ));

    // Remove: a multi-pick of the project's own tags, Space toggles.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ctrl('t'));
    assert!(matches!(app.modals.top(), Some(Modal::MultiPick(_))));
    press(&mut app, Key::ch(' '));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::RemoveTags { project, tags } if **project == selected && tags == &vec!["draft".to_string()]
    ));
}

/// A move is a job of its own: the app starts it and follows it, the dialog
/// up — "moving", blank — until its worker answers.
#[test]
fn move_picks_a_target_and_starts_a_move_job() {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('m'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let (kind, items) = job_started(&effects).expect("a job starts");
    assert_eq!(kind, fastf::core::jobs::JobKind::Move);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, selected.id);
    assert_eq!(Path::new(&items[0].target), Path::new("/media/usb/archive"));
    assert!(
        app.busy.is_none(),
        "the app is not busy: the job is not its work"
    );
    assert_eq!(app.shown_title(), "moving…");
    assert!(app.shown_progress().is_some(), "the dialog is up");

    let effects = update(&mut app, Msg::JobStarted(Ok("18d8-1-0".to_string())));
    assert!(effects.contains(&Effect::WatchJobs));
    assert_eq!(app.background.following.as_deref(), Some("18d8-1-0"));
}

#[test]
fn notes_run_their_actions_and_the_editor_suspends() {
    // The quick note types inline and appends.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ctrl('n'));
    assert!(matches!(app.modals.top(), Some(Modal::Note(_))));
    type_text(&mut app, "mixing started");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::AppendNote { project, text } if **project == selected && text == "mixing started"
    ));

    // `N` opens $EDITOR, which runs while the screen is suspended.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    let effects = press(&mut app, Key::ch('N'));
    assert!(matches!(
        effects.as_slice(),
        [Effect::Suspend(Suspended::Note(project))] if **project == selected
    ));
}

#[test]
fn an_action_done_patch_forgets_the_stale_sizes() {
    use fastf::tui::effect::ActionOutcome;

    let mut app = fixture(12, 80, 24);
    let selected_path = app.library.selected().unwrap().path.clone();
    app.library.sizes.insert(selected_path.clone(), Some(10));

    // Start an add-tag action to get a busy id.
    press(&mut app, Key::ch('A'));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id = match effects.as_slice() {
        [Effect::Run(id, _)] => *id,
        other => panic!("{other:?}"),
    };
    assert!(app.busy.is_some());

    let mut patched = app.library.selected().unwrap().clone();
    patched.tags.push("client/Acme".to_string());
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(patched),
                    was: selected_path.clone(),
                    stale: vec![selected_path.clone()],
                },
                "Added 1 tag",
            ))),
        },
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![selected_path])));
    assert!(app.busy.is_none());
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Discover { .. })),
        "a tag patch must not rescan: {effects:?}"
    );
}

#[test]
fn an_action_done_removal_clamps_the_selection() {
    use fastf::tui::effect::ActionOutcome;

    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('G'));
    let doomed = app.library.selected().unwrap().path.clone();

    // Unregister: a yes, and the row goes.
    press(&mut app, Key::ch('u'));
    let effects = press(&mut app, Key::ch('y'));
    let id = match effects.as_slice() {
        [Effect::Run(id, _)] => *id,
        other => panic!("{other:?}"),
    };

    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(ActionOutcome::new(
                ListChange::Removed {
                    path: doomed.clone(),
                },
                "Unregistered",
            ))),
        },
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![doomed])));
    assert_eq!(app.library.len(), 2);
    assert_eq!(app.library.selected, Some(1));
}

#[test]
fn delete_asks_for_the_word_and_a_mismatch_deletes_nothing() {
    let mut app = fixture(12, 80, 24);
    let name = app.library.selected().unwrap().name.clone();
    press(&mut app, Key::ch('D'));
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("delete asks in a text prompt");
    };
    assert!(
        prompt.title.contains(&name) && prompt.title.contains("Type delete"),
        "the question names the folder and the word: {}",
        prompt.title
    );
    type_text(&mut app, "delet");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(effects.is_empty(), "nothing runs: {effects:?}");
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("a mismatch keeps the prompt up, text and all");
    };
    assert_eq!(prompt.input.text(), "delet");
    assert!(
        prompt
            .error
            .as_deref()
            .is_some_and(|e| e.contains("type delete to confirm — nothing deleted")),
        "{:?}",
        prompt.error
    );
    type_text(&mut app, "e");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(
            job_started(&effects),
            Some((fastf::core::jobs::JobKind::Delete, _))
        ),
        "the word, any case, deletes: {effects:?}"
    );
    assert!(app.modals.is_empty());
}

#[test]
fn y_and_n_answer_a_confirm_without_enter() {
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();

    press(&mut app, Key::ch('u'));
    assert!(matches!(app.modals.top(), Some(Modal::Confirm(_))));
    // `n` answers without Enter and runs nothing.
    let effects = press(&mut app, Key::ch('n'));
    assert!(effects.is_empty());
    assert!(app.modals.is_empty());

    // `y` runs the unregister action.
    press(&mut app, Key::ch('u'));
    let effects = press(&mut app, Key::ch('y'));
    assert!(matches!(
        action_of(&effects),
        Action::Unregister(project) if **project == selected
    ));
}

/// The job dialog: Ctrl-C asks the job to stop, Esc hides the dialog and
/// the job goes on — and `q` quits the app, leaving the job to finish, since
/// it is not the app's.
#[test]
fn ctrl_c_cancels_a_job_esc_hides_it_and_q_leaves_it_running() {
    use fastf::core::assets::Progress;
    use fastf::core::jobs::JobKind;

    let mut app = fixture(12, 80, 24);
    let id = fastf::tui::testing::follow_job(&mut app, JobKind::Move, Progress::new(&[]));
    assert!(app.job_dialog_up());
    assert_eq!(press(&mut app, Key::ctrl('c')), vec![Effect::CancelJob(id)]);
    assert_eq!(press(&mut app, Key::plain(KeyCode::Esc)), vec![]);
    assert!(!app.job_dialog_up(), "hidden");
    assert!(
        app.background.live().next().is_some(),
        "and the job goes on"
    );
    let effects = press(&mut app, Key::ch('q'));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Quit(_))),
        "q quits: {effects:?}"
    );
}

/// Past the publish a cancel cannot undo a move, and the app does not pretend
/// it can: no cancel is sent, the move carries on, and the log says why.
#[test]
fn a_cancel_after_the_publish_is_answered_not_sent() {
    use fastf::core::assets::{JobPhase, Progress};

    let mut app = fixture(12, 80, 24);
    let mut progress = Progress::new(&[]);
    progress.phase = JobPhase::Removing;
    progress.committed = true;
    fastf::tui::testing::follow_job(&mut app, fastf::core::jobs::JobKind::Move, progress);

    assert_eq!(press(&mut app, Key::ctrl('c')), vec![]);
    assert!(app.job_dialog_up(), "the move is still running");
    let said = app
        .log
        .back()
        .map(|entry| entry.text.clone())
        .unwrap_or_default();
    assert!(said.contains("too late to cancel"), "{said}");
}

/// A copy out of the library and a reconcile are long jobs too: each puts up
/// the progress dialog as it starts, and only then.
#[test]
fn a_copy_and_a_reconcile_are_jobs_with_the_dialog_up() {
    use fastf::core::jobs::JobKind;

    let mut app = fixture(12, 80, 24);
    let effects = press(&mut app, Key::ch('!'));
    assert!(matches!(
        job_started(&effects),
        Some((JobKind::Reconcile, []))
    ));
    assert!(
        app.shown_progress().is_some(),
        "reconcile shows its progress"
    );

    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('C'));
    assert!(
        app.shown_progress().is_none(),
        "not while the folder is being typed"
    );
    type_text(&mut app, "/mnt/backup");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(job_started(&effects), Some((JobKind::Copy, [_]))));
    assert!(
        app.shown_progress().is_some(),
        "the copy shows its progress"
    );
}

/// A job that ends is reported: its summary on the status line, its
/// warning in a dialog, the list reloaded, and the job marked seen.
#[test]
fn a_job_that_ends_is_reported_once_and_marked_seen() {
    use fastf::core::assets::{JobStatus, Progress};
    use fastf::core::jobs::JobKind;

    let mut app = fixture(12, 80, 24);
    let id = fastf::tui::testing::follow_job(&mut app, JobKind::Move, Progress::new(&[]));
    let mut ended = fastf::tui::testing::job_view(
        &id,
        JobKind::Move,
        &[("ID0248", "2026-08-28_Lullaby_Remix_ID0248")],
        Progress::new(&[]),
        false,
        JobStatus::Done,
    );
    let state = ended.state.as_mut().unwrap();
    state.summary = "moved ID0248 to /media/usb/archive".to_string();
    state.items[0].done = true;
    state.items[0].warning = Some(
        "the original at /mnt/projects/x is still there, whole: a program has a file in \
         it open. `fastf reconcile` finishes the move."
            .to_string(),
    );
    let effects = update(&mut app, Msg::Jobs(vec![ended.clone()]));
    assert!(effects.contains(&Effect::MarkSeen(id.clone())));
    assert!(effects.contains(&Effect::LoadSummary), "{effects:?}");
    assert!(
        app.status.text.contains("moved ID0248"),
        "{}",
        app.status.text
    );
    let Some(Modal::Message { title, lines, .. }) = app.modals.top() else {
        panic!("the warning is a dialog: {:?}", app.modals.top());
    };
    assert_eq!(title, "needs a look");
    assert!(
        lines.join(" ").contains("Reconcile (`!`)"),
        "the app's key: {lines:?}"
    );
    assert!(!app.job_dialog_up());

    // Read again: reported already, so nothing new.
    let effects = update(&mut app, Msg::Jobs(vec![ended]));
    assert!(!effects.contains(&Effect::MarkSeen(id)));
}

/// While a job holds the library's lock every change would only wait for it:
/// the verbs say so, and browsing goes on.
#[test]
fn a_job_holding_the_lock_greys_out_the_verbs_that_would_wait() {
    use fastf::core::assets::Progress;
    use fastf::core::jobs::JobKind;
    use fastf::tui::command::{Availability, CommandId, find};

    let mut app = fixture(12, 80, 24);
    let mut progress = Progress::new(&[]);
    progress.holds_lock = true;
    fastf::tui::testing::follow_job(&mut app, JobKind::Move, progress);
    let rename = (find(CommandId::Rename).available)(&app);
    assert!(
        matches!(rename, Availability::Disabled(why) if why.contains("holds the library")),
        "{rename:?}"
    );
    assert!(matches!(
        (find(CommandId::Search).available)(&app),
        Availability::Enabled
    ));
}

/// A move that could not remove its original says why in a paragraph, and a
/// refused move lists every entry it cannot copy; the status line shows one
/// line, so either arrives whole in a dialog instead of as its first words.
#[test]
fn a_paragraph_warning_or_a_many_line_error_opens_a_dialog() {
    use fastf::tui::effect::ActionOutcome;

    let mut app = fixture(12, 80, 24);
    let start = |app: &mut fastf::tui::app::App| {
        press(app, Key::ch('A'));
        run_id(&press(app, Key::plain(KeyCode::Enter)))
    };

    let id = start(&mut app);
    let warning = "the original at /mnt/projects/x is still there, whole, and fastf removed \
                   nothing: a program has a file in it open. When that is resolved, \
                   `fastf reconcile` finishes the move."
        .to_string();
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(
                ActionOutcome::new(ListChange::SummaryOnly, "Moved to /mnt/archive/x")
                    .warning(Some(warning.clone())),
            )),
        },
    );
    let Some(Modal::Message { title, lines, .. }) = app.modals.top() else {
        panic!(
            "a paragraph of warning opens a dialog: {:?}",
            app.modals.top()
        );
    };
    assert_eq!(title, "needs a look");
    assert_eq!(lines.join("\n"), warning);
    assert!(
        app.status.text.contains("see the report"),
        "{}",
        app.status.text
    );
    app.modals.pop();

    let id = start(&mut app);
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err(
                "x holds 2 entries that cannot be copied to another drive:\n  \
                          a.sock: a socket\n  b.pipe: a pipe"
                    .to_string(),
            ),
        },
    );
    let Some(Modal::Message { title, lines, .. }) = app.modals.top() else {
        panic!("a many-line error opens a dialog: {:?}", app.modals.top());
    };
    assert_eq!(title, "error");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        !app.status.text.contains('\n'),
        "the status line keeps one line: {}",
        app.status.text
    );
}

/// What ended while no app was open is said once, on the first read of the
/// jobs: a finished job's summary, and a killed one as needing Reconcile. A
/// job someone has already seen is not said again.
#[test]
fn jobs_that_ended_while_the_app_was_closed_are_said_on_the_first_look() {
    use fastf::core::assets::{JobStatus, Progress};
    use fastf::core::jobs::JobKind;
    use fastf::tui::testing::job_view;

    let mut app = fixture(12, 80, 24);
    let item = [("ID0248", "2026-08-28_Lullaby_Remix_ID0248")];
    let mut done = job_view(
        "18d8-2-0",
        JobKind::Copy,
        &item,
        Progress::new(&[]),
        false,
        JobStatus::Done,
    );
    done.state.as_mut().unwrap().summary = "copied ID0248 to /mnt/backup".to_string();
    let mut seen = done.clone();
    seen.id = "18d8-1-0".to_string();
    seen.seen = true;
    let killed = job_view(
        "18d8-3-0",
        JobKind::Move,
        &item,
        Progress::new(&[]),
        false,
        JobStatus::Running,
    );

    let effects = update(&mut app, Msg::Jobs(vec![killed, done, seen]));
    assert!(effects.contains(&Effect::MarkSeen("18d8-2-0".to_string())));
    assert!(effects.contains(&Effect::MarkSeen("18d8-3-0".to_string())));
    assert!(!effects.contains(&Effect::MarkSeen("18d8-1-0".to_string())));
    let said: Vec<String> = app.log.iter().map(|entry| entry.text.clone()).collect();
    assert!(
        said.iter()
            .any(|line| line.contains("while fastf was closed") && line.contains("copied ID0248")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|line| line.contains("stopped when its process ended")
                && line.contains("Reconcile")),
        "{said:?}"
    );
}

/// While a job is still starting its dialog is already up: Esc hides it
/// rather than quitting, and Ctrl-C is kept for the moment the worker
/// answers, when the job is asked to stop.
#[test]
fn esc_and_ctrl_c_answer_while_a_job_is_starting() {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    press(&mut app, Key::ch('m'));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(job_started(&effects).is_some());
    assert!(app.job_dialog_up(), "the dialog is up at once");

    assert_eq!(press(&mut app, Key::ctrl('c')), vec![]);
    assert!(
        !app.log.iter().any(|entry| entry.text.contains("Goodbye")),
        "Ctrl-C did not quit"
    );
    let effects = update(&mut app, Msg::JobStarted(Ok("18d8-1-0".to_string())));
    assert!(effects.contains(&Effect::CancelJob("18d8-1-0".to_string())));
}

/// A read of the jobs can land before the app hears its own job started:
/// the job is there, with no worker and no state yet. It is not ended, so
/// its end is still reported when it comes — and the dialog closes then.
#[test]
fn a_job_seen_while_starting_is_still_reported_when_it_ends() {
    use fastf::core::assets::{JobStatus, Progress};
    use fastf::core::jobs::JobKind;
    use fastf::tui::testing::job_view;

    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Jobs(Vec::new()));
    app.summary = Some(sample_summary_moveable(12));
    press(&mut app, Key::ch('m'));
    let _ = press(&mut app, Key::plain(KeyCode::Enter));

    let item = [("ID0248", "2026-08-28_Lullaby_Remix_ID0248")];
    let mut starting = job_view(
        "18d8-1-0",
        JobKind::Move,
        &item,
        Progress::new(&[]),
        false,
        JobStatus::Running,
    );
    starting.state = None;
    starting.young = true;
    let effects = update(&mut app, Msg::Jobs(vec![starting]));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::MarkSeen(_)))
    );

    let _ = update(&mut app, Msg::JobStarted(Ok("18d8-1-0".to_string())));
    let mut ended = job_view(
        "18d8-1-0",
        JobKind::Move,
        &item,
        Progress::new(&[]),
        false,
        JobStatus::Done,
    );
    ended.state.as_mut().unwrap().summary = "moved ID0248".to_string();
    let effects = update(&mut app, Msg::Jobs(vec![ended]));
    assert!(effects.contains(&Effect::MarkSeen("18d8-1-0".to_string())));
    assert!(!app.job_dialog_up(), "the dialog closed with the job");
    assert!(
        app.status.text.contains("moved ID0248"),
        "{}",
        app.status.text
    );
}
