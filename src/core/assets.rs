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

/// A single deferred file copy (always a verbatim byte copy). Creates defer
/// nothing; [`crate::core::provisioning`] builds one for each copy owed by a
/// create journal a fastf before v2.0.0 left on a shared drive.
#[derive(Debug, Clone)]
pub struct CopyJob {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub bytes: u64,
}

/// Live progress of a long job: a create's resumed copies, a move, a copy out
/// of the library, a reconcile.
///
/// **Every step is named and counted as it happens**, since a step after the
/// copy can take ten minutes of API calls on a cloud mount: each step is a
/// [`JobPhase`], [`Self::step_done`] of [`Self::step_total`] moves for every
/// entry it touches, and [`Self::finished`] keeps what each finished step
/// counted. [`crate::core::progress::Ticker`] is how the engine writes it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub total_files: usize,
    pub done_files: usize,
    /// The entry the current step is at, relative to what it walks.
    pub current_file: String,
    pub status: JobStatus,
    pub phase: JobPhase,
    /// The steps this job passes through, in order, when that is known up
    /// front (a move, a copy); empty for a reconcile, whose steps depend on
    /// what it finds.
    pub steps: Vec<JobPhase>,
    /// The steps finished so far, each with what it counted.
    pub finished: Vec<FinishedStep>,
    /// How far the current step has got, in its own unit
    /// ([`JobPhase::unit`]).
    pub step_done: usize,
    /// How far it will go; 0 when that is not known (a scan, a delete).
    pub step_total: usize,
    /// Which of [`Self::items`] is under way, from 1; 0 when the job is one
    /// thing. A reconcile counts what the header counted as needing attention.
    pub item: usize,
    pub items: usize,
    /// What the current item is about.
    pub item_label: String,
    /// What the job is, for its log lines: `move ID0047 Shoot to /mnt/b`.
    pub subject: String,
    /// The move record's operation id, once there is one.
    pub operation: Option<String>,
    /// The job is past its point of no return: a move has published, and what
    /// is left is housekeeping a cancel cannot undo. A surface answers a
    /// cancel from here with "too late" rather than pretending to stop. A
    /// reconcile never sets it — it can stop between items, and inside a
    /// removal.
    pub committed: bool,
    /// The job holds the data lock right now, so every other fastf's changes
    /// wait for it: while a move copies, not while it removes an old copy.
    pub holds_lock: bool,
    /// Why a job that ended [`JobStatus::Failed`] failed.
    pub error: Option<String>,
    /// Unix-epoch milliseconds of the last observed movement, written on
    /// every `touch`; what [`Self::stalled_ms`] is measured from.
    pub last_progress_at: u64,
    /// The folder the current step works in, as a person would recognise
    /// it: what a stall is "no answer from".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub working_in: String,
    /// How long the job has waited on its filesystem without an answer, in
    /// milliseconds; 0 while it moves. A job's worker sets it as it writes
    /// its state ([`Self::note_stall`]), so every surface following the job
    /// can say "no answer from … for N s" instead of looking frozen.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub stalled_ms: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// How long without movement a running job's step is a stall worth saying.
pub const STALL_AFTER_MS: u64 = 5_000;

/// A step a job has finished, and what it counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishedStep {
    pub phase: JobPhase,
    pub count: usize,
}

/// Where a background job has got to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    #[default]
    Running,
    Done,
    Failed,
    Cancelled,
    /// Stopped before its publish because a mount stopped answering, its
    /// copy kept (`move_engine::Paused`): it goes on when it is run again, or
    /// with `fastf reconcile` once the mount is back.
    Paused,
    /// A state a later fastf wrote, sharing this data dir. Read as ended:
    /// nothing here can follow it further.
    #[serde(other)]
    Unknown,
}

