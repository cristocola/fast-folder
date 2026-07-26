//! Windows filesystem semantics that differ from POSIX in ways that lose data
//! or corrupt names quietly.
//!
//! Most of this file is `#[cfg(windows)]` because the behaviours only exist
//! there. The name-hygiene rules are the exception: they are applied on every
//! platform on purpose, so a project created on Linux still opens on Windows,
//! and those cases are asserted everywhere.

use std::fs;
use std::path::{Path, PathBuf};

use fastf::core::{library, naming, project_info};

/// A discoverable project folder, so `library` functions have something real.
fn write_project(base: &Path, folder: &str, id: &str) -> PathBuf {
    let dir = base.join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        project_info::pinfo_path(&dir),
        format!(
            "---\nid: {id}\ntemplate: t\ntemplate_name: T\n\
             created: 2026-01-01T00:00:00Z\nfolder: {folder}\npath: x\n\
             variables: {{}}\ntags: []\n---\n"
        ),
    )
    .unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Name hygiene — enforced on every platform
// ---------------------------------------------------------------------------

/// Every name fastf produces must be creatable on Windows. This is the property
/// that `sanitize_name` exists to guarantee, checked by actually creating the
/// directory rather than by reasoning about the rules.
#[test]
fn sanitized_names_are_actually_creatable() {
    let tmp = tempfile::tempdir().unwrap();
    let hostile = [
        "CON",
        "con",
        "NUL",
        "PRN",
        "AUX",
        "COM1",
        "LPT9", // reserved devices
        "CON.txt",
        "nul.tar.gz", // reserved with extensions
        "Draft.",
        "Draft ",
        "Draft . .", // trailing dot/space
        "a:b*c?d",
        "pipe|name",
        "quote\"name", // illegal characters
        "tab\tname",
        "nul\u{0}byte", // control characters
        "Ünïcödé",
        "проект",
        "日本語",
        "emoji🎬name", // non-ASCII
        "normal_name",
        "Release v1.2.3", // must pass through untouched
    ];

    for raw in hostile {
        let safe = naming::sanitize_name(raw);
        if safe.is_empty() {
            continue; // e.g. ".." reduces to nothing; callers reject it
        }
        let dir = tmp.path().join(&safe);
        fs::create_dir(&dir)
            .unwrap_or_else(|e| panic!("sanitize_name({raw:?}) => {safe:?} is not creatable: {e}"));

        // And the name on disk must be exactly what we asked for — Windows
        // silently drops trailing dots and spaces, which would desynchronize
        // the recorded folder name from reality.
        let on_disk = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n == &safe);
        assert_eq!(
            on_disk.as_deref(),
            Some(safe.as_str()),
            "sanitize_name({raw:?}) => {safe:?} did not round-trip through the filesystem"
        );
        fs::remove_dir(&dir).unwrap();
    }
}

/// Renaming only the capitalisation is a normal tidy-up and must work.
#[test]
fn case_only_rename_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "myproject", "ID0001");
    fs::write(base.join("myproject/keep.txt"), "content").unwrap();

    let project = library::scan_base(base).remove(0);
    let renamed = library::rename_project(&project, "MyProject").unwrap();

    assert_eq!(renamed.name, "MyProject");
    assert_eq!(
        fs::read_to_string(renamed.path.join("keep.txt")).unwrap(),
        "content",
        "content must survive the two-step rename"
    );
    let on_disk = fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.eq_ignore_ascii_case("myproject"))
        .expect("folder present");
    assert_eq!(on_disk, "MyProject", "the new casing must reach the disk");
    assert_eq!(library::scan_base(base).len(), 1);
}

/// Non-ASCII project names must survive a full create → discover → rename cycle.
#[test]
fn non_ascii_names_round_trip_through_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    for (folder, id) in [("Ünïcödé_проект", "ID0001"), ("映画_🎬", "ID0002")] {
        write_project(base, folder, id);
    }
    let found = library::scan_base(base);
    assert_eq!(found.len(), 2, "unicode folders must be discoverable");

    let target = found
        .iter()
        .find(|p| p.name.contains("Ünïcödé"))
        .expect("unicode project found");
    let renamed = library::rename_project(target, "Ünïcödé_renamed").unwrap();
    assert_eq!(renamed.name, "Ünïcödé_renamed");
    assert!(renamed.path.is_dir());
}

// ---------------------------------------------------------------------------
// Windows-only behaviours
// ---------------------------------------------------------------------------

