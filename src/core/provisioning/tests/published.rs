//! Published moves: the original set aside and merged away, whatever became of
//! either copy meanwhile.

use super::*;

#[test]
fn cleanup_pending_retries_but_identity_mismatch_never_mutates() {
    let temp = tempfile::tempdir().unwrap();
    let source_base = temp.path().join("source");
    let target_base = temp.path().join("target");
    fs::create_dir(&source_base).unwrap();
    fs::create_dir(&target_base).unwrap();
    let cfg = move_config(&source_base, &target_base);
    let (source, manifest, mut transaction) = prepared_transaction(&source_base, &target_base);
    let staging = fill_staging(&source, &manifest, &transaction);
    transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
    let final_path = target_base.join("project");
    fs::rename(staging, &final_path).unwrap();
    transaction.set_phase(MovePhase::CleanupPending).unwrap();
    let operation = transaction.operation_dir.clone();

    crate::core::project_info::write_frontmatter(
        &crate::core::project_info::pinfo_path(&final_path),
        |metadata| metadata.id = "ID9999".to_string(),
    )
    .unwrap();
    let mismatch = reconcile_unlocked(&cfg);
    assert_eq!(mismatch.completed, 0);
    assert!(!mismatch.unrecoverable.is_empty());
    assert!(source.is_dir(), "identity mismatch must preserve source");
    assert!(operation.is_dir(), "transaction must remain for inspection");

    let repeated = reconcile_unlocked(&cfg);
    assert_eq!(repeated.completed, 0);
    assert!(source.is_dir());
    assert!(operation.is_dir());
}

/// **An original 3.11 half deleted in place is finished.** What is left is
/// exactly what the move recorded, so it is provably redundant, and the
/// pass finishes it — rewriting the journal as version 3 before the
/// rename, so a 3.11 binary cannot then lose track of it.
#[cfg(debug_assertions)]
#[test]
fn a_311_original_it_half_deleted_is_finished_and_rewritten_first() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    fs::remove_file(operation.join(transactions::PUBLISHED_FILE)).unwrap();
    as_written_by_311(&operation);
    // And the manifest as 3.11 wrote it: version 1.
    let manifest_path = operation.join(transactions::MANIFEST_FILE);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["version"] = serde_json::json!(1);
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::remove_file(source.join("payload.part")).unwrap();
    fs::remove_dir(source.join("empty")).unwrap();

    let held =
        crate::util::faults::with_thread_fault("move:source-cleanup", || reconcile_unlocked(&cfg));
    assert_eq!(held.completed, 0, "{held:?}");
    assert_eq!(
        journal_version(&operation),
        3,
        "rewritten before the rename"
    );
    assert!(
        source.is_dir(),
        "a retire that failed leaves it where it was"
    );

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!source.exists());
    assert!(hidden_folders(&source_base).is_empty());
    assert!(!operation.exists());
    assert_eq!(
        fs::read(final_path.join("payload.part")).unwrap(),
        [0_u8, 1, 255]
    );
}

/// The same, with `PROJECT_INFO.md` among what 3.11 removed: no identity
/// to read, but everything left is recorded and unchanged.
#[test]
fn a_311_residue_without_its_project_info_is_finished_too() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    fs::remove_file(operation.join(transactions::PUBLISHED_FILE)).unwrap();
    as_written_by_311(&operation);
    fs::remove_file(crate::core::project_info::pinfo_path(&source)).unwrap();
    fs::remove_file(source.join("payload.part")).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!source.exists());
}

