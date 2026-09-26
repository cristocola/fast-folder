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

/// What a failed filesystem call means for whoever made it: whether to try
/// again, wait for the mount, or stop and say why. **Only [`Denied`], [`Locked`],
/// [`Full`], [`ReadOnly`] and [`NameRefused`] are reasons for a move to stop**;
/// the rest are waited out.
///
/// [`Denied`]: ErrorClass::Denied
/// [`Locked`]: ErrorClass::Locked
/// [`Full`]: ErrorClass::Full
/// [`ReadOnly`]: ErrorClass::ReadOnly
/// [`NameRefused`]: ErrorClass::NameRefused
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Nothing is there: ENOENT, ENOTDIR.
    Gone,
    /// Not allowed: EACCES, EPERM, `ERROR_ACCESS_DENIED`.
    Denied,
    /// A program holds it: EBUSY, ETXTBSY, a Windows sharing or lock violation.
    Locked,
    /// Worth asking again soon: EIO, EAGAIN, EINTR, ETIMEDOUT, ESTALE, a reset
    /// connection.
    Transient,
    /// The mount is not there to ask: ENOTCONN and the network errors — an
    /// rclone that restarted, an sshfs whose connection dropped.
    NotConnected,
    /// No room: ENOSPC, EDQUOT.
    Full,
    /// A read-only filesystem.
    ReadOnly,
    /// A name the filesystem will not take: EILSEQ, ENAMETOOLONG, a Windows
    /// invalid name.
    NameRefused,
    /// A folder that still holds something: ENOTEMPTY — on a cloud mount, often
    /// a listing that has not caught up yet.
    NotEmpty,
    Other,
}

/// See [`ErrorClass`].
pub fn classify(error: &io::Error) -> ErrorClass {
    if crate::util::paths::is_absence(error) {
        return ErrorClass::Gone;
    }
    if let Some(code) = error.raw_os_error()
        && let Some(class) = classify_code(code)
    {
        return class;
    }
    match error.kind() {
        io::ErrorKind::PermissionDenied => ErrorClass::Denied,
        io::ErrorKind::ResourceBusy | io::ErrorKind::ExecutableFileBusy => ErrorClass::Locked,
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => ErrorClass::Full,
        io::ErrorKind::ReadOnlyFilesystem => ErrorClass::ReadOnly,
        io::ErrorKind::DirectoryNotEmpty => ErrorClass::NotEmpty,
        io::ErrorKind::InvalidFilename => ErrorClass::NameRefused,
        io::ErrorKind::NotConnected
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::HostUnreachable
        | io::ErrorKind::NetworkUnreachable
        | io::ErrorKind::NetworkDown => ErrorClass::NotConnected,
        io::ErrorKind::TimedOut
        | io::ErrorKind::Interrupted
        | io::ErrorKind::WouldBlock
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::StaleNetworkFileHandle => ErrorClass::Transient,
        _ => ErrorClass::Other,
    }
}

#[cfg(unix)]
fn classify_code(code: i32) -> Option<ErrorClass> {
    Some(match code {
        libc::EACCES | libc::EPERM => ErrorClass::Denied,
        libc::EBUSY | libc::ETXTBSY => ErrorClass::Locked,
        libc::EIO
        | libc::EAGAIN
        | libc::EINTR
        | libc::ETIMEDOUT
        | libc::ESTALE
        | libc::ECONNRESET => ErrorClass::Transient,
        libc::ENOTCONN
        | libc::ESHUTDOWN
        | libc::ECONNABORTED
        | libc::EHOSTDOWN
        | libc::EHOSTUNREACH
        | libc::ENETDOWN
        | libc::ENETUNREACH
        | libc::ENODEV
        | libc::ENXIO => ErrorClass::NotConnected,
        libc::ENOSPC | libc::EDQUOT => ErrorClass::Full,
        libc::EROFS => ErrorClass::ReadOnly,
        libc::EILSEQ | libc::ENAMETOOLONG => ErrorClass::NameRefused,
        libc::ENOTEMPTY => ErrorClass::NotEmpty,
        _ => return None,
    })
}