/// The step a job is at. A staged move passes through every one of these but
/// `Waiting`, in this order; a copy skips the probe and everything about the
/// original.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobPhase {
    /// Nothing has happened yet.
    #[default]
    Starting,
    /// Another fastf holds the data lock.
    Waiting,
    /// Walking the project to record what it holds.
    Scanning,
    /// Asking the original's base whether the original can leave it.
    Probing,
    Copying,
    /// Walking the copy and the original again, and comparing.
    Verifying,
    /// Writing `PROJECT_INFO.md` last, which makes the copy the project.
    Publishing,
    /// Checking the original is the project that was moved, then taking it
    /// out of the library in one step: renamed aside, or on a cloud mount its
    /// `PROJECT_INFO.md` removed.
    SettingAside,
    /// Walking the moved copy, once, for the merge that removes the old copy.
    Checking,
    /// Removing the set-aside copy, entry by entry.
    Removing,
    /// Removing the move's record.
    Clearing,
    Done,
    /// A step a later fastf wrote, sharing this data dir.
    #[serde(other)]
    Unknown,
}

impl JobPhase {
    /// What the step is doing, as a person reads it.
    pub fn as_str(self) -> &'static str {
        match self {
            JobPhase::Starting => "starting",
            JobPhase::Waiting => "waiting for another fastf",
            JobPhase::Scanning => "scanning",
            JobPhase::Probing => "checking the original's base",
            JobPhase::Copying => "copying",
            JobPhase::Verifying => "verifying",
            JobPhase::Publishing => "publishing PROJECT_INFO.md",
            JobPhase::SettingAside => "setting the original aside",
            JobPhase::Checking => "checking the old copy",
            JobPhase::Removing => "removing the old copy",
            JobPhase::Clearing => "clearing the record",
            JobPhase::Done => "done",
            JobPhase::Unknown => "working",
        }
    }

    /// What the step did, once it has.
    pub fn past(self) -> &'static str {
        match self {
            JobPhase::Starting => "started",
            JobPhase::Waiting => "waited for another fastf",
            JobPhase::Scanning => "scanned",
            JobPhase::Probing => "checked the original's base",
            JobPhase::Copying => "copied",
            JobPhase::Verifying => "verified",
            JobPhase::Publishing => "published PROJECT_INFO.md",
            JobPhase::SettingAside => "set the original aside",
            JobPhase::Checking => "checked the old copy",
            JobPhase::Removing => "removed the old copy",
            JobPhase::Clearing => "cleared the record",
            JobPhase::Done => "done",
            JobPhase::Unknown => "worked",
        }
    }

    /// What the step counts, if it counts anything.
    pub fn unit(self) -> Option<&'static str> {
        match self {
            JobPhase::Copying => Some("files"),
            JobPhase::Scanning
            | JobPhase::Verifying
            | JobPhase::SettingAside
            | JobPhase::Checking
            | JobPhase::Removing => Some("entries"),
            _ => None,
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
            total_files: jobs.len(),
            phase: if jobs.is_empty() {
                JobPhase::Starting
            } else {
                JobPhase::Copying
            },
            last_progress_at: now_millis(),
            ..Self::default()
        }
    }

    /// Record that the job just made progress. Call alongside every mutation
    /// that represents real movement.
    pub fn touch(&mut self) {
        self.last_progress_at = now_millis();
    }

    /// Set [`Self::stalled_ms`] from the time since the last movement: a
    /// running step that has not moved for [`STALL_AFTER_MS`]. Waiting for
    /// the data lock is not a stall — another fastf holds it, and says so.
    pub fn note_stall(&mut self) {
        let quiet = now_millis().saturating_sub(self.last_progress_at);
        self.stalled_ms = if self.status == JobStatus::Running
            && !matches!(
                self.phase,
                JobPhase::Waiting | JobPhase::Starting | JobPhase::Done
            )
            && quiet >= STALL_AFTER_MS
        {
            quiet
        } else {
            0
        };
    }

    /// "no answer from /mnt/cloud for 42 s", while the job is stalled.
    pub fn stall_text(&self) -> Option<String> {
        if self.stalled_ms == 0 {
            return None;
        }
        let place = if self.working_in.is_empty() {
            "the filesystem".to_string()
        } else {
            self.working_in.clone()
        };
        Some(format!(
            "no answer from {place} for {} s; fastf waits for it",
            self.stalled_ms / 1000
        ))
    }

    /// The current step's count in words: `312 of 1473 entries`, `312
    /// entries` when the total is not known, nothing for a step that counts
    /// nothing.
    pub fn count_text(&self) -> String {
        count_text(self.phase, self.step_done, self.step_total)
    }

    /// Whether the current step has a total to draw a bar against.
    pub fn has_total(&self) -> bool {
        self.step_total > 0 && self.phase.unit().is_some()
    }

    /// The current step and its count: `copying 312 of 1473 files`.
    pub fn step_text(&self) -> String {
        let count = self.count_text();
        if count.is_empty() {
            self.phase.as_str().to_string()
        } else {
            format!("{} {count}", self.phase.as_str())
        }
    }

    /// `item 2 of 5`, or nothing for a job that is one thing.
    pub fn item_text(&self) -> String {
        if self.items == 0 && self.item == 0 {
            return String::new();
        }
        format!("{} of {}", self.item.max(1), self.items.max(self.item))
    }
}

