//! `fastf todo`: listing, adding under a phase, ticking, rewording and
//! removing by number.

use super::*;

/// `text` without the stamps the writers put after a todo — ` added:<stamp>`
/// and ` done:<stamp>` — so the rest of a line can be checked whole.
fn unstamped(text: &str) -> String {
    let mut out = text.to_string();
    for key in [" added:", " done:"] {
        while let Some(at) = out.find(key) {
            let end = out[at + 1..]
                .find(|c: char| c.is_whitespace())
                .map_or(out.len(), |n| at + 1 + n);
            out.replace_range(at..end, "");
        }
    }
    out
}

/// `fastf todo` is the command line's half of the pane: the list numbered over
/// the tasks alone, a task added under the phase it names, and a tick that
/// refuses a number the list does not have.
#[test]
fn todo_lists_adds_under_a_phase_and_ticks_by_number() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");

    // An empty list says how to start one.
    let empty = sb.ok(&["todo", "list", "ID0001"]);
    assert!(empty.contains("no todos yet"), "{empty}");

    sb.ok(&["todo", "add", "ID0001", "read the order"]);
    sb.ok(&[
        "todo",
        "add",
        "ID0001",
        "download the files",
        "--phase",
        "Setup",
    ]);
    sb.ok(&[
        "todo",
        "add",
        "ID0001",
        "copy the audio",
        "--phase",
        "setup",
    ]);
    let body = fs::read_to_string(&pinfo).unwrap();
    assert!(
        unstamped(&body).contains("### Setup\n- [ ] download the files\n- [ ] copy the audio\n"),
        "a label matches whatever its case, and the task joins the end of its run:\n{body}"
    );
    assert!(
        body.contains("- [ ] read the order added:20"),
        "an add stamps its line: {body}"
    );

    // The numbers count tasks, so the label between them takes none.
    let listed = sb.ok(&["todo", "list", "ID0001"]);
    assert!(listed.contains("1.") && listed.contains("3."), "{listed}");
    assert!(listed.contains("Setup"), "the label is shown:\n{listed}");
    assert!(listed.contains("0/3 done"), "{listed}");

    let ticked = sb.ok(&["todo", "done", "ID0001", "2"]);
    assert!(ticked.contains("download the files"), "{ticked}");
    let body = fs::read_to_string(&pinfo).unwrap();
    assert!(
        body.contains("- [x] download the files added:20"),
        "the one byte changed: {body}"
    );
    assert!(
        body.contains("download the files added:20") && body.contains(" done:20"),
        "and the tick stamped when: {body}"
    );
    // Twice is not an error, and undo puts it back.
    assert!(
        sb.ok(&["todo", "done", "ID0001", "2"])
            .contains("already done")
    );
    assert!(
        sb.ok(&["todo", "done", "ID0001", "2", "--undo"])
            .contains("Reopened")
    );
    let body = fs::read_to_string(&pinfo).unwrap();
    assert!(
        body.contains("- [ ] download the files added:20") && !body.contains("done:"),
        "an untick takes the done stamp off and leaves the rest: {body}"
    );

    // Only the open ones, when that is what you want.
    sb.ok(&["todo", "done", "ID0001", "1"]);
    let open = sb.ok(&["todo", "list", "ID0001", "--open"]);
    assert!(!open.contains("read the order"), "{open}");

    let err = sb.fails(&["todo", "done", "ID0001", "9"]);
    assert!(
        err.contains("3 todos") && err.contains("todo list"),
        "{err}"
    );
    let err = sb.fails(&["todo", "add", "ID0001", "  "]);
    assert!(err.contains("empty"), "{err}");
}

