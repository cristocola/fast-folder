use super::*;
use crate::core::project_info::Metadata;
use crate::core::template::{Transform, Variable};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn project(tags: &[&str], template: &str) -> Project {
    Project {
        id: "ID0001".to_string(),
        id_number: Some(1),
        template: template.to_string(),
        template_name: template.to_string(),
        path: PathBuf::from("/mnt/projects/ID0001_One"),
        name: "ID0001_One".to_string(),
        base: PathBuf::from("/mnt/projects"),
        created: "2026-01-01T00:00:00Z".to_string(),
        description: String::new(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        exists: true,
    }
}

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

fn metadata(variables: &[(&str, &str)]) -> Metadata {
    Metadata {
        id: "ID0001".to_string(),
        id_number: Some(1),
        template: "client".to_string(),
        template_name: "Client".to_string(),
        description: String::new(),
        created: String::new(),
        folder: String::new(),
        path: String::new(),
        variables: variables
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        tags: Vec::new(),
        auto_tags: Vec::new(),
        provisioning: false,
    }
}

#[test]
fn rows_are_one_per_tag_then_add_tag_and_reading_until_the_detail_lands() {
    let rows = pane_rows(&project(&["draft", "client/Acme"], "client"), None, 0);
    assert_eq!(
        rows,
        vec![
            PaneRow::Name("ID0001_One".to_string()),
            // The row is there to be filled even when nothing was written.
            PaneRow::Description(String::new()),
            PaneRow::Facts(vec![Fact::Template, Fact::Base, Fact::Created]),
            // Before the read the size is all the pane knows.
            PaneRow::Facts(vec![Fact::Size]),
            PaneRow::Rule(PaneSection::Tags),
            PaneRow::Tag("draft".to_string()),
            PaneRow::Tag("client/Acme".to_string()),
            PaneRow::AddTag,
            PaneRow::Reading,
        ]
    );
}

fn note(timestamp: Option<&str>, text: &str) -> crate::core::body::Note {
    crate::core::body::Note {
        timestamp: timestamp.map(str::to_string),
        text: text.to_string(),
    }
}

#[test]
fn a_new_phase_is_drawn_where_the_writer_will_open_it() {
    let todo = |text: &str, phase: Option<&str>| crate::core::body::Todo {
        done: false,
        text: text.to_string(),
        phase: phase.map(str::to_string),
    };
    let heading = |name: &str, total: usize| PaneRow::Phase {
        name: name.to_string(),
        done: 0,
        total,
    };
    let todo_rows = |rows: Vec<PaneRow>| -> Vec<PaneRow> {
        rows.into_iter()
            .skip_while(|r| *r != PaneRow::Rule(PaneSection::Todo))
            .skip(1)
            .take_while(|r| *r != PaneRow::Rule(PaneSection::Notes))
            .filter(|r| !matches!(r, PaneRow::Todo { .. }))
            .collect()
    };
    let plain = ProjectDetail {
        todos: vec![todo("read the order", None)],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&plain), 0);

    // The name is typed at the end of the list, where the label will go.
    assert_eq!(
        todo_rows(with_naming(rows.clone())),
        vec![PaneRow::Adding, PaneRow::AddTodo, PaneRow::AddPhase]
    );
    // Named, the heading is drawn over the line its todos are typed on.
    assert_eq!(
        todo_rows(with_adding(rows, &TodoPlace::Phase("Grade".to_string()))),
        vec![
            heading("Grade", 0),
            PaneRow::Adding,
            PaneRow::AddTodo,
            PaneRow::AddPhase
        ]
    );

    // A list that keeps an `### Other` opens a new label above it.
    let other = ProjectDetail {
        todos: vec![
            todo("download", Some("Setup")),
            todo("chase the invoice", Some("Other")),
        ],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&other), 0);
    assert_eq!(
        todo_rows(with_naming(rows.clone())),
        vec![
            heading("Setup", 1),
            PaneRow::Adding,
            heading("Other", 1),
            PaneRow::AddTodo,
            PaneRow::AddPhase
        ]
    );
    assert_eq!(
        todo_rows(with_adding(
            rows.clone(),
            &TodoPlace::Phase("Grade".to_string())
        )),
        vec![
            heading("Setup", 1),
            heading("Grade", 0),
            PaneRow::Adding,
            heading("Other", 1),
            PaneRow::AddTodo,
            PaneRow::AddPhase
        ]
    );
    // A name the list has, in any case, is that phase: no second label.
    assert_eq!(
        todo_rows(with_adding(rows, &TodoPlace::Phase("setup".to_string()))),
        vec![
            heading("Setup", 1),
            PaneRow::Adding,
            heading("Other", 1),
            PaneRow::AddTodo,
            PaneRow::AddPhase
        ]
    );
}