/// `count` in a step's unit, against `total` when it is known. The unit
/// agrees with the number it counts: `1 entry`, `2 of 1473 entries`.
pub fn count_text(phase: JobPhase, count: usize, total: usize) -> String {
    let Some(unit) = phase.unit() else {
        return String::new();
    };
    let unit = if total.max(count) == 1 || (total == 0 && count == 1) {
        match unit {
            "entries" => "entry",
            "files" => "file",
            other => other,
        }
    } else {
        unit
    };
    if total > 0 {
        format!("{count} of {total} {unit}")
    } else {
        format!("{count} {unit}")
    }
}

/// Unix-epoch milliseconds, from this machine's clock — the only one that ever
/// reads them.
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
///
/// Unlike [`copy_file`] this takes an already-joined `dest`, because a
/// [`CopyJob`] is a pair of absolute paths by the time it exists. Its one
/// production caller — `provisioning`'s create-journal resume — derives that
/// destination through [`crate::util::paths::contained_destination`] first, and
/// a new caller must do the same.
pub fn copy_job(job: &CopyJob, progress: &Mutex<Progress>, cancel: &AtomicBool) -> Result<()> {
    if entry_exists(&job.dest)? {
        anyhow::bail!(
            "copy destination is already occupied: {}",
            crate::util::paths::display_path(&job.dest)
        );
    }
    if let Some(parent) = job.dest.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating parent dirs for {}",
                crate::util::paths::display_path(&job.dest)
            )
        })?;
    }
    let (tmp, mut writer) = crate::util::atomic::create_temp_for(&job.dest)?;

    let result = (|| -> Result<()> {
        let mut reader = fs::File::open(&job.src)
            .with_context(|| format!("opening {}", crate::util::paths::display_path(&job.src)))?;
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
            .with_context(|| format!("flushing {}", crate::util::paths::display_path(&tmp)))?;
        writer
            .sync_all()
            .with_context(|| format!("syncing {}", crate::util::paths::display_path(&tmp)))?;
        Ok(())
    })();
    drop(writer);

    match result {
        Ok(()) => {
            match entry_exists(&job.dest) {
                Ok(false) => {}
                Ok(true) => {
                    let _ = crate::util::fs_retry::remove_file(&tmp);
                    anyhow::bail!(
                        "copy destination became occupied: {}",
                        crate::util::paths::display_path(&job.dest)
                    );
                }
                Err(error) => {
                    let _ = crate::util::fs_retry::remove_file(&tmp);
                    return Err(error);
                }
            }
            crate::util::fs_retry::rename(&tmp, &job.dest)
                .with_context(|| {
                    format!("finalizing {}", crate::util::paths::display_path(&job.dest))
                })
                .inspect_err(|_| {
                    let _ = crate::util::fs_retry::remove_file(&tmp);
                })?;
            Ok(())
        }
        Err(e) => {
            let _ = crate::util::fs_retry::remove_file(&tmp);
            Err(e)
        }
    }
}

