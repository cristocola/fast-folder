//! Filesystem-as-truth project library (v0.9).
//!
//! The source of truth for "what projects exist" is the filesystem: a folder is
//! a project **iff** it contains a `PROJECT_INFO.md` with YAML frontmatter. The
//! `id` in that frontmatter is the authoritative ID; the folder name is cosmetic
//! and never consulted for discovery.
//!
//! To keep fastf's startup fast, each base directory carries a disposable cache
//! (`.fastf-index.json`) co-located with its projects, so it travels with them
//! across machines. The cache is **never** an authority — it is always
//! reconcilable from the folders:
//!   - No cache, or the base dir's mtime is newer than the cache → rescan +
//!     rewrite.
//!   - Otherwise → load the cache and cheaply existence-check each entry,
//!     dropping (and rewriting away) any whose folder has since disappeared.
//!
//! Cache entries are **base-relative** (`dir`), so a cache written on Linux
//! (`/mnt/proj/...`) is valid when the same base is read on Windows (`D:\...`).
//! There is no manual prune: the "missing" state is transient and self-heals.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::config::Config;
use crate::core::naming;
use crate::core::project_info::{self, Metadata};

/// Filename of the per-base disposable cache, co-located with the projects.
pub const CACHE_FILENAME: &str = ".fastf-index.json";

/// Scan depth beneath each base directory. `1` = direct children only, which
/// matches the user's flat project layouts. Kept as a constant so it could
/// become configurable later without hunting for magic numbers.
const SCAN_DEPTH: usize = 1;

/// A discovered project — the in-memory view built either from a freshly-read
/// `PROJECT_INFO.md` or from a cache entry.
#[derive(Debug, Clone)]
pub struct Project {
    /// Authoritative ID from the `PROJECT_INFO.md` frontmatter.
    pub id: String,
    pub template: String,
    pub template_name: String,
    /// Folder basename (cosmetic).
    pub name: String,
    /// Absolute path (`base.join(dir)`).
    pub path: PathBuf,
    /// ISO-8601 creation timestamp from metadata (folder mtime as a fallback).
    pub created: String,
    pub tags: Vec<String>,
    /// `true` for freshly-scanned projects; cache entries are stat-checked and
    /// only surface when their folder still exists, so this is effectively
    /// always `true` for returned projects (the field exists so future callers
    /// can render a transient "missing" state without a signature change).
    pub exists: bool,
}

// ---------------------------------------------------------------------------
// Cache model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    /// Base-relative directory (portable across OSes / drive letters).
    dir: String,
    id: String,
    template: String,
    template_name: String,
    name: String,
    created: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    #[serde(default)]
    entries: Vec<CacheEntry>,
}

const CACHE_VERSION: u32 = 1;

impl CacheEntry {
    fn from_project(project: &Project, base: &Path) -> Self {
        // `dir` is base-relative; fall back to the basename if strip fails
        // (shouldn't happen — projects are always built as `base.join(...)`).
        let dir = project
            .path
            .strip_prefix(base)
            .map(to_forward_slashes)
            .unwrap_or_else(|_| project.name.clone());
        Self {
            dir,
            id: project.id.clone(),
            template: project.template.clone(),
            template_name: project.template_name.clone(),
            name: project.name.clone(),
            created: project.created.clone(),
            tags: project.tags.clone(),
        }
    }

    fn into_project(self, base: &Path) -> Project {
        let path = base.join(self.dir.replace('/', std::path::MAIN_SEPARATOR_STR));
        Project {
            id: self.id,
            template: self.template,
            template_name: self.template_name,
            name: self.name,
            path,
            created: self.created,
            tags: self.tags,
            exists: true,
        }
    }
}