/// The default list shows each phase's open tasks and folds its done ones
/// into one `✓ n done` line, and a number is still the task's place in the
/// file — so a folded task keeps its number, `--all` prints it, and `done`
/// acts on the number shown. An empty label is shown where `add --phase`
/// would put a task; `--open` has no fold lines; `--all` and `--open` are
/// two answers to one question.
#[test]
fn todo_list_folds_the_done_ones_and_keeps_the_file_numbers() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");
    let mut file = fs::read_to_string(&pinfo).unwrap();
    file.push_str(concat!(
        "\n## Todo\n\n",
        "- [x] read the brief\n",
        "- [ ] book the studio\n",
        "### Shoot\n",
        "- [x] shoot the band\n",
        "- [x] shoot the crowd\n",
        "### Edit\n",
        "- [x] sync the audio\n",
        "- [ ] cut the first minute\n",
        "### Grade\n",
    ));
    fs::write(&pinfo, file).unwrap();

    let listed = sb.ok(&["todo", "list", "ID0001"]);
    let lines: Vec<&str> = listed.lines().map(str::trim_end).collect();
    let at = |text: &str| {
        lines
            .iter()
            .position(|line| line.trim() == text)
            .unwrap_or_else(|| panic!("no line {text:?} in:\n{listed}"))
    };
    let book = at("2. [ ] book the studio");
    let shoot = at("Shoot");
    let edit = at("Edit");
    let cut = at("6. [ ] cut the first minute");
    let grade = at("Grade");
    assert_eq!(lines[book + 1].trim(), "✓ 1 done", "{listed}");
    assert_eq!(
        (shoot + 1, lines[shoot + 1].trim()),
        (shoot + 1, "✓ 2 done"),
        "a finished phase is its heading and its count:\n{listed}"
    );
    assert!(shoot < edit && edit < cut && cut < grade, "{listed}");
    assert_eq!(lines[cut + 1].trim(), "✓ 1 done", "{listed}");
    assert!(
        lines[book + 1].starts_with("     ✓"),
        "the count sits where the boxes are: {:?}",
        lines[book + 1]
    );
    for folded in ["read the brief", "shoot the band", "sync the audio"] {
        assert!(!listed.contains(folded), "{folded} is folded:\n{listed}");
    }
    assert!(listed.contains("4/6 done"), "{listed}");

    let open = sb.ok(&["todo", "list", "ID0001", "--open"]);
    assert!(
        open.contains("2. [ ] book the studio") && open.contains("6. [ ] cut the first minute"),
        "{open}"
    );
    assert!(
        !open.contains('✓') && !open.contains("Shoot") && !open.contains("Grade"),
        "only the open tasks, under the labels they have:\n{open}"
    );

    let all = sb.ok(&["todo", "list", "ID0001", "--all"]);
    for (number, line) in [
        "[x] read the brief",
        "[ ] book the studio",
        "[x] shoot the band",
        "[x] shoot the crowd",
        "[x] sync the audio",
        "[ ] cut the first minute",
    ]
    .iter()
    .enumerate()
    {
        let wanted = format!("{}. {line}", number + 1);
        assert!(all.contains(&wanted), "{wanted:?} in:\n{all}");
    }
    assert!(!all.contains('✓') && all.contains("Grade"), "{all}");

    let both = sb.run(&["todo", "list", "ID0001", "--all", "--open"]);
    assert!(!both.status.success(), "{both:?}");
    let err = String::from_utf8_lossy(&both.stderr);
    assert!(err.contains("--all") && err.contains("--open"), "{err}");

    // The number a folded todo had is the number it has.
    let reopened = sb.ok(&["todo", "done", "ID0001", "4", "--undo"]);
    assert!(reopened.contains("shoot the crowd"), "{reopened}");
    let listed = sb.ok(&["todo", "list", "ID0001"]);
    assert!(
        listed.contains("4. [ ] shoot the crowd") && listed.contains("✓ 1 done"),
        "{listed}"
    );
}

/// The trailer columns are measured over the todos shown and capped so an
/// aligned line fits: piped, a hundred columns. A long done todo the
/// default list folds pads nothing; shown with `--all`, and a long open
/// one always, keeps its owner and day two spaces after its text while
/// every other line stays inside the width.
#[test]
fn todo_list_aligns_the_trailer_over_what_it_shows_within_the_width() {
    use unicode_width::UnicodeWidthStr;
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");
    let long = "trim ".repeat(60);
    let long = long.trim_end();
    let mut file = fs::read_to_string(&pinfo).unwrap();
    file.push_str(&format!(
        "\n## Todo\n\n\
         - [x] {long} @sales added:2026-09-01T00:00:00Z done:2026-09-02T00:00:00Z\n\
         - [ ] cut @claude added:2026-10-01T00:00:00Z\n\
         - [ ] deliver added:2026-10-02T00:00:00Z\n"
    ));
    fs::write(&pinfo, &file).unwrap();

    let listed = sb.ok(&["todo", "list", "ID0001"]);
    for line in listed.lines() {
        assert!(line.width() <= 100, "{} columns: {line}", line.width());
    }
    let cut = listed.lines().find(|l| l.contains("cut")).unwrap();
    let deliver = listed.lines().find(|l| l.contains("deliver")).unwrap();
    assert_eq!(
        cut.find("2026-10-01"),
        deliver.find("2026-10-02"),
        "the days in one column:\n{listed}"
    );
    assert_eq!(
        cut.find("@claude"),
        Some(cut.find("cut").unwrap() + "deliver".len() + 2),
        "the owners two after the widest text shown, not the folded long one: {cut:?}"
    );

    let all = sb.ok(&["todo", "list", "ID0001", "--all"]);
    for line in all.lines().filter(|line| !line.contains(long)) {
        assert!(line.width() <= 100, "{} columns: {line}", line.width());
    }
    let row = all.lines().find(|l| l.contains(long)).unwrap();
    assert!(
        row.ends_with(&format!("{long}  @sales  2026-09-02")),
        "its trailer two spaces after it: {row:?}"
    );

    // Long and open, among todos with owners, itself without one.
    file = file.replace(
        &format!("- [x] {long} @sales added:2026-09-01T00:00:00Z done:2026-09-02T00:00:00Z"),
        &format!("- [ ] {long} added:2026-09-01T00:00:00Z"),
    );
    fs::write(&pinfo, &file).unwrap();
    let listed = sb.ok(&["todo", "list", "ID0001"]);
    for line in listed.lines().filter(|line| !line.contains(long)) {
        assert!(line.width() <= 100, "{} columns: {line}", line.width());
    }
    let row = listed.lines().find(|l| l.contains(long)).unwrap();
    assert!(
        row.ends_with(&format!("{long}  2026-09-01")),
        "no owner column's padding after a long text: {row:?}"
    );
}

