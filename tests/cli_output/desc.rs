//! `fastf desc` and `--description`: the one line that says what a project is,
//! written at creation, changed later, shown in every list, and refused when
//! it is not one line.

use super::*;

/// The round trip through the command: set, print, change, clear — and a
/// cleared description leaves the file as it was before one was written, so a
/// project nobody described and one whose description was removed read the
/// same.
#[test]
fn desc_set_print_and_clear_round_trip_through_the_command() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    let untouched = fs::read_to_string(dir.join("PROJECT_INFO.md")).unwrap();

    let none = sb.ok(&["desc", "ID0001"]);
    assert!(none.contains("(no description)"), "{none}");

    let set = sb.ok(&["desc", "ID0001", "A Fiverr music video for Ariana Grande"]);
    assert!(
        set.contains("Description set on") && set.contains("ID0001"),
        "{set}"
    );
    let printed = sb.ok(&["desc", "ID0001"]);
    assert_eq!(printed.trim(), "A Fiverr music video for Ariana Grande");
    let file = fs::read_to_string(dir.join("PROJECT_INFO.md")).unwrap();
    assert!(
        file.contains("description: A Fiverr music video for Ariana Grande\n"),
        "the line lands in the frontmatter as a plain scalar:\n{file}"
    );
    assert!(
        file.ends_with("## Notes\n"),
        "the body is not the frontmatter's to rewrite:\n{file}"
    );

    sb.ok(&["desc", "ID0001", "  Changed, with spaces around  "]);
    assert_eq!(
        sb.ok(&["desc", "ID0001"]).trim(),
        "Changed, with spaces around"
    );

    let cleared = sb.ok(&["desc", "ID0001", "--clear"]);
    assert!(cleared.contains("Description cleared on"), "{cleared}");
    assert_eq!(
        fs::read_to_string(dir.join("PROJECT_INFO.md")).unwrap(),
        untouched,
        "cleared, the file holds no trace of the key"
    );
}

/// A description is one line and no longer than the rule says; each refusal
/// names the rule, and nothing is written.
#[test]
fn desc_refuses_a_second_line_and_a_paragraph_by_name() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    let before = fs::read_to_string(dir.join("PROJECT_INFO.md")).unwrap();

    let err = sb.fails(&["desc", "ID0001", "first line\nsecond line"]);
    assert!(err.contains("one line"), "{err}");

    let long = "x".repeat(201);
    let err = sb.fails(&["desc", "ID0001", &long]);
    assert!(
        err.contains("at most 200 characters") && err.contains("got 201"),
        "{err}"
    );

    assert_eq!(
        fs::read_to_string(dir.join("PROJECT_INFO.md")).unwrap(),
        before,
        "a refused description writes nothing"
    );
}

/// `--description` on `new` lands in the file and in every list; the plain
/// list prints it on a row of its own between the project row and the path
/// row, so the `•` and `→` rows keep their shape.
#[test]
fn new_with_a_description_shows_it_in_every_list() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let created = sb.ok(&[
        "new",
        "race",
        "--name=One",
        "--description=Lookbook for the spring line",
        "--yes",
        "--no-preview",
    ]);
    assert!(
        created.contains("Lookbook for the spring line"),
        "{created}"
    );

    let plain = sb.ok(&["recent", "--plain"]);
    let lines: Vec<&str> = plain.lines().collect();
    let row = lines
        .iter()
        .position(|line| line.contains("•") && line.contains("R0001"))
        .expect("the project row");
    assert_eq!(
        lines[row + 1].trim(),
        "Lookbook for the spring line",
        "the description on its own row, under the project:\n{plain}"
    );
    assert!(
        lines[row + 2].trim_start().starts_with("→"),
        "then the path row:\n{plain}"
    );

    let listed = sb.ok(&["recent", "--json"]);
    let rows: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(rows[0]["description"], "Lookbook for the spring line");

    let shown = sb.ok(&["show", "R0001"]);
    assert!(shown.contains("Lookbook for the spring line"), "{shown}");

    // The search grammar knows the field, and a bare term reaches it.
    let found = sb.ok(&["search", "description=*spring*", "--plain"]);
    assert!(found.contains("R0001"), "{found}");
    let found = sb.ok(&["search", "lookbook", "--plain"]);
    assert!(found.contains("R0001"), "{found}");
    let missed = sb.ok(&["search", "description=*autumn*", "--plain"]);
    assert!(!missed.contains("R0001"), "{missed}");
}

