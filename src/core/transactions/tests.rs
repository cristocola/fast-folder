use super::*;

/// A folder on another device is another filesystem — unless the root
/// moved with it, which is a mount that came back.
#[test]
fn a_remounted_root_is_the_same_filesystem_a_nested_mount_is_not() {
    let root = Path::new("/mnt/projects/project");
    let remounted = RootDevice::recorded(root, Some(1), |_| Some(2));
    assert!(!remounted.elsewhere_than(Some(1)));
    assert!(
        !remounted.elsewhere_than(Some(2)),
        "the root is on 2 now as well: the same filesystem, remounted"
    );
    assert!(!remounted.elsewhere_than(Some(2)));
    let nested = RootDevice::recorded(root, Some(1), |_| Some(1));
    assert!(
        nested.elsewhere_than(Some(3)),
        "a filesystem mounted inside"
    );
    let gone = RootDevice::recorded(root, Some(1), |_| None);
    assert!(gone.elsewhere_than(Some(3)), "a root that does not answer");
    let unknown = RootDevice::recorded(root, None, |_| None);
    assert!(!unknown.elsewhere_than(Some(3)), "no device to compare");
}

#[test]
fn manifest_preserves_exact_topology_and_source_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let staging = temp.path().join("staging");
    fs::create_dir_all(source.join("empty")).unwrap();
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("zero.tmp"), []).unwrap();
    fs::write(source.join("nested/data.part"), [0_u8, 255, 7]).unwrap();
    // A move never interpolates: literal braces survive in names and bytes.
    fs::write(source.join("notes_{client}.md"), "hello {name}").unwrap();
    fs::create_dir(&staging).unwrap();

    let manifest = MoveManifest::scan(&source).unwrap();
    let progress = Mutex::new(Progress::new(&[]));
    copy_to_staging(
        &manifest,
        &source,
        &staging,
        &progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    manifest.verify_destination(&staging).unwrap();
    manifest.verify_source_unchanged(&source).unwrap();
    assert!(staging.join("empty").is_dir());
    assert_eq!(fs::read(staging.join("zero.tmp")).unwrap(), b"");
    assert_eq!(
        fs::read(staging.join("nested/data.part")).unwrap(),
        [0_u8, 255, 7]
    );
    assert_eq!(
        fs::read_to_string(staging.join("notes_{client}.md")).unwrap(),
        "hello {name}"
    );
}

/// Create a directory link inside a test tree, cross-platform. Windows
/// junctions need no elevation (unlike symlinks, which want Developer
/// Mode), so `mklink /J` is the portable-enough choice there. Returns
/// `false` when the OS refused, so a test can skip rather than fail on a
/// machine with restrictive policy.
fn make_dir_link(link: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

/// The data-loss regression, now guarded where the invariant lives. A
/// junction inside a project was once invisible to the walk, so a staged
/// move copied around it, verification walked the same blind way and
/// reported success, and the source was deleted. A link is now recorded by
/// its target — never followed, never silently omitted — so what is behind
/// it is neither copied nor, when the original is removed, deleted.
#[test]
fn scan_records_a_link_by_its_target_and_never_follows_it() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("real_asset_library");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("payload.txt"), "irreplaceable").unwrap();

    let source = temp.path().join("project");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("normal.txt"), "ordinary").unwrap();
    if !make_dir_link(&source.join("linked"), &target) {
        eprintln!("skipping: OS refused to create a directory link");
        return;
    }

    let manifest = MoveManifest::scan(&source).unwrap();
    let linked = manifest.entry(Path::new("linked")).expect("recorded");
    assert!(linked.kind.is_link(), "{linked:?}");
    #[cfg(unix)]
    assert_eq!(linked.kind, ManifestKind::Symlink);
    #[cfg(windows)]
    assert_eq!(linked.kind, ManifestKind::Junction);
    assert_eq!(
        fs::read_link(source.join("linked")).ok(),
        linked.link_target.clone()
    );
    assert!(
        manifest.entries.iter().all(|entry| !entry
            .path
            .parent()
            .is_some_and(|parent| parent.starts_with("linked"))),
        "nothing behind the link is walked: {manifest:?}"
    );
}