/// `edit` and `remove` take the number `list` prints, as `done` does, and
/// refuse the same wrong numbers in the same words. A reworded todo keeps its
/// tick and its phase; an empty rewording is refused and pointed at
/// `remove`, because on the command line it is more likely an unset variable
/// than a wish.
#[test]
fn todo_edit_rewords_and_remove_removes_by_number() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");
    sb.ok(&["todo", "add", "ID0001", "read the order"]);
    for task in ["download the files", "copy the audio", "sort the clips"] {
        sb.ok(&["todo", "add", "ID0001", task, "--phase", "Setup"]);
    }
    sb.ok(&["todo", "done", "ID0001", "2"]);

    let reworded = sb.ok(&["todo", "edit", "ID0001", "2", "  fetch the files "]);
    assert!(
        reworded.contains("Reworded in ID0001: fetch the files"),
        "{reworded}"
    );
    let body = fs::read_to_string(&pinfo).unwrap();
    assert!(
        unstamped(&body).contains("### Setup\n- [x] fetch the files\n- [ ] copy the audio\n"),
        "the tick and the phase stay, and only the text moved:\n{body}"
    );
    assert!(
        body.contains("- [x] fetch the files added:20") && body.contains(" done:20"),
        "and the stamps stay where they were:\n{body}"
    );
    // The same text again writes nothing and says so.
    let same = sb.ok(&["todo", "edit", "ID0001", "2", "fetch the files"]);
    assert!(same.contains("already reads"), "{same}");

    let removed = sb.ok(&["todo", "remove", "ID0001", "3"]);
    assert!(
        removed.contains("Removed from ID0001: copy the audio"),
        "{removed}"
    );
    let removed = sb.ok(&["todo", "rm", "ID0001", "1"]);
    assert!(
        removed.contains("Removed from ID0001: read the order"),
        "`rm` is `remove`: {removed}"
    );

    let listed = sb.ok(&["todo", "list", "ID0001"]);
    assert!(
        listed.contains("2. [ ] sort the clips") && listed.contains("✓ 1 done"),
        "the done one is counted, the open one keeps its number:\n{listed}"
    );
    let listed = sb.ok(&["todo", "list", "ID0001", "--all"]);
    assert!(
        listed.contains("1. [x] fetch the files") && listed.contains("2. [ ] sort the clips"),
        "{listed}"
    );
    assert!(
        !listed.contains("copy the audio") && !listed.contains("read the order"),
        "{listed}"
    );
    assert!(listed.contains("1/2 done"), "{listed}");
    assert!(
        unstamped(&fs::read_to_string(&pinfo).unwrap())
            .contains("### Setup\n- [x] fetch the files\n- [ ] sort the clips\n"),
        "the label stays over what is left"
    );

    // Wrong numbers are refused by every verb in `done`'s words.
    let before = fs::read_to_string(&pinfo).unwrap();
    for verb in [
        &["todo", "done", "ID0001"][..],
        &["todo", "remove", "ID0001"][..],
        &["todo", "edit", "ID0001"][..],
    ] {
        let with = |number: &str| {
            let mut args = verb.to_vec();
            args.push(number);
            if verb[1] == "edit" {
                args.push("new text");
            }
            sb.fails(&args)
        };
        let err = with("0");
        assert!(err.contains("numbered from 1"), "{verb:?} 0: {err}");
        let err = with("3");
        assert!(
            err.contains("2 todos") && err.contains("todo list ID0001"),
            "{verb:?} 3: {err}"
        );
    }

    // An empty rewording is not a removal here: it says which verb is.
    let err = sb.fails(&["todo", "edit", "ID0001", "1", "   "]);
    assert!(
        err.contains("empty") && err.contains("fastf todo remove ID0001 1"),
        "{err}"
    );
    let err = sb.fails(&["todo", "edit", "ID0001", "1", "two\nlines"]);
    assert!(err.contains("one line"), "{err}");
    assert_eq!(
        fs::read_to_string(&pinfo).unwrap(),
        before,
        "nothing refused wrote anything"
    );
}

