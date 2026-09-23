//! The detail pane as an editor you enter on purpose: nothing changes until
//! Enter on a row, Esc leaves the row as it was, and what can be typed is
//! what the file can hold.

use crate::harness::*;
use fastf::core::project_info::Metadata;
use fastf::core::template::{Transform, VarType, Variable};
use fastf::tui::app::data::ProjectDetail;
use fastf::tui::app::pane::{PaneEdit, PaneRow};
use fastf::tui::command::Context;
use fastf::tui::effect::Action;
use std::collections::BTreeMap;

fn variable(slug: &str, var_type: VarType, options: &[&str]) -> Variable {
    Variable {
        slug: slug.to_string(),
        label: slug.to_uppercase(),
        var_type,
        required: false,
        options: options.iter().map(|o| o.to_string()).collect(),
        default: String::new(),
        transform: Transform::None,
    }
}

/// A pane with a text variable, a select, notes, and the fixture's tags,
/// focused and ready.
fn editing_fixture() -> App {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('j'));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(project.tags, vec!["client/Acme", "draft"]);
    let meta = Metadata {
        id: project.id.clone(),
        id_number: project.id_number,
        template: project.template.clone(),
        template_name: project.template_name.clone(),
        created: project.created.clone(),
        folder: project.name.clone(),
        path: String::new(),
        variables: BTreeMap::from([
            ("artist".to_string(), "Ariana".to_string()),
            ("tier".to_string(), "Indie".to_string()),
        ]),
        tags: project.tags.clone(),
        auto_tags: Vec::new(),
        provisioning: false,
    };
    let detail = ProjectDetail {
        meta: Some(meta),
        variables: vec![
            variable("artist", VarType::Text, &[]),
            variable("tier", VarType::Select, &["Indie", "Major"]),
        ],
        notes: vec![
            fastf::core::body::Note {
                timestamp: None,
                text: "first cut Friday".to_string(),
            },
            fastf::core::body::Note {
                timestamp: Some("2026-08-28T10:00:00Z".to_string()),
                text: "began the edit\nrough cut by Friday".to_string(),
            },
        ],
        todos: vec![
            fastf::core::body::Todo {
                done: true,
                text: "ingested the videos".to_string(),
                phase: None,
            },
            fastf::core::body::Todo {
                done: false,
                text: "delivered the video".to_string(),
                phase: None,
            },
        ],
        ..Default::default()
    };
    update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::new(detail),
        },
    );
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail);
    app
}

fn go_to(app: &mut App, wanted: impl Fn(&PaneRow) -> bool) {
    let rows = app.pane_rows();
    let at = rows
        .iter()
        .position(wanted)
        .expect("the row is in the pane");
    press(app, Key::ch('g'));
    while app.pane_cursor < at {
        let before = app.pane_cursor;
        press(app, Key::ch('j'));
        assert!(app.pane_cursor > before, "the cursor cannot reach row {at}");
    }
    assert_eq!(app.pane_cursor, at);
}

fn sent(effects: &[Effect]) -> Option<&Action> {
    effects.iter().find_map(|e| match e {
        Effect::Run(_, action) => Some(action.as_ref()),
        _ => None,
    })
}

#[test]
fn nothing_edits_before_enter_and_esc_leaves_the_row_as_it_was() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
    );
    for key in [Key::ch('x'), Key::ch('a'), Key::plain(KeyCode::Backspace)] {
        let effects = press(&mut app, key);
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Run(..))),
            "{} ran something without Enter: {effects:?}",
            key.label()
        );
    }
    assert!(
        app.pane_edit.is_none(),
        "typing on a row does not open an edit"
    );
    // `a` opened the action menu — that key still works in the pane.
    assert!(matches!(app.modals.top(), Some(Modal::Actions(_))));
    press(&mut app, Key::plain(KeyCode::Esc));

    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(&app.pane_edit, Some(PaneEdit::Line { .. })),
        "Enter opens the line editor: {:?}",
        app.pane_edit
    );
    assert_eq!(app.context(), Context::PaneEdit);
    type_text(&mut app, " Grande");
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.pane_edit.is_none(), "Esc closes the edit");
    let rows = app.pane_rows();
    assert!(
        matches!(&rows[app.pane_cursor], PaneRow::Variable { value, .. } if value == "Ariana"),
        "and the row is as it was"
    );
}

#[test]
fn enter_on_a_text_variable_edits_in_place_and_enter_again_sends_it() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
    );
    let row = app.pane_cursor;
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "_Grande");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(
        sent(&effects),
        Some(&Action::SetVariable {
            project: Box::new(project.clone()),
            slug: "artist".to_string(),
            value: "Ariana_Grande".to_string(),
        })
    );
    assert!(
        app.pane_edit.as_ref().is_some_and(|edit| edit.pending()),
        "the edit stays open, pending, until the worker answers"
    );
    // Typing while pending changes nothing.
    type_text(&mut app, "zzz");
    assert!(
        matches!(&app.pane_edit, Some(PaneEdit::Line { input, .. }) if input.text() == "Ariana_Grande")
    );

    // The worker answers: the row is patched, the edit closes, the cursor
    // stays on the row, and the row pulses.
    app.theme = fastf::tui::theme::Theme::rich();
    app.motion = fastf::tui::motion::Motion::On;
    app.elapsed_ms = 2_000;
    let mut patched = project.clone();
    patched.tags.push("tier/Indie".to_string());
    let id = app.busy_id.expect("an action in flight");
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                fastf::tui::effect::ListChange::Patched {
                    project: Box::new(patched),
                    was: project.path.clone(),
                    stale: vec![project.path.clone()],
                },
                "Set artist",
            ))),
        },
    );
    assert!(app.pane_edit.is_none(), "an Ok closes the edit");
    // The patch dropped the cached detail, so the variables are gone
    // until the re-read lands — and a tag row arrived above them. The
    // cursor follows the variable, not its old index.
    let rows = app.pane_rows();
    assert!(
        rows.iter().all(|r| !matches!(r, PaneRow::Variable { .. })),
        "the detail is being re-read"
    );
    let detail = ProjectDetail {
        meta: Some(Metadata {
            id: project.id.clone(),
            id_number: project.id_number,
            template: project.template.clone(),
            template_name: project.template_name.clone(),
            created: project.created.clone(),
            folder: project.name.clone(),
            path: String::new(),
            variables: BTreeMap::from([
                ("artist".to_string(), "Ariana_Grande".to_string()),
                ("tier".to_string(), "Indie".to_string()),
            ]),
            tags: Vec::new(),
            auto_tags: Vec::new(),
            provisioning: false,
        }),
        variables: vec![
            variable("artist", VarType::Text, &[]),
            variable("tier", VarType::Select, &["Indie", "Major"]),
        ],
        ..Default::default()
    };
    update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::new(detail),
        },
    );
    let rows = app.pane_rows();
    assert!(
        matches!(&rows[app.pane_cursor], PaneRow::Variable { slug, value, .. } if slug == "artist" && value == "Ariana_Grande"),
        "the cursor is on the variable that changed, wherever it is now: {:?}",
        rows[app.pane_cursor]
    );
    assert_ne!(
        app.pane_cursor, row,
        "which is one row down, under the new tag"
    );
    assert!(
        app.pane_pulses
            .style_for(&app.pane_cursor, 2_000, &app.theme, app.motion)
            .is_some(),
        "and that row pulses"
    );
    assert_eq!(
        app.tick_interval(),
        Some(std::time::Duration::from_millis(
            fastf::tui::motion::FRAME_MS
        ))
    );
}

