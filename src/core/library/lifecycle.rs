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
// Unregister / delete / rename
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

/// The suffix on the folder a case-only rename passes through.
///
/// The staging name is `.<target>.fastf-case[n]`: dot-prefixed so nothing can
/// mistake it for a project while it is there, and carrying the **target** name
/// so the operation can still be finished by anything that finds it later.
/// `provisioning::reconcile` is that anything — this is spelled here, beside the
/// only writer, and read there.
pub(crate) const CASE_STAGING_SUFFIX: &str = ".fastf-case";

/// The staging folder name for a case-only rename to `target`, attempt `n`.
pub(crate) fn case_staging_name(target: &str, attempt: u32) -> String {
    if attempt == 0 {
        format!(".{target}{CASE_STAGING_SUFFIX}")
    } else {
        format!(".{target}{CASE_STAGING_SUFFIX}{attempt}")
    }
}

/// The folder name a case-only staging directory was on its way to, or `None`
/// if `name` is not one of ours.
pub(crate) fn case_staging_target(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('.')?;
    let at = rest.rfind(CASE_STAGING_SUFFIX)?;
    // Whatever trails the suffix is the collision counter and must be digits —
    // otherwise `.notes.fastf-case-backup` would be read as ours.
    let (target, trailing) = rest.split_at(at);
    if !trailing[CASE_STAGING_SUFFIX.len()..]
        .chars()
        .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    (!target.is_empty()).then_some(target)
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
    //
    // **Folded over the whole string, not just its ASCII.** This was
    // `eq_ignore_ascii_case`, which sees no difference to fold in `проект` →
    // `ПРОЕКТ`: the rename was classified as an ordinary one, `entry_exists`
    // answered `true` because NTFS *is* case-insensitive over Cyrillic, and
    // the verb bailed with `rename target already exists` — the file it was
    // being asked to rename. The identical ASCII rename worked, so the bug
    // was invisible to a test suite written in English.
    //
    // `to_lowercase` is full Unicode simple lowercasing rather than NTFS's own
    // uppercase table, so the two can still disagree at the margins (`ß`
    // against `SS`, say). They disagree safely: a name this calls case-only
    // that NTFS thinks is distinct merely takes the staging path and arrives
    // correctly anyway, and the reverse — the case that failed — is what this
    // fixes.
    let case_only_change = sanitized.to_lowercase() == project.name.to_lowercase();
    if case_only_change {
        let mut attempt = 0;
        let mut staging = base.join(case_staging_name(&sanitized, attempt));
        while assets::entry_exists(&staging)? {
            attempt += 1;
            staging = base.join(case_staging_name(&sanitized, attempt));
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

#[cfg(test)]
mod case_staging_tests {
    use super::{case_staging_name, case_staging_target};

    #[test]
    fn a_staging_name_names_the_folder_it_was_going_to() {
        assert_eq!(case_staging_name("Album", 0), ".Album.fastf-case");
        assert_eq!(case_staging_name("Album", 3), ".Album.fastf-case3");
        assert_eq!(case_staging_target(".Album.fastf-case"), Some("Album"));
        assert_eq!(case_staging_target(".Album.fastf-case3"), Some("Album"));
    }

    #[test]
    fn nothing_else_is_read_as_ours() {
        // Somebody else's dot-folder, and near-misses of our own shape.
        for name in [
            ".git",
            "Album.fastf-case",
            ".fastf-case",
            ".Album.fastf-case-backup",
            ".Album.fastf-caseX",
        ] {
            assert_eq!(case_staging_target(name), None, "{name} is not ours");
        }
    }

    #[test]
    fn a_target_that_contains_the_suffix_survives_the_round_trip() {
        // `rfind`, not `find`: the last occurrence is the one we appended.
        let staged = case_staging_name("Album.fastf-case", 0);
        assert_eq!(case_staging_target(&staged), Some("Album.fastf-case"));
    }
}
