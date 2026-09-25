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

#[test]
fn move_picks_a_target_and_runs_a_move_action() {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('m'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::Move { project, target } if **project == selected
            && target == Path::new("/media/usb/archive")
    ));
    assert!(app.move_progress.is_some(), "the progress modal is up");
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
    assert!(app.move_progress.is_none());
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
    let name = app.library.selected().unwrap().name.clone();

    // Delete: the word confirms, and the prompt names the folder.
    press(&mut app, Key::ch('D'));
    assert!(
        matches!(app.modals.top(), Some(Modal::TextPrompt(prompt)) if prompt.title.contains(&name))
    );
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
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
                "Deleted",
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
        matches!(action_of(&effects), Action::Delete(_)),
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

#[test]
fn quit_keys_cancel_a_running_move() {
    use fastf::core::assets::Progress;

    let mut app = fixture(12, 80, 24);
    app.move_progress = Some(Progress::new(&[]));
    // Every quit gesture cancels the job instead of abandoning it mid-write.
    assert_eq!(press(&mut app, Key::ctrl('c')), vec![Effect::CancelMove]);
    assert_eq!(
        press(&mut app, Key::plain(KeyCode::Esc)),
        vec![Effect::CancelMove]
    );
    assert_eq!(press(&mut app, Key::ch('q')), vec![Effect::CancelMove]);
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
    app.move_progress = Some(progress);

    assert_eq!(press(&mut app, Key::ctrl('c')), vec![]);
    assert!(app.move_progress.is_some(), "the move is still running");
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
fn a_copy_and_a_reconcile_put_the_progress_dialog_up() {
    let mut app = fixture(12, 80, 24);
    let effects = press(&mut app, Key::ch('!'));
    assert!(matches!(action_of(&effects), Action::Reconcile));
    assert!(app.move_progress.is_some(), "reconcile shows its progress");

    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('C'));
    assert!(
        app.move_progress.is_none(),
        "not while the folder is being typed"
    );
    type_text(&mut app, "/mnt/backup");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(action_of(&effects), Action::CopyTo { .. }));
    assert!(app.move_progress.is_some(), "the copy shows its progress");
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
