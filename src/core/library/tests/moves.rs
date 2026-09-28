//! A move between bases: the rename, the staged copy at every failpoint, and
//! what may stop one.

use super::*;

#[test]
fn move_project_round_trip() {
    // `move_project` takes `DataLock`, whose path is
    // `paths::install_dir().join(".fastf.lock")`. Without this the unit test
    // locks the developer's real data directory — blocking any `fastf` they
    // have open for the full 30-second timeout, and leaving a lock file behind.
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    // Extra content so the copy fallback path (if hit) is exercised on a tree.
    fs::create_dir_all(old_base.join("proj_a/assets")).unwrap();
    fs::write(old_base.join("proj_a/assets/raw_{x}.txt"), "keep {braces}").unwrap();

    let cfg = cfg_for(old_base, &[new_base]);
    let projects = discover(&cfg);
    assert_eq!(projects.len(), 1);

    let moved = move_project(&projects[0], new_base).unwrap();

    let new_canon = new_base.canonicalize().unwrap();
    assert_eq!(moved.base, new_canon);
    assert_eq!(moved.path, new_canon.join("proj_a"));
    assert!(moved.path.is_dir(), "moved folder should exist");
    assert!(!old_base.join("proj_a").exists(), "source should be gone");
    // Bytes untouched.
    assert_eq!(
        fs::read_to_string(moved.path.join("assets/raw_{x}.txt")).unwrap(),
        "keep {braces}"
    );
    // Metadata `path` patched — in the readable form, not the `\\?\`
    // verbatim one that `canonicalize` hands back on Windows.
    let meta = read_project_meta(&moved.path).unwrap();
    assert_eq!(meta.path, crate::util::paths::display_path(&moved.path));
    assert!(
        !meta.path.starts_with(r"\\?\"),
        "verbatim prefix leaked into metadata: {}",
        meta.path
    );
    assert_eq!(meta.id, "ID0001");
    // Caches on both sides are fresh.
    let old_cache = load_cache(&old_base.canonicalize().unwrap()).unwrap();
    assert!(old_cache.entries.iter().all(|e| e.dir != "proj_a"));
    let new_cache = load_cache(&new_canon).unwrap();
    assert!(new_cache.entries.iter().any(|e| e.dir == "proj_a"));
    // Discovery now finds it under the new base only.
    let after = discover(&cfg);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].base, new_canon);
}

#[test]
fn staged_move_copies_verifies_commits_and_removes_source() {
    // Exercises the cross-filesystem path directly (a same-fs test would take
    // the instant fs::rename fast path and never stage/verify).
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::create_dir_all(old_base.join("proj_a/assets")).unwrap();
    fs::create_dir_all(old_base.join("proj_a/empty")).unwrap();
    fs::write(old_base.join("proj_a/assets/big.bin"), vec![1u8; 8000]).unwrap();
    fs::write(old_base.join("proj_a/notes_{x}.md"), "keep {braces}").unwrap();
    fs::write(old_base.join("proj_a/real.tmp"), []).unwrap();
    fs::write(old_base.join("proj_a/real.part"), [0_u8, 1, 2, 255]).unwrap();

    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    staged_copy_verify_commit(&project, new_base, &new_path, &progress, &cancel).unwrap();

    // Progress must actually advance, through every step in order. The steps
    // and their counts are the only feedback during a multi-minute network
    // move, and a step that says nothing looks exactly like a hung one.
    crate::core::progress::settle(&progress, &cancel, &Ok(()));
    {
        use crate::core::assets::JobPhase;
        let p = progress.lock().unwrap();
        assert_eq!(p.phase, JobPhase::Done, "the phase should have advanced");
        let passed: Vec<JobPhase> = p.finished.iter().map(|step| step.phase).collect();
        assert_eq!(
            passed,
            crate::core::move_engine::MOVE_STEPS.to_vec(),
            "every step, in order"
        );
        let counted = |phase| {
            p.finished
                .iter()
                .find(|step| step.phase == phase)
                .map(|step| step.count)
                .unwrap()
        };
        let recorded = counted(JobPhase::Scanning);
        assert!(recorded >= 7, "the scan counts every entry: {recorded}");
        assert_eq!(
            counted(JobPhase::Copying),
            p.total_files - 1,
            "every file but the one the publish writes"
        );
        assert_eq!(counted(JobPhase::Verifying), recorded * 2 - 1);
        assert_eq!(
            counted(JobPhase::Removing),
            recorded,
            "the old copy's removal counts every entry it takes"
        );
        assert_eq!(p.status, crate::core::assets::JobStatus::Done);
        assert!(
            p.committed,
            "a published move is past its point of no return"
        );
        assert!(p.total_files >= 3, "files counted: {}", p.total_files);
        assert_eq!(
            p.done_files, p.total_files,
            "every copied file must be reported done"
        );
        assert!(p.copied_bytes >= 8000, "bytes copied: {}", p.copied_bytes);
    }

    // Copied verbatim, verified, committed, source removed.
    assert_eq!(
        fs::read(new_path.join("assets/big.bin")).unwrap(),
        vec![1u8; 8000]
    );
    assert_eq!(
        fs::read_to_string(new_path.join("notes_{x}.md")).unwrap(),
        "keep {braces}"
    );
    assert_eq!(
        fs::read(new_path.join("real.tmp")).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(
        fs::read(new_path.join("real.part")).unwrap(),
        [0_u8, 1, 2, 255]
    );
    assert!(new_path.join("empty").is_dir());
    assert!(
        !old_base.join("proj_a").exists(),
        "source removed only after verify"
    );
    assert_eq!(v2_transaction_count(new_base), 0);
}

#[test]
fn cancelled_staged_move_leaves_source_intact() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/data.bin"), vec![9u8; 4096]).unwrap();

    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    // Pre-cancelled → copy aborts on the first chunk.
    let cancel = AtomicBool::new(true);

    let err = staged_copy_verify_commit(&project, new_base, &new_path, &progress, &cancel)
        .unwrap_err()
        .to_string();
    assert!(err.contains("cancelled"), "err: {err}");
    assert!(
        old_base.join("proj_a").is_dir(),
        "source untouched on cancel"
    );
    assert!(!new_path.exists(), "no target committed");
    assert_eq!(v2_transaction_count(new_base), 0);
}

