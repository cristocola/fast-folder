//! Todos and phases in the pane: Enter ticks, F2 rewords, `+` adds where the
//! todo will land, and a phase is written with its first todo.

use crate::harness::*;
use crate::pane_editor::{editing_fixture, go_to, sent};
use fastf::tui::app::actions::TextThen;
use fastf::tui::app::data::ProjectDetail;
use fastf::tui::app::pane::{PaneEdit, PaneRow};
use fastf::tui::command::Context;
use fastf::tui::effect::Action;

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

/// **A phase is named where its heading will be, then filled.** `P` opens a
/// line at the end of the list; Enter on a name draws the heading and opens
/// the todo line under it; the first todo is sent into that phase, which the
/// writer opens with it. Nothing is written before a todo is.
#[test]
fn p_names_a_phase_where_it_will_land_and_its_first_todo_writes_it() {
    let mut app = editing_fixture();
    with_todos(&mut app, &[(false, "read the order", None)]);
    press(&mut app, Key::ch('P'));
    assert_eq!(app.context(), Context::PaneEdit, "the line takes the keys");
    let rows = app.pane_rows();
    let at = rows.iter().position(|row| *row == PaneRow::Adding).unwrap();
    assert!(matches!(rows[at - 1], PaneRow::Todo { ordinal: 0, .. }));
    assert_eq!(rows[at + 1], PaneRow::AddTodo);

    type_text(&mut app, "## Grade:");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&effects).is_none(), "a name alone writes nothing");
    let rows = app.pane_rows();
    let at = rows.iter().position(|row| *row == PaneRow::Adding).unwrap();
    assert_eq!(
        rows[at - 1],
        PaneRow::Phase {
            name: "Grade".to_string(),
            done: 0,
            total: 0
        },
        "the heading reads as the file will"
    );
    assert_eq!(app.pane_cursor, at, "the cursor is on the todo line");

    type_text(&mut app, "match the reference");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(
            sent(&effects),
            Some(Action::AddTodos { texts, place: fastf::core::body::TodoPlace::Phase(name), .. })
                if texts == &["match the reference"] && name == "Grade"
        ),
        "{effects:?}"
    );
    added(&mut app, 1);
    assert!(app.pane_edit.is_some(), "the line stays for the next todo");
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.pane_edit.is_none());
    assert!(
        !app.status.text.contains("no phase"),
        "a phase with a todo in it was written: {}",
        app.status.text
    );
}

/// Left without a todo, the heading was only drawn, and the status says so.
/// An empty name is a cancel; a name that is only markdown is refused.
#[test]
fn a_phase_left_empty_is_not_written_and_says_so() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddPhase));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(sent(&effects).is_none());
    assert!(app.pane_edit.is_some(), "Enter on the row opens the line");

    type_text(&mut app, "##");
    press(&mut app, Key::plain(KeyCode::Enter));
    match &app.pane_edit {
        Some(PaneEdit::Line { error, input, .. }) => {
            assert_eq!(error.as_deref(), Some("a phase needs a name"));
            assert_eq!(input.text(), "##", "the text is kept");
        }
        other => panic!("the naming line: {other:?}"),
    }
    press(&mut app, Key::plain(KeyCode::Backspace));
    press(&mut app, Key::plain(KeyCode::Backspace));
    type_text(&mut app, "Delivery");
    press(&mut app, Key::plain(KeyCode::Enter));
    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(sent(&effects).is_none());
    assert!(app.pane_edit.is_none());
    assert!(
        app.status.text.contains("no todo, so no phase"),
        "{}",
        app.status.text
    );
    assert!(
        !app.pane_rows()
            .iter()
            .any(|row| matches!(row, PaneRow::Phase { .. })),
        "the drawn heading goes with the line"
    );
    assert_eq!(
        app.pane_rows()[app.pane_cursor],
        PaneRow::AddPhase,
        "back where it started"
    );

    // An empty Enter on the name is a cancel, and says nothing.
    press(&mut app, Key::ch('P'));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(app.pane_edit.is_none());
}

/// Where the pane cannot show the list, the name and the first todo are
/// asked for in turn, and written together.
#[test]
fn without_the_list_on_screen_a_phase_is_two_prompts() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('P'));
    assert!(
        matches!(
            app.modals.top(),
            Some(Modal::TextPrompt(prompt)) if matches!(prompt.then, TextThen::AddPhase(_))
        ),
        "no detail read: the name is asked for"
    );
    type_text(&mut app, "Main Edit");
    press(&mut app, Key::plain(KeyCode::Enter));
    match app.modals.top() {
        Some(Modal::TextPrompt(prompt)) => assert!(
            prompt.title.contains("Main Edit"),
            "the todo prompt names its phase: {}",
            prompt.title
        ),
        other => panic!("the todo prompt: {other:?}"),
    }
    type_text(&mut app, "cut the first minute");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(
            sent(&effects),
            Some(Action::AddTodos { texts, place: fastf::core::body::TodoPlace::Phase(name), .. })
                if texts == &["cut the first minute"] && name == "Main Edit"
        ),
        "{effects:?}"
    );
}

/// `+` on "add a phase" is one more phase, as on "add a tag" it is a tag.
#[test]
fn plus_on_add_a_phase_adds_a_phase() {
    let mut app = editing_fixture();
    go_to(&mut app, |row| matches!(row, PaneRow::AddPhase));
    press(&mut app, Key::ch('+'));
    assert!(
        matches!(
            app.pane_edit,
            Some(PaneEdit::Line {
                target: fastf::tui::app::pane::EditTarget::NewPhase { .. },
                ..
            })
        ),
        "{:?}",
        app.pane_edit
    );
}
