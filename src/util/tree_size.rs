//! Read-only logical size snapshots for project directory trees.
//!
//! A snapshot is deliberately all-or-nothing: if any directory entry or file
//! metadata cannot be read, callers get `None` rather than a plausible-looking
//! partial total. Links (including Windows junctions) are never followed, and
//! special filesystem nodes do not contribute bytes.

use std::fs;
use std::io;
use std::path::Path;

/// Sum the logical lengths of every regular file below `root`.
///
/// Hidden files and `PROJECT_INFO.md` are ordinary files and therefore count.
/// Symlinks/junctions and special nodes are ignored. `None` means the root was
/// not a directory, an entry could not be inspected, or the total overflowed.
pub(crate) fn directory_size(root: &Path) -> Option<u64> {
    directory_size_inner(root).ok()
}

fn directory_size_inner(root: &Path) -> io::Result<u64> {
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
            directory_size_inner(&path)?
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

/// Treat every Windows reparse point as link-like. Junctions are the important
/// case here: some are directories without `FileType::is_symlink()`, but walking
/// them would still leave the project tree and could introduce cycles.
fn is_link_like(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::directory_size;
    use std::fs;

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

        // Creating links can require Developer Mode on older Windows hosts.
        // Skip only that environmental restriction; every created link is
        // asserted not to contribute to the total.
        if let Err(error) = symlink_file(outside.join("large.bin"), root.join("file-link")) {
            if error.kind() == ErrorKind::PermissionDenied {
                return;
            }
            panic!("creating file link: {error}");
        }
        if let Err(error) = symlink_dir(&outside, root.join("dir-link")) {
            if error.kind() == ErrorKind::PermissionDenied {
                return;
            }
            panic!("creating directory link: {error}");
        }

        assert_eq!(directory_size(&root), Some(11));
        assert_eq!(directory_size(&root.join("dir-link")), None);
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
