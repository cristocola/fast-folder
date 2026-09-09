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
    clear_readonly_tree_at(path, 0);
}

/// Best-effort, so the depth limit simply stops descending rather than
/// reporting: the caller is about to try a delete either way, and an
/// unreachable read-only attribute past `paths::MAX_WALK_DEPTH` levels is not
/// the reason it will fail.
#[cfg(windows)]
fn clear_readonly_tree_at(path: &Path, depth: usize) {
    if depth >= crate::util::paths::MAX_WALK_DEPTH {
        return;
    }
    clear_readonly(path);
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        // Never follow links: clearing attributes through a link would touch
        // data outside the tree being removed.
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => clear_readonly_tree_at(&child, depth + 1),
            _ => clear_readonly(&child),
        }
    }
}

/// [`std::fs::rename`] with transient-contention retries, and — on Windows —
/// one more attempt with the destination's read-only *attribute* cleared.
///
/// This is the publish step of [`crate::util::atomic::write`], so it is how
/// **every** metadata mutation lands: a tag, a journal note, a rename's
/// bookkeeping, a move's, the config and the counters. Unix `rename(2)` never
/// consults the target's mode — replacing a file is a property of the
/// directory — so on Linux a read-only `PROJECT_INFO.md` is simply written
/// through. Windows refuses `MoveFileEx` onto a target carrying
/// `FILE_ATTRIBUTE_READONLY`, and the retry schedule then spent its full
/// backoff before failing with `Access is denied`. A project restored from a
/// backup, copied off read-only media or synced down by a cloud client
/// arrives with that attribute set, and every one of those verbs stopped
/// working on it.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    match retry(|| std::fs::rename(from, to)) {
        Ok(()) => Ok(()),
        Err(err) => rename_without_readonly(from, to, err),
    }
}

/// The read-only **attribute** is not a permission, and this is careful to
/// bypass only the former.
///
/// `FILE_ATTRIBUTE_READONLY` is a flag on the file, routinely set by backup
/// and copy tools without anybody choosing it, and it has no counterpart in
/// the mode bits Unix would consult here. A real denial — an ACL, a share
/// permission, a locked file — reports `Access is denied` too, but leaves the
/// attribute clear, so the check below distinguishes them: the retry happens
/// only when the destination actually carries the attribute, and any other
/// cause returns the original error untouched.
///
/// The attribute is put back on the newly published file. The marking was the
/// user's and the update was fastf's; keeping both is the only answer that
/// loses neither.
#[cfg(windows)]
fn rename_without_readonly(from: &Path, to: &Path, err: io::Error) -> io::Result<()> {
    if !is_transient(&err) && err.kind() != io::ErrorKind::PermissionDenied {
        return Err(err);
    }
    let was_readonly = std::fs::symlink_metadata(to)
        .map(|meta| meta.permissions().readonly())
        .unwrap_or(false);
    if !was_readonly {
        // Denied for some other reason — an ACL, a share, an open handle.
        // Not ours to work around.
        return Err(err);
    }
    clear_readonly(to);
    let outcome = retry(|| std::fs::rename(from, to));
    if outcome.is_ok()
        && let Ok(meta) = std::fs::symlink_metadata(to)
    {
        let mut perms = meta.permissions();
        perms.set_readonly(true);
        let _ = std::fs::set_permissions(to, perms);
    }
    outcome
}

#[cfg(not(windows))]
fn rename_without_readonly(_from: &Path, _to: &Path, err: io::Error) -> io::Result<()> {
    Err(err)
}

/// Last resort after a failed removal: on Windows, clear read-only attributes
/// and try once more. On Unix there is nothing to clear, so the original error
/// stands.
///
/// These are split by platform rather than written with `#[cfg]` inside the
/// expression: the Unix arm of the inlined version collapsed to
/// `or_else(|e| Err(e))`, which is both pointless and a clippy error — invisible
/// from Windows, because that code only compiles on Linux.
#[cfg(windows)]
fn retry_without_readonly(path: &Path, err: io::Error, recursive: bool) -> io::Result<()> {
    if !is_transient(&err) && err.kind() != io::ErrorKind::PermissionDenied {
        return Err(err);
    }
    if recursive {
        clear_readonly_tree(path);
        retry(|| std::fs::remove_dir_all(path))
    } else {
        clear_readonly(path);
        std::fs::remove_file(path)
    }
}

#[cfg(not(windows))]
fn retry_without_readonly(_path: &Path, err: io::Error, _recursive: bool) -> io::Result<()> {
    Err(err)
}

/// [`std::fs::remove_file`] with transient-contention retries.
pub fn remove_file(path: &Path) -> io::Result<()> {
    match retry(|| std::fs::remove_file(path)) {
        Ok(()) => Ok(()),
        Err(err) => retry_without_readonly(path, err, false),
    }
}

/// [`std::fs::remove_dir_all`] with transient-contention retries, plus a
/// read-only sweep on Windows before the final attempt.
pub fn remove_dir_all(path: &Path) -> io::Result<()> {
    match retry(|| std::fs::remove_dir_all(path)) {
        Ok(()) => Ok(()),
        Err(err) => retry_without_readonly(path, err, true),
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

    /// The publish step of `atomic::write` is a rename **over** an existing
    /// file, and Windows refuses that when the target carries the read-only
    /// attribute. Unix `rename(2)` never looks at the target's mode, so every
    /// metadata verb — tag, note, rename, move, config, the counters — worked
    /// on Linux and failed here with `Access is denied` after the full
    /// backoff. Found by driving a real project whose `PROJECT_INFO.md` had
    /// the attribute set, which is how one arrives from a backup, from
    /// read-only media, or from a cloud client.
    #[cfg(windows)]
    #[test]
    fn a_read_only_destination_does_not_block_the_publish() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("new.tmp");
        let target = dir.path().join("PROJECT_INFO.md");
        std::fs::write(&tmp, "fresh").unwrap();
        std::fs::write(&target, "stale").unwrap();

        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&target, perms).unwrap();

        rename(&tmp, &target).expect("the publish must go through");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "fresh");
        assert!(
            std::fs::metadata(&target).unwrap().permissions().readonly(),
            "the user's marking is put back on the file that replaced it"
        );

        // Leave nothing undeletable behind for the tempdir's own cleanup.
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&target, perms).unwrap();
    }

    /// The attribute is not a permission, and only the attribute is bypassed.
    /// A rename that fails for any other reason keeps its original error —
    /// nothing here reaches past an ACL, a share permission or an open handle.
    #[cfg(windows)]
    #[test]
    fn a_denial_that_is_not_the_attribute_keeps_its_error() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("new.tmp");
        std::fs::write(&tmp, "fresh").unwrap();
        // A directory that does not exist: denied for a reason no attribute
        // sweep could ever fix.
        let target = dir.path().join("absent").join("target.md");
        let err = rename(&tmp, &target).unwrap_err();
        assert_ne!(
            err.kind(),
            io::ErrorKind::AlreadyExists,
            "the original failure is what is reported: {err}"
        );
        assert!(tmp.exists(), "and the source is left where it was");
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