/// What a walked entry actually is.
///
/// An enum rather than a pair of bools on purpose: adding a variant makes the
/// compiler point at every consumer that must decide what to do with it.
/// **`walk` drops no entry**, whatever its kind: an entry a copy skips is one a
/// verification built on the same walk is blind to, on both sides.
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
    /// This is the *textual* form: globs and `SafeRelativePath` reason about
    /// names as text. Never join it to open or create a file — a name that is
    /// not valid UTF-8 would open a `?`-substituted path that does not exist.
    /// Use [`Self::os_rel`], which is exact.
    pub rel: String,
    /// The same path as the filesystem actually spells it.
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
    for entry in fs::read_dir(current)
        .with_context(|| format!("reading {}", crate::util::paths::display_path(current)))?
    {
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

/// Does a directory entry occupy this exact path? Unlike `Path::exists`, this
/// sees broken symlinks and propagates metadata errors instead of treating them
/// as a free destination.
pub fn entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("inspecting {}", crate::util::paths::display_path(path))),
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
///
/// **A component is only interpolated when it contains a token**, exactly as
/// in [`interp_rel_os`], so the two agree by construction for every UTF-8
/// path.
///
/// The separator collapse in `interpolate_name_with` is there for one job: an
/// empty optional variable must take its leftover `_` or `-` with it rather
/// than leaving a dangling one. A component with no `{` has no variable that
/// could have vanished, so there is nothing to clean up and its name is meant
/// literally. Collapsed anyway, `pkg/__init__.py` is *listed* as
/// `pkg/init_.py` while the copy writes the real one, and `apply` probes
/// `entry_exists` with the collapsed name, so its plan reports `[create]` for
/// a file already on disk and `[skip]` for one it is about to write.
pub fn interp_rel_with(
    rel: &str,
    vars: &HashMap<String, String>,
    ctx: &crate::core::naming::RenderContext,
) -> String {
    rel.split('/')
        .map(|segment| {
            if segment.contains('{') {
                crate::core::naming::interpolate_name_with(segment, vars, ctx)
            } else {
                segment.to_string()
            }
        })
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

/// What a create will do with one entry under `files/`.
///
/// An enum rather than a pair of booleans, for the reason [`EntryKind`] is one:
/// adding a case makes the compiler name every consumer that has to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    /// An `exclude` glob matched, or the rendered path is one fastf owns.
    /// Nothing is written, so nothing is listed and nothing is previewed.
    Skipped,
    /// A directory to reproduce, so a deliberately empty one survives.
    Folder,
    /// A link or special file: reported by the walk, never reproduced.
    Unsupported,
    /// A plain file whose `{tokens}` are substituted — if it reads as UTF-8.
    Interpolated,
    /// A plain file copied byte for byte: a `verbatim` glob, or larger than
    /// [`TEXT_MAX_BYTES`]. Its `{braces}` reach the project intact.
    Verbatim,
}

/// One walked entry, with the decision made and the path it will take.
#[derive(Debug, Clone)]
pub struct PlannedEntry<'a> {
    pub entry: &'a AssetEntry,
    /// The project-relative path this entry takes, tokens resolved — the same
    /// spelling the copy is called with.
    pub rel: String,
    pub action: FileAction,
}

