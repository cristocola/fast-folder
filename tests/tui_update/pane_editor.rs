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
            },
            fastf::core::body::Todo {
                done: false,
                text: "delivered the video".to_string(),
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

    go_to(&mut app, |row| matches!(row, PaneRow::Name));
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
        matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
        "add a todo asks for its text"
    );
    type_text(&mut app, "invoice");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let project = app.library.selected().unwrap().clone();
    assert_eq!(
        sent(&effects),
        Some(&Action::AddTodo {
            project: Box::new(project.clone()),
            text: "invoice".to_string(),
        })
    );
    assert!(app.modals.is_empty());
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
            },
            fastf::core::body::Todo {
                done: true,
                text: "delivered the video".to_string(),
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
