//! `fastf note` and `fastf notes`: what is read from a hand-edited file, and
//! what an editor hands back.

use super::*;

/// `fastf notes` survives a hand-edited PROJECT_INFO.md with multi-byte text
/// where the timestamp goes: a timestamp sliced to 10 *bytes* panics
/// mid-character. This is `hostile_fs.rs`'s promise — corrupt metadata
/// degrades, never panics — kept for the journal body.
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

/// **A note is read as far as it can be, whatever shape it was typed in.**
///
/// Every line under the heading belongs to the note above it, and a line
/// starting with a date starts one, with or without the ` — ` separator.
#[test]
fn notes_reads_a_hand_written_note_and_every_line_under_it() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let pinfo = dir.join("PROJECT_INFO.md");
    // Under the heading the planted file already has — typed the way a
    // person types, not the way fastf writes.
    let mut text = fs::read_to_string(&pinfo).unwrap();
    assert!(text.ends_with("## Notes\n"), "{text:?}");
    text.push_str(
        "\nfree text above the first entry\n\n\
         - 2026-05-01 Should probably call the client again.\n\
         - 2026-05-02T10:00:00Z — Timeline v02 has a render problem\n\
         render with quicktime\n",
    );
    fs::write(&pinfo, text).unwrap();

    let out = sb.ok(&["notes", "ID0001"]);
    for line in [
        "free text above the first entry",
        "2026-05-01  Should probably call the client again.",
        "2026-05-02  Timeline v02 has a render problem",
        "render with quicktime",
        "3 notes",
    ] {
        assert!(out.contains(line), "missing {line:?} in:\n{out}");
    }
    // `--since` compares the day, and leaves the undated note out.
    let out = sb.ok(&["notes", "ID0001", "--since", "2026-05-02"]);
    assert!(
        out.contains("render problem") && !out.contains("free text"),
        "{out}"
    );
    assert!(out.contains("1 note\n"), "{out}");
}

/// **A multi-line note round-trips through `note add` and `notes`.** Lines
/// 2+ are written indented under the first and read back with it, so a note
/// read from stdin or an editor keeps every line.
#[test]
fn a_multi_line_note_round_trips() {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "proj", "ID0001");
    let mut child = sb
        .spawn_with_stdin(&["note", "add", "ID0001", "-"])
        .expect("spawn note add");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(b"client made a poem for me\n\noh you who edit my videos\nroad is long\n")
            .unwrap();
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "note add failed: {status}");

    let file = fs::read_to_string(dir.join("PROJECT_INFO.md")).unwrap();
    assert!(
        file.ends_with(
            " — client made a poem for me\n\n  oh you who edit my videos\n  road is long\n"
        ),
        "the note's other lines are indented under its first:\n{file}"
    );
    let out = sb.ok(&["notes", "ID0001"]);
    assert!(out.contains("client made a poem for me"), "{out}");
    assert!(
        out.contains("oh you who edit my videos") && out.contains("road is long"),
        "{out}"
    );
    assert!(out.contains("1 note\n"), "{out}");
}

/// `note add` with no message resolves the editor through the documented
/// `$EDITOR` fallback; the raw `editor` config field is empty on an
/// unconfigured install.
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

/// `note add` holds no write handle on the scratch file while the editor runs.
/// Windows Notepad saves by reopening the file for writing with
/// `FILE_SHARE_READ` alone, so a handle fastf keeps open is a sharing
/// violation on every save: Notepad falls back to a Save As dialog, and the
/// note never reaches the journal.
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

/// An editor that failed did not open the project, whatever it exited for.
///
/// The editor's exit status is the answer, not the spawn: on Windows
/// `cmd /c start` spawns for an editor that does not exist, so a typo in the
/// `editor` key would report success over nothing at all.
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