#[test]
fn a_refusal_lands_on_the_open_edit_with_the_text_still_there() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
    );
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "!");
    press(&mut app, Key::plain(KeyCode::Enter));
    let id = app.busy_id.expect("an action in flight");
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("artist is required".to_string()),
        },
    );
    match &app.pane_edit {
        Some(PaneEdit::Line {
            input,
            error,
            pending,
            ..
        }) => {
            assert_eq!(
                input.text(),
                "Ariana!",
                "the text is still there to correct"
            );
            assert_eq!(error.as_deref(), Some("artist is required"));
            assert!(!pending, "and it can be sent again");
        }
        other => panic!("the edit closed on a refusal: {other:?}"),
    }
    assert!(
        app.modals.is_empty(),
        "no dialog for a refusal under the field"
    );
    // Typing clears the message.
    press(&mut app, Key::plain(KeyCode::Backspace));
    assert!(app.pane_edit.as_ref().unwrap().error().is_none());
}

#[test]
fn a_tag_is_edited_in_place_and_emptied_it_is_removed() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Tag(tag) if tag == "draft"),
    );
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(&app.pane_edit, Some(PaneEdit::Line { input, .. }) if input.text() == "draft")
    );
    // Unchanged is a cancel, not a write.
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&effects).is_none());
    assert!(app.pane_edit.is_none());

    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "-v2");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(
        sent(&effects),
        Some(&Action::ReplaceTag {
            project: Box::new(project.clone()),
            from: "draft".to_string(),
            to: Some("draft-v2".to_string()),
        })
    );
    let id = app.busy_id.unwrap();
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("no".to_string()),
        },
    );
    press(&mut app, Key::plain(KeyCode::Esc));

    press(&mut app, Key::plain(KeyCode::Enter));
    for _ in 0..5 {
        press(&mut app, Key::plain(KeyCode::Backspace));
    }
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(
        sent(&effects),
        Some(&Action::ReplaceTag {
            project: Box::new(project),
            from: "draft".to_string(),
            to: None,
        }),
        "an emptied tag is removed"
    );
}

#[test]
fn a_tag_that_is_not_a_tag_is_refused_under_the_line() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Tag(tag) if tag == "draft"),
    );
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, " a poem");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&effects).is_none(), "nothing was sent");
    match &app.pane_edit {
        Some(PaneEdit::Line { error, .. }) => {
            assert!(
                error.as_deref().is_some_and(|e| e.contains("one word")),
                "{error:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn enter_on_add_tag_the_name_and_the_journal_open_the_flows_that_exist() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTag));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(
            app.modals.top(),
            Some(Modal::Pick(_)) | Some(Modal::TextPrompt(_))
        ),
        "add a tag is the tag flow: {:?}",
        app.modals.top().map(|m| m.context())
    );
    press(&mut app, Key::plain(KeyCode::Esc));

    go_to(&mut app, |row| matches!(row, PaneRow::Name(_)));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
        "the name is the rename prompt"
    );
    press(&mut app, Key::plain(KeyCode::Esc));

    go_to(&mut app, |row| matches!(row, PaneRow::AddNote));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(app.modals.top(), Some(Modal::Note(_))),
        "add a note is the quick note"
    );
    press(&mut app, Key::plain(KeyCode::Esc));

    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        app.pane_edit.as_ref().is_some_and(PaneEdit::is_adding),
        "add a todo opens a line in the list, where the todo will land"
    );
    assert!(app.modals.is_empty(), "not a dialog");
    type_text(&mut app, "invoice");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(
        sent(&effects),
        Some(&Action::AddTodos {
            project: Box::new(project.clone()),
            texts: vec!["invoice".to_string()],
            place: fastf::core::body::TodoPlace::End,
        })
    );
    let id = app.busy_id.unwrap();
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                fastf::tui::effect::ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                "Todo added.",
            ))),
        },
    );
}

/// A todo too wide for the pane wraps onto the rows under it, and Enter on it
/// still names the whole todo: the row holds only what fits, and a toggle that
/// named the row's text would be refused as changed meanwhile.
#[test]
fn a_todo_too_wide_for_the_pane_wraps_and_toggles_by_its_whole_text() {
    let mut app = editing_fixture();
    let project = app.library.selected().unwrap().clone();
    let long = "send the rough cut to the label and ask whether the lyric video is still in scope";
    update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::new(ProjectDetail {
                todos: vec![fastf::core::body::Todo {
                    done: false,
                    text: long.to_string(),
                    phase: None,
                }],
                ..Default::default()
            }),
        },
    );
    let rows = app.pane_rows();
    assert!(
        rows.iter()
            .any(|row| matches!(row, PaneRow::TodoLine { .. })),
        "the rest of the todo is on the rows under it: {rows:?}"
    );
    go_to(&mut app, |row| matches!(row, PaneRow::Todo { .. }));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(
        sent(&effects),
        Some(&Action::ToggleTodo {
            project: Box::new(project),
            ordinal: 0,
            was: long.to_string(),
        })
    );
}

