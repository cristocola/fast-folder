//! Cross-process concurrency.
//!
//! These spawn **real processes**, not threads, and that is the whole point.
//! An in-process `Mutex` would pass a thread-based test while production stayed
//! broken: the actual collision is one `fastf new` racing another in a second
//! terminal. Ten concurrent creates reliably minted duplicate IDs.
//!
//! Each test drives the built binary with its own `FASTF_INSTALL_DIR`, so the
//! only thing shared between the processes is the sandbox on disk — exactly the
//! situation on a real machine.

mod common;

use common::{Sandbox, project_dirs};
use fastf::core::project_info;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Child;

/// How many processes to race. Enough to lose reliably when unsynchronized —
/// the original bug showed up as 8 distinct IDs out of 10.
const RACERS: usize = 10;

/// The racing suite always wants the `race` template installed.
fn racing_sandbox() -> Sandbox {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb
}

/// The headline regression: ten simultaneous creates must mint ten distinct IDs.
///
/// Before the cross-process lock this produced eight — `ID0012` and `ID0015`
/// each minted twice — silently breaking the tool's central promise that IDs are
/// unique across every project.
#[test]
fn concurrent_creates_mint_distinct_ids() {
    let sb = racing_sandbox();

    let children: Vec<Child> = (0..RACERS)
        .map(|i| {
            sb.spawn(&[
                "new",
                "race",
                &format!("--name=R{i}"),
                "--yes",
                "--no-preview",
            ])
        })
        .collect();
    for mut child in children {
        let _ = child.wait();
    }

    let ids = sb.ids_on_disk();
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        RACERS,
        "expected {RACERS} projects, got {}: {ids:?}",
        ids.len()
    );
    assert_eq!(
        unique.len(),
        ids.len(),
        "duplicate IDs minted under concurrency: {ids:?}"
    );

    // The counter must also agree with reality, or the next create collides.
    //
    // It lives in the **base**, not the data directory: the projects already sit
    // on a drive every OS on the machine can mount, so the number that indexes
    // them belongs there too. Keeping it in `%APPDATA%` / `~/.config` is what
    // forced a dual-boot install to symlink one home directory into the other.
    let counters = fs::read_to_string(sb.base.join(".fastf-counter.toml"))
        .expect("the base should carry the counter");
    assert!(
        counters.contains(&format!("global = {RACERS}")),
        "counter out of step with {RACERS} projects: {counters}"
    );
    // And to the data directory, which is the half that survives a base being
    // unplugged. The base file is the cross-OS half; this is the cross-base one.
    let local = fs::read_to_string(sb.install.join("counters.toml")).expect("local counter");
    assert!(
        local.contains(&format!("global = {RACERS}")),
        "local counter out of step with {RACERS} projects: {local}"
    );
}

/// Racing creates that resolve to the *same* folder name: exactly one may win.
///
/// The old `exists()`-then-`create_dir_all()` pair let two racers both pass the
/// check and write into one folder, the second overwriting the first's files and
/// metadata. `create_dir` now fails atomically, so the filesystem arbitrates.
#[test]
fn concurrent_same_name_creates_produce_no_merged_folder() {
    let sb = racing_sandbox();

    let children: Vec<Child> = (0..RACERS)
        .map(|_| sb.spawn(&["new", "race", "--name=Twin", "--yes", "--no-preview"]))
        .collect();
    let mut succeeded = 0;
    for mut child in children {
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            succeeded += 1;
        }
    }

    let dirs = project_dirs(&sb.base);
    assert_eq!(
        dirs.len(),
        succeeded,
        "every success must correspond to exactly one folder \
         ({succeeded} succeeded, {} folders)",
        dirs.len()
    );

    // No folder may have been written into twice: each carries exactly one id,
    // and all ids across the base are distinct.
    let ids = sb.ids_on_disk();
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "a folder was clobbered: {ids:?}");
}

/// Concurrent `config set` of different keys must not lose an update.
///
/// `Config::save` was a read-modify-write with a bare `fs::write`; two of these
/// at once dropped one of the changes and a crash mid-write truncated the file.
#[test]
fn concurrent_config_writes_do_not_lose_updates() {
    let sb = racing_sandbox();

    let writes: Vec<(&str, &str)> = vec![
        ("date-format", "%Y%m%d"),
        ("recent-limit", "42"),
        ("preview-lines", "3"),
        ("register-naming-pattern", "{name}_{id}"),
        ("confirm-create", "false"),
    ];
    let children: Vec<Child> = writes
        .iter()
        .map(|(key, value)| sb.spawn(&["config", "set", key, value]))
        .collect();
    for mut child in children {
        let _ = child.wait();
    }

    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    for (key, value) in &writes {
        // The stored field is `recent_default_limit`; `recent-limit` is what
        // the key was renamed to at v3.0.0.
        let field = match *key {
            "recent-limit" => "recent_default_limit".to_string(),
            other => other.replace('-', "_"),
        };
        let expected_present = config
            .lines()
            .any(|l| l.starts_with(&field) && l.contains(value));
        assert!(
            expected_present,
            "lost the update to {key} = {value}\n--- config.toml ---\n{config}"
        );
    }
    // base-dir set during setup must have survived them all.
    assert!(config.contains("base_dir"), "base_dir was lost:\n{config}");
}

