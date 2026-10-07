//! **A base that stopped answering holds nothing else up.** A dead SMB, NFS
//! or FUSE mount blocks every call into it for the kernel's own timeout, so
//! nothing asks it without a deadline — and a command about a project in
//! another base is not about it at all.
//!
//! Real processes, with `paths:stall-base` armed: every look into the folder
//! holding `.fastf-test-stall` blocks while the file is there, as a dead mount
//! does. Each command has to be done long before that look would come back.
//! Debug builds only, like every suite that arms a failpoint.
#![cfg(debug_assertions)]

use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

mod common;

use common::Sandbox;

/// How long a command may take. A look into the stalled base comes back after
/// two minutes; the deadline a base is given is a second and a half.
const LIMIT: Duration = Duration::from_secs(20);

/// A library of three bases: the one projects are made in, one that stopped
/// answering, and one listed after it that holds a project.
struct Library {
    sb: Sandbox,
    far: PathBuf,
    project: PathBuf,
}

fn library() -> Library {
    let sb = Sandbox::new();
    sb.write_template("race");
    let mut bases = sb.with_bases(&["stalled", "far"]);
    let far = bases.remove(1);
    let stalled = bases.remove(0);
    let project = sb.plant_project(&far, "2026-01-01_Shoot_ID0007", "ID0007");
    fs::write(project.join("take.mov"), "frames").unwrap();
    fs::write(stalled.join(fastf::util::paths::STALL_MARKER), "").unwrap();
    Library { sb, far, project }
}

/// What a command printed, and how it ended.
struct Ended {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// `fastf <args>` with the stall armed, killed and failed if it is not done
/// inside [`LIMIT`].
fn run(library: &Library, args: &[&str]) -> Ended {
    let label = args.join("-").replace(['/', '\\', ':'], "_");
    let out_path = library.sb.tmp.path().join(format!("out-{label}"));
    let err_path = library.sb.tmp.path().join(format!("err-{label}"));
    let mut child = library
        .sb
        .command()
        .args(args)
        .env("FASTF_FAULT", "paths:stall-base")
        .stdin(Stdio::null())
        .stdout(fs::File::create(&out_path).unwrap())
        .stderr(fs::File::create(&err_path).unwrap())
        .spawn()
        .expect("starting fastf");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "`fastf {}` was held up by a base that does not answer:\n{}",
                args.join(" "),
                fs::read_to_string(&err_path).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ended {
        ok: status.success(),
        stdout: fs::read_to_string(&out_path).unwrap_or_default(),
        stderr: fs::read_to_string(&err_path).unwrap_or_default(),
    }
}

fn done(library: &Library, args: &[&str]) -> Ended {
    let ended = run(library, args);
    assert!(
        ended.ok,
        "`fastf {}` failed:\n{}\n{}",
        args.join(" "),
        ended.stdout,
        ended.stderr
    );
    ended
}

/// A create reads the counter's floor from every base and writes the new
/// number back to each: from and to the ones that answer.
#[test]
fn a_create_is_not_held_up() {
    let library = library();
    done(
        &library,
        &["new", "race", "--name=Solo", "--yes", "--no-post"],
    );
    let made: Vec<String> = common::project_dirs(&library.sb.base)
        .iter()
        .filter_map(|dir| dir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(made.len(), 1, "{made:?}");
    assert!(
        made[0].starts_with("R0008"),
        "the floor came from the base that answers: {made:?}"
    );
    assert_eq!(
        library.sb.base_counter(&library.far),
        8,
        "and went back to it"
    );
}

/// A preview reads the same floor.
#[test]
fn a_preview_is_not_held_up() {
    let library = library();
    let preview = done(
        &library,
        &["new", "race", "--name=Solo", "--dry-run", "--no-post"],
    );
    assert!(preview.stdout.contains("R0008"), "{}", preview.stdout);
}

/// A change to a project in a base listed after the one that does not
/// answer: every mutation first finds the project's own base among the
/// configured ones.
#[test]
fn a_change_to_a_project_in_another_base_is_not_held_up() {
    let library = library();
    done(&library, &["tag", "add", "ID0007", "draft"]);
    let info = fs::read_to_string(library.project.join("PROJECT_INFO.md")).unwrap();
    assert!(info.contains("draft"), "{info}");

    done(&library, &["desc", "ID0007", "the second shoot"]);
    let info = fs::read_to_string(library.project.join("PROJECT_INFO.md")).unwrap();
    assert!(info.contains("description: the second shoot"), "{info}");

    done(
        &library,
        &["rename", "ID0007", "2026-01-01_Shoot_Two_ID0007", "--yes"],
    );
    assert!(library.far.join("2026-01-01_Shoot_Two_ID0007").is_dir());
}

/// A register looks for its id in every base, and for the folder's own base
/// among the configured ones.
#[test]
fn a_register_is_not_held_up() {
    let library = library();
    let folder = library.far.join("Old_Reel");
    fs::create_dir_all(&folder).unwrap();
    done(&library, &["register", &folder.display().to_string()]);
    assert!(folder.join("PROJECT_INFO.md").is_file());
}

/// A reindex rescans the bases that answer, and counts those.
#[test]
fn a_reindex_is_not_held_up() {
    let library = library();
    let said = done(&library, &["reindex"]);
    assert!(
        said.stdout.contains("1 project") && said.stdout.contains("2 bases"),
        "{}",
        said.stdout
    );
}

/// A reconcile says the base is waited for, and goes on to the others.
#[test]
fn a_reconcile_is_not_held_up() {
    let library = library();
    let said = done(&library, &["reconcile"]);
    assert!(
        said.stdout.contains("does not answer") && said.stdout.contains("stalled"),
        "{}",
        said.stdout
    );
}

/// A move between two bases that answer.
#[test]
fn a_move_between_two_other_bases_is_not_held_up() {
    let library = library();
    let target = library.sb.base.display().to_string();
    done(&library, &["move", "ID0007", &target, "--yes"]);
    assert!(
        library
            .sb
            .base
            .join("2026-01-01_Shoot_ID0007/take.mov")
            .is_file()
    );
    assert!(!library.project.exists());
}

/// A copy out is checked against every base that answers; one that does not
/// cannot hold a folder that does.
#[test]
fn a_copy_out_is_not_held_up() {
    let library = library();
    let outside = library.sb.tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    done(
        &library,
        &["copy-to", "ID0007", &outside.display().to_string(), "--yes"],
    );
    assert!(outside.join("2026-01-01_Shoot_ID0007/take.mov").is_file());
}

/// A project in the base that does not answer is refused by name, at once:
/// nothing about it can be read.
#[test]
fn a_change_in_the_base_that_does_not_answer_is_refused_at_once() {
    let library = library();
    // The project's folder, as a path: it cannot be looked up, its base
    // holds the marker.
    let inside = library
        .sb
        .tmp
        .path()
        .join("stalled")
        .join("2026-01-01_Lost_ID0003");
    let refused = run(&library, &["register", &inside.display().to_string()]);
    assert!(!refused.ok, "{}", refused.stdout);
    assert!(
        refused.stderr.contains("does not answer"),
        "{}",
        refused.stderr
    );
}
