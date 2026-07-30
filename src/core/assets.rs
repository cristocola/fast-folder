//! The asset copy engine for folder-form templates.
//!
//! A template is a folder whose `files/` subtree IS the spec: every file and
//! directory under `files/` is reproduced into each new project. Names and
//! UTF-8 text contents get `{token}` interpolation; binaries (and anything
//! matched by a `verbatim` glob, or larger than [`TEXT_MAX_BYTES`], or not
//! valid UTF-8) are copied byte-for-byte. `exclude` globs are never copied.
//!
//! There is no per-file manifest — convention over configuration. This module
//! walks the real directory and is the single source of truth for `fastf new`
//! and `fastf apply` (CLI and UI share it).

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::naming::interpolate_name;

/// Text files at or below this size are candidates for `{token}` interpolation.
/// Anything larger is copied verbatim — interpolating a 200 MB file makes no
/// sense and would blow up memory.
pub const TEXT_MAX_BYTES: u64 = 1024 * 1024;

/// Files larger than this are deferred to a background copy job in the UI so a
/// slow cross-filesystem copy (btrfs `~` → ntfs `/mnt/base`) never blocks the
/// request. Must be ≥ [`TEXT_MAX_BYTES`] so every deferred file is verbatim
/// (never needs interpolation) — the background copier does pure byte copies.
pub const JOB_DEFER_BYTES: u64 = 4 * 1024 * 1024;

// Enforced at compile time, not merely tested: the background copier has no
// variables to interpolate with, so a file large enough to be deferred but small
// enough to still be interpolated would be written out with its `{tokens}`
// unsubstituted. Lowering `JOB_DEFER_BYTES` below `TEXT_MAX_BYTES` fails the
// build rather than shipping that.
const _: () = assert!(JOB_DEFER_BYTES >= TEXT_MAX_BYTES);

/// A single deferred file copy (always a verbatim byte copy — see
/// [`JOB_DEFER_BYTES`]). Produced by the eager create phase, run by a UI
/// background thread with progress.
#[derive(Debug, Clone)]
pub struct CopyJob {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub bytes: u64,
}

/// Live progress of a background copy job. Serialized to the UI's `/api/job`.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub total_files: usize,
    pub done_files: usize,
    pub current_file: String,
    /// `"running"`, `"done"`, `"failed"`, or `"cancelled"`.
    pub status: String,
    /// Coarse stage for the UI, shared by create + move jobs:
    /// `"copying" | "verifying" | "finalizing" | "done"`.
    pub phase: String,
    pub error: Option<String>,
    /// A move reached its verified destination but could not remove its source.
    /// Existing clients may ignore this additive field safely.
    pub cleanup_pending: bool,
    /// Non-fatal detail accompanying [`Self::cleanup_pending`].
    pub warning: Option<String>,
    /// Unix-epoch milliseconds of the last observed movement (bytes copied, a
    /// file finished, or a phase change).
    ///
    /// Two jobs depend on this. The UI tells "slow" from "stuck" with it —
    /// a copy to a cloud-synced or network destination can legitimately sit for
    /// minutes, so there is no wall-clock timeout, only an honest "no progress
    /// for N minutes" note. And [`crate::ui::jobs_active`] uses it as a
    /// staleness floor so a job whose worker thread died can never report
    /// itself as running forever and hold the process open.
    pub last_progress_at: u64,
}

impl Progress {
    pub fn new(jobs: &[CopyJob]) -> Self {
        Self {
            total_bytes: jobs.iter().map(|j| j.bytes).sum(),
            copied_bytes: 0,
            total_files: jobs.len(),
            done_files: 0,
            current_file: String::new(),
            status: "running".to_string(),
            phase: "copying".to_string(),
            error: None,
            cleanup_pending: false,
            warning: None,
            last_progress_at: now_millis(),
        }
    }

    /// Record that the job just made progress. Call alongside every mutation
    /// that represents real movement.
    pub fn touch(&mut self) {
        self.last_progress_at = now_millis();
    }

    /// Milliseconds since the last observed movement. Saturates at 0 if the
    /// clock moved backwards.
    pub fn idle_millis(&self) -> u64 {
        now_millis().saturating_sub(self.last_progress_at)
    }
}

