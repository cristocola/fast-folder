//! Old copies no record names, and what only a person can settle.

use super::*;

/// A retired folder with no transaction anywhere is reported, never
/// removed — and never looked inside for a create to resume, though it may
/// well hold one.
#[test]
fn an_orphan_retired_folder_is_reported_and_never_resumed() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("base");
    let orphan = base.join(".fastf-moved-18d863aff116f53c-47fa4-8");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join(CREATE_JOURNAL_V2), b"{}").unwrap();
    fs::write(orphan.join("keep.txt"), b"keep").unwrap();
    let cfg = config_for(&base);

    let report = reconcile_unlocked(&cfg);
    assert_eq!(report.resumed, 0);
    assert!(report.unrecoverable.is_empty(), "{report:?}");
    assert_eq!(report.leftovers.len(), 1, "{report:?}");
    assert!(report.leftovers[0].contains("no record"), "{report:?}");
    assert_eq!(fs::read(orphan.join("keep.txt")).unwrap(), b"keep");
    assert!(
        list_incomplete(&cfg)
            .iter()
            .any(|item| item.kind == IncompleteKind::Leftover)
    );
}

/// **An old copy 3.13 left without a record is finished by content.**
/// Every entry the project holds the same, byte for byte, goes; what
/// differs stays, named. No record is needed: nothing removed exists
/// nowhere else.
#[test]
fn a_recordless_old_copy_goes_where_the_project_holds_the_same() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let moved = write_project(&target_base, "project", "ID0001");
    let old = recordless_copy_of(&moved, &source_base);

    let report = reconcile_unlocked(&cfg);
    assert!(!old.exists(), "{report:?}");
    assert_eq!(report.cleared, 1, "{report:?}");
    assert!(
        moved.join("payload.part").is_file(),
        "the project is untouched"
    );

    let old = recordless_copy_of(&moved, &source_base);
    fs::write(old.join("payload.part"), b"edited only here").unwrap();
    fs::write(old.join("extra.txt"), b"only here too").unwrap();
    let report = reconcile_unlocked(&cfg);
    let said = report.leftovers.join("\n");
    assert!(
        said.contains("payload.part") && said.contains("extra.txt"),
        "{said}"
    );
    assert_eq!(
        fs::read(old.join("payload.part")).unwrap(),
        b"edited only here"
    );
    assert!(old.join("extra.txt").is_file());
    assert!(!old.join("empty").exists(), "what the project holds went");
    assert!(
        !old.join(crate::core::project_info::RESERVED_FILENAME)
            .exists(),
        "its PROJECT_INFO.md is the project's but for its place"
    );
}

/// An empty folder a cloud mount put back after an old copy was removed
/// holds nothing to lose, and goes; an old copy whose project fastf
/// cannot find is kept whole, and says so.
#[test]
fn a_recordless_marker_goes_and_an_unknown_projects_copy_stays() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let marker = source_base.join(".fastf-moved-18d8e4375a9294cf-10a6c5-0");
    fs::create_dir_all(marker.join("nested/empty")).unwrap();
    let elsewhere = temp.path().join("elsewhere");
    let moved = write_project(&elsewhere, "project", "ID0042");
    let orphan = recordless_copy_of(&moved, &source_base);
    let _ = target_base;

    let report = reconcile_unlocked(&cfg);
    assert!(!marker.exists(), "{report:?}");
    assert!(orphan.join("payload.part").is_file(), "kept whole");
    let said = report.leftovers.join("\n");
    assert!(said.contains("cannot find the project"), "{said}");
}