/// Links cross as links — relative, absolute, dangling — and verification
/// compares their target text: a retargeted link, or a file where a link
/// was, is a copy that does not match.
#[cfg(unix)]
#[test]
fn links_are_copied_as_links_and_verified_by_their_target() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let staging = temp.path().join("staging");
    fs::create_dir_all(source.join("node_modules/.bin")).unwrap();
    fs::write(source.join("real.txt"), "real").unwrap();
    let links = [
        ("node_modules/.bin/vite", "../vite/bin/vite.js"),
        ("to_real", "real.txt"),
        ("absolute", "/nonexistent/fastf/test/target"),
    ];
    for (link, target) in links {
        std::os::unix::fs::symlink(target, source.join(link)).unwrap();
    }
    fs::create_dir(&staging).unwrap();

    let manifest = MoveManifest::scan(&source).unwrap();
    assert_eq!(manifest.total_links(), 3);
    copy_to_staging(
        &manifest,
        &source,
        &staging,
        &Mutex::new(Progress::new(&[])),
        &AtomicBool::new(false),
    )
    .unwrap();
    manifest.verify_destination(&staging).unwrap();
    for (link, target) in links {
        assert_eq!(
            fs::read_link(staging.join(link)).unwrap(),
            Path::new(target)
        );
    }

    fs::remove_file(staging.join("to_real")).unwrap();
    std::os::unix::fs::symlink("elsewhere.txt", staging.join("to_real")).unwrap();
    let error = format!("{:#}", manifest.verify_destination(&staging).unwrap_err());
    assert!(
        error.contains("to_real: a link to elsewhere.txt now, was a link to real.txt"),
        "{error}"
    );
    fs::remove_file(staging.join("to_real")).unwrap();
    fs::write(staging.join("to_real"), "real").unwrap();
    let error = format!("{:#}", manifest.verify_destination(&staging).unwrap_err());
    assert!(error.contains("to_real: a 4-byte file now"), "{error}");
}

/// A link is kept exactly, so one that climbs out of the project, or names
/// the original folder by its full path, may mean something else after the
/// move — and the outcome says which.
#[test]
fn links_whose_meaning_the_new_place_may_change_are_named() {
    let original = Path::new("/projects/base/Album_ID0001");
    let link = |path: &str, target: &str| ManifestEntry {
        path: PathBuf::from(path),
        kind: ManifestKind::Symlink,
        bytes: 0,
        source_modified: ModifiedTime::from_system_time(UNIX_EPOCH),
        link_target: Some(PathBuf::from(target)),
    };
    let manifest = MoveManifest {
        version: MANIFEST_VERSION,
        entries: vec![
            link("inside", "sub/file"),
            link("sub/sibling", "../other"),
            link("sub/up_and_out", "../../shared"),
            link("out", "../shared_assets"),
            link("self", "/projects/base/Album_ID0001/sub/file"),
            link("elsewhere", "/mnt/library/stock"),
        ],
    };
    let notes = manifest.link_notes(original).join("\n");
    assert!(
        !notes.contains("inside") && !notes.contains("sub/sibling"),
        "{notes}"
    );
    assert!(notes.contains("sub/up_and_out points outside"), "{notes}");
    assert!(notes.contains("out points outside"), "{notes}");
    assert!(
        notes.contains("self points into the original folder"),
        "{notes}"
    );
    assert!(
        !notes.contains("elsewhere"),
        "an absolute link elsewhere still works: {notes}"
    );
}

/// sshfs's default `contain_symlinks` hands back `EPERM` for every link
/// that climbs with `..`; the refusal names the option that lifts it.
#[test]
fn a_link_the_mount_will_not_read_names_the_option() {
    let problem = Problem::LinkNotReadable("Operation not permitted (os error 1)".into());
    let said = problem.to_string();
    assert!(said.contains("no_contain_symlinks"), "{said}");
    assert!(said.contains("Operation not permitted"), "{said}");
}

#[cfg(unix)]
#[test]
fn a_refused_link_is_said_in_words() {
    let refusal = link_refusal(&std::io::Error::from_raw_os_error(libc::EPERM));
    assert!(refusal.contains("cannot hold links"), "{refusal}");
    // What an rclone mount answers without `--links`, found on a real one.
    let refusal = link_refusal(&std::io::Error::from_raw_os_error(libc::EIO));
    assert!(refusal.contains("--links"), "{refusal}");
}

/// A missing source is an error, never an empty manifest that would verify
/// against an empty destination.
#[test]
fn scan_of_a_missing_tree_is_an_error() {
    let temp = tempfile::tempdir().unwrap();
    assert!(MoveManifest::scan(&temp.path().join("missing")).is_err());
}

