//! `fastf journal`: every project's dated notes as one timeline.

use super::*;
use fastf::core::library;
use std::path::Path;

/// Three projects, notes written out of order across them: the journal is
/// newest first whichever project a note belongs to, each note with its
/// project's id, and `-n` cuts the list and says so.
#[test]
fn the_journal_is_every_projects_notes_newest_first() {
    let sb = Sandbox::new();
    for (folder, id) in [("alpha", "ID0001"), ("beta", "ID0002"), ("gamma", "ID0003")] {
        sb.plant_project(&sb.base, folder, id);
    }
    // Notes planted with their own stamps, so the order under test is the
    // stamps', not the order they were written in.
    plant_note(
        &sb.base.join("alpha"),
        "2026-10-01T10:00:00Z",
        "alpha began",
    );
    plant_note(
        &sb.base.join("gamma"),
        "2026-10-03T09:00:00Z",
        "gamma: first cut\nthe chorus held",
    );
    plant_note(
        &sb.base.join("beta"),
        "2026-10-02T08:00:00Z",
        "beta: brief read",
    );
    plant_note(
        &sb.base.join("alpha"),
        "2026-10-04T07:00:00Z",
        "alpha delivered",
    );

    let out = sb.ok(&["journal"]);
    let firsts: Vec<&str> = out
        .lines()
        .filter(|line| line.trim_start().starts_with('•'))
        .collect();
    assert_eq!(firsts.len(), 4, "{out}");
    assert!(
        firsts[0].contains("ID0001") && firsts[0].contains("alpha delivered"),
        "{out}"
    );
    assert!(
        firsts[1].contains("ID0003") && firsts[1].contains("gamma: first cut"),
        "{out}"
    );
    assert!(firsts[2].contains("ID0002"), "{out}");
    assert!(firsts[3].contains("alpha began"), "{out}");
    assert!(
        out.contains("the chorus held"),
        "the rest of a note is under it: {out}"
    );
    assert!(out.contains("4 notes"), "{out}");

    let cut = sb.ok(&["journal", "-n", "2"]);
    assert!(
        cut.contains("alpha delivered") && !cut.contains("alpha began"),
        "{cut}"
    );
    assert!(cut.contains("2 of 4 notes"), "{cut}");
    let err = sb.fails(&["journal", "-n", "0"]);
    assert!(err.contains("--limit must be at least 1"), "{err}");
}

/// `--since` is the note's date, not the project's: a new note on an old
/// project is in this month's journal. `--grep` is a case-insensitive
/// substring, which is how a `lesson:` is read again.
#[test]
fn since_is_about_the_note_and_grep_finds_the_lesson() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "old", "ID0001"); // created 2026-01-01
    plant_note(
        &sb.base.join("old"),
        "2026-03-01T10:00:00Z",
        "an early note",
    );
    plant_note(
        &sb.base.join("old"),
        "2026-10-05T10:00:00Z",
        "Lesson: ask for the stems first",
    );

    let month = sb.ok(&["journal", "--since", "2026-10"]);
    assert!(
        month.contains("ask for the stems") && !month.contains("an early note"),
        "{month}"
    );
    let err = sb.fails(&["journal", "--since", "2026-6-1"]);
    assert!(err.contains("--since needs a date"), "{err}");

    let lessons = sb.ok(&["journal", "--grep", "lesson:"]);
    assert!(
        lessons.contains("ask for the stems") && !lessons.contains("an early note"),
        "{lessons}"
    );
    assert!(lessons.contains("1 note"), "{lessons}");
    let none = sb.ok(&["journal", "--grep", "venture:"]);
    assert!(none.contains("No notes match those filters"), "{none}");
}

/// `--template`, `--tag` and `--base` choose the projects, and a filter that
/// names what the library does not have is refused the way `recent` refuses
/// it, with the real answers listed.
#[test]
fn the_project_filters_scope_the_journal_and_are_refused_by_name() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["new", "race", "--name=One", "--yes", "--no-preview"]);
    sb.plant_project(&sb.base, "planted", "ID0009");
    sb.ok(&["note", "add", "R0001", "from the race"]);
    sb.ok(&["note", "add", "ID0009", "from the planted one"]);
    sb.ok(&["tag", "add", "ID0009", "client/Acme"]);

    let raced = sb.ok(&["journal", "--template", "race"]);
    assert!(
        raced.contains("from the race") && !raced.contains("planted one"),
        "{raced}"
    );
    let tagged = sb.ok(&["journal", "--tag", "client/Acme"]);
    assert!(
        tagged.contains("planted one") && !tagged.contains("from the race"),
        "{tagged}"
    );
    let base = library::base_label(&sb.base);
    let based = sb.ok(&["journal", "--base", &base]);
    assert!(
        based.contains("from the race") && based.contains("planted one"),
        "{based}"
    );

    let err = sb.fails(&["journal", "--template", "nope"]);
    assert!(
        err.contains("--template 'nope' is not a template") && err.contains("race"),
        "{err}"
    );
    let err = sb.fails(&["journal", "--base", "nowhere"]);
    assert!(
        err.contains("--base 'nowhere' is not a configured base"),
        "{err}"
    );
}