#[cfg(debug_assertions)]
#[test]
fn cleanup_failure_is_a_reported_success_and_retains_the_marker() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    write_project(old.path(), "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old.path().join("proj/payload.bin"), [0_u8, 1, 2, 255]).unwrap();
    let project = scan_base(old.path()).remove(0);
    let final_path = new.path().join("proj");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    let outcome = crate::util::faults::with_thread_fault("move:source-cleanup", || {
        staged_copy_verify_commit(&project, new.path(), &final_path, &progress, &cancel)
    })
    .expect("publication remains a successful move");

    assert!(
        matches!(outcome.source, SourceOutcome::KeptWhole { .. }),
        "{:?}",
        outcome.source
    );
    assert_eq!(
        fs::read(final_path.join("payload.bin")).unwrap(),
        [0_u8, 1, 2, 255]
    );
    assert!(project.path.is_dir(), "failed cleanup leaves source intact");
    assert_eq!(
        fs::read(project.path.join("payload.bin")).unwrap(),
        [0_u8, 1, 2, 255],
        "and whole"
    );
    assert!(
        retired_folders(old.path()).is_empty(),
        "a retire that failed leaves nothing retired"
    );
    assert_eq!(
        v2_transaction_count(new.path()),
        1,
        "cleanup-pending move must retain its v2 transaction"
    );
}

#[test]
fn conventional_v1_staging_and_marker_are_payload_not_move_authority() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    write_project(old.path(), "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let project = scan_base(old.path()).remove(0);
    let final_path = new.path().join("proj");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    // The v1 staging and marker names, written literally: fastf has no
    // writer for either format any more, and a v2 move must treat both as
    // ordinary payload it never reads, follows, or removes.
    let staging = new.path().join(".proj.fastf-part");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join("sentinel"), b"owned by someone else").unwrap();
    let marker = new.path().join(".fastf-move-proj.json");
    fs::write(&marker, b"foreign marker bytes").unwrap();
    let outcome =
        staged_copy_verify_commit(&project, new.path(), &final_path, &progress, &cancel).unwrap();
    assert!(!outcome.cleanup_pending());
    assert_eq!(
        fs::read(staging.join("sentinel")).unwrap(),
        b"owned by someone else"
    );
    assert_eq!(fs::read(marker).unwrap(), b"foreign marker bytes");
}

#[test]
fn only_the_cross_device_error_licenses_copy_fallback() {
    #[cfg(unix)]
    let cross_device = std::io::Error::from_raw_os_error(libc::EXDEV);
    #[cfg(windows)]
    let cross_device = std::io::Error::from_raw_os_error(17);

    assert!(is_cross_device_error(&cross_device));
    assert!(!is_cross_device_error(&std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied"
    )));
    assert!(!is_cross_device_error(&std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "missing"
    )));
}