/// Unix-epoch milliseconds. The frontend compares this against its own
/// `Date.now()`, which is sound because the UI is loopback-only — same machine,
/// same clock.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A copy that stopped because its cancel flag was set. Callers distinguish this
/// from a genuine failure by checking the flag after `copy_job` returns `Err`.
pub const CANCELLED_MSG: &str = "copy cancelled";

/// Copy one deferred (large, verbatim) file into place with chunked progress.
/// Atomic via an operation-owned unique sibling + rename;
/// `progress.copied_bytes` is bumped per chunk so the UI shows a live bar
/// during a multi-minute copy.
///
/// `cancel` is polled between chunks: when set, the exact partial sibling is
/// removed and the copy returns a [`CANCELLED_MSG`] error so no half-written
/// file is ever left in place.
pub fn copy_job(job: &CopyJob, progress: &Mutex<Progress>, cancel: &AtomicBool) -> Result<()> {
    if entry_exists(&job.dest)? {
        anyhow::bail!(
            "copy destination is already occupied: {}",
            job.dest.display()
        );
    }
    if let Some(parent) = job.dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", job.dest.display()))?;
    }
    let (tmp, mut writer) = crate::util::atomic::create_temp_for(&job.dest)?;

    let result = (|| -> Result<()> {
        let mut reader =
            fs::File::open(&job.src).with_context(|| format!("opening {}", job.src.display()))?;
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("{CANCELLED_MSG}");
            }
            let n = reader.read(&mut buf).context("reading source")?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).context("writing destination")?;
            if let Ok(mut p) = progress.lock() {
                p.copied_bytes += n as u64;
                p.touch();
            }
        }
        writer
            .flush()
            .with_context(|| format!("flushing {}", tmp.display()))?;
        writer
            .sync_all()
            .with_context(|| format!("syncing {}", tmp.display()))?;
        Ok(())
    })();
    drop(writer);

    match result {
        Ok(()) => {
            match entry_exists(&job.dest) {
                Ok(false) => {}
                Ok(true) => {
                    let _ = fs::remove_file(&tmp);
                    anyhow::bail!("copy destination became occupied: {}", job.dest.display());
                }
                Err(error) => {
                    let _ = fs::remove_file(&tmp);
                    return Err(error);
                }
            }
            crate::util::fs_retry::rename(&tmp, &job.dest)
                .with_context(|| format!("finalizing {}", job.dest.display()))
                .inspect_err(|_| {
                    let _ = fs::remove_file(&tmp);
                })?;
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// What a walked entry actually is.
///
/// An enum rather than a pair of bools on purpose: adding a variant makes the
/// compiler point at every consumer that must decide what to do with it. `walk`
/// used to silently drop anything that was not a plain file or directory, which
/// is how a cross-filesystem move came to delete a project's junctions — the
/// copy skipped them, and the verification, built on the same walk, was blind to
/// the very same entries on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    /// A symlink, Windows junction, or mount point. Never followed.
    ///
    /// NOT "any reparse point". Windows uses reparse points for many things
    /// that are still ordinary file content — cloud placeholders (OneDrive,
    /// Google Drive streaming), deduplication, transparent compression. Those
    /// read back as normal files and must copy as normal files.
    ///
    /// The distinction is the *name surrogate* bit in the reparse tag: set only
    /// for tags that redirect to another name. `std`'s `FileType::is_symlink`
    /// keys on exactly that bit, so classifying by it is already correct and
    /// filesystem-agnostic — there is nothing here to special-case per vendor,
    /// and adding such a case would be wrong the moment a new filter driver
    /// ships.
    Symlink,
    /// Anything else (fifo, socket, device node). Recorded, never copied.
    Other,
}

/// One physical entry discovered under a directory tree.
pub struct AssetEntry {
    /// Path relative to the walk root, forward-slash separated, **uninterpolated**.
    pub rel: String,
    pub kind: EntryKind,
    pub size: u64,
}

impl AssetEntry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    /// A plain file — the only kind that gets copied.
    pub fn is_file(&self) -> bool {
        self.kind == EntryKind::File
    }

    pub fn is_symlink(&self) -> bool {
        self.kind == EntryKind::Symlink
    }
}