/// Enter on a todo writes the toggle at once — there is nothing to type
/// — with no edit open, and the answer lands on that row: the cursor
/// stays there and the row pulses, as an edit's does. Nothing on the
/// list lights up, because nothing on the list changed.
#[test]
fn enter_on_a_todo_toggles_it_and_the_answer_lands_on_its_row() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Todo { text, .. } if text == "delivered the video"),
    );
    let at = app.pane_cursor;
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(
        sent(&effects),
        Some(&Action::ToggleTodo {
            project: Box::new(project.clone()),
            ordinal: 1,
            was: "delivered the video".to_string(),
        })
    );
    assert!(app.pane_edit.is_none(), "a toggle opens nothing");
    // Nothing else starts while the write is on its way.
    let again = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&again).is_none(), "{again:?}");

    let id = app.busy_id.unwrap();
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                fastf::tui::effect::ListChange::DetailOnly {
                    path: project.path.clone(),
                },
                "Done.",
            ))),
        },
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDetail(p) if *p == project.path)),
        "the detail is read again: {effects:?}"
    );
    assert!(
        app.pulses.is_empty(),
        "no row of the list changed, so none lights up"
    );
    assert_eq!(app.pane_cursor, at, "the cursor stays on the todo");
    assert!(!app.pane_pulses.is_empty(), "and the todo's row pulses");
    // The pane keeps what it shows until the re-read lands — no
    // `reading…` frame between the keypress and the answer — and the
    // re-read lands with the todo flipped, the cursor still on it.
    assert!(
        app.details.contains_key(&project.path),
        "the detail on screen stays until the fresh one arrives"
    );
    let fresh = fastf::tui::app::data::ProjectDetail {
        todos: vec![
            fastf::core::body::Todo {
                done: true,
                text: "ingested the videos".to_string(),
                phase: None,
            },
            fastf::core::body::Todo {
                done: true,
                text: "delivered the video".to_string(),
                phase: None,
            },
        ],
        ..Default::default()
    };
    update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::new(fresh),
        },
    );
    assert!(matches!(
        app.pane_rows()[app.pane_cursor],
        PaneRow::Todo {
            done: true,
            ordinal: 1,
            ..
        }
    ));
}

/// `… n earlier` shows every note; Enter on it is `J`.
#[test]
fn enter_on_earlier_notes_shows_them_all() {
    let mut app = editing_fixture();
    let project = app.library.selected().unwrap().clone();
    let mut detail = app.details.get(&project.path).cloned().unwrap();
    detail.notes = (0..7)
        .map(|n| fastf::core::body::Note {
            timestamp: Some(format!("2026-01-0{}T00:00:00Z", n + 1)),
            text: format!("note {n}"),
        })
        .collect();
    update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::new(detail),
        },
    );
    go_to(&mut app, |row| matches!(row, PaneRow::EarlierNotes(2)));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadView {
                kind: fastf::tui::effect::ViewKind::Journal,
                ..
            }
        )),
        "{effects:?}"
    );
}

#[test]
fn a_select_variable_offers_only_its_options_and_the_pick_is_the_edit() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "tier"),
    );
    press(&mut app, Key::plain(KeyCode::Enter));
    let labels: Vec<String> = match app.modals.top() {
        Some(Modal::Pick(pick)) => pick.items.iter().map(|i| i.label.clone()).collect(),
        other => panic!("a select opens a picker: {other:?}"),
    };
    assert_eq!(labels, vec!["Indie", "Major"]);
    press(&mut app, Key::plain(KeyCode::Down));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(
        sent(&effects),
        Some(&Action::SetVariable {
            project: Box::new(project),
            slug: "tier".to_string(),
            value: "Major".to_string(),
        })
    );
    assert!(app.modals.is_empty());
}

#[test]
fn a_note_is_a_text_area_saved_with_ctrl_s_and_enter_is_a_new_line() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Note { ordinal: 0, .. })
    });
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        &app.pane_edit,
        Some(PaneEdit::Note { ordinal: 0, .. })
    ));
    press(&mut app, Key::plain(KeyCode::End));
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "then colour");
    assert!(
        app.pane_edit.is_some(),
        "Enter in a note is a new line, not a send"
    );
    let effects = press(&mut app, Key::ctrl('s'));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(
        sent(&effects),
        Some(&Action::ReplaceNote {
            project: Box::new(project.clone()),
            ordinal: 0,
            was: "first cut Friday".to_string(),
            text: "first cut Friday\nthen colour".to_string(),
        }),
        "Ctrl-S saves the note, naming the text it read"
    );
    let id = app.busy_id.unwrap();
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("the note changed meanwhile — reload and edit it again".to_string()),
        },
    );
    assert!(
        matches!(&app.pane_edit, Some(PaneEdit::Note { error: Some(e), .. }) if e.contains("changed meanwhile")),
        "a refusal lands on the note editor"
    );

    // A dated note edits the same way, and its other lines are there.
    press(&mut app, Key::plain(KeyCode::Esc));
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Note { ordinal: 1, .. })
    });
    press(&mut app, Key::plain(KeyCode::Enter));
    match &app.pane_edit {
        Some(PaneEdit::Note { area, was, .. }) => {
            assert_eq!(area.text(), "began the edit\nrough cut by Friday");
            assert_eq!(was, "began the edit\nrough cut by Friday");
        }
        other => panic!("{other:?}"),
    }
    // Unchanged, Ctrl-S is a cancel.
    let effects = press(&mut app, Key::ctrl('s'));
    assert!(sent(&effects).is_none() && app.pane_edit.is_none());
}

#[test]
fn leaving_the_pane_or_the_row_drops_an_open_edit_untouched() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
    );
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "x");
    // `←` is the registry's while a line is being edited? No — the field
    // has the arrows as its caret, so it is Esc, then ←.
    press(&mut app, Key::plain(KeyCode::Left));
    assert!(app.pane_edit.is_some(), "← moves the caret, not the focus");
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.pane_edit.is_none());
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Projects);
    assert!(
        app.pane_edit.is_none(),
        "leaving the pane leaves the row as it was"
    );
}

