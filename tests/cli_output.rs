//! The command surface: What a command says it did, and what it does without a terminal.
//!
//! Driven as a **real process** — see `common::mod`'s preamble for why.

mod common;

use common::{Sandbox, ids_in, shown_path};
use std::fs;

/// Break `config.toml` so every command has to decide what a config it cannot
/// read means.
fn corrupt_the_config(sb: &Sandbox) -> std::path::PathBuf {
    let path = sb.install.join("config.toml");
    let mut raw = fs::read_to_string(&path).expect("config.toml written by Sandbox::new");
    raw.push_str("\nthis is = not [valid toml\n");
    fs::write(&path, raw).unwrap();
    path
}

/// Assert that a headless run refused because there is no terminal, and named
/// the way to do it without one.
fn refuses_without_a_terminal(sb: &Sandbox, args: &[&str], escape: &str) {
    let err = sb.fails_headless(args);
    let cmd = args.join(" ");
    assert!(
        err.contains("no terminal"),
        "`fastf {cmd}` must say there is no terminal, not leak dialoguer's error:\n{err}"
    );
    assert!(
        err.contains(escape),
        "`fastf {cmd}` must name `{escape}` as the way through:\n{err}"
    );
}

/// `fastf notes` sliced the timestamp to 10 *bytes*, so a hand-edited
/// PROJECT_INFO.md with any multi-byte text where the timestamp goes panicked
/// mid-character. `hostile_fs.rs` promises corrupt metadata degrades, never
/// panics — it just never covered the journal body.
#[test]
fn notes_survives_a_hand_edited_journal_timestamp() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");
    let mut text = fs::read_to_string(&pinfo).unwrap();
    text.push_str("\n## Journal\n\n- 日本語のタイムスタンプ — hand-edited entry\n");
    fs::write(&pinfo, text).unwrap();

    let out = sb.run(&["notes", "ID0001"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "fastf notes panicked on a hand-edited timestamp:\n{stderr}"
    );
    assert!(out.status.success(), "fastf notes failed: {out:?}");
}

/// `note add` with no message passed the raw `editor` config field, so the
/// documented `$EDITOR` fallback never happened: an unconfigured install failed
/// with `launching editor ''`.
#[test]
fn note_add_falls_back_to_the_editor_env_var() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");

    // `true` exits 0 and writes nothing, so the note comes back empty — which
    // only happens if the editor was actually launched.
    let editor = if cfg!(windows) { "cmd" } else { "true" };
    let out = sb
        .command()
        .args(["note", "add", "ID0001"])
        .env("EDITOR", editor)
        .output()
        .expect("running fastf");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("launching editor ''"),
        "the $EDITOR fallback was skipped:\n{stderr}"
    );
}

/// `tag reauto` on a folder registered without a template failed with
/// "template '(registered)' not found", which reads like a broken install.
#[test]
fn tag_reauto_on_a_registered_project_explains_itself() {
    let sb = Sandbox::new();
    let folder = sb.base.join("adopted");
    fs::create_dir_all(&folder).unwrap();
    sb.ok(&["register", &folder.display().to_string(), "--yes"]);

    let err = sb.fails(&["tag", "reauto", "ID0001"]);
    assert!(
        !err.contains("not found"),
        "a registered project is not a missing template: {err}"
    );
    assert!(
        err.contains("without a template"),
        "the message must explain there is nothing to re-derive: {err}"
    );
}

/// `template from-folder --force` merged into the previous generation's
/// `files/`, so a template regenerated from a different folder still carried
/// the old files — and `files/` is what create copies, so they landed
/// in every new project.
#[test]
fn from_folder_force_replaces_the_bundled_files() {
    let sb = Sandbox::new();
    let src1 = sb.tmp.path().join("src1");
    let src2 = sb.tmp.path().join("src2");
    fs::create_dir_all(&src1).unwrap();
    fs::create_dir_all(&src2).unwrap();
    fs::write(src1.join("one.txt"), "one").unwrap();
    fs::write(src2.join("two.txt"), "two").unwrap();

    sb.ok(&[
        "template",
        "from-folder",
        &src1.display().to_string(),
        "gen",
    ]);
    sb.ok(&[
        "template",
        "from-folder",
        &src2.display().to_string(),
        "gen",
        "--force",
    ]);

    let files = sb.install.join("templates/gen/files");
    assert!(files.join("two.txt").exists(), "the new file must be there");
    assert!(
        !files.join("one.txt").exists(),
        "--force must replace the template, not merge into it"
    );
}

