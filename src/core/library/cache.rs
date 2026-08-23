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
use super::discovery::{read_project_meta, scan_base};
use super::model::{CACHE_FILENAME, Project};

// ---------------------------------------------------------------------------
// Cache model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CacheEntry {
    /// Base-relative directory (portable across OSes / drive letters).
    pub(crate) dir: String,
    pub(crate) id: String,
    pub(crate) template: String,
    pub(crate) template_name: String,
    pub(crate) name: String,
    pub(crate) created: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Cache {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) entries: Vec<CacheEntry>,
}

pub(crate) const CACHE_VERSION: u32 = 1;

impl CacheEntry {
    pub(crate) fn from_project(project: &Project, base: &Path) -> Self {
        // `dir` is base-relative; fall back to the basename if strip fails
        // (shouldn't happen — projects are always built as `base.join(...)`).
        Self {
            dir: entry_dir(project, base),
            id: project.id.clone(),
            template: project.template.clone(),
            template_name: project.template_name.clone(),
            name: project.name.clone(),
            created: project.created.clone(),
            tags: project.tags.clone(),
        }
    }

    pub(crate) fn into_project(self, base: &Path) -> Project {
        let path = base.join(self.dir.replace('/', std::path::MAIN_SEPARATOR_STR));
        Project {
            id: self.id,
            template: self.template,
            template_name: self.template_name,
            name: self.name,
            path,
            base: base.to_path_buf(),
            created: self.created,
            tags: self.tags,
            exists: true,
        }
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
/// This exists for the main-menu frame, which must cost nothing. A summary that
/// scanned would make opening the menu slower the larger the library got, which
/// is exactly backwards; a summary that is a few minutes out of date is fine as
/// long as it says so, which is why every surface labels it "from index".
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
    let cache = load_cache(base)?;
    let max_id = cache
        .entries
        .iter()
        .max_by_key(|entry| naming::id_value(&entry.id))
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

/// Write the cache for `base` atomically. Best-effort: a failure is returned but
/// callers ignore it — the cache is disposable and the folders remain the truth.
///
/// Uses the shared [`crate::util::atomic`] writer, whose temp name carries the
/// process id: two fastf processes refreshing the same base cache no longer
/// collide on a single fixed `.tmp` path.
pub(crate) fn write_cache(base: &Path, projects: &[Project]) -> Result<()> {
    let cache = Cache {
        version: CACHE_VERSION,
        entries: projects
            .iter()
            .map(|p| CacheEntry::from_project(p, base))
            .collect(),
    };
    crate::util::atomic::write_json(&cache_path(base), &cache)
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
    let mut projects: Vec<Project> = match load_cache(base) {
        Some(cache) => cache
            .entries
            .into_iter()
            .map(|e| e.into_project(base))
            .collect(),
        None => scan_base(base),
    };
    // The base-relative directory is the identity. Computing it directly beats
    // building a throwaway `CacheEntry` — with every other field cloned — once
    // per project already in the cache, just to read one string off it.
    let new_dir = entry_dir(project, base);
    projects.retain(|p| entry_dir(p, base) != new_dir);
    projects.push(project.clone());
    let _ = write_cache(base, &projects);
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
        .map(|e| e.into_project(base))
        .collect();
    let _ = write_cache(base, &projects);
}