/// The bar says what the pane is for once it has the focus, and what an
/// open edit answers to.
#[test]
fn the_hint_bar_reads_the_pane_and_the_edit() {
    let mut app = editing_fixture();
    let hints: Vec<String> = fastf::tui::command::hints(app.context(), &app, 200)
        .into_iter()
        .map(|(key, what)| format!("{key} {what}"))
        .collect();
    assert!(hints.iter().any(|h| h == "Enter edit"), "{hints:?}");
    assert!(hints.iter().any(|h| h == "← list"), "{hints:?}");
    assert!(
        hints.iter().any(|h| h == "a actions"),
        "`a` still works in the pane: {hints:?}"
    );
    // Enter says what it will do to the row under the cursor.
    for (wanted, verb) in [
        (
            Box::new(|row: &PaneRow| matches!(row, PaneRow::Todo { .. }))
                as Box<dyn Fn(&PaneRow) -> bool>,
            "Enter toggle",
        ),
        (
            Box::new(|row: &PaneRow| matches!(row, PaneRow::AddNote)),
            "Enter add",
        ),
        (
            Box::new(|row: &PaneRow| matches!(row, PaneRow::AddTodo)),
            "Enter add",
        ),
        (
            Box::new(|row: &PaneRow| matches!(row, PaneRow::Note { .. })),
            "Enter edit",
        ),
    ] {
        go_to(&mut app, wanted);
        let hints: Vec<String> = fastf::tui::command::hints(app.context(), &app, 200)
            .into_iter()
            .map(|(key, what)| format!("{key} {what}"))
            .collect();
        assert!(hints.iter().any(|h| h == verb), "{verb}: {hints:?}");
    }
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Note { ordinal: 0, .. })
    });
    press(&mut app, Key::plain(KeyCode::Enter));
    let hints: Vec<String> = fastf::tui::command::hints(app.context(), &app, 200)
        .into_iter()
        .map(|(key, what)| format!("{key} {what}"))
        .collect();
    assert!(hints.iter().any(|h| h == "Ctrl-s save"), "{hints:?}");
    assert!(hints.iter().any(|h| h == "Esc cancel"), "{hints:?}");
    assert!(
        !hints.iter().any(|h| h.starts_with("Enter")),
        "Enter is a new line here: {hints:?}"
    );
}

/// **The caret sits in the field being typed into.** The pane's editor drew
/// its field but the terminal's cursor was only ever placed for the search
/// bar, so an edit in the pane had no caret at all.
#[test]
fn the_caret_sits_in_the_field_being_edited() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
    );
    let (_, caret) = fastf::tui::testing::render_with_caret(&app, 120, 40);
    assert_eq!(caret, None, "no field open, no caret");

    press(&mut app, Key::plain(KeyCode::Enter));
    let pane = app.regions().detail.expect("a pane at 120 columns");
    let row = (app.pane_edit.as_ref().unwrap().row() - app.detail_scroll) as u16;
    let (_, caret) = fastf::tui::testing::render_with_caret(&app, 120, 40);
    let caret = caret.expect("an open edit shows the caret");
    assert_eq!(caret.y, pane.y + 1 + row, "on the edit's own row");
    assert!(
        caret.x > pane.x && caret.x < pane.x + pane.width - 1,
        "inside the pane: {caret:?} in {pane:?}"
    );

    press(&mut app, Key::plain(KeyCode::Esc));
    let (_, caret) = fastf::tui::testing::render_with_caret(&app, 120, 40);
    assert_eq!(caret, None, "closed again, the caret goes with it");
}

/// A note on the pane's last visible row opens its editor all the same,
/// slid up over the rows above it. It used to draw nothing: the editor wanted
/// two rows under its own and returned without a frame, while the edit stayed
/// open and took the keys.
#[test]
fn a_note_opened_on_the_last_visible_row_slides_up() {
    let mut app = fixture(6, 120, 20);
    press(&mut app, Key::ch('j'));
    let path = app.library.selected().unwrap().path.clone();
    let detail = ProjectDetail {
        notes: (0..6)
            .map(|n| fastf::core::body::Note {
                timestamp: Some(format!("2026-01-0{}T00:00:00Z", n + 1)),
                text: format!("note {n}"),
            })
            .collect(),
        ..Default::default()
    };
    update(
        &mut app,
        Msg::Detail {
            path,
            detail: Box::new(detail),
        },
    );
    press(&mut app, Key::plain(KeyCode::Right));
    let last_note = app
        .pane_rows()
        .iter()
        .rposition(|row| matches!(row, PaneRow::Note { .. }))
        .unwrap();
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Note { ordinal: 5, .. })
    });
    assert_eq!(app.pane_cursor, last_note);
    let pane = app.regions().detail.expect("a pane at 120 columns");
    let visible = (pane.height - 2) as usize;
    assert_eq!(
        app.pane_cursor - app.detail_scroll,
        visible - 1,
        "the fixture puts the note on the last visible row"
    );

    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(app.pane_edit, Some(PaneEdit::Note { .. })));
    let (buffer, caret) = fastf::tui::testing::render_with_caret(&app, 120, 20);
    let caret = caret.expect("the note editor is drawn, and takes the caret");
    assert!(
        caret.y > pane.y && caret.y < pane.y + pane.height - 1,
        "inside the pane: {caret:?} in {pane:?}"
    );
    let text: String = (pane.y..pane.y + pane.height)
        .flat_map(|y| (pane.x..pane.x + pane.width).map(move |x| (x, y)))
        .map(|(x, y)| buffer[(x, y)].symbol().to_string())
        .collect();
    assert!(text.contains("note 5"), "the note's text is in the editor");
    assert!(text.contains("save"), "and its key line under it");
}

