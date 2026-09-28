//! Rename, unregister and delete, and what a killed one leaves for reconcile.

use super::*;

/// Renaming only the capitalisation is legitimate, though `exists()` is
/// case-insensitive on Windows and the target "already exists" — it is the
/// source.
#[test]
fn rename_allows_case_only_change() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let dir = base.join("myproject");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        project_info::pinfo_path(&dir),
        "---\nid: ID0001\ntemplate: t\ntemplate_name: T\n\
         created: 2026-01-01T00:00:00Z\nfolder: myproject\npath: x\n\
         variables: {}\ntags: []\n---\n",
    )
    .unwrap();
    fs::write(dir.join("keep.txt"), "content").unwrap();

    let project = scan_base(base).into_iter().next().unwrap();
    let renamed = rename_project_unlocked(&project, "MyProject").unwrap();

    assert_eq!(renamed.name, "MyProject");
    assert!(renamed.path.join("keep.txt").is_file(), "content survived");
    // The folder really carries the new casing on disk.
    let on_disk = fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.eq_ignore_ascii_case("myproject"))
        .expect("project folder present");
    assert_eq!(on_disk, "MyProject");
    // No staging folder stranded — a dot-prefixed name is invisible to
    // discovery, so a leftover would make the project disappear.
    assert!(
        !fs::read_dir(base)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("fastf-case")),
        "case-rename staging folder left behind"
    );
    assert_eq!(scan_base(base).len(), 1, "still exactly one project");
}

/// **A rename waits for a folder that is held for a moment.** Windows renames
/// no folder while a file in it is open, and a scanner or an indexer reading
/// what was just written holds it for a second or so: the folder schedule
/// (`fs_retry::schedule::FOLDER_RENAME`) outlasts that, the file one does not.
#[cfg(windows)]
#[test]
fn a_rename_waits_for_a_folder_held_for_a_moment() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let project = scan_base(base).remove(0);

    let held = held_for_a_moment(&base.join("proj_a"));
    let renamed = rename_project_unlocked(&project, "proj_b");
    held.join().unwrap();

    let renamed = renamed.expect("the folder was let go inside the folder schedule");
    assert_eq!(renamed.name, "proj_b");
    assert!(base.join("proj_b").join("held.mov").is_file());
    assert!(!base.join("proj_a").exists());
}

/// The same for a rename that changes only the capitalisation, which is two
/// renames through a hidden name.
#[cfg(windows)]
#[test]
fn a_case_only_rename_waits_for_a_folder_held_for_a_moment() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let project = scan_base(base).remove(0);

    let held = held_for_a_moment(&base.join("proj_a"));
    let renamed = rename_project_unlocked(&project, "Proj_A");
    held.join().unwrap();

    let renamed = renamed.expect("the folder was let go inside the folder schedule");
    assert_eq!(renamed.name, "Proj_A");
    let names: Vec<String> = fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with(".fastf-"))
        .collect();
    assert_eq!(names, ["Proj_A"], "one folder, under its new name");
}

#[test]
fn stale_project_identity_cannot_authorize_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(base.join("proj/sentinel"), b"keep").unwrap();
    let stale = scan_base(base).remove(0);

    project_info::write_frontmatter(&project_info::pinfo_path(&stale.path), |metadata| {
        metadata.id = "ID9999".to_string();
    })
    .unwrap();
    let error = delete_project_unlocked(&stale).unwrap_err();
    assert!(error.to_string().contains("identity changed"));
    assert_eq!(fs::read(base.join("proj/sentinel")).unwrap(), b"keep");
}

/// Removing a project must drop its cache entry, or `recent` keeps listing
/// something that is gone until the staleness gate happens to fire.
#[test]
fn delete_unregister_and_rename_all_drop_the_old_cache_entry() {
    let cached_dirs = |base: &Path| -> Vec<String> {
        load_cache(base)
            .map(|c| c.entries.into_iter().map(|e| e.dir).collect())
            .unwrap_or_default()
    };

    // delete
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "gone", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_project(base, "stays", "ID0002", "gen", "2026-01-02T00:00:00Z");
    write_cache(base, &scan_base(base)).unwrap();
    let doomed = scan_base(base)
        .into_iter()
        .find(|p| p.name == "gone")
        .unwrap();
    delete_project_unlocked(&doomed).unwrap();
    let dirs = cached_dirs(base);
    assert!(!dirs.contains(&"gone".to_string()), "stale entry: {dirs:?}");
    assert!(dirs.contains(&"stays".to_string()), "collateral: {dirs:?}");

    // unregister
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "dropme", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_cache(base, &scan_base(base)).unwrap();
    let p = scan_base(base).into_iter().next().unwrap();
    unregister_project_unlocked(&p).unwrap();
    assert!(
        !cached_dirs(base).contains(&"dropme".to_string()),
        "unregister must drop the cache entry"
    );
    assert!(base.join("dropme").is_dir(), "the folder itself stays");

    // rename
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "before", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_cache(base, &scan_base(base)).unwrap();
    let p = scan_base(base).into_iter().next().unwrap();
    rename_project_unlocked(&p, "after").unwrap();
    let dirs = cached_dirs(base);
    assert!(!dirs.contains(&"before".to_string()), "old entry: {dirs:?}");
    assert!(dirs.contains(&"after".to_string()), "new entry: {dirs:?}");
}