/// A create racing a `register` — both mint IDs from the same counter, through
/// different code paths.
#[test]
fn concurrent_create_and_register_do_not_collide() {
    let sb = racing_sandbox();

    // Folders for register to adopt.
    let adoptees: Vec<PathBuf> = (0..4)
        .map(|i| {
            let dir = sb.base.join(format!("existing_{i}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("notes.txt"), "pre-existing work").unwrap();
            dir
        })
        .collect();

    let mut children: Vec<Child> = (0..4)
        .map(|i| {
            sb.spawn(&[
                "new",
                "race",
                &format!("--name=N{i}"),
                "--yes",
                "--no-preview",
            ])
        })
        .collect();
    for dir in &adoptees {
        children.push(sb.spawn(&["register", &dir.display().to_string(), "--yes"]));
    }
    for mut child in children {
        let _ = child.wait();
    }

    let ids = sb.ids_on_disk();
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "create and register minted colliding IDs: {ids:?}"
    );
}

/// Tag and note commands both rewrite PROJECT_INFO.md. Racing them must retain
/// every frontmatter and journal update, not let the last atomic rename erase
/// changes read by another process before it acquired the mutation lock.
#[test]
fn concurrent_tag_and_note_updates_do_not_lose_metadata() {
    let sb = racing_sandbox();
    sb.ok(&["new", "race", "--name=Shared", "--yes", "--no-preview"]);
    let project = project_dirs(&sb.base).pop().expect("created project");

    let mut children = Vec::new();
    for index in 0..5 {
        children.push(sb.spawn(&["tag", "add", "R0001", &format!("tag-{index}")]));
        children.push(sb.spawn(&["note", "add", "R0001", &format!("note-{index}")]));
    }
    for mut child in children {
        let status = child.wait().expect("wait for mutation");
        assert!(status.success(), "concurrent mutation failed: {status}");
    }

    let metadata = project_info::read_metadata(&project)
        .unwrap()
        .expect("project metadata");
    let notes = project_info::read_journal_entries(&project).unwrap();
    for index in 0..5 {
        assert!(
            metadata.tags.contains(&format!("tag-{index}")),
            "lost tag-{index}: {:?}",
            metadata.tags
        );
        assert!(
            notes
                .iter()
                .any(|entry| entry.message == format!("note-{index}")),
            "lost note-{index}; retained messages: {:?}",
            notes
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>()
        );
    }
}

/// Deleting a template must wait for whatever fastf operation is in flight.
///
/// `fastf template delete` used to call `fs::remove_dir_all` directly, with no
/// lock at all: it could remove a template's `files/` out from under a
/// `fastf new` that was halfway through copying them, in another terminal.
///
/// The holder here is **this test process**, not a second fastf. That is
/// deliberate: a race between two spawned children is a race, and would pass or
/// fail on scheduling. Holding the real `DataLock` on the sandbox's own lock
/// file makes the question exact — while it is held, the delete must make no
/// progress at all, and the template must still be on disk.
#[test]
fn deleting_a_template_waits_for_the_data_lock() {
    use fastf::util::lockfile::DataLock;
    use std::time::{Duration, Instant};

    let sb = racing_sandbox();
    let template_dir = sb.install.join("templates").join("race");
    assert!(template_dir.is_dir(), "fixture template should exist");

    let held = DataLock::acquire_at(&sb.install.join(".fastf.lock"), Duration::from_secs(5))
        .expect("the sandbox lock should be free");

    let mut child = sb.spawn(&["template", "delete", "race", "--yes"]);

    // Give it far longer than it needs to start up, reach the lock, and (on an
    // unlocked build) finish the whole delete.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut exited = None;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("polling the child") {
            exited = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    assert!(
        exited.is_none(),
        "the delete ran to completion while the data lock was held: {exited:?}"
    );
    assert!(
        template_dir.is_dir(),
        "the template was removed while another operation held the lock"
    );

    // Released — and only now may it proceed.
    drop(held);
    let status = child.wait().expect("waiting for the delete");
    assert!(
        status.success(),
        "the delete should succeed once the lock is free"
    );
    assert!(
        !template_dir.exists(),
        "the delete should have removed the template after acquiring the lock"
    );
}
