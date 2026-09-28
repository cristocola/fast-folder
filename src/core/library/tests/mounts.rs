//! Moves on mounts that misbehave: emptied in place, put back, dropped, and
//! paused.

use super::*;

/// **On a cloud mount the original is emptied where it stands**, its
/// `PROJECT_INFO.md` first: nothing is renamed, no `.fastf-moved-*` folder is
/// made, and a pointer beside the original names the record while it waits
/// out the settle. Once a later pass finds it still gone, the record and the
/// pointer go, the pointer last.
#[cfg(debug_assertions)]
#[test]
fn on_a_cloud_mount_the_original_is_emptied_in_place() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::create_dir_all(old_base.join("proj_a/src/deep")).unwrap();
    fs::write(old_base.join("proj_a/src/deep/a.rs"), "fn main() {}").unwrap();
    fs::write(old_base.join("proj_a/b.bin"), [1_u8, 2, 3]).unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));

    crate::util::faults::with_thread_fault("fs:as-rclone", || {
        let outcome = staged_copy_verify_commit(
            &project,
            new_base,
            &new_path,
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(outcome.source, SourceOutcome::Removed);
        assert!(!old_base.join("proj_a").exists(), "emptied where it stood");
        let hidden: Vec<String> = fs::read_dir(old_base)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(transactions::RETIRED_PREFIX))
            .collect();
        assert_eq!(hidden.len(), 1, "only the pointer: {hidden:?}");
        let operation = transactions::pointer_operation(&hidden[0])
            .expect("a pointer, not a renamed folder")
            .to_string();
        let record = new_base
            .join(transactions::TRANSACTIONS_DIR)
            .join(&operation);
        let journal = transactions::read_journal(&record).unwrap();
        assert_eq!(journal.retire, transactions::RetireStrategy::InPlace);
        assert_eq!(
            fs::read_to_string(new_path.join("src/deep/a.rs")).unwrap(),
            "fn main() {}"
        );

        // Ten minutes on, still gone: the record goes, then the pointer.
        let mut entry = crate::core::records::get(&operation).unwrap();
        entry.gone_at = Some(crate::util::time::now_unix() - crate::core::records::SETTLE_SECS);
        crate::core::records::add(&entry);
        let report = provisioning::reconcile_unlocked(&cfg);
        assert!(!record.exists(), "{report:?}");
        assert!(!old_base.join(&hidden[0]).exists(), "and the pointer, last");
    });
}

/// **A cloud mount can put the original's `PROJECT_INFO.md` back** after the
/// move removed it — an upload still queued; the lab's edit-then-move on R2
/// got the pre-edit file back. An older version holds nothing the moved copy
/// lacks and goes; one edited after the copy is a choice between two
/// versions, and needs a person. Neither is ever listed as the project twice.
#[cfg(debug_assertions)]
#[test]
fn a_project_info_put_back_goes_when_older_and_is_asked_about_when_newer() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/b.bin"), [1_u8, 2, 3]).unwrap();
    let before = fs::read_to_string(old_base.join("proj_a/PROJECT_INFO.md")).unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let old_copy = old_base.join("proj_a");
    let put_back = |text: &str, modified: std::time::SystemTime| {
        fs::create_dir_all(&old_copy).unwrap();
        fs::write(old_copy.join("PROJECT_INFO.md"), text).unwrap();
        fs::File::options()
            .write(true)
            .open(old_copy.join("PROJECT_INFO.md"))
            .unwrap()
            .set_modified(modified)
            .unwrap();
    };
    let listed = |cfg: &Config| {
        discover(cfg)
            .iter()
            .filter(|found| found.id == "ID0001")
            .count()
    };

    crate::util::faults::with_thread_fault("fs:as-rclone", || {
        let progress = Mutex::new(Progress::new(&[]));
        staged_copy_verify_commit(
            &project,
            new_base,
            &new_path,
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!old_copy.exists());

        // The version from before the move comes back, older than the copy.
        let long_ago = std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        put_back(&before, long_ago);
        assert_eq!(listed(&cfg), 1, "the old copy is not listed as the project");
        let report = provisioning::reconcile_unlocked(&cfg);
        assert!(!old_copy.exists(), "the stale version goes: {report:?}");
        assert!(report.verdicts.is_empty(), "nobody is asked: {report:?}");

        // An edit made after the copy comes back instead: a newer version.
        let edited = format!("{before}\n- 2026-01-02T00:00:00Z — written after the copy\n");
        put_back(
            &edited,
            std::time::SystemTime::now() + Duration::from_secs(60),
        );
        assert_eq!(listed(&cfg), 1, "still listed once");
        let report = provisioning::reconcile_unlocked(&cfg);
        assert!(
            old_copy.join("PROJECT_INFO.md").is_file(),
            "kept: {report:?}"
        );
        let attention = crate::core::attention::attention(&cfg);
        let item = attention
            .items
            .iter()
            .find(|item| item.path == old_copy || item.path.ends_with("proj_a"))
            .unwrap_or_else(|| panic!("{attention:?}"));
        assert_eq!(item.state, crate::core::attention::State::NeedsYou);
        assert_eq!(listed(&cfg), 1, "and still listed once");

        // Keeping the moved copy's version finishes it.
        crate::core::attention::resolve(
            &cfg,
            &item.path,
            crate::core::attention::Action::KeepMoved,
        )
        .unwrap();
        assert!(!old_copy.exists());
        assert_eq!(listed(&cfg), 1);
    });
}

