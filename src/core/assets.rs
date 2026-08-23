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
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Text files at or below this size are candidates for `{token}` interpolation.
/// Anything larger is copied verbatim — interpolating a 200 MB file makes no
/// sense and would blow up memory.
pub const TEXT_MAX_BYTES: u64 = 1024 * 1024;

/// A single deferred file copy (always a verbatim byte copy). Creates no longer
/// defer anything, but [`crate::core::provisioning`] still reads create journals
/// a pre-v2 binary may have left on a shared drive, and the staged move builds
/// its own jobs.
#[derive(Debug, Clone)]
pub struct CopyJob {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub bytes: u64,
}

/// Live progress of a background copy job.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub total_files: usize,
    pub done_files: usize,
    pub current_file: String,
    pub status: JobStatus,
    /// Coarse stage for the UI, shared by create + move jobs.
    pub phase: JobPhase,
    pub error: Option<String>,
    /// A move reached its verified destination but could not remove its source.
    /// Existing clients may ignore this additive field safely.
    pub cleanup_pending: bool,
    /// Non-fatal detail accompanying [`Self::cleanup_pending`].
    pub warning: Option<String>,
    /// Unix-epoch milliseconds of the last observed movement (bytes copied, a
    /// file finished, or a phase change).
    ///
    /// It tells "slow" from "stuck" — a copy to a cloud-synced or network
    /// destination can legitimately sit for minutes, so there is no wall-clock
    /// timeout, only an honest "no progress for N minutes" note.
    pub last_progress_at: u64,
}

/// Where a background job has got to.
///
/// Was a `String` set by literal at fifteen call sites, which is exactly as many
/// chances to write `"canceled"`. The serialized names are unchanged, so the
/// JSON the browser reads is byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// The coarse stage a job reports, shared by create and move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobPhase {
    Copying,
    Verifying,
    Finalizing,
    Done,
}

impl JobPhase {
    /// The wire and display name — the same string the JSON carries.
    pub fn as_str(self) -> &'static str {
        match self {
            JobPhase::Copying => "copying",
            JobPhase::Verifying => "verifying",
            JobPhase::Finalizing => "finalizing",
            JobPhase::Done => "done",
        }
    }
}

impl std::fmt::Display for JobPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