/// A paste lands in the field the pane has open — the first line in a line
/// field, as every one-line field takes it. It used to fall through to
/// "pasted text ignored".
#[test]
fn a_paste_lands_in_the_pane_field() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
    );
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::ctrl('u'));
    update(
        &mut app,
        Msg::Paste("Beyoncé\nand a second line".to_string()),
    );
    match &app.pane_edit {
        Some(PaneEdit::Line { input, .. }) => assert_eq!(input.text(), "Beyoncé"),
        other => panic!("the line edit is still open: {other:?}"),
    }
    assert!(
        app.status.text.contains("kept the first"),
        "and says what it dropped: {:?}",
        app.status
    );
}

fn row_of(app: &App, wanted: impl Fn(&PaneRow) -> bool) -> usize {
    app.pane_rows()
        .iter()
        .position(wanted)
        .expect("the row is in the pane")
}

/// **A resize loses nothing.** A note being written stays open with its text,
/// on its own row, and the cursor stays on it — through a resize to the same
/// size (one arrives after every `$EDITOR` note and every `fg`) and through a
/// real one. Both used to close the editor and throw the text away.
#[test]
fn a_resize_keeps_the_panes_cursor_and_an_open_note_with_its_text() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Note { ordinal: 1, .. })
    });
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::ctrl('e'));
    type_text(&mut app, " and the grade");

    for (width, height) in [(120, 40), (110, 36)] {
        update(&mut app, Msg::Resize(width, height));
        match &app.pane_edit {
            Some(PaneEdit::Note { area, ordinal, .. }) => {
                assert_eq!(*ordinal, 1);
                assert!(
                    area.text().contains("rough cut by Friday and the grade"),
                    "the typed text survives a resize to {width}×{height}: {:?}",
                    area.text()
                );
            }
            other => panic!("the note editor closed on a {width}×{height} resize: {other:?}"),
        }
        let note = row_of(&app, |row| matches!(row, PaneRow::Note { ordinal: 1, .. }));
        assert_eq!(app.pane_edit.as_ref().unwrap().row(), note);
        assert_eq!(app.pane_cursor, note, "and the cursor is still on it");
        assert_eq!(app.focus, Focus::Detail);
    }
}

/// A resize re-wraps every todo at the new width, so the row a todo starts on
/// moves; the cursor stays on the todo, not on the index it had.
#[test]
fn a_resize_rewraps_and_the_cursor_stays_on_its_todo() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('j'));
    let path = app.library.selected().unwrap().path.clone();
    let long = "check the colour of every shot against the reference stills, then the sound";
    let detail = ProjectDetail {
        todos: vec![
            fastf::core::body::Todo {
                done: false,
                text: long.to_string(),
                phase: None,
            },
            fastf::core::body::Todo {
                done: false,
                text: "export".to_string(),
                phase: None,
            },
        ],
        ..Default::default()
    };
    update(
        &mut app,
        Msg::Detail {
            path,
            detail: Box::new(detail),
        },
    );
    press(&mut app, Key::plain(KeyCode::Right));
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 1, .. })
    });
    let before = app.pane_cursor;

    update(&mut app, Msg::Resize(104, 40));
    let after = row_of(&app, |row| matches!(row, PaneRow::Todo { ordinal: 1, .. }));
    assert_ne!(before, after, "the fixture re-wraps the first todo");
    assert_eq!(app.pane_cursor, after, "the cursor followed its todo");
}

/// A re-read landing while a tag is being edited leaves the editor on that
/// tag. It used to look the tag up by the text being typed — which is no row
/// until the write lands — and move the editor to "add a tag".
#[test]
fn a_detail_refresh_keeps_an_open_tag_edit_on_its_tag() {
    let mut app = editing_fixture();
    go_to(
        &mut app,
        |row| matches!(row, PaneRow::Tag(tag) if tag == "draft"),
    );
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "final");

    let path = app.library.selected().unwrap().path.clone();
    let detail = app.details.get(&path).cloned().unwrap();
    update(
        &mut app,
        Msg::Detail {
            path,
            detail: Box::new(detail),
        },
    );
    let draft = row_of(
        &app,
        |row| matches!(row, PaneRow::Tag(tag) if tag == "draft"),
    );
    let edit = app.pane_edit.as_ref().expect("the edit is still open");
    assert_eq!(edit.row(), draft, "on the tag it was opened on");
    match edit {
        PaneEdit::Line { input, .. } => assert_eq!(input.text(), "final"),
        other => panic!("a line edit: {other:?}"),
    }
}

/// A list change that leaves the same project selected — a metadata read
/// landing, a discovery — keeps the pane's cursor where it was.
#[test]
fn a_reload_landing_keeps_the_pane_cursor() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    let at = app.pane_cursor;
    update(&mut app, Msg::MetaLoaded(Vec::new()));
    assert_eq!(app.pane_cursor, at, "the cursor did not go back to the top");
    assert_eq!(app.focus, Focus::Detail);
}

fn done_ok(app: &mut App, message: &str) {
    let id = app.busy_id.expect("an action is running");
    let path = app.library.selected().unwrap().path.clone();
    update(
        app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                fastf::tui::effect::ListChange::DetailOnly { path },
                message,
            ))),
        },
    );
}

/// An add's answer: written, with the last todo at `ordinal`, as the
/// runtime reports it.
fn added(app: &mut App, ordinal: usize) {
    let id = app.busy_id.expect("an add is on its way");
    let path = app.library.selected().unwrap().path.clone();
    update(
        app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(
                fastf::tui::effect::ActionOutcome::new(
                    fastf::tui::effect::ListChange::DetailOnly { path },
                    "Todo added.",
                )
                .todo(ordinal),
            )),
        },
    );
}

fn with_todos(app: &mut App, todos: &[(bool, &str, Option<&str>)]) {
    let path = app.library.selected().unwrap().path.clone();
    let mut detail = app.details.get(&path).cloned().unwrap_or_default();
    detail.todos = todos
        .iter()
        .map(|(done, text, phase)| fastf::core::body::Todo {
            done: *done,
            text: text.to_string(),
            phase: phase.map(str::to_string),
        })
        .collect();
    update(
        app,
        Msg::Detail {
            path,
            detail: Box::new(detail),
        },
    );
}

