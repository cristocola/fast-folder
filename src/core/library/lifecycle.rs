//! Unregister, delete and rename: the three mutations that are not a move.

use anyhow::Result;
use std::path::Path;

use crate::core::config::Config;
use crate::core::project_info;

use super::cache::*;
use super::guard::*;
use super::model::*;
use crate::core::assets;

// ---------------------------------------------------------------------------
// Unregister / delete / rename (v1.0)
// ---------------------------------------------------------------------------

/// Unregister a project: remove its `PROJECT_INFO.md` so it stops being a
/// project. The folder and everything else inside it are untouched.
///
/// **Mutates without holding [`crate::util::lockfile::DataLock`]**, which the
/// name is there to admit. Applications call
/// [`unregister_project_configured`]; this shape exists for library callers and
/// tests that supply their own tree.
#[doc(hidden)]
pub fn unregister_project_unlocked(project: &Project) -> Result<()> {
    let project = revalidate_recorded_project(project)?;
    unregister_project_inner(&project)
}

/// Application entry point for unregistering. Configuration and project
/// identity are reloaded while holding the mutation lock, so a stale cache or
/// configuration change cannot authorize removal of a different metadata file.
pub fn unregister_project_configured(project: &Project) -> Result<()> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let config = Config::load()?;
    let project = revalidate_project(&config, project)?;
    unregister_project_inner(&project)
}

pub(crate) fn unregister_project_inner(project: &Project) -> Result<()> {
    let pinfo = project_info::pinfo_path(&project.path);
    if !pinfo.is_file() {
        anyhow::bail!(
            "'{}' has no PROJECT_INFO.md — already unregistered?",
            project.name
        );
    }
    crate::util::fs_retry::remove_file(&pinfo)?;
    remove_from_base_cache(project);
    Ok(())
}

/// Permanently delete a project's folder (recursive).
///
/// Guards before any removal: the folder must still contain a
/// `PROJECT_INFO.md` (never `remove_dir_all` an arbitrary path) and must be a
/// direct child of its base. Callers additionally restrict operations to
/// configured bases and confirm with the user — same convention as move.
///
/// **Mutates without holding [`crate::util::lockfile::DataLock`]**, which the
/// name is there to admit — and this one removes a tree. Applications call
/// [`delete_project_configured`].
#[doc(hidden)]
pub fn delete_project_unlocked(project: &Project) -> Result<()> {
    let project = revalidate_recorded_project(project)?;
    delete_project_inner(&project)
}

/// Application entry point for deletion with configured-base and identity
/// validation performed under the mutation lock.
pub fn delete_project_configured(project: &Project) -> Result<()> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let config = Config::load()?;
    let project = revalidate_project(&config, project)?;
    delete_project_inner(&project)
}

pub(crate) fn delete_project_inner(project: &Project) -> Result<()> {
    let path = project
        .path
        .canonicalize()
        .unwrap_or_else(|_| project.path.clone());
    let base = project
        .base
        .canonicalize()
        .unwrap_or_else(|_| project.base.clone());
    if path.parent() != Some(base.as_path()) {
        anyhow::bail!(
            "refusing to delete: {} is not a direct child of its base {}",
            path.display(),
            base.display()
        );
    }
    if !project_info::pinfo_path(&path).is_file() {
        anyhow::bail!(
            "refusing to delete: {} has no PROJECT_INFO.md",
            path.display()
        );
    }
    crate::util::fs_retry::remove_dir_all(&path)?;
    remove_from_base_cache(project);
    Ok(())
}

/// Rename a project's folder in place (same base). Same-parent `fs::rename`
/// is atomic; the metadata `folder`/`path` are patched best-effort (display
/// truth only, like move) and the base cache is updated. Returns the renamed
/// [`Project`].
///
/// **Mutates without holding [`crate::util::lockfile::DataLock`]**, which the
/// name is there to admit. Applications call [`rename_project_configured`].
#[doc(hidden)]
pub fn rename_project_unlocked(project: &Project, new_folder: &str) -> Result<Project> {
    let project = revalidate_recorded_project(project)?;
    rename_project_inner(&project, new_folder)
}

