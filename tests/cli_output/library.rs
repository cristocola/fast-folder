//! The data directory, the configuration and the library's own commands, as
//! processes.
//!
//! The library functions underneath them are tested on their own. What a
//! *command* prints and what exit code it gives is a separate contract — the
//! one a script and a user both depend on.

use super::*;

/// A `config.toml` that exists but does not parse stops every command, which
/// names the file and says how to get out of it.
///
/// Falling back to defaults is not resilience when the fallback answers a
/// different question: the config decides which directory is the library, so
/// `recent --plain` would print "No projects yet" and exit 0 while the real
/// projects sit in the configured base.
#[test]
fn a_corrupt_config_stops_every_command() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");
    let config_path = corrupt_the_config(&sb);
    let shown = config_path.display().to_string();

    for args in [
        vec!["recent", "--plain"],
        vec!["search", "proj", "--plain"],
        vec!["tag", "list", "ID0001"],
        vec!["notes", "ID0001"],
        vec!["reconcile"],
    ] {
        let out = sb.run(&args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let cmd = args.join(" ");
        assert!(
            !out.status.success(),
            "`fastf {cmd}` must fail on an unreadable config, got {out:?}"
        );
        assert!(
            stderr.contains(&shown) && stderr.contains("parsing"),
            "`fastf {cmd}` must name the file it could not parse:\n{stderr}"
        );
        assert!(
            stderr.contains("hint:") && stderr.contains("delete it"),
            "`fastf {cmd}` must say how to recover:\n{stderr}"
        );
        assert!(
            stdout.trim().is_empty(),
            "`fastf {cmd}` must not report anything about a library it could not read:\n{stdout}"
        );
    }
}

/// fastf's own bookkeeping stays off every surface a user reads.
///
/// `--relaunched` is what the relaunch puts on the rerun's command line, and
/// `main` takes it off again before clap sees it. A hidden clap argument is
/// not equivalent: `hide` keeps it out of `--help` and the man pages, but
/// **not** out of the generated completions, so `fastf --<TAB>` would offer
/// the user a flag that is none of their business.
#[test]
fn the_relaunch_flag_is_on_no_surface_a_user_reads() {
    let sb = Sandbox::new();

    for shell in ["bash", "zsh", "fish"] {
        let script = sb.ok(&["completions", shell]);
        assert!(
            !script.contains("relaunched"),
            "the {shell} completions offer it"
        );
    }
    assert!(
        !sb.ok(&["--help"]).contains("relaunched"),
        "--help names it"
    );

    let man = sb.tmp.path().join("man");
    sb.ok(&["mangen", &man.display().to_string()]);
    let page = std::fs::read_to_string(man.join("fastf.1")).expect("the man page");
    assert!(!page.contains("relaunched"), "the man page names it");

    // Anywhere but the one position the relaunch uses it is a word the user
    // typed, on every platform.
    let out = sb.run(&["recent", "--relaunched"]);
    assert!(
        !out.status.success(),
        "a word the user typed must not be eaten: {out:?}"
    );

    // And it still works where the relaunch puts it: first, ahead of the
    // subcommand. Unix only, because the relaunch is — Windows allocates a
    // console for a console application, so there is nothing there to mark and
    // the flag is an unknown argument like any other.
    #[cfg(unix)]
    {
        let out = sb.run(&["--relaunched", "recent", "--plain"]);
        assert!(out.status.success(), "{out:?}");
    }
}

/// `fastf paths` tells you where fastf keeps its things. Every path it prints
/// must be real, or the answer is worse than no answer.
#[test]
fn paths_reports_the_data_directory_it_is_actually_using() {
    let sb = Sandbox::new();
    let out = sb.ok(&["paths"]);

    assert!(
        out.contains(&sb.install.display().to_string()),
        "the data dir in use should be named:\n{out}"
    );
    assert!(
        out.contains("templates"),
        "and the templates directory:\n{out}"
    );
}

/// `fastf reindex` rescans every base and says how many projects it found. It is
/// the escape hatch for changes fastf could not observe, so it must report a
/// number rather than succeeding silently.
#[test]
fn reindex_rescans_and_reports_a_count() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    sb.plant_project(&sb.base, "2026-01-02_Beta_ID0002", "ID0002");

    let out = sb.ok(&["reindex"]);
    assert!(out.contains('2'), "expected a count of 2:\n{out}");
    assert!(
        sb.base.join(".fastf-index.json").exists(),
        "and a cache to show for it"
    );
}