/// Verification is what stands between a move and deleting a good source,
/// so it has to catch the real network-share failure modes: a truncated
/// file and a dropped one.
#[test]
fn verify_destination_detects_short_and_missing_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let staging = temp.path().join("staging");
    fs::create_dir_all(source.join("sub")).unwrap();
    fs::write(source.join("a.txt"), "hello").unwrap();
    fs::write(source.join("sub/b.bin"), vec![0_u8; 2048]).unwrap();
    fs::create_dir(&staging).unwrap();

    let manifest = MoveManifest::scan(&source).unwrap();
    let progress = Mutex::new(Progress::new(&[]));
    copy_to_staging(
        &manifest,
        &source,
        &staging,
        &progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    manifest.verify_destination(&staging).unwrap();

    // Truncated at the destination.
    fs::write(staging.join("sub/b.bin"), vec![0_u8; 1024]).unwrap();
    let said = format!("{:#}", manifest.verify_destination(&staging).unwrap_err());
    assert!(said.contains("1 changed"), "{said}");
    assert!(said.contains("b.bin: 1024 bytes now, was 2048"), "{said}");
    fs::write(staging.join("sub/b.bin"), vec![0_u8; 2048]).unwrap();
    manifest.verify_destination(&staging).unwrap();

    // Dropped at the destination.
    fs::remove_file(staging.join("a.txt")).unwrap();
    let said = format!("{:#}", manifest.verify_destination(&staging).unwrap_err());
    assert!(
        said.contains("1 of the 3 recorded entries missing"),
        "{said}"
    );
    assert!(said.contains("a.txt: missing"), "{said}");
}

#[test]
fn source_metadata_changes_are_detected_after_copy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"one").unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();
    fs::write(source.join("file"), b"two-two").unwrap();
    let said = format!(
        "{:#}",
        manifest.verify_source_unchanged(&source).unwrap_err()
    );
    assert!(said.contains("file: 7 bytes now, was 3"), "{said}");
}

#[test]
fn transaction_journal_derives_target_and_staging_from_location() {
    let temp = tempfile::tempdir().unwrap();
    let source_base = temp.path().join("source-base");
    let target_base = temp.path().join("target-base");
    fs::create_dir(&source_base).unwrap();
    fs::create_dir(&target_base).unwrap();
    let transaction = MoveTransaction::begin(
        &source_base,
        Path::new("project"),
        &target_base,
        Path::new("project"),
        "ID0001",
        Operation::Move,
    )
    .unwrap();
    assert_eq!(transaction.final_path(), target_base.join("project"));
    assert_eq!(
        transaction.staging_path(),
        transaction.final_path(),
        "a copy is made in its final place"
    );
    assert_eq!(transaction.old_staging_path(), None);
    let raw = fs::read_to_string(transaction.operation_dir.join(JOURNAL_FILE)).unwrap();
    assert!(!raw.contains("staging"));
    assert!(!raw.contains(&target_base.display().to_string()));
}

/// Running as root, a permission test proves nothing: root reads a
/// mode-000 folder. The install lab's containers run as root.
#[cfg(unix)]
fn running_as_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// A tree with several things a move cannot take used to be refused one
/// name at a time: fix the first, run again, meet the second.
#[cfg(unix)]
#[test]
fn a_scan_names_every_problem_not_just_the_first() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir_all(source.join("run")).unwrap();
    fs::write(source.join("notes.md"), "ordinary").unwrap();
    let fifo = std::ffi::CString::new(
        source
            .join("run/pipe")
            .as_os_str()
            .as_encoded_bytes()
            .to_vec(),
    )
    .unwrap();
    // SAFETY: a valid NUL-terminated path and a plain mode.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let _socket = std::os::unix::net::UnixListener::bind(source.join("run/app.sock")).unwrap();

    let error = MoveManifest::scan(&source).unwrap_err().to_string();
    assert!(error.contains("2 entries"), "{error}");
    assert!(
        error.contains("run/pipe") && error.contains("run/app.sock"),
        "{error}"
    );
    assert!(error.contains("socket, pipe or device"), "{error}");
}

