//! The pass itself: what it lists, counts, clears and leaves byte for byte.

use super::*;

#[test]
fn obsolete_markers_are_byte_identical_after_reconcile() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let project = base.join("project");
    let outside = temp.path().join("outside-sentinel");
    fs::create_dir_all(&project).unwrap();
    fs::write(&outside, b"untouched").unwrap();
    let hostile = format!(
        "{{\"version\":1,\"src\":\"{}\",\"temp\":\"{}\",\"final_path\":\"{}\"}}",
        outside.display(),
        outside.display(),
        outside.display()
    );
    let create = legacy_create_marker_path(&project);
    let moved = base.join(format!("{MARKER_MOVE_PREFIX}project.json"));
    fs::write(&create, hostile.as_bytes()).unwrap();
    fs::write(&moved, hostile.as_bytes()).unwrap();
    let before_create = fs::read(&create).unwrap();
    let before_move = fs::read(&moved).unwrap();

    let first = reconcile_unlocked(&config_for(&base));
    let second = reconcile_unlocked(&config_for(&base));
    assert_eq!(first.obsolete.len(), 2);
    assert_eq!(second.obsolete.len(), 2);
    assert_eq!(fs::read(create).unwrap(), before_create);
    assert_eq!(fs::read(moved).unwrap(), before_move);
    assert_eq!(fs::read(outside).unwrap(), b"untouched");
}

#[test]
fn create_journal_never_serializes_absolute_copy_paths() {
    let temp = tempfile::tempdir().unwrap();
    let template_files = temp.path().join("template/files");
    let project = temp.path().join("base/project");
    fs::create_dir_all(&template_files).unwrap();
    fs::create_dir_all(&project).unwrap();
    let source = template_files.join("nested/asset.bin");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, b"payload").unwrap();
    let job = CopyJob {
        src: source,
        dest: project.join("nested/asset.bin"),
        bytes: 7,
    };
    write_create_journal(
        &project,
        "general",
        &template_files,
        std::slice::from_ref(&job),
    )
    .unwrap();
    let raw = fs::read_to_string(create_journal_path(&project)).unwrap();
    assert!(!raw.contains(&temp.path().display().to_string()));
    assert!(raw.contains("nested/asset.bin"));
}

/// A move's record in a base since dropped from `bases` is still found,
/// through the data dir's index, and its old copy is finished — not
/// reported as having "no record of the move left".
#[test]
fn a_record_in_a_base_no_longer_configured_is_found_through_the_index() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, _) = bases(temp.path());
    let (source, final_path, mut transaction) =
        published_awaiting_cleanup(&source_base, &target_base);
    let operation = transaction.operation_dir.clone();
    crate::core::records::add(&crate::core::records::Entry {
        operation: transaction.journal.operation_id.clone(),
        kind: "move".to_string(),
        record: operation.clone(),
        source_base: source_base.clone(),
        target_base: target_base.clone(),
        ..Default::default()
    });
    let retired = transaction.retired_path();
    fs::rename(&source, &retired).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();

    let report = reconcile_unlocked(&config_for(&source_base));
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(report.leftovers.is_empty(), "{report:?}");
    assert!(!retired.exists());
    assert!(!operation.exists());
    assert!(final_path.join("payload.part").is_file());
}

/// A move killed while probing its source base leaves the probe; the next
/// pass clears it, and it clears nothing that is not a probe's.
#[test]
fn a_probe_a_killed_move_left_is_cleared() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let probe = crate::core::move_preflight::probe_path(&base, "18d863aff116f53c-1-3");
    fs::create_dir_all(&probe).unwrap();
    fs::write(probe.join("f"), b"fastf").unwrap();
    let cfg = config_for(&base);
    assert!(
        list_incomplete(&cfg)
            .iter()
            .any(|item| item.kind == IncompleteKind::Leftover)
    );

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.cleared, 1, "{report:?}");
    assert!(!probe.exists());
}

/// A project named like one of fastf's hidden folders, caught mid-way
/// through a case-only rename, is put back — never taken for a deleted
/// project's leftover and removed.
#[test]
fn a_project_named_like_a_hidden_folder_is_restored_not_removed() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let stranded = base.join(".fastf-deleted-Scenes.fastf-case");
    write_project(&base, ".fastf-deleted-Scenes.fastf-case", "ID0042");
    let cfg = config_for(&base);

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.restored, 1, "{report:?}");
    assert_eq!(report.cleared, 0, "{report:?}");
    assert!(!stranded.exists());
    assert!(base.join("fastf-deleted-Scenes/payload.part").is_file());
}