/// `reindex` writes down the number behind each id for projects that predate
/// the field, and leaves alone the ones it cannot resolve.
///
/// This is the only repair path for the defect `Metadata::id_number` exists to
/// prevent — a lossy id rendering parsed back into a number that is far too
/// large, taken as the counter floor, and, because the counter never descends,
/// renumbering the whole library on the next create.
#[test]
fn reindex_writes_down_the_number_behind_an_id_it_can_resolve() {
    let sb = Sandbox::new();
    sb.write_template("race"); // prefix `R`, four digits

    let plant = |folder: &str, id: &str, template: &str| {
        let dir = sb.base.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("PROJECT_INFO.md"),
            format!(
                "---\nid: {id}\ntemplate: {template}\ntemplate_name: T\n\
                 created: 2026-01-01T00:00:00Z\nfolder: {folder}\npath: x\n\
                 variables: {{}}\ntags: []\n---\n"
            ),
        )
        .unwrap();
        dir
    };
    // Written before the field existed: no `id_number:` line anywhere.
    let known = plant("R0042_Known", "R0042", "race");
    // Its template is gone, so nothing can say where the prefix ends.
    let orphan = plant("X0007_Orphan", "X0007", "vanished");

    sb.ok(&["reindex"]);

    let known_meta = fs::read_to_string(known.join("PROJECT_INFO.md")).unwrap();
    assert!(
        known_meta.contains("id_number: 42"),
        "the number behind R0042 should be written down:\n{known_meta}"
    );
    let orphan_meta = fs::read_to_string(orphan.join("PROJECT_INFO.md")).unwrap();
    assert!(
        !orphan_meta.contains("id_number"),
        "a project whose template is gone is left alone, never guessed at:\n{orphan_meta}"
    );
}

/// A project fastf cannot read is named on stderr rather than quietly missing
/// from the count.
///
/// `reindex` is the command whose whole job is to look again, so "Reindexed 1
/// project" over a base holding two is the worst possible answer: it reports
/// the loss as a success. Driven as a process because the warning goes through
/// `util::diag` to stderr, which only a process has.
#[test]
fn reindex_names_a_project_it_cannot_read() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    let broken = sb.plant_project(&sb.base, "2026-01-02_Beta_ID0002", "ID0002");
    // Not UTF-8: what a Windows editor saving as the ANSI codepage leaves on a
    // drive both operating systems mount.
    fs::write(broken.join("PROJECT_INFO.md"), b"---\nid: ID\xe9002\n---\n").unwrap();

    let out = sb.run(&["reindex"]);
    assert!(out.status.success(), "a bad file is not a failed reindex");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Beta"),
        "the folder fastf could not read must be named:\n{stderr}"
    );
    assert!(
        stderr.contains("PROJECT_INFO.md"),
        "and the file, so the user knows what to open:\n{stderr}"
    );
}

/// A hand-edited `PROJECT_INFO.md` missing a field that is not the project's
/// identity keeps the project in the library.
///
/// `docs/projects.md` says "After creation the file is yours", so deleting one
/// `created:` line must not take the folder out of `recent`, `search` and the
/// app with nothing said anywhere.
#[test]
fn a_hand_edit_that_drops_created_does_not_drop_the_project() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    let path = dir.join("PROJECT_INFO.md");
    let kept: String = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with("created:"))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(&path, kept).unwrap();

    let out = sb.run(&["recent", "--plain"]);
    assert!(out.status.success());
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(
        listed.contains("ID0001"),
        "the project is still on disk and must still be listed:\n{listed}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "and nothing is wrong, so nothing should be said"
    );
}

/// The key `config set` takes is the key `config.toml` holds is the key
/// `config show` prints.
///
/// The Rust field is `recent_default_limit`, so it carries `serde(rename)`
/// plus an alias for the old spelling: `Config` has no `deny_unknown_fields`,
/// and a key hand-written under any other name is silently ignored.
#[test]
fn the_recent_limit_key_is_one_word_everywhere() {
    let sb = Sandbox::new();
    sb.ok(&["config", "set", "recent-limit", "50"]);

    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    assert!(
        config.contains("recent_limit = 50"),
        "the file must hold the name every surface shows:\n{config}"
    );

    // And a value written by hand under that name is read.
    let edited = config.replace("recent_limit = 50", "recent_limit = 7");
    fs::write(sb.install.join("config.toml"), edited).unwrap();
    let shown = sb.ok(&["config", "show"]);
    assert!(
        shown.contains("recent_limit:") && shown.contains('7'),
        "a hand-written value must be the one in force:\n{shown}"
    );

    // A config.toml an older fastf wrote, under the old key, still parses.
    let old_spelling = fs::read_to_string(sb.install.join("config.toml"))
        .unwrap()
        .replace("recent_limit = 7", "recent_default_limit = 9");
    fs::write(sb.install.join("config.toml"), old_spelling).unwrap();
    let shown = sb.ok(&["config", "show"]);
    assert!(
        shown.contains('9'),
        "the old key has to keep parsing:\n{shown}"
    );
}