/// **F2 rewords a todo where it sits; Enter still ticks it.** The text opens
/// on the todo's own line behind its box, whole; Enter sends the new words
/// with the old ones as the check against a file changed meanwhile; unchanged
/// is a cancel; emptied, the todo is removed — the rule tags and notes keep.
#[test]
fn f2_rewords_a_todo_on_its_line_and_emptied_removes_it() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 1, .. })
    });
    let project = app.library.selected().unwrap().clone();

    // Enter is still the tick.
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        sent(&effects),
        Some(Action::ToggleTodo { ordinal: 1, .. })
    ));
    done_ok(&mut app, "Done.");

    press(&mut app, Key::plain(KeyCode::F(2)));
    match &app.pane_edit {
        Some(PaneEdit::Line { input, .. }) => assert_eq!(input.text(), "delivered the video"),
        other => panic!("F2 opens the todo's text in place: {other:?}"),
    }
    // Unchanged: nothing is written.
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&effects).is_none() && app.pane_edit.is_none());

    press(&mut app, Key::plain(KeyCode::F(2)));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "deliver the masters");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(
        sent(&effects),
        Some(&Action::ReplaceTodo {
            project: Box::new(project.clone()),
            ordinal: 1,
            was: "delivered the video".to_string(),
            text: "deliver the masters".to_string(),
        })
    );
    done_ok(&mut app, "Todo reworded.");
    assert!(app.pane_edit.is_none(), "a landed edit closes");

    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 0, .. })
    });
    press(&mut app, Key::plain(KeyCode::F(2)));
    press(&mut app, Key::ctrl('u'));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(sent(&effects), Some(Action::ReplaceTodo { ordinal: 0, text, .. }) if text.is_empty()),
        "emptied, the todo is removed: {effects:?}"
    );
}

/// A todo that changed on disk since the pane read it is refused, and the
/// refusal lands under the line with what was typed still there.
#[test]
fn a_todo_changed_on_disk_meanwhile_is_refused_under_the_line() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 1, .. })
    });
    press(&mut app, Key::plain(KeyCode::F(2)));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "send the masters");
    press(&mut app, Key::plain(KeyCode::Enter));
    let id = app.busy_id.unwrap();
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("the todo changed meanwhile — reload and try again".to_string()),
        },
    );
    match &app.pane_edit {
        Some(PaneEdit::Line {
            input,
            error,
            pending,
            ..
        }) => {
            assert_eq!(input.text(), "send the masters", "the text is kept");
            assert!(error.as_deref().is_some_and(|e| e.contains("meanwhile")));
            assert!(!pending, "and can be sent again");
        }
        other => panic!("the edit stays open with the refusal: {other:?}"),
    }
}

/// **`+` adds where the cursor is, and keeps the line open for the next.** On
/// a todo under a phase the new one lands at the end of that phase; Enter
/// writes it, and once it has landed the next line is already open under it,
/// so a list is typed in one go. An empty Enter ends the run.
#[test]
fn plus_adds_into_the_cursors_phase_and_keeps_the_line_open_for_the_next() {
    let mut app = editing_fixture();
    with_todos(
        &mut app,
        &[
            (true, "write the brief", Some("Setup")),
            (false, "book the room", Some("Setup")),
            (false, "cut", Some("Edit")),
        ],
    );
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 0, .. })
    });
    press(&mut app, Key::ch('+'));
    let rows = app.pane_rows();
    let adding = rows
        .iter()
        .position(|row| *row == PaneRow::Adding)
        .expect("the add line");
    assert!(
        matches!(rows[adding - 1], PaneRow::Todo { ordinal: 1, .. })
            && matches!(rows[adding + 1], PaneRow::Phase { .. }),
        "at the end of the Setup phase: {rows:?}"
    );
    assert_eq!(app.pane_cursor, adding, "the cursor is where the typing is");

    type_text(&mut app, "send the invite");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(sent(&effects), Some(Action::AddTodos { texts, place, .. })
            if texts == &["send the invite".to_string()]
                && *place == fastf::core::body::TodoPlace::Phase("Setup".to_string())),
        "{effects:?}"
    );
    added(&mut app, 2);
    assert!(
        app.pane_edit.as_ref().is_some_and(PaneEdit::is_adding),
        "the next line is open"
    );
    // The re-read lands with the new todo in it: the line sits under it.
    with_todos(
        &mut app,
        &[
            (true, "write the brief", Some("Setup")),
            (false, "book the room", Some("Setup")),
            (false, "send the invite", Some("Setup")),
            (false, "cut", Some("Edit")),
        ],
    );
    let rows = app.pane_rows();
    let adding = rows.iter().position(|row| *row == PaneRow::Adding).unwrap();
    assert!(
        matches!(&rows[adding - 1], PaneRow::Todo { ordinal: 2, text, .. } if text == "send the invite"),
        "{rows:?}"
    );
    assert_eq!(app.pane_cursor, adding);
    assert!(!app.pane_pulses.is_empty(), "the todo that landed pulses");

    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&effects).is_none(), "an empty line writes nothing");
    assert!(app.pane_edit.is_none(), "and ends the run");
    let rows = app.pane_rows();
    assert!(
        matches!(&rows[app.pane_cursor], PaneRow::Todo { ordinal: 2, .. }),
        "the cursor is on the last todo it added, never on a heading: {:?}",
        rows[app.pane_cursor]
    );
}

/// **Keys typed while a todo is being written are kept.** Enter empties the
/// line at once, so the next todo's first letters land in it while the write
/// is on its way; an Enter then waits its turn and goes when the first has
/// landed. Nothing typed is lost to the write.
#[test]
fn keys_typed_while_a_todo_is_written_are_kept_and_the_next_waits_its_turn() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    press(&mut app, Key::ch('+'));
    type_text(&mut app, "one");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(sent(&effects), Some(Action::AddTodos { texts, .. }) if texts == &["one"]));
    type_text(&mut app, "two");
    match &app.pane_edit {
        Some(PaneEdit::Line { input, .. }) => {
            assert_eq!(input.text(), "two", "typed while writing")
        }
        other => panic!("the add line: {other:?}"),
    }
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&effects).is_none(), "one write at a time: it waits");
    type_text(&mut app, "thr");

    // The first lands: the second goes at once, and "thr" is still there.
    let id = app.busy_id.unwrap();
    let path = app.library.selected().unwrap().path.clone();
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(
                fastf::tui::effect::ActionOutcome::new(
                    fastf::tui::effect::ListChange::DetailOnly { path },
                    "Todo added.",
                )
                .todo(2),
            )),
        },
    );
    assert!(
        matches!(sent(&effects), Some(Action::AddTodos { texts, .. }) if texts == &["two"]),
        "the waiting one goes when the first lands: {effects:?}"
    );
    match &app.pane_edit {
        Some(PaneEdit::Line { input, .. }) => assert_eq!(input.text(), "thr"),
        other => panic!("the add line: {other:?}"),
    }

    // Esc while the second is on its way: done, once it has landed.
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.pane_edit.is_some(), "the line waits for its write");
    added(&mut app, 3);
    assert!(app.pane_edit.is_none(), "then it closes");
}