/// The staged (copying) move carries a link as a link — and, removing the
/// original afterwards, never deletes through it.
///
/// A link to an asset library inside a project, a staged move, and the
/// original removed: were the removal to follow the link, the library's own
/// files would go with it. Reached through the private staged path because
/// the public entry point only stages after `fs::rename` fails, and a test
/// cannot conjure a second filesystem.
#[test]
fn a_staged_move_carries_a_link_and_never_deletes_through_it() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    // Join components separately: `join("proj_a/linked")` yields a
    // mixed-separator path on Windows (`...\proj_a/linked`), and `cmd` then
    // reads `/linked` as a switch.
    let link = base.join("proj_a").join("linked");
    let target = base.join("shared");
    fs::create_dir_all(&target).unwrap();

    // A silent skip here would be worse than no test: it reports "ok" while
    // asserting nothing. Junctions need no elevation on Windows and symlinks
    // work normally on Unix, so failing to create one is a real problem — say
    // so loudly rather than passing.
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .expect("running mklink");
        assert!(
            out.status.success(),
            "could not create a junction (needs no elevation on Windows):\n\
             stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).expect("creating a symlink");

    fs::write(target.join("payload.txt"), "irreplaceable").unwrap();
    let recorded_target = fs::read_link(&link).unwrap();

    let other = tempfile::tempdir().unwrap();
    let cfg = cfg_for(base, &[other.path()]);
    let project = discover(&cfg)
        .into_iter()
        .find(|found| found.name == "proj_a")
        .unwrap();
    let new_path = other.path().join("proj_a");
    let outcome = staged_copy_verify_commit(
        &project,
        other.path(),
        &new_path,
        &Mutex::new(Progress::new(&[])),
        &AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(outcome.source, SourceOutcome::Removed);
    assert_eq!(outcome.links, 1);
    assert!(!base.join("proj_a").exists());
    assert_eq!(
        fs::read_to_string(target.join("payload.txt")).unwrap(),
        "irreplaceable",
        "removing the original never went through the link"
    );
    let moved_link = new_path.join("linked");
    assert!(
        fs::symlink_metadata(&moved_link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "a link, not a copy of what is behind it"
    );
    assert_eq!(fs::read_link(&moved_link).unwrap(), recorded_target);
    assert_eq!(
        fs::read_to_string(moved_link.join("payload.txt")).unwrap(),
        "irreplaceable",
        "and it still leads there"
    );
    // Still nothing named for it: an absolute link to outside the project
    // points where it always did.
    assert!(outcome.notes.is_empty(), "{:?}", outcome.notes);
}

/// The move invariant, at every failpoint: the source is intact **or** the
/// destination is complete — never neither, and never a silent half-state.
///
/// The failure is injected rather than raced, so each boundary is hit
/// deterministically instead of "wherever the kill happened to land". These
/// go through the private staged path directly: a same-filesystem test would
/// take the instant `fs::rename` fast path and never stage or verify.
///
/// Debug-only: failpoints are compiled out of release builds.
#[cfg(debug_assertions)]
#[test]
fn interrupted_staged_move_never_loses_data_at_any_failpoint() {
    const MOVE_POINTS: &[&str] = &[
        "move:before-marker-write",
        "move:after-staging",
        "move:after-verify",
        "move:before-commit-rename",
        "move:after-publish-write",
        "move:after-commit-before-source-removal",
        "move:before-retire",
        "move:source-cleanup",
        "move:after-retire",
        "move:mid-gc",
    ];
    // Published, the original still whole at its path. A publish whose write
    // reported an error after its file landed is a publish too: rolling it
    // back would remove the project, clearing its record would leave both.
    const KEPT: &[&str] = &[
        "move:after-publish-write",
        "move:after-commit-before-source-removal",
        "move:before-retire",
        "move:source-cleanup",
    ];
    // Published, the original out of the library, its retired copy not gone.
    const RETIRED: &[&str] = &["move:after-retire", "move:mid-gc"];

    for point in MOVE_POINTS {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let (old_base, new_base) = (tmp1.path(), tmp2.path());
        write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::write(old_base.join("proj_a/payload.bin"), vec![7u8; 4096]).unwrap();

        let cfg = cfg_for(old_base, &[new_base]);
        let project = discover(&cfg).remove(0);
        let new_path = new_base.join("proj_a");
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        // Armed per-thread, so a move test running in parallel cannot see
        // this fault — an env var would fire inside every one of them.
        let result = crate::util::faults::with_thread_fault(point, || {
            staged_copy_verify_commit(&project, new_base, &new_path, &progress, &cancel)
        });

        if KEPT.contains(point) {
            assert!(
                result
                    .as_ref()
                    .is_ok_and(|outcome| matches!(outcome.source, SourceOutcome::KeptWhole { .. })),
                "[{point}] publication with the original kept whole: {result:?}"
            );
        } else if RETIRED.contains(point) {
            assert!(
                result.as_ref().is_ok_and(|outcome| matches!(
                    outcome.source,
                    SourceOutcome::Leftover {
                        redundant: true,
                        ..
                    }
                )),
                "[{point}] the original out of the library, a leftover reported: {result:?}"
            );
            assert!(
                !old_base.join("proj_a").exists(),
                "[{point}] no husk: the original left in one rename"
            );
            assert_eq!(retired_folders(old_base).len(), 1, "[{point}]");
        } else {
            assert!(result.is_err(), "[{point}] should have failed");
        }

        // The invariant. `after-commit-before-source-removal` is the one
        // point where the commit already landed, so the destination holds
        // the data and the (still-present) source is redundant.
        let source_ok = old_base.join("proj_a/payload.bin").is_file();
        let dest_ok = new_path.join("payload.bin").is_file();
        assert!(
            source_ok || dest_ok,
            "[{point}] data exists in neither location — this is data loss"
        );

        if KEPT.contains(point) || RETIRED.contains(point) {
            assert!(dest_ok, "[{point}] commit landed, destination must hold it");
        } else {
            assert!(
                source_ok,
                "[{point}] nothing was committed, so the source must be intact"
            );
            assert!(
                !new_path.exists(),
                "[{point}] an uncommitted move must leave no destination"
            );
        }

        // Whatever happened, reconcile must reach a consistent end state
        // with the payload still present exactly once.
        let report = provisioning::reconcile_unlocked(&cfg);
        let after_source = old_base.join("proj_a/payload.bin").is_file();
        let after_dest = new_path.join("payload.bin").is_file();
        assert!(
            after_source || after_dest,
            "[{point}] reconcile lost the data"
        );
        assert_eq!(
            v2_transaction_count(new_base),
            0,
            "[{point}] reconcile left a transaction behind: {report:?}"
        );
        assert!(
            retired_folders(old_base).is_empty(),
            "[{point}] reconcile left a retired original behind: {report:?}"
        );
        assert!(report.leftovers.is_empty(), "[{point}] {report:?}");
    }
}

/// **A removal stopped part of the way never leaves a husk** — here by a
/// read-only folder inside the project. A husk would still hold
/// `PROJECT_INFO.md` and be listed as the project, most of it gone; the
/// original leaves the library in one rename, and its retired copy is
/// removed after, read-only folder and all.
#[cfg(unix)]
#[test]
fn a_read_only_folder_inside_the_project_cannot_leave_a_husk() {
    use std::os::unix::fs::PermissionsExt;
    // Root writes into a read-only folder, so as root this proves nothing.
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let sealed = old_base.join("proj_a/aaa_modcache");
    fs::create_dir_all(&sealed).unwrap();
    fs::write(sealed.join("module.go"), "package x").unwrap();
    fs::write(old_base.join("proj_a/zzz_after.txt"), "after").unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o555)).unwrap();

    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    let outcome = staged_copy_verify_commit(
        &project,
        new_base,
        &new_path,
        &progress,
        &AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(outcome.source, SourceOutcome::Removed);
    assert!(!old_base.join("proj_a").exists(), "no husk at the old path");
    assert!(
        retired_folders(old_base).is_empty(),
        "and nothing left hidden"
    );
    assert_eq!(
        fs::read_to_string(new_path.join("aaa_modcache/module.go")).unwrap(),
        "package x"
    );
    assert_eq!(
        fs::read_to_string(new_path.join("zzz_after.txt")).unwrap(),
        "after"
    );
}

/// A base fastf cannot write in — a read-only mount, a share with read access
/// only — cannot give up the original after the copy. The move says so before
/// it copies anything, and leaves nothing behind on either side.
#[cfg(unix)]
#[test]
fn a_read_only_source_base_is_refused_before_anything_is_copied() {
    use std::os::unix::fs::PermissionsExt;
    // Root writes into a read-only folder, so as root this proves nothing.
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/payload.bin"), vec![3_u8; 4096]).unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    fs::set_permissions(old_base, fs::Permissions::from_mode(0o555)).unwrap();

    let progress = Mutex::new(Progress::new(&[]));
    let result = staged_copy_verify_commit(
        &project,
        new_base,
        &new_base.join("proj_a"),
        &progress,
        &AtomicBool::new(false),
    );
    fs::set_permissions(old_base, fs::Permissions::from_mode(0o755)).unwrap();

    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("needs to write in"), "{error}");
    assert!(error.contains("Nothing was copied"), "{error}");
    assert_eq!(progress.lock().unwrap().copied_bytes, 0, "not a byte");
    assert_eq!(v2_transaction_count(new_base), 0);
    assert!(!new_base.join("proj_a").exists());
    assert_eq!(
        fs::read(old_base.join("proj_a/payload.bin")).unwrap(),
        vec![3_u8; 4096]
    );
    assert!(retired_folders(old_base).is_empty());
}

