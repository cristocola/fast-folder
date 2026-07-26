//! A cross-process lock over the fastf data directory.
//!
//! The global ID counter is a read-modify-write across two files
//! (`counters.toml` plus a scan of every base), and until now nothing
//! serialized it between processes. The browser UI's `WRITE_LOCK` is an
//! in-process `Mutex`, so it cannot see a `fastf new` running in a terminal —
//! which is the documented workflow. Ten concurrent creates reliably minted
//! duplicate IDs.
//!
//! This lock closes that. It is held across the whole plan→create→save span, so
//! ID allocation and the folder claim are one indivisible step no matter how
//! many fastf processes are running.
//!
//! **Implementation:** no FFI on Windows — opening the lock file with
//! `share_mode(0)` makes `CreateFile` itself the mutual-exclusion primitive, and
//! the OS drops the lock when the process dies (including a hard kill), so a
//! crash can never strand it. On Unix the same guarantee comes from `flock`,
//! which the kernel likewise releases on exit. `libc` is already a Unix-only
//! dependency; nothing new is pulled in.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Filename of the data-directory lock.
pub const LOCK_FILENAME: &str = ".fastf.lock";

/// How long to wait for another process before giving up. Generous enough to
/// cover a slow create on a network base, short enough that a genuinely stuck
/// process reports rather than hanging forever.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Poll interval while waiting. Short enough to feel instant on release.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// An acquired lock. Releasing happens on drop (the file handle closes), and
/// the OS releases it on process death, so there is no stale-lock recovery path
/// to get wrong.
#[derive(Debug)]
pub struct DataLock {
    _file: File,
    path: PathBuf,
}

impl DataLock {
    /// Lock the data directory, waiting up to the default timeout.
    pub fn acquire() -> Result<Self> {
        Self::acquire_at(&lock_path(), DEFAULT_TIMEOUT)
    }

    /// Lock `path`, waiting up to `timeout`. Exposed for tests.
    pub fn acquire_at(path: &Path, timeout: Duration) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }

        let deadline = Instant::now() + timeout;
        loop {
            match try_lock(path) {
                Ok(Some(file)) => {
                    return Ok(Self {
                        _file: file,
                        path: path.to_path_buf(),
                    });
                }
                // Held by someone else — wait and retry.
                Ok(None) => {}
                Err(err) => {
                    return Err(err).with_context(|| format!("locking {}", path.display()));
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "another fastf process is busy (waited {}s for {}).\n\
                     If no other fastf is running, delete the lock file and retry.",
                    timeout.as_secs(),
                    path.display()
                );
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Path of the held lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The data-directory lock path.
pub fn lock_path() -> PathBuf {
    crate::util::paths::install_dir().join(LOCK_FILENAME)
}

/// One non-blocking attempt. `Ok(None)` means "held by another process".
///
/// Windows: `share_mode(0)` denies all sharing, so a second `CreateFile` on the
/// same path fails with `ERROR_SHARING_VIOLATION` while we hold it.
#[cfg(windows)]
fn try_lock(path: &Path) -> Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;

    /// The file is open in another process and shares nothing.
    const ERROR_SHARING_VIOLATION: i32 = 32;
    /// Another handle exists with an incompatible sharing mode.
    const ERROR_ACCESS_DENIED: i32 = 5;

    match OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .share_mode(0)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(err)
            if matches!(
                err.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION) | Some(ERROR_ACCESS_DENIED)
            ) =>
        {
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

/// One non-blocking `flock` attempt. `Ok(None)` means another process holds it.
#[cfg(unix)]
fn try_lock(path: &Path) -> Result<Option<File>> {
    use std::os::unix::io::AsRawFd;

    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: `file` owns a valid fd for the duration of the call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Some(file));
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EWOULDBLOCK) => Ok(None),
        _ => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_release_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILENAME);

        let lock = DataLock::acquire_at(&path, Duration::from_secs(1)).unwrap();
        assert_eq!(lock.path(), path);
        drop(lock);

        // Released — immediately re-acquirable.
        DataLock::acquire_at(&path, Duration::from_millis(200)).unwrap();
    }

    #[test]
    fn creates_missing_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deep").join(LOCK_FILENAME);
        DataLock::acquire_at(&path, Duration::from_millis(500)).unwrap();
        assert!(path.exists());
    }

    /// The whole point: a second holder must be excluded. Threads share a
    /// process, so this checks the OS primitive rather than a `Mutex` — on
    /// Windows `share_mode(0)` is per-handle and on Unix `flock` is per-fd, so
    /// both genuinely exclude here.
    #[test]
    fn second_acquire_times_out_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILENAME);

        let held = DataLock::acquire_at(&path, Duration::from_secs(1)).unwrap();
        let err = DataLock::acquire_at(&path, Duration::from_millis(150)).unwrap_err();
        assert!(
            err.to_string().contains("another fastf process is busy"),
            "unexpected error: {err}"
        );

        drop(held);
        DataLock::acquire_at(&path, Duration::from_millis(500))
            .expect("lock must be available once released");
    }
}