/// The incident's first message was `classifying …: No such file or
/// directory` — true, and no help. An entry the folder lists but `lstat`
/// cannot examine is now named as exactly that, and a folder that cannot
/// be listed at all is named as that.
#[cfg(unix)]
#[test]
fn entries_that_cannot_be_examined_or_listed_are_named_as_such() {
    if running_as_root() {
        eprintln!("skipping: root can examine anything");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir_all(source.join("listed_only")).unwrap();
    fs::create_dir_all(source.join("sealed")).unwrap();
    fs::write(source.join("listed_only/inside.txt"), "x").unwrap();
    fs::write(source.join("sealed/hidden.txt"), "x").unwrap();
    set_mode(&source.join("listed_only"), 0o644);
    set_mode(&source.join("sealed"), 0o000);

    let result = MoveManifest::scan(&source);
    set_mode(&source.join("listed_only"), 0o755);
    set_mode(&source.join("sealed"), 0o755);
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains(
            "listed_only/inside.txt: listed by the filesystem, but it cannot be examined"
        ),
        "{error}"
    );
    assert!(
        error.contains("sealed: its contents cannot be listed"),
        "{error}"
    );
}

/// Beneath a folder that cannot be read, an entry is unknown, not missing:
/// "1457 missing" about files nobody could look for would be false.
#[cfg(unix)]
#[test]
fn entries_beneath_an_unreadable_folder_are_not_counted_missing() {
    if running_as_root() {
        eprintln!("skipping: root can list anything");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir_all(source.join("sealed")).unwrap();
    fs::write(source.join("sealed/a.txt"), "a").unwrap();
    fs::write(source.join("sealed/b.txt"), "b").unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();
    set_mode(&source.join("sealed"), 0o000);
    let walk = Walk::of(&source, "tree");
    set_mode(&source.join("sealed"), 0o755);

    let diff = manifest.compare(&walk.unwrap(), Match::Exact);
    assert!(diff.missing.is_empty(), "{diff:?}");
    assert_eq!(diff.problems.len(), 1, "{diff:?}");
    assert!(!diff.is_residue());
}

/// 3.11 wrote version-1 manifests. Comparing whole manifests, `version`
/// included, would read every transaction it left as "changed" forever.
#[test]
fn a_version_1_manifest_still_verifies_against_a_fresh_scan() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir_all(source.join("sub")).unwrap();
    fs::write(source.join("sub/file.bin"), [1_u8, 2, 3]).unwrap();
    let mut stored = serde_json::to_value(MoveManifest::scan(&source).unwrap()).unwrap();
    stored["version"] = serde_json::json!(1);
    let operation = temp.path().join("operation");
    fs::create_dir(&operation).unwrap();
    fs::write(
        operation.join(MANIFEST_FILE),
        serde_json::to_vec(&stored).unwrap(),
    )
    .unwrap();

    let recovered = read_manifest(&operation).unwrap();
    recovered.verify_source_unchanged(&source).unwrap();
    recovered.verify_destination(&source).unwrap();
}

/// A difference names the path and what is different about it, so the
/// person reading a refused move or a stuck reconcile knows where to look.
#[cfg(unix)]
#[test]
fn a_difference_names_each_path_and_what_changed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir_all(source.join("sub")).unwrap();
    fs::write(source.join("grown.txt"), "12345").unwrap();
    fs::write(source.join("gone.txt"), "x").unwrap();
    fs::write(source.join("sub/was_a_file"), "abc").unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();

    fs::write(source.join("grown.txt"), "123456789012").unwrap();
    fs::remove_file(source.join("gone.txt")).unwrap();
    fs::write(source.join("new.txt"), "n").unwrap();
    fs::remove_file(source.join("sub/was_a_file")).unwrap();
    std::os::unix::fs::symlink("elsewhere", source.join("sub/was_a_file")).unwrap();

    let diff = manifest.compare(&Walk::of(&source, "tree").unwrap(), Match::Content);
    let summary = diff.summary(10);
    assert!(
        summary.contains("grown.txt: 12 bytes now, was 5"),
        "{summary}"
    );
    assert!(summary.contains("gone.txt: missing"), "{summary}");
    assert!(summary.contains("new.txt: not in the record"), "{summary}");
    assert!(
        summary.contains("sub/was_a_file: a link to elsewhere now, was a 3-byte file"),
        "{summary}"
    );
    assert!(!diff.is_residue());
    assert_eq!(diff.present(), diff.recorded - 1);
}