/// **Work written into the original after the scan is kept — in the
/// moved copy.** The merge carries a file that exists only there into the
/// moved copy (within the hour after the publish) and removes the rest. A
/// 3.11 record is merged as a residue: nothing is written into the moved
/// copy, and what is new stays in the hidden old copy, with its record,
/// named.
#[test]
fn work_written_into_the_original_after_the_scan_is_kept() {
    for from_311 in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        if from_311 {
            as_written_by_311(&operation);
            fs::remove_file(source.join("payload.part")).unwrap();
        }
        fs::write(source.join("written-after.txt"), b"new work").unwrap();

        let report = reconcile_unlocked(&cfg);
        assert!(!source.exists(), "the original left the library either way");
        if from_311 {
            assert_eq!(report.completed, 0, "{report:?}");
            let said = report.leftovers.join("\n");
            assert!(said.contains("written-after.txt"), "{said}");
            assert!(!said.contains("delete"), "{said}");
            let hidden = hidden_folders(&source_base);
            assert_eq!(hidden.len(), 1, "{hidden:?}");
            assert_eq!(
                fs::read(source_base.join(&hidden[0]).join("written-after.txt")).unwrap(),
                b"new work"
            );
            assert!(
                !final_path.join("written-after.txt").exists(),
                "a residue never writes into the moved copy"
            );
            assert!(operation.is_dir(), "the record stays with it");
        } else {
            assert_eq!(report.completed, 1, "{report:?}");
            assert_eq!(
                fs::read(final_path.join("written-after.txt")).unwrap(),
                b"new work"
            );
            assert!(hidden_folders(&source_base).is_empty());
            assert!(!operation.exists());
        }
    }
}

/// Once retired, the retired copy may be the only one left if the moved
/// copy has since gone. Never removed then; the report says how to put it
/// back.
#[test]
fn a_retired_original_is_kept_when_the_moved_copy_is_gone() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    fs::remove_dir_all(&final_path).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 0, "{report:?}");
    assert!(retired.join("payload.part").is_file(), "never removed");
    assert!(
        report.unrecoverable.join("\n").contains("rename it back"),
        "{report:?}"
    );
}

/// A file written into the retired copy since — a program that had it
/// open in the original — exists nowhere else. Within the hour after the
/// publish it is carried into the moved copy; later it stays where it is,
/// named, with the record, and only the rest of the old copy goes.
#[test]
fn a_file_written_into_the_retired_copy_is_carried_across_within_the_hour() {
    for late in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let operation = transaction.operation_dir.clone();
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        fs::write(retired.join("autosave.tmp"), b"only here").unwrap();
        if late {
            let published = fs::File::options()
                .write(true)
                .open(operation.join(transactions::PUBLISHED_FILE))
                .unwrap();
            published
                .set_modified(
                    std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600),
                )
                .unwrap();
        }

        let report = reconcile_unlocked(&cfg);
        if late {
            assert_eq!(report.completed, 0, "{report:?}");
            let said = report.leftovers.join("\n");
            assert!(said.contains("autosave.tmp"), "{said}");
            assert_eq!(
                fs::read(retired.join("autosave.tmp")).unwrap(),
                b"only here"
            );
            assert!(
                !retired.join("payload.part").exists(),
                "what the moved copy holds went"
            );
            assert!(!final_path.join("autosave.tmp").exists());
            assert!(operation.is_dir());
        } else {
            assert_eq!(report.completed, 1, "{report:?}");
            assert_eq!(
                fs::read(final_path.join("autosave.tmp")).unwrap(),
                b"only here"
            );
            assert!(!retired.exists());
            assert!(!operation.exists());
        }
    }
}

/// **An old copy on a mount that does not answer keeps its record.** An
/// error is not "gone": the pass waits, changes nothing, and the next one
/// that can look finishes.
#[cfg(debug_assertions)]
#[test]
fn a_path_that_does_not_answer_keeps_the_record() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    let operation = transaction.operation_dir.clone();

    let report =
        crate::util::faults::with_thread_fault("presence:lstat:eio", || reconcile_unlocked(&cfg));
    assert_eq!(report.completed, 0, "{report:?}");
    assert_eq!(report.waiting.len(), 1, "{report:?}");
    assert!(report.waiting[0].contains("does not answer"), "{report:?}");
    assert!(
        !report.needs_a_look(),
        "waiting is not a person's job: {report:?}"
    );
    assert!(retired.join("payload.part").is_file(), "nothing removed");
    assert!(operation.is_dir(), "the record stays");

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!retired.exists());
    assert!(!operation.exists());
    assert_eq!(
        fs::read(final_path.join("payload.part")).unwrap(),
        [0_u8, 1, 255]
    );
}