/// A refused add gives its text back to the line when nothing was typed
/// after it, and names what was not added when something was.
#[test]
fn a_refused_add_gives_its_words_back() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    press(&mut app, Key::ch('+'));
    type_text(&mut app, "invoice");
    press(&mut app, Key::plain(KeyCode::Enter));
    let id = app.busy_id.unwrap();
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("the project changed meanwhile".to_string()),
        },
    );
    match &app.pane_edit {
        Some(PaneEdit::Line { input, error, .. }) => {
            assert_eq!(input.text(), "invoice", "back on the line");
            assert!(error.as_deref().is_some_and(|e| e.contains("meanwhile")));
        }
        other => panic!("the add line stays: {other:?}"),
    }

    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "first");
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "second");
    let id = app.busy_id.unwrap();
    update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("refused".to_string()),
        },
    );
    match &app.pane_edit {
        Some(PaneEdit::Line { input, error, .. }) => {
            assert_eq!(input.text(), "second", "what was typed after is kept");
            assert!(
                error
                    .as_deref()
                    .is_some_and(|e| e.contains("not added: first")),
                "and the refusal names the one not written: {error:?}"
            );
        }
        other => panic!("the add line stays: {other:?}"),
    }
}

/// **Where an add landed is the writer's answer**: with a phase named twice,
/// the cursor settles on the todo the file has, not on a guess.
#[test]
fn an_add_lands_where_the_writer_says() {
    let mut app = editing_fixture();
    with_todos(
        &mut app,
        &[
            (false, "a", Some("Shoot")),
            (false, "b", Some("Deliver")),
            (false, "c", Some("Shoot")),
        ],
    );
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 2, .. })
    });
    press(&mut app, Key::ch('+'));
    let rows = app.pane_rows();
    let at = rows.iter().position(|row| *row == PaneRow::Adding).unwrap();
    assert!(
        matches!(rows[at - 1], PaneRow::Todo { ordinal: 2, .. }),
        "the line opens in the last Shoot run, where the writer puts it: {rows:?}"
    );
    type_text(&mut app, "d");
    press(&mut app, Key::plain(KeyCode::Enter));
    added(&mut app, 3);
    press(&mut app, Key::plain(KeyCode::Esc));
    with_todos(
        &mut app,
        &[
            (false, "a", Some("Shoot")),
            (false, "b", Some("Deliver")),
            (false, "c", Some("Shoot")),
            (false, "d", Some("Shoot")),
        ],
    );
    let rows = app.pane_rows();
    assert!(
        matches!(&rows[app.pane_cursor], PaneRow::Todo { ordinal: 3, text, .. } if text == "d"),
        "{:?}",
        rows[app.pane_cursor]
    );
}

/// Esc on an add line opened in a phase that is not the last leaves the
/// cursor on the row `+` was pressed on — not on the heading that took the
/// line's place.
#[test]
fn closing_an_add_line_leaves_the_cursor_on_a_row_it_can_rest_on() {
    let mut app = editing_fixture();
    with_todos(
        &mut app,
        &[(false, "a", Some("Shoot")), (false, "b", Some("Deliver"))],
    );
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 0, .. })
    });
    press(&mut app, Key::ch('+'));
    press(&mut app, Key::plain(KeyCode::Esc));
    let rows = app.pane_rows();
    assert!(
        rows[app.pane_cursor].selectable(),
        "{:?}",
        rows[app.pane_cursor]
    );
    assert!(matches!(
        rows[app.pane_cursor],
        PaneRow::Todo { ordinal: 0, .. }
    ));
    // Leaving the pane with the line open does the same.
    press(&mut app, Key::ch('+'));
    press(&mut app, Key::plain(KeyCode::Left));
    press(&mut app, Key::plain(KeyCode::Right));
    let rows = app.pane_rows();
    assert!(
        rows[app.pane_cursor].selectable(),
        "{:?}",
        rows[app.pane_cursor]
    );
}

/// `+` on a todo that sits under no phase, in a list that has phases below,
/// adds beside it — not at the end of the last phase.
#[test]
fn plus_on_a_loose_todo_adds_with_the_loose_ones() {
    let mut app = editing_fixture();
    with_todos(
        &mut app,
        &[(false, "loose", None), (false, "cut", Some("Edit"))],
    );
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 0, .. })
    });
    press(&mut app, Key::ch('+'));
    let rows = app.pane_rows();
    let at = rows.iter().position(|row| *row == PaneRow::Adding).unwrap();
    assert!(matches!(rows[at - 1], PaneRow::Todo { ordinal: 0, .. }));
    assert!(matches!(rows[at + 1], PaneRow::Phase { .. }));
    type_text(&mut app, "also loose");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(
            sent(&effects),
            Some(Action::AddTodos {
                place: fastf::core::body::TodoPlace::Loose,
                ..
            })
        ),
        "{effects:?}"
    );
}