/// A removal of the retired copy that stops part of the way leaves a hidden
/// leftover — not a project — and the next reconcile finishes it.
#[cfg(debug_assertions)]
#[test]
fn a_removal_stopped_part_way_leaves_only_a_hidden_leftover() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    for name in ["a.bin", "b.bin", "c.bin"] {
        fs::write(old_base.join("proj_a").join(name), name).unwrap();
    }
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));

    let outcome = crate::util::faults::with_thread_fault("move:mid-gc", || {
        staged_copy_verify_commit(
            &project,
            new_base,
            &new_path,
            &progress,
            &AtomicBool::new(false),
        )
    })
    .unwrap();

    let SourceOutcome::Leftover {
        path,
        redundant: true,
        ..
    } = &outcome.source
    else {
        panic!("expected a redundant leftover: {:?}", outcome.source);
    };
    assert!(path.is_dir(), "the retired copy is still there");
    assert!(!old_base.join("proj_a").exists());
    assert!(
        discover(&cfg)
            .iter()
            .all(|found| !found.path.starts_with(old_base)),
        "the leftover is never listed as a project"
    );
    assert!(
        outcome
            .project
            .path
            .starts_with(crate::util::paths::canonical(new_base).unwrap()),
        "the project is the moved copy: {}",
        outcome.project.path.display()
    );

    let report = provisioning::reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!path.exists());
    assert!(retired_folders(old_base).is_empty());
    assert_eq!(v2_transaction_count(new_base), 0);
    assert!(
        provisioning::reconcile_unlocked(&cfg).is_empty(),
        "and a second pass has nothing left to do"
    );
}