/// Removing part of a tree moves its folders' times and nothing else, so
/// a half-removed tree is a residue: everything left is as recorded.
/// Anything added or changed, and it is not.
#[test]
fn a_partly_removed_tree_is_a_residue_and_an_edited_one_is_not() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir_all(source.join("sub")).unwrap();
    for name in ["a.txt", "sub/b.txt", "sub/c.txt"] {
        fs::write(source.join(name), name).unwrap();
    }
    let manifest = MoveManifest::scan(&source).unwrap();
    // Folder times have whole-second resolution on some filesystems.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::remove_file(source.join("sub/b.txt")).unwrap();
    fs::remove_file(source.join("a.txt")).unwrap();

    let walk = Walk::of(&source, "tree").unwrap();
    let whole = manifest.compare(&walk, Match::Whole);
    assert!(whole.is_residue(), "{whole:?}");
    assert!(!whole.is_clean());
    assert_eq!(whole.missing.len(), 2);
    assert_eq!(whole.present(), 2, "sub and sub/c.txt are left");
    let exact = manifest.compare(&walk, Match::Exact);
    assert!(
        exact
            .changed
            .iter()
            .any(|(path, _)| path == Path::new("sub")),
        "an exact comparison sees the folder's time move: {exact:?}"
    );

    fs::write(source.join("sub/c.txt"), "edited, and longer").unwrap();
    let edited = manifest.compare(&Walk::of(&source, "tree").unwrap(), Match::Whole);
    assert!(!edited.is_residue(), "{edited:?}");
}

/// Every name the target will not hold is found before any content is
/// copied, and named together. A name already taken in a staging folder
/// fastf made empty is how a case-insensitive drive refuses `README` beside
/// `readme`; planting the clash is how a test on a case-sensitive one gets
/// there.
/// Every folder the target will not hold is found before any content is
/// copied, and named together; a file name it will not hold is found when
/// the file is reached, named the same way, still before anything is
/// published. A name already taken in a staging folder fastf made empty is
/// how a case-insensitive drive refuses `README` beside `readme`; planting
/// the clash is how a test on a case-sensitive one gets there.
#[test]
fn the_names_pass_refuses_every_folder_and_a_file_is_refused_when_reached() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let staging = temp.path().join("staging");
    fs::create_dir_all(source.join("sub")).unwrap();
    fs::create_dir_all(source.join("other")).unwrap();
    fs::write(source.join("a.txt"), "aaa").unwrap();
    fs::write(source.join("c.txt"), "ccc").unwrap();
    fs::write(source.join("sub/b.txt"), "bbb").unwrap();
    fs::create_dir(&staging).unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();
    fs::write(staging.join("sub"), "").unwrap();
    fs::write(staging.join("other"), "").unwrap();

    let run = || {
        copy_to_staging(
            &manifest,
            &source,
            &staging,
            &Mutex::new(Progress::new(&[])),
            &AtomicBool::new(false),
        )
        .map_err(|error| error.to_string())
    };
    let error = run().unwrap_err();
    assert!(error.contains("2 names"), "{error}");
    assert!(
        error.contains("sub: the filesystem there takes this name"),
        "{error}"
    );
    assert!(
        error.contains("other: the filesystem there takes this name"),
        "{error}"
    );
    assert!(error.contains("No file's contents were copied"), "{error}");
    assert!(!staging.join("a.txt").exists(), "no file was made");

    fs::remove_file(staging.join("sub")).unwrap();
    fs::remove_file(staging.join("other")).unwrap();
    fs::write(staging.join("c.txt"), "").unwrap();
    let error = run().unwrap_err();
    assert!(error.contains("c.txt cannot be made"), "{error}");
    assert!(error.contains("takes this name"), "{error}");
    assert_eq!(
        fs::read(staging.join("a.txt")).unwrap(),
        b"aaa",
        "written once, whole"
    );
}