/// The stranded-rename message must name the path the folder is actually at.
///
/// This branch is unreachable by a real filesystem failure in a test — it
/// needs the commit *and* the rollback to fail — so what is pinned here is
/// the thing that matters when it does happen: a user staring at an error
/// can find their project again. A dot-prefixed name is invisible to
/// discovery, so an error that omits it leaves nothing to go on.
#[test]
fn a_stranded_case_rename_names_the_folder_it_left_behind() {
    let staging = Path::new("/library/base/.MyProject.fastf-case");
    let message = stranded_rename_message(
        "renaming 'myproject' to 'MyProject'",
        staging,
        "Permission denied (os error 13)",
    );
    assert!(
        message.contains("renaming 'myproject' to 'MyProject'"),
        "{message}"
    );
    assert!(message.contains(".MyProject.fastf-case"), "{message}");
    assert!(message.contains("Permission denied"), "{message}");
    assert!(
        message.contains("by hand"),
        "the user needs a next step, not just a diagnosis: {message}"
    );
}

/// `fastf delete` leaves the library in one rename too, and a removal that
/// stops after it leaves a hidden folder reconcile clears — never a husk.
#[cfg(debug_assertions)]
#[test]
fn a_delete_stopped_after_its_rename_is_cleared_by_reconcile() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(base.join("proj/payload.bin"), [1_u8, 2, 3]).unwrap();
    let cfg = cfg_for(base, &[]);
    let project = scan_base(base).remove(0);

    crate::util::faults::with_thread_fault("delete:after-retire", || {
        delete_project_inner(&project)
    })
    .unwrap();

    assert!(!base.join("proj").exists(), "no husk at the old path");
    assert_eq!(retired_folders(base).len(), 1);
    assert!(scan_base(base).is_empty(), "nothing listed");

    let report = provisioning::reconcile_unlocked(&cfg);
    assert_eq!(report.cleared, 1, "{report:?}");
    assert!(retired_folders(base).is_empty());
}

#[test]
fn rename_sanitizes_and_rejects_bad_names() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let cfg = cfg_for(base, &[]);
    let project = discover(&cfg).remove(0);

    // Illegal filesystem chars are sanitized, not fatal.
    let renamed = rename_project_unlocked(&project, "New: Name?").unwrap();
    assert_eq!(renamed.name, "New_ Name_");
    assert!(renamed.path.is_dir());
    assert!(!project.path.exists());

    // Dot-prefixed names would be invisible to discovery → rejected.
    let err = rename_project_unlocked(&renamed, ".hidden")
        .unwrap_err()
        .to_string();
    assert!(err.contains("may not start with '.'"), "err: {err}");
    // Same-name rename is a no-op error, not a silent success.
    let err = rename_project_unlocked(&renamed, "New_ Name_")
        .unwrap_err()
        .to_string();
    assert!(err.contains("already the folder's name"), "err: {err}");
    assert!(renamed.path.is_dir(), "folder intact after failed renames");
}

#[test]
fn unregister_and_delete_guard_rails() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(base.join("proj_a").join("keep.txt"), "data").unwrap();
    let cfg = cfg_for(base, &[]);
    let project = discover(&cfg).remove(0);

    // Unregister removes only the metadata file.
    unregister_project_unlocked(&project).unwrap();
    assert!(project.path.join("keep.txt").is_file());
    assert!(!project_info::pinfo_path(&project.path).exists());
    // Double-unregister is a clean error.
    assert!(unregister_project_unlocked(&project).is_err());

    // Delete refuses a folder without PROJECT_INFO.md (the guard rail).
    let err = delete_project_unlocked(&project).unwrap_err().to_string();
    assert!(err.contains("no PROJECT_INFO.md"), "err: {err}");
    assert!(project.path.is_dir());

    // Re-register (rewrite metadata) → delete removes the whole folder.
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    delete_project_unlocked(&project).unwrap();
    assert!(!project.path.exists());
}

