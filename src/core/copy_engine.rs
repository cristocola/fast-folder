//! Copying a project to somewhere outside the library.
//!
//! A copy is a move that keeps its source: the same manifest scan, the same
//! private staging, the same exact path/type/size verification, the same atomic
//! publish — and then nothing, because the source was never the thing being
//! given up. `move_engine` and this module share
//! [`transactions`] rather than each other, so the
//! one invariant they both live by is stated in one place: **a destination is
//! published only after it has been copied and verified in full.**
//!
//! **The copy keeps its ID.** It is the same project on another drive, and the
//! base is what tells two of them apart — which is why the destination may not
//! be inside a configured base. Two rows with one id in one library is a
//! library that cannot answer "which one"; two rows with one id in two bases
//! is a backup, and the BASE column says which is which.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::assets::{self, JobPhase, Progress};
use crate::core::config::Config;
use crate::core::library::{Project, revalidate_project};
use crate::core::transactions::{self, MoveManifest, MoveTransaction};

#[derive(Debug, Clone)]
pub struct CopyOutcome {
    /// Where the copy landed.
    pub path: PathBuf,
    /// Files and bytes copied.
    pub copied: (usize, u64),
}

/// Copy `project` into `destination`, keeping its folder name and its id.
///
/// Holds the data lock, reloads the configuration under it and revalidates the
/// source against that fresh snapshot — the same guard every other mutation
/// takes — then checks the destination and stages.
pub fn copy_project_configured(
    project: &Project,
    destination: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<CopyOutcome> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let cfg = Config::load()?;
    let project = revalidate_project(&cfg, project)?;
    let destination = resolve_destination(&cfg, &project, destination)?;
    copy_unlocked(&project, &destination, progress, cancel)
}

/// What a destination has to be, and why.
///
/// A real directory that is not inside the project being copied — the obvious
/// infinite one, and checked **first** because a project sits inside a base and
/// the base rule would otherwise answer it with the wrong sentence — and not a
/// configured base or inside one. That second rule keeps the library's
/// one-id-one-row property: a copy into a base is a duplicate fastf itself
/// cannot tell apart, and it would be made by a keystroke.
///
/// Returns the canonical destination *folder* — `destination/<the project's
/// folder name>` — which is what gets published.
pub fn resolve_destination(cfg: &Config, project: &Project, destination: &Path) -> Result<PathBuf> {
    let root = destination.canonicalize().with_context(|| {
        format!(
            "resolving the copy destination {}",
            crate::util::paths::display_path(destination)
        )
    })?;
    crate::util::paths::require_real_directory(&root, "copy destination")?;

    let source = project
        .path
        .canonicalize()
        .unwrap_or_else(|_| project.path.clone());
    if root == source || root.starts_with(&source) {
        anyhow::bail!(
            "'{}' is inside the project being copied",
            crate::util::paths::display_path(&root)
        );
    }

    for base in cfg.effective_bases() {
        let Ok(base) = base.canonicalize() else {
            continue;
        };
        if root == base || root.starts_with(&base) {
            anyhow::bail!(
                "'{}' is inside the configured base {} — a copy there would be a \
                 second project with id {}, and nothing could tell the two apart. \
                 Copy somewhere outside your bases; if you want it in the library, \
                 add that folder as a base afterwards and both will list, told \
                 apart by their base.",
                crate::util::paths::display_path(&root),
                crate::util::paths::display_path(&base),
                project.id
            );
        }
    }

    let folder = project
        .path
        .file_name()
        .map(PathBuf::from)
        .context("the project path has no folder name")?;
    let target = root.join(&folder);
    if assets::entry_exists(&target)? {
        anyhow::bail!(
            "'{}' already exists — nothing was copied",
            crate::util::paths::display_path(&target)
        );
    }
    Ok(target)
}

/// The staged body. Everything before publication lives in one exclusively
/// created operation directory under the destination; a cancellation or a
/// failure removes exactly that and leaves both ends untouched.
fn copy_unlocked(
    project: &Project,
    target: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<CopyOutcome> {
    let root = target
        .parent()
        .map(Path::to_path_buf)
        .context("the copy destination has no parent")?;
    let source_base = project
        .base
        .canonicalize()
        .with_context(|| format!("resolving project base {}", project.base.display()))?;
    let folder = project
        .path
        .file_name()
        .map(PathBuf::from)
        .context("the project path has no folder name")?;

    crate::util::faults::check("copy:before-marker-write")?;
    let transaction = MoveTransaction::begin(&source_base, &folder, &root, &folder, &project.id)?;

    let staged = (|| -> Result<(usize, u64)> {
        // Deny-by-default, exactly as a cross-drive move is: a link cannot be
        // reproduced faithfully somewhere else, and following one would
        // silently restructure the copy.
        let manifest = MoveManifest::scan(&project.path)?;
        transaction.write_manifest(&manifest)?;
        let totals = (manifest.total_files(), manifest.total_bytes());
        {
            let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
            state.phase = JobPhase::Copying;
            state.total_bytes = manifest.total_bytes();
            state.total_files = manifest.total_files();
            state.done_files = 0;
            state.copied_bytes = 0;
            state.touch();
        }
        let staging = transaction.claim_staging()?;
        if let Err(error) =
            transactions::copy_to_staging(&manifest, &project.path, &staging, progress, cancel)
        {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("copy of '{}' cancelled", project.name);
            }
            return Err(error)
                .with_context(|| format!("copying '{}' into private staging", project.name));
        }
        crate::util::faults::check("copy:after-staging")?;
        set_phase(progress, JobPhase::Verifying);
        manifest.verify_destination(&staging)?;
        // The source has to be what it was when the manifest was taken, or the
        // copy is of two different moments.
        manifest.verify_source_unchanged(&project.path)?;
        crate::util::faults::check("copy:after-verify")?;
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("copy of '{}' cancelled", project.name);
        }
        set_phase(progress, JobPhase::Finalizing);
        if assets::entry_exists(target)? {
            anyhow::bail!(
                "the copy destination became occupied: {}",
                crate::util::paths::display_path(target)
            );
        }
        crate::util::fs_retry::rename(&staging, target)
            .with_context(|| format!("publishing the copy at {}", target.display()))?;
        Ok(totals)
    })();

    // **Whatever happened, the transaction goes.** There is no cleanup-pending
    // state here: a move keeps its transaction when the *source* could not be
    // removed, and a copy removes no source.
    let removal = transaction.remove();
    let copied = staged?;
    if let Err(error) = removal {
        crate::util::diag::warn(format!(
            "could not clear the completed copy transaction: {error:#}"
        ));
    }
    set_phase(progress, JobPhase::Done);
    finish_progress(progress);
    Ok(CopyOutcome {
        path: target.to_path_buf(),
        copied,
    })
}

fn set_phase(progress: &Mutex<Progress>, phase: JobPhase) {
    if let Ok(mut state) = progress.lock() {
        state.phase = phase;
        state.touch();
    }
}

fn finish_progress(progress: &Mutex<Progress>) {
    if let Ok(mut state) = progress.lock() {
        state.status = crate::core::assets::JobStatus::Done;
        state.touch();
    }
}