/// An in-place retire's pointer outlives its record only when a pass
/// stopped between the two: with its folder gone it is removed; with its
/// folder a project again, it names nothing to remove and goes too.
#[test]
fn a_pointer_without_its_record_goes() {
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let pointer = transactions::RetirePointer {
        version: 1,
        operation: "18d8e2f16082c791-e6a94-0".to_string(),
        project_id: "ID0001".to_string(),
        folder: PathBuf::from("gone"),
        target_base: target_base.clone(),
        target_folder: PathBuf::from("gone"),
    };
    let path = transactions::pointer_path(&source_base, &pointer.operation);
    fs::write(&path, serde_json::to_vec(&pointer).unwrap()).unwrap();

    let report = reconcile_unlocked(&cfg);
    assert!(!path.exists(), "{report:?}");

    write_project(&source_base, "gone", "ID0001");
    fs::write(&path, serde_json::to_vec(&pointer).unwrap()).unwrap();
    let report = reconcile_unlocked(&cfg);
    assert!(!path.exists(), "{report:?}");
    assert!(
        source_base.join("gone/payload.part").is_file(),
        "the project stays"
    );
}

/// **A conflict needs you until a version is chosen**, and then nothing
/// does: keeping the moved copy's version removes the old copy; taking
/// the old copy's puts it into the moved one first. Either way the move
/// finishes, and attention is empty.
#[test]
fn a_conflict_needs_you_until_a_version_is_chosen() {
    use crate::core::attention::{Action, State, attention, resolve};
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    for keep in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let (source_base, target_base, cfg) = bases(temp.path());
        let (source, final_path, mut transaction) =
            published_awaiting_cleanup(&source_base, &target_base);
        let retired = transaction.retired_path();
        fs::rename(&source, &retired).unwrap();
        transaction.set_phase(MovePhase::Retired).unwrap();
        fs::write(retired.join("payload.part"), b"the old side").unwrap();
        fs::write(final_path.join("payload.part"), b"the moved side").unwrap();

        let report = reconcile_unlocked(&cfg);
        assert_eq!(report.completed, 0, "{report:?}");
        let now = attention(&cfg);
        let item = now
            .items
            .iter()
            .find(|item| item.state == State::NeedsYou)
            .unwrap_or_else(|| panic!("{now:?}"));
        assert_eq!(item.path, retired);
        assert_eq!(item.actions, vec![Action::KeepMoved, Action::TakeOld]);
        assert_eq!(now.needs_you(), 1);

        let action = if keep {
            Action::KeepMoved
        } else {
            Action::TakeOld
        };
        resolve(&cfg, &retired, action).unwrap();
        assert!(!retired.exists());
        assert_eq!(
            fs::read(final_path.join("payload.part")).unwrap(),
            if keep {
                b"the moved side".to_vec()
            } else {
                b"the old side".to_vec()
            }
        );
        assert!(!transaction.operation_dir.exists(), "the move is finished");
        let after = attention(&cfg);
        assert!(after.items.is_empty(), "{after:?}");
    }
}

/// An old copy with no record whose project fastf cannot find needs you:
/// put it back as the project, or discard it.
#[test]
fn an_old_copy_nobody_can_place_is_put_back() {
    use crate::core::attention::{Action, State, attention, resolve};
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let temp = tempfile::tempdir().unwrap();
    let (source_base, _target_base, cfg) = bases(temp.path());
    let elsewhere = temp.path().join("elsewhere");
    let moved = write_project(&elsewhere, "project", "ID0042");
    // The list holds the bases' spelling of a path: on Windows, verbatim.
    let old = crate::util::paths::canonical(&recordless_copy_of(&moved, &source_base)).unwrap();
    fs::remove_dir_all(&elsewhere).unwrap();

    reconcile_unlocked(&cfg);
    let now = attention(&cfg);
    let item = now
        .items
        .iter()
        .find(|item| item.path == old)
        .unwrap_or_else(|| panic!("{now:?}"));
    assert_eq!(item.state, State::NeedsYou);
    assert_eq!(item.actions, vec![Action::PutBack, Action::Discard]);

    resolve(&cfg, &old, Action::PutBack).unwrap();
    assert!(!old.exists());
    assert!(source_base.join("project/payload.part").is_file());
    assert!(
        crate::core::library::discover(&cfg)
            .iter()
            .any(|project| project.id == "ID0042"),
        "it is the project again"
    );
    assert!(attention(&cfg).items.is_empty());
}

