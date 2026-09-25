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
use crate::core::progress::Ticker;
use crate::core::transactions::{self, MoveManifest, MoveTransaction, Operation};

#[derive(Debug, Clone)]
pub struct CopyOutcome {
    /// Where the copy landed.
    pub path: PathBuf,
    /// Files and bytes copied.
    pub copied: (usize, u64),
    /// Links carried as links.
    pub links: usize,
    /// Links whose meaning the new place may change (`MoveManifest::link_notes`).
    pub link_notes: Vec<String>,
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
    let result = (|| {
        let ticker = Ticker::new(progress, cancel);
        ticker.subject(format!(
            "copy {} {} to {}",
            project.id,
            project.name,
            crate::util::paths::display_path(destination)
        ));
        let _data_lock = crate::util::lockfile::DataLock::acquire_then(|| {
            ticker.phase(JobPhase::Waiting, 0);
        })?;
        ticker.update(|state| state.holds_lock = true);
        let copied = (|| {
            let cfg = Config::load()?;
            let project = revalidate_project(&cfg, project)?;
            let destination = resolve_destination(&cfg, &project, destination)?;
            copy_unlocked(&project, &destination, progress, cancel)
        })();
        ticker.update(|state| state.holds_lock = false);
        copied
    })();
    crate::core::progress::settle(progress, cancel, &result);
    result
}

/// The steps a copy passes through, in order.
pub const COPY_STEPS: &[JobPhase] = &[
    JobPhase::Scanning,
    JobPhase::Copying,
    JobPhase::Verifying,
    JobPhase::Publishing,
    JobPhase::Clearing,
];

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
    let root = crate::util::paths::canonical(destination).with_context(|| {
        format!(
            "resolving the copy destination {}",
            crate::util::paths::display_path(destination)
        )
    })?;
    crate::util::paths::require_real_directory(&root, "copy destination")?;

    let source =
        crate::util::paths::canonical(&project.path).unwrap_or_else(|_| project.path.clone());
    if root == source || root.starts_with(&source) {
        anyhow::bail!(
            "'{}' is inside the project being copied",
            crate::util::paths::display_path(&root)
        );
    }

    for base in cfg.effective_bases() {
        // An unplugged base holds nothing to collide with. Any other failure
        // refuses: a base that cannot be resolved cannot be proven apart from
        // the destination, and a skipped check is a copy into the library.
        let base = match crate::util::paths::canonical(&base) {
            Ok(base) => base,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => anyhow::bail!(
                "cannot resolve the configured base {} ({err}), so the copy cannot \
                 be checked against it",
                crate::util::paths::display_path(&base)
            ),
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
    let source_base = crate::util::paths::canonical(&project.base)
        .with_context(|| format!("resolving project base {}", project.base.display()))?;
    let folder = project
        .path
        .file_name()
        .map(PathBuf::from)
        .context("the project path has no folder name")?;

    let ticker = Ticker::new(progress, cancel);
    ticker.plan(COPY_STEPS);
    crate::util::faults::check("copy:before-marker-write")?;
    let transaction = MoveTransaction::begin(
        &source_base,
        &folder,
        &root,
        &folder,
        &project.id,
        Operation::Copy,
    )?;

    let staged = (|| -> Result<((usize, u64), usize, Vec<String>)> {
        // The same scan as a cross-drive move: links recorded as links, never
        // followed, and anything it cannot copy refused, all of it named.
        ticker.phase(JobPhase::Scanning, 0);
        let manifest = MoveManifest::scan_with(&project.path, ticker)?;
        transaction.write_manifest(&manifest)?;
        // A copy removes nothing, so it needs no write access to its source —
        // only the room to land.
        crate::core::move_preflight::check_space(&root, manifest.total_bytes())?;
        let totals = (
            (manifest.total_files(), manifest.total_bytes()),
            manifest.total_links(),
            manifest.link_notes(&project.path),
        );
        // Everything but `PROJECT_INFO.md`, which the publish writes: the
        // step counts what it copies, so its bar reaches its end.
        let body = manifest.without_root_metadata();
        ticker.phase(JobPhase::Copying, body.total_files());
        ticker.update(|state| {
            state.total_bytes = manifest.total_bytes();
            state.total_files = manifest.total_files();
            state.done_files = 0;
            state.copied_bytes = 0;
        });
        // Made at its final path, `PROJECT_INFO.md` last — see `transactions`.
        let staging = transaction.claim_staging()?;
        if let Err(error) =
            transactions::copy_to_staging(&body, &project.path, &staging, progress, cancel)
        {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("copy of '{}' cancelled", project.name);
            }
            return Err(error)
                .with_context(|| format!("copying '{}' into {}", project.name, root.display()));
        }
        crate::util::faults::check("copy:after-staging")?;
        ticker.phase(
            JobPhase::Verifying,
            body.entries.len() + manifest.entries.len(),
        );
        // The source has to be what it was when the manifest was taken, or the
        // copy is of two different moments.
        let verified = body
            .verify_destination_with(&staging, ticker)
            .and_then(|_| manifest.verify_source_unchanged_with(&project.path, ticker));
        if verified.is_err() && ticker.cancelled() {
            anyhow::bail!("copy of '{}' cancelled", project.name);
        }
        verified?;
        crate::util::faults::check("copy:after-verify")?;
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("copy of '{}' cancelled", project.name);
        }
        ticker.phase(JobPhase::Publishing, 0);
        ticker.update(|state| state.committed = true);
        transactions::copy_to_staging(
            &manifest.only_root_metadata(),
            &project.path,
            &staging,
            progress,
            &AtomicBool::new(false),
        )
        .with_context(|| format!("publishing the copy at {}", target.display()))?;
        Ok(totals)
    })();

    // **Whatever happened, the transaction goes.** There is no cleanup-pending
    // state here: a move keeps its transaction when the *source* could not be
    // removed, and a copy removes no source. An unpublished copy goes with it
    // (`MoveTransaction::remove`); a published one is at `Copying` in the
    // record but holds its `PROJECT_INFO.md`, which `remove` sees.
    ticker.phase(JobPhase::Clearing, 0);
    let removal = transaction.remove();
    let (copied, links, link_notes) = staged?;
    if let Err(error) = removal {
        crate::util::diag::warn(format!(
            "could not clear the completed copy transaction: {error:#}"
        ));
    }
    Ok(CopyOutcome {
        path: target.to_path_buf(),
        copied,
        links,
        link_notes,
    })
}
