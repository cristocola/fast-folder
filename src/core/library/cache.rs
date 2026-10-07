//! The per-base `.fastf-index.json`: a disposable accelerator, never authority.
//!
//! Entries are **base-relative**, so a cache written on Linux
//! (`/mnt/projects/...`) is valid when the same base is read on Windows
//! (`D:\\...`). Every write here is best-effort and atomic: a cache failure
//! never fails a command, because the folders are the truth.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::naming;

use super::discovery::project_from_meta;
use super::discovery::{read_project_meta, scan_listing};
use super::model::{CACHE_FILENAME, Project};

// ---------------------------------------------------------------------------
// Cache model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheEntry {
    /// Base-relative directory (portable across OSes / drive letters).
    pub(crate) dir: String,
    pub(crate) id: String,
    /// The number behind the id. `serde(default)` rather than a version bump:
    /// an older cache simply reads as `None`, and `Project::number`'s fallback
    /// covers it, so nothing has to be rescanned to adopt this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) id_number: Option<u64>,
    pub(crate) template: String,
    pub(crate) template_name: String,
    pub(crate) name: String,
    pub(crate) created: String,
    /// One line, or absent. `serde(default)` with no version bump, as
    /// `id_number`: an older fastf that rewrites the index drops the key, and
    /// a list then shows no description until the project is opened or the
    /// base reindexed, which is untidy and nothing worse.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Cache {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) entries: Vec<CacheEntry>,
    /// **Every name the base held when this index was built** — folders and
    /// files, dot-names left out — sorted, as the listing the scan walked
    /// returned them. The index is current while a names-only listing of the
    /// base returns exactly these ([`super::discovery::freshness`]): one
    /// request, and the one question an rclone base can answer, whose folder
    /// times read 2000-01-01 once its directory cache expires. Times alone
    /// also miss a project added while a scan that missed it is being
    /// written. fastf's own writers keep it current (`cache_upsert`,
    /// `cache_remove`). `None` in an index an older fastf wrote, or one
    /// written without a listing: stale once, and the rescan records them.
    /// No version bump: an older fastf ignores the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) seen: Option<Vec<String>>,
}

pub(crate) const CACHE_VERSION: u32 = 1;

impl CacheEntry {
    pub(crate) fn from_project(project: &Project, base: &Path) -> Self {
        // `dir` is base-relative; fall back to the basename if strip fails
        // (shouldn't happen — projects are always built as `base.join(...)`).
        Self {
            dir: entry_dir(project, base),
            id: project.id.clone(),
            id_number: project.id_number,
            template: project.template.clone(),
            template_name: project.template_name.clone(),
            name: project.name.clone(),
            created: project.created.clone(),
            description: project.description.clone(),
            tags: project.tags.clone(),
        }
    }

    /// Rebuild a `Project` from a cache entry, or drop the entry.
    ///
    /// **A cache entry is a hint, and a hint may not name a path outside its
    /// own base.** `Path::join` *replaces* the base when given an absolute
    /// path, so an unchecked entry reading `/etc` would be a "project" at
    /// `/etc`, and `../..` would survive the `strip_prefix` on the next
    /// rewrite. Caches travel with the projects by design — that is what
    /// makes them portable across operating systems — so a synced folder or
    /// an unpacked archive is a delivery route for one, and overwriting the
    /// file in place does not bump the base's mtime, so a planted cache reads
    /// as fresh.
    ///
    /// Discovery is depth-1 (`SCAN_DEPTH`), so a legitimate `dir` is exactly
    /// one ordinary path component and never dot-prefixed (`scan_base` skips
    /// those). Anything else answers `None`, and the caller abandons the whole
    /// cache for the rescan that rebuilds it from the folders.
    pub(crate) fn into_project(self, base: &Path) -> Option<Project> {
        let dir = crate::core::validated::SafeRelativePath::parse(&self.dir).ok()?;
        let dir = dir.as_str();
        if dir.contains('/') || dir.starts_with('.') {
            return None;
        }

        Some(Project {
            id: self.id,
            id_number: self.id_number,
            template: self.template,
            template_name: self.template_name,
            name: self.name,
            path: base.join(dir),
            base: base.to_path_buf(),
            created: self.created,
            description: self.description,
            tags: self.tags,
            exists: true,
        })
    }
}

