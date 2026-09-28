use super::*;

const HEAD: &str = "---\nid: ID0001\ntemplate: t\n---\n\n# Project Info\n\n";

fn doc(body: &str) -> String {
    format!("{HEAD}{body}")
}

fn dated(ts: &str, text: &str) -> Note {
    Note {
        timestamp: Some(ts.to_string()),
        text: text.to_string(),
    }
}

fn undated(text: &str) -> Note {
    Note {
        timestamp: None,
        text: text.to_string(),
    }
}

/// A temp file holding `content`, for the writers.
fn file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = crate::core::project_info::pinfo_path(dir.path());
    fs::write(&path, content).unwrap();
    (dir, path)
}

#[test]
fn a_heading_is_matched_where_it_starts_a_line_in_any_spelling() {
    for heading in [
        "## Notes",
        "## notes",
        "## NOTES:",
        "##Notes",
        "##  Notes  ",
    ] {
        let content = doc(&format!("{heading}\n\ntext\n"));
        let span = section_span(&content, Section::Notes).unwrap();
        assert_eq!(&content[span], &format!("{heading}\n\ntext\n"), "{heading}");
    }
    for not in [
        "### Notes",
        "# Notes",
        "Notes",
        "  ## Notes",
        "see ## Notes below",
    ] {
        let content = doc(&format!("{not}\n\ntext\n"));
        assert_eq!(section_span(&content, Section::Notes), None, "{not}");
    }
    for todo in [
        "## Todo", "## TODO:", "## Todos", "## To do", "## to-do", "## Tasks",
    ] {
        assert!(section_span(&doc(&format!("{todo}\n")), Section::Todo).is_some());
    }
}

#[test]
fn a_section_ends_at_the_next_heading_or_the_end_of_the_file() {
    let content = doc("## Notes\n\nfree\n\n## Archive\n\nkept\n");
    let span = section_span(&content, Section::Notes).unwrap();
    assert_eq!(&content[span.clone()], "## Notes\n\nfree\n");
    assert_eq!(&content[span.end..], "\n## Archive\n\nkept\n");
    assert_eq!(section_span(&content, Section::Todo), None);

    let content = doc("## Notes\n\nfree");
    let span = section_span(&content, Section::Notes).unwrap();
    assert_eq!(span.end, content.len());

    // A `###` inside the section is part of it.
    let content = doc("## Notes\n\n### day one\n\nfree\n");
    let span = section_span(&content, Section::Notes).unwrap();
    assert_eq!(&content[span], "## Notes\n\n### day one\n\nfree\n");

    // A heading in the frontmatter is a value, not a section.
    let content = "---\nid: ID0001\ntitle: '## Notes'\n---\n\nbody\n";
    assert_eq!(section_span(content, Section::Notes), None);
    // And a BOM does not shift the offsets.
    let bom = format!("\u{feff}{}", doc("## Notes\n\nfree\n"));
    let span = section_span(&bom, Section::Notes).unwrap();
    assert_eq!(&bom[span], "## Notes\n\nfree\n");
}

#[test]
fn every_shape_of_entry_is_read_and_every_line_under_it_belongs_to_it() {
    let content = doc("## Notes\n\n\
             - 2026-01-01T00:00:00Z — the bytes fastf writes\n  \
               with a second line\n\n  \
               and a third after a blank\n\
             - 2026-01-02 no separator at all\n\
             - 2026-01-03: a colon\n\
             * 2026-01-04 - a hyphen, on a star\n\
             - 2026-01-05T10:00:00Z — corrupted by the old writer\n\
             line two at column zero\n\
             - [ ] not an entry, so still line three\n\
             oh you — who edit my videos\n\n\n");
    let notes = notes_in(&content);
    assert_eq!(
        notes,
        vec![
            dated(
                "2026-01-01T00:00:00Z",
                "the bytes fastf writes\nwith a second line\n\nand a third after a blank"
            ),
            dated("2026-01-02", "no separator at all"),
            dated("2026-01-03", "a colon"),
            dated("2026-01-04", "a hyphen, on a star"),
            dated(
                "2026-01-05T10:00:00Z",
                "corrupted by the old writer\nline two at column zero\n- [ ] not an entry, so still line three\noh you — who edit my videos"
            ),
        ]
    );
}

