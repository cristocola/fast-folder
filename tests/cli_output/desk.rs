//! `fastf desk` and `fastf peek`: every project grouped by the owner of its
//! next open todo, and one project's summary.

use super::*;

/// Four projects, each with a different owner for its next todo, so
/// every heading the desk can show is on it.
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

/// Every group in the desk's order: no owner first when no hat is set,
/// then the owners by how many projects each has, then no open todos; and
/// yours first once a hat is set. Each project is one line under its group.
#[test]
fn the_desk_groups_every_project_by_its_assignee() {
    let sb = desk_library();
    let out = sb.ok(&["desk", "--plain"]);
    assert_eq!(
        headings(&out),
        ["no owner", "@claude", "@sales", "no open todos"],
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
        "a project with no open todo shows none: {out}"
    );
    assert!(
        rows[0].contains(" min") || rows[0].contains(" h"),
        "how long quiet: {out}"
    );

    let as_sales = sb.ok(&["desk", "--as", "@sales", "--plain"]);
    assert_eq!(
        headings(&as_sales),
        ["yours", "no owner", "@claude", "no open todos"],
        "{as_sales}"
    );
    sb.ok(&["hat", "claude"]);
    let configured = sb.ok(&["desk", "--plain"]);
    assert_eq!(
        headings(&configured)[0],
        "yours",
        "the configured hat: {configured}"
    );
}

/// The flags: one group at a time, the quiet projects, a random pick, and
/// the refusals that name their rule.
#[test]
fn the_desk_narrows_to_one_group_and_picks_one() {
    let sb = desk_library();
    let claude = sb.ok(&["desk", "--owner", "claude", "--plain"]);
    assert_eq!(headings(&claude), ["@claude"], "{claude}");
    assert!(
        claude.contains("ID0001") && !claude.contains("ID0002"),
        "{claude}"
    );
    let no_owner = sb.ok(&["desk", "--no-owner", "--plain"]);
    assert_eq!(headings(&no_owner), ["no owner"], "{no_owner}");
    let planned = sb.ok(&["desk", "--no-open-todos", "--plain"]);
    assert!(
        planned.contains("ID0003") && !planned.contains("ID0001"),
        "{planned}"
    );
    let mine = sb.ok(&["desk", "--as", "sales", "--mine", "--plain"]);
    assert_eq!(headings(&mine), ["yours"], "{mine}");
    assert!(mine.contains("ID0004"), "{mine}");

    let err = sb.fails(&["desk", "--mine"]);
    assert!(err.contains("needs a hat"), "{err}");
    let err = sb.fails(&["desk", "--owner", "a", "--no-owner"]);
    assert!(err.contains("pass one"), "{err}");
    let err = sb.fails(&["desk", "--order", "bogus"]);
    assert!(err.contains("quiet, recent, name or id"), "{err}");

    // Nothing has been quiet for a decade; the random pick is one line.
    let quiet = sb.ok(&["desk", "--quiet", "3650", "--plain"]);
    assert!(quiet.contains("No project matches"), "{quiet}");
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
    assert_eq!(alpha["group"], "owner");
    assert_eq!(alpha["owner"], "claude");
    assert_eq!(alpha["next_todo"]["number"], 2);
    assert_eq!(alpha["next_todo"]["text"], "cut the first minute");
    assert_eq!(alpha["open"], 1);
    assert_eq!(alpha["done"], 1);
    assert!(alpha["quiet_since"].is_string() && alpha["quiet_for"].is_string());
    let gamma = rows.iter().find(|r| r["id"] == "ID0003").unwrap();
    assert_eq!(gamma["group"], "no-open-todos");
    assert!(gamma["next_todo"].is_null());
    let delta = rows.iter().find(|r| r["id"] == "ID0004").unwrap();
    assert_eq!(delta["next_todo"]["phase"], "Launch");
}

/// One project in `base`, its `## Todo` holding `list`.
#[cfg(unix)]
fn plant_with_todos(sb: &Sandbox, base: &std::path::Path, name: &str, id: &str, list: &str) {
    let dir = sb.plant_project(base, name, id);
    let path = dir.join("PROJECT_INFO.md");
    let mut file = fs::read_to_string(&path).unwrap();
    file.push_str("\n## Todo\n\n");
    file.push_str(list);
    fs::write(&path, file).unwrap();
}

/// `state.toml` with these bases marked inactive, each a TOML string
/// whatever its path holds.
#[cfg(unix)]
fn mark_inactive(sb: &Sandbox, bases: &[&std::path::Path]) {
    let marks: Vec<String> = bases
        .iter()
        .map(|base| toml::Value::String(base.display().to_string()).to_string())
        .collect();
    fs::write(
        sb.install.join("state.toml"),
        format!(
            "bases_view = \"active\"\ninactive_bases = [{}]\n",
            marks.join(", ")
        ),
    )
    .unwrap();
}