fn to_forward_slashes(p: &Path) -> String {
    p.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Discover every project across all effective bases, newest first.
///
/// Per base: cache-first with a staleness gate (see module docs). Absent /
/// unmounted bases are skipped honestly rather than surfacing stale entries.
pub fn discover(cfg: &Config) -> Vec<Project> {
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
fn discover_base(base: &Path) -> Vec<Project> {
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
                let project = entry.into_project(base);
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
fn cache_is_stale(base: &Path) -> bool {
    let base_m = dir_mtime(base);
    let cache_m = dir_mtime(&cache_path(base));
    match (base_m, cache_m) {
        (Some(b), Some(c)) => b > c,
        _ => true,
    }
}

fn dir_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Read the direct children (depth `SCAN_DEPTH`) of `base` and return a
/// [`Project`] for every subdirectory that carries a `PROJECT_INFO.md`.
/// Subdirectories without one are skipped — sitting in a base is necessary but
/// not sufficient to be a project.
pub fn scan_base(base: &Path) -> Vec<Project> {
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
        if let Some(project) = project_at(&path) {
            out.push(project);
        }
    }
    out
}

/// Build a [`Project`] from a folder iff it contains a readable
/// `PROJECT_INFO.md` with parseable frontmatter. Uses the fixed reserved
/// filename directly (no config lookup) — metadata is now the project identity.
fn project_at(dir: &Path) -> Option<Project> {
    let meta = read_project_meta(dir)?;
    Some(project_from_meta(meta, dir))
}

/// Read + parse the frontmatter of `<dir>/PROJECT_INFO.md`. `None` on any
/// failure (missing file, no frontmatter, malformed YAML).
fn read_project_meta(dir: &Path) -> Option<Metadata> {
    let path = dir.join(project_info::RESERVED_FILENAME);
    let body = fs::read_to_string(&path).ok()?;
    let (frontmatter, _) = project_info::split_frontmatter_body(&body)?;
    serde_yaml::from_str::<Metadata>(frontmatter).ok()
}

fn project_from_meta(meta: Metadata, dir: &Path) -> Project {
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
        created,
        tags: meta.tags,
        exists: true,
    }
}

/// Fallback creation timestamp from the folder's own mtime (some filesystems
/// have no birth time), rendered ISO-8601. Empty string if even that fails.
fn folder_created_fallback(dir: &Path) -> String {
    let Some(mtime) = dir_mtime(dir) else {
        return String::new();
    };
    let dt: chrono::DateTime<chrono::Utc> = mtime.into();
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// Cache I/O
// ---------------------------------------------------------------------------

fn cache_path(base: &Path) -> PathBuf {
    base.join(CACHE_FILENAME)
}

fn load_cache(base: &Path) -> Option<Cache> {
    let raw = fs::read_to_string(cache_path(base)).ok()?;
    serde_json::from_str::<Cache>(&raw).ok()
}

/// Write the cache for `base` atomically (`.tmp` + rename). Best-effort: a
/// failure is returned but callers ignore it — the cache is disposable and the
/// folders remain the truth.
fn write_cache(base: &Path, projects: &[Project]) -> Result<()> {
    let cache = Cache {
        version: CACHE_VERSION,
        entries: projects
            .iter()
            .map(|p| CacheEntry::from_project(p, base))
            .collect(),
    };
    let raw = serde_json::to_string_pretty(&cache)?;
    let final_path = cache_path(base);
    let tmp_path = final_path.with_extension("json.tmp");
    fs::write(&tmp_path, raw)?;
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Cache mutation helpers (used by writers so list/search stay fresh without a
// full rescan). All best-effort — a cache error never fails the command.
// ---------------------------------------------------------------------------

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
    let new_dir = CacheEntry::from_project(project, base).dir;
    projects.retain(|p| CacheEntry::from_project(p, base).dir != new_dir);
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
    let project = project_from_meta(meta, project_dir);
    if let Some(base) = project_dir.parent() {
        cache_upsert(base, &project);
    }
}

/// Remove the entry for base-relative `dir` from `base`'s cache. No-op when the
/// cache is missing (the entry is already absent, definitionally).
pub fn cache_remove(base: &Path, dir: &str) {
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

// ---------------------------------------------------------------------------
// Resolution + counter self-heal
// ---------------------------------------------------------------------------

/// Resolve a project by query: exact ID → ID prefix → case-insensitive name
/// substring. Ambiguous queries error with the candidate list.
pub fn resolve(cfg: &Config, query: &str) -> Result<Project> {
    let projects = discover(cfg);
    if projects.is_empty() {
        anyhow::bail!("no projects found — create one with `fastf new` first");
    }

    // 1. Exact ID.
    let mut matches: Vec<&Project> = projects.iter().filter(|p| p.id == query).collect();
    // 2. ID prefix.
    if matches.is_empty() {
        matches = projects
            .iter()
            .filter(|p| p.id.starts_with(query))
            .collect();
    }
    // 3. Name substring (case-insensitive).
    if matches.is_empty() {
        let q = query.to_lowercase();
        matches = projects
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&q))
            .collect();
    }

    match matches.len() {
        0 => anyhow::bail!("no project matches '{}' — try `fastf recent`", query),
        1 => Ok(matches[0].clone()),
        _ => {
            let mut msg = format!(
                "'{}' is ambiguous — {} matches. Specify a full ID:\n",
                query,
                matches.len()
            );
            for p in matches.iter().take(10) {
                msg.push_str(&format!("  {}  {}  ({})\n", p.id, p.name, p.template));
            }
            anyhow::bail!("{}", msg.trim_end())
        }
    }
}

/// Highest numeric ID across all bases — the floor the global counter
/// self-heals up to, so deleting `target/release/` can never drop below reality.
/// Prefix-agnostic (reads each project's trailing digit run).
///
/// **Read-only**: unlike [`discover`], this never writes a cache, so it is safe
/// to call from `plan()` / preview (which must not touch disk). It reads a fresh
/// cache when present, else scans the base directly.
pub fn max_id(cfg: &Config) -> u64 {
    let mut max = 0u64;
    for base in cfg.effective_bases() {
        if !base.is_dir() {
            continue;
        }
        for project in read_base_readonly(&base) {
            if let Some(value) = naming::id_value(&project.id) {
                max = max.max(value);
            }
        }
    }
    max
}

/// Read a base's projects **without** writing the cache: use a fresh cache if
/// one is present, else scan the directory. Never mutates disk — safe for the
/// preview/plan path.
fn read_base_readonly(base: &Path) -> Vec<Project> {
    match load_cache(base) {
        Some(cache) if !cache_is_stale(base) => cache
            .entries
            .into_iter()
            .map(|entry| entry.into_project(base))
            .collect(),
        _ => scan_base(base),
    }
}

/// Force a full rescan of every base and rewrite each `.fastf-index.json`,
/// ignoring the staleness gate. The cheap recovery path for external edits
/// (files added/moved outside fastf). Returns the total project count.
pub fn reindex(cfg: &Config) -> usize {
    let mut total = 0;
    for base in cfg.effective_bases() {
        if !base.is_dir() {
            continue;
        }
        let projects = scan_base(&base);
        total += projects.len();
        let _ = write_cache(&base, &projects);
    }
    total
}

/// Current UTC timestamp, ISO-8601 (seconds precision). Lives here in the
/// library layer (the successor to the deleted `index` module).
pub fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    /// Write a project folder with a valid `PROJECT_INFO.md` frontmatter block.
    fn write_project(base: &Path, folder: &str, id: &str, template: &str, created: &str) {
        let dir = base.join(folder);
        fs::create_dir_all(&dir).unwrap();
        let fm = format!(
            "---\nid: {id}\ntemplate: {template}\ntemplate_name: \"{template} name\"\n\
             created: \"{created}\"\nfolder: {folder}\npath: \"{}\"\nvariables: {{}}\ntags: []\n\
             ---\n\n# Project Info\n",
            dir.display()
        );
        fs::write(dir.join(project_info::RESERVED_FILENAME), fm).unwrap();
    }

    fn cfg_for(base: &Path, extra: &[&Path]) -> Config {
        Config {
            base_dir: base.display().to_string(),
            bases: extra.iter().map(|p| p.display().to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn scan_finds_only_project_info_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        // A folder without PROJECT_INFO.md is not a project.
        fs::create_dir_all(base.join("not_a_project/sub")).unwrap();
        // A loose file is ignored.
        fs::write(base.join("loose.txt"), "hi").unwrap();

        let projects = scan_base(base);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "ID0001");
        assert_eq!(projects[0].name, "proj_a");
    }

    #[test]
    fn cache_round_trips_base_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

        let projects = scan_base(base);
        write_cache(base, &projects).unwrap();

        // The on-disk cache stores a base-relative `dir`, never an absolute path.
        let raw = fs::read_to_string(cache_path(base)).unwrap();
        assert!(raw.contains("\"dir\": \"proj_a\""), "raw cache: {raw}");
        assert!(
            !raw.contains(&base.display().to_string()),
            "cache must not contain absolute base path"
        );

        // Loading reconstructs the absolute path via base.join(dir).
        let cache = load_cache(base).unwrap();
        assert_eq!(cache.entries.len(), 1);
        let reconstructed = cache.entries[0].clone().into_project(base);
        assert_eq!(reconstructed.path, base.join("proj_a"));
    }

    #[test]
    fn staleness_triggers_rescan_on_base_mtime_bump() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

        let cfg = cfg_for(base, &[]);
        let first = discover(&cfg);
        assert_eq!(first.len(), 1);

        // Add a second project after the cache was written; creating a new
        // subdir bumps the base dir's mtime past the cache's.
        sleep(Duration::from_millis(20));
        write_project(base, "proj_b", "ID0002", "gen", "2026-02-01T00:00:00Z");

        let second = discover(&cfg);
        assert_eq!(second.len(), 2, "stale cache should have rescanned");
        let cache = load_cache(base).unwrap();
        assert_eq!(cache.entries.len(), 2, "cache should have been rewritten");
    }

    #[test]
    fn existence_check_drops_missing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

        // Seed a cache that includes a phantom entry for a folder that never
        // existed. Building projects directly lets us plant the phantom.
        let real = scan_base(base);
        let phantom = Project {
            id: "ID0099".to_string(),
            template: "gen".to_string(),
            template_name: "gen name".to_string(),
            name: "proj_ghost".to_string(),
            path: base.join("proj_ghost"),
            created: "2026-03-01T00:00:00Z".to_string(),
            tags: vec![],
            exists: true,
        };
        let mut planted = real.clone();
        planted.push(phantom);
        write_cache(base, &planted).unwrap();

        // Re-touch the cache in place (no dir-entry change) so cache mtime is
        // strictly newer than the base mtime → the fast (non-stale) path runs,
        // exercising the existence-check drop rather than a full rescan.
        sleep(Duration::from_millis(20));
        let raw = fs::read_to_string(cache_path(base)).unwrap();
        fs::write(cache_path(base), raw).unwrap();
        assert!(!cache_is_stale(base), "cache should read as fresh");

        let cfg = cfg_for(base, &[]);
        let projects = discover(&cfg);
        assert_eq!(projects.len(), 1, "phantom entry should be dropped");
        assert_eq!(projects[0].id, "ID0001");
        // The drop is persisted.
        let cache = load_cache(base).unwrap();
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn multi_base_union_sorted_newest_first() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        write_project(
            tmp1.path(),
            "proj_old",
            "ID0010",
            "gen",
            "2026-01-01T00:00:00Z",
        );
        write_project(
            tmp2.path(),
            "proj_new",
            "ID0020",
            "gen",
            "2026-06-01T00:00:00Z",
        );

        let cfg = cfg_for(tmp1.path(), &[tmp2.path()]);
        let projects = discover(&cfg);
        assert_eq!(projects.len(), 2);
        // Newest first.
        assert_eq!(projects[0].id, "ID0020");
        assert_eq!(projects[1].id, "ID0010");
    }

    #[test]
    fn max_id_across_bases() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        // Inconsistent padding on purpose — value is what matters.
        write_project(
            tmp1.path(),
            "a_ID007",
            "ID007",
            "gen",
            "2026-01-01T00:00:00Z",
        );
        write_project(
            tmp2.path(),
            "b_ID0030",
            "ID0030",
            "gen",
            "2026-02-01T00:00:00Z",
        );

        let cfg = cfg_for(tmp1.path(), &[tmp2.path()]);
        assert_eq!(max_id(&cfg), 30);
    }

    #[test]
    fn max_id_empty_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path(), &[]);
        assert_eq!(max_id(&cfg), 0);
    }

    #[test]
    fn resolve_by_id_prefix_and_name() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(
            base,
            "music_video_alpha",
            "ID0042",
            "mv",
            "2026-01-01T00:00:00Z",
        );
        write_project(
            base,
            "research_beta",
            "ID0100",
            "rn",
            "2026-02-01T00:00:00Z",
        );
        let cfg = cfg_for(base, &[]);

        // Exact id.
        assert_eq!(resolve(&cfg, "ID0042").unwrap().name, "music_video_alpha");
        // Id prefix (unique).
        assert_eq!(resolve(&cfg, "ID004").unwrap().id, "ID0042");
        // Name substring (case-insensitive).
        assert_eq!(resolve(&cfg, "BETA").unwrap().id, "ID0100");
        // No match.
        assert!(resolve(&cfg, "nope").is_err());
    }

    #[test]
    fn resolve_ambiguous_errors_with_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "shared_one", "ID0011", "gen", "2026-01-01T00:00:00Z");
        write_project(base, "shared_two", "ID0012", "gen", "2026-02-01T00:00:00Z");
        let cfg = cfg_for(base, &[]);

        // "shared" matches both by name substring.
        let err = resolve(&cfg, "shared").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "err: {err}");
        assert!(err.contains("ID0011") && err.contains("ID0012"));
    }
}
