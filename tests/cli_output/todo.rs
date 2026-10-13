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
        listed.contains("fetch the files") && listed.contains("sort the clips"),
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

/// A todo's court: `add` takes `--owner`, a trailing `@name` in the text, or
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

    // No hat, no `--mine`; a hat worn, and it is the owner.
    let err = sb.fails(&["todo", "add", "ID0001", "call back", "--mine"]);
    assert!(
        err.contains("no hat is worn") && err.contains("fastf hat"),
        "{err}"
    );
    let worn = sb.ok(&["hat", "@sales"]);
    assert!(worn.contains("hat = sales"), "{worn}");
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
