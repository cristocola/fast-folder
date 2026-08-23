//! Moving a project into another base.
//!
//! Its own module, next to `transactions`, because it depends on a different
//! world from the rest of the library: staged copies, manifests, retries,
//! failpoints and a progress handle. `library::move_project*` delegates here.
//!
//! **The invariant, restated because it is the whole point: the source is never
//! removed until the destination is fully copied and verified.**

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::core::assets::{self, JobPhase, Progress};
use crate::core::config::Config;
use crate::core::library::{
    Project, cache_remove, cache_upsert, project_from_meta, revalidate_project,
    revalidate_recorded_project, to_forward_slashes,
};
use crate::core::project_info;
use crate::core::transactions::{self, MoveManifest, MovePhase, MoveTransaction};

#[derive(Debug, Clone)]
pub struct MoveOutcome {
    pub project: Project,
    pub cleanup_pending: bool,
}

/// Move a project folder into another base directory, keeping its folder name.
///
/// The historical compatibility shape, kept for library callers: it holds the
/// coarse data lock, revalidates the recorded source base and identity beneath
/// it, and runs with throwaway progress/cancel handles. Applications use
/// [`move_project_configured_with_outcome`], which also revalidates the target
/// against freshly loaded configuration.
///
/// **Safety invariant: the source is never removed until the destination is
/// fully copied AND verified.** Same-filesystem moves take an instant, atomic
/// `fs::rename`. Cross-filesystem / network moves use a private v2 transaction
/// below the target base, verify exact path/type/size topology plus a second
/// source metadata scan, atomically publish the staging directory, and only
/// then remove the source.
pub fn move_project(project: &Project, new_base: &Path) -> Result<Project> {
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    let outcome = {
        let _data_lock = crate::util::lockfile::DataLock::acquire()?;
        let revalidated = revalidate_recorded_project(project)?;
        move_project_unlocked(&revalidated, new_base, &progress, &cancel)?
    };
    report_cleanup_pending(&outcome, &project.path);
    Ok(outcome.project)
}

/// Application move entry point. It reloads configuration under the coarse
/// mutation lock, then revalidates both source and target against that fresh
/// snapshot before touching either path.
pub fn move_project_configured_with_outcome(
    project: &Project,
    new_base: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<MoveOutcome> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let cfg = Config::load()?;
    let project = revalidate_project(&cfg, project)?;
    let wanted = new_base
        .canonicalize()
        .with_context(|| format!("resolving target base {}", new_base.display()))?;
    let target = cfg
        .effective_bases()
        .into_iter()
        .filter_map(|base| base.canonicalize().ok())
        .find(|base| *base == wanted)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "'{}' is not a currently configured base",
                new_base.display()
            )
        })?;
    move_project_unlocked(&project, &target, progress, cancel)
}

/// Exercise the private copy transaction even when the test's two bases share
/// a filesystem. This is intentionally absent from release builds; production
/// always lets the OS rename first and stages only after `EXDEV`.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn move_project_staged_for_test(project: &Project, new_base: &Path) -> Result<MoveOutcome> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let cfg = Config::load()?;
    let project = revalidate_project(&cfg, project)?;
    let wanted = new_base
        .canonicalize()
        .with_context(|| format!("resolving target base {}", new_base.display()))?;
    let target = cfg
        .effective_bases()
        .into_iter()
        .filter_map(|base| base.canonicalize().ok())
        .find(|base| *base == wanted)
        .ok_or_else(|| anyhow::anyhow!("'{}' is not a configured base", new_base.display()))?;
    let old_base = project
        .base
        .canonicalize()
        .with_context(|| format!("resolving source base {}", project.base.display()))?;
    if target == old_base {
        anyhow::bail!("move target is the source base");
    }
    let folder = project
        .path
        .file_name()
        .map(PathBuf::from)
        .context("move source has no folder name")?;
    let new_path = target.join(&folder);
    if assets::entry_exists(&new_path)? {
        anyhow::bail!("move target already exists: {}", new_path.display());
    }
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    staged_copy_verify_commit(&project, &target, &new_path, &progress, &cancel)
}