#[test]
fn text_above_the_first_entry_is_one_undated_note_and_both_sections_are_read() {
    let content = doc(
        "## Notes\n\nremember the invoice\n- and a list\n\n- 2026-02-01 — under notes\n\n\
             ## Journal\n\n- 2026-01-01T00:00:00Z — under the journal\n",
    );
    assert_eq!(
        notes_in(&content),
        vec![
            undated("remember the invoice\n- and a list"),
            dated("2026-02-01", "under notes"),
            dated("2026-01-01T00:00:00Z", "under the journal"),
        ]
    );
    assert!(notes_in(&doc("## Notes\n\n")).is_empty());
    assert!(notes_in(&doc("# nothing\n")).is_empty());
    assert!(notes_in(&doc("## Notes\n\n\n\n")).is_empty());
    // A hand-edited timestamp is whatever was typed; nothing panics.
    let odd = doc("## Journal\n\n- 日本語のタイムスタンプ — hand-edited entry\n");
    assert_eq!(
        notes_in(&odd),
        vec![dated("日本語のタイムスタンプ", "hand-edited entry")]
    );
}

#[test]
fn a_dated_note_is_written_one_line_as_before_and_its_other_lines_indented() {
    assert_eq!(
        render_entry("2026-01-01T00:00:00Z", "one line"),
        "- 2026-01-01T00:00:00Z — one line\n"
    );
    assert_eq!(
        render_entry(
            "T",
            "first\nsecond  \n\n## not a heading\n- not an entry — really"
        ),
        "- T — first\n  second\n\n  ## not a heading\n  - not an entry — really\n"
    );
    // And what was written is what is read.
    let content = doc(&format!(
        "## Notes\n\n{}",
        render_entry(
            "T",
            "first\nsecond\n\n## not a heading\n- not an entry — really"
        )
    ));
    assert_eq!(
        notes_in(&content),
        vec![dated(
            "T",
            "first\nsecond\n\n## not a heading\n- not an entry — really"
        )]
    );
    assert_eq!(
        section_span(&content, Section::Notes).unwrap().end,
        content.len()
    );
}

#[test]
fn an_append_lands_at_the_end_of_the_section_under_a_blank_line() {
    // A fresh file: the shape `render_at` writes.
    let (_dir, path) = file(&doc("## Notes\n\n"));
    append_journal_entry(&path, "first").unwrap();
    append_journal_entry(&path, "second\nline two").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    let body = &after[HEAD.len()..];
    assert!(body.starts_with("## Notes\n\n- "), "{body:?}");
    assert!(body.contains(" — first\n- "), "{body:?}");
    assert!(body.ends_with(" — second\n  line two\n"), "{body:?}");
    assert!(!after.contains("## Journal"));
    assert_eq!(
        notes_in(&after)
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["first", "second\nline two"]
    );

    // A legacy file: the journal keeps the notes, byte for byte around
    // the new line, and the user's own section stays below.
    let legacy =
        doc("## Notes\n\n## Journal\n\n- 2026-01-01T00:00:00Z — a\n\n## Archive\n\nmine\n");
    let (_dir, path) = file(&legacy);
    append_journal_entry(&path, "b").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    let ts = notes_in(&after)[1].timestamp.clone().unwrap();
    assert_eq!(
        after,
        legacy.replace(
            "— a\n\n## Archive",
            &format!("— a\n- {ts} — b\n\n## Archive")
        )
    );

    // A section with prose and no entry yet gets a blank line first; a
    // heading with nothing under it likewise; a file with no section
    // gets one at the end.
    let (_dir, path) = file(&doc("## Notes\n\nremember\n"));
    append_journal_entry(&path, "c").unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("remember\n\n- ")
    );
    let (_dir, path) = file(&doc("## Notes\n## Archive\n"));
    append_journal_entry(&path, "d").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("## Notes\n\n- "), "{after:?}");
    assert!(after.contains(" — d\n\n## Archive\n"), "{after:?}");
    let (_dir, path) = file(&doc("# Project Info\n\nprose"));
    append_journal_entry(&path, "e").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("prose\n\n## Notes\n\n- "), "{after:?}");
    assert!(after.ends_with(" — e\n"), "{after:?}");
}

