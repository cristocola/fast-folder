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

/// Windows Notepad saves by reopening the file for writing with
/// `FILE_SHARE_READ` alone, and `note add` kept its own write handle on the
/// scratch file open for as long as the editor ran — a sharing violation on
/// every save. Notepad reported it as "cannot create the file" and fell back
/// to a Save As dialog opened in fastf's working directory, which from the
/// Start Menu shortcut is the install folder under `Program Files`, where the
/// second attempt was refused too. The note never reached the journal.
#[cfg(windows)]
#[test]
fn note_add_survives_an_editor_that_saves_like_notepad() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");

    // The editor string is split on whitespace, so the script's path may not
    // hold any; a temp dir that does is a limit of this harness, not a defect.
    let script = sb.tmp.path().join("notepad-like.ps1");
    if script.display().to_string().contains(' ') {
        eprintln!("skipping: the temp dir path contains a space");
        return;
    }
    // Exactly Notepad's open: create-or-truncate, write access, readers only.
    fs::write(
        &script,
        "$path = $args[0]\n\
         try {\n\
         $stream = [System.IO.File]::Open($path, 'Create', 'Write', 'Read')\n\
         } catch {\n\
         [Console]::Error.WriteLine('sharing violation: ' + $_.Exception.Message)\n\
         exit 1\n\
         }\n\
         $writer = New-Object System.IO.StreamWriter($stream)\n\
         $writer.WriteLine('written by the editor')\n\
         $writer.Dispose()\n\
         exit 0\n",
    )
    .unwrap();
    let editor = format!(
        "powershell -NoProfile -ExecutionPolicy Bypass -File {}",
        script.display()
    );

    let out = sb
        .command()
        .args(["note", "add", "ID0001"])
        .env("EDITOR", &editor)
        .output()
        .expect("running fastf");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "an editor that saves the way Notepad does must be able to save:\n{stderr}"
    );
    let pinfo = fs::read_to_string(dir.join("PROJECT_INFO.md")).unwrap();
    assert!(
        pinfo.contains("written by the editor"),
        "the editor's text never reached the journal:\n{pinfo}"
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

/// fastf's own bookkeeping stays off every surface a user reads.
///
/// `--relaunched` is what the relaunch puts on the rerun's command line, and
/// `main` takes it off again before clap sees it. Declaring it as a clap
/// argument instead looked equivalent: `hide` kept it out of `--help` and the
/// man pages, but **not** out of the generated completions, so `fastf --<TAB>`
/// offered the user a flag that is none of their business.
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
/// renumbering the whole library on the next create. It had no test at all.
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
/// `docs/projects.md` says "After creation the file is yours", and deleting one
/// `created:` line used to take the folder out of `recent`, `search` and the
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
/// They were three different words for one setting: the Rust field is
/// `recent_default_limit` and carried no `serde(rename)`, so `config set
/// recent-limit 50` printed `Set recent_limit = 50` and wrote
/// `recent_default_limit = 50`. Somebody following the docs and hand-writing
/// `recent_limit = 50` into the file got it silently ignored — `Config` has no
/// `deny_unknown_fields` — and fell back to the default.
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

    // Every config.toml written before this still parses.
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

/// A recursive register that registered nothing is not a success.
///
/// Each failure was an `eprintln!` on stderr and the tail printed
/// `✓ Registered 0 folders.` and returned `Ok(())` regardless, so a script saw
/// a clean exit for a run that onboarded nothing.
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

/// An editor that failed did not open the project, whatever it exited for.
///
/// Both `spawn_editor` arms dropped the child's `ExitStatus` and propagated
/// only the *spawn* error, so `✓ opened in <editor>` was printed
/// unconditionally — and on Windows `cmd /c start` succeeds for an editor that
/// does not exist, so a typo in the `editor` key reported success over nothing
/// at all. `git_init` and a template's `commands`, in the same function, have
/// always checked.
///
/// Unix-only for the fixture: `false` is coreutils' one-line "exit 1", always
/// present, and `common::recorder` hard-codes `exit 0` so it cannot say this.
#[cfg(unix)]
#[test]
fn an_editor_that_failed_is_not_reported_as_having_opened_anything() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["config", "set", "post_create.open_in_editor", "true"]);
    sb.ok(&["config", "set", "editor", "false"]);

    let out = sb.run(&["new", "race", "--name=One", "--yes", "--no-preview"]);
    assert!(
        out.status.success(),
        "the project is still created: {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stdout.contains("opened in") && !stderr.contains("opened in"),
        "nothing opened, so nothing may say it did:\n{stdout}{stderr}"
    );
    assert!(
        stdout.contains("could not open editor") || stderr.contains("could not open editor"),
        "and the failure is worth a word:\n{stdout}{stderr}"
    );
}