/// A move begun on another machine needs you, and discarding it removes
/// its record and nothing else.
#[test]
fn a_move_from_another_machine_is_discarded_record_only() {
    use crate::core::attention::{Action, State, attention, resolve};
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let (source, final_path, transaction) = published_awaiting_cleanup(&source_base, &target_base);
    let operation = crate::util::paths::canonical(&transaction.operation_dir).unwrap();
    edit_journal(&operation, |journal| {
        journal.insert("host".into(), serde_json::json!("elsewhere"));
        journal.insert("machine".into(), serde_json::json!("0000another0machine"));
    });
    let now = attention(&cfg);
    let item = now
        .items
        .iter()
        .find(|item| item.path == operation)
        .unwrap_or_else(|| panic!("{now:?}"));
    assert_eq!(item.state, State::NeedsYou);
    assert!(item.reason.contains("elsewhere"), "{}", item.reason);
    assert!(
        resolve(&cfg, &operation, Action::PutBack).is_err(),
        "not offered"
    );
    resolve(&cfg, &operation, Action::Discard).unwrap();
    assert!(!operation.exists());
    assert!(
        source.join("payload.part").is_file(),
        "the original is untouched"
    );
    assert!(final_path.is_dir(), "and the moved copy");
}

/// An old copy waiting out the settle is a quiet waiting item — not
/// counted in the header — until its time is up; then it is fastf's to
/// finish, so the app's own reconcile clears the record.
#[test]
fn a_settling_old_copy_waits_quietly_then_is_fastfs_to_finish() {
    use crate::core::attention::{A_SETTLE, State, attention};
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    let source_base = crate::util::paths::canonical(&source_base).unwrap();
    let operation = "18d8f68b4e22da23-378707-0".to_string();
    let mut entry = crate::core::records::Entry {
        operation: operation.clone(),
        kind: "move".to_string(),
        project_id: "ID0007".to_string(),
        record: target_base.join(".fastf-transactions").join(&operation),
        source_base: source_base.clone(),
        source_folder: PathBuf::from("2026-01-01_Cloud_ID0007"),
        gone_at: Some(crate::util::time::now_unix() - 60),
        ..crate::core::records::Entry::default()
    };
    crate::core::records::add(&entry);
    let now = attention(&cfg);
    let item = now
        .items
        .iter()
        .find(|item| item.what == A_SETTLE)
        .unwrap_or_else(|| panic!("{now:?}"));
    assert_eq!(item.state, State::Waiting);
    assert!(item.reason.contains("looks again in"), "{}", item.reason);
    assert_eq!(now.waiting_work(), 0, "nothing for the header to count");

    entry.gone_at = Some(crate::util::time::now_unix() - crate::core::records::SETTLE_SECS - 1);
    crate::core::records::add(&entry);
    let later = attention(&cfg);
    let item = later
        .items
        .iter()
        .find(|item| item.what == A_SETTLE)
        .unwrap_or_else(|| panic!("{later:?}"));
    assert_eq!(item.state, State::Auto, "its time is up: fastf's to finish");
    assert_eq!(later.auto(), 1);
}

/// An interrupted move fastf can finish is not a person's business.
#[test]
fn an_interrupted_move_is_finished_by_fastf() {
    use crate::core::attention::{State, attention};
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let temp = tempfile::tempdir().unwrap();
    let (source_base, target_base, cfg) = bases(temp.path());
    published_awaiting_cleanup(&source_base, &target_base);
    let now = attention(&cfg);
    assert_eq!(now.items.len(), 1, "{now:?}");
    assert_eq!(now.items[0].state, State::Auto);
    assert_eq!(now.auto(), 1);
    assert_eq!(now.needs_you(), 0);
}