/// A file changed in the original between the move's last look and the
/// retire — a dev server's log line — is carried into the moved copy by the
/// merge, and the old copy still goes whole.
#[cfg(debug_assertions)]
#[test]
fn a_log_written_into_the_old_copy_after_the_retire_reaches_the_moved_copy() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::create_dir_all(old_base.join("proj_a/.astro")).unwrap();
    fs::write(old_base.join("proj_a/.astro/dev.log"), "started\n").unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    let (outcome, housekeeping) = crate::core::move_engine::staged_copy_verify_commit_in_parts(
        &project, new_base, &new_path, &progress, &cancel,
    )
    .unwrap();
    let housekeeping = housekeeping.expect("the old copy is left to remove");
    // The dev server's open handle followed the rename into the old copy.
    let old_copy = housekeeping.path();
    fs::write(old_copy.join(".astro/dev.log"), "started\nreloaded\n").unwrap();
    let outcome = crate::core::move_engine::finish_housekeeping(
        outcome,
        Some(housekeeping),
        crate::core::progress::Ticker::none(),
    );

    assert_eq!(outcome.source, SourceOutcome::Removed);
    assert!(!old_copy.exists());
    assert_eq!(
        fs::read_to_string(new_path.join(".astro/dev.log")).unwrap(),
        "started\nreloaded\n",
        "fast-forwarded into the moved copy"
    );
}

/// Changed in both copies since the move: only that entry stays behind,
/// with the record, and the moved copy's version is left as it is.
#[cfg(debug_assertions)]
#[test]
fn a_file_changed_in_both_copies_keeps_only_that_entry_and_the_record() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/notes.txt"), "v1").unwrap();
    fs::write(old_base.join("proj_a/other.txt"), "untouched").unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    let (outcome, housekeeping) = crate::core::move_engine::staged_copy_verify_commit_in_parts(
        &project, new_base, &new_path, &progress, &cancel,
    )
    .unwrap();
    let housekeeping = housekeeping.unwrap();
    let old_copy = housekeeping.path();
    fs::write(old_copy.join("notes.txt"), "v2 in the old copy").unwrap();
    fs::write(new_path.join("notes.txt"), "v2 in the moved copy").unwrap();
    let outcome = crate::core::move_engine::finish_housekeeping(
        outcome,
        Some(housekeeping),
        crate::core::progress::Ticker::none(),
    );

    let SourceOutcome::Leftover {
        redundant: false,
        reason,
        ..
    } = &outcome.source
    else {
        panic!("a conflict is kept: {:?}", outcome.source);
    };
    assert!(reason.contains("notes.txt"), "{reason}");
    assert_eq!(
        fs::read_to_string(old_copy.join("notes.txt")).unwrap(),
        "v2 in the old copy"
    );
    assert_eq!(
        fs::read_to_string(new_path.join("notes.txt")).unwrap(),
        "v2 in the moved copy"
    );
    assert!(!old_copy.join("other.txt").exists(), "the rest went");
    assert_eq!(
        fs::read_dir(new_base.join(transactions::TRANSACTIONS_DIR))
            .unwrap()
            .count(),
        1,
        "the record stays"
    );
}