/// **`mouse` is a retired key.** It was a setting in v3.6.0; the app never
/// takes the mouse now. `config set mouse` is accepted and says so rather than
/// failing a script that still sets it, a `config.toml` that still names it
/// parses, and `config show` no longer lists it.
#[test]
fn the_retired_mouse_key_is_accepted_and_ignored() {
    let sb = Sandbox::new();
    let out = sb.ok(&["config", "set", "mouse", "on"]);
    assert!(out.contains("mouse is no longer used"), "{out}");

    let config_path = sb.install.join("config.toml");
    let raw = fs::read_to_string(&config_path).unwrap();
    fs::write(&config_path, format!("mouse = \"on\"\n{raw}")).unwrap();
    let shown = sb.ok(&["config", "show"]);
    assert!(!shown.contains("mouse:"), "{shown}");
}

/// A recursive register that registered nothing is not a success: a script
/// reads the exit code, and the summary counts what was skipped.
///
/// Unix-only because a read-only directory is what makes the *write* fail while
/// the folder is still a target: the code path is platform-independent.
#[cfg(unix)]
#[test]
fn a_recursive_register_that_onboards_nothing_fails() {
    use std::os::unix::fs::PermissionsExt;

    let sb = Sandbox::new();
    let mut made = Vec::new();
    for name in ["Alpha", "Beta"] {
        let dir = sb.base.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        made.push(dir);
    }

    let base = sb.base.display().to_string();
    let out = sb.run(&["register", "--recursive", &base]);
    // Restore before any assertion, so a failure still leaves a removable
    // tempdir behind.
    for dir in &made {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
    }

    assert!(
        !out.status.success(),
        "registering nothing is not a success: {out:?}"
    );
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(
        listed.contains("skipped 2"),
        "the summary must count what it skipped:\n{listed}"
    );
}

/// `fastf reconcile` on a library with nothing outstanding says so and exits 0.
/// Reporting "nothing to do" is the common case and the one that must be quiet.
#[test]
fn reconcile_reports_a_clean_library() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    let out = sb.ok(&["reconcile"]);
    assert!(
        !out.contains(".fastf-transactions"),
        "a clean library has nothing outstanding to name:\n{out}"
    );
    assert!(
        out.contains("Reconcile") || out.contains("Nothing"),
        "and it should still say it looked:\n{out}"
    );
}

/// And with an interrupted move journal planted under the base, it finds it.
#[test]
fn reconcile_finds_an_interrupted_move() {
    let sb = Sandbox::new();
    let project = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    // A v2 move transaction left mid-copy: the state a `kill -9` during a
    // cross-drive move leaves behind.
    let txn = sb.base.join(".fastf-transactions").join("20260101-1-1");
    fs::create_dir_all(txn.join("staging")).unwrap();
    fs::write(
        txn.join("move.json"),
        format!(
            r#"{{"version":2,"operation_id":"20260101-1-1","project_id":"ID0001",
                "source_base":"{}","source_folder":"2026-01-01_Alpha_ID0001",
                "target_folder":"2026-01-01_Alpha_ID0001","phase":"Copying"}}"#,
            sb.base.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();

    let out = sb.ok(&["reconcile"]);
    assert!(
        out.contains(".fastf-transactions"),
        "the outstanding transaction should be named:\n{out}"
    );
    assert!(
        out.contains("Copying"),
        "and the state it was left in:\n{out}"
    );
    assert!(
        project.join("PROJECT_INFO.md").exists(),
        "and the project it belongs to left alone"
    );
}

/// `--json` and `fastf show`: the machine surface. An array for a list, one
/// object for a project, and never a picker whatever the terminal is.
#[test]
fn json_output_is_an_array_and_show_is_one_project_whole() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");
    sb.ok(&[
        "todo",
        "add",
        "ID0001",
        "cut the first minute",
        "--phase",
        "Main Edit",
    ]);
    sb.ok(&["note", "add", "ID0001", "began the edit"]);

    let listed = sb.ok(&["recent", "--json"]);
    let rows: serde_json::Value = serde_json::from_str(&listed).expect("recent --json is JSON");
    assert!(
        rows.is_array(),
        "a bare array, so `jq '.[].id'` works: {listed}"
    );
    assert_eq!(rows[0]["id"], "ID0001");
    assert!(rows[0]["path"].is_string() && rows[0]["base_label"].is_string());

    let searched = sb.ok(&["search", "proj", "--json"]);
    let found: serde_json::Value = serde_json::from_str(&searched).expect("search --json is JSON");
    assert_eq!(found[0]["id"], "ID0001");

    let shown = sb.ok(&["show", "ID0001", "--json"]);
    let one: serde_json::Value = serde_json::from_str(&shown).expect("show --json is JSON");
    assert_eq!(one["id"], "ID0001");
    assert_eq!(one["todos"][0]["text"], "cut the first minute");
    assert_eq!(
        one["todos"][0]["phase"], "Main Edit",
        "a todo carries its phase into the JSON: {shown}"
    );
    assert_eq!(one["notes"][0]["text"], "began the edit");

    // The summary names the project and counts what it has.
    let summary = sb.ok(&["show", "ID0001"]);
    assert!(
        summary.contains("ID0001") && summary.contains("1 note"),
        "{summary}"
    );

    // Two formats are one too many.
    let err = sb.fails(&["recent", "--json", "--plain"]);
    assert!(err.contains("cannot be used with"), "{err}");
}