/// A base that is still configured but not mounted is waited for, not
/// reported as gone: an unplugged drive is ordinary.
#[test]
fn a_source_base_that_is_not_mounted_is_waited_for() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (_source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let unplugged = temp.path().join("unplugged");
    fs::rename(&source_base, &unplugged).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(
        report.waiting.len(),
        2,
        "the base, and the move on it: {report:?}"
    );
    assert!(report.unrecoverable.is_empty(), "{report:?}");
    assert!(!report.needs_a_look(), "{report:?}");
    assert!(operation.is_dir(), "the record stays");
    fs::rename(&unplugged, &source_base).unwrap();
    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
}

/// **A rename that stopped part of the way** — an S3 bucket through rclone
/// copies and deletes object by object — leaves part of the original at
/// its path and part at its retired name. Both are the move's own: the
/// retired half goes, then what is left of the original, and the record
/// only after both.
#[test]
fn a_split_rename_is_finished_on_both_halves() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let retired = transaction.retired_path();
    fs::create_dir(&retired).unwrap();
    fs::rename(source.join("payload.part"), retired.join("payload.part")).unwrap();
    fs::rename(source.join("empty"), retired.join("empty")).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!retired.exists(), "the retired half is removed");
    assert!(
        !source.exists(),
        "and so is the half left at the original's path"
    );
    assert!(!operation.exists(), "the record goes last");
    assert_eq!(
        fs::read(final_path.join("payload.part")).unwrap(),
        [0_u8, 1, 255]
    );
}

/// A split whose half at the original's path holds something the move did
/// not record, beside our identity, is not a residue: both halves wait
/// for a person, and so does the record.
#[test]
fn a_split_half_holding_something_new_is_kept_with_its_record() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let retired = transaction.retired_path();
    fs::create_dir(&retired).unwrap();
    fs::rename(source.join("payload.part"), retired.join("payload.part")).unwrap();
    fs::write(source.join("notes-since.txt"), b"written since").unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 0, "{report:?}");
    assert_eq!(
        fs::read(source.join("notes-since.txt")).unwrap(),
        b"written since"
    );
    assert!(operation.is_dir(), "the record stays");
    assert!(
        report.leftovers.join("\n").contains("notes-since.txt"),
        "{report:?}"
    );
}

/// A folder a program made again at the original's path in the moment
/// between the retire and its record holds no identity and nothing the
/// move recorded: it is not a residue, it is left alone, and the move is
/// finished.
#[test]
fn a_folder_made_again_before_retired_was_recorded_is_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(source.join("saved-late.txt"), b"late").unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!retired.exists());
    assert!(!operation.exists());
    assert_eq!(fs::read(source.join("saved-late.txt")).unwrap(), b"late");
}

/// A record whose publish could not read its own `PROJECT_INFO.md` back,
/// and so left it out of `published.json`, still finishes.
#[test]
fn a_published_record_without_project_info_still_finishes() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let published_path = operation.join(transactions::PUBLISHED_FILE);
    let mut published: serde_json::Value =
        serde_json::from_slice(&fs::read(&published_path).unwrap()).unwrap();
    published["entries"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| entry["path"] != crate::core::project_info::RESERVED_FILENAME);
    fs::write(&published_path, serde_json::to_vec(&published).unwrap()).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!source.exists());
    assert!(!operation.exists());
}

/// A moved copy missing entries after the original was retired is
/// completed from the retired copy — where the original's entries are by
/// then, not at its old, empty path.
#[test]
fn a_moved_copy_missing_entries_after_the_retire_is_completed_from_it() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    fs::remove_file(final_path.join("payload.part")).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert_eq!(
        fs::read(final_path.join("payload.part")).unwrap(),
        [0_u8, 1, 255]
    );
    assert!(!retired.exists());
}