#[cfg(windows)]
fn classify_code(code: i32) -> Option<ErrorClass> {
    Some(match code {
        // ERROR_ACCESS_DENIED: a permission, or a file somebody holds while it
        // is deleted — the retries in this module tell the two apart.
        5 => ErrorClass::Denied,
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_USER_MAPPED_FILE.
        32 | 33 | 1224 => ErrorClass::Locked,
        // ERROR_NOT_READY, ERROR_IO_DEVICE, ERROR_SEM_TIMEOUT, ERROR_UNEXP_NET_ERR.
        21 | 1117 | 121 | 59 => ErrorClass::Transient,
        // ERROR_BAD_NETPATH, ERROR_NETNAME_DELETED, ERROR_BAD_NET_NAME,
        // ERROR_NETWORK_UNREACHABLE, ERROR_REM_NOT_LIST, ERROR_DEV_NOT_EXIST.
        53 | 64 | 67 | 1231 | 51 | 55 => ErrorClass::NotConnected,
        // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL, ERROR_DISK_QUOTA_EXCEEDED.
        39 | 112 | 1295 => ErrorClass::Full,
        // ERROR_WRITE_PROTECT.
        19 => ErrorClass::ReadOnly,
        // ERROR_INVALID_NAME, ERROR_FILENAME_EXCED_RANGE.
        123 | 206 => ErrorClass::NameRefused,
        // ERROR_DIR_NOT_EMPTY.
        145 => ErrorClass::NotEmpty,
        _ => return None,
    })
}

#[cfg(not(any(unix, windows)))]
fn classify_code(_code: i32) -> Option<ErrorClass> {
    None
}

/// How long a filesystem that is not connected — a mount that dropped, an
/// rclone that restarts — is waited for before a call gives up on it.
pub const MOUNT_WAIT: Duration = Duration::from_secs(120);

/// The schedule for an error worth asking again ([`ErrorClass::Transient`],
/// [`ErrorClass::Locked`]): 0.2 s doubling to 5 s, six more tries, about
/// eleven seconds in all.
const CLASS_BACKOFF_MS: [u64; 6] = [200, 400, 800, 1600, 3200, 5000];

/// Run `op`, about `path`, **by what its error means** ([`classify`]): an
/// error worth asking again is asked again with backoff; a mount that is not
/// connected is waited for, up to [`MOUNT_WAIT`], until the same mount
/// answers again ([`wait_for_mount`]); anything else — a permission, no
/// room, a name refused, "nothing there" — is answered at once, since asking
/// again changes nothing. On every platform: EIO from a mount that restarts,
/// ESTALE, and a lagging ENOTCONN are Linux's too, and 3.13 never retried
/// anything on unix.
pub fn with_retry<T>(path: &Path, op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    with_retry_for(path, mount_wait(), op)
}

/// [`MOUNT_WAIT`] — or a second, under the `fs:short-mount-wait` decision, so
/// a test reaches what happens once the wait runs out.
pub fn mount_wait() -> Duration {
    if crate::util::faults::is_armed("fs:short-mount-wait") {
        Duration::from_secs(1)
    } else {
        MOUNT_WAIT
    }
}

/// [`with_retry`], waiting at most `mount_wait` for a mount.
pub fn with_retry_for<T>(
    path: &Path,
    mount_wait: Duration,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut tries = 0;
    let deadline = std::time::Instant::now() + mount_wait;
    // The mount to wait for, read at the first "not connected" rather than
    // before every call: a FUSE mount whose daemon went away is still in the
    // mount table then, so it names the same mount — and reading the table
    // on every walk's look and every unlink cost more than a local unlink
    // (a 20 000-file delete on a local disk, 0.4 s → 1.1 s).
    let mut mount: Option<Option<String>> = None;
    loop {
        let error = match op() {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        match classify(&error) {
            ErrorClass::Transient | ErrorClass::Locked if tries < CLASS_BACKOFF_MS.len() => {
                crate::util::log::debug(format!(
                    "{}: {error}; asking again",
                    crate::util::paths::display_path(path)
                ));
                std::thread::sleep(Duration::from_millis(CLASS_BACKOFF_MS[tries]));
                tries += 1;
            }
            ErrorClass::NotConnected if std::time::Instant::now() < deadline => {
                crate::util::log::info(format!(
                    "{}: {error}; waiting for its mount",
                    crate::util::paths::display_path(path)
                ));
                let mount = mount.get_or_insert_with(|| crate::util::fs_kind::mount_identity(path));
                wait_for_mount(path, mount.as_deref(), deadline);
            }
            _ => return Err(error),
        }
    }
}

/// **One sentence for a person**: what kind of problem stopped something,
/// and what to do about it — when the error an `anyhow` chain holds is one of
/// the few that stop a move (a permission, a lock, no room, a read-only drive,
/// a name refused) or a mount that stopped answering. The errno and the
/// chain are for the report and the log; this is what comes first.
pub fn explain(error: &anyhow::Error) -> Option<&'static str> {
    sentence(class_of(error)?)
}

