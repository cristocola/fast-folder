//! Retrying wrappers for the mutating filesystem calls.
//!
//! On Windows, Defender, the Search Indexer, Explorer preview handlers and
//! OneDrive routinely hold a brief handle on a file that was just written.
//! `rename` and `remove_dir_all` then fail with `ERROR_SHARING_VIOLATION` or
//! `ERROR_ACCESS_DENIED` even though nothing is actually wrong — the handle is
//! gone milliseconds later. Without a retry this surfaces as a random failed
//! create or move with a baffling OS error, and it is the usual reason a
//! file-heavy tool is "fine on Linux, flaky on Windows".
//!
//! Retries are deliberately **Windows-only**: on Unix these error codes mean
//! what they say, and silently retrying would mask real bugs. On Unix every
//! function here is a direct passthrough, so Linux behaviour is unchanged.
//!
//! A genuine error (`NotFound`, for instance) is never retried — the predicate
//! is a small allow-list, not a catch-all.

use std::io;
use std::path::Path;
use std::time::Duration;

/// Backoff schedule between attempts. Total worst-case wait ≈ 310 ms, which is
/// well past a typical antivirus scan window while staying imperceptible.
const BACKOFF_MS: [u64; 5] = [10, 20, 40, 80, 160];

/// Windows error codes worth retrying.
#[cfg(windows)]
mod codes {
    /// The file is in use by another process.
    pub const ERROR_ACCESS_DENIED: i32 = 5;
    /// Another process has the file open and won't share it.
    pub const ERROR_SHARING_VIOLATION: i32 = 32;
    /// A byte-range lock is held on the file.
    pub const ERROR_LOCK_VIOLATION: i32 = 33;
    /// A directory still had entries — transient while a scanner releases them.
    pub const ERROR_DIR_NOT_EMPTY: i32 = 145;
}

/// True when `err` is the kind of transient contention worth waiting out.
#[cfg(windows)]
fn is_transient(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(codes::ERROR_ACCESS_DENIED)
            | Some(codes::ERROR_SHARING_VIOLATION)
            | Some(codes::ERROR_LOCK_VIOLATION)
            | Some(codes::ERROR_DIR_NOT_EMPTY)
    )
}

#[cfg(not(windows))]
fn is_transient(_err: &io::Error) -> bool {
    false
}

/// Run `op`, retrying transient contention with backoff. Returns the last error
/// if every attempt fails, so the caller sees the real cause.
fn retry<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut last = match op() {
        Ok(value) => return Ok(value),
        Err(err) if is_transient(&err) => err,
        Err(err) => return Err(err),
    };
    for delay in BACKOFF_MS {
        std::thread::sleep(Duration::from_millis(delay));
        match op() {
            Ok(value) => return Ok(value),
            Err(err) if is_transient(&err) => last = err,
            Err(err) => return Err(err),
        }
    }
    Err(last)
}

/// Clear the read-only attribute so removal can proceed.
///
/// Windows refuses to delete a read-only file, and `fs::remove_dir_all` gives up
/// on the whole tree when it hits one. Assets copied from a network share, a
/// CD, or a git object store are commonly read-only, so this is a real failure
/// mode rather than a theoretical one. Best-effort: failure here just means the
/// retry loop reports the original error.
#[cfg(windows)]
fn clear_readonly(path: &Path) {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

/// Recursively clear read-only attributes across a tree before removing it.
#[cfg(windows)]
fn clear_readonly_tree(path: &Path) {
    clear_readonly(path);
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        // Never follow links: clearing attributes through a link would touch
        // data outside the tree being removed.
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => clear_readonly_tree(&child),
            _ => clear_readonly(&child),
        }
    }
}

/// [`std::fs::rename`] with transient-contention retries.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    retry(|| std::fs::rename(from, to))
}

/// [`std::fs::remove_file`] with transient-contention retries.
pub fn remove_file(path: &Path) -> io::Result<()> {
    retry(|| std::fs::remove_file(path)).or_else(|err| {
        #[cfg(windows)]
        if is_transient(&err) || err.kind() == io::ErrorKind::PermissionDenied {
            clear_readonly(path);
            return std::fs::remove_file(path);
        }
        Err(err)
    })
}

/// [`std::fs::remove_dir_all`] with transient-contention retries, plus a
/// read-only sweep on Windows before the final attempt.
pub fn remove_dir_all(path: &Path) -> io::Result<()> {
    match retry(|| std::fs::remove_dir_all(path)) {
        Ok(()) => Ok(()),
        Err(err) => {
            #[cfg(windows)]
            if is_transient(&err) || err.kind() == io::ErrorKind::PermissionDenied {
                clear_readonly_tree(path);
                return retry(|| std::fs::remove_dir_all(path));
            }
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn rename_and_remove_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "x").unwrap();
        rename(&a, &b).unwrap();
        assert!(b.exists() && !a.exists());
        remove_file(&b).unwrap();
        assert!(!b.exists());
    }

    #[test]
    fn remove_dir_all_clears_nested_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tree");
        std::fs::create_dir_all(root.join("deep/deeper")).unwrap();
        std::fs::write(root.join("deep/deeper/f.txt"), "x").unwrap();
        remove_dir_all(&root).unwrap();
        assert!(!root.exists());
    }

    #[cfg(windows)]
    #[test]
    fn remove_dir_all_handles_readonly_files() {
        // Windows refuses to delete read-only files; the sweep must handle it.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ro");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("locked.txt");
        std::fs::write(&file, "x").unwrap();
        let mut perms = std::fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&file, perms).unwrap();

        remove_dir_all(&root).unwrap();
        assert!(!root.exists(), "read-only file blocked the removal");
    }

    #[test]
    fn genuine_errors_are_not_retried() {
        // A missing file must fail immediately, not after the full backoff.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.txt");
        let start = std::time::Instant::now();
        let err = remove_file(&missing).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "NotFound should not go through the backoff schedule"
        );
    }

    #[test]
    fn retry_gives_up_and_returns_last_error() {
        // Non-transient on every platform → exactly one attempt.
        let attempts = AtomicU32::new(0);
        let err = retry(|| -> io::Result<()> {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(io::Error::new(io::ErrorKind::NotFound, "gone"))
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn retry_returns_success_on_first_try() {
        let attempts = AtomicU32::new(0);
        retry(|| -> io::Result<()> {
            attempts.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }
}