#[test]
fn a_note_is_rewritten_over_its_own_lines_or_taken_out() {
    let before = doc(
        "## Notes\n\nfree text\n\n- 2026-01-01 — one\n  more\n- 2026-01-02 — two\n\n## Archive\n\nmine\n",
    );
    let (_dir, path) = file(&before);
    replace_note(&path, 1, "one\nmore", "changed\nand longer\n\nstill").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        before.replace(
            "- 2026-01-01 — one\n  more\n",
            "- 2026-01-01 — changed\n  and longer\n\n  still\n"
        )
    );
    replace_note(&path, 0, "free text", "other text\nsecond").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("## Notes\n\nother text\nsecond\n\n- 2026-01-01"),
        "{after:?}"
    );
    // Removal takes the lines and the blank line it would have doubled.
    replace_note(&path, 1, "changed\nand longer\n\nstill", "").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("second\n\n- 2026-01-02 — two\n\n## Archive"),
        "{after:?}"
    );
    // The wrong ordinal, or a note that moved on, changes nothing.
    let err = replace_note(&path, 0, "free text", "x")
        .unwrap_err()
        .to_string();
    assert!(err.contains("changed meanwhile"), "{err}");
    let err = replace_note(&path, 9, "", "x").unwrap_err().to_string();
    assert!(err.contains("no longer there"), "{err}");
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
    // The undated note may not hold a heading.
    let err = replace_note(&path, 0, "other text\nsecond", "fine\n## Journal\nnot")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("## Journal") && err.contains("end the notes"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
}

#[test]
fn every_label_is_read_with_the_tasks_above_it_empty_ones_too() {
    let content = doc(concat!(
        "## Todo\n\n",
        "- [ ] loose\n\n",
        "### Grade\n\n",
        "### Setup\n",
        "- [ ] a\n",
        "- [x] b\n",
        "###\n",
        "### Other\n",
    ));
    let label = |name: &str, before| PhaseLabel {
        name: name.to_string(),
        before,
    };
    assert_eq!(
        phase_labels_in(&content),
        vec![label("Grade", 1), label("Setup", 1), label("Other", 3)],
        "an empty label is kept; a bare `###` is no label"
    );
    assert!(phase_labels_in(&doc("## Notes\n\n- a\n")).is_empty());
}

#[test]
fn a_phase_is_written_as_the_name_it_reads_back_as() {
    assert_eq!(phase_label("  Grade "), Some("Grade".to_string()));
    assert_eq!(phase_label("## Grade:"), Some("Grade".to_string()));
    assert_eq!(phase_label("### Main Edit"), Some("Main Edit".to_string()));
    assert_eq!(phase_label(" # : "), None);
    assert_eq!(phase_label(""), None);
    assert_eq!(phase_label("two\nlines"), None);
    // Whatever is typed, the label written reads back as the name given.
    for typed in [
        "Grade: :",
        "# # Grade",
        "Grade ::",
        "#Grade",
        "  ###  Grade  :  ",
    ] {
        let name = phase_label(typed).unwrap();
        assert_eq!(name, "Grade", "{typed:?}");
        assert_eq!(phase_name(&format!("### {name}")), Some(name.as_str()));
    }

    // Typed with its markdown, it still reads back as what was meant.
    let plain = doc("## Todo\n\n- [ ] read the order\n");
    let (_d, path) = file(&plain);
    add_todo_in(&path, "grade the reel", Some("## Grade:")).unwrap();
    let written = fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("\n### Grade\n- [ ] grade the reel\n"),
        "{written}"
    );
    let todos = todos_in(&written);
    assert_eq!(todos[1].phase.as_deref(), Some("Grade"));
}

#[test]
fn a_todo_joins_the_phase_it_names_or_opens_one_where_a_reader_would_look() {
    let grouped = doc(concat!(
        "## Todo\n\n",
        "- [x] read the order\n\n",
        "### Setup\n",
        "- [x] download\n\n",
        "### Other\n",
        "- [ ] chase the invoice\n",
    ));

    // An existing label: the end of its run, not the end of the section.
    let (_d, path) = file(&grouped);
    add_todo_in(&path, "copy the audio", Some("setup")).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        grouped.replace("- [x] download\n", "- [x] download\n- [ ] copy the audio\n")
    );

    // A new label goes above `### Other`, which is where a list keeps
    // what belongs to no phase.
    let (_d, path) = file(&grouped);
    add_todo_in(&path, "listen to the song", Some("Creative Plan")).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        grouped.replace(
            "### Other\n",
            "### Creative Plan\n- [ ] listen to the song\n\n### Other\n"
        )
    );

    // With no `### Other`, a new label opens at the end of the section.
    let plain = doc("## Todo\n\n- [ ] read the order\n\n## Something Else\n\nafter\n");
    let (_d, path) = file(&plain);
    add_todo_in(&path, "cut the first minute", Some("Main Edit")).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        plain.replace(
            "- [ ] read the order\n",
            "- [ ] read the order\n\n### Main Edit\n- [ ] cut the first minute\n"
        ),
        "the section after it is untouched"
    );

    // No section at all: the heading, the label and the task.
    let bare = doc("## Notes\n\n- 2026-01-01 — a\n");
    let (_d, path) = file(&bare);
    add_todo_in(&path, "first", Some("Setup")).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{bare}\n## Todo\n\n### Setup\n- [ ] first\n")
    );

    // And with no phase, the task goes at the end of the section.
    let (_d, path) = file(&grouped);
    add_todo_in(&path, "last", None).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        grouped.replace(
            "- [ ] chase the invoice\n",
            "- [ ] chase the invoice\n- [ ] last\n"
        )
    );
}