/// **Every command's own help text sits at the margin.** The prose after the
/// options is a string in `main.rs`, and one written without `\` line
/// continuations prints every line after the first with the source file's
/// indent in front of it. Examples are indented two columns; only a hanging
/// line of an indented item goes further.
#[test]
fn every_help_text_sits_at_the_margin() {
    let sb = Sandbox::new();
    let commands = |args: &[&str]| -> Vec<String> {
        let mut help = args.to_vec();
        help.push("--help");
        let text = sb.ok(&help);
        text.lines()
            .skip_while(|line| *line != "Commands:")
            .skip(1)
            .take_while(|line| !line.is_empty())
            .filter_map(|line| line.split_whitespace().next().map(str::to_string))
            .filter(|name| name != "help")
            .collect()
    };
    let mut every: Vec<Vec<String>> = Vec::new();
    for command in commands(&[]) {
        let nested = commands(&[command.as_str()]);
        every.push(vec![command.clone()]);
        every.extend(nested.into_iter().map(|sub| vec![command.clone(), sub]));
    }
    assert!(every.len() > 20, "the walk found the commands: {every:?}");
    for path in every {
        let mut args: Vec<&str> = path.iter().map(String::as_str).collect();
        // `-h`: one line per option, so the first blank line after `Options:`
        // is where the command's own prose begins.
        args.push("-h");
        let text = sb.ok(&args);
        // What follows the options block is the command's own prose.
        let prose: Vec<&str> = text
            .lines()
            .skip_while(|line| *line != "Options:")
            .skip_while(|line| !line.is_empty())
            .collect();
        // A line may sit further in only to continue an item that is itself
        // indented — a numbered step, a table row. A missing `\` puts the
        // source's indent under a line at the margin.
        let indent = |line: &str| line.len() - line.trim_start().len();
        for pair in prose.windows(2) {
            let (above, line) = (pair[0], pair[1]);
            assert!(
                indent(line) < 3 || indent(above) >= 2,
                "`fastf {}` prints its help indented: {line:?}",
                path.join(" ")
            );
        }
    }
}

/// **Every verb works on a drive Windows cannot name.** On an rclone or other
/// WinFsp mount, `Path::canonicalize` fails for every path, and every mutation
/// canonicalizes its base, so a project there could be created and listed but
/// not changed — `resolving project base S:\: The volume does not contain a
/// recognized file system`. The `paths:unnamed-volume` failpoint sends every
/// canonicalization down the walking path `util::paths::canonical` falls back
/// to there, on any platform, so this is the whole library living on such a
/// drive. Debug-only, like every failpoint.
#[cfg(debug_assertions)]
#[test]
fn every_verb_works_where_windows_cannot_name_the_volume() {
    let sb = Sandbox::new();
    let cloud = sb.with_bases(&["cloud"]).remove(0);
    sb.plant_project(&sb.base, "proj", "ID0001");
    let walking = |args: &[&str]| {
        let out = sb
            .command()
            .args(args)
            .env("FASTF_FAULT", "paths:unnamed-volume")
            .output()
            .expect("running fastf");
        assert!(
            out.status.success(),
            "`fastf {}` failed where the volume has no name:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    };

    walking(&["todo", "add", "ID0001", "check the render"]);
    walking(&["tag", "add", "ID0001", "client"]);
    walking(&["note", "add", "ID0001", "first note"]);
    walking(&["rename", "ID0001", "renamed", "--yes"]);
    assert!(sb.base.join("renamed").is_dir(), "the rename must land");
    walking(&["move", "ID0001", &cloud.display().to_string(), "--yes"]);
    let moved = fs::read_to_string(cloud.join("renamed").join("PROJECT_INFO.md"))
        .expect("the move must land in the other base");
    assert!(moved.contains("check the render") && moved.contains("first note"));
    walking(&["delete", "ID0001", "--yes"]);
    assert!(!cloud.join("renamed").exists(), "the delete must land");
}