/// Two names that differ only in case are found by reading the record,
/// once the target has said it ignores case — before anything is copied.
#[test]
fn names_that_differ_only_in_case_are_found_in_the_record() {
    // The record as a case-keeping filesystem would have produced it —
    // built by hand, since a Windows tree could not hold both `Docs`.
    let entry = |path: &str, kind| ManifestEntry {
        path: PathBuf::from(path),
        kind,
        bytes: 0,
        source_modified: ModifiedTime::from_system_time(UNIX_EPOCH),
        link_target: None,
    };
    let manifest = MoveManifest {
        version: MANIFEST_VERSION,
        entries: vec![
            entry("Docs", ManifestKind::Directory),
            entry("Docs/README.md", ManifestKind::File),
            entry("docs", ManifestKind::Directory),
            entry("docs/other.md", ManifestKind::File),
            entry("docs/readme.md", ManifestKind::File),
        ],
    };
    assert_eq!(
        case_clashes(&manifest),
        vec![(PathBuf::from("Docs"), PathBuf::from("docs"))],
        "a clash between folders, and none between files in different folders"
    );
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        target_ignores_case(temp.path()),
        cfg!(windows),
        "Windows ignores case; this unix filesystem keeps it"
    );
    assert!(
        !temp.path().join(".Fastf-Case-Probe").exists(),
        "the probe is gone"
    );
}

#[cfg(unix)]
#[test]
fn a_refused_name_is_said_in_words() {
    let from = |code| name_refusal(&std::io::Error::from_raw_os_error(code));
    assert!(from(libc::EINVAL).contains("does not allow this name"));
    assert!(from(libc::EILSEQ).contains("does not allow this name"));
    assert!(from(libc::ENAMETOOLONG).contains("too long"));
    assert!(from(libc::EEXIST).contains("takes this name for another"));
}

/// 3.11's journals are read, marked as ones whose source it may have
/// half-removed, and rewritten as version 3 by the next phase change; a
/// version-2 journal claiming what only version 3 records, and a journal
/// from a future version, are refused.
#[test]
fn journals_are_read_across_versions_and_rewritten_as_the_newest() {
    let temp = tempfile::tempdir().unwrap();
    let source_base = temp.path().join("source-base");
    let target_base = temp.path().join("target-base");
    fs::create_dir(&source_base).unwrap();
    fs::create_dir(&target_base).unwrap();
    let transaction = MoveTransaction::begin(
        &source_base,
        Path::new("project"),
        &target_base,
        Path::new("project"),
        "ID0001",
        Operation::Move,
    )
    .unwrap();
    let operation = transaction.operation_dir.clone();
    let journal_path = operation.join(JOURNAL_FILE);
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    assert_eq!(written["version"], 3);
    assert!(written.get("legacy_cleanup").is_none(), "{written}");

    let write = |edit: &dyn Fn(&mut serde_json::Map<String, serde_json::Value>)| {
        let mut journal = written.clone();
        edit(journal.as_object_mut().unwrap());
        fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    };
    write(&|journal| {
        journal.insert("version".into(), serde_json::json!(2));
        journal.remove("operation");
        journal.remove("host");
        journal.remove("machine");
        journal.remove("in_place");
    });
    let legacy = read_journal(&operation).unwrap();
    assert!(legacy.source_may_be_partial());
    let mut rewritten = transaction_from_journal(&target_base, &operation, legacy);
    rewritten.set_phase(MovePhase::CleanupPending).unwrap();
    let reread = read_journal(&operation).unwrap();
    assert_eq!(reread.version, 3);
    assert!(
        reread.source_may_be_partial(),
        "carried through the rewrite"
    );

    for (bad, why) in [
        (
            serde_json::json!({"version": 2, "phase": "Retired"}),
            "only version 3",
        ),
        (
            serde_json::json!({"version": 4}),
            "unsupported move journal version 4",
        ),
    ] {
        write(&|journal| {
            journal.remove("host");
            journal.remove("machine");
            journal.remove("in_place");
            for (key, value) in bad.as_object().unwrap() {
                journal.insert(key.clone(), value.clone());
            }
        });
        let error = format!("{:#}", read_journal(&operation).unwrap_err());
        assert!(error.contains(why), "expected '{why}', got: {error}");
    }
}