/// `--description` on `register`, for one folder; a base of them shares no
/// description, and saying so is the refusal.
#[test]
fn register_takes_a_description_for_one_folder_and_refuses_it_for_a_base() {
    let sb = Sandbox::new();
    let folder = sb.base.join("adopted");
    fs::create_dir_all(&folder).unwrap();
    sb.ok(&[
        "register",
        &folder.display().to_string(),
        "--yes",
        "--description=An old shoot, adopted",
    ]);
    assert_eq!(sb.ok(&["desc", "ID0001"]).trim(), "An old shoot, adopted");

    // Before any undeclared token clap's own `conflicts_with` answers; after
    // one — a variable — only `RegisterFlags::validate` sees the flag, and it
    // is the authority. Both refuse.
    let base = sb.base.display().to_string();
    let err = sb.fails(&[
        "register",
        &base,
        "--recursive",
        "--description=One line for all",
    ]);
    assert!(err.contains("cannot be used with"), "{err}");
    let err = sb.fails(&[
        "register",
        &base,
        "--recursive",
        "--artist=X",
        "--description=One line for all",
    ]);
    assert!(
        err.contains("--description cannot be used with --recursive"),
        "{err}"
    );
}

/// A template with a variable called `description` keeps `--description` as
/// that variable: the flag filled it before the project had a line of its
/// own, and a script written then must go on working. The note says where
/// the value went and how the project's line is set.
#[test]
fn a_template_with_a_description_variable_keeps_the_flag_for_the_variable() {
    let sb = Sandbox::new();
    let dir = sb.install.join("templates").join("brief");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: Brief\nslug: brief\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: B\n  digits: 4\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n\
         \x20   transform: none\n  - slug: description\n    label: Description\n    type: text\n\
         \x20   transform: none\n",
    )
    .unwrap();

    let out = sb.run(&[
        "new",
        "brief",
        "--name=One",
        "--description=The variable's value",
        "--yes",
        "--no-preview",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("variable named 'description'") && stderr.contains("fastf desc"),
        "{stderr}"
    );
    let file =
        fs::read_to_string(common::project_dirs(&sb.base)[0].join("PROJECT_INFO.md")).unwrap();
    assert!(
        file.contains("  description: The variable's value\n"),
        "the value went to the variable:\n{file}"
    );
    assert!(
        !file.contains("\ndescription:"),
        "and the project's own line stayed empty:\n{file}"
    );
    assert!(sb.ok(&["desc", "B0001"]).contains("(no description)"));
}

/// A description somebody wrote by hand as a block scalar deserializes with
/// its newlines; every list prints its first line only, so the plain list
/// never gains a row that starts like a path.
#[test]
fn a_hand_written_multi_line_description_is_shown_as_one_line() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    let path = dir.join("PROJECT_INFO.md");
    let file = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        file.replace(
            "template_name: General\n",
            "template_name: General\ndescription: |\n  First line\n  → not a path\n",
        ),
    )
    .unwrap();
    sb.ok(&["reindex"]);

    let plain = sb.ok(&["recent", "--plain"]);
    assert!(plain.contains("First line"), "{plain}");
    assert!(!plain.contains("not a path"), "{plain}");
    assert_eq!(sb.ok(&["desc", "ID0001"]).trim(), "First line");
    let listed = sb.ok(&["recent", "--json"]);
    let rows: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(rows[0]["description"], "First line");
}