/// A paste onto the add line goes in at the caret: what was typed is the
/// start of the first todo, and a single pasted line loses its checklist box.
#[test]
fn a_paste_onto_the_add_line_keeps_what_was_typed() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    press(&mut app, Key::ch('+'));
    type_text(&mut app, "col");
    let effects = update(
        &mut app,
        Msg::Paste(
            "our
sound mix"
                .to_string(),
        ),
    );
    assert!(
        matches!(sent(&effects), Some(Action::AddTodos { texts, .. })
            if texts == &["colour", "sound mix"]),
        "{effects:?}"
    );

    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    press(&mut app, Key::ch('+'));
    update(&mut app, Msg::Paste("- [ ] export the stills".to_string()));
    match &app.pane_edit {
        Some(PaneEdit::Line { input, .. }) => assert_eq!(input.text(), "export the stills"),
        other => panic!("the add line: {other:?}"),
    }
}

/// A todo with no words — a `- [ ]` left in the file — is removed the way
/// any todo is: F2, then Enter on the empty line.
#[test]
fn an_empty_todo_can_be_removed_with_f2() {
    let mut app = editing_fixture();
    with_todos(&mut app, &[(false, "", None), (false, "b", None)]);
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Todo { ordinal: 0, .. })
    });
    press(&mut app, Key::plain(KeyCode::F(2)));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(sent(&effects), Some(Action::ReplaceTodo { ordinal: 0, text, .. }) if text.is_empty()),
        "{effects:?}"
    );
}

/// F2 on the name is the rename, with the rename's rule: with projects
/// marked it is not offered, and says why when pressed.
#[test]
fn f2_on_the_name_keeps_the_renames_rule_about_marks() {
    use fastf::tui::command::{Availability, CommandId, find};

    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::Name(_)));
    press(&mut app, Key::ch(' '));
    assert!(matches!(
        (find(CommandId::PaneEditText).available)(&app),
        Availability::Disabled(_)
    ));
}

/// **A pasted list is that many todos, in one write**, the markers a
/// checklist is copied with taken off.
#[test]
fn a_pasted_list_becomes_one_todo_per_line() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    press(&mut app, Key::ch('+'));
    let effects = update(
        &mut app,
        Msg::Paste("- [ ] colour\n* sound mix\n\n3. deliver the masters\n".to_string()),
    );
    assert!(
        matches!(sent(&effects), Some(Action::AddTodos { texts, place: fastf::core::body::TodoPlace::End, .. })
            if texts == &["colour", "sound mix", "deliver the masters"]),
        "one todo per line, the markers off, in one write: {effects:?}"
    );
    // The line stays open and takes keys while the write is on its way.
    match &app.pane_edit {
        Some(edit @ PaneEdit::Line { input, pending, .. }) => {
            assert!(edit.adding_in_flight(), "the write is on its way");
            assert!(!pending, "and the line still takes keys");
            assert!(input.is_empty(), "emptied for what comes next");
        }
        other => panic!("the add line is still there: {other:?}"),
    }
}

/// The pane's bar on a todo says what each key does there: Enter ticks, F2
/// edits, `+` adds — and on the add line, Enter adds and Esc is done.
#[test]
fn a_todo_row_says_enter_ticks_f2_edits_plus_adds() {
    use fastf::tui::command::hints;

    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::Todo { .. }));
    let bar = hints(Context::Detail, &app, 118);
    for pair in [
        ("Enter", "toggle"),
        ("F2", "edit"),
        ("+", "add"),
        ("←", "list"),
        ("?", "help"),
    ] {
        assert!(
            bar.iter().any(|(k, t)| k == pair.0 && *t == pair.1),
            "{pair:?} is on the pane's bar: {bar:?}"
        );
    }
    assert!(
        !bar.iter().any(|(k, _)| k == "o" || k == "T"),
        "the list's own verbs stay on the list's bar: {bar:?}"
    );
    press(&mut app, Key::ch('+'));
    let bar = hints(Context::PaneEdit, &app, 118);
    assert!(
        bar.iter().any(|(k, t)| k == "Enter" && *t == "add"),
        "{bar:?}"
    );
    assert!(
        bar.iter().any(|(k, t)| k == "Esc" && *t == "done"),
        "{bar:?}"
    );
}

/// **A paste's line breaks are the terminal's**, and many terminals send a
/// bare carriage return: a list pasted that way is still a list, and a note
/// pasted that way still has its lines.
#[test]
fn a_paste_with_carriage_returns_is_still_several_lines() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
    press(&mut app, Key::ch('+'));
    let effects = update(&mut app, Msg::Paste("colour\rsound\r\ndeliver".to_string()));
    assert!(
        matches!(sent(&effects), Some(Action::AddTodos { texts, .. })
            if texts == &["colour", "sound", "deliver"]),
        "{effects:?}"
    );

    let mut app = editing_fixture();
    go_to(&mut app, |row| {
        matches!(row, PaneRow::Note { ordinal: 0, .. })
    });
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::ctrl('e'));
    update(&mut app, Msg::Paste("\rsecond\rthird".to_string()));
    match &app.pane_edit {
        Some(PaneEdit::Note { area, .. }) => {
            assert_eq!(area.text(), "first cut Friday\nsecond\nthird");
        }
        other => panic!("the note editor: {other:?}"),
    }
}

/// `>` from a variable to a project with no variables lands nowhere in
/// particular — and forgets it was looking, so a refresh that later brings
/// variables in does not move the cursor unasked.
#[test]
fn a_walk_to_a_project_without_the_section_forgets_it() {
    use fastf::tui::app::pane::{PaneSection, section_at};

    let mut app = editing_fixture();
    let with_variables = app
        .details
        .get(&app.library.selected().unwrap().path)
        .cloned()
        .unwrap();
    go_to(&mut app, |row| matches!(row, PaneRow::Variable { .. }));
    press(&mut app, Key::ch('>'));
    let path = app.library.selected().unwrap().path.clone();
    update(
        &mut app,
        Msg::Detail {
            path: path.clone(),
            detail: Box::new(ProjectDetail::default()),
        },
    );
    let at = app.pane_cursor;
    assert_ne!(section_at(&app.pane_rows(), at), PaneSection::Variables);
    // The file gains variables later, and the pane re-reads it.
    update(
        &mut app,
        Msg::Detail {
            path,
            detail: Box::new(with_variables),
        },
    );
    assert!(
        !matches!(app.pane_rows()[app.pane_cursor], PaneRow::Variable { .. }),
        "the cursor did not jump to a variable nobody asked for"
    );
}