/// A phase is a file created, never a rename: the highest marker present
/// is the phase, above whatever `move.json` says — which on a cloud mount
/// may be the only write of it that arrived.
#[test]
fn a_phase_is_a_marker_file_and_the_highest_one_wins() {
    let temp = tempfile::tempdir().unwrap();
    let source_base = temp.path().join("source-base");
    let target_base = temp.path().join("target-base");
    fs::create_dir(&source_base).unwrap();
    fs::create_dir(&target_base).unwrap();
    let mut transaction = MoveTransaction::begin(
        &source_base,
        Path::new("project"),
        &target_base,
        Path::new("project"),
        "ID0001",
        Operation::Move,
    )
    .unwrap();
    let operation = transaction.operation_dir.clone();
    let journal_bytes = fs::read(operation.join(JOURNAL_FILE)).unwrap();
    transaction.set_phase(MovePhase::CleanupPending).unwrap();
    transaction.set_phase(MovePhase::Retired).unwrap();
    assert_eq!(
        fs::read(operation.join(JOURNAL_FILE)).unwrap(),
        journal_bytes,
        "move.json is written once"
    );
    assert!(operation.join("phase.CleanupPending").is_file());
    assert!(operation.join("phase.Retired").is_file());
    assert_eq!(read_journal(&operation).unwrap().phase, MovePhase::Retired);
    // Setting a phase already reached again is fine, and lower markers
    // never lower the phase.
    transaction.set_phase(MovePhase::CleanupPending).unwrap();
    assert_eq!(read_journal(&operation).unwrap().phase, MovePhase::Retired);
}

#[test]
fn validate_refuses_link_entries_that_do_not_hold_together() {
    let entry = |kind, bytes, link_target: Option<&str>| ManifestEntry {
        path: PathBuf::from("entry"),
        kind,
        bytes,
        source_modified: ModifiedTime::from_system_time(UNIX_EPOCH),
        link_target: link_target.map(PathBuf::from),
    };
    let manifest = |version, entry| MoveManifest {
        version,
        entries: vec![entry],
    };
    manifest(2, entry(ManifestKind::Symlink, 0, Some("target")))
        .validate()
        .unwrap();
    for (bad, why) in [
        (
            manifest(1, entry(ManifestKind::Symlink, 0, Some("target"))),
            "cannot hold a link",
        ),
        (
            manifest(2, entry(ManifestKind::Junction, 0, None)),
            "has no target",
        ),
        (
            manifest(2, entry(ManifestKind::File, 3, Some("target"))),
            "is not a link",
        ),
        (
            manifest(2, entry(ManifestKind::Directory, 3, None)),
            "non-zero byte length",
        ),
        (
            manifest(2, entry(ManifestKind::Symlink, 6, Some("target"))),
            "non-zero byte length",
        ),
        (
            manifest(3, entry(ManifestKind::File, 0, None)),
            "unsupported move manifest version 3",
        ),
    ] {
        let error = bad.validate().unwrap_err().to_string();
        assert!(error.contains(why), "expected '{why}', got: {error}");
    }
}

/// **The pool's walk finds exactly what one thread walking depth-first
/// finds** — every entry and every problem, in the same order — over
/// random trees of folders, files and links, beside a folder that cannot
/// be listed, a pipe, and a chain deeper than any walk goes.
#[cfg(unix)]
#[test]
fn the_parallel_walk_finds_what_one_thread_finds() {
    use proptest::prelude::*;
    let config = ProptestConfig {
        cases: 24,
        ..ProptestConfig::default()
    };
    proptest!(config, |(
        shape in prop::collection::vec((0u8..4, 0usize..8, "[a-c]{1,3}"), 1..120),
        deep in any::<bool>(),
    )| {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("tree");
        fs::create_dir(&root).unwrap();
        let mut folders = vec![root.clone()];
        for (n, (kind, parent, name)) in shape.iter().enumerate() {
            let path = folders[parent % folders.len()].join(format!("{name}{n}"));
            match kind {
                0 => {
                    fs::create_dir(&path).unwrap();
                    folders.push(path);
                }
                1 => std::os::unix::fs::symlink(format!("../{name}"), &path).unwrap(),
                _ => fs::write(&path, name.repeat(n)).unwrap(),
            }
        }
        let fifo = std::ffi::CString::new(
            root.join("pipe").as_os_str().as_encoded_bytes().to_vec(),
        )
        .unwrap();
        // SAFETY: a valid NUL-terminated path and a plain mode.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let closed = folders.get(1).cloned();
        if let Some(closed) = &closed
            && !running_as_root()
        {
            set_mode(closed, 0o000);
        }
        if deep {
            let mut chain = root.join("deep");
            for _ in 0..70 {
                chain = chain.join("d");
            }
            fs::create_dir_all(&chain).unwrap();
        }

        let parallel = Walk::of(&root, "tree");
        let mut reference = Walk::default();
        let device = device_of(&fs::symlink_metadata(&root).unwrap());
        let sequential = walk_at(&root, &root, 0, device, &mut reference).map(|()| {
            reference
                .entries
                .sort_by(|left, right| left.path.cmp(&right.path));
            reference
                .problems
                .sort_by(|left, right| left.path.cmp(&right.path));
            reference
        });
        if let Some(closed) = &closed {
            set_mode(closed, 0o755);
        }
        let (parallel, sequential) = (parallel.unwrap(), sequential.unwrap());
        prop_assert!(parallel.problems.iter().any(|p| p.problem == Problem::Special));
        prop_assert_eq!(
            deep,
            parallel.problems.iter().any(|p| p.problem == Problem::TooDeep)
        );
        prop_assert_eq!(parallel, sequential);
    });
}

