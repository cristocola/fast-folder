//! `fastf path` and `fastf copy`: one bare line, for another command to take.

use super::*;

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
            // and which file is missing — never that the *folder* has gone,
            // which is the one thing still there.
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
/// `open` prints. A terminal gets a picker instead; a script must not.
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