fn move_project_unlocked(
    project: &Project,
    new_base: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<MoveOutcome> {
    crate::util::paths::require_real_directory(new_base, "target base")?;
    let new_base = new_base
        .canonicalize()
        .with_context(|| format!("resolving target base {}", new_base.display()))?;
    let old_base = project
        .base
        .canonicalize()
        .with_context(|| format!("resolving source base {}", project.base.display()))?;
    if new_base == old_base {
        anyhow::bail!(
            "'{}' is already in base {}",
            project.name,
            new_base.display()
        );
    }

    let folder_os = project.path.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "project path has no folder name: {}",
            project.path.display()
        )
    })?;
    let folder = PathBuf::from(folder_os);
    let new_path = new_base.join(&folder);
    if assets::entry_exists(&new_path)? {
        anyhow::bail!("move target already exists: {}", new_path.display());
    }

    // Fast path: same-filesystem rename is atomic and instant — no staging,
    // no verification needed (there is no window in which data is half-there).
    // It also preserves links perfectly, because nothing is copied, so the link
    // refusal in the transaction scanner deliberately applies only to the
    // staged fallback.
    // Deliberately NOT `fs_retry::rename`: this call is *expected* to fail on a
    // cross-device move, and that failure is the signal to take the staged path.
    // Retrying would add the full backoff to every cross-drive move for nothing.
    let outcome = match fs::rename(&project.path, &new_path) {
        Ok(()) => {
            let moved = finish_move_bookkeeping(project, &old_base, &new_base, &new_path);
            MoveOutcome {
                project: moved,
                cleanup_pending: false,
            }
        }
        Err(error) if is_cross_device_error(&error) => {
            return staged_copy_verify_commit(project, &new_base, &new_path, progress, cancel);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "renaming project {} to {}",
                    project.path.display(),
                    new_path.display()
                )
            });
        }
    };
    Ok(outcome)
}

pub(crate) fn is_cross_device_error(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::EXDEV)
    }
    #[cfg(windows)]
    {
        // ERROR_NOT_SAME_DEVICE
        error.raw_os_error() == Some(17)
    }
    #[cfg(not(any(unix, windows)))]
    {
        error.kind() == std::io::ErrorKind::CrossesDevices
    }
}

fn report_cleanup_pending(outcome: &MoveOutcome, source: &Path) {
    if outcome.cleanup_pending {
        crate::util::diag::warn(format!(
            "move published at {}, but source cleanup is pending at {}; \
             the move transaction was retained",
            outcome.project.path.display(),
            source.display()
        ));
    }
}