#[test]
fn a_task_takes_the_phase_label_above_it_and_the_ordinals_do_not_move() {
    let before = doc(concat!(
        "## Todo\n\n",
        "- [x] read the order\n", // before any label: no phase
        "### Main Edit\n",
        "- [ ] cut the first minute\n",
        "#### Grade:\n", // deeper, and a trailing colon
        "- [ ] match the cameras\n",
        "###\n", // an empty label leaves the one above standing
        "- [ ] export\n",
        "### Other\n", // a label with nothing under it
    ));
    let todos = todos_in(&before);
    let phases: Vec<Option<&str>> = todos.iter().map(|t| t.phase.as_deref()).collect();
    assert_eq!(
        phases,
        vec![None, Some("Main Edit"), Some("Grade"), Some("Grade")]
    );

    // A label is not a task, so it consumes no ordinal and a toggle still
    // names the same line it named before the phases were written.
    let (_dir, path) = file(&before);
    assert!(toggle_todo(&path, 3, "export").unwrap());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before.replace("[ ] export", "[x] export")
    );
}

#[test]
fn a_phase_label_belongs_to_its_own_section_only() {
    // A `###` line under the notes is prose there — the undated note the
    // notes grammar already makes of text above the first entry — and it
    // never reaches the todo reader.
    let before = doc("## Notes\n\n### not a phase\n\n- 2026-01-01 — a\n\n## Todo\n\n- [ ] plain\n");
    assert_eq!(todos_in(&before)[0].phase, None);
    let notes = notes_in(&before);
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].timestamp, None);
    assert_eq!(notes[0].text, "### not a phase");
}

#[test]
fn a_task_is_read_in_any_indent_and_toggled_by_one_byte() {
    let before = doc(
        "## Notes\n\n- 2026-01-01 — a\n\n## TODO:\n\nsome prose\n- [x] ingested\n  * [ ] edited\n- [] delivered\n- not a task\n- [y] nor this\n",
    );
    assert_eq!(
        todos_in(&before),
        vec![
            Todo {
                done: true,
                text: "ingested".to_string(),
                phase: None,
            },
            Todo {
                done: false,
                text: "edited".to_string(),
                phase: None,
            },
            Todo {
                done: false,
                text: "delivered".to_string(),
                phase: None,
            },
        ]
    );
    let (_dir, path) = file(&before);
    assert!(!toggle_todo(&path, 0, "ingested").unwrap());
    assert!(toggle_todo(&path, 1, "edited").unwrap());
    assert!(toggle_todo(&path, 2, "delivered").unwrap());
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        before
            .replace("[x] ingested", "[ ] ingested")
            .replace("[ ] edited", "[x] edited")
            .replace("[] delivered", "[x] delivered")
    );
    let err = toggle_todo(&path, 0, "edited").unwrap_err().to_string();
    assert!(err.contains("changed meanwhile"), "{err}");
    assert!(toggle_todo(&path, 9, "").is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
}

#[test]
fn a_todo_is_added_at_the_end_of_its_section_or_the_section_is_opened() {
    let (_dir, path) = file(&doc("## Notes\n\n- 2026-01-01 — a\n"));
    add_todo(&path, "  deliver the video ").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(
        after.ends_with("— a\n\n## Todo\n\n- [ ] deliver the video\n"),
        "{after:?}"
    );
    add_todo(&path, "invoice").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(
        after.ends_with("- [ ] deliver the video\n- [ ] invoice\n"),
        "{after:?}"
    );
    assert_eq!(todos_in(&after).len(), 2);

    // Mid-file, with the user's own section under it; and an empty
    // heading gets its blank line.
    let (_dir, path) = file(&doc("## Todo\n## Archive\n"));
    add_todo(&path, "one").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("## Todo\n\n- [ ] one\n\n## Archive\n"),
        "{after:?}"
    );

    for bad in ["", "  ", "two\nlines"] {
        assert!(add_todo(&path, bad).is_err(), "{bad:?}");
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
}

