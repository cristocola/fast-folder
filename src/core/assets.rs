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
        }
    }
}

/// A copy that stopped because its cancel flag was set. Callers distinguish this
/// from a genuine failure by checking the flag after `copy_job` returns `Err`.
pub const CANCELLED_MSG: &str = "copy cancelled";

/// Copy one deferred (large, verbatim) file into place with chunked progress.
/// Atomic via `.part` + rename; `progress.copied_bytes` is bumped per chunk so
/// the UI shows a live bar during a multi-minute copy.
///
/// `cancel` is polled between chunks: when set, the partial `.part` is removed
/// and the copy returns an [`CANCELLED_MSG`] error so no half-written file is
/// ever left in place.
pub fn copy_job(job: &CopyJob, progress: &Mutex<Progress>, cancel: &AtomicBool) -> Result<()> {
    if let Some(parent) = job.dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", job.dest.display()))?;
    }
    let mut tmp_os = job.dest.as_os_str().to_owned();
    tmp_os.push(".part");
    let tmp = PathBuf::from(tmp_os);

    let result = (|| -> Result<()> {
        let mut reader =
            fs::File::open(&job.src).with_context(|| format!("opening {}", job.src.display()))?;
        let mut writer =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
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
            }
        }
        writer.sync_all().ok();
        Ok(())
    })();

    match result {
        Ok(()) => {
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
    /// A symlink, Windows junction, or other reparse point. Never followed.
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
    if dst.exists() {
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

/// Disposable/transient files that must not count toward a copy or verification:
/// the per-base cache, provisioning/move markers, half-written `.part` temps, and
/// the `.tmp` scratch files left by [`crate::util::atomic`]. A tree copy skips
/// them and verification ignores them on both sides, so moving a project that
/// itself carries stale scaffolding still verifies cleanly.
fn is_transient(rel: &str) -> bool {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    base == crate::core::library::CACHE_FILENAME
        || base == crate::core::provisioning::MARKER_CREATE
        || base.starts_with(crate::core::provisioning::MARKER_MOVE_PREFIX)
        || base.ends_with(".part")
        || crate::util::atomic::is_temp_file(base)
}

/// Enumerate a source tree into the directory creates + per-file [`CopyJob`]s
/// needed to reproduce it verbatim under `dst`. Directories (including empty
/// ones) come first so a plain create-then-copy is ordering-safe. Transient
/// scaffolding files are skipped. Used by the staged (cross-filesystem / network)
/// move so the copy can report live progress and honor cancellation.
///
/// Errors on a link or special file. Callers pre-flight with [`find_links`] and
/// refuse the move up front, so this is the backstop that guarantees no caller
/// can ever reach the "copied, verified, now delete the source" step while
/// having silently skipped something.
pub fn jobs_for_tree(src: &Path, dst: &Path) -> Result<(Vec<PathBuf>, Vec<CopyJob>)> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in walk(src)? {
        let rel = entry.rel.replace('/', std::path::MAIN_SEPARATOR_STR);
        let target = dst.join(&rel);
        match entry.kind {
            EntryKind::Dir => dirs.push(target),
            EntryKind::File => {
                if !is_transient(&entry.rel) {
                    files.push(CopyJob {
                        src: src.join(&rel),
                        dest: target,
                        bytes: entry.size,
                    });
                }
            }
            EntryKind::Symlink | EntryKind::Other => anyhow::bail!(
                "cannot copy '{}': it is a link or special file. \
                 Copying it faithfully is not supported, and skipping it would silently lose data.",
                entry.rel
            ),
        }
    }
    Ok((dirs, files))
}

/// Verify that `dst` faithfully reproduces `src`: every non-transient file in
/// `src` exists under `dst` with an identical byte size, and the file counts
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
    let sizes = |root: &Path, side: &str| -> Result<HashMap<String, u64>> {
        let mut map = HashMap::new();
        for entry in walk(root)? {
            if is_transient(&entry.rel) {
                continue;
            }
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
    if dst_files.len() < src_files.len() {
        anyhow::bail!(
            "verification failed: destination has {} files, source has {}",
            dst_files.len(),
            src_files.len()
        );
    }
    Ok(())
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

/// Copy one file, atomically (`.part` temp + rename).
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

    let mut tmp_os = dest.as_os_str().to_owned();
    tmp_os.push(".part");
    let tmp = PathBuf::from(tmp_os);

    let interpolated = if force_verbatim {
        None
    } else {
        // Try to read as text; a non-UTF-8 file yields Err → verbatim copy.
        fs::read_to_string(src).ok()
    };

    match interpolated {
        Some(text) => {
            let rendered = crate::core::naming::interpolate(&text, vars, date_format);
            fs::write(&tmp, rendered).with_context(|| format!("writing {}", tmp.display()))?;
        }
        None => {
            fs::copy(src, &tmp)
                .with_context(|| format!("copying {} → {}", src.display(), tmp.display()))?;
        }
    }

    crate::util::fs_retry::rename(&tmp, dest)
        .with_context(|| format!("finalizing {}", dest.display()))
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })?;
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

    #[test]
    fn atomic_write_temp_files_are_transient() {
        // A `.tmp` left by a concurrent atomic write must not break a move's
        // verification the way an uncounted file would.
        assert!(is_transient("config.toml.1234.0.tmp"));
        assert!(is_transient("sub/dir/.fastf-index.json"));
        assert!(is_transient("big.bin.part"));
        assert!(!is_transient("notes.md"));
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