/// Application entry point for rename with configured-base and identity
/// validation performed under the mutation lock.
pub fn rename_project_configured(project: &Project, new_folder: &str) -> Result<Project> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let config = Config::load()?;
    let project = revalidate_project(&config, project)?;
    rename_project_inner(&project, new_folder)
}

/// What to say when a case-only rename could neither commit nor be undone.
///
/// The folder is parked under a dot-prefixed staging name at this point, and
/// discovery skips dot-prefixed directories. Reporting only the rename failure
/// would be a lie by omission: the project has not stayed put, it has become
/// invisible, and nothing but this message says where it went.
pub(crate) fn stranded_rename_message(context: &str, staging: &Path, rollback: &str) -> String {
    format!(
        "{context}; the folder is left at {} and could not be put back ({rollback}) \
         — rename it back by hand to make the project visible again",
        crate::util::paths::display_path(staging)
    )
}

pub(crate) fn rename_project_inner(project: &Project, new_folder: &str) -> Result<Project> {
    let sanitized = crate::core::validated::ProjectFolderName::parse(new_folder)?.into_string();
    if sanitized == project.name {
        anyhow::bail!("'{}' is already the folder's name", sanitized);
    }

    let base = project
        .base
        .canonicalize()
        .unwrap_or_else(|_| project.base.clone());
    let new_path = base.join(&sanitized);

    // A rename that only changes capitalisation is legitimate — and common, when
    // tidying up a folder name. On Windows `exists()` is case-insensitive, so the
    // target "already exists": it is the source. Detect that and go through a
    // temporary name, which is the only way the OS will apply the new casing.
    let case_only_change = sanitized.eq_ignore_ascii_case(&project.name);
    if case_only_change {
        let mut staging = base.join(format!(".{sanitized}.fastf-case"));
        let mut attempt = 0;
        while assets::entry_exists(&staging)? {
            attempt += 1;
            staging = base.join(format!(".{sanitized}.fastf-case{attempt}"));
        }
        crate::util::fs_retry::rename(&project.path, &staging)?;
        if let Err(err) = crate::util::fs_retry::rename(&staging, &new_path) {
            let context = format!("renaming '{}' to '{}'", project.name, sanitized);
            // Put it back rather than leaving the project under a dot-prefixed
            // name, which discovery skips — that would make it vanish. Retried
            // like every other destructive rename: a Windows sharing violation is
            // exactly the kind of thing that failed the commit a moment ago.
            if let Err(rollback) = crate::util::fs_retry::rename(&staging, &project.path) {
                return Err(anyhow::anyhow!(err).context(stranded_rename_message(
                    &context,
                    &staging,
                    &rollback.to_string(),
                )));
            }
            return Err(anyhow::anyhow!(err).context(context));
        }
    } else {
        if assets::entry_exists(&new_path)? {
            anyhow::bail!("rename target already exists: {}", new_path.display());
        }
        crate::util::fs_retry::rename(&project.path, &new_path)?;
    }

    let mut renamed = project.clone();
    renamed.path = new_path.canonicalize().unwrap_or(new_path);
    renamed.name = sanitized.clone();
    renamed.base = base.clone();

    // Keep the displayed metadata truthful; discovery never reads `folder` or
    // `path`, so a failure here is a warning, not a failed rename.
    let pinfo = project_info::pinfo_path(&renamed.path);
    if pinfo.exists()
        && let Err(err) = project_info::write_frontmatter(&pinfo, |meta| {
            meta.folder = sanitized.clone();
            meta.path = crate::util::paths::display_path(&renamed.path);
        })
    {
        crate::util::diag::warn(format!(
            "could not update PROJECT_INFO.md folder/path: {err:#}"
        ));
    }

    remove_from_base_cache(project);
    cache_upsert(&base, &renamed);
    Ok(renamed)
}

/// Drop a project's entry from its base cache, best-effort (mirrors the
/// old-side bookkeeping of a completed move).
pub(crate) fn remove_from_base_cache(project: &Project) {
    let base = project
        .base
        .canonicalize()
        .unwrap_or_else(|_| project.base.clone());
    let dir = project
        .path
        .strip_prefix(&base)
        .map(to_forward_slashes)
        .unwrap_or_else(|_| project.name.clone());
    cache_remove(&base, &dir);
}