/// The staged cross-filesystem move body. All pre-publication state lives in
/// one exclusively-created operation directory; cancellation or an ordinary
/// error before publication removes exactly that directory and leaves the
/// source untouched. Once publication begins cancellation is deliberately too
/// late.
pub(crate) fn staged_copy_verify_commit(
    project: &Project,
    new_base: &Path,
    new_path: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<MoveOutcome> {
    use std::sync::atomic::Ordering;

    let old_base = project
        .base
        .canonicalize()
        .with_context(|| format!("resolving source base {}", project.base.display()))?;
    let folder = project
        .path
        .file_name()
        .map(PathBuf::from)
        .context("move source has no folder name")?;
    crate::util::faults::check("move:before-marker-write")?;
    let mut transaction =
        MoveTransaction::begin(&old_base, &folder, new_base, &folder, &project.id)?;
    let mut published = false;
    let pre_publication = (|| -> Result<MoveManifest> {
        let manifest = MoveManifest::scan(&project.path)?;
        transaction.write_manifest(&manifest)?;
        {
            let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
            state.phase = crate::core::assets::JobPhase::Copying;
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
                anyhow::bail!("move of '{}' cancelled", project.name);
            }
            return Err(error)
                .with_context(|| format!("copying '{}' into private staging", project.name));
        }
        crate::util::faults::check("move:after-staging")?;
        set_phase(progress, JobPhase::Verifying);
        manifest.verify_destination(&staging)?;
        manifest.verify_source_unchanged(&project.path)?;
        crate::util::faults::check("move:after-verify")?;
        crate::util::faults::check("move:post-verification")?;

        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("move of '{}' cancelled", project.name);
        }
        set_phase(progress, JobPhase::Finalizing);
        transaction.set_phase(MovePhase::ReadyToCommit)?;
        crate::util::faults::check("move:before-commit-rename")?;
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("move of '{}' cancelled", project.name);
        }
        if assets::entry_exists(new_path)? {
            anyhow::bail!("move target became occupied: {}", new_path.display());
        }
        crate::util::fs_retry::rename(&staging, new_path)
            .with_context(|| format!("finalizing move into {}", new_path.display()))?;
        published = true;
        Ok(manifest)
    })();

    let manifest = match pre_publication {
        Ok(manifest) => manifest,
        Err(error) if !published => {
            return match transaction.remove() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(error).context(format!(
                    "also could not remove the owned transaction: {cleanup:#}"
                )),
            };
        }
        Err(error) => {
            // Publication is a point of no return. Preserve the transaction and
            // return a truthful successful outcome with cleanup pending.
            crate::util::diag::warn(format!(
                "move published at {}, but cleanup is pending ({error:#})",
                new_path.display()
            ));
            let moved = moved_view(project, new_base, new_path);
            return Ok(MoveOutcome {
                project: moved,
                cleanup_pending: true,
            });
        }
    };

    let mut cleanup_pending = false;
    let mut retain_transaction = false;

    if let Err(error) = crate::util::faults::check("move:after-publication")
        .and_then(|()| crate::util::faults::check("move:after-commit-before-source-removal"))
    {
        crate::util::diag::warn(format!(
            "move published at {}, but cleanup is pending ({error:#})",
            new_path.display()
        ));
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = transaction.set_phase(MovePhase::CleanupPending) {
        crate::util::diag::warn(format!(
            "move published at {}, but the cleanup phase could not be recorded ({error:#})",
            new_path.display()
        ));
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = crate::util::faults::check("move:before-source-cleanup")
        .and_then(|()| crate::util::faults::check("move:source-cleanup"))
    {
        crate::util::diag::warn(format!(
            "move published at {}, but source cleanup is pending at {} ({error:#})",
            new_path.display(),
            project.path.display()
        ));
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = revalidate_recorded_project(project)
        .and_then(|_| manifest.verify_recovery_pair(&project.path, new_path))
        .and_then(|_| crate::util::fs_retry::remove_dir_all(&project.path).map_err(Into::into))
    {
        crate::util::diag::warn(format!(
            "move published at {}, but source cleanup is pending at {} ({error:#})",
            new_path.display(),
            project.path.display()
        ));
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = crate::util::faults::check("move:after-source-cleanup") {
        crate::util::diag::warn(format!(
            "source cleanup completed, but transaction cleanup is pending ({error:#})"
        ));
        retain_transaction = true;
    }

    let moved = if cleanup_pending {
        moved_view(project, new_base, new_path)
    } else {
        finish_move_bookkeeping(project, &old_base, new_base, new_path)
    };
    if !retain_transaction && let Err(error) = transaction.remove() {
        crate::util::diag::warn(format!(
            "could not clear completed move transaction: {error:#}"
        ));
    }
    set_phase(progress, JobPhase::Done);
    Ok(MoveOutcome {
        project: moved,
        cleanup_pending,
    })
}

fn finish_move_bookkeeping(
    project: &Project,
    old_base: &Path,
    new_base: &Path,
    new_path: &Path,
) -> Project {
    let moved = moved_view(project, new_base, new_path);

    let pinfo = project_info::pinfo_path(&moved.path);
    if let Err(error) = project_info::write_frontmatter(&pinfo, |metadata| {
        metadata.path = crate::util::paths::display_path(&moved.path);
        metadata.folder = moved.name.clone();
    }) {
        crate::util::diag::warn(format!(
            "could not update PROJECT_INFO.md after move: {error:#}"
        ));
    }

    let old_dir = project
        .path
        .strip_prefix(old_base)
        .map(to_forward_slashes)
        .unwrap_or_else(|_| project.name.clone());
    cache_remove(old_base, &old_dir);
    cache_upsert(new_base, &moved);
    moved
}

fn moved_view(project: &Project, new_base: &Path, new_path: &Path) -> Project {
    let mut moved = project.clone();
    moved.path = new_path
        .canonicalize()
        .unwrap_or_else(|_| new_path.to_path_buf());
    moved.base = new_base.to_path_buf();
    moved
}

/// Complete bookkeeping for a move recovered from a v2 transaction.
pub(crate) fn finish_recovered_move(
    source_base: &Path,
    source_folder: &Path,
    target_base: &Path,
    final_path: &Path,
) -> Result<()> {
    let metadata = project_info::read_metadata(final_path)?
        .ok_or_else(|| anyhow::anyhow!("recovered destination has no readable metadata"))?;
    let original = project_from_meta(metadata, source_base, &source_base.join(source_folder));
    finish_move_bookkeeping(&original, source_base, target_base, final_path);
    Ok(())
}

fn set_phase(progress: &Mutex<Progress>, phase: JobPhase) {
    if let Ok(mut p) = progress.lock() {
        p.phase = phase;
        // A phase change is real movement: without it, verifying a large tree
        // looks identical to a dead worker to both the staleness floor and the
        // frontend's stall notice.
        p.touch();
    }
}