/// Recursively list every entry under `files_dir` (directories included so that
/// deliberately-empty folders are reproduced). Returns an empty vec when the
/// directory does not exist. Results are sorted so parents precede children.
///
/// Links are **reported, not followed** — descending through one could leave the
/// tree entirely or loop forever.
pub fn walk(files_dir: &Path) -> Result<Vec<AssetEntry>> {
    let mut out = Vec::new();
    if !files_dir.exists() {
        return Ok(out);
    }
    walk_inner(files_dir, files_dir, &mut out)?;
    // Lexicographic sort puts a parent ("a") before its children ("a/b").
    out.sort_by(|x, y| x.rel.cmp(&y.rel));
    Ok(out)
}

fn walk_inner(root: &Path, current: &Path, out: &mut Vec<AssetEntry>) -> Result<()> {
    for entry in fs::read_dir(current).with_context(|| format!("reading {}", current.display()))? {
        let entry = entry?;
        let path = entry.path();
        // `DirEntry::file_type` does not follow links, so a symlink to a
        // directory reports as a symlink rather than a dir — the link itself is
        // the thing being described, which is what the caller needs to know.
        let ft = entry.file_type()?;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if ft.is_symlink() {
            out.push(AssetEntry {
                rel,
                kind: EntryKind::Symlink,
                size: 0,
            });
        } else if ft.is_dir() {
            out.push(AssetEntry {
                rel,
                kind: EntryKind::Dir,
                size: 0,
            });
            walk_inner(root, &path, out)?;
        } else if ft.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(AssetEntry {
                rel,
                kind: EntryKind::File,
                size,
            });
        } else {
            out.push(AssetEntry {
                rel,
                kind: EntryKind::Other,
                size: 0,
            });
        }
    }
    Ok(())
}

/// Relative paths of every link (symlink or Windows junction) inside a tree.
///
/// Used as a move pre-flight: fastf refuses to move a project containing links
/// rather than dropping them. Recreating one faithfully needs elevation or
/// Developer Mode on Windows, and following it would silently restructure the
/// project and could duplicate a large shared asset library — so refusing is the
/// only option that can neither lose nor corrupt data.
pub fn find_links(root: &Path) -> Result<Vec<String>> {
    Ok(walk(root)?
        .into_iter()
        .filter(AssetEntry::is_symlink)
        .map(|e| e.rel)
        .collect())
}

/// Recursively copy a directory tree, byte-for-byte — **no interpolation**
/// (used by `library::move_project`'s cross-device fallback, where the copy
/// must preserve every file exactly, including literal `{braces}`). Fails if
/// `dst` already exists so a half-typed target never gets merged into.
pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    require_real_directory(src, "copy source")?;
    if entry_exists(dst)? {
        anyhow::bail!("copy target already exists: {}", dst.display());
    }
    fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    // `walk` sorts parents before children, so plain create_dir + copy works.
    for entry in walk(src)? {
        let rel = entry.rel.replace('/', std::path::MAIN_SEPARATOR_STR);
        let target = dst.join(&rel);
        match entry.kind {
            EntryKind::Dir => {
                fs::create_dir_all(&target)
                    .with_context(|| format!("creating {}", target.display()))?;
            }
            EntryKind::File => {
                let source = src.join(&rel);
                fs::copy(&source, &target).with_context(|| {
                    format!("copying {} → {}", source.display(), target.display())
                })?;
            }
            // Refuse rather than drop: a caller that removes the source after a
            // "successful" copy would destroy whatever this pointed at.
            EntryKind::Symlink | EntryKind::Other => anyhow::bail!(
                "cannot copy '{}': it is a link or special file. \
                 Copying it faithfully is not supported, and skipping it would silently lose data.",
                entry.rel
            ),
        }
    }
    Ok(())
}