/// **Finishing a case-only rename waits for a folder held for a moment**, as
/// the rename itself does: Windows renames no folder while a file in it is
/// open, and 0.8 s is past the file rename's schedule and inside the folder
/// one.
#[cfg(windows)]
#[test]
fn a_stranded_rename_waits_for_a_folder_held_for_a_moment() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let stranded = write_project(&base, ".ALBUM.fastf-case", "ID0042");
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(stranded.join("payload.part"))
        .unwrap();
    let letting_go = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        drop(held);
    });

    let report = reconcile_unlocked(&config_for(&base));
    letting_go.join().unwrap();

    assert_eq!(report.restored, 1, "{report:?}");
    assert!(report.unrecoverable.is_empty(), "{report:?}");
    assert!(base.join("ALBUM/payload.part").is_file());
    assert!(!stranded.exists());
}

/// A deleted project's hidden folder is finished by the next pass.
#[test]
fn a_deleted_projects_hidden_folder_is_cleared() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let deleted = base.join(".fastf-deleted-18d863aff116f53c-47fa4-9");
    fs::create_dir_all(deleted.join("sub")).unwrap();
    fs::write(deleted.join("sub/file"), b"x").unwrap();

    let report = reconcile_unlocked(&config_for(&base));
    assert_eq!(report.cleared, 1, "{report:?}");
    assert!(!deleted.exists());
}

/// A reconcile counts what the header counted as needing attention, names
/// each item as it takes it, and counts what each removal takes.
#[test]
fn a_reconcile_counts_its_items_and_what_each_removes() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    deleted_folder(&base, "18d863aff116f53c-47fa4-9", 3);
    deleted_folder(&base, "18d863aff116f53c-47fa4-a", 2);
    let cfg = config_for(&base);
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    let report = reconcile_unlocked_with(&cfg, Ticker::new(&progress, &cancel));

    assert_eq!(report.cleared, 2, "{report:?}");
    let state = progress.lock().unwrap().clone();
    assert_eq!((state.item, state.items), (2, 2));
    assert_eq!(state.item_label, "a deleted project's folder");
    assert_eq!(state.phase, JobPhase::Removing);
    assert!(state.step_done >= 3, "the last removal counted: {state:?}");
}

/// A cancel before a pass reaches an item leaves it exactly as it was, and
/// the report says the pass stopped — never that there was nothing to do.
#[test]
fn a_cancelled_reconcile_stops_between_items() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let first = deleted_folder(&base, "18d863aff116f53c-47fa4-9", 3);
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(true);

    let report = reconcile_unlocked_with(&config_for(&base), Ticker::new(&progress, &cancel));

    assert!(report.cancelled);
    assert!(!report.is_empty(), "a stopped pass is not a clean one");
    assert_eq!(report.cleared, 0);
    assert_eq!(fs::read_dir(first.join("sub")).unwrap().count(), 3);
}

/// A cancel mid-removal stops it where it is. What is left is a leftover
/// the report names, and the next pass finishes it.
#[cfg(debug_assertions)]
#[test]
fn a_cancel_stops_a_removal_and_the_next_pass_finishes_it() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let deleted = deleted_folder(&base, "18d863aff116f53c-47fa4-9", 40);
    let cfg = config_for(&base);
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    let report = std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..2500 {
                if progress.lock().unwrap().step_done >= 3 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        crate::util::faults::with_thread_fault("pool:serial,remove:each-entry:delay-10", || {
            reconcile_unlocked_with(&cfg, Ticker::new(&progress, &cancel))
        })
    });

    assert_eq!(report.cleared, 0, "{report:?}");
    assert_eq!(report.leftovers.len(), 1, "{report:?}");
    assert!(deleted.exists(), "stopped part of the way");

    let finished = reconcile_unlocked(&cfg);
    assert_eq!(finished.cleared, 1, "{finished:?}");
    assert!(!deleted.exists());
}

/// These names are on disk in journals another build has to read, so the
/// enum must serialize to exactly these strings.
#[test]
fn incomplete_kinds_serialize_to_their_documented_names() {
    use super::IncompleteKind;

    for (value, name) in [
        (IncompleteKind::Create, "create"),
        (IncompleteKind::Move, "move"),
        (IncompleteKind::ObsoleteCreateV1, "obsolete-create-v1"),
        (IncompleteKind::ObsoleteMoveV1, "obsolete-move-v1"),
        (IncompleteKind::CreateV2Invalid, "create-v2-invalid"),
        (IncompleteKind::MoveV2Invalid, "move-v2-invalid"),
    ] {
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            format!("\"{name}\"")
        );
    }
}