/// **The bases marked inactive in the app stay off the desk**, and the list
/// says what it left out, counted after every other flag: `--every-base`
/// shows them, `--base` naming one shows that one, and `--json` keeps stdout
/// to the rows and says it on stderr. A base the configuration names
/// through a link is matched whether it was marked by that spelling or by
/// its real path, though discovery hands its projects the real path. A mark
/// on the default base, under any spelling, as in the app; a mark the
/// configuration no longer names; and a state file that will not parse hide
/// nothing.
#[cfg(unix)]
#[test]
fn the_desk_leaves_out_the_bases_marked_inactive() {
    let sb = Sandbox::new();
    let archive = sb.tmp.path().join("archive");
    let cold = sb.tmp.path().join("cold-disk");
    let cold_link = sb.tmp.path().join("cold");
    let home_link = sb.tmp.path().join("home-again");
    fs::create_dir_all(&archive).unwrap();
    fs::create_dir_all(&cold).unwrap();
    std::os::unix::fs::symlink(&cold, &cold_link).unwrap();
    std::os::unix::fs::symlink(&sb.base, &home_link).unwrap();
    sb.ok(&[
        "config",
        "set",
        "bases",
        &format!(
            "{},{},{}",
            archive.display(),
            cold_link.display(),
            home_link.display()
        ),
    ]);
    plant_with_todos(&sb, &sb.base, "Today", "ID0001", "- [ ] cut the trailer\n");
    plant_with_todos(
        &sb,
        &archive,
        "Old",
        "ID0002",
        "- [ ] uninstall the old backup tool @editor\n",
    );
    plant_with_todos(&sb, &cold, "Older", "ID0003", "- [x] everything\n");
    let gone = std::path::Path::new("/nowhere/gone");
    mark_inactive(&sb, &[&archive, &cold_link, gone]);

    let out = sb.ok(&["desk", "--plain"]);
    assert!(out.contains("ID0001"), "{out}");
    assert!(
        !out.contains("ID0002") && !out.contains("ID0003"),
        "the inactive bases are off the desk, the linked one too: {out}"
    );
    assert!(
        out.contains(
            "2 projects in 2 inactive bases left out (archive, cold-disk); --every-base shows them"
        ),
        "{out}"
    );

    let every = sb.ok(&["desk", "--plain", "--every-base"]);
    assert!(
        ["ID0001", "ID0002", "ID0003"]
            .iter()
            .all(|id| every.contains(id)),
        "{every}"
    );
    assert!(!every.contains("left out"), "{every}");

    let named = sb.ok(&["desk", "--plain", "--base", "archive"]);
    assert!(
        named.contains("ID0002") && !named.contains("ID0001"),
        "{named}"
    );
    assert!(
        !named.contains("left out"),
        "a base named is what was asked for: {named}"
    );

    // The count is what --every-base would add under the same flags.
    let theirs = sb.ok(&["desk", "--plain", "--owner", "editor"]);
    assert!(
        theirs.contains("No project matches those flags."),
        "{theirs}"
    );
    assert!(
        theirs.contains("1 project in 1 inactive base left out (archive); --every-base shows it"),
        "{theirs}"
    );
    let finished = sb.ok(&["desk", "--plain", "--no-open-todos"]);
    assert!(
        finished.contains("1 project in 1 inactive base left out (cold-disk)"),
        "{finished}"
    );
    let quiet = sb.ok(&["desk", "--plain", "--quiet", "3650"]);
    assert!(!quiet.contains("left out"), "{quiet}");

    let out = sb.run(&["desk", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1, "{rows}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("note: 2 projects in 2 inactive bases left out"),
        "{stderr}"
    );
    let out = sb.run(&["desk", "--json", "--every-base"]);
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 3, "{rows}");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("left out"));

    // Marked by its real path alone, the linked base is still inactive.
    mark_inactive(&sb, &[&archive, &cold]);
    let out = sb.ok(&["desk", "--plain"]);
    assert!(
        !out.contains("ID0003") && out.contains("2 projects in 2 inactive bases left out"),
        "{out}"
    );

    // The default base is never inactive, as in the app — not by its own
    // spelling, its real path, or another configured spelling of it: new
    // projects land there.
    let real_home = fs::canonicalize(&sb.base).unwrap();
    mark_inactive(
        &sb,
        &[&sb.base, &real_home, &home_link, &archive, &cold_link],
    );
    let out = sb.ok(&["desk", "--plain"]);
    assert!(out.contains("ID0001"), "the default base stays: {out}");
    assert!(
        out.contains("2 projects in 2 inactive bases left out"),
        "{out}"
    );

    // Every project inactive: the list says so rather than "no todo yet".
    fs::remove_dir_all(sb.base.join("Today")).unwrap();
    let none = sb.ok(&["desk", "--plain"]);
    assert!(
        none.contains("Every project is in a base marked inactive."),
        "{none}"
    );
    assert!(
        none.contains("2 projects in 2 inactive bases left out"),
        "{none}"
    );

    // A state file that will not parse hides nothing, and says why.
    fs::write(
        sb.install.join("state.toml"),
        "inactive_bases = [unterminated\n",
    )
    .unwrap();
    let out = sb.run(&["desk", "--plain"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ID0002") && stdout.contains("ID0003"),
        "{stdout}"
    );
    assert!(!stdout.contains("left out"), "{stdout}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("state.toml could not be read"),
        "{out:?}"
    );
}