/// The first-run banner is on stderr, so `$(fastf path …)` is a path.
///
/// `ensure_bootstrapped` runs for every command but `completions` and
/// `mangen`, and printed two lines with `println!`. On a machine whose data
/// directory does not exist yet but whose base already holds projects — a
/// second computer, a portable base, a scripted `FASTF_INSTALL_DIR` —
/// `cd "$(fastf path lullaby)"` got `fastf: initialized in …` prepended to the
/// path. `docs/cli.md` states the contract: "prints the path followed by a
/// newline — no colour, no decoration, nothing else on stdout".
#[test]
fn the_first_run_banner_never_lands_in_a_command_substitution() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");

    // Throw the data directory away, keeping the base and its projects: the
    // next command bootstraps from scratch.
    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    fs::remove_dir_all(&sb.install).unwrap();
    fs::create_dir_all(&sb.install).unwrap();
    fs::write(sb.install.join("config.toml"), config).unwrap();

    let out = sb.run(&["path", "ID0001"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end(),
        shown_path(&dir),
        "stdout is the path and nothing else:\n{stdout}"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("initialized in"),
        "and the banner is still said, on the stream for saying things"
    );
}

/// `tag reauto` re-derives the template's own tags and leaves the free-form
/// ones alone.
///
/// It is the safety valve for a template whose `tag_from` changed, and it
/// **removes** tags before re-deriving them — so a bug here loses tags a user
/// typed. The only test it had asserted a *refusal* (a project registered
/// without a template has nothing to re-derive), so the path that actually
/// touches the file had none at all.
#[test]
fn tag_reauto_re_derives_the_automatic_tags_and_keeps_the_free_form_ones() {
    let sb = Sandbox::new();
    // A template whose `tier` variable drives an auto tag.
    let dir = sb.install.join("templates").join("client");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: Client\nslug: client\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: C\n  digits: 4\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n\
         \x20   transform: none\n  - slug: tier\n    label: Tier\n    type: text\n\
         \x20   transform: none\n\
         tag_from: [\"tier\"]\n",
    )
    .unwrap();

    sb.ok(&[
        "new",
        "client",
        "--name=One",
        "--tier=Indie",
        "--yes",
        "--no-preview",
    ]);
    sb.ok(&["tag", "add", "C0001", "urgent"]);

    let before = sb.ok(&["tag", "list", "C0001"]);
    assert!(before.contains("tier/Indie"), "{before}");
    assert!(before.contains("urgent"), "{before}");

    let out = sb.ok(&["tag", "reauto", "C0001"]);
    assert!(
        out.contains("C0001"),
        "the verb should name what it did:\n{out}"
    );

    let after = sb.ok(&["tag", "list", "C0001"]);
    assert!(
        after.contains("tier/Indie"),
        "the derived tag comes back:\n{after}"
    );
    assert!(
        after.contains("urgent"),
        "and a tag the user typed is not the template's to remove:\n{after}"
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
        err.contains("ID0001")
            && err.contains("cannot be used")
            // The layers say one thing each: which project, which folder,
            // and which file is missing. The outer one used to claim the
            // *folder* had gone, which is the one thing that was still there.
            && err.contains("not a project folder")
            && err.contains("project metadata is missing"),
        "path must refuse a project whose metadata has gone, and say so:\n{err}"
    );
    let err = sb.fails(&["copy", "ID0001"]);
    assert!(
        err.contains("ID0001") && err.contains("cannot be copied"),
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

/// Without a desktop session there is nothing to open a window on: `open` and
/// `term` say so and exit non-zero, rather than starting a program that dies
/// at once and reporting it opened.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn open_and_term_refuse_without_a_display() {
    let sb = Sandbox::new();
    sb.plant_project(&sb.base, "proj", "ID0001");
    for verb in ["open", "term"] {
        let out = sb
            .command()
            .args([verb, "ID0001"])
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .expect("running fastf");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success(),
            "{verb} must not claim success:\n{stderr}"
        );
        assert!(
            stderr.contains("no display"),
            "{verb} should say what is missing:\n{stderr}"
        );
    }
}

