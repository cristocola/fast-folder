//! Every verb over the marks.

use crate::harness::*;

#[test]
fn a_tag_over_marks_is_asked_once_and_runs_as_a_job_in_display_order() {
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    press(&mut app, Key::ch(' ')); // mark row 2
    let rows: Vec<PathBuf> = (0..3)
        .map(|row| app.library.row(row).unwrap().path.clone())
        .collect();

    press(&mut app, Key::ch('A'));
    let Some(Modal::Pick(pick)) = app.modals.top() else {
        panic!("A opens the tag picker");
    };
    assert!(pick.title.contains("3 projects"), "{}", pick.title);
    // Type a new tag rather than pick one.
    let new_tag = pick
        .items
        .iter()
        .position(|item| item.value == fastf::tui::app::actions::NEW_TAG)
        .expect("the picker offers a new tag");
    for _ in 0..new_tag {
        press(&mut app, Key::plain(KeyCode::Down));
    }
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "reviewed");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));

    let job = app.job.as_ref().expect("a tag job is running");
    assert!(matches!(&job.kind, fastf::tui::app::jobs::JobKind::AddTag(tag) if tag == "reviewed"));
    assert_eq!(job.pending.len(), 2, "one in flight, two to go");
    let id1 = run_id(&effects);
    assert!(
        matches!(action_of(&effects), Action::AddTag { project, tag } if project.path == rows[0] && tag == "reviewed")
    );

    // The first item lands patched; the next starts; the mark stays until the
    // row changes say it is done.
    let mut patched = app.library.row(0).unwrap().clone();
    patched.tags.push("reviewed".to_string());
    let effects = update(
        &mut app,
        item_done(
            id1,
            ListChange::Patched {
                project: Box::new(patched),
                was: rows[0].clone(),
                stale: vec![rows[0].clone()],
            },
        ),
    );
    assert!(
        matches!(action_of(&effects), Action::AddTag { project, .. } if project.path == rows[1])
    );
    assert!(
        app.library
            .row(0)
            .unwrap()
            .tags
            .contains(&"reviewed".to_string()),
        "the row shows the tag as soon as its item lands"
    );
}

/// **The batch's effects are the app's too.** They were dropped, so a
/// `Reload` — which `discover` arms by setting `inflight` *before* returning
/// the effect that answers it — left the app waiting on a generation nothing
/// would send, and every later patch only set `dirty`. A batch re-derive of
/// tags rewrote every file, showed nothing, and froze the list for the rest of
/// the session.
#[test]
fn a_batch_item_returns_the_effects_its_change_asked_for() {
    use fastf::tui::command::CommandId;

    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let effects = app.run(CommandId::ReautoTags);
    let id1 = run_id(&effects);

    let effects = update(&mut app, item_done(id1, ListChange::Reload));
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Discover { .. })),
        "a reload must reach the runtime: {effects:?}"
    );
    assert!(
        effects.iter().any(|e| matches!(e, Effect::LoadSummary)),
        "and so must the summary it asked for: {effects:?}"
    );

    // The discovery it armed is the one in flight, so its answer installs.
    let generation = app.library.inflight.expect("a discovery is in flight");
    update(
        &mut app,
        Msg::Discovered {
            generation,
            projects: sample_projects(12),
        },
    );
    assert!(
        app.library.inflight.is_none(),
        "the list is not left waiting on a generation nothing will send"
    );
}

/// Marks are kept by path and survive a filter change; `targets()` intersects
/// them with the rows on screen. When the two disagreed, `batching()` said yes
/// and every batch verb hit an early return with no picker, no dialog and no
/// message — which is what "batch tagging does nothing" was.
#[test]
fn a_verb_aimed_at_marks_a_filter_hides_says_so() {
    use fastf::tui::command::CommandId;

    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    assert_eq!(app.library.marks.len(), 2);

    // A query that keeps nothing the marks are on.
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "zzzznothing");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(app.library.is_empty(), "the filter hides every marked row");

    let effects = app.run(CommandId::AddTag);
    assert!(app.modals.is_empty(), "no picker over nothing");
    assert!(effects.is_empty());
    assert!(
        app.status.text.contains("every marked row is hidden"),
        "the refusal has to be said out loud: {:?}",
        app.status.text
    );
    assert_eq!(app.library.marks.len(), 2, "the marks are not touched");
}

#[test]
fn a_quick_note_takes_several_lines_and_goes_to_every_mark() {
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ctrl('n'));
    assert!(matches!(app.modals.top(), Some(Modal::Note(note)) if note.count == 1));
    type_text(&mut app, "first cut");
    let mut alt_enter = Key::plain(KeyCode::Enter);
    alt_enter.alt = true;
    press(&mut app, alt_enter);
    type_text(&mut app, "due Friday");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::AppendNote { text, .. } if text == "first cut\ndue Friday"),
        "Enter saves, Alt-Enter broke the line: {effects:?}"
    );
    assert!(app.modals.is_empty());

    // Over marks the note is asked once and becomes a job.
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ctrl('n'));
    assert!(matches!(app.modals.top(), Some(Modal::Note(note)) if note.count == 2));
    let _ = update(&mut app, Msg::Paste("line one\nline two\n".to_string()));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(&app.job.as_ref().unwrap().kind, fastf::tui::app::jobs::JobKind::Note(text) if text == "line one\nline two"),
        "a pasted paragraph is one note"
    );
    assert!(matches!(action_of(&effects), Action::AppendNote { .. }));
}

#[test]
fn a_paste_never_becomes_keystrokes() {
    let mut app = fixture(12, 120, 40);
    let before = app.library.selected().unwrap().path.clone();
    // Nothing takes typing: the paste is ignored and said so.
    let effects = update(&mut app, Msg::Paste("D\ndelete\n".to_string()));
    assert!(effects.is_empty() && app.modals.is_empty());
    assert_eq!(app.library.selected().unwrap().path, before);
    assert!(
        app.status.text.contains("pasted text ignored"),
        "{}",
        app.status.text
    );

    // A single-line field keeps the first line and says so.
    press(&mut app, Key::ch('r'));
    press(&mut app, Key::ctrl('u')); // the prompt opens on the old name
    let _ = update(&mut app, Msg::Paste("New_Name\nq\nD".to_string()));
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("the rename prompt is still up");
    };
    assert_eq!(prompt.input.text(), "New_Name");
    assert!(
        app.status.text.contains("kept the first"),
        "{}",
        app.status.text
    );
}

#[test]
fn rename_does_not_batch_and_says_so() {
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    let effects = press(&mut app, Key::ch('r'));
    assert!(effects.is_empty() && app.modals.is_empty());
    assert!(
        app.status.text.contains("one folder at a time"),
        "{}",
        app.status.text
    );
    press(&mut app, Key::ch('-'));
    press(&mut app, Key::ch('r'));
    assert!(matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
}