/// A label whose tasks were all removed stays in the file, and a new
/// todo in its phase goes under it: the pane draws it, and draws the line
/// there, rather than opening a second heading at the end.
#[test]
fn an_empty_label_is_drawn_and_a_todo_for_it_lands_under_it() {
    let label = |name: &str, before| crate::core::body::PhaseLabel {
        name: name.to_string(),
        before,
    };
    let heading = |name: &str, total: usize| PaneRow::Phase {
        name: name.to_string(),
        done: 0,
        total,
    };
    let todo_rows = |rows: Vec<PaneRow>| -> Vec<PaneRow> {
        rows.into_iter()
            .skip_while(|r| *r != PaneRow::Rule(PaneSection::Todo))
            .skip(1)
            .take_while(|r| *r != PaneRow::Rule(PaneSection::Notes))
            .map(|r| match r {
                PaneRow::Todo { ordinal, .. } => PaneRow::More(ordinal),
                other => other,
            })
            .collect()
    };
    let detail = ProjectDetail {
        todos: vec![crate::core::body::Todo {
            done: false,
            text: "a".to_string(),
            phase: Some("Setup".to_string()),
        }],
        phases: vec![label("Grade", 0), label("Setup", 0), label("Review", 1)],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
    assert_eq!(
        todo_rows(rows.clone()),
        vec![
            heading("Grade", 0),
            heading("Setup", 1),
            PaneRow::More(0),
            heading("Review", 0),
            PaneRow::AddTodo,
            PaneRow::AddPhase
        ]
    );
    assert_eq!(
        todo_rows(with_adding(
            rows.clone(),
            &TodoPlace::Phase("grade".to_string())
        ))[1],
        PaneRow::Adding,
        "under the empty label the writer will find"
    );
    // A new phase opens under the last task, above the empty label after it.
    assert_eq!(
        todo_rows(with_naming(rows.clone()))[3],
        PaneRow::Adding,
        "{:?}",
        todo_rows(with_naming(rows))
    );

    // An empty `### Other` at the top: a new phase opens above it.
    let other_first = ProjectDetail {
        phases: vec![label("Other", 0), label("Setup", 0)],
        ..detail
    };
    let rows = pane_rows(&project(&[], "client"), Some(&other_first), 0);
    assert_eq!(todo_rows(with_naming(rows))[0], PaneRow::Adding);
}

#[test]
fn a_phase_label_is_drawn_where_it_changes_and_the_cursor_steps_over_it() {
    let todo = |done: bool, text: &str, phase: Option<&str>| crate::core::body::Todo {
        done,
        text: text.to_string(),
        phase: phase.map(str::to_string),
    };
    let detail = ProjectDetail {
        todos: vec![
            todo(true, "read the order", None),
            todo(false, "download the files", Some("Setup")),
            todo(false, "copy the audio", Some("Setup")),
            todo(false, "listen to the song", Some("Creative Plan")),
        ],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
    let todo_rows: Vec<&PaneRow> = rows
        .iter()
        .skip_while(|r| **r != PaneRow::Rule(PaneSection::Todo))
        .take_while(|r| **r != PaneRow::Rule(PaneSection::Notes))
        .collect();
    assert_eq!(
        todo_rows,
        vec![
            &PaneRow::Rule(PaneSection::Todo),
            &PaneRow::Todo {
                ordinal: 0,
                done: true,
                text: "read the order".to_string(),
                phased: false,
            },
            &PaneRow::Phase {
                name: "Setup".to_string(),
                done: 0,
                total: 2,
            },
            &PaneRow::Todo {
                ordinal: 1,
                done: false,
                text: "download the files".to_string(),
                phased: true,
            },
            // The second task of a run draws no second label.
            &PaneRow::Todo {
                ordinal: 2,
                done: false,
                text: "copy the audio".to_string(),
                phased: true,
            },
            &PaneRow::Phase {
                name: "Creative Plan".to_string(),
                done: 0,
                total: 1,
            },
            &PaneRow::Todo {
                ordinal: 3,
                done: false,
                text: "listen to the song".to_string(),
                phased: true,
            },
            &PaneRow::AddTodo,
            &PaneRow::AddPhase,
        ],
        "a label where the phase changes, and the ordinals still count tasks only"
    );
    assert!(
        !PaneRow::Phase {
            name: "Setup".to_string(),
            done: 0,
            total: 2
        }
        .selectable(),
        "Enter on a phase would have nothing to toggle"
    );
}

#[test]
fn selectable_rows_skip_the_facts_the_rules_the_listing_and_a_notes_other_lines() {
    let detail = ProjectDetail {
        listing: vec![Entry {
            name: "src".to_string(),
            is_dir: true,
        }],
        notes: vec![
            note(None, "free text\nsecond line"),
            note(Some("2026-01-01T10:00:00Z"), "began"),
        ],
        todos: vec![crate::core::body::Todo {
            done: true,
            text: "ingested".to_string(),
            phase: None,
        }],
        ..Default::default()
    };
    let rows = pane_rows(&project(&["draft"], "client"), Some(&detail), 0);
    let selectable: Vec<&PaneRow> = rows.iter().filter(|r| r.selectable()).collect();
    assert_eq!(
        selectable,
        vec![
            &PaneRow::Name("ID0001_One".to_string()),
            &PaneRow::Description(String::new()),
            &PaneRow::Tag("draft".to_string()),
            &PaneRow::AddTag,
            &PaneRow::Todo {
                ordinal: 0,
                done: true,
                text: "ingested".to_string(),
                phased: false,
            },
            &PaneRow::AddTodo,
            &PaneRow::AddPhase,
            &PaneRow::Note {
                ordinal: 0,
                date: None,
                first: "free text".to_string(),
            },
            &PaneRow::Note {
                ordinal: 1,
                date: Some("2026-01-01".to_string()),
                first: "began".to_string(),
            },
            &PaneRow::AddNote,
        ]
    );
    assert!(rows.contains(&PaneRow::Rule(PaneSection::Inside)));
    assert!(rows.contains(&PaneRow::NoteLine("second line".to_string())));
    assert!(!rows.contains(&PaneRow::Reading));
    // The rows of a note sit together, under one rule, over the todos.
    let at = |wanted: &PaneRow| rows.iter().position(|r| r == wanted).unwrap();
    assert_eq!(
        at(&PaneRow::NoteLine("second line".to_string())),
        at(&PaneRow::Note {
            ordinal: 0,
            date: None,
            first: "free text".to_string()
        }) + 1
    );
    assert!(at(&PaneRow::Rule(PaneSection::Todo)) < at(&PaneRow::AddTodo));
    assert!(at(&PaneRow::AddTodo) < at(&PaneRow::Rule(PaneSection::Notes)));
    assert!(at(&PaneRow::Rule(PaneSection::Notes)) < at(&PaneRow::AddNote));
    assert!(at(&PaneRow::AddNote) < at(&PaneRow::Rule(PaneSection::Inside)));
}

#[test]
fn the_latest_notes_are_shown_under_earlier_and_a_long_note_is_cut_with_more() {
    let long = (0..12)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let detail = ProjectDetail {
        notes: (0..8)
            .map(|n| note(Some("2026-01-01"), if n == 7 { &long } else { "short" }))
            .collect(),
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
    assert!(rows.contains(&PaneRow::EarlierNotes(3)));
    let shown: Vec<usize> = rows
        .iter()
        .filter_map(|r| match r {
            PaneRow::Note { ordinal, .. } => Some(*ordinal),
            _ => None,
        })
        .collect();
    assert_eq!(shown, vec![3, 4, 5, 6, 7], "the latest, in file order");
    let lines = rows
        .iter()
        .filter(|r| matches!(r, PaneRow::NoteLine(_)))
        .count();
    assert_eq!(lines, NOTE_LINES_SHOWN - 1);
    assert!(rows.contains(&PaneRow::NoteMore(12 - NOTE_LINES_SHOWN)));
    assert_eq!(
        rows.iter()
            .position(|r| matches!(r, PaneRow::EarlierNotes(_))),
        Some(
            rows.iter()
                .position(|r| r == &PaneRow::Rule(PaneSection::Notes))
                .unwrap()
                + 1
        )
    );
}

#[test]
fn variables_follow_the_template_and_a_select_offers_only_its_options() {
    let detail = ProjectDetail {
        meta: Some(metadata(&[
            ("tier", "Indie"),
            ("name", "One"),
            ("extra", "x"),
        ])),
        variables: vec![
            variable("name", VarType::Text, &[]),
            variable("tier", VarType::Select, &["Indie", "Major"]),
        ],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
    let variables: Vec<&PaneRow> = rows
        .iter()
        .filter(|r| matches!(r, PaneRow::Variable { .. }))
        .collect();
    assert_eq!(variables.len(), 3);
    assert_eq!(
        variables[0],
        &PaneRow::Variable {
            slug: "name".to_string(),
            label: "NAME".to_string(),
            kind: VarKind::Text,
            value: "One".to_string(),
        },
        "the template's order, not the alphabet's"
    );
    assert_eq!(
        variables[1],
        &PaneRow::Variable {
            slug: "tier".to_string(),
            label: "TIER".to_string(),
            kind: VarKind::Select(vec!["Indie".to_string(), "Major".to_string()]),
            value: "Indie".to_string(),
        }
    );
    assert_eq!(
        variables[2],
        &PaneRow::Variable {
            slug: "extra".to_string(),
            label: "extra".to_string(),
            kind: VarKind::Text,
            value: "x".to_string(),
        },
        "a variable the template no longer declares is free text, after the others"
    );
}

#[test]
fn a_registered_project_offers_every_variable_as_text() {
    let detail = ProjectDetail {
        meta: Some(metadata(&[("b", "2"), ("a", "1")])),
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "(registered)"), Some(&detail), 0);
    let kinds: Vec<(&str, &VarKind)> = rows
        .iter()
        .filter_map(|r| match r {
            PaneRow::Variable { slug, kind, .. } => Some((slug.as_str(), kind)),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec![("a", &VarKind::Text), ("b", &VarKind::Text)]);
}

#[test]
fn the_cursor_walks_selectable_rows_and_stops() {
    let rows = pane_rows(&project(&["draft"], "client"), None, 0);
    // Name(0) Description(1) Facts Figures Rule Tag(5) AddTag(6) Reading
    assert_eq!(
        step_cursor(&rows, 0, 1),
        1,
        "down from the name is the description"
    );
    assert_eq!(step_cursor(&rows, 1, 1), 5, "over the facts and the rule");
    assert_eq!(step_cursor(&rows, 5, 1), 6);
    assert_eq!(step_cursor(&rows, 6, 1), 6, "the end is the end");
    assert_eq!(step_cursor(&rows, 6, -1), 5);
    assert_eq!(step_cursor(&rows, 0, -1), 0);
    assert_eq!(step_cursor(&rows, 0, isize::MAX), 6);
    assert_eq!(step_cursor(&rows, 6, isize::MIN), 0);
    assert_eq!(
        step_cursor(&rows, 3, 0),
        5,
        "a cursor on a row it may not rest on settles below"
    );
    assert_eq!(step_cursor(&rows, 99, 0), 6, "or on the last, past the end");
}

#[test]
fn a_line_wider_than_the_pane_breaks_at_a_space_and_keeps_every_word() {
    assert_eq!(wrap_columns("one two three", 0), ["one two three"]);
    assert_eq!(wrap_columns("one two three", 13), ["one two three"]);
    assert_eq!(wrap_columns("one two three", 8), ["one two", "three"]);
    assert_eq!(wrap_columns("one two three", 7), ["one two", "three"]);
    assert_eq!(
        wrap_columns("abcdefghij", 4),
        ["abcd", "efgh", "ij"],
        "a word wider than a line is broken where it must be"
    );
    assert_eq!(
        wrap_columns("  indented words here", 12),
        ["  indented", "words here"],
        "the indent a line starts with stays, and is never a row of its own"
    );
    assert_eq!(
        wrap_columns("日本語のメモ", 6),
        ["日本語", "のメモ"],
        "measured in display columns, not characters"
    );
}

#[test]
fn a_long_note_and_a_long_todo_continue_on_the_rows_under_them() {
    let detail = ProjectDetail {
        notes: vec![note(
            Some("2026-01-01T10:00:00Z"),
            "recut the second verse around the new take\nthen colour",
        )],
        todos: vec![crate::core::body::Todo {
            done: false,
            text: "send the rough cut to the label".to_string(),
            phase: None,
        }],
        ..Default::default()
    };
    // 31 columns inside the border: 20 for a note's text, 27 for a todo's.
    let rows = pane_rows(&project(&[], "client"), Some(&detail), 31);
    let at = rows
        .iter()
        .position(|r| matches!(r, PaneRow::Note { .. }))
        .unwrap();
    assert_eq!(
        rows[at..at + 4],
        [
            PaneRow::Note {
                ordinal: 0,
                date: Some("2026-01-01".to_string()),
                first: "recut the second".to_string(),
            },
            PaneRow::NoteLine("verse around the new".to_string()),
            PaneRow::NoteLine("take".to_string()),
            PaneRow::NoteLine("then colour".to_string()),
        ]
    );
    let todo = rows
        .iter()
        .position(|r| matches!(r, PaneRow::Todo { .. }))
        .unwrap();
    assert_eq!(
        rows[todo..todo + 2],
        [
            PaneRow::Todo {
                ordinal: 0,
                done: false,
                text: "send the rough cut to the".to_string(),
                phased: false,
            },
            PaneRow::TodoLine {
                done: false,
                text: "label".to_string(),
                phased: false,
            },
        ]
    );
    assert!(
        !rows[todo + 1].selectable(),
        "the cursor rests on the todo, not on the rest of it"
    );

    let long = ProjectDetail {
        notes: vec![note(None, &"word ".repeat(200))],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&long), 31);
    let lines = rows
        .iter()
        .filter(|r| matches!(r, PaneRow::NoteLine(_)))
        .count();
    assert_eq!(lines, NOTE_LINES_SHOWN - 1, "wrapped rows count as lines");
    assert!(rows.iter().any(|r| matches!(r, PaneRow::NoteMore(_))));
}

/// A page is counted in drawn rows and lands on the farthest row Enter can
/// act on inside it — or, when the page holds none, the next one past it.
#[test]
fn a_page_moves_by_drawn_rows_and_lands_on_a_row_that_can_be_acted_on() {
    let note = |ordinal| PaneRow::Note {
        ordinal,
        date: None,
        first: String::new(),
    };
    let line = || PaneRow::NoteLine(String::new());
    let mut rows = vec![
        PaneRow::Name(String::new()),
        PaneRow::Facts(Vec::new()),
        PaneRow::Facts(Vec::new()),
        PaneRow::Rule(PaneSection::Tags),
        PaneRow::Tag("draft".to_string()),
        PaneRow::AddTag,
        PaneRow::Rule(PaneSection::Notes),
        note(0),
    ];
    rows.extend((0..5).map(|_| line()));
    rows.push(note(1));
    rows.extend((0..5).map(|_| line()));
    rows.extend([
        PaneRow::AddNote,
        PaneRow::Rule(PaneSection::Todo),
        PaneRow::AddTodo,
    ]);
    let last = rows.len() - 1;

    assert_eq!(page_cursor(&rows, 0, 6), 5, "the farthest inside the page");
    assert_eq!(page_cursor(&rows, 5, 6), 7);
    assert_eq!(
        page_cursor(&rows, 7, 3),
        13,
        "a page of wrapped lines only reaches the next note"
    );
    assert_eq!(page_cursor(&rows, 13, 100), last, "and stops at the end");
    assert_eq!(page_cursor(&rows, last, 100), last);

    assert_eq!(page_cursor(&rows, 13, -6), 7, "up, the farthest back");
    assert_eq!(page_cursor(&rows, 7, -1), 5, "or the next one before it");
    assert_eq!(page_cursor(&rows, 0, -10), 0, "and stops at the top");
    assert_eq!(page_cursor(&[], 3, 5), 0, "no rows, no cursor");
}

/// A name wider than the pane wraps after the joints of a fastf name —
/// `_`, `-`, `.`, a space — so a row ends on a whole part of it, and
/// inside a part only when the part alone is wider than a row. Nothing is
/// lost: the rows are the name.
#[test]
fn a_long_name_wraps_after_its_separators() {
    let name = "2026-01-02_acme_studio_Spring_Campaign-Extended_Directors_Cut_ID0201";
    let rows = wrap_name(name, 24);
    assert_eq!(rows.concat(), name, "nothing dropped: {rows:?}");
    assert!(rows.iter().all(|row| row.width() <= 24), "{rows:?}");
    for row in &rows[..rows.len() - 1] {
        assert!(
            row.ends_with(['_', '-', '.', ' ']),
            "a row ends on a joint: {rows:?}"
        );
    }
    assert_eq!(wrap_name("short", 24), vec!["short"]);
    assert_eq!(wrap_name(name, 0), vec![name], "no width, no wrap");
    // A part wider than a row is cut inside it, and still goes forward.
    let rows = wrap_name("abcdefghijklmnopqrstuvwxyz", 10);
    assert_eq!(rows, vec!["abcdefghij", "klmnopqrst", "uvwxyz"]);
    let rows = wrap_name("日本語のフォルダ", 3);
    assert_eq!(rows.concat(), "日本語のフォルダ");
    assert!(rows.iter().all(|row| !row.is_empty()));
}

/// Facts flow whole: as many to a row as fit with the gap between them,
/// the next on the row under — never half a date on one row and half on
/// the next.
#[test]
fn facts_flow_whole_and_wrap_between_them() {
    let project = project(&[], "client-project");
    let wide = pane_rows(&project, None, 200);
    assert_eq!(
        wide[2],
        PaneRow::Facts(vec![Fact::Template, Fact::Base, Fact::Created]),
        "a wide pane has what the project is on one row"
    );
    // "client-project" (14) + gap (7) + "projects" (8) = 29; the date's 18
    // more does not fit 40, so it starts the next row.
    let narrow = pane_rows(&project, None, 40);
    assert_eq!(narrow[2], PaneRow::Facts(vec![Fact::Template, Fact::Base]));
    assert_eq!(narrow[3], PaneRow::Facts(vec![Fact::Created]));
    // Narrower than any two: one fact to a row.
    let tight = pane_rows(&project, None, 15);
    let facts: Vec<&PaneRow> = tight
        .iter()
        .filter(|row| matches!(row, PaneRow::Facts(_)))
        .collect();
    assert!(
        facts
            .iter()
            .all(|row| matches!(row, PaneRow::Facts(f) if f.len() == 1))
    );
}

/// The size is measured at the widest a size cell gets, so the rows the
/// figures take cannot change when a size lands — which would move every
/// row under the cursor by one.
#[test]
fn the_figures_row_count_does_not_depend_on_the_size() {
    let project = project(&[], "client");
    let detail = ProjectDetail::default();
    // Size (11) + gap (7) + "no notes" (8) = 26: at 26 they share a row,
    // whatever the size turns out to read.
    let rows = pane_rows(&project, Some(&detail), 26);
    assert!(rows.contains(&PaneRow::Facts(vec![Fact::Size, Fact::Notes(0)])));
    let rows = pane_rows(&project, Some(&detail), 25);
    assert!(rows.contains(&PaneRow::Facts(vec![Fact::Size])));
}

/// The sections you act on come before the ones you look things up in.
#[test]
fn the_living_sections_come_first() {
    let meta = metadata(&[("client", "Acme")]);
    let detail = ProjectDetail {
        meta: Some(meta),
        variables: vec![variable("client", VarType::Text, &[])],
        listing: vec![Entry {
            name: "src".to_string(),
            is_dir: true,
        }],
        ..Default::default()
    };
    let rows = pane_rows(&project(&["draft"], "client"), Some(&detail), 60);
    let rules: Vec<PaneSection> = rows
        .iter()
        .filter_map(|row| match row {
            PaneRow::Rule(section) => Some(*section),
            _ => None,
        })
        .collect();
    assert_eq!(
        rules,
        vec![
            PaneSection::Tags,
            PaneSection::Todo,
            PaneSection::Notes,
            PaneSection::Variables,
            PaneSection::Inside,
        ]
    );
}

/// A pasted line loses the marker a checklist was copied with — a box, a
/// bullet, a number — and nothing else.
#[test]
fn a_pasted_line_loses_its_list_marker_and_nothing_else() {
    for (line, todo) in [
        ("- [ ] colour", "colour"),
        ("- [x] shot the stills", "shot the stills"),
        ("* [X] graded", "graded"),
        ("* sound mix", "sound mix"),
        ("+ export", "export"),
        ("1. deliver", "deliver"),
        ("12) invoice", "invoice"),
        ("[ ] bare box", "bare box"),
        ("  plain words  ", "plain words"),
        (
            "2026-01-02 is a date, not a number",
            "2026-01-02 is a date, not a number",
        ),
        ("3.5 GB to upload", "3.5 GB to upload"),
        ("-dash without a space", "-dash without a space"),
    ] {
        assert_eq!(todo_text_of(line), todo, "{line:?}");
    }
}

/// The add line opens where `body::add_todos_at` will write: the last run
/// of the phase, whatever the case of its name; after the loose tasks;
/// or at the end.
#[test]
fn the_add_line_opens_where_the_todo_will_land() {
    let todo = |text: &str, phase: Option<&str>| crate::core::body::Todo {
        done: false,
        text: text.to_string(),
        phase: phase.map(str::to_string),
    };
    let detail = ProjectDetail {
        todos: vec![
            todo("loose", None),
            todo("brief", Some("Setup")),
            todo("room", Some("Setup")),
            todo("cut", Some("Edit")),
            todo("again", Some("setup")),
        ],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&detail), 0);
    let around = |place: TodoPlace| {
        let rows = with_adding(rows.clone(), &place);
        let at = rows.iter().position(|r| *r == PaneRow::Adding).unwrap();
        (rows[at - 1].clone(), rows[at + 1].clone())
    };
    let (above, below) = around(TodoPlace::Phase("Setup".to_string()));
    assert!(
        matches!(above, PaneRow::Todo { ordinal: 4, .. }) && below == PaneRow::AddTodo,
        "the last run named setup, in any case: {above:?} / {below:?}"
    );
    let (above, below) = around(TodoPlace::Loose);
    assert!(matches!(above, PaneRow::Todo { ordinal: 0, .. }));
    assert!(matches!(below, PaneRow::Phase { .. }), "{below:?}");
    let (_, below) = around(TodoPlace::End);
    assert_eq!(below, PaneRow::AddTodo);
    let (_, below) = around(TodoPlace::Phase("Deliver".to_string()));
    assert_eq!(below, PaneRow::AddTodo, "a phase not there yet: the end");

    // `+` chooses the place from the row it is pressed on.
    let on =
        |wanted: &dyn Fn(&PaneRow) -> bool| place_at(&rows, rows.iter().position(wanted).unwrap());
    assert_eq!(
        on(&|r| matches!(r, PaneRow::Todo { ordinal: 0, .. })),
        TodoPlace::Loose
    );
    assert_eq!(
        on(&|r| matches!(r, PaneRow::Todo { ordinal: 3, .. })),
        TodoPlace::Phase("Edit".to_string())
    );
    assert_eq!(on(&|r| *r == PaneRow::AddTodo), TodoPlace::End);
}

/// A phase heading counts what is done under it, and its tasks sit in from
/// it; a list with no phases is not indented at all.
#[test]
fn a_phase_heading_counts_its_tasks_and_indents_them() {
    let todo = |done: bool, phase: Option<&str>| crate::core::body::Todo {
        done,
        text: "task".to_string(),
        phase: phase.map(str::to_string),
    };
    let detail = ProjectDetail {
        todos: vec![
            todo(true, Some("Shoot")),
            todo(true, Some("Shoot")),
            todo(false, Some("Cut")),
        ],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&detail), 30);
    let headings: Vec<(&str, usize, usize)> = rows
        .iter()
        .filter_map(|row| match row {
            PaneRow::Phase { name, done, total } => Some((name.as_str(), *done, *total)),
            _ => None,
        })
        .collect();
    assert_eq!(headings, vec![("Shoot", 2, 2), ("Cut", 0, 1)]);
    assert!(
        rows.iter()
            .all(|row| !matches!(row, PaneRow::Todo { phased: false, .. }))
    );

    let flat = ProjectDetail {
        todos: vec![todo(false, None)],
        ..Default::default()
    };
    let rows = pane_rows(&project(&[], "client"), Some(&flat), 30);
    assert!(
        rows.iter()
            .any(|row| matches!(row, PaneRow::Todo { phased: false, .. }))
    );
}
