//! Records of moves that never published, and of copies: discarded, or
//! finished where the publish had in fact landed.

use super::*;

#[test]
fn copying_recovery_discards_only_the_owned_transaction() {
    let temp = tempfile::tempdir().unwrap();
    let source_base = temp.path().join("source");
    let target_base = temp.path().join("target");
    fs::create_dir(&source_base).unwrap();
    fs::create_dir(&target_base).unwrap();
    let (source, manifest, transaction) = prepared_transaction(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    let staging = fill_staging(&source, &manifest, &transaction);
    fs::write(target_base.join("real.tmp"), b"bystander").unwrap();

    let report = reconcile_unlocked(&move_config(&source_base, &target_base));
    assert_eq!(report.rolled_back, 1, "{report:?}");
    assert!(source.is_dir());
    assert!(!operation.exists());
    assert!(!staging.exists());
    assert_eq!(
        fs::read(target_base.join("real.tmp")).unwrap(),
        b"bystander"
    );
}

#[test]
fn ready_with_staging_rolls_back_and_ready_after_publication_finishes() {
    let temp = tempfile::tempdir().unwrap();
    let source_base = temp.path().join("source");
    let target_base = temp.path().join("target");
    fs::create_dir(&source_base).unwrap();
    fs::create_dir(&target_base).unwrap();
    let cfg = move_config(&source_base, &target_base);

    let (source, manifest, mut transaction) = prepared_transaction(&source_base, &target_base);
    fill_staging(&source, &manifest, &transaction);
    transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.rolled_back, 1, "{report:?}");
    assert!(source.is_dir());
    assert!(!target_base.join("project").exists());

    let manifest = MoveManifest::scan(&source).unwrap();
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
    transaction.write_manifest(&manifest).unwrap();
    let staging = fill_staging(&source, &manifest, &transaction);
    transaction.set_phase(MovePhase::ReadyToCommit).unwrap();
    fs::rename(&staging, target_base.join("project")).unwrap();

    let first = reconcile_unlocked(&cfg);
    let second = reconcile_unlocked(&cfg);
    assert_eq!(first.completed, 1, "{first:?}");
    assert!(second.is_empty(), "recovery must be idempotent: {second:?}");
    assert!(!source.exists());
    assert_eq!(
        fs::read(target_base.join("project/payload.part")).unwrap(),
        [0_u8, 1, 255]
    );
    assert_eq!(
        fs::read_dir(transactions::transaction_root(&target_base))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn malformed_v2_transaction_is_report_only() {
    let temp = tempfile::tempdir().unwrap();
    let source_base = temp.path().join("source");
    let target_base = temp.path().join("target");
    fs::create_dir(&source_base).unwrap();
    fs::create_dir(&target_base).unwrap();
    let sentinel = source_base.join("sentinel");
    fs::write(&sentinel, b"keep").unwrap();
    let root = transactions::ensure_transaction_root(&target_base).unwrap();
    let operation = root.join("bad-operation");
    fs::create_dir(&operation).unwrap();
    let journal = operation.join(transactions::JOURNAL_FILE);
    fs::write(
        &journal,
        format!(
            "{{\"version\":2,\"operation_id\":\"../escape\",\"project_id\":\"ID0001\",\"source_base\":\"{}\",\"source_folder\":\"../sentinel\",\"target_folder\":\"project\",\"phase\":\"CleanupPending\"}}",
            source_base.display()
        ),
    )
    .unwrap();
    let before = fs::read(&journal).unwrap();

    let report = reconcile_unlocked(&move_config(&source_base, &target_base));
    assert!(!report.unrecoverable.is_empty());
    assert_eq!(fs::read(&journal).unwrap(), before);
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
}

/// A record killed between making its folder and finishing its journal
/// holds nothing else — a move writes its manifest next, and only then
/// copies — and is removed rather than called invalid for ever.
#[test]
fn a_record_killed_before_its_journal_was_written_is_removed() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let root = transactions::ensure_transaction_root(&target_base).unwrap();
    let torn = root.join("18d8e31be3d08289-eae64-7");
    fs::create_dir(&torn).unwrap();
    fs::write(
        torn.join(transactions::JOURNAL_FILE),
        b"{\"version\":3,\"operat",
    )
    .unwrap();
    let empty = root.join("18d8e31be3d08289-eae64-8");
    fs::create_dir(&empty).unwrap();
    let _ = source_base;

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.cleared, 2, "{report:?}");
    assert!(report.unrecoverable.is_empty(), "{report:?}");
    assert!(!torn.exists() && !empty.exists());
}

/// A copy's record never licenses removing its source, whatever phase a
/// damaged journal claims.
#[test]
fn a_copy_record_never_removes_the_original() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    edit_journal(&operation, |journal| {
        journal.insert("operation".into(), serde_json::json!("Copy"));
    });

    reconcile_unlocked(&cfg);
    assert!(source.join("payload.part").is_file());
    assert!(final_path.join("payload.part").is_file());
    assert!(hidden_folders(&source_base).is_empty());
}