/// **A change made while a project moves is kept.** A file appended to, a
/// file removed, a folder made — each found by the next look and brought
/// across, where 3.13 failed the move after the whole copy and threw the
/// copy away. A file changed before its copy is copied as it is then.
#[test]
fn what_changes_during_the_copy_is_copied_too() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir_all(source.join("logs")).unwrap();
    fs::write(source.join("PROJECT_INFO.md"), "---\nid: ID0001\n---\n").unwrap();
    fs::write(source.join("logs/dev.log"), "line 1\n").unwrap();
    fs::write(source.join("gone.txt"), "x").unwrap();
    fs::write(source.join("early.txt"), "as scanned").unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();
    let mut body = manifest.without_root_metadata();
    let staging = temp.path().join("copy");
    fs::create_dir(&staging).unwrap();
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    // Changed before its copy: copied as it is.
    fs::write(source.join("early.txt"), "changed before the copy").unwrap();
    let copied = copy_to_staging(&body, &source, &staging, &progress, &cancel).unwrap();
    body = body.with_copied(copied);
    // Changed after it.
    fs::write(source.join("logs/dev.log"), "line 1\nline 2\n").unwrap();
    fs::remove_file(source.join("gone.txt")).unwrap();
    fs::create_dir(source.join("cache")).unwrap();
    fs::write(source.join("cache/new.bin"), [7_u8; 9]).unwrap();

    let root = settle_copy(
        &mut body,
        &source,
        &staging,
        &progress,
        &cancel,
        Ticker::none(),
    )
    .unwrap();
    assert!(root.is_some(), "the root PROJECT_INFO.md as last seen");
    assert_eq!(
        fs::read_to_string(staging.join("early.txt")).unwrap(),
        "changed before the copy"
    );
    assert_eq!(
        fs::read_to_string(staging.join("logs/dev.log")).unwrap(),
        "line 1\nline 2\n"
    );
    assert!(!staging.join("gone.txt").exists());
    assert_eq!(fs::read(staging.join("cache/new.bin")).unwrap(), [7_u8; 9]);
    body.verify_destination(&staging).unwrap();
    assert!(
        !body
            .entries
            .iter()
            .any(|entry| entry.path == Path::new("gone.txt")),
        "the record is what the copy holds"
    );
}

/// A project something keeps writing the whole time — a dev server's
/// log — is not a reason to stop: after the last round the copy is
/// published as the last round left it, consistent with its record, and
/// the merge after the retire carries the rest.
#[test]
fn a_project_that_never_holds_still_is_published_as_last_caught_up() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("project");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("busy.log"), "0\n").unwrap();
    let manifest = MoveManifest::scan(&source).unwrap();
    let mut body = manifest.without_root_metadata();
    let staging = temp.path().join("copy");
    fs::create_dir(&staging).unwrap();
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    let copied = copy_to_staging(&body, &source, &staging, &progress, &cancel).unwrap();
    body = body.with_copied(copied);
    let busy = source.join("busy.log");
    let mut n = 0;
    BEFORE_EACH_LOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            n += 1;
            let mut log = fs::OpenOptions::new().append(true).open(&busy).unwrap();
            writeln!(log, "{n}").unwrap();
        }));
    });
    let settled = settle_copy(
        &mut body,
        &source,
        &staging,
        &progress,
        &cancel,
        Ticker::none(),
    );
    BEFORE_EACH_LOOK.with(|hook| *hook.borrow_mut() = None);
    settled.unwrap();
    body.verify_destination(&staging)
        .expect("the copy is what its record says");
    let copied = fs::read_to_string(staging.join("busy.log")).unwrap();
    assert!(copied.contains(&format!("{SETTLE_ROUNDS}\n")), "{copied}");
    assert!(
        fs::read_to_string(source.join("busy.log")).unwrap().len() > copied.len(),
        "the last line is for the merge"
    );
}