/// The JSON is an array, one object per note, with the project it belongs to;
/// an undated note and a line whose "timestamp" is not a date are not on the
/// timeline; a project whose file will not read is skipped, not fatal.
#[test]
fn json_rows_carry_the_project_and_undated_notes_stay_off_the_timeline() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let path = dir.join("PROJECT_INFO.md");
    let mut file = fs::read_to_string(&path).unwrap();
    file.push_str("free text above the first entry\n");
    file.push_str("- 2026-10-02T10:00:00Z — dated\n");
    file.push_str("- Remember — this is not a date\n");
    fs::write(&path, file).unwrap();
    let broken = sb.plant_project(&sb.base, "broken", "ID0002");
    fs::write(broken.join("PROJECT_INFO.md"), "---\nid: [\n---\n").unwrap();

    let out = sb.run(&["journal", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], "ID0001");
    assert_eq!(rows[0]["name"], "proj");
    assert_eq!(rows[0]["timestamp"], "2026-10-02T10:00:00Z");
    assert_eq!(rows[0]["text"], "dated");
    assert!(rows[0]["path"].is_string());
}

fn plant_note(dir: &Path, stamp: &str, text: &str) {
    let path = dir.join("PROJECT_INFO.md");
    let mut file = fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    file.push_str(&format!("- {stamp} — {}\n", lines.next().unwrap()));
    for line in lines {
        file.push_str(&format!("  {line}\n"));
    }
    fs::write(&path, file).unwrap();
}

/// `--todos` puts a todo's events on the timeline by their own dates — a
/// tick where it happened, and with `all` the add too — each row saying
/// which it is, and `--json` carrying the kind; without the flag the
/// journal is the notes it always was.
#[test]
fn todos_join_the_timeline_by_their_dates() {
    let sb = Sandbox::new();
    let alpha = sb.plant_project(&sb.base, "alpha", "ID0001");
    plant_note(&alpha, "2026-10-02T10:00:00Z", "alpha began");
    let path = alpha.join("PROJECT_INFO.md");
    let mut file = fs::read_to_string(&path).unwrap();
    file.push_str(concat!(
        "\n## Todo\n\n",
        "- [x] cut the first minute @claude added:2026-10-01T09:00:00Z done:2026-10-03T08:00:00Z\n",
        "- [ ] colour grade @claude added:2026-10-04T09:00:00Z\n",
        "- [x] ticked by hand\n",
    ));
    fs::write(&path, file).unwrap();

    let notes = sb.ok(&["journal"]);
    assert!(
        notes.contains("alpha began") && !notes.contains("cut the first minute"),
        "{notes}"
    );
    assert!(notes.contains("1 note"), "{notes}");

    let out = sb.ok(&["journal", "--todos"]);
    let rows: Vec<&str> = out
        .lines()
        .filter(|line| line.trim_start().starts_with('•'))
        .collect();
    assert_eq!(rows.len(), 2, "{out}");
    assert!(
        rows[0].contains("done") && rows[0].contains("cut the first minute @claude"),
        "the tick, newest, with who finished it: {out}"
    );
    assert!(rows[1].contains("alpha began"), "{out}");
    assert!(out.contains("2 entries"), "{out}");

    let all = sb.ok(&["journal", "--todos", "all"]);
    let rows: Vec<&str> = all
        .lines()
        .filter(|line| line.trim_start().starts_with('•'))
        .collect();
    assert_eq!(rows.len(), 4, "{all}");
    assert!(
        rows[0].contains("added") && rows[0].contains("colour grade"),
        "{all}"
    );
    assert!(rows[1].contains("done"), "{all}");
    assert!(rows[2].contains("alpha began"), "{all}");
    assert!(
        rows[3].contains("added") && rows[3].contains("cut the first minute"),
        "{all}"
    );

    let out = sb.run(&["journal", "--todos", "all", "--json"]);
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let kinds: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["added", "done", "note", "added"]);
    let since = sb.ok(&["journal", "--todos", "--since", "2026-10-03"]);
    assert!(
        since.contains("cut the first minute") && !since.contains("alpha began"),
        "{since}"
    );
}

/// A date written by hand has a day and no time. Its row keeps the columns
/// of a row stamped by fastf, so the ids and the texts stay in line.
#[test]
fn a_day_without_a_time_keeps_the_columns() {
    let sb = Sandbox::new();
    let alpha = sb.plant_project(&sb.base, "alpha", "ID0001");
    let path = alpha.join("PROJECT_INFO.md");
    let mut file = fs::read_to_string(&path).unwrap();
    file.push_str(concat!(
        "\n## Todo\n\n",
        "- [x] stamped by fastf done:2026-10-03T08:00:00Z\n",
        "- [x] dated by hand done:2026-10-05\n",
    ));
    fs::write(&path, file).unwrap();

    let out = sb.ok(&["journal", "--todos"]);
    let rows: Vec<&str> = out
        .lines()
        .filter(|line| line.trim_start().starts_with('•'))
        .collect();
    assert_eq!(rows.len(), 2, "{out}");
    assert!(rows[0].contains("dated by hand"), "{out}");
    let id_at = |row: &str| row.find("ID0001").unwrap_or_else(|| panic!("{out}"));
    assert_eq!(id_at(rows[0]), id_at(rows[1]), "{out}");
}