/// **The settle.** On a mount that can put a removed folder back (rclone
/// can, from uploads still queued, minutes after the removal), the
/// record outlives the removal: a pass inside the settle keeps it without
/// anyone being told, one after it clears it, and an old copy that came
/// back is removed by the record that is still there.
#[cfg(debug_assertions)]
#[test]
fn a_removal_on_a_mount_that_can_put_it_back_waits_out_the_settle() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, mut transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let id = transaction.journal.operation_id.clone();
    crate::core::records::add(&crate::core::records::Entry {
        operation: id.clone(),
        kind: "move".to_string(),
        record: operation.clone(),
        source_base: source_base.clone(),
        target_base: target_base.clone(),
        ..Default::default()
    });
    let retired = transaction.retired_path();
    let modified = fs::metadata(source.join("payload.part"))
        .unwrap()
        .modified()
        .unwrap();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();

    crate::util::faults::with_thread_fault("fs:as-rclone", || {
        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 1, "{report:?}");
        assert!(!retired.exists(), "the old copy is removed");
        assert!(operation.is_dir(), "the record waits out the settle");
        assert!(
            list_incomplete(&cfg).is_empty(),
            "a settling record is nobody's business"
        );

        // The mount puts part of it back: the upload it still had queued,
        // the same bytes with the same time.
        fs::create_dir(&retired).unwrap();
        fs::write(retired.join("payload.part"), [0_u8, 1, 255]).unwrap();
        fs::File::options()
            .write(true)
            .open(retired.join("payload.part"))
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let report = reconcile_unlocked(&cfg);
        assert!(!retired.exists(), "what came back is removed: {report:?}");
        assert!(operation.is_dir(), "and the settle starts again");

        // Ten minutes on, still gone: the record goes.
        let mut entry = crate::core::records::get(&id).unwrap();
        entry.gone_at = Some(crate::util::time::now_unix() - crate::core::records::SETTLE_SECS);
        crate::core::records::add(&entry);
        let report = reconcile_unlocked(&cfg);
        assert!(!operation.exists(), "{report:?}");
        assert_eq!(crate::core::records::get(&id), None, "and its index entry");
    });
}

/// Something written at the original's path after it was retired — an
/// editor saving to the old path — is not the original, and is not
/// touched.
#[test]
fn a_folder_recreated_at_the_original_path_is_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, mut transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(source.join("saved-late.txt"), b"late").unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!retired.exists(), "the retired copy is removed");
    assert_eq!(fs::read(source.join("saved-late.txt")).unwrap(), b"late");
}

/// Another project now at the original's path is not the original: it is
/// left alone, still listed, and the move is finished.
#[test]
fn another_project_at_the_original_path_is_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    fs::remove_dir_all(&source).unwrap();
    write_project(&source_base, "project", "ID0999");

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(
        report
            .unrecoverable
            .join("\n")
            .contains("different project"),
        "{report:?}"
    );
    assert!(source.join("payload.part").is_file(), "untouched");
    assert!(!operation.exists());
    assert!(
        crate::core::library::discover(&cfg)
            .iter()
            .any(|project| project.id == "ID0999"),
        "and still listed"
    );
}

/// The retired original renamed back by hand, the moved copy gone: the
/// move is undone, and the record goes with it.
#[test]
fn an_original_put_back_by_hand_undoes_the_move() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    fs::remove_dir_all(&final_path).unwrap();
    fs::rename(&retired, &source).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.rolled_back, 1, "{report:?}");
    assert!(source.join("payload.part").is_file());
    assert!(!operation.exists());
}

