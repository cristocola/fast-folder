//! How much room a folder's filesystem has, for this user.
//!
//! Asked once before a cross-drive copy, so a move that cannot fit says so
//! before it copies anything rather than after it has filled the disk. **An
//! answer the platform cannot give is `None`, and `None` never refuses**: some
//! FUSE filesystems report no sizes at all, a network share reports a quota or
//! a guess, and a compressing filesystem can hold more than it says. The copy
//! is still checked byte for byte; this only saves a copy that cannot finish.

use std::path::Path;

/// Bytes free to this user on the filesystem holding `path`, when the
/// platform can say.
pub fn available(path: &Path) -> Option<u64> {
    imp::available(path)
}

/// `statvfs`'s answer as bytes, or `None` for a filesystem that reports no
/// blocks at all — the FUSE way of saying it does not know.
#[cfg(any(unix, test))]
pub fn from_statvfs(blocks: u64, available_blocks: u64, fragment_size: u64) -> Option<u64> {
    if blocks == 0 || fragment_size == 0 {
        return None;
    }
    available_blocks.checked_mul(fragment_size)
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::path::Path;

    pub fn available(path: &Path) -> Option<u64> {
        let path = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        // SAFETY: an all-zero `statvfs` is a valid value to be overwritten.
        let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: a NUL-terminated path and a valid, writable struct.
        if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)] // the field widths differ by platform
        super::from_statvfs(
            stats.f_blocks as u64,
            stats.f_bavail as u64,
            stats.f_frsize as u64,
        )
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_to_caller: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }

    pub fn available(path: &Path) -> Option<u64> {
        // The call wants a folder, and a folder name ending in a separator:
        // without it a UNC share root is refused.
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.last() != Some(&u16::from(b'\\')) {
            wide.push(u16::from(b'\\'));
        }
        wide.push(0);
        let mut free_to_caller = 0_u64;
        // SAFETY: a NUL-terminated wide string and a valid out-pointer; the
        // two optional out-pointers are null, which the call allows.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_to_caller,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free_to_caller)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    pub fn available(_path: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filesystem_that_reports_no_blocks_is_unknown() {
        assert_eq!(from_statvfs(0, 0, 4096), None);
        assert_eq!(from_statvfs(100, 10, 0), None);
        assert_eq!(from_statvfs(100, 10, 4096), Some(40960));
        assert_eq!(from_statvfs(1, u64::MAX, 4096), None, "overflow is unknown");
    }

    #[test]
    fn the_temp_folder_has_an_answer() {
        let temp = tempfile::tempdir().unwrap();
        assert!(available(temp.path()).is_some());
    }
}
