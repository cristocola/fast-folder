//! `fastf desk` and `fastf peek`: the work across every project by whose
//! court the ball is in, and one project's glance.

use super::*;

/// Four projects, one ball each in a different court, so every heading
/// the desk can wear is on it.
fn desk_library() -> Sandbox {
    let sb = Sandbox::new();
    let todos = [
        (
            "ID0001",
            "Alpha",
            "- [x] shoot @sales added:2026-09-01T00:00:00Z done:2026-09-02T00:00:00Z\n- [ ] cut the first minute @claude added:2026-10-01T00:00:00Z\n",
        ),
        ("ID0002", "Beta", "- [ ] send the invoice\n"),
        ("ID0003", "Gamma", "- [x] everything\n"),
        (
            "ID0004",
            "Delta",
            "### Launch\n- [ ] call the label back @sales\n",
        ),
    ];
    for (id, name, list) in todos {
        let dir = sb.plant_project(&sb.base, name, id);
        let path = dir.join("PROJECT_INFO.md");
        let mut file = fs::read_to_string(&path).unwrap();
        file.push_str("\n## Todo\n\n");
        file.push_str(list);
        fs::write(&path, file).unwrap();
    }
    sb
}

fn headings(out: &str) -> Vec<String> {
    out.lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("    ") && line.contains(" · "))
        .map(|line| line.trim().split(" · ").next().unwrap().to_string())
        .collect()
}

/// Every court in the desk's order — unowned first with no hat worn, then
/// the owners by how many balls each holds, then nothing planned — and
/// yours first once a hat is worn; each project one line under its court.
#[test]
fn the_desk_groups_every_project_by_its_court() {
    let sb = desk_library();
    let out = sb.ok(&["desk", "--plain"]);
    assert_eq!(
        headings(&out),
        ["unowned", "with @claude", "with @sales", "nothing planned"],
        "{out}"
    );
    let rows: Vec<&str> = out.lines().filter(|l| l.starts_with("    ")).collect();
    assert_eq!(rows.len(), 4, "one line per project:\n{out}");
    assert!(
        rows[0].contains("ID0002") && rows[0].contains("send the invoice"),
        "{out}"
    );
    assert!(
        rows[1].contains("ID0001") && rows[1].contains("cut the first minute"),
        "{out}"
    );
    assert!(
        rows[2].contains("ID0004") && rows[2].contains("Launch · call the label back"),
        "{out}"
    );
    assert!(
        rows[3].contains("ID0003") && !rows[3].contains("everything"),
        "nothing planned has no ball: {out}"
    );
    assert!(
        rows[0].contains(" min") || rows[0].contains(" h"),
        "how long quiet: {out}"
    );

    let worn = sb.ok(&["desk", "--as", "@sales", "--plain"]);
    assert_eq!(
        headings(&worn),
        ["your court", "unowned", "with @claude", "nothing planned"],
        "{worn}"
    );
    sb.ok(&["hat", "claude"]);
    let worn = sb.ok(&["desk", "--plain"]);
    assert_eq!(
        headings(&worn)[0],
        "your court",
        "the configured hat: {worn}"
    );
}