/// A `config.toml` that exists but does not parse used to be swallowed by
/// twenty `Config::load().unwrap_or_default()` calls, which silently changed
/// which directory is the library: `recent --plain` printed "No projects yet"
/// and exited 0 while the real projects sat in the configured base.
///
/// Falling back to defaults is not resilience when the fallback answers a
/// different question. Every command stops, names the file, and says how to
/// get out of it.
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

/// The cursor restore is guarded by `is_terminal` on each stream, because
/// `Term::show_cursor` emits its escape whatever it is writing to — an
/// unguarded call put a literal `\x1b[?25h` into the output a script reads.
/// Moving the restore into the interrupt path must not lose that guard.
#[test]
fn a_piped_failure_leaks_no_terminal_escapes() {
    let sb = Sandbox::new();
    corrupt_the_config(&sb);

    let out = sb.run(&["recent", "--plain"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        !stdout.contains("\x1b[?25h") && !stderr.contains("\x1b[?25h"),
        "a piped failure must not emit terminal escapes:\nstdout: {stdout:?}\nstderr: {stderr:?}"
    );
}

/// `fastf new` printed the same header for a preview and for the real thing:
/// "Preview · dry run — nothing will be created", immediately followed by the
/// project it had just created. A header that contradicts the command is worse
/// than no header.
#[test]
fn a_real_create_is_not_labelled_a_dry_run() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let committed = sb.ok(&["new", "race", "--name=Committed", "--yes"]);
    assert!(
        !committed.contains("nothing will be created"),
        "a create that creates must not claim otherwise:\n{committed}"
    );
    assert!(
        committed.contains("Preview"),
        "the plan is still shown before the commit:\n{committed}"
    );
    assert!(
        sb.base.join("R0001_Committed").is_dir(),
        "the project should exist: {:?}",
        ids_in(&sb.base)
    );

    let previewed = sb.ok(&["new", "race", "--name=Previewed", "--dry-run"]);
    assert!(
        previewed.contains("nothing will be created"),
        "a dry run must say so:\n{previewed}"
    );
    assert!(
        !sb.base.join("R0002_Previewed").exists(),
        "a dry run must write nothing"
    );
}

/// Same defect on the other printer: `apply` announced a dry run and then
/// applied the template.
#[test]
fn a_real_apply_is_not_labelled_a_dry_run() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let target = sb.tmp.path().join("existing");
    fs::create_dir_all(&target).unwrap();
    let target = target.display().to_string();

    let previewed = sb.ok(&["apply", "race", &target, "--name=X", "--dry-run"]);
    assert!(
        previewed.contains("nothing will be created"),
        "a dry run must say so:\n{previewed}"
    );

    let committed = sb.ok(&["apply", "race", &target, "--name=X", "--yes"]);
    assert!(
        !committed.contains("nothing will be created"),
        "an apply that applies must not claim otherwise:\n{committed}"
    );
    assert!(
        committed.contains("Preview") && committed.contains("Template applied"),
        "the plan is still shown before the commit:\n{committed}"
    );
}

#[test]
fn every_prompt_refuses_with_a_way_through() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let target = sb.tmp.path().join("existing");
    fs::create_dir_all(&target).unwrap();
    let target = target.display().to_string();
    let legacy = sb.base.join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    let legacy = legacy.display().to_string();

    // apply's confirmation
    refuses_without_a_terminal(&sb, &["apply", "race", &target, "--name=x"], "--yes");
    // register's rename confirmation
    refuses_without_a_terminal(&sb, &["register", &legacy, "--rename"], "--yes");
    // the template picker `fastf new` falls back to with no slug
    refuses_without_a_terminal(&sb, &["new"], "fastf new <slug>");
    // the interactive menu itself
    refuses_without_a_terminal(&sb, &[], "--help");
}