#[test]
fn move_project_rejects_same_base_and_collision() {
    let (_env, _sandbox) = crate::util::test_env::EnvGuard::sandbox();
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    write_project(
        tmp1.path(),
        "proj_a",
        "ID0001",
        "gen",
        "2026-01-01T00:00:00Z",
    );
    let cfg = cfg_for(tmp1.path(), &[tmp2.path()]);
    let project = discover(&cfg).remove(0);

    // Same base → bail.
    let err = move_project(&project, tmp1.path()).unwrap_err().to_string();
    assert!(err.contains("already in base"), "err: {err}");

    // Target name collision → bail, source untouched. (An empty folder
    // there gives way: it holds nothing to lose.)
    fs::create_dir_all(tmp2.path().join("proj_a")).unwrap();
    fs::write(tmp2.path().join("proj_a/theirs.txt"), "not ours").unwrap();
    let err = move_project(&project, tmp2.path()).unwrap_err().to_string();
    assert!(err.contains("already exists"), "err: {err}");
    assert!(project.path.is_dir(), "source must be untouched on bail");
}

/// A directory symbolic link crosses as a directory symbolic link — made with
/// the link's own directory attribute, since its target may not exist to ask.
/// Windows lets an account make one only with Developer Mode on; without it
/// the move names that before copying anything.
#[cfg(windows)]
#[test]
fn a_staged_move_carries_a_directory_symlink_or_names_developer_mode() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let link = old_base.join("proj_a").join("dangling_dir_link");
    match std::os::windows::fs::symlink_dir(r"..\nowhere", &link) {
        Ok(()) => {}
        // No Developer Mode here: the move's own refusal is what the test
        // below would meet too, and a symlink cannot be planted to prove it.
        Err(error) if error.raw_os_error() == Some(1314) => return,
        Err(error) => panic!("{error}"),
    }
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let outcome = staged_copy_verify_commit(
        &project,
        new_base,
        &new_path,
        &Mutex::new(Progress::new(&[])),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(outcome.source, SourceOutcome::Removed);
    let moved = new_path.join("dangling_dir_link");
    use std::os::windows::fs::FileTypeExt;
    assert!(
        fs::symlink_metadata(&moved)
            .unwrap()
            .file_type()
            .is_symlink_dir()
    );
    assert_eq!(fs::read_link(&moved).unwrap(), Path::new(r"..\nowhere"));
}