/// A todo's owner: `add` takes `--owner`, a trailing `@name` in the text, or
/// `--mine` under a hat; `owner` prints it, hands the todo over, or takes it
/// away with `--none`; `list` shows the owner and the day beside the text;
/// and an owner is one word, refused by name otherwise.
#[test]
fn todo_owner_is_set_by_add_and_owner_and_shown_by_list() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");

    let added = sb.ok(&[
        "todo",
        "add",
        "ID0001",
        "cut the first minute",
        "--owner",
        "claude",
    ]);
    assert!(added.contains("for @claude"), "{added}");
    let added = sb.ok(&["todo", "add", "ID0001", "approve the storyboard @acme"]);
    assert!(added.contains("for @acme"), "{added}");
    sb.ok(&["todo", "add", "ID0001", "deliver"]);
    let body = fs::read_to_string(&pinfo).unwrap();
    assert!(
        body.contains("- [ ] cut the first minute @claude added:20")
            && body.contains("- [ ] approve the storyboard @acme added:20")
            && body.contains("- [ ] deliver added:20"),
        "{body}"
    );

    // No hat, no `--mine`; with a hat set, it is the owner.
    let err = sb.fails(&["todo", "add", "ID0001", "call back", "--mine"]);
    assert!(
        err.contains("no hat is set") && err.contains("fastf hat"),
        "{err}"
    );
    let set = sb.ok(&["hat", "@sales"]);
    assert!(set.contains("hat = sales"), "{set}");
    assert!(sb.ok(&["hat"]).contains("@sales"));
    sb.ok(&["todo", "add", "ID0001", "call back", "--mine"]);
    assert!(
        fs::read_to_string(&pinfo)
            .unwrap()
            .contains("- [ ] call back @sales added:20")
    );
    assert!(sb.ok(&["config", "show"]).contains("@sales"));
    let off = sb.ok(&["hat", "--off"]);
    assert!(off.contains("(none)"), "{off}");
    assert!(sb.ok(&["hat"]).contains("no hat"));

    // The list shows the owner and the day, each in its column.
    let listed = sb.ok(&["todo", "list", "ID0001"]);
    let cut = listed
        .lines()
        .find(|line| line.contains("cut the first minute"))
        .unwrap();
    assert!(
        cut.contains("@claude") && cut.contains("  20"),
        "the owner and the day after the text: {cut}"
    );
    let deliver = listed
        .lines()
        .find(|line| line.contains("deliver"))
        .unwrap();
    assert!(
        !deliver.contains('@') && deliver.contains("  20"),
        "{deliver}"
    );

    // `owner` reads, hands over and takes away.
    assert!(sb.ok(&["todo", "owner", "ID0001", "1"]).contains("@claude"));
    assert!(sb.ok(&["todo", "owner", "ID0001", "3"]).contains("nobody"));
    let handed = sb.ok(&["todo", "owner", "ID0001", "3", "@me"]);
    assert!(handed.contains("@me's"), "{handed}");
    let body = fs::read_to_string(&pinfo).unwrap();
    assert!(body.contains("- [ ] deliver @me added:20"), "{body}");
    let taken = sb.ok(&["todo", "owner", "ID0001", "1", "--none"]);
    assert!(taken.contains("nobody's now"), "{taken}");
    let body = fs::read_to_string(&pinfo).unwrap();
    assert!(
        body.contains("- [ ] cut the first minute added:20"),
        "the owner went and the date stayed: {body}"
    );
    let err = sb.fails(&["todo", "owner", "ID0001", "1", "two words"]);
    assert!(err.contains("an owner is one word"), "{err}");
    let err = sb.fails(&["todo", "add", "ID0001", "x", "--owner", "a b"]);
    assert!(err.contains("an owner is one word"), "{err}");
    assert_eq!(
        fs::read_to_string(&pinfo).unwrap(),
        body,
        "a refusal wrote nothing"
    );

    // `show --json` carries the trailer, as added fields.
    let out = sb.run(&["show", "ID0001", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for (index, todo) in json["todos"].as_array().unwrap().iter().enumerate() {
        assert_eq!(todo["number"], index + 1, "the number `todo` takes: {todo}");
    }
    assert_eq!(json["todos"][2]["owner"], "me");
    assert!(
        json["todos"][2]["added"]
            .as_str()
            .unwrap()
            .starts_with("20")
    );
    assert!(json["todos"][2]["done_at"].is_null());
    assert!(json["todos"][0]["owner"].is_null());
}