/// The menu prints a banner before it asks anything. Failing after the banner
/// puts decoration on stdout for a session that never existed.
#[test]
fn the_menu_refuses_before_it_draws_anything() {
    let sb = Sandbox::new();
    let out = sb.run_headless(&[]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "a menu that cannot run must not draw its banner: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `fastf move` skipped its confirmation when stdout was not a terminal and
/// moved the project anyway — the one prompt whose absence changes what happens
/// on disk.
#[test]
fn a_move_without_a_terminal_refuses_instead_of_moving() {
    let sb = Sandbox::new();
    let archive = sb.with_bases(&["archive"]).remove(0);
    let project = sb.plant_project(&sb.base, "proj", "ID0001");

    let err = sb.fails_headless(&["move", "ID0001", "archive"]);
    assert!(
        err.contains("no terminal") && err.contains("--yes"),
        "a move that cannot confirm must refuse and say how:\n{err}"
    );
    assert!(project.is_dir(), "the project must still be where it was");
    assert!(
        !archive.join("proj").exists(),
        "nothing may be moved by a confirmation that never happened"
    );

    // With --yes there is nothing to confirm, so it goes through.
    let out = sb.run_headless(&["move", "ID0001", "archive", "--yes"]);
    assert!(out.status.success(), "move --yes failed: {out:?}");
    assert!(archive.join("proj").is_dir(), "--yes must still move it");

    // No base and no terminal: the picker cannot run, and the usage line is the answer.
    let err = sb.fails_headless(&["move", "ID0001"]);
    assert!(
        err.contains("no terminal") && err.contains("fastf move"),
        "the base picker must refuse with the noninteractive form:\n{err}"
    );
}

/// `template from-folder --bundle-assets` confirms the total size with no way
/// to answer from a script: no `--yes` existed, so the command was unusable
/// noninteractively. `--dry-run` reports the same scan without writing.
#[test]
fn from_folder_can_be_driven_without_a_terminal() {
    let sb = Sandbox::new();
    let src = sb.tmp.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("notes.txt"), "hello {name}").unwrap();
    fs::write(src.join("blob.bin"), vec![0u8; 128 * 1024]).unwrap();
    let src = src.display().to_string();

    refuses_without_a_terminal(
        &sb,
        &["template", "from-folder", &src, "t1", "--bundle-assets"],
        "--yes",
    );

    let out = sb.run_headless(&["template", "from-folder", &src, "t2", "--dry-run"]);
    assert!(out.status.success(), "dry run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("nothing will be written"),
        "a dry run must say so:\n{stdout}"
    );
    assert!(
        stdout.contains("notes.txt") && stdout.contains("sub"),
        "the preview must show what it scanned:\n{stdout}"
    );
    assert!(
        !sb.install.join("templates/t2").exists(),
        "a dry run must write no template"
    );

    let out = sb.run_headless(&[
        "template",
        "from-folder",
        &src,
        "t3",
        "--bundle-assets",
        "--yes",
    ]);
    assert!(out.status.success(), "from-folder --yes failed: {out:?}");
    assert!(
        sb.install.join("templates/t3/files/blob.bin").is_file(),
        "--yes must accept the bundle prompt and copy the asset"
    );
}

/// A terminal is on stderr and stdin; stdout is the output. `fastf new t >
/// out.txt` refused to prompt because the guard probed the wrong stream.
#[cfg(unix)]
#[test]
fn a_redirected_stdout_still_has_a_terminal_to_prompt_on() {
    use common::pty;
    use std::time::Duration;

    let sb = Sandbox::new();
    sb.write_template("race");
    let captured = sb.tmp.path().join("out.txt");

    let (transcript, code) = pty::run_stdout_to(
        common::FASTF,
        &["new", "race", "--name=Redirected"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        // Confirm answers on the keypress itself — no Enter, or the newline
        // survives into the next prompt.
        &pty::Script::new().key("y").pause(400).key("n").build(),
        Duration::from_secs(20),
        &captured,
    );
    assert_eq!(
        code, 0,
        "a redirected stdout must not stop the prompt:\n{transcript}"
    );
    assert!(
        sb.base.join("R0001_Redirected").is_dir(),
        "the project should exist: {:?}",
        common::project_dirs(&sb.base)
    );
    let captured = fs::read_to_string(&captured).unwrap();
    assert!(
        captured.contains("R0001_Redirected"),
        "the redirected file is where the output went:\n{captured}"
    );
}

/// A template file whose name is not valid UTF-8 reaches the new project spelled
/// exactly as it was.
///
/// Unix only: a Windows filename is UTF-16 and cannot hold these bytes. The walk
/// used to describe every entry with `to_string_lossy`, so this file was opened
/// at a `?`-substituted path that does not exist — the copy failed naming a path
/// the user never wrote.
#[cfg(unix)]
#[test]
fn a_template_file_with_a_non_utf8_name_is_reproduced_byte_for_byte() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let sb = Sandbox::new();
    sb.write_template("race");
    let files = sb.install.join("templates/race/files");
    // 0xFF is not valid UTF-8 in any position.
    let hostile = OsStr::from_bytes(b"note\xff.txt");
    fs::write(files.join(hostile), b"payload").unwrap();

    sb.ok(&["new", "race", "--name=Solo", "--yes"]);

    let project = common::project_dirs(&sb.base)
        .into_iter()
        .next()
        .expect("a project was created");
    let landed = fs::read_dir(&project)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect::<Vec<_>>();
    assert!(
        landed.iter().any(|name| name.as_bytes() == b"note\xff.txt"),
        "the file should keep its exact bytes, got {landed:?}"
    );
    assert_eq!(
        fs::read(project.join(hostile)).unwrap(),
        b"payload",
        "and its contents"
    );
}

// ---------------------------------------------------------------------------
// The commands that had no process-level test at all
//
// `paths`, `reindex`, `reconcile`, `tag` and `template` were exercised only
// through the library functions underneath them. What a *command* prints and
// what exit code it gives is a separate contract — the one a script and a user
// both depend on.
// ---------------------------------------------------------------------------

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

/// `fastf tag` end to end as a process: add, list, remove.
#[test]
fn tag_add_list_and_remove_round_trip_through_the_command() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    sb.ok(&["tag", "add", "ID0001", "draft"]);
    let listed = sb.ok(&["tag", "list", "ID0001"]);
    assert!(
        listed.contains("draft"),
        "the tag should be listed:\n{listed}"
    );

    sb.ok(&["tag", "remove", "ID0001", "draft"]);
    let after = sb.ok(&["tag", "list", "ID0001"]);
    assert!(!after.contains("draft"), "and gone once removed:\n{after}");
}