/// Resolve a walked `files/` subtree against a template's globs: **the one
/// place that decides what happens to a template file.** The copy, the dry
/// run's file list and previews, and `apply`'s plan all read it, which is what
/// makes `docs/cli.md`'s "the preview is built by the same code the commit
/// runs" true. A preview must never iterate the in-memory text buffer instead:
/// it holds every UTF-8 file under `files/`, so an `exclude`d file would
/// preview a body it never gets and a `verbatim` one its `{braces}` filled in.
///
/// One [`PlannedEntry`] per walked entry, `Skipped` included, so a caller that
/// counts entries — a failpoint, a progress bar — still sees them all. It is
/// infallible on purpose: this decides *policy*, and each caller keeps its own
/// `SafeRelativePath` validation, which is load-bearing in `apply`, the one
/// path that never goes through `plan()`.
pub fn plan_entries<'a>(
    entries: &'a [AssetEntry],
    exclude: &[String],
    verbatim: &[String],
    vars: &HashMap<String, String>,
    ctx: &crate::core::naming::RenderContext,
) -> Vec<PlannedEntry<'a>> {
    entries
        .iter()
        .map(|entry| {
            // Globs are matched against the path **as written in the
            // template** — an author writes `exclude: ["*.tmp"]` about the
            // files they can see, not about the names those become.
            let excluded = is_excluded(&entry.rel, exclude);
            let rel = interp_rel_with(&entry.rel, vars, ctx);
            let action = if excluded
                // fastf owns PROJECT_INFO.md and its journals — a bundled file
                // may never clobber one, so it is not written and not promised.
                || crate::core::project_info::path_is_reserved(&rel)
                || crate::core::provisioning::path_is_reserved(&rel)
            {
                FileAction::Skipped
            } else if entry.is_dir() {
                FileAction::Folder
            } else if !entry.is_file() {
                FileAction::Unsupported
            } else if is_verbatim(&entry.rel, verbatim) || entry.size > TEXT_MAX_BYTES {
                FileAction::Verbatim
            } else {
                FileAction::Interpolated
            };
            PlannedEntry { entry, rel, action }
        })
        .collect()
}