/// **Every template the list names can be shown and created from.**
///
/// A manifest whose `slug:` disagreed with its directory was listed under the
/// manifest's name, which every lookup then rejected — `fastf template show`
/// answering "not found — run `fastf template list`" about a name that list
/// had just printed. Only a real process sees both halves.
#[test]
fn every_template_the_list_names_can_be_shown_and_created() {
    let sb = Sandbox::new();
    // The `cp -r templates/general templates/my-kit` case: folder renamed,
    // manifest not.
    let dir = sb.install.join("templates").join("my-kit");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: My Kit\nslug: general\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: K\n  digits: 3\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n",
    )
    .unwrap();

    let listed = sb.ok(&["template", "list"]);
    assert!(
        listed.contains("my-kit"),
        "the list names the folder: {listed}"
    );

    for slug in listed
        .lines()
        .filter_map(|line| line.trim().strip_prefix("• "))
        .map(|line| {
            line.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|slug| !slug.is_empty())
    {
        sb.ok(&["template", "show", &slug]);
    }

    // And it can be created from — `new` used to print a whole preview and
    // *then* fail, because the picker's template and the one `operations`
    // re-resolves under the lock were looked up two different ways.
    let out = sb.ok(&["new", "my-kit", "--name=Probe", "--dry-run", "--yes"]);
    assert!(out.contains("K001"), "{out}");
}

/// **`template show` promises "copied byte-for-byte" — so it must only list
/// files that are.** A root `PROJECT_INFO.md` is stripped from the text buffer
/// (fastf owns that name) and dropped by every copy path, so it was absent from
/// the buffer, present on disk, and named here as a bundled asset. An excluded
/// file was listed for the same reason.
#[test]
fn template_show_lists_only_assets_that_are_really_copied() {
    let sb = Sandbox::new();
    let dir = sb.install.join("templates").join("kit");
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        "name: Kit\nslug: kit\nnaming_pattern: \"{id}_{name}\"\n\
         id:\n  prefix: K\n  digits: 3\n\
         exclude: [\"*.tmp\"]\n\
         variables:\n  - slug: name\n    label: Name\n    type: text\n    required: true\n",
    )
    .unwrap();
    // A real bundled asset, plus two files no create will ever write.
    fs::write(dir.join("files/logo.bin"), [0u8, 255, 16]).unwrap();
    fs::write(dir.join("files/PROJECT_INFO.md"), "---\nid: nope\n---\n").unwrap();
    fs::write(dir.join("files/scratch.tmp"), [0u8, 1]).unwrap();

    let shown = sb.ok(&["template", "show", "kit"]);
    assert!(
        shown.contains("logo.bin"),
        "the real asset is listed:\n{shown}"
    );
    assert!(
        !shown.contains("PROJECT_INFO.md"),
        "a file fastf drops is not promised byte-for-byte:\n{shown}"
    );
    assert!(
        !shown.contains("scratch.tmp"),
        "nor an excluded one:\n{shown}"
    );
}