/// `fastf template list | show | delete --yes` as a process.
#[test]
fn template_list_show_and_delete_work_from_the_command_line() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let listed = sb.ok(&["template", "list"]);
    assert!(
        listed.contains("race"),
        "the template should be listed:\n{listed}"
    );

    let shown = sb.ok(&["template", "show", "race"]);
    assert!(
        shown.contains("race") && shown.contains("Name"),
        "show should print the template's shape:\n{shown}"
    );

    sb.ok(&["template", "delete", "race", "--yes"]);
    assert!(
        !sb.install.join("templates/race").exists(),
        "delete --yes should remove the template directory"
    );
    let after = sb.ok(&["template", "list"]);
    assert!(!after.contains("race"), "and it should be gone:\n{after}");
}

// ---------------------------------------------------------------------------
// `fastf path` and `fastf copy`
// ---------------------------------------------------------------------------

/// `fastf path` exists to be substituted into another command
/// (`cd "$(fastf path api)"`), so its entire contract is one bare line. Not a
/// heading, not a colour, not a "→ Opening" — the path and a newline.
#[test]
fn path_prints_the_bare_path_and_nothing_else() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    let out = sb.run(&["path", "ID0001"]);
    assert!(out.status.success(), "fastf path failed: {out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{expected}\n"),
        "stdout must be the path and a newline, nothing else"
    );

    // The numeric tier reaches the same project.
    let numeric = sb.run(&["path", "1"]);
    assert_eq!(
        String::from_utf8_lossy(&numeric.stdout),
        format!("{expected}\n")
    );
}

