//! Re-resolving a cached project against the filesystem before anything
//! destructive. A cached `Project` is a hint; this is what turns it into a fact.

use anyhow::{Context, Result};
use std::path::Path;

use crate::core::config::Config;
use crate::core::project_info;

use super::discovery::*;
use super::model::*;
use std::fs;

/// Re-resolve a cached/discovered project against the configured filesystem
/// boundary before an operation that can rename or delete anything.
///
/// Caches are hints only. The project must still be a real (non-symlink) direct
/// child of a currently configured base, and its real `PROJECT_INFO.md` must
/// carry the same ID as the candidate supplied by the caller.
pub fn revalidate_project(cfg: &Config, candidate: &Project) -> Result<Project> {
    let candidate_base = candidate
        .base
        .canonicalize()
        .with_context(|| format!("resolving project base {}", candidate.base.display()))?;
    let configured = cfg
        .effective_bases()
        .into_iter()
        .filter_map(|base| base.canonicalize().ok())
        .find(|base| *base == candidate_base)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "refusing to modify '{}': its base {} is not currently configured",
                candidate.name,
                candidate.base.display()
            )
        })?;
    revalidate_project_in_base(candidate, &configured)
}

/// The cheap sibling of `revalidate_project_in_base`, for handing a
/// discovered path to **another program**.
///
/// `fastf open` and the TUI's Reveal spawn the system file manager on a path
/// that came from a cache, and a cache is a file that travels with the projects
/// — a synced folder or an unpacked archive can bring one along. The write paths
/// have always revalidated; these read paths did not, so a forged entry named
/// the directory that got opened.
///
/// Deliberately *not* the full guard: no canonicalize, no config reload, no id
/// comparison. Those exist to protect a mutation. Opening a folder needs three
/// things — it is really a directory, it is a direct child of its own base, and
/// it holds a `PROJECT_INFO.md`, which is what makes it a project at all.
///
/// Ordinary metadata reads keep trusting discovery: after `CacheEntry::into_project`'s
/// one-component rule the path is a direct child of the base by construction,
/// and reading the user's own `PROJECT_INFO.md` is what discovery *is*.
pub fn revalidate_for_read(project: &Project) -> Result<()> {
    crate::util::paths::require_real_directory(&project.path, "project folder")?;
    if project.path.parent() != Some(project.base.as_path()) {
        anyhow::bail!(
            "refusing to open: {} is not a direct child of its base {}",
            crate::util::paths::display_path(&project.path),
            crate::util::paths::display_path(&project.base)
        );
    }
    crate::util::paths::require_real_file(
        &project_info::pinfo_path(&project.path),
        "project metadata",
    )
    .with_context(|| {
        format!(
            "refusing to open {}: it is not a project folder",
            crate::util::paths::display_path(&project.path)
        )
    })?;
    Ok(())
}

/// The compatibility-library boundary does not own a [`Config`], but it still
/// refuses stale, forged, linked, or non-child project records.
pub(crate) fn revalidate_recorded_project(candidate: &Project) -> Result<Project> {
    let base = candidate
        .base
        .canonicalize()
        .with_context(|| format!("resolving project base {}", candidate.base.display()))?;
    revalidate_project_in_base(candidate, &base)
}

pub(crate) fn revalidate_project_in_base(candidate: &Project, base: &Path) -> Result<Project> {
    crate::util::paths::require_real_directory(base, "project base")?;
    crate::util::paths::require_real_directory(&candidate.path, "project source")?;
    let path = candidate
        .path
        .canonicalize()
        .with_context(|| format!("resolving project {}", candidate.path.display()))?;
    if path.parent() != Some(base) {
        anyhow::bail!(
            "refusing to modify: {} is not a direct child of configured base {}",
            path.display(),
            base.display()
        );
    }

    let pinfo = project_info::pinfo_path(&path);
    let pinfo_metadata = match fs::symlink_metadata(&pinfo) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "refusing to modify: {} has no PROJECT_INFO.md",
                path.display()
            );
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("checking project identity at {}", pinfo.display()));
        }
    };
    if pinfo_metadata.file_type().is_symlink() || !pinfo_metadata.file_type().is_file() {
        anyhow::bail!(
            "refusing to modify: {} is not a real PROJECT_INFO.md file",
            pinfo.display()
        );
    }
    let metadata = project_info::read_metadata(&path)?
        .ok_or_else(|| anyhow::anyhow!("{} has no readable project identity", pinfo.display()))?;
    if metadata.id != candidate.id {
        anyhow::bail!(
            "refusing to modify '{}': project identity changed (expected {}, found {})",
            candidate.name,
            candidate.id,
            metadata.id
        );
    }
    Ok(project_from_meta(metadata, base, &path))
}
