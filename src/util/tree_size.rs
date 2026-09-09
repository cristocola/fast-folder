//! Read-only logical size snapshots for project directory trees.
//!
//! A snapshot is deliberately all-or-nothing: if any directory entry or file
//! metadata cannot be read, callers get `None` rather than a plausible-looking
//! partial total. Links (including Windows junctions) are never followed, and
//! special filesystem nodes do not contribute bytes.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::util::paths::is_link_like;

/// Sum the logical lengths of every regular file below `root`, abandoning the
/// walk once `cancel` is set.
///
/// Hidden files and `PROJECT_INFO.md` are ordinary files and therefore count.
/// Symlinks/junctions and special nodes are ignored. `None` means the root was
/// not a directory, an entry could not be inspected, the total overflowed — or
/// the walk was cancelled:
///
/// a cancelled walk gives the same answer an unreadable tree does, because the
/// snapshot is all-or-nothing either way. A caller that cancels must therefore
/// **discard** the result rather than record it: there `None` means "no answer",
/// not "unavailable". Cancellation is checked once per directory entry, which is
/// what bounds teardown when the tree lives on a slow network share.
pub(crate) fn directory_size_until(root: &Path, cancel: &AtomicBool) -> Option<u64> {
    directory_size_inner(root, cancel).ok()
}

/// The depth-0 entry point. **Nothing else may call it** — the recursive step
/// goes to `directory_size_at` with `depth + 1`, which is what it used to do
/// through here instead, resetting the counter at every level and leaving the
/// limit below unreachable.
fn directory_size_inner(root: &Path, cancel: &AtomicBool) -> io::Result<u64> {
    directory_size_at(root, cancel, 0)
}