/// `--json` answers JSON whatever the library holds: an empty one is `[]`,
/// never a sentence a parser chokes on.
#[test]
fn desk_json_on_an_empty_library_is_an_empty_array() {
    let sb = Sandbox::new();
    let out = sb.run(&["desk", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rows, serde_json::json!([]));
}

/// Piped, `desk --plain` fits a hundred columns: the todo column is capped
/// at what the id, the name and the age leave, a todo longer than that
/// keeps its age two spaces after it, and a name in wide characters is
/// padded by the columns it takes, not by its characters, so every todo
/// starts in one column.
#[test]
fn desk_plain_fits_the_width_and_a_long_todo_keeps_its_age_after_it() {
    use unicode_width::UnicodeWidthStr;
    let sb = Sandbox::new();
    let long = "trim ".repeat(60);
    let long = long.trim_end();
    let todos = [
        (
            "ID0001",
            "映画の編集",
            "- [ ] cut the first minute\n".to_string(),
        ),
        ("ID0002", "Beta", format!("- [ ] {long}\n")),
        ("ID0003", "Проекты", "- [ ] send the invoice\n".to_string()),
    ];
    for (id, name, list) in todos {
        let dir = sb.plant_project(&sb.base, name, id);
        let path = dir.join("PROJECT_INFO.md");
        let mut file = fs::read_to_string(&path).unwrap();
        file.push_str("\n## Todo\n\n");
        file.push_str(&list);
        fs::write(&path, file).unwrap();
    }
    let out = sb.ok(&["desk", "--plain"]);
    let rows: Vec<&str> = out.lines().filter(|l| l.starts_with("    ")).collect();
    assert_eq!(rows.len(), 3, "{out}");
    for row in &rows {
        if row.contains(long) {
            let after = &row[row.find(long).unwrap() + long.len()..];
            assert!(!after.trim().is_empty(), "{row:?}");
            assert_eq!(
                after,
                format!("  {:>6}", after.trim()),
                "the age two spaces after the long todo, in its six columns"
            );
        } else {
            assert!(row.width() <= 100, "{} columns: {row}", row.width());
        }
    }
    let todo_column = |row: &str, todo: &str| row[..row.find(todo).unwrap()].width();
    let short: Vec<usize> = rows
        .iter()
        .filter_map(|row| {
            ["cut the first minute", "send the invoice"]
                .iter()
                .find(|todo| row.contains(*todo))
                .map(|todo| todo_column(row, todo))
        })
        .collect();
    assert_eq!(short.len(), 2, "{out}");
    assert_eq!(
        short[0], short[1],
        "every todo starts in one column, whatever the names are written in:\n{out}"
    );
    assert_eq!(
        todo_column(rows.iter().find(|r| r.contains(long)).unwrap(), long),
        short[0],
        "{out}"
    );
}

/// `peek` prints one project's summary: the next todo and its owner, the
/// todo counts, which programs work in the folder, the newest file, the
/// repository's head when it is one, the last note, how long quiet.
#[test]
fn peek_prints_one_projects_summary() {
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
        field("next todo").contains("cut the first minute")
            && field("next todo").contains("@claude"),
        "{out}"
    );
    assert!(
        field("next todo").contains("todo 2"),
        "the number `todo done` takes: {out}"
    );
    assert!(
        field("todos").contains("1 open · 1 done") && field("todos").contains("@claude 1"),
        "{out}"
    );
    // Who is working in a folder is known on Linux alone; elsewhere the
    // row is absent rather than a `nobody` nothing stands behind.
    if cfg!(target_os = "linux") {
        assert!(
            field("working").contains("nobody") || field("working").contains("pid"),
            "{out}"
        );
    } else {
        assert!(
            !out.lines().any(|l| l.trim_start().starts_with("working")),
            "{out}"
        );
    }
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

    // No repository, no head row; no open todo says so.
    let gamma = sb.ok(&["peek", "ID0003"]);
    assert!(
        !gamma.lines().any(|l| l.trim_start().starts_with("head")),
        "{gamma}"
    );
    assert!(gamma.contains("no open todos"), "{gamma}");
}