#[test]
fn replace_todo_keeps_indent_marker_and_state() {
    // Every shape `task_line` reads: both list markers, every bracket
    // state, a space indent and a tab one, and text with room around it.
    // A numbered item is not a task, so it is not here.
    let before = doc(concat!(
        "## Todo\n\n",
        "- [ ] plain\n",
        "* [x] starred and done\n",
        "  - [X] indented, capital\n",
        "- [] empty brackets\n",
        "\t* [ ]   spaced out  \n",
    ));
    let (_dir, path) = file(&before);
    replace_todo(&path, 0, "plain", "  reworded ").unwrap();
    replace_todo(&path, 1, "starred and done", "still done").unwrap();
    replace_todo(&path, 2, "indented, capital", "still indented").unwrap();
    replace_todo(&path, 3, "empty brackets", "still empty").unwrap();
    replace_todo(&path, 4, "spaced out", "tabbed").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        doc(concat!(
            "## Todo\n\n",
            "- [ ] reworded\n",
            "* [x] still done\n",
            "  - [X] still indented\n",
            "- [] still empty\n",
            "\t* [ ] tabbed\n",
        ))
    );
    let done: Vec<bool> = todos_in(&after).iter().map(|t| t.done).collect();
    assert_eq!(done, [false, true, true, false, false]);
}

#[test]
fn replace_todo_removes_the_line_without_a_doubled_blank() {
    let before = doc(concat!(
        "## Todo\n\n",
        "- [ ] first\n\n",
        "### Shoot\n",
        "- [ ] only one\n\n",
        "### Deliver\n",
        "- [x] a\n",
        "- [ ] b\n\n",
        "## Archive\n\nmine\n",
    ));
    let (_dir, path) = file(&before);

    // The first task, between the heading's blank line and the next one.
    replace_todo(&path, 0, "first", "").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before.replace("## Todo\n\n- [ ] first\n\n", "## Todo\n\n")
    );
    // The last of a phase: the label is the user's line and stays.
    replace_todo(&path, 0, "only one", "   ").unwrap();
    // The last of the section, above the user's own.
    replace_todo(&path, 1, "b", "").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        doc(concat!(
            "## Todo\n\n",
            "### Shoot\n\n",
            "### Deliver\n",
            "- [x] a\n\n",
            "## Archive\n\nmine\n",
        ))
    );

    // A last task with the next heading right under it: the section
    // stops short of that line's `\n`, and the removal still takes it.
    let tight = doc("## Todo\n- [ ] a\n- [ ] b\n## Archive\n");
    let (_dir, path) = file(&tight);
    replace_todo(&path, 1, "b", "").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        doc("## Todo\n- [ ] a\n## Archive\n")
    );
    // And the only task, at the end of a file with no final newline.
    let bare = doc("## Todo\n\n- [ ] a");
    let (_dir, path) = file(&bare);
    replace_todo(&path, 0, "a", "").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), doc("## Todo\n\n"));
}

#[test]
fn replace_todo_refuses_a_todo_changed_meanwhile() {
    let before = doc("## Todo\n\n- [ ] one\n- [x] two\n");
    let (_dir, path) = file(&before);
    for text in ["reworded", ""] {
        let err = replace_todo(&path, 0, "two", text).unwrap_err().to_string();
        assert!(err.contains("changed meanwhile"), "{err}");
        let err = replace_todo(&path, 2, "one", text).unwrap_err().to_string();
        assert!(err.contains("no longer there"), "{err}");
    }
    // The wording is the toggle's, so the two read as one refusal.
    let toggled = toggle_todo(&path, 0, "two").unwrap_err().to_string();
    let replaced = replace_todo(&path, 0, "two", "x").unwrap_err().to_string();
    assert_eq!(toggled, replaced);
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn replace_todo_keeps_crlf() {
    let before = doc(concat!(
        "## Todo\r\n\r\n",
        "- [ ] one\r\n",
        "  * [x] two\r\n\r\n",
        "- [ ] three\r\n\r\n",
        "## Archive\r\n",
        "- [ ] not a todo\r\n",
    ));
    let (_dir, path) = file(&before);
    replace_todo(&path, 0, "one", "uno").unwrap();
    replace_todo(&path, 1, "two", "dos").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        before
            .replace("- [ ] one\r\n", "- [ ] uno\r\n")
            .replace("  * [x] two\r\n", "  * [x] dos\r\n")
    );
    assert_eq!(todos_in(&after)[1].text, "dos", "no `\\r` in the text");

    // A removal takes the `\r\n`, and a blank `\r\n` line is a blank line.
    replace_todo(&path, 2, "three", "").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        after.replace("\r\n\r\n- [ ] three\r\n\r\n", "\r\n\r\n")
    );
    replace_todo(&path, 1, "dos", "").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        doc("## Todo\r\n\r\n- [ ] uno\r\n\r\n## Archive\r\n- [ ] not a todo\r\n")
    );

    // The last task right above the next heading keeps its `\r\n` too.
    let tight = doc("## Todo\r\n- [ ] a\r\n## Archive\r\n");
    let (_dir, path) = file(&tight);
    replace_todo(&path, 0, "a", "b").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        doc("## Todo\r\n- [ ] b\r\n## Archive\r\n")
    );
}

