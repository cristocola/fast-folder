//! Durable provisioning markers (v0.11).
//!
//! Two flows write large amounts of data outside the fast request/response path:
//! background asset copies during `fastf new` (UI) and staged cross-filesystem
//! moves. Both leave a small on-disk marker so a crash mid-copy is always
//! recoverable and never silent data loss:
//!
//!   - **Create marker** — `.fastf-provisioning.json` inside the new project
//!     root, listing the deferred file copies still in flight. Deleted once every
//!     copy has landed. Its presence means "this project is not fully provisioned."
//!   - **Move marker** — `.fastf-move-<folder>.json` at the *target* base root,
//!     recording the source, the `.part` staging folder, and the final path. The
//!     source is only ever removed after the destination is copied AND verified,
//!     so reconcile is trivial: finish the commit if it already happened, else
//!     roll the staging folder back and leave the source untouched.
//!
//! Everything here is best-effort and never panics: the folders are the truth,
//! the markers are just a to-do list for recovery.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::core::assets::{self, CopyJob, Progress};
use crate::core::config::Config;
use crate::core::library;

/// Filename of the per-project create-provisioning marker.
pub const MARKER_CREATE: &str = ".fastf-provisioning.json";
/// Filename prefix of a per-base staged-move marker (`.fastf-move-<folder>.json`).
pub const MARKER_MOVE_PREFIX: &str = ".fastf-move-";

const MARKER_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Create marker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeferredCopy {
    src: String,
    dest: String,
    bytes: u64,
    done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CreateMarker {
    version: u32,
    started_at: String,
    jobs: Vec<DeferredCopy>,
}

fn create_marker_path(root: &Path) -> PathBuf {
    root.join(MARKER_CREATE)
}

/// Write (or overwrite) the create marker into a new project root, one entry per
/// deferred copy, all `done: false`. Call this *before* the background copy
/// starts so a crash has a record to reconcile from.
pub fn write_create_marker(root: &Path, jobs: &[CopyJob]) -> Result<()> {
    let marker = CreateMarker {
        version: MARKER_VERSION,
        started_at: library::now_iso8601(),
        jobs: jobs
            .iter()
            .map(|j| DeferredCopy {
                src: j.src.display().to_string(),
                dest: j.dest.display().to_string(),
                bytes: j.bytes,
                done: false,
            })
            .collect(),
    };
    write_atomic(&create_marker_path(root), &marker)
}

/// Flip the entry for `dest` to `done` in a project's create marker. Best-effort:
/// a missing/unreadable marker is a no-op.
pub fn mark_done(root: &Path, dest: &Path) {
    let path = create_marker_path(root);
    let Some(mut marker) = read_json::<CreateMarker>(&path) else {
        return;
    };
    let target = dest.display().to_string();
    for job in &mut marker.jobs {
        if job.dest == target {
            job.done = true;
        }
    }
    let _ = write_atomic(&path, &marker);
}

/// Delete a project's create marker — the project is now fully provisioned.
pub fn clear_create(root: &Path) {
    let _ = fs::remove_file(create_marker_path(root));
}

// ---------------------------------------------------------------------------
// Move marker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MoveMarker {
    version: u32,
    started_at: String,
    src: String,
    temp: String,
    final_path: String,
    phase: String,
}

/// The staging-folder path for a cross-filesystem move (`.<folder>.fastf-part`
/// under the target base). Dot-prefixed so discovery skips it.
pub fn staging_path(target_base: &Path, folder: &str) -> PathBuf {
    target_base.join(format!(".{folder}.fastf-part"))
}

fn move_marker_path(target_base: &Path, folder: &str) -> PathBuf {
    target_base.join(format!("{MARKER_MOVE_PREFIX}{folder}.json"))
}

/// Write the staged-move marker at the target base root.
pub fn write_move_marker(
    target_base: &Path,
    folder: &str,
    src: &Path,
    temp: &Path,
    final_path: &Path,
    phase: &str,
) -> Result<()> {
    let marker = MoveMarker {
        version: MARKER_VERSION,
        started_at: library::now_iso8601(),
        src: src.display().to_string(),
        temp: temp.display().to_string(),
        final_path: final_path.display().to_string(),
        phase: phase.to_string(),
    };
    write_atomic(&move_marker_path(target_base, folder), &marker)
}