/// Deleting a project containing read-only files must work.
///
/// Windows refuses to delete a read-only file, and `remove_dir_all` gives up on
/// the whole tree when it hits one. Assets copied from a network share, a CD, or
/// a git object store are routinely read-only.
#[cfg(windows)]
#[test]
fn delete_succeeds_despite_read_only_files() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let dir = write_project(base, "proj", "ID0001");
    let locked = dir.join("readonly.txt");
    fs::write(&locked, "immutable").unwrap();
    let mut perms = fs::metadata(&locked).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&locked, perms).unwrap();

    let project = library::scan_base(base).remove(0);
    library::delete_project(&project).expect("read-only file must not block deletion");
    assert!(!dir.exists());
}

/// A handle held briefly by another process (antivirus, the search indexer,
/// Explorer's preview pane) must not fail the operation outright — the retry
/// layer exists for exactly this.
#[cfg(windows)]
#[test]
fn transient_sharing_violation_is_retried_not_fatal() {
    use std::os::windows::fs::OpenOptionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("tree");
    fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("scanned.bin");
    fs::write(&victim, vec![0u8; 1024]).unwrap();

    // share_mode(0) denies all sharing — the same thing a scanner does while it
    // reads a freshly written file. Deletion fails until this handle closes.
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&victim)
        .expect("opening exclusively");

    // Release it partway through the retry schedule (10/20/40/80/160 ms).
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(60));
        drop(handle);
    });

    let result = fastf::util::fs_retry::remove_dir_all(&dir);
    releaser.join().unwrap();

    assert!(
        result.is_ok(),
        "a transient sharing violation should be waited out, got {result:?}"
    );
    assert!(!dir.exists());
}

/// A same-filesystem move of a project containing a junction must succeed and
/// keep the junction — `fs::rename` copies nothing, so there is nothing to lose.
///
/// This is the counterpart to the refusal: the guard is deliberately scoped to
/// the staged (copying) path, because refusing here would block the common case
/// for no benefit. The staged refusal itself is covered in `library`'s unit
/// tests, which can reach the private path without needing two real filesystems.
#[cfg(windows)]
#[test]
fn same_filesystem_move_preserves_a_junction() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("shared_assets");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("payload.bin"), vec![3u8; 2048]).unwrap();

    let base = tmp.path().join("base");
    fs::create_dir_all(&base).unwrap();
    let dir = write_project(&base, "proj", "ID0001");
    fs::write(dir.join("ordinary.txt"), "keep me").unwrap();

    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(dir.join("linked"))
        .arg(&target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !made {
        eprintln!("skipping: the OS refused to create a junction");
        return;
    }

    let project = library::scan_base(&base).remove(0);
    assert_eq!(
        fastf::core::assets::find_links(&project.path).unwrap(),
        vec!["linked".to_string()],
        "the junction must be visible to the pre-flight check"
    );

    // Same filesystem → atomic rename, nothing copied, so the junction rides
    // along untouched and the move is allowed.
    let other_base = tmp.path().join("other");
    fs::create_dir_all(&other_base).unwrap();
    let moved = library::move_project(&project, &other_base)
        .expect("a same-filesystem move copies nothing and must be allowed");

    assert!(moved.path.join("ordinary.txt").is_file(), "content moved");
    assert!(
        moved.path.join("linked").exists(),
        "the junction must survive a rename-based move"
    );
    assert!(
        moved.path.join("linked/payload.bin").is_file(),
        "the junction must still resolve to its target"
    );
    assert!(
        target.join("payload.bin").is_file(),
        "the junction's target must be untouched"
    );
    assert!(!dir.exists(), "source folder gone after a successful move");
}

/// Paths beyond MAX_PATH must work end to end. `canonicalize` returns the
/// `\\?\` form that makes this possible; the display layer strips it, and that
/// separation is what this checks.
#[cfg(windows)]
#[test]
fn long_paths_work_and_display_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let mut deep = tmp.path().to_path_buf();
    for _ in 0..4 {
        deep = deep.join("x".repeat(60));
    }
    if fs::create_dir_all(&deep).is_err() {
        eprintln!("skipping: long paths unavailable on this system");
        return;
    }

    let dir = write_project(&deep, "proj", "ID0001");
    assert!(
        dir.display().to_string().len() > 260,
        "test should exceed MAX_PATH, got {}",
        dir.display().to_string().len()
    );

    let found = library::scan_base(&deep);
    assert_eq!(found.len(), 1, "long-path project must be discoverable");

    // Canonical form keeps the prefix (that is what makes it work); the
    // rendered form must not show it.
    let canonical = dir.canonicalize().unwrap();
    let shown = fastf::util::paths::display_path(&canonical);
    assert!(
        !shown.starts_with(r"\\?\"),
        "verbatim prefix leaked into display output: {shown}"
    );
    assert!(Path::new(&shown).is_absolute());
}
