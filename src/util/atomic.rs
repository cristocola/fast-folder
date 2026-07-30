//! One atomic file write, shared by every writer that must not leave a
//! half-written file behind.
//!
//! Before this module the same temp-file-plus-rename dance was open-coded in
//! four places (`provisioning::write_atomic`, `library::write_cache`, and a
//! variant inside `assets::copy_file`) while `Config::save` and
//! `Counters::save` did a bare `fs::write` — so a crash mid-write truncated the
//! config or the ID counter.
//!
//! The temp name carries the process id and a per-process counter, so two
//! processes writing the same target never collide on the temp itself. A
//! leftover unique temp is harmless scaffolding. Recovery never sweeps files
//! by suffix; only the operation that created an exact temp path may remove it.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Suffix used in our unique scratch names. This is diagnostic only: payload
/// walkers and recovery never infer ownership from a filename suffix.
pub const TMP_SUFFIX: &str = ".tmp";

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique sibling temp path for `target`, e.g. `config.toml.4812.3.tmp`.
/// Siblings matter: the rename must stay on one filesystem to be atomic.
pub(crate) fn temp_path_for(target: &Path) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = target.as_os_str().to_owned();
    name.push(format!(".{}.{}{}", std::process::id(), seq, TMP_SUFFIX));
    PathBuf::from(name)
}

/// Exclusively claim a unique sibling temp path. The counter makes collisions
/// exceptional, while `create_new` makes PID reuse or a deliberately planted
/// filename safe: an active writer never truncates a temp it did not create.
pub(crate) fn create_temp_for(target: &Path) -> Result<(PathBuf, fs::File)> {
    const MAX_ATTEMPTS: usize = 64;
    for _ in 0..MAX_ATTEMPTS {
        let temp = temp_path_for(target);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("creating temp file {}", temp.display()));
            }
        }
    }
    anyhow::bail!(
        "could not claim a unique temp file for {} after {MAX_ATTEMPTS} attempts",
        target.display()
    )
}

/// Write `contents` to `path` atomically: full write into a sibling temp,
/// flushed and synced through the OS, then renamed over the target. A reader
/// either sees the old file or the complete new one, never a partial write.
/// This is not a storage-level power-loss durability guarantee.
///
/// `fs::rename` replaces an existing destination on both Unix and Windows
/// (std uses `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING`), so this works for
/// updates as well as first writes.
pub fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", path.display()))?;
    }

    let (tmp, file) = create_temp_for(path)?;
    let result = (|| -> Result<()> {
        {
            use std::io::Write;
            let mut writer = std::io::BufWriter::new(&file);
            writer
                .write_all(contents.as_ref())
                .with_context(|| format!("writing {}", tmp.display()))?;
            writer
                .flush()
                .with_context(|| format!("flushing {}", tmp.display()))?;
        }
        // Request an OS flush before publication. Propagate failures, while
        // making no storage-level durability promise.
        file.sync_all()
            .with_context(|| format!("syncing {}", tmp.display()))?;
        Ok(())
    })();
    drop(file);

    if let Err(err) = result {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }

    crate::util::fs_retry::rename(&tmp, path)
        .with_context(|| format!("finalizing {}", path.display()))
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
}

/// Serialize `value` as pretty JSON and write it atomically.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let raw = serde_json::to_string_pretty(value)
        .with_context(|| format!("serializing {}", path.display()))?;
    write(path, raw)
}

/// Stream one file into a unique atomic sibling, propagating write, flush, and
/// sync failures before publication.
pub fn copy(src: &Path, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", path.display()))?;
    }

    let (tmp, destination) = create_temp_for(path)?;
    let result = (|| -> Result<()> {
        let source = fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
        let mut reader = std::io::BufReader::with_capacity(1024 * 1024, source);
        let mut writer = std::io::BufWriter::with_capacity(1024 * 1024, &destination);
        std::io::copy(&mut reader, &mut writer)
            .with_context(|| format!("copying {} to {}", src.display(), tmp.display()))?;
        use std::io::Write;
        writer
            .flush()
            .with_context(|| format!("flushing {}", tmp.display()))?;
        drop(writer);
        destination
            .sync_all()
            .with_context(|| format!("syncing {}", tmp.display()))?;
        Ok(())
    })();
    drop(destination);

    if let Err(error) = result {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    crate::util::fs_retry::rename(&tmp, path)
        .with_context(|| format!("finalizing {}", path.display()))
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
}

/// True for the scratch naming scheme this module creates. This is diagnostic
/// only: cleanup and tree walking never infer ownership from a suffix.
pub fn is_temp_file(name: &str) -> bool {
    name.ends_with(TMP_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_creates_then_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");

        write(&target, b"first").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "first");

        // Replacing an existing file works (the Windows rename-over case).
        write(&target, b"second").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "second");
    }

    #[test]
    fn write_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a.txt"), b"x").unwrap();
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| is_temp_file(n))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    }

    #[test]
    fn write_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nested/deep/file.json");
        write(&target, b"{}").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
    }

    #[test]
    fn temp_paths_are_unique_per_call() {
        let target = Path::new("/tmp/thing.json");
        let a = temp_path_for(target);
        let b = temp_path_for(target);
        assert_ne!(a, b, "concurrent writers must not share a temp path");
        assert!(is_temp_file(&a.to_string_lossy()));
    }

    #[test]
    fn write_json_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("v.json");
        write_json(&target, &vec![1, 2, 3]).unwrap();
        let back: Vec<i32> = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(back, vec![1, 2, 3]);
    }
}