/// A cloud mount can misplace uploads while a 3.12.0 record's staging
/// folder is renamed into place, so the moved copy is published missing
/// files. The original is whole and unchanged and holds them, so reconcile
/// puts them back itself — folder, file and link alike — and finishes the
/// move. Nobody copies files by hand.
#[test]
fn a_moved_copy_missing_entries_is_completed_from_the_original() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let source = write_project(&source_base, "project", "ID0001");
    fs::create_dir_all(source.join("node_modules/three/src")).unwrap();
    fs::write(source.join("node_modules/three/src/Water2.js"), "water").unwrap();
    fs::write(source.join("node_modules/three/src/Uniform.js"), "uniform").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../src/Water2.js", source.join("node_modules/three/link")).unwrap();
    let mut transaction = as_staged_under_the_record(
        MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
            Operation::Move,
        )
        .unwrap(),
    );
    let manifest = MoveManifest::scan(&source).unwrap();
    transaction.write_manifest(&manifest).unwrap();
    let staging = fill_staging(&source, &manifest, &transaction);
    let staged = manifest.verify_destination(&staging).unwrap();
    transaction.write_published(&staged).unwrap();
    transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
    fs::rename(staging, target_base.join("project")).unwrap();
    transaction.set_phase(MovePhase::CleanupPending).unwrap();
    let final_path = target_base.join("project");
    // What the mount lost: a file, a whole folder, and a link.
    fs::remove_file(final_path.join("node_modules/three/src/Uniform.js")).unwrap();
    fs::remove_dir_all(final_path.join("empty")).unwrap();
    #[cfg(unix)]
    fs::remove_file(final_path.join("node_modules/three/link")).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(report.unrecoverable.is_empty(), "{report:?}");
    assert_eq!(
        fs::read_to_string(final_path.join("node_modules/three/src/Uniform.js")).unwrap(),
        "uniform"
    );
    assert!(final_path.join("empty").is_dir());
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(final_path.join("node_modules/three/link")).unwrap(),
        Path::new("../src/Water2.js")
    );
    assert!(
        !source.exists(),
        "the original left, once the copy was whole"
    );
    assert!(hidden_folders(&source_base).is_empty());
    assert_eq!(
        fs::read_dir(transactions::transaction_root(&target_base))
            .unwrap()
            .count(),
        0
    );
}

/// A moved copy that holds an *older* file than was published — restored
/// from a backup taken before the move — is not something fastf
/// overwrites, or removes the original's version for. The original leaves
/// the library; that one file of it stays, hidden, with the record, and
/// nobody is told to delete anything.
#[test]
fn a_moved_copy_older_than_published_keeps_the_originals_version() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400);
    let file = fs::File::options()
        .write(true)
        .open(final_path.join("payload.part"))
        .unwrap();
    file.set_modified(old).unwrap();
    drop(file);

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 0, "{report:?}");
    let said = report.leftovers.join("\n");
    assert!(said.contains("older in the moved copy"), "{said}");
    assert!(!said.contains("delete"), "{said}");
    assert!(!source.exists(), "the original left the library");
    let hidden = hidden_folders(&source_base);
    assert_eq!(hidden.len(), 1, "{hidden:?}");
    let kept = source_base.join(&hidden[0]);
    assert!(
        kept.join("payload.part").is_file(),
        "the original's version is kept"
    );
    assert!(
        !kept
            .join(crate::core::project_info::RESERVED_FILENAME)
            .exists(),
        "and only what differs"
    );
    assert!(transaction.operation_dir.is_dir(), "with its record");
}