#[test]
fn replace_todo_leaves_phases_and_every_other_byte() {
    let before = "---\nid: ID0001\ntemplate: t\nobsidian_folder: Work/Clients\n---\n\n\
             # Project Info\n\n| Variable | Value |\n|---|---|\n| Artist | Aria |\n\n\
             ## Notes\n\n- 2026-01-01T00:00:00Z — a note\n  - [ ] not a todo\n\n\
             ## Todo\n\nsome prose first\n\n\
             - [x] read the order\n\n\
             ### Main Edit\n\
             - [ ] cut the first minute\n\
             - [ ] colour grade\n\
             #### Grade:\n\
             - [ ] match the cameras\n\n\
             ## Archive\n\n- [ ] not a todo either\n";
    let (_dir, path) = file(before);
    let phases_before: Vec<Option<String>> =
        todos_in(before).into_iter().map(|t| t.phase).collect();

    replace_todo(&path, 2, "colour grade", "grade the colour").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    let old = "- [ ] colour grade\n";
    let new = "- [ ] grade the colour\n";
    let at = before.find(old).unwrap();
    assert_eq!(after[..at], before[..at], "every byte above the line");
    assert_eq!(after[at..at + new.len()], *new);
    assert_eq!(
        after[at + new.len()..],
        before[at + old.len()..],
        "every byte below it"
    );
    let todos = todos_in(&after);
    assert_eq!(
        todos.iter().map(|t| t.phase.clone()).collect::<Vec<_>>(),
        phases_before,
        "every task keeps its phase and its number"
    );
    assert_eq!(todos[2].text, "grade the colour");
    assert_eq!(notes_in(&after), notes_in(before));
}

#[test]
fn replace_todo_refuses_two_lines() {
    let before = doc("## Todo\n\n- [ ] one\n");
    let (_dir, path) = file(&before);
    for bad in ["two\nlines", "two\r\nlines", "a\rb", "\n"] {
        let err = replace_todo(&path, 0, "one", bad).unwrap_err().to_string();
        assert_eq!(err, "a todo is one line", "{bad:?}");
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn add_todos_in_writes_several_in_order_under_a_phase() {
    let grouped = doc(concat!(
        "## Todo\n\n",
        "- [x] read the order\n\n",
        "### Setup\n",
        "- [x] download\n\n",
        "### Other\n",
        "- [ ] chase the invoice\n",
    ));
    let three = ["one".to_string(), "two".to_string(), "three".to_string()];
    let run = "- [ ] one\n- [ ] two\n- [ ] three\n";

    // An existing label: the end of its run, all three together.
    let (_d, path) = file(&grouped);
    add_todos_in(&path, &three, Some("setup")).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        grouped.replace("- [x] download\n", &format!("- [x] download\n{run}"))
    );
    // A new label, above `### Other`.
    let (_d, path) = file(&grouped);
    add_todos_in(&path, &three, Some("Shoot")).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        grouped.replace("### Other\n", &format!("### Shoot\n{run}\n### Other\n"))
    );
    // No phase: the end of the section.
    let (_d, path) = file(&grouped);
    add_todos_in(&path, &three, None).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{grouped}{run}")
    );
    // No section: the heading, the label and the run.
    let bare = doc("## Notes\n\n- 2026-01-01 — a\n");
    let (_d, path) = file(&bare);
    add_todos_in(&path, &three, Some("Setup")).unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after, format!("{bare}\n## Todo\n\n### Setup\n{run}"));
    assert_eq!(
        todos_in(&after)
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
}