/// There is no portable clipboard: Wayland and X11 disagree and a headless
/// session has neither. "No clipboard tool here" is an ordinary answer, so
/// `copy` prints the path instead and still exits 0 — a terminal selection is
/// then one drag away, which is more than a silent failure gives you.
#[test]
fn copy_without_any_clipboard_tool_prints_the_path_instead() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let expected = shown_path(&dir);

    let empty = sb.tmp.path().join("no-tools");
    fs::create_dir_all(&empty).unwrap();
    let out = sb
        .command()
        .args(["copy", "ID0001"])
        .env("PATH", &empty)
        .output()
        .expect("running fastf");

    assert!(
        out.status.success(),
        "a system without a clipboard tool is not an error: {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no clipboard tool found"),
        "copy must say what it did:\n{stdout}"
    );
    assert!(
        stdout.contains(&expected),
        "copy must fall back to printing the path:\n{stdout}"
    );
}

/// A resolved project may have come from the per-base cache, and a cache is a
/// file that travels with the projects — a synced folder or an unpacked archive
/// brings one along. Both verbs hand their answer to something else (a
/// clipboard, a shell substitution), so both check the folder first.
#[test]
fn path_and_copy_refuse_a_stale_project() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");

    // One successful run, so the cache exists and holds this project.
    assert!(sb.run(&["path", "ID0001"]).status.success());

    // The folder stays; its metadata does not. The cache only stat-checks the
    // directory, so the project still resolves — and must then be refused.
    fs::remove_file(dir.join("PROJECT_INFO.md")).unwrap();

    // The cache is consulted only while it is newer than its own base, and an
    // atomic write renames into that base — bumping the directory's mtime after
    // the file's. Whether the fast path is taken at all therefore comes down to
    // the filesystem's timestamp granularity, which is not what this test is
    // about. Re-stamp the cache so it is unambiguously the newer of the two.
    let cache = fs::OpenOptions::new()
        .write(true)
        .open(sb.base.join(".fastf-index.json"))
        .expect("the first run should have written a cache");
    cache
        .set_times(
            fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)),
        )
        .unwrap();

    let err = sb.fails(&["path", "ID0001"]);
    assert!(
        err.contains("ID0001") && err.contains("has no folder at"),
        "path must refuse a project whose metadata has gone:\n{err}"
    );
    let err = sb.fails(&["copy", "ID0001"]);
    assert!(
        err.contains("ID0001") && err.contains("cannot be copied at"),
        "copy must refuse a project whose metadata has gone:\n{err}"
    );
}

/// Piped, an ambiguous query is an error listing the candidates — the same text
/// `open` has always printed. A terminal gets a picker instead; a script must
/// not, and this pins the contract from the phase before the picker exists.
#[test]
fn an_ambiguous_copy_errors_with_candidates_when_piped() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "shared_one", "ID0011");
    sb.plant_project(&sb.base, "shared_two", "ID0012");

    for verb in ["copy", "path", "open", "term"] {
        let out = sb.run_headless(&[verb, "shared"]);
        assert!(
            !out.status.success(),
            "`fastf {verb} shared` must not pick one silently: {out:?}"
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("is ambiguous")
                && err.contains("Specify a full ID")
                && err.contains("ID0011")
                && err.contains("ID0012"),
            "`fastf {verb}` must list the candidates when piped:\n{err}"
        );
    }
}