pub(crate) fn to_forward_slashes(p: &Path) -> String {
    p.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------
// Cache I/O
// ---------------------------------------------------------------------------

pub(crate) fn cache_path(base: &Path) -> PathBuf {
    base.join(CACHE_FILENAME)
}

/// Load a base's cache, or `None` if it is absent, unreadable, or not a version
/// this build understands.
///
/// The version check matters more than it looks. Caches are deliberately
/// co-located with the projects so they travel between machines, which means an
/// older fastf will meet caches written by a newer one. Without this, a cache
/// whose shape happened to still deserialize would be *trusted* — and every
/// project it failed to describe would silently vanish from the library.
/// Rejecting an unknown version costs one rescan and cannot hide anything.
pub(crate) fn load_cache(base: &Path) -> Option<Cache> {
    let raw = fs::read_to_string(cache_path(base)).ok()?;
    let cache = serde_json::from_str::<Cache>(&raw).ok()?;
    (cache.version == CACHE_VERSION).then_some(cache)
}

/// One base's own picture of itself, read from `.fastf-index.json` and nothing
/// else — no staleness check, no directory scan, no metadata read.
///
/// This exists for the guided app's first frame, which must cost nothing. A
/// summary that scanned would make opening the app slower the larger the
/// library got, which is exactly backwards; a summary a few minutes out of
/// date is fine as long as it says so, as the app's "from index" does.
#[derive(Debug, Clone)]
pub struct IndexSummary {
    pub projects: usize,
    /// Highest ID in the cache, by numeric value.
    pub max_id: Option<String>,
    /// Newest project's id and folder name (the cache is not sorted, so this is
    /// computed by `created`).
    pub newest: Option<(String, String)>,
}

/// `None` when the base has no readable cache of a version this build knows.
pub fn index_summary(base: &Path) -> Option<IndexSummary> {
    crate::util::paths::stall_if_marked(base);
    let cache = load_cache(base)?;
    let max_id = cache
        .entries
        .iter()
        .max_by_key(|entry| entry.id_number.or_else(|| naming::id_value(&entry.id)))
        .map(|entry| entry.id.clone());
    let newest = cache
        .entries
        .iter()
        .max_by(|a, b| a.created.cmp(&b.created))
        .map(|entry| (entry.id.clone(), entry.name.clone()));
    Some(IndexSummary {
        projects: cache.entries.len(),
        max_id,
        newest,
    })
}

/// Write the cache for `base` atomically, with the names the listing it was
/// built from saw (`seen`, see [`Cache::seen`]). Best-effort: a failure is
/// returned but callers ignore it — the cache is disposable and the folders
/// remain the truth.
///
/// Uses the shared [`crate::util::atomic`] writer, whose temp name carries the
/// process id: two fastf processes refreshing the same base cache never
/// collide on a single fixed `.tmp` path.
pub(crate) fn write_index(
    base: &Path,
    projects: &[Project],
    seen: Option<Vec<String>>,
) -> Result<()> {
    let cache = Cache {
        version: CACHE_VERSION,
        entries: projects
            .iter()
            .map(|p| CacheEntry::from_project(p, base))
            .collect(),
        seen,
    };
    crate::util::atomic::write_json(&cache_path(base), &cache)?;
    // The atomic write stamps the file when its bytes are written and the base
    // directory when the rename publishes it — two instants, and the kernel's
    // file clock is coarse enough (a few milliseconds) that they straddle a
    // tick every so often. Then `cache_is_stale` reads the base as newer than
    // its own index and the next discovery rescans it for nothing. Re-stamping
    // the file after the rename puts it at or after the directory, always.
    super::touch_cache(base);
    Ok(())
}

/// [`write_index`] with the names the base holds as it is written — what a
/// test that plants an index means: current, whatever it says.
#[cfg(test)]
pub(crate) fn write_cache(base: &Path, projects: &[Project]) -> Result<()> {
    write_index(base, projects, super::discovery::base_names(base).ok())
}

// ---------------------------------------------------------------------------
// Cache mutation helpers (used by writers so list/search stay fresh without a
// full rescan). All best-effort — a cache error never fails the command.
// ---------------------------------------------------------------------------

/// A project's base-relative directory, the way a cache entry records it.
fn entry_dir(project: &Project, base: &Path) -> String {
    project
        .path
        .strip_prefix(base)
        .map(to_forward_slashes)
        .unwrap_or_else(|_| project.name.clone())
}

/// Insert or update `project` in `base`'s cache (matched by base-relative dir).
/// If the cache is missing/unreadable, seed it from a full scan first so the new
/// entry lands in a complete cache.
pub fn cache_upsert(base: &Path, project: &Project) {
    let (mut projects, seen): (Vec<Project>, _) = match load_cache(base) {
        Some(cache) => (
            cache
                .entries
                .into_iter()
                .filter_map(|e| e.into_project(base))
                .collect(),
            cache.seen,
        ),
        None => {
            let scanned = scan_listing(base);
            (scanned.projects, scanned.names)
        }
    };
    // The base-relative directory is the identity. Computing it directly beats
    // building a throwaway `CacheEntry` — with every other field cloned — once
    // per project already in the cache, just to read one string off it.
    let new_dir = entry_dir(project, base);
    projects.retain(|p| entry_dir(p, base) != new_dir);
    projects.push(project.clone());
    // The folder is there — fastf made it, or read it just now — so the index
    // knows its name. Only fastf's own change is added: a folder someone else
    // put in the base meanwhile still makes the next listing differ.
    let seen = seen.map(|mut names| {
        if let Err(at) = names.binary_search(&new_dir) {
            names.insert(at, new_dir);
        }
        names
    });
    let _ = write_index(base, &projects, seen);
}

/// Re-read a project's `PROJECT_INFO.md` and refresh its entry in the base
/// cache. Best-effort — used after tag mutations so `recent`/`search` reflect
/// the change without a full rescan. No-op if the folder has no readable
/// metadata or no parent.
pub fn refresh_cache(project_dir: &Path) {
    let Some(meta) = read_project_meta(project_dir) else {
        return;
    };
    let Some(base) = project_dir.parent() else {
        return;
    };
    let project = project_from_meta(meta, base, project_dir);
    cache_upsert(base, &project);
}

/// Remove the entry for base-relative `dir` from `base`'s cache. No-op when the
/// cache is missing (the entry is already absent, definitionally).
pub(crate) fn cache_remove(base: &Path, dir: &str) {
    let Some(cache) = load_cache(base) else {
        return;
    };
    let target = dir.replace('\\', "/");
    let projects: Vec<Project> = cache
        .entries
        .into_iter()
        .filter(|e| e.dir != target)
        .filter_map(|e| e.into_project(base))
        .collect();
    // The name goes only with the folder: an unregistered project's folder
    // stays, and an old copy emptied in place keeps its name until it is
    // gone. A folder that does not answer keeps it too; if it has gone, the
    // next listing says so.
    let gone = matches!(
        crate::util::paths::presence(&base.join(&target)),
        crate::util::paths::Presence::Absent
    );
    let seen = cache.seen.map(|mut names| {
        if gone {
            names.retain(|name| *name != target);
        }
        names
    });
    let _ = write_index(base, &projects, seen);
}

#[cfg(test)]
mod tests {
    use super::write_cache;
    use crate::core::library::discovery::{cache_is_stale, dir_mtime};
    use crate::core::library::model::CACHE_FILENAME;

    /// The rename that publishes the index bumps the base directory; the file
    /// must never be left older than the directory it sits in.
    #[test]
    fn a_freshly_written_index_is_never_older_than_its_base() {
        let base = tempfile::tempdir().unwrap();
        for _ in 0..20 {
            write_cache(base.path(), &[]).unwrap();
            let file = dir_mtime(&base.path().join(CACHE_FILENAME)).unwrap();
            let dir = dir_mtime(base.path()).unwrap();
            assert!(file >= dir, "index {file:?} older than base {dir:?}");
            assert!(!cache_is_stale(base.path()));
        }
    }
}