/// A move another machine began names a source path that means something
/// else here. Report it; touch nothing.
#[test]
fn a_move_begun_on_another_machine_is_only_reported() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    edit_journal(&operation, |journal| {
        journal.insert(
            "host".into(),
            serde_json::json!("another-machine-for-fastf-tests"),
        );
        journal.insert(
            "machine".into(),
            serde_json::json!("0000000000000000another0machine"),
        );
    });

    let report = reconcile_unlocked(&cfg);
    assert!(
        report
            .unrecoverable
            .join("\n")
            .contains("another-machine-for-fastf-tests"),
        "{report:?}"
    );
    assert!(source.join("payload.part").is_file());
    assert!(operation.is_dir());
}

/// A power loss can keep the retire — a rename on the source's
/// filesystem — and lose the phases written after the publish. A retired
/// original beside a `ReadyToCommit` record with its destination there is
/// that, and it is finished, not reported as something fastf never does.
#[test]
fn a_publish_whose_later_phases_a_power_loss_took_back_is_finished() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, _final, mut transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::ReadyToCommit).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!retired.exists());
    assert!(hidden_folders(&source_base).is_empty());
}

/// A copy killed before it staged anything leaves only its record.
#[test]
fn a_copy_record_with_nothing_staged_is_cleared() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    write_project(&source_base, "project", "ID0001");
    let transaction = MoveTransaction::begin(
        &source_base,
        Path::new("project"),
        &target_base,
        Path::new("project"),
        "ID0001",
        Operation::Copy,
    )
    .unwrap();
    let operation = transaction.operation_dir.clone();

    let report = reconcile_unlocked(&cfg);
    assert!(report.unrecoverable.is_empty(), "{report:?}");
    assert!(!operation.exists());
    assert!(source_base.join("project/payload.part").is_file());
}

/// A copy made in its final place and killed before `PROJECT_INFO.md`
/// landed is fastf's own unfinished folder: not a project, and taken away
/// with the record. The original is untouched.
#[test]
fn an_in_place_copy_killed_before_its_publish_is_rolled_back() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let source = write_project(&source_base, "project", "ID0001");
    let transaction = MoveTransaction::begin(
        &source_base,
        Path::new("project"),
        &target_base,
        Path::new("project"),
        "ID0001",
        Operation::Move,
    )
    .unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();
    transaction.write_manifest(&manifest).unwrap();
    let staging = transaction.claim_staging().unwrap();
    assert_eq!(
        staging,
        target_base.join("project"),
        "made in its final place"
    );
    transactions::copy_to_staging(
        &manifest.without_root_metadata(),
        &source,
        &staging,
        &Mutex::new(Progress::new(&[])),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(staging.join("payload.part").is_file());
    assert!(
        !staging.join("PROJECT_INFO.md").exists(),
        "not a project yet"
    );

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.rolled_back, 1, "{report:?}");
    assert!(!staging.exists(), "the unfinished copy is gone");
    assert!(
        source.join("payload.part").is_file(),
        "the original is whole"
    );
    assert_eq!(
        fs::read_dir(transactions::transaction_root(&target_base))
            .unwrap()
            .count(),
        0
    );
}

/// Killed the instant after `PROJECT_INFO.md` landed, before the record
/// could say so: the copy is a project, so the move is finished from
/// there.
#[test]
fn an_in_place_copy_published_before_its_record_said_so_is_finished() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let source = write_project(&source_base, "project", "ID0001");
    let transaction = MoveTransaction::begin(
        &source_base,
        Path::new("project"),
        &target_base,
        Path::new("project"),
        "ID0001",
        Operation::Move,
    )
    .unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();
    transaction.write_manifest(&manifest).unwrap();
    let staging = transaction.claim_staging().unwrap();
    transactions::copy_to_staging(
        &manifest,
        &source,
        &staging,
        &Mutex::new(Progress::new(&[])),
        &AtomicBool::new(false),
    )
    .unwrap();

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!source.exists(), "the original left");
    assert!(staging.join("PROJECT_INFO.md").is_file());
    assert!(hidden_folders(&source_base).is_empty());
}

/// A record written by 3.12.0 on a cloud mount can say `Copying` about a
/// move that published and retired long ago: each later phase was a
/// rename the mount misplaced. A destination that holds the project's
/// `PROJECT_INFO.md` is published, whatever the record says, and with the
/// original gone the move is finished from there.
#[test]
fn a_stale_copying_record_of_a_published_move_is_finished() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    fs::remove_dir_all(&source).unwrap();
    // The record as the mount kept it: the first write, and nothing after.
    edit_journal(&operation, |journal| {
        journal.insert("phase".into(), serde_json::json!("Copying"));
    });
    for entry in fs::read_dir(&operation).unwrap().flatten() {
        if entry.file_name().to_string_lossy().starts_with("phase.") {
            fs::remove_file(entry.path()).unwrap();
        }
    }

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(final_path.join("PROJECT_INFO.md").is_file());
    assert!(!operation.exists());
}