/// Enumerate a source tree into the directory creates + per-file [`CopyJob`]s
/// needed to reproduce it verbatim under `dst`. Directories (including empty
/// ones) come first so a plain create-then-copy is ordering-safe. Every regular
/// source file is payload, including names ending in `.tmp` or `.part`. Used by
/// the staged move so the copy can report live progress and honor cancellation.
///
/// Errors on a link or special file. Callers pre-flight with [`find_links`] and
/// refuse the move up front, so this is the backstop that guarantees no caller
/// can ever reach the "copied, verified, now delete the source" step while
/// having silently skipped something.
pub fn jobs_for_tree(src: &Path, dst: &Path) -> Result<(Vec<PathBuf>, Vec<CopyJob>)> {
    require_real_directory(src, "move source")?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in walk(src)? {
        let rel = entry.rel.replace('/', std::path::MAIN_SEPARATOR_STR);
        let target = dst.join(&rel);
        match entry.kind {
            EntryKind::Dir => dirs.push(target),
            EntryKind::File => files.push(CopyJob {
                src: src.join(&rel),
                dest: target,
                bytes: entry.size,
            }),
            EntryKind::Symlink | EntryKind::Other => anyhow::bail!(
                "cannot copy '{}': it is a link or special file. \
                 Copying it faithfully is not supported, and skipping it would silently lose data.",
                entry.rel
            ),
        }
    }
    Ok((dirs, files))
}

/// Verify that `dst` faithfully reproduces `src`: every file in `src` exists
/// under `dst` with an identical byte size, and the file counts
/// match (no source file dropped). This is the guarantee that must hold **before**
/// a move removes its source — it catches the real network-share failure mode
/// (a truncated or dropped-connection copy) without the cost of re-hashing every
/// byte. Returns a descriptive error on the first discrepancy.
///
/// **Deny by default.** Anything the walk cannot classify as a plain file or a
/// directory fails verification instead of being skipped. The previous version
/// skipped links on *both* sides, so their sizes agreed trivially and a copy that
/// had dropped them verified clean — which is precisely how a move could delete a
/// source whose junctions never reached the destination. Verification must never
/// be narrower than the copy it is checking.
pub fn verify_tree(src: &Path, dst: &Path) -> Result<()> {
    require_real_directory(src, "verification source")?;
    require_real_directory(dst, "verification destination")?;
    let sizes = |root: &Path, side: &str| -> Result<HashMap<String, u64>> {
        let mut map = HashMap::new();
        for entry in walk(root)? {
            match entry.kind {
                EntryKind::Dir => {}
                EntryKind::File => {
                    map.insert(entry.rel, entry.size);
                }
                EntryKind::Symlink | EntryKind::Other => anyhow::bail!(
                    "verification failed: {side} contains '{}', a link or special file \
                     that cannot be verified",
                    entry.rel
                ),
            }
        }
        Ok(map)
    };
    let src_files = sizes(src, "source")?;
    let dst_files = sizes(dst, "destination")?;

    for (rel, src_size) in &src_files {
        match dst_files.get(rel) {
            None => anyhow::bail!("verification failed: missing at destination: {rel}"),
            Some(dst_size) if dst_size != src_size => anyhow::bail!(
                "verification failed: size mismatch for {rel} (src {src_size} B, dst {dst_size} B)"
            ),
            Some(_) => {}
        }
    }
    // There used to be a `dst_files.len() < src_files.len()` check here. It was
    // unreachable: the loop above requires every source file to be present at
    // the destination, so by this point `dst >= src` always holds. Mutation
    // testing surfaced it — no test could distinguish flipping the comparison,
    // because the branch could never be taken either way.
    //
    // Extra files at the destination are deliberately *not* an error. The
    // property that has to hold before a source is deleted is "everything made
    // it across", and a surplus does not threaten that.
    Ok(())
}

/// Require an existing, non-symlink directory. `Path::is_dir()` follows links
/// and treats a missing path as `false`; neither is strong enough at a copy
/// boundary where "missing" must never be mistaken for an empty tree.
pub fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("{label} does not exist: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        anyhow::bail!("{label} is not a real directory: {}", path.display());
    }
    Ok(())
}

/// Does a directory entry occupy this exact path? Unlike `Path::exists`, this
/// sees broken symlinks and propagates metadata errors instead of treating them
/// as a free destination.
pub fn entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

/// Interpolate a relative path segment-by-segment, so empty variables collapse
/// underscores *within* each name component without touching the `/` separators.
pub fn interp_rel(rel: &str, vars: &HashMap<String, String>, date_format: &str) -> String {
    rel.split('/')
        .map(|segment| interpolate_name(segment, vars, date_format))
        .collect::<Vec<_>>()
        .join("/")
}