/// **A mount that drops mid-removal is waited for, not given up on**: three
/// unlinks answered "not connected", then the mount is back, and the old copy
/// goes whole — no leftover, no reconcile.
#[cfg(debug_assertions)]
#[test]
fn a_mount_that_drops_mid_removal_is_ridden_out() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    for n in 0..10 {
        fs::write(old_base.join(format!("proj_a/f{n}.bin")), [n as u8; 16]).unwrap();
    }
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));

    let outcome = crate::util::faults::with_thread_fault("remove:unlink:enotconn-3", || {
        staged_copy_verify_commit(
            &project,
            new_base,
            &new_path,
            &progress,
            &AtomicBool::new(false),
        )
    })
    .unwrap();
    assert_eq!(outcome.source, SourceOutcome::Removed);
    assert!(retired_folders(old_base).is_empty());
    assert!(!old_base.join("proj_a").exists());
}

/// A write the mount fails once is copied again, whole: the move finishes
/// and the file arrives as it was.
#[cfg(debug_assertions)]
#[test]
fn a_write_the_mount_fails_once_is_copied_again() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/big.bin"), vec![7_u8; 3 * 1024 * 1024]).unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));

    let outcome = crate::util::faults::with_thread_fault("copy:write:eio-1", || {
        staged_copy_verify_commit(
            &project,
            new_base,
            &new_path,
            &progress,
            &AtomicBool::new(false),
        )
    })
    .unwrap();
    assert_eq!(outcome.source, SourceOutcome::Removed);
    assert_eq!(
        fs::read(new_path.join("big.bin")).unwrap(),
        vec![7_u8; 3 * 1024 * 1024]
    );
}

/// A removal the filesystem refuses — no permission — is not retried for
/// ever and not waved through: the path is named, and the record stays.
#[cfg(debug_assertions)]
#[test]
fn a_refused_removal_names_the_path_and_keeps_the_record() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/locked.bin"), [1_u8]).unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));

    let outcome = crate::util::faults::with_thread_fault("remove:unlink:eacces", || {
        staged_copy_verify_commit(
            &project,
            new_base,
            &new_path,
            &progress,
            &AtomicBool::new(false),
        )
    })
    .unwrap();
    let SourceOutcome::Leftover { reason, .. } = &outcome.source else {
        panic!("a refused removal is a leftover: {:?}", outcome.source);
    };
    assert!(
        reason.contains("locked.bin") || reason.contains("PROJECT_INFO.md"),
        "{reason}"
    );
    assert_eq!(
        fs::read_dir(new_base.join(transactions::TRANSACTIONS_DIR))
            .unwrap()
            .count(),
        1,
        "the record stays"
    );
}

/// **A move whose mount stops answering for good pauses, and goes on from its
/// copy.** Nothing copied is thrown away: the record says it paused, and
/// moving the project again takes over the copy — what arrived whole stays,
/// only the rest is copied — then finishes like any move.
#[cfg(debug_assertions)]
#[test]
fn a_move_that_loses_its_mount_pauses_and_resumes_from_its_copy() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/aa-empty.txt"), "").unwrap();
    fs::write(old_base.join("proj_a/data.bin"), [9_u8; 4096]).unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    // `move_project_in_parts` reads the configuration under the lock.
    cfg.save().unwrap();
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    let paused = crate::util::faults::with_thread_fault(
        "pool:serial,move:force-staged,fs:short-mount-wait,copy:write:enotconn",
        || crate::core::move_engine::move_project_in_parts(&project, new_base, &progress, &cancel),
    );
    let error = paused.unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::core::move_engine::Paused>()
            .is_some(),
        "{error:#}"
    );
    assert!(
        project.path.join("data.bin").is_file(),
        "the original is untouched"
    );
    assert!(
        new_path.join("aa-empty.txt").is_file(),
        "what arrived whole is kept"
    );
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(new_path.join("aa-empty.txt")).unwrap().ino()
    };

    let (outcome, housekeeping) =
        crate::core::move_engine::move_project_in_parts(&project, new_base, &progress, &cancel)
            .unwrap();
    let outcome = crate::core::move_engine::finish_housekeeping(
        outcome,
        housekeeping,
        crate::core::progress::Ticker::none(),
    );
    assert_eq!(outcome.source, SourceOutcome::Removed);
    assert_eq!(fs::read(new_path.join("data.bin")).unwrap(), [9_u8; 4096]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(new_path.join("aa-empty.txt")).unwrap().ino(),
            inode,
            "adopted, not copied again"
        );
    }
    assert!(!project.path.exists());
}