/// The cuts: one court at a time, the quiet ones, a random pick, and the
/// refusals that name their rule.
#[test]
fn the_desk_cuts_to_one_court_and_picks_one() {
    let sb = desk_library();
    let claude = sb.ok(&["desk", "--owner", "claude", "--plain"]);
    assert_eq!(headings(&claude), ["with @claude"], "{claude}");
    assert!(
        claude.contains("ID0001") && !claude.contains("ID0002"),
        "{claude}"
    );
    let unowned = sb.ok(&["desk", "--unowned", "--plain"]);
    assert_eq!(headings(&unowned), ["unowned"], "{unowned}");
    let planned = sb.ok(&["desk", "--nothing-planned", "--plain"]);
    assert!(
        planned.contains("ID0003") && !planned.contains("ID0001"),
        "{planned}"
    );
    let mine = sb.ok(&["desk", "--as", "sales", "--mine", "--plain"]);
    assert_eq!(headings(&mine), ["your court"], "{mine}");
    assert!(mine.contains("ID0004"), "{mine}");

    let err = sb.fails(&["desk", "--mine"]);
    assert!(err.contains("needs a hat"), "{err}");
    let err = sb.fails(&["desk", "--owner", "a", "--unowned"]);
    assert!(err.contains("pass one"), "{err}");
    let err = sb.fails(&["desk", "--order", "bogus"]);
    assert!(err.contains("quiet, recent, name or id"), "{err}");

    // Nothing has been quiet for a decade; the random pick is one line.
    let quiet = sb.ok(&["desk", "--quiet", "3650", "--plain"]);
    assert!(quiet.contains("Nothing on the desk matches"), "{quiet}");
    let one = sb.ok(&["desk", "--random", "--owner", "claude", "--plain"]);
    let rows: Vec<&str> = one.lines().filter(|l| l.starts_with("    ")).collect();
    assert_eq!(rows.len(), 1, "{one}");
    assert!(rows[0].contains("ID0001"), "{one}");

    let out = sb.run(&["desk", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let alpha = rows.iter().find(|r| r["id"] == "ID0001").unwrap();
    assert_eq!(alpha["court"], "owner");
    assert_eq!(alpha["owner"], "claude");
    assert_eq!(alpha["ball"]["number"], 2);
    assert_eq!(alpha["ball"]["text"], "cut the first minute");
    assert_eq!(alpha["open"], 1);
    assert_eq!(alpha["done"], 1);
    assert!(alpha["quiet_since"].is_string() && alpha["quiet_for"].is_string());
    let gamma = rows.iter().find(|r| r["id"] == "ID0003").unwrap();
    assert_eq!(gamma["court"], "nothing-planned");
    assert!(gamma["ball"].is_null());
    let delta = rows.iter().find(|r| r["id"] == "ID0004").unwrap();
    assert_eq!(delta["ball"]["phase"], "Launch");
}

/// `peek` is the glance at one project: the ball and its court, how the
/// todos stand, who is in the folder, what last moved, the repository's
/// head when it is one, the last note, how long quiet.
#[test]
fn peek_is_the_glance_at_one_project() {
    let sb = desk_library();
    let dir = sb.base.join("Alpha");
    let git = dir.join(".git");
    fs::create_dir_all(git.join("logs")).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(
        git.join("logs/HEAD"),
        "0000000000000000000000000000000000000000 f0e1d2c3b4a5968778695a4b3c2d1e0f12345678 A Person <a@example.com> 1767312000 +0000\tcommit: Cut the first minute\n",
    )
    .unwrap();
    let path = dir.join("PROJECT_INFO.md");
    let mut file = fs::read_to_string(&path).unwrap();
    file = file.replace(
        "## Notes\n",
        "## Notes\n\n- 2026-10-02T10:00:00Z — treatment approved\n",
    );
    fs::write(&path, file).unwrap();
    // The record is an hour old and the note is written after it, so the
    // walk has one newest file whatever the filesystem's clock resolution.
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
        .unwrap();
    fs::write(dir.join("notes.txt"), "the newest file").unwrap();

    let out = sb.ok(&["peek", "ID0001"]);
    assert!(out.contains("ID0001") && out.contains("Alpha"), "{out}");
    let field = |label: &str| {
        out.lines()
            .find(|line| line.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("no `{label}` row:\n{out}"))
            .to_string()
    };
    assert!(
        field("ball").contains("cut the first minute") && field("ball").contains("with @claude"),
        "{out}"
    );
    assert!(
        field("ball").contains("todo 2"),
        "the number `todo done` takes: {out}"
    );
    assert!(
        field("todos").contains("1 open · 1 done") && field("todos").contains("with @claude 1"),
        "{out}"
    );
    assert!(
        field("working").contains("nobody") || field("working").contains("pid"),
        "{out}"
    );
    assert!(
        field("last moved").contains("notes.txt"),
        "the newest file outside the repository's own folder: {out}"
    );
    assert!(
        field("head").contains("main @ f0e1d2c") && field("head").contains("Cut the first minute"),
        "{out}"
    );
    assert!(field("last note").contains("treatment approved"), "{out}");
    assert!(field("quiet for").contains("since 20"), "{out}");

    // No repository, no head row; nothing planned says so.
    let gamma = sb.ok(&["peek", "ID0003"]);
    assert!(
        !gamma.lines().any(|l| l.trim_start().starts_with("head")),
        "{gamma}"
    );
    assert!(gamma.contains("nothing planned"), "{gamma}");
}