/// Delete a staged-move marker (the move committed or was rolled back).
pub fn clear_move(target_base: &Path, folder: &str) {
    let _ = fs::remove_file(move_marker_path(target_base, folder));
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

/// One incomplete provisioning item, for the UI banner / `/api/state`.
#[derive(Debug, Clone, Serialize)]
pub struct Incomplete {
    /// Project root (create) or final target path (move).
    pub path: String,
    /// `"create"` or `"move"`.
    pub kind: String,
    /// Files still pending (create) or `0` (move — it's all-or-nothing).
    pub pending: usize,
}

/// Depth-1 scan of every base for provisioning markers, for display. Cheap and
/// read-only — no copying is performed.
pub fn list_incomplete(cfg: &Config) -> Vec<Incomplete> {
    let mut out = Vec::new();
    for base in cfg.effective_bases() {
        let Ok(read_dir) = fs::read_dir(&base) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if let Some(marker) = read_json::<CreateMarker>(&create_marker_path(&path)) {
                    let pending = marker.jobs.iter().filter(|j| !j.done).count();
                    out.push(Incomplete {
                        path: path.display().to_string(),
                        kind: "create".to_string(),
                        pending,
                    });
                }
            } else if name.starts_with(MARKER_MOVE_PREFIX)
                && name.ends_with(".json")
                && let Some(marker) = read_json::<MoveMarker>(&path)
            {
                out.push(Incomplete {
                    path: marker.final_path,
                    kind: "move".to_string(),
                    pending: 0,
                });
            }
        }
    }
    out
}

/// Outcome of a reconcile pass.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ReconcileReport {
    /// Create jobs finished (pending copies completed).
    pub resumed: usize,
    /// Moves whose commit was finished (source removed, marker cleared).
    pub completed: usize,
    /// Staged moves rolled back (staging removed, source left intact).
    pub rolled_back: usize,
    /// Items that could not be recovered (e.g. a create source template file is
    /// gone) — surfaced to the user, marker left in place.
    pub unrecoverable: Vec<String>,
}

impl ReconcileReport {
    pub fn is_empty(&self) -> bool {
        self.resumed == 0
            && self.completed == 0
            && self.rolled_back == 0
            && self.unrecoverable.is_empty()
    }
}

/// Reconcile all incomplete provisioning across every base. Resumes create
/// copies, finishes or rolls back staged moves. Best-effort — a per-item failure
/// is recorded, never fatal.
pub fn reconcile(cfg: &Config) -> ReconcileReport {
    let mut report = ReconcileReport::default();
    for base in cfg.effective_bases() {
        let Ok(read_dir) = fs::read_dir(&base) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() && create_marker_path(&path).exists() {
                reconcile_create(&path, &mut report);
            } else if name.starts_with(MARKER_MOVE_PREFIX) && name.ends_with(".json") {
                reconcile_move(&base, &path, &mut report);
            }
        }
    }
    report
}

/// Finish a project's outstanding deferred copies. A copy already on disk with
/// the right size counts as done; a missing source is unrecoverable.
fn reconcile_create(root: &Path, report: &mut ReconcileReport) {
    let path = create_marker_path(root);
    let Some(mut marker) = read_json::<CreateMarker>(&path) else {
        return;
    };
    let mut all_done = true;
    for job in &mut marker.jobs {
        if job.done {
            continue;
        }
        let dest = PathBuf::from(&job.dest);
        if dest.exists() && fs::metadata(&dest).map(|m| m.len()).unwrap_or(0) == job.bytes {
            job.done = true;
            continue;
        }
        let src = PathBuf::from(&job.src);
        if !src.exists() {
            all_done = false;
            report
                .unrecoverable
                .push(format!("{} (missing source {})", job.dest, job.src));
            continue;
        }
        let copy = CopyJob {
            src,
            dest,
            bytes: job.bytes,
        };
        let progress = Mutex::new(Progress::new(std::slice::from_ref(&copy)));
        match assets::copy_job(&copy, &progress, &AtomicBool::new(false)) {
            Ok(()) => {
                job.done = true;
                report.resumed += 1;
            }
            Err(_) => {
                all_done = false;
                report.unrecoverable.push(job.dest.clone());
            }
        }
    }
    if all_done {
        clear_create(root);
    } else {
        let _ = write_atomic(&path, &marker);
    }
}