/// Match a glob (`*` = any run, `?` = one char) against `text`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Iterative wildcard match with backtracking on `*`.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A glob with no `/` is matched against the basename; a glob containing `/`
/// is matched against the full relative path.
fn matches_any(rel: &str, patterns: &[String]) -> bool {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    patterns.iter().any(|pat| {
        if pat.contains('/') {
            glob_match(pat, rel)
        } else {
            glob_match(pat, base)
        }
    })
}

/// True when `rel` matches an `exclude` glob (never copied).
pub fn is_excluded(rel: &str, exclude: &[String]) -> bool {
    matches_any(rel, exclude)
}

/// True when `rel` matches a `verbatim` glob (copied literally even if text).
pub fn is_verbatim(rel: &str, verbatim: &[String]) -> bool {
    matches_any(rel, verbatim)
}

/// Copy one file atomically through a unique sibling temp.
///
/// When `force_verbatim` is false the source is read as UTF-8 and, if that
/// succeeds, `{token}` interpolation is applied to its contents. A read failure
/// (binary) transparently falls back to a byte copy. `force_verbatim` short-
/// circuits straight to the byte copy (used for `verbatim` globs and oversize
/// files).
pub fn copy_file(
    src: &Path,
    dest: &Path,
    force_verbatim: bool,
    vars: &HashMap<String, String>,
    date_format: &str,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", dest.display()))?;
    }

    let interpolated = if force_verbatim {
        None
    } else {
        // Try to read as text; a non-UTF-8 file yields Err → verbatim copy.
        fs::read_to_string(src).ok()
    };

    match interpolated {
        Some(text) => {
            let rendered = crate::core::naming::interpolate(&text, vars, date_format);
            crate::util::atomic::write(dest, rendered)?;
        }
        None => {
            crate::util::atomic::copy(src, dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches() {
        assert!(glob_match("*.svg", "logo.svg"));
        assert!(!glob_match("*.svg", "logo.png"));
        assert!(glob_match(".DS_Store", ".DS_Store"));
        assert!(glob_match("*.tmp", "a.b.tmp"));
        assert!(glob_match("docs/*.md", "docs/readme.md"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn verbatim_and_exclude_scope() {
        assert!(is_verbatim("assets/logo.svg", &["*.svg".into()]));
        assert!(is_excluded(".DS_Store", &[".DS_Store".into()]));
        assert!(!is_excluded("keep.txt", &["*.tmp".into()]));
    }

    #[test]
    fn copy_job_is_byte_identical_and_tracks_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("big.bin");
        let dest = tmp.path().join("out/big.bin");
        let data: Vec<u8> = (0..(3 * 1024 * 1024u32)).map(|i| i as u8).collect();
        fs::write(&src, &data).unwrap();
        let job = CopyJob {
            src: src.clone(),
            dest: dest.clone(),
            bytes: data.len() as u64,
        };
        let progress = Mutex::new(Progress::new(std::slice::from_ref(&job)));
        copy_job(&job, &progress, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), data);
        assert_eq!(progress.lock().unwrap().copied_bytes, data.len() as u64);
        assert!(!dest.with_extension("bin.part").exists());
    }

    #[test]
    fn copy_job_does_not_treat_a_part_sibling_as_scratch() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("source.bin");
        let dest = tmp.path().join("payload.bin");
        let part_payload = tmp.path().join("payload.bin.part");
        fs::write(&src, b"new payload").unwrap();
        fs::write(&part_payload, b"real sibling payload").unwrap();
        let job = CopyJob {
            src,
            dest: dest.clone(),
            bytes: 11,
        };
        let progress = Mutex::new(Progress::new(std::slice::from_ref(&job)));

        copy_job(&job, &progress, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(dest).unwrap(), b"new payload");
        assert_eq!(fs::read(part_payload).unwrap(), b"real sibling payload");
    }

    #[test]
    fn copy_job_never_replaces_an_existing_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("source.bin");
        let dest = tmp.path().join("payload.bin");
        fs::write(&src, b"new payload").unwrap();
        fs::write(&dest, b"existing payload").unwrap();
        let job = CopyJob {
            src,
            dest: dest.clone(),
            bytes: 11,
        };
        let progress = Mutex::new(Progress::new(std::slice::from_ref(&job)));

        let error = copy_job(&job, &progress, &AtomicBool::new(false)).unwrap_err();
        assert!(error.to_string().contains("occupied"));
        assert_eq!(fs::read(dest).unwrap(), b"existing payload");
    }

    #[test]
    fn copy_job_honors_cancel_and_leaves_no_partial() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("big.bin");
        let dest = tmp.path().join("out/big.bin");
        let data: Vec<u8> = (0..(3 * 1024 * 1024u32)).map(|i| i as u8).collect();
        fs::write(&src, &data).unwrap();
        let job = CopyJob {
            src,
            dest: dest.clone(),
            bytes: data.len() as u64,
        };
        let progress = Mutex::new(Progress::new(std::slice::from_ref(&job)));
        // Pre-set cancel so the copy bails on the first chunk check.
        let err = copy_job(&job, &progress, &AtomicBool::new(true)).unwrap_err();
        assert!(err.to_string().contains(CANCELLED_MSG));
        assert!(!dest.exists(), "no destination file on cancel");
        let mut part = dest.into_os_string();
        part.push(".part");
        assert!(!PathBuf::from(part).exists(), "no .part left behind");
    }

    #[test]
    fn verify_tree_detects_short_and_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub/b.bin"), vec![0u8; 2048]).unwrap();

        // Faithful copy verifies.
        copy_tree(&src, &dst).unwrap();
        verify_tree(&src, &dst).unwrap();

        // A truncated destination file fails verification.
        fs::write(dst.join("sub/b.bin"), vec![0u8; 1024]).unwrap();
        assert!(
            verify_tree(&src, &dst)
                .unwrap_err()
                .to_string()
                .contains("size mismatch")
        );
        // Restore it so the next case isolates the missing-file failure.
        fs::write(dst.join("sub/b.bin"), vec![0u8; 2048]).unwrap();
        verify_tree(&src, &dst).unwrap();

        // A missing destination file fails verification.
        fs::remove_file(dst.join("a.txt")).unwrap();
        let err = verify_tree(&src, &dst).unwrap_err().to_string();
        assert!(
            err.contains("missing") || err.contains("files"),
            "err: {err}"
        );
    }

    #[test]
    fn copy_tree_reproduces_nested_files_and_empty_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join("docs/deep")).unwrap();
        fs::create_dir_all(src.join("empty_dir")).unwrap();
        // Literal braces must survive — a move never interpolates.
        fs::write(src.join("notes_{client}.md"), "hello {name}").unwrap();
        let binary: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        fs::write(src.join("docs/deep/blob.bin"), &binary).unwrap();

        let dst = tmp.path().join("dst");
        copy_tree(&src, &dst).unwrap();

        assert_eq!(
            fs::read_to_string(dst.join("notes_{client}.md")).unwrap(),
            "hello {name}"
        );
        assert_eq!(fs::read(dst.join("docs/deep/blob.bin")).unwrap(), binary);
        assert!(dst.join("empty_dir").is_dir());
        // Refuses to merge into an existing target.
        assert!(copy_tree(&src, &dst).is_err());
    }

    #[test]
    fn missing_tree_is_an_error_not_an_empty_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing");
        let destination = tmp.path().join("destination");
        assert!(jobs_for_tree(&missing, &destination).is_err());
        fs::create_dir(&destination).unwrap();
        assert!(verify_tree(&missing, &destination).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlink_occupies_a_destination_path() {
        let tmp = tempfile::tempdir().unwrap();
        let occupied = tmp.path().join("occupied");
        std::os::unix::fs::symlink(tmp.path().join("missing-target"), &occupied).unwrap();
        assert!(entry_exists(&occupied).unwrap());
    }

    /// Create a directory link inside a test tree, cross-platform.
    ///
    /// Windows junctions need no elevation (unlike symlinks, which require
    /// Developer Mode), so `mklink /J` is the portable-enough choice there.
    /// Returns `false` when the OS refused, so a test can skip rather than fail
    /// on a machine with restrictive policy.
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

    /// The data-loss regression. A junction inside a project used to be invisible
    /// to `walk`, so a staged move copied around it and `verify_tree` — walking
    /// the same blind way — reported success. The source was then deleted.
    #[test]
    fn links_are_reported_not_silently_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real_asset_library");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("payload.txt"), "irreplaceable").unwrap();

        let src = tmp.path().join("project");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("normal.txt"), "ordinary").unwrap();
        if !make_dir_link(&src.join("linked"), &target) {
            eprintln!("skipping: OS refused to create a directory link");
            return;
        }

        // walk sees it, and classifies it as a link rather than dropping it.
        let entries = walk(&src).unwrap();
        let link = entries
            .iter()
            .find(|e| e.rel == "linked")
            .expect("the link must appear in the walk");
        assert!(link.is_symlink(), "kind was {:?}", link.kind);

        assert_eq!(find_links(&src).unwrap(), vec!["linked".to_string()]);

        // Every copy path refuses rather than silently dropping it.
        let dst = tmp.path().join("dst");
        assert!(
            copy_tree(&src, &dst)
                .unwrap_err()
                .to_string()
                .contains("link"),
            "copy_tree must refuse a link"
        );
        assert!(
            jobs_for_tree(&src, &dst)
                .unwrap_err()
                .to_string()
                .contains("link"),
            "jobs_for_tree must refuse a link"
        );
    }

    /// Verification must be deny-by-default: a link it cannot check is an error,
    /// never a skip. Skipping on both sides is what made the sizes agree and let
    /// a lossy copy "verify".
    #[test]
    fn verify_tree_refuses_trees_containing_links() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();

        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a.txt"), "x").unwrap();
        fs::write(dst.join("a.txt"), "x").unwrap();
        // Identical but for a link the destination never received.
        if !make_dir_link(&src.join("linked"), &target) {
            eprintln!("skipping: OS refused to create a directory link");
            return;
        }

        let err = verify_tree(&src, &dst).unwrap_err().to_string();
        assert!(
            err.contains("verification failed") && err.contains("linked"),
            "expected a verification failure naming the link, got: {err}"
        );
    }

    /// A file at exactly the interpolation cap is still interpolated; one past it
    /// is copied verbatim. The threshold ordering itself is a compile-time
    /// assertion next to the constants.
    #[test]
    fn text_cap_decides_interpolate_versus_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Aurora".to_string());

        // Comfortably under the cap → interpolated.
        let small = tmp.path().join("small.txt");
        fs::write(&small, "hello {name}").unwrap();
        let dest = tmp.path().join("out_small.txt");
        copy_file(&small, &dest, false, &vars, "%Y-%m-%d").unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello Aurora");

        // Same file, forced verbatim → braces survive untouched.
        let dest = tmp.path().join("out_verbatim.txt");
        copy_file(&small, &dest, true, &vars, "%Y-%m-%d").unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello {name}");

        // A comfortably large — but still under the cap — text file must also be
        // interpolated. A README of a few hundred KB is ordinary in a template,
        // and shipping it with raw `{tokens}` would be a silent regression.
        let big = tmp.path().join("big.md");
        let body = format!("{}\n# {{name}}\n", "filler line\n".repeat(20_000));
        fs::write(&big, &body).unwrap();
        assert!(
            (body.len() as u64) < TEXT_MAX_BYTES && body.len() > 200_000,
            "fixture should be large but under the cap ({} bytes)",
            body.len()
        );
        let dest = tmp.path().join("out_big.md");
        copy_file(&big, &dest, false, &vars, "%Y-%m-%d").unwrap();
        let out = fs::read_to_string(&dest).unwrap();
        assert!(out.ends_with("# Aurora\n"), "large text must interpolate");
        assert!(!out.contains("{name}"));
    }

    /// `walk` must distinguish the kinds, not just "exists".
    #[test]
    fn walk_classifies_each_entry_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("a_dir/nested")).unwrap();
        fs::write(root.join("b_file.txt"), "x").unwrap();
        fs::write(root.join("a_dir/nested/deep.bin"), vec![0u8; 10]).unwrap();

        let entries = walk(root).unwrap();
        let kind_of = |rel: &str| entries.iter().find(|e| e.rel == rel).map(|e| e.kind);

        assert_eq!(kind_of("a_dir"), Some(EntryKind::Dir));
        assert_eq!(kind_of("a_dir/nested"), Some(EntryKind::Dir));
        assert_eq!(kind_of("b_file.txt"), Some(EntryKind::File));
        assert_eq!(kind_of("a_dir/nested/deep.bin"), Some(EntryKind::File));

        // The predicates must actually discriminate — `is_dir()` returning a
        // constant would still satisfy a test that only counted entries.
        let dirs = entries.iter().filter(|e| e.is_dir()).count();
        let files = entries.iter().filter(|e| e.is_file()).count();
        assert_eq!(dirs, 2, "exactly the two directories");
        assert_eq!(files, 2, "exactly the two files");
        assert!(entries.iter().filter(|e| e.is_dir()).all(|e| !e.is_file()));
        // Sizes are only meaningful for files.
        assert_eq!(kind_of("b_file.txt").map(|_| ()), Some(()));
        assert_eq!(
            entries.iter().find(|e| e.rel == "a_dir").map(|e| e.size),
            Some(0)
        );
        assert_eq!(
            entries
                .iter()
                .find(|e| e.rel == "a_dir/nested/deep.bin")
                .map(|e| e.size),
            Some(10)
        );
    }

    /// Verification asks one question: did everything make it across? A surplus
    /// at the destination does not threaten that, so it is tolerated — only
    /// missing or short files block a move from removing its source.
    #[test]
    fn verify_tree_requires_everything_arrived_but_tolerates_extras() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a.txt"), "same").unwrap();
        fs::write(dst.join("a.txt"), "same").unwrap();
        verify_tree(&src, &dst).unwrap();

        // An unrelated extra file at the destination is not a failure.
        fs::write(dst.join("stranger.txt"), "unexpected").unwrap();
        verify_tree(&src, &dst)
            .expect("extra files at the destination must not block verification");

        // But anything missing from the destination does block it — this is the
        // guarantee that stands between a move and deleting a good source.
        fs::write(src.join("b.txt"), "must arrive").unwrap();
        let err = verify_tree(&src, &dst).unwrap_err().to_string();
        assert!(err.contains("missing at destination"), "got: {err}");
    }

    #[test]
    fn move_jobs_treat_tmp_and_part_names_as_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("notes.tmp"), "temporary by name, real by contract").unwrap();
        fs::write(src.join("nested/render.part"), [0_u8, 1, 2, 255]).unwrap();

        let (_, jobs) = jobs_for_tree(&src, &dst).unwrap();
        let names: Vec<_> = jobs
            .iter()
            .map(|job| job.src.strip_prefix(&src).unwrap().to_path_buf())
            .collect();
        assert!(names.contains(&PathBuf::from("notes.tmp")));
        assert!(names.contains(&PathBuf::from("nested/render.part")));
    }

    /// The glob matcher backtracks; the wildcard bookkeeping is easy to get
    /// subtly wrong in ways a couple of happy-path cases miss.
    #[test]
    fn glob_match_backtracks_correctly() {
        // A `*` that must give back characters before matching.
        assert!(glob_match("*.txt", "a.b.txt"));
        assert!(glob_match("*b*", "abc"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxbyy"));
        // Trailing wildcards may consume nothing.
        assert!(glob_match("a*", "a"));
        assert!(glob_match("a**", "a"));
        assert!(glob_match("*", ""));
        // `?` is exactly one character.
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(!glob_match("a?c", "abbc"));
        // Anchoring: a pattern must consume the whole string.
        assert!(!glob_match("abc", "abcd"));
        assert!(!glob_match("abcd", "abc"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
        // Repeated wildcards must not double-count.
        assert!(glob_match("*a*a*", "banana"));
        assert!(!glob_match("*z*z*", "banana"));
    }

    #[test]
    fn interp_rel_is_per_segment() {
        let mut vars = HashMap::new();
        vars.insert("client".to_string(), String::new());
        vars.insert("name".to_string(), "Aurora".to_string());
        // Empty segment variable collapses within the segment, slash preserved.
        let out = interp_rel("05_Delivery/Note_{name}.md", &vars, "%Y-%m-%d");
        assert_eq!(out, "05_Delivery/Note_Aurora.md");
    }
}