/// **A file a program holds open keeps the original whole.** Windows will not
/// rename a folder while anything in it is open without delete sharing — an
/// editor's project file, a video in a timeline. The move publishes, cannot
/// retire the original, says so, and removes nothing from it; once the file is
/// closed, reconcile finishes the move without copying again.
#[cfg(windows)]
#[test]
fn a_file_held_open_keeps_the_original_whole_until_reconcile() {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let held_path = old_base.join("proj_a").join("timeline.prproj");
    fs::write(&held_path, vec![5_u8; 4096]).unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");

    // Readable by others, never deletable: what an editor holding a file does.
    const FILE_SHARE_READ: u32 = 0x1;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&held_path)
        .unwrap();
    let outcome = staged_copy_verify_commit(
        &project,
        new_base,
        &new_path,
        &Mutex::new(Progress::new(&[])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let SourceOutcome::KeptWhole { reason } = &outcome.source else {
        panic!("expected the original kept whole: {:?}", outcome.source);
    };
    assert!(reason.contains("open"), "{reason}");
    assert_eq!(fs::read(&held_path).unwrap(), vec![5_u8; 4096], "whole");
    assert_eq!(
        fs::read(new_path.join("timeline.prproj")).unwrap(),
        vec![5_u8; 4096]
    );
    assert!(retired_folders(old_base).is_empty());

    drop(held);
    let report = crate::core::provisioning::reconcile_unlocked(&cfg);
    assert_eq!(report.completed, 1, "{report:?}");
    assert!(!old_base.join("proj_a").exists());
    assert_eq!(v2_transaction_count(new_base), 0);
}

/// **A program writing a file in the project stops the move, by name,
/// before anything is copied**: after the move its writes would land in the
/// old copy, which is then removed. One merely working in the folder does
/// not stop it; the result says to restart it from the new place.
#[cfg(all(target_os = "linux", debug_assertions))]
#[test]
fn a_writer_refuses_the_move_and_a_program_in_the_folder_is_a_note() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/session.txt"), "take 1\n").unwrap();
    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    let wait_for = |check: &dyn Fn() -> bool| {
        for _ in 0..100 {
            if check() {
                return;
            }
            sleep(Duration::from_millis(20));
        }
        panic!("waited two seconds for a program to show in the folder");
    };

    let mut writer = std::process::Command::new("sh")
        .arg("-c")
        .arg("exec 3>>proj_a/session.txt; exec sleep 30")
        .current_dir(old_base)
        .spawn()
        .unwrap();
    wait_for(&|| {
        !crate::core::holders::in_tree(&project.path)
            .blocking
            .is_empty()
    });
    let refused = staged_copy_verify_commit(
        &project,
        new_base,
        &new_path,
        &progress,
        &AtomicBool::new(false),
    );
    let _ = writer.kill();
    let _ = writer.wait();
    let error = format!("{:#}", refused.unwrap_err());
    assert!(
        error.contains("session.txt") && error.contains("open for writing"),
        "{error}"
    );
    assert!(error.contains("sleep"), "names the program: {error}");
    assert!(
        project.path.join("session.txt").is_file(),
        "nothing was touched"
    );
    assert!(!new_path.exists());

    let mut sitter = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(&project.path)
        .spawn()
        .unwrap();
    wait_for(&|| {
        !crate::core::holders::in_tree(&project.path)
            .working_in
            .is_empty()
    });
    let moved = staged_copy_verify_commit(
        &project,
        new_base,
        &new_path,
        &progress,
        &AtomicBool::new(false),
    );
    let _ = sitter.kill();
    let _ = sitter.wait();
    let moved = moved.unwrap();
    assert!(
        moved
            .notes
            .iter()
            .any(|note| note.contains("restart it from the new one")),
        "{:?}",
        moved.notes
    );
}