/// [`explain`]'s sentence for an error of `class`, if it is one to say.
pub fn sentence(class: ErrorClass) -> Option<&'static str> {
    Some(match class {
        ErrorClass::Denied => {
            "fastf is not allowed to change something there (no permission): give your \
             account write access to it, then try again"
        }
        ErrorClass::Locked => {
            "a program has a file there open and locked: close it, then try again"
        }
        ErrorClass::Full => "the drive is full: free some space, then try again",
        ErrorClass::ReadOnly => "the drive is read-only, so nothing there can be changed",
        ErrorClass::NameRefused => {
            "the drive will not take one of the names: rename it, then try again"
        }
        ErrorClass::NotConnected => {
            "the drive stopped answering and did not come back while fastf waited: run it \
             again once the mount is back"
        }
        ErrorClass::Transient => {
            "the drive kept failing (input/output errors): check the mount, then try again"
        }
        ErrorClass::Gone | ErrorClass::NotEmpty | ErrorClass::Other => return None,
    })
}

/// [`explain`]'s sentence, then the error as it came — or the error alone.
pub fn explained(error: &anyhow::Error) -> String {
    match explain(error) {
        Some(sentence) => format!("{sentence}. ({error:#})"),
        None => format!("{error:#}"),
    }
}

/// The class ([`classify`]) of the filesystem error an `anyhow` chain holds,
/// if it holds one.
pub fn class_of(error: &anyhow::Error) -> Option<ErrorClass> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<io::Error>())
        .map(classify)
}

/// A folder's whole listing, as paths, asked again as a whole — by what the
/// error means ([`with_retry`]) — when the filesystem fails at the start or
/// part of the way through: a listing that stopped half-way cannot say what
/// it did not list, and an sshfs that drops mid-listing answers the next one
/// whole. `before` runs ahead of each attempt (a failpoint).
pub fn list_dir(
    dir: &Path,
    before: impl Fn() -> io::Result<()>,
) -> io::Result<Vec<std::path::PathBuf>> {
    with_retry(dir, || {
        before()?;
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            paths.push(entry?.path());
        }
        Ok(paths)
    })
}