/// Finish or roll back a staged move. If the final target already exists the
/// commit happened — remove any leftover source and clear the marker. Otherwise
/// roll back: delete the staging folder, leave the source intact.
fn reconcile_move(base: &Path, marker_path: &Path, report: &mut ReconcileReport) {
    let Some(marker) = read_json::<MoveMarker>(marker_path) else {
        let _ = fs::remove_file(marker_path);
        return;
    };
    let final_path = PathBuf::from(&marker.final_path);
    let src = PathBuf::from(&marker.src);
    let temp = PathBuf::from(&marker.temp);

    if final_path.exists() {
        // Commit already landed. Clean up any source the crash didn't reach.
        if src.exists() && src != final_path {
            let _ = fs::remove_dir_all(&src);
        }
        let _ = fs::remove_file(marker_path);
        report.completed += 1;
    } else {
        // Nothing committed — discard the partial copy; the source is untouched.
        let _ = fs::remove_dir_all(&temp);
        let _ = fs::remove_file(marker_path);
        report.rolled_back += 1;
    }
    let _ = base; // base kept for symmetry / future per-base cache refresh
}

// ---------------------------------------------------------------------------
// Small JSON helpers (atomic write, tolerant read)
// ---------------------------------------------------------------------------

fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let raw = serde_json::to_string_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<T>(&raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_marker_round_trip_and_mark_done() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let job = CopyJob {
            src: root.join("src.bin"),
            dest: root.join("dest.bin"),
            bytes: 10,
        };
        write_create_marker(root, std::slice::from_ref(&job)).unwrap();
        assert!(create_marker_path(root).exists());

        mark_done(root, &job.dest);
        let marker = read_json::<CreateMarker>(&create_marker_path(root)).unwrap();
        assert!(marker.jobs[0].done);

        clear_create(root);
        assert!(!create_marker_path(root).exists());
    }

    #[test]
    fn reconcile_resumes_pending_create_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let root = base.join("proj");
        fs::create_dir_all(&root).unwrap();
        let src = base.join("asset.bin");
        let data = vec![7u8; 5000];
        fs::write(&src, &data).unwrap();
        let dest = root.join("asset.bin");
        let job = CopyJob {
            src: src.clone(),
            dest: dest.clone(),
            bytes: data.len() as u64,
        };
        write_create_marker(&root, std::slice::from_ref(&job)).unwrap();

        let cfg = Config {
            base_dir: base.display().to_string(),
            ..Default::default()
        };
        let report = reconcile(&cfg);
        assert_eq!(report.resumed, 1);
        assert_eq!(fs::read(&dest).unwrap(), data);
        // Marker cleared once everything landed.
        assert!(!create_marker_path(&root).exists());
    }

    #[test]
    fn reconcile_rolls_back_uncommitted_move_leaving_source_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let src = base.join("proj");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("keep.txt"), "important").unwrap();
        let temp = staging_path(base, "proj");
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("keep.txt"), "partial").unwrap();
        // The final target does not exist yet → the commit never happened.
        let final_path = base.join("committed_target_that_does_not_exist");
        write_move_marker(base, "proj", &src, &temp, &final_path, "copying").unwrap();

        let cfg = Config {
            base_dir: base.display().to_string(),
            ..Default::default()
        };
        let report = reconcile(&cfg);
        assert_eq!(report.rolled_back, 1);
        assert!(!temp.exists(), "staging removed");
        assert_eq!(
            fs::read_to_string(src.join("keep.txt")).unwrap(),
            "important",
            "source untouched"
        );
        assert!(!move_marker_path(base, "proj").exists(), "marker cleared");
    }
}