/// `fastf delete` removes the project and never what a link in it points at:
/// its removal walks the tree itself, not through std's `remove_dir_all`, so
/// the guard std would give for free is this test's to keep.
#[test]
fn a_delete_never_removes_what_a_link_points_at() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("base");
    let library_folder = tmp.path().join("asset_library");
    fs::create_dir_all(&library_folder).unwrap();
    fs::write(library_folder.join("stock.mov"), "irreplaceable").unwrap();
    write_project(&base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let link = base.join("proj").join("assets");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&library_folder, &link).unwrap();
    #[cfg(windows)]
    assert!(
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&library_folder)
            .output()
            .unwrap()
            .status
            .success()
    );
    let project = scan_base(&base).remove(0);

    // Out of the library in one rename; then the hidden folder's removal,
    // which a caller runs once the data lock is released.
    delete_project_inner(&project)
        .unwrap()
        .run(crate::core::progress::Ticker::none());

    assert!(!base.join("proj").exists());
    assert!(retired_folders(&base).is_empty());
    assert_eq!(
        fs::read_to_string(library_folder.join("stock.mov")).unwrap(),
        "irreplaceable"
    );
}

/// **On a cloud mount a delete empties the folder where it stands**: its
/// record first — every entry the project holds — then its
/// `PROJECT_INFO.md`, so it leaves the library at once; then only what the
/// record lists. Something written there by path after the delete began is
/// kept, with the record, and named. Nothing is renamed.
#[cfg(debug_assertions)]
#[test]
fn on_a_cloud_mount_a_delete_empties_the_folder_in_place() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::create_dir_all(base.join("proj/src")).unwrap();
    fs::write(base.join("proj/src/a.rs"), "fn a() {}").unwrap();
    let cfg = cfg_for(base, &[]);
    let project = scan_base(base).remove(0);

    crate::util::faults::with_thread_fault("fs:as-rclone", || {
        let housekeeping = delete_project_inner(&project).unwrap();
        assert!(
            !base.join("proj/PROJECT_INFO.md").exists(),
            "out of the library first"
        );
        assert!(scan_base(base).is_empty(), "nothing listed");
        assert!(
            retired_folders(base)
                .iter()
                .all(|name| !base.join(name).is_dir()),
            "nothing renamed"
        );
        // A program writes into it by path meanwhile.
        fs::write(base.join("proj/src/late.log"), "written after").unwrap();
        let fate = housekeeping.run(crate::core::progress::Ticker::none());
        let crate::core::move_cleanup::SourceFate::Leftover { reason, .. } = fate else {
            panic!("what came after is kept: {fate:?}");
        };
        assert!(reason.contains("late.log"), "{reason}");
        assert!(!base.join("proj/src/a.rs").exists(), "what it held went");
        assert_eq!(
            fs::read_to_string(base.join("proj/src/late.log")).unwrap(),
            "written after"
        );
        let records: Vec<String> = fs::read_dir(base)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| crate::core::move_cleanup::deleted_record_operation(name).is_some())
            .collect();
        assert_eq!(records.len(), 1, "the record stays with it");

        // Once it is gone, reconcile clears the record (past the settle).
        fs::remove_file(base.join("proj/src/late.log")).unwrap();
        let operation = crate::core::move_cleanup::deleted_record_operation(&records[0])
            .unwrap()
            .to_string();
        let report = provisioning::reconcile_unlocked(&cfg);
        assert!(!base.join("proj").exists(), "{report:?}");
        let mut entry = crate::core::records::get(&operation).unwrap();
        entry.gone_at = Some(crate::util::time::now_unix() - crate::core::records::SETTLE_SECS);
        crate::core::records::add(&entry);
        let report = provisioning::reconcile_unlocked(&cfg);
        assert!(!base.join(&records[0]).exists(), "{report:?}");
    });
}

/// An in-place delete killed before its housekeeping ran — its record
/// written, its `PROJECT_INFO.md` gone, nothing else removed — is finished by
/// the next reconcile, and so is one killed before even the
/// `PROJECT_INFO.md` went.
#[cfg(debug_assertions)]
#[test]
fn an_in_place_delete_killed_part_of_the_way_is_finished_by_reconcile() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    for before_the_identity_went in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::write(base.join("proj/payload.bin"), [1_u8, 2, 3]).unwrap();
        let cfg = cfg_for(base, &[]);
        let project = scan_base(base).remove(0);
        let identity = fs::read(base.join("proj/PROJECT_INFO.md")).unwrap();

        crate::util::faults::with_thread_fault("fs:as-rclone", || {
            let housekeeping = delete_project_inner(&project).unwrap();
            drop(housekeeping);
            if before_the_identity_went {
                fs::write(base.join("proj/PROJECT_INFO.md"), &identity).unwrap();
            }
            let report = provisioning::reconcile_unlocked(&cfg);
            assert!(!base.join("proj").exists(), "{report:?}");
        });
        assert!(scan_base(base).is_empty());
    }
}