/// A 3.12.0 record whose original is gone, and whose old staging folder
/// holds two files the mount uploaded there after the rename — the one
/// durable copy of each. They are moved into place, never deleted, and
/// only then does the record go.
#[test]
fn files_a_mount_left_in_an_old_records_staging_are_moved_into_place() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    fs::remove_dir_all(&source).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    let operation = transaction.operation_dir.clone();
    // The stray upload: recorded, at its recorded size, at the old path.
    let stray_dir = operation.join(transactions::STAGING_DIR);
    fs::create_dir_all(&stray_dir).unwrap();
    fs::write(stray_dir.join("payload.part"), [0_u8, 1, 255]).unwrap();
    fs::remove_file(final_path.join("payload.part")).unwrap();
    // And one the move never recorded: kept, and the record with it.
    fs::write(stray_dir.join("unrecorded.tmp"), b"?").unwrap();

    let first = reconcile_unlocked(&cfg);
    assert_eq!(first.completed, 0, "{first:?}");
    assert_eq!(
        fs::read(final_path.join("payload.part")).unwrap(),
        [0_u8, 1, 255],
        "the stray is in place"
    );
    assert_eq!(fs::read(stray_dir.join("unrecorded.tmp")).unwrap(), b"?");
    assert!(
        operation.is_dir(),
        "the record stays while something is left"
    );
    assert!(
        first.leftovers.join("\n").contains("unrecorded.tmp"),
        "{first:?}"
    );

    fs::remove_file(stray_dir.join("unrecorded.tmp")).unwrap();
    let second = reconcile_unlocked(&cfg);
    assert_eq!(second.completed, 1, "{second:?}");
    assert!(!operation.exists());
}

/// **A repair is not something to look at.** Strays moved into place with
/// nothing left behind finish the record, and the pass says what it put right
/// without asking anybody to do anything.
#[test]
fn strays_moved_into_place_are_a_repair_and_need_no_look() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    fs::remove_dir_all(&source).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    let operation = transaction.operation_dir.clone();
    let stray_dir = operation.join(transactions::STAGING_DIR);
    fs::create_dir_all(&stray_dir).unwrap();
    fs::write(stray_dir.join("payload.part"), [0_u8, 1, 255]).unwrap();
    fs::remove_file(final_path.join("payload.part")).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!operation.exists(), "the record is gone");
    assert_eq!(
        fs::read(final_path.join("payload.part")).unwrap(),
        [0_u8, 1, 255]
    );
    assert_eq!(report.repaired.len(), 1, "{report:?}");
    assert!(
        report.repaired[0].contains("moved into place"),
        "{report:?}"
    );
    assert!(report.unrecoverable.is_empty(), "{report:?}");
    assert!(!report.needs_a_look(), "{report:?}");
}

/// The same, with the manifest gone too (the mount misplaced its rename
/// as well): a stray goes where the moved copy holds nothing, and one the
/// moved copy already has a file for is kept, since nothing says which is
/// right.
#[test]
fn strays_are_placed_without_a_manifest_only_where_nothing_is() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    fs::remove_dir_all(&source).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    let operation = transaction.operation_dir.clone();
    fs::remove_file(operation.join(transactions::MANIFEST_FILE)).unwrap();
    let stray_dir = operation.join(transactions::STAGING_DIR);
    fs::create_dir_all(stray_dir.join("empty")).unwrap();
    fs::write(stray_dir.join("payload.part"), [0_u8, 1, 255]).unwrap();
    fs::remove_file(final_path.join("payload.part")).unwrap();
    fs::write(stray_dir.join("PROJECT_INFO.md"), b"a late upload").unwrap();
    // The same bytes as the moved copy already holds: moved over it.
    fs::create_dir_all(stray_dir.join("empty")).unwrap();
    let same = fs::read(final_path.join("PROJECT_INFO.md")).unwrap();
    fs::create_dir_all(stray_dir.join("sub")).unwrap();
    fs::write(stray_dir.join("sub/twin.md"), &same).unwrap();
    fs::create_dir_all(final_path.join("sub")).unwrap();
    fs::write(final_path.join("sub/twin.md"), &same).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(
        fs::read(final_path.join("payload.part")).unwrap(),
        [0_u8, 1, 255],
        "placed where nothing was"
    );
    assert!(
        !stray_dir.join("sub/twin.md").exists(),
        "the twin was moved over"
    );
    assert!(
        fs::read_to_string(final_path.join("PROJECT_INFO.md"))
            .unwrap()
            .contains("id: ID0001"),
        "the moved copy's own file was not replaced"
    );
    assert!(stray_dir.join("PROJECT_INFO.md").is_file(), "kept");
    assert!(operation.is_dir(), "and the record with it: {report:?}");
    assert!(report.leftovers.join("\n").contains("PROJECT_INFO.md"));
}