/// Wait until `path` answers again — present or absent, anything but "not
/// connected" — on the mount it was on (`mount`, `util::fs_kind::
/// mount_identity`), or until `deadline`. An unmounted FUSE mount point is an
/// empty folder that answers "nothing there" for everything under it: that
/// is not the mount being back.
pub fn wait_for_mount(path: &Path, mount: Option<&str>, deadline: std::time::Instant) {
    while std::time::Instant::now() < deadline {
        let answers = match crate::util::paths::presence(path) {
            crate::util::paths::Presence::Unknown(error) => {
                classify(&error) != ErrorClass::NotConnected
            }
            _ => true,
        };
        let same_mount =
            mount.is_none() || crate::util::fs_kind::mount_identity(path).as_deref() == mount;
        if answers && same_mount {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Backoff schedule between attempts. Total worst-case wait ≈ 310 ms, which is
/// well past a typical antivirus scan window while staying imperceptible.
const BACKOFF_MS: [u64; 5] = [10, 20, 40, 80, 160];

/// The longer schedule for renaming a folder, ≈ 2.5 s. A folder whose every
/// file was just read or written — a staging tree about to be published, a
/// source about to leave the library — is exactly what an indexer or a virus
/// scanner is still holding, and the short schedule gave up on it: at publish
/// that threw away a verified copy.
const DIR_BACKOFF_MS: [u64; 7] = [20, 50, 100, 200, 400, 700, 1000];

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
fn retry<T>(op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    retry_on(&BACKOFF_MS, op)
}

fn retry_on<T>(schedule: &[u64], mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut last = match op() {
        Ok(value) => return Ok(value),
        Err(err) if is_transient(&err) => err,
        Err(err) => return Err(err),
    };
    for &delay in schedule {
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

/// [`std::fs::rename`] of a **folder**, with the longer retry schedule and no
/// read-only fallback: the attribute means nothing on a folder's name, and a
/// folder that stays refused is held by a program — which is what the caller
/// has to say, see [`describe_rename_error`].
pub fn rename_dir(from: &Path, to: &Path) -> io::Result<()> {
    retry_on(&DIR_BACKOFF_MS, || std::fs::rename(from, to))
}

/// [`std::fs::remove_dir`] of an empty folder, with transient-contention
/// retries.
pub fn remove_dir(path: &Path) -> io::Result<()> {
    retry(|| std::fs::remove_dir(path))
}

/// What a failed folder rename means, in words. On Windows a folder cannot be
/// renamed while any file in it is open without delete sharing, or while it is
/// some program's working folder, and the error only says `Access is denied`.
pub fn describe_rename_error(err: &io::Error) -> String {
    #[cfg(windows)]
    if is_transient(err) {
        return format!(
            "a program has a file in it open, or is working in it ({err}); \
             close it and try again"
        );
    }
    err.to_string()
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

    /// A call that fails the way a mount does is asked again by what the
    /// error means: transient and locked with backoff, and not again when
    /// asking again changes nothing.
    #[cfg(unix)]
    #[test]
    fn a_call_is_retried_by_what_its_error_means() {
        let temp = tempfile::tempdir().unwrap();
        let mut left = 2;
        let answer = with_retry(temp.path(), || {
            if left > 0 {
                left -= 1;
                Err(io::Error::from_raw_os_error(libc::EIO))
            } else {
                Ok(7)
            }
        });
        assert_eq!(answer.unwrap(), 7, "EIO twice, then an answer");

        let mut calls = 0;
        let denied = with_retry(temp.path(), || {
            calls += 1;
            Err::<(), _>(io::Error::from_raw_os_error(libc::EACCES))
        });
        assert!(denied.is_err());
        assert_eq!(calls, 1, "a permission is not asked again");

        let mut calls = 0;
        let gone = with_retry(temp.path(), || {
            calls += 1;
            Err::<(), _>(io::Error::from_raw_os_error(libc::ENOENT))
        });
        assert_eq!(classify(&gone.unwrap_err()), ErrorClass::Gone);
        assert_eq!(calls, 1);
    }

    /// The errors that stop something get one sentence first, naming what
    /// to do; the rest are said as they came.
    #[cfg(unix)]
    #[test]
    fn a_stopping_error_is_explained_in_one_sentence() {
        let denied =
            anyhow::Error::new(io::Error::from_raw_os_error(libc::EACCES)).context("removing /a/b");
        let said = explained(&denied);
        assert!(said.starts_with("fastf is not allowed"), "{said}");
        assert!(said.contains("removing /a/b"), "the details stay: {said}");
        let other = anyhow::anyhow!("something else");
        assert_eq!(explained(&other), "something else");
    }

    /// Not connected: waited for, and asked again once the path answers.
    #[cfg(unix)]
    #[test]
    fn a_mount_that_drops_is_waited_for() {
        let temp = tempfile::tempdir().unwrap();
        let mut left = 1;
        let started = std::time::Instant::now();
        let answer = with_retry_for(temp.path(), Duration::from_secs(5), || {
            if left > 0 {
                left -= 1;
                Err(io::Error::from_raw_os_error(libc::ENOTCONN))
            } else {
                Ok(())
            }
        });
        assert!(answer.is_ok());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// The classes a move acts on, from the errors a mount really gives.
    #[cfg(unix)]
    #[test]
    fn errors_are_classed_by_what_to_do_next() {
        let class = |code| classify(&io::Error::from_raw_os_error(code));
        assert_eq!(class(libc::ENOENT), ErrorClass::Gone);
        assert_eq!(class(libc::ENOTDIR), ErrorClass::Gone);
        assert_eq!(class(libc::EACCES), ErrorClass::Denied);
        assert_eq!(class(libc::EBUSY), ErrorClass::Locked);
        assert_eq!(class(libc::EIO), ErrorClass::Transient);
        assert_eq!(class(libc::ESTALE), ErrorClass::Transient);
        assert_eq!(class(libc::ENOTCONN), ErrorClass::NotConnected);
        assert_eq!(class(libc::ENOSPC), ErrorClass::Full);
        assert_eq!(class(libc::EROFS), ErrorClass::ReadOnly);
        assert_eq!(class(libc::ENAMETOOLONG), ErrorClass::NameRefused);
        assert_eq!(class(libc::ENOTEMPTY), ErrorClass::NotEmpty);
        assert_eq!(
            classify(&io::Error::other("something else")),
            ErrorClass::Other
        );
    }
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
