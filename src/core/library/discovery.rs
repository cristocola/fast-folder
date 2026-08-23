//! Finding projects: the cache-accelerated walk over each configured base.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::core::config::Config;
use crate::core::project_info::{self, Metadata};

use super::cache::*;
use super::model::*;

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Discover every project across all effective bases, newest first.
///
/// Per base: cache-first with a staleness gate (see module docs). Absent /
/// unmounted bases are skipped honestly rather than surfacing stale entries.
pub fn discover(cfg: &Config) -> Vec<Project> {
    crate::util::trace::hit("discover");
    let mut all = Vec::new();
    for base in cfg.effective_bases() {
        if !base.is_dir() {
            continue;
        }
        all.extend(discover_base(&base));
    }
    // Sort by created desc; ISO-8601 sorts lexicographically. Ties broken by
    // name for stable ordering.
    all.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
    all
}

/// Cache-first discovery for a single base with the staleness gate applied.
pub(crate) fn discover_base(base: &Path) -> Vec<Project> {
    match load_cache(base) {
        None => {
            let projects = scan_base(base);
            let _ = write_cache(base, &projects);
            projects
        }
        Some(cache) => {
            if cache_is_stale(base) {
                let projects = scan_base(base);
                let _ = write_cache(base, &projects);
                return projects;
            }
            // Fast path: trust cached metadata, but drop entries whose folder
            // has since disappeared. A drop rewrites the cache so the "missing"
            // state is transient.
            let mut projects = Vec::new();
            let mut dropped = false;
            for entry in cache.entries {
                // A *rejected* entry is not the same as a vanished folder. A
                // folder that has gone is an ordinary, transient state: drop the
                // row and rewrite. An entry that names a path outside its own
                // base means the file is not fastf's own bookkeeping any more,
                // and the only honest response is to stop reading it and go
                // back to the folders — which are the truth.
                let Some(project) = entry.into_project(base) else {
                    let projects = scan_base(base);
                    let _ = write_cache(base, &projects);
                    return projects;
                };
                if project.path.is_dir() {
                    projects.push(project);
                } else {
                    dropped = true;
                }
            }
            if dropped {
                let _ = write_cache(base, &projects);
            }
            projects
        }
    }
}

/// The cache is stale when the base directory's mtime is newer than the cache
/// file's (a project was added/removed since the cache was written), or when
/// either mtime can't be read (be conservative and rescan).
pub(crate) fn cache_is_stale(base: &Path) -> bool {
    let base_m = dir_mtime(base);
    let cache_m = dir_mtime(&cache_path(base));
    match (base_m, cache_m) {
        (Some(b), Some(c)) => b > c,
        _ => true,
    }
}

pub(crate) fn dir_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Re-stamp a base's cache as current, without rereading anything.
///
/// Writing `.fastf-counter.toml` into a base bumps that base's directory mtime,
/// which `cache_is_stale` reads as "a project was added or removed" — so
/// propagating the ID counter would otherwise force a full rescan of every base
/// on every create, defeating the cache entirely. The counter write provably
/// changes no project, so the honest repair is to say the cache is still good.
///
/// Only safe because fastf's own writers are serialized by `DataLock`. A change
/// made outside fastf during this instant would be masked until the next
/// `fastf reindex` — the same contract external edits already carry.
pub fn touch_cache(base: &Path) {
    let path = cache_path(base);
    if !path.exists() {
        return;
    }
    if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
        let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
    }
}

/// Read the direct children (depth `SCAN_DEPTH`) of `base` and return a
/// [`Project`] for every subdirectory that carries a `PROJECT_INFO.md`.
/// Subdirectories without one are skipped — sitting in a base is necessary but
/// not sufficient to be a project.
pub fn scan_base(base: &Path) -> Vec<Project> {
    crate::util::trace::hit("scan_base");
    debug_assert_eq!(SCAN_DEPTH, 1, "only depth-1 scanning is implemented");
    let mut out = Vec::new();
    let Ok(read_dir) = fs::read_dir(base) else {
        return out;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip dot-prefixed dirs, including `.fastf-transactions`, whose private
        // staging may carry PROJECT_INFO.md. An in-flight move must never
        // surface as a phantom duplicate project.
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        if let Some(project) = project_at(base, &path) {
            out.push(project);
        }
    }
    out
}

/// Build a [`Project`] from a folder iff it contains a readable
/// `PROJECT_INFO.md` with parseable frontmatter. Uses the fixed reserved
/// filename directly (no config lookup) — metadata is now the project identity.
pub(crate) fn project_at(base: &Path, dir: &Path) -> Option<Project> {
    let meta = read_project_meta(dir)?;
    Some(project_from_meta(meta, base, dir))
}

/// Read + parse the frontmatter of `<dir>/PROJECT_INFO.md`. `None` on any
/// failure (missing file, no frontmatter, malformed YAML).
pub(crate) fn read_project_meta(dir: &Path) -> Option<Metadata> {
    let path = dir.join(project_info::RESERVED_FILENAME);
    let body = fs::read_to_string(&path).ok()?;
    let (frontmatter, _) = project_info::split_frontmatter_body(&body)?;
    crate::util::yaml::from_str::<Metadata>(frontmatter).ok()
}

pub(crate) fn project_from_meta(meta: Metadata, base: &Path, dir: &Path) -> Project {
    let name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let created = if meta.created.trim().is_empty() {
        folder_created_fallback(dir)
    } else {
        meta.created
    };
    Project {
        id: meta.id,
        template: meta.template,
        template_name: meta.template_name,
        name,
        path: dir.to_path_buf(),
        base: base.to_path_buf(),
        created,
        tags: meta.tags,
        exists: true,
    }
}

/// Fallback creation timestamp from the folder's own mtime (some filesystems
/// have no birth time), rendered ISO-8601. Empty string if even that fails.
pub(crate) fn folder_created_fallback(dir: &Path) -> String {
    let Some(mtime) = dir_mtime(dir) else {
        return String::new();
    };
    let dt: chrono::DateTime<chrono::Utc> = mtime.into();
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