#[test]
fn add_todos_in_skips_empty_and_refuses_a_newline() {
    let before = doc("## Todo\n\n- [ ] first\n");
    let (_dir, path) = file(&before);
    let texts = ["  a ", "", "   ", "b"].map(String::from);
    add_todos_in(&path, &texts, None).unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after, format!("{before}- [ ] a\n- [ ] b\n"));

    // One bad line refuses the lot: nothing is half-written.
    let err = add_todos_in(&path, &["fine".to_string(), "two\nlines".to_string()], None)
        .unwrap_err()
        .to_string();
    assert_eq!(err, "a todo is one line");
    for empty in [vec![], vec![String::new(), "  ".to_string()]] {
        let err = add_todos_in(&path, &empty, None).unwrap_err().to_string();
        assert_eq!(err, "the todo is empty — nothing written");
    }
    let err = add_todos_in(&path, &["a".to_string()], Some("two\nlines"))
        .unwrap_err()
        .to_string();
    assert_eq!(err, "a phase is one line");
    assert_eq!(fs::read_to_string(&path).unwrap(), after);
}

#[test]
fn add_todo_in_is_add_todos_in_with_one_text() {
    // `add_todo_in` is `add_todos_in` with one text: the same bytes
    // written, the same words refused with.
    let shapes = [
        doc("## Todo\n\n- [x] read the order\n\n### Setup\n- [x] download\n\n### Other\n"),
        doc("## Todo\n## Archive\n"),
        doc("## Notes\n\n- 2026-01-01 — a\n"),
        doc("# Project Info\n\nprose"),
    ];
    for shape in &shapes {
        for phase in [None, Some("setup"), Some("Main Edit")] {
            let (_a, one) = file(shape);
            let (_b, several) = file(shape);
            add_todo_in(&one, "  a task ", phase).unwrap();
            add_todos_in(&several, &["  a task ".to_string()], phase).unwrap();
            assert_eq!(
                fs::read_to_string(&one).unwrap(),
                fs::read_to_string(&several).unwrap(),
                "{shape:?} under {phase:?}"
            );
        }
    }
    let (_dir, path) = file(&shapes[1]);
    add_todo_in(&path, "one", None).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        doc("## Todo\n\n- [ ] one\n\n## Archive\n")
    );
    for (text, phase, refusal) in [
        ("two\nlines", None, "a todo is one line"),
        ("  ", None, "the todo is empty — nothing written"),
        ("a", Some("x\ny"), "a phase is one line"),
    ] {
        let err = add_todo_in(&path, text, phase).unwrap_err().to_string();
        assert_eq!(err, refusal);
    }
    let (_dir, bare) = file("# no frontmatter\n");
    let err = add_todo_in(&bare, "x", None).unwrap_err().to_string();
    assert!(err.ends_with("cannot add a todo"), "{err}");
}

#[test]
fn a_file_without_frontmatter_is_refused_by_every_writer() {
    let (_dir, path) = file("# no frontmatter\n\n## Notes\n\n- 2026-01-01 — a\n");
    for err in [
        append_journal_entry(&path, "x").unwrap_err(),
        replace_note(&path, 0, "a", "b").unwrap_err(),
        toggle_todo(&path, 0, "").unwrap_err(),
        replace_todo(&path, 0, "", "x").unwrap_err(),
        add_todo(&path, "x").unwrap_err(),
        add_todos_in(&path, &["x".to_string(), "y".to_string()], None).unwrap_err(),
    ] {
        assert!(err.to_string().contains("no YAML frontmatter"), "{err}");
    }
}

/// Add `texts` at `place` in a file holding `body`, and answer what the
/// writer said and what the file then reads as.
fn add_at(body: &str, texts: &[&str], place: TodoPlace) -> (usize, String) {
    let (_dir, path) = file(&doc(body));
    let texts: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
    let first = add_todos_at(&path, &texts, &place).unwrap();
    (first, fs::read_to_string(&path).unwrap())
}

fn ordinal_of(content: &str, text: &str) -> usize {
    todos_in(content)
        .iter()
        .position(|t| t.text == text)
        .unwrap_or_else(|| panic!("{text} is in the list:\n{content}"))
}

