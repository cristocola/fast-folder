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