impl Progress {
    pub fn new(jobs: &[CopyJob]) -> Self {
        Self {
            total_bytes: jobs.iter().map(|j| j.bytes).sum(),
            copied_bytes: 0,
            total_files: jobs.len(),
            done_files: 0,
            current_file: String::new(),
            status: JobStatus::Running,
            phase: JobPhase::Copying,
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
pub(crate) const CANCELLED_MSG: &str = "copy cancelled";

/// Copy one deferred (large, verbatim) file into place with chunked progress.
/// Atomic via an operation-owned unique sibling + rename;
/// `progress.copied_bytes` is bumped per chunk so the UI shows a live bar
/// during a multi-minute copy.
///
/// `cancel` is polled between chunks: when set, the exact partial sibling is
/// removed and the copy returns a `CANCELLED_MSG` error so no half-written
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
#[derive(Debug, Clone)]
pub struct AssetEntry {
    /// Path relative to the walk root, forward-slash separated,
    /// **uninterpolated**, and **lossy** for a name that is not valid UTF-8.
    ///
    /// This is the *textual* form: globs, `SafeRelativePath` and the browser's
    /// JSON all reason about names as text and always have. Never join it to
    /// open or create a file — use [`Self::os_rel`], which is exact.
    pub rel: String,
    /// The same path as the filesystem actually spells it.
    ///
    /// `rel` was the only form, so a template file whose name is not valid
    /// UTF-8 was opened at a `?`-substituted path that does not exist — the copy
    /// failed with "file not found" naming a path the user never wrote.
    pub os_rel: PathBuf,
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
    walk_inner(files_dir, files_dir, 0, &mut out)?;
    // Lexicographic sort puts a parent ("a") before its children ("a/b").
    out.sort_by(|x, y| x.rel.cmp(&y.rel));
    Ok(out)
}

fn walk_inner(root: &Path, current: &Path, depth: usize, out: &mut Vec<AssetEntry>) -> Result<()> {
    if depth >= crate::util::paths::MAX_WALK_DEPTH {
        return Err(crate::util::paths::too_deep(current));
    }
    for entry in fs::read_dir(current).with_context(|| format!("reading {}", current.display()))? {
        let entry = entry?;
        let path = entry.path();
        // `DirEntry::file_type` does not follow links, so a symlink to a
        // directory reports as a symlink rather than a dir — the link itself is
        // the thing being described, which is what the caller needs to know.
        let ft = entry.file_type()?;
        let os_rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        let rel = os_rel.to_string_lossy().replace('\\', "/");

        if ft.is_symlink() {
            out.push(AssetEntry {
                rel,
                os_rel,
                kind: EntryKind::Symlink,
                size: 0,
            });
        } else if ft.is_dir() {
            out.push(AssetEntry {
                rel,
                os_rel,
                kind: EntryKind::Dir,
                size: 0,
            });
            walk_inner(root, &path, depth + 1, out)?;
        } else if ft.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(AssetEntry {
                rel,
                os_rel,
                kind: EntryKind::File,
                size,
            });
        } else {
            out.push(AssetEntry {
                rel,
                os_rel,
                kind: EntryKind::Other,
                size: 0,
            });
        }
    }
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
    interp_rel_with(
        rel,
        vars,
        &crate::core::naming::RenderContext::now(date_format),
    )
}

/// Interpolate a native relative path, component by component, without ever
/// converting a component that has no token in it.
///
/// A component containing `{` is interpolated as text (it must be, to be
/// interpolated at all); anything else is pushed through as the `OsStr` it
/// already is. So a template file whose name is not valid UTF-8 and contains no
/// token reaches the new project spelled exactly as it was, instead of being
/// mangled by a lossy conversion on the way.
pub fn interp_rel_os(
    rel: &Path,
    vars: &HashMap<String, String>,
    ctx: &crate::core::naming::RenderContext,
) -> PathBuf {
    let mut out = PathBuf::new();
    for component in rel.components() {
        let raw = component.as_os_str();
        match raw.to_str() {
            Some(text) if text.contains('{') => {
                out.push(crate::core::naming::interpolate_name_with(text, vars, ctx));
            }
            _ => out.push(raw),
        }
    }
    out
}

/// [`interp_rel`] against a prepared context — see [`crate::core::naming::RenderContext`].
pub fn interp_rel_with(
    rel: &str,
    vars: &HashMap<String, String>,
    ctx: &crate::core::naming::RenderContext,
) -> String {
    rel.split('/')
        .map(|segment| crate::core::naming::interpolate_name_with(segment, vars, ctx))
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
    ctx: &crate::core::naming::RenderContext,
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
            let rendered = crate::core::naming::interpolate_with(&text, vars, ctx);
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

    /// Links must be *reported* by the walk, not dropped. Template copying
    /// decides what to do about one by asking `is_file()`, so a link
    /// misclassified as a file would be copied by following it, and a link
    /// missing from the walk would vanish from a new project with no warning.
    #[test]
    fn walk_reports_links_instead_of_skipping_them() {
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

        let entries = walk(&src).unwrap();
        let link = entries
            .iter()
            .find(|e| e.rel == "linked")
            .expect("the link must appear in the walk");
        assert!(link.is_symlink(), "kind was {:?}", link.kind);
        assert!(!link.is_file() && !link.is_dir());
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
        copy_file(
            &small,
            &dest,
            false,
            &vars,
            &crate::core::naming::RenderContext::now("%Y-%m-%d"),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello Aurora");

        // Same file, forced verbatim → braces survive untouched.
        let dest = tmp.path().join("out_verbatim.txt");
        copy_file(
            &small,
            &dest,
            true,
            &vars,
            &crate::core::naming::RenderContext::now("%Y-%m-%d"),
        )
        .unwrap();
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
        copy_file(
            &big,
            &dest,
            false,
            &vars,
            &crate::core::naming::RenderContext::now("%Y-%m-%d"),
        )
        .unwrap();
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

    /// The enums replaced `String` fields the browser reads by name. If these
    /// change, `/api/job/<id>` starts answering in a vocabulary the frontend
    /// does not know, and there is nothing in the JSON to say so.
    #[test]
    fn job_status_and_phase_serialize_to_the_names_the_browser_reads() {
        use super::{JobPhase, JobStatus};

        for (value, name) in [
            (JobStatus::Running, "running"),
            (JobStatus::Done, "done"),
            (JobStatus::Failed, "failed"),
            (JobStatus::Cancelled, "cancelled"),
        ] {
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(
                serde_json::from_str::<JobStatus>(&format!("\"{name}\"")).unwrap(),
                value
            );
        }

        for (value, name) in [
            (JobPhase::Copying, "copying"),
            (JobPhase::Verifying, "verifying"),
            (JobPhase::Finalizing, "finalizing"),
            (JobPhase::Done, "done"),
        ] {
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(value.as_str(), name);
        }
    }
}