/// **The ordinal a writer answers is where the todo is.** Whatever the
/// shape of the list — a label named twice, in another case, empty at the
/// end, missing, an `### Other` — the number `add_todos_at` gives back is
/// the one the file reads the new todo at.
#[test]
fn the_ordinal_an_add_answers_is_where_the_file_has_it() {
    let cases: &[(&str, TodoPlace)] = &[
        ("## Todo\n\n- [ ] a\n- [ ] b\n", TodoPlace::End),
        (
            "## Todo\n\n### Shoot\n- [ ] a\n\n### Deliver\n- [ ] b\n\n### Shoot\n- [ ] c\n",
            TodoPlace::Phase("Shoot".to_string()),
        ),
        (
            "## Todo\n\n### Shoot\n- [ ] a\n\n### Deliver\n- [ ] b\n",
            TodoPlace::Phase("shoot".to_string()),
        ),
        (
            "## Todo\n\n### Shoot\n- [ ] a\n\n### shoot\n",
            TodoPlace::Phase("Shoot".to_string()),
        ),
        (
            "## Todo\n\n### Edit\n- [ ] a\n\n### Other\n- [ ] z\n",
            TodoPlace::Phase("Deliver".to_string()),
        ),
        (
            "## Todo\n\n### Edit\n- [ ] a\n",
            TodoPlace::Phase("Deliver".to_string()),
        ),
        ("# Notes only\n", TodoPlace::Phase("Deliver".to_string())),
        ("# Notes only\n", TodoPlace::End),
        (
            "## Todo\n\n- [ ] loose one\n- [ ] loose two\n\n### Edit\n- [ ] cut\n",
            TodoPlace::Loose,
        ),
        ("## Todo\n\n### Edit\n- [ ] cut\n", TodoPlace::Loose),
        ("## Todo\n\n- [ ] a\n", TodoPlace::Loose),
        ("## Todo\n\n- [ ] a\n## Notes\n\n- one\n", TodoPlace::End),
    ];
    for (body, place) in cases {
        let (first, content) = add_at(body, &["new one", "new two"], place.clone());
        assert_eq!(
            first,
            ordinal_of(&content, "new one"),
            "{place:?} in\n{body}\n→\n{content}"
        );
        assert_eq!(
            first + 1,
            ordinal_of(&content, "new two"),
            "{place:?}:\n{content}"
        );
    }
}

/// Loose tasks go with the loose tasks: after the last of them, never into
/// the label below; with none, above the first label.
#[test]
fn a_loose_todo_lands_with_the_loose_ones() {
    let (_, content) = add_at(
        "## Todo\n\n- [ ] loose\n\n### Edit\n- [ ] cut\n",
        &["another"],
        TodoPlace::Loose,
    );
    assert!(
        content.ends_with("## Todo\n\n- [ ] loose\n- [ ] another\n\n### Edit\n- [ ] cut\n"),
        "{content}"
    );
    let todos = todos_in(&content);
    assert_eq!(todos[1].phase, None, "not under Edit");

    let (_, content) = add_at(
        "## Todo\n\n### Edit\n- [ ] cut\n",
        &["first"],
        TodoPlace::Loose,
    );
    assert!(
        content.ends_with("## Todo\n\n- [ ] first\n\n### Edit\n- [ ] cut\n"),
        "{content}"
    );
}

/// A file saved with `\r\n` throughout keeps them on every line a writer
/// adds — a todo, a label, a note — and on a note it rewrites.
#[test]
fn a_crlf_file_keeps_its_line_endings_when_written_to() {
    let body = "## Notes\r\n\r\n- 2026-01-01T00:00:00Z — began\r\n\r\n## Todo\r\n\r\n- [ ] a\r\n";
    let content = doc(body).replace('\n', "\r\n").replace("\r\r\n", "\r\n");
    let (_dir, path) = file(&content);
    let crlf_only = |text: &str| text.matches('\n').count() == text.matches("\r\n").count();
    assert!(crlf_only(&content), "the fixture is CRLF throughout");

    add_todos_at(
        &path,
        &["b".to_string()],
        &TodoPlace::Phase("Deliver".to_string()),
    )
    .unwrap();
    append_journal_entry(&path, "a second note\nwith two lines").unwrap();
    let written = fs::read_to_string(&path).unwrap();
    assert!(
        crlf_only(&written),
        "every line is still CRLF:\n{written:?}"
    );
    let notes = notes_in(&written);
    replace_note(&path, 0, &notes[0].text, "began again\nover two lines").unwrap();
    let written = fs::read_to_string(&path).unwrap();
    assert!(crlf_only(&written), "and after a rewrite:\n{written:?}");
    assert_eq!(notes_in(&written)[0].text, "began again\nover two lines");
    assert_eq!(todos_in(&written).len(), 2);

    // A file that mixes the two keeps what it has and gains `\n` lines.
    let mixed = doc("## Todo\r\n\r\n- [ ] a\n");
    let (_dir, path) = file(&mixed);
    add_todos_at(&path, &["b".to_string()], &TodoPlace::End).unwrap();
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.ends_with("- [ ] a\n- [ ] b\n"), "{written:?}");
}