/// Copy one file atomically through a unique sibling temp.
///
/// When `force_verbatim` is false the source is read as UTF-8 and, if that
/// succeeds, `{token}` interpolation is applied to its contents. A read failure
/// (binary) transparently falls back to a byte copy. `force_verbatim` short-
/// circuits straight to the byte copy (used for `verbatim` globs and oversize
/// files).
///
/// `dest_root` is the tree the copy must stay inside, and `rel` is the path
/// beneath it: the two are joined here, through
/// [`crate::util::paths::contained_destination`], immediately before the write.
/// An already-joined path would skip exactly the check that matters —
/// `create_dir_all` walks straight through an existing `docs -> /outside`.
pub fn copy_file(
    src: &Path,
    dest_root: &Path,
    rel: &Path,
    force_verbatim: bool,
    vars: &HashMap<String, String>,
    ctx: &crate::core::naming::RenderContext,
) -> Result<()> {
    let dest = crate::util::paths::contained_destination(dest_root, rel)?;
    let dest = dest.as_path();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating parent dirs for {}",
                crate::util::paths::display_path(dest)
            )
        })?;
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

    /// A text file under the interpolation cap is interpolated, however large;
    /// one forced verbatim keeps its `{braces}`.
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
            tmp.path(),
            Path::new("out_small.txt"),
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
            tmp.path(),
            Path::new("out_verbatim.txt"),
            true,
            &vars,
            &crate::core::naming::RenderContext::now("%Y-%m-%d"),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello {name}");

        // A comfortably large — but still under the cap — text file must also be
        // interpolated. A README of a few hundred KB is ordinary in a template,
        // and shipping it with raw `{tokens}` would fail in silence.
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
            tmp.path(),
            Path::new("out_big.md"),
            false,
            &vars,
            &crate::core::naming::RenderContext::now("%Y-%m-%d"),
        )
        .unwrap();
        let out = fs::read_to_string(&dest).unwrap();
        assert!(out.ends_with("# Aurora\n"), "large text must interpolate");
        assert!(!out.contains("{name}"));
    }

    /// The limit itself: a file of exactly [`TEXT_MAX_BYTES`] is interpolated,
    /// and one byte more is copied as it is.
    #[test]
    fn a_file_at_the_size_limit_is_interpolated_and_one_byte_over_is_not() {
        let entry = |rel: &str, size: u64| AssetEntry {
            rel: rel.to_string(),
            os_rel: PathBuf::from(rel),
            kind: EntryKind::File,
            size,
        };
        let entries = [
            entry("at.md", TEXT_MAX_BYTES),
            entry("over.md", TEXT_MAX_BYTES + 1),
        ];
        let planned = plan_entries(
            &entries,
            &[],
            &[],
            &HashMap::new(),
            &crate::core::naming::RenderContext::now("%Y-%m-%d"),
        );
        let actions: Vec<FileAction> = planned.iter().map(|planned| planned.action).collect();
        assert_eq!(actions, [FileAction::Interpolated, FileAction::Verbatim]);
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

    /// **A name only opts into separator collapse by containing a token.**
    ///
    /// The collapse exists so an empty optional variable does not leave a
    /// dangling `_` behind; a component with no `{` in it has no variable to
    /// vanish, so there is nothing to clean up and the name is meant
    /// literally — in `interp_rel_with`, which every *preview* goes through,
    /// as in `interp_rel_os`, which the copy writes through.
    #[test]
    fn a_component_with_no_token_is_left_exactly_as_written() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Aurora".to_string());

        assert_eq!(
            interp_rel("pkg/__init__.py", &vars, "%Y-%m-%d"),
            "pkg/__init__.py",
            "a literal dunder survives: it is not a collapsed variable"
        );
        assert_eq!(
            interp_rel("__pycache__/_leading", &vars, "%Y-%m-%d"),
            "__pycache__/_leading"
        );
        // And a component that *does* carry a token still collapses.
        vars.insert("client".to_string(), String::new());
        assert_eq!(
            interp_rel("{client}_{name}.md", &vars, "%Y-%m-%d"),
            "Aurora.md",
            "an empty optional variable still takes its separator with it"
        );
    }

    /// The path a preview renders and the path the copy writes are the same
    /// path.
    #[test]
    fn the_previewed_name_is_the_written_name() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Aurora".to_string());
        vars.insert("client".to_string(), String::new());
        let ctx = crate::core::naming::RenderContext::now("%Y-%m-%d");

        for rel in [
            "pkg/__init__.py",
            "docs/{name}/README.md",
            "{client}_{name}/notes.md",
            "plain/nested/file.txt",
            "__pycache__/keep",
        ] {
            let previewed = interp_rel_with(rel, &vars, &ctx);
            let written = interp_rel_os(Path::new(rel), &vars, &ctx);
            assert_eq!(
                PathBuf::from(previewed.replace('/', std::path::MAIN_SEPARATOR_STR)),
                written,
                "preview and write disagree about {rel}"
            );
        }
    }

    /// The names a job's progress is written with. A job's state file holds
    /// them and every other fastf reads them, so a rename is a format change,
    /// not a refactor.
    #[test]
    fn job_status_and_phase_serialize_to_stable_names() {
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
            (JobPhase::Starting, "starting"),
            (JobPhase::Scanning, "scanning"),
            (JobPhase::Copying, "copying"),
            (JobPhase::SettingAside, "setting-aside"),
            (JobPhase::Removing, "removing"),
            (JobPhase::Done, "done"),
        ] {
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{name}\"")
            );
        }
    }

    /// A progress record written by one version reads in another: unknown
    /// fields are ignored and missing ones take their defaults.
    #[test]
    fn a_progress_record_reads_whatever_it_can() {
        let read: super::Progress =
            serde_json::from_str(r#"{"phase":"removing","step_done":3,"from_the_future":1}"#)
                .unwrap();
        assert_eq!(read.phase, super::JobPhase::Removing);
        assert_eq!(read.step_done, 3);
        assert_eq!(read.count_text(), "3 entries");
    }

    #[test]
    fn a_count_names_its_unit_and_its_total_when_known() {
        use super::{JobPhase, count_text};
        assert_eq!(
            count_text(JobPhase::Removing, 312, 1473),
            "312 of 1473 entries"
        );
        assert_eq!(count_text(JobPhase::Copying, 2, 0), "2 files");
        assert_eq!(count_text(JobPhase::Publishing, 1, 1), "");
        assert_eq!(count_text(JobPhase::Copying, 1, 0), "1 file");
        assert_eq!(count_text(JobPhase::Removing, 1, 1), "1 of 1 entry");
        assert_eq!(count_text(JobPhase::Removing, 1, 2), "1 of 2 entries");
    }
}