/// The body, carrying the depth. Past the limit the whole snapshot fails, which
/// the caller renders as `unavailable` — consistent with every other read
/// failure here, and the only honest answer when part of the tree was not
/// counted.
fn directory_size_at(root: &Path, cancel: &AtomicBool, depth: usize) -> io::Result<u64> {
    if depth >= crate::util::paths::MAX_WALK_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory tree is too deep to measure",
        ));
    }
    // `symlink_metadata` is load-bearing: `metadata` would follow a root link.
    let root_metadata = fs::symlink_metadata(root)?;
    if is_link_like(&root_metadata) || !root_metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "size root is not a directory",
        ));
    }

    let mut total = 0_u64;
    for entry in fs::read_dir(root)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "size walk cancelled",
            ));
        }
        let entry = entry?;
        let path = entry.path();

        // One non-following metadata lookup gives both the node kind and logical
        // file length. On Windows, `is_link_like` additionally catches junctions
        // and other reparse points that are not exposed as ordinary symlinks.
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if is_link_like(&metadata) {
            continue;
        }

        let bytes = if file_type.is_dir() {
            directory_size_at(&path, cancel, depth + 1)?
        } else if file_type.is_file() {
            metadata.len()
        } else {
            // Sockets, FIFOs, devices, and other special nodes have no regular
            // file content to include and must never be opened.
            0
        };
        total = total.checked_add(bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "directory size overflowed u64")
        })?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::directory_size_until;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    /// The uncancelled walk every case here but one wants.
    fn directory_size(root: &Path) -> Option<u64> {
        directory_size_until(root, &AtomicBool::new(false))
    }

    #[test]
    fn counts_nested_hidden_and_metadata_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        fs::create_dir_all(root.join("nested/empty")).unwrap();
        fs::write(root.join("PROJECT_INFO.md"), b"metadata").unwrap();
        fs::write(root.join(".hidden"), b"hidden").unwrap();
        fs::write(root.join("nested/data.bin"), [0_u8; 17]).unwrap();

        assert_eq!(directory_size(&root), Some(8 + 6 + 17));
    }

    /// The depth limit is enforced, so a pathological tree is `None` rather
    /// than a dead process.
    ///
    /// It was written down and unreachable: `directory_size_at` checked `depth`
    /// and then recursed through `directory_size_inner`, which starts again at
    /// zero. This walk runs on `util::size_scan`'s workers without anybody
    /// asking for it, and a stack overflow is not an unwind — so the failure was
    /// the whole app dying while looking at a folder.
    ///
    /// Unix only, for the setup's sake rather than the code's: a tree this deep
    /// needs a path past Windows' MAX_PATH, which `create_dir_all` refuses
    /// without long-path support. `MAX_WALK_DEPTH` is cross-platform.
    #[cfg(unix)]
    #[test]
    fn a_tree_past_the_depth_limit_is_unavailable_rather_than_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("deep");
        let mut path = root.clone();
        for level in 0..crate::util::paths::MAX_WALK_DEPTH + 5 {
            path = path.join(format!("l{level}"));
        }
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("leaf.bin"), [0_u8; 4]).unwrap();

        assert_eq!(
            directory_size(&root),
            None,
            "past the limit the snapshot is unavailable, like any other read failure"
        );

        // And a tree just inside the limit still measures, so the bound is the
        // one that is written down rather than something stricter.
        let shallow = tmp.path().join("shallow");
        let mut path = shallow.clone();
        for level in 0..crate::util::paths::MAX_WALK_DEPTH - 2 {
            path = path.join(format!("l{level}"));
        }
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("leaf.bin"), [0_u8; 4]).unwrap();
        assert_eq!(directory_size(&shallow), Some(4));
    }

    #[test]
    fn an_empty_directory_is_zero_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("empty");
        fs::create_dir(&root).unwrap();
        assert_eq!(directory_size(&root), Some(0));
    }

    #[test]
    fn preserves_large_u64_totals_without_reading_file_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("large");
        fs::create_dir(&root).unwrap();
        let first = fs::File::create(root.join("first.bin")).unwrap();
        let second = fs::File::create(root.join("second.bin")).unwrap();
        let first_len = 5 * 1024_u64.pow(3) + 41;
        let second_len = 3 * 1024_u64.pow(3) + 7;
        first.set_len(first_len).unwrap();
        second.set_len(second_len).unwrap();

        assert_eq!(directory_size(&root), Some(first_len + second_len));
    }

    #[test]
    fn missing_or_non_directory_roots_are_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(directory_size(&tmp.path().join("missing")), None);
        let file = tmp.path().join("file");
        fs::write(&file, b"content").unwrap();
        assert_eq!(directory_size(&file), None);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_followed() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("local.bin"), [0_u8; 11]).unwrap();
        fs::write(outside.join("large.bin"), [0_u8; 101]).unwrap();
        symlink(outside.join("large.bin"), root.join("file-link")).unwrap();
        symlink(&outside, root.join("dir-link")).unwrap();

        assert_eq!(directory_size(&root), Some(11));
        assert_eq!(directory_size(&root.join("dir-link")), None);
    }

    #[cfg(windows)]
    #[test]
    fn links_and_junction_like_directory_links_are_not_followed() {
        use std::io::ErrorKind;
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("local.bin"), [0_u8; 11]).unwrap();
        fs::write(outside.join("large.bin"), [0_u8; 101]).unwrap();

        // Creating a symlink needs `SeCreateSymbolicLinkPrivilege`, which an
        // ordinary account has only under Developer Mode or elevation. Skip
        // that environmental restriction; every link that *is* created is
        // asserted not to contribute to the total.
        //
        // **The refusal is `ERROR_PRIVILEGE_NOT_HELD` (1314), not
        // `PermissionDenied`.** Rust maps only `ERROR_ACCESS_DENIED` (5) to
        // that kind and leaves 1314 uncategorized, so a guard reading the kind
        // alone never fires — and this test failed on every un-elevated
        // Windows machine without Developer Mode, which is the ordinary state
        // of a contributor's box. It passed in CI because the runners are
        // elevated, so the whole suite was red for outside contributors and
        // green everywhere it was looked at. Elsewhere the crate sidesteps the
        // privilege entirely by testing with `mklink /J` junctions, which need
        // none; here real symlinks are the subject, so the skip is the answer.
        let unprivileged = |error: &std::io::Error| {
            error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1314)
        };
        if let Err(error) = symlink_file(outside.join("large.bin"), root.join("file-link")) {
            if unprivileged(&error) {
                return;
            }
            panic!("creating file link: {error}");
        }
        if let Err(error) = symlink_dir(&outside, root.join("dir-link")) {
            if unprivileged(&error) {
                return;
            }
            panic!("creating directory link: {error}");
        }

        assert_eq!(directory_size(&root), Some(11));
        assert_eq!(directory_size(&root.join("dir-link")), None);
    }

    /// A cancelled walk must not be mistaken for a measured one: it has no
    /// answer at all, which is why the scanner discards it.
    #[test]
    fn a_cancelled_walk_has_no_result() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("payload.bin"), [0_u8; 32]).unwrap();

        let cancel = AtomicBool::new(true);
        assert_eq!(directory_size_until(&root, &cancel), None);
        // The same tree measures fine once nothing is asking us to stop.
        assert_eq!(directory_size(&root), Some(32));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_subdirectory_makes_the_snapshot_unavailable() {
        use std::os::unix::fs::PermissionsExt;

        // Root can bypass mode bits; there is no unreadable-directory fixture
        // available in that environment.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let blocked = root.join("blocked");
        fs::create_dir_all(&blocked).unwrap();
        fs::write(blocked.join("secret.bin"), [0_u8; 19]).unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

        assert_eq!(directory_size(&root), None);

        // Restore traversal so TempDir can clean up reliably.
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
    }
}
