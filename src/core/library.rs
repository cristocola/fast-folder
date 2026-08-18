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

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use crate::core::assets::{self, Progress};
use crate::core::config::Config;
use crate::core::naming;
use crate::core::project_info::{self, Metadata};
use crate::core::transactions::{self, MoveManifest, MovePhase, MoveTransaction};

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
    /// The effective base this project was discovered under (canonicalized when
    /// it came through `discover`; always the project folder's parent).
    pub base: PathBuf,
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
            base: base.to_path_buf(),
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
fn project_at(base: &Path, dir: &Path) -> Option<Project> {
    let meta = read_project_meta(dir)?;
    Some(project_from_meta(meta, base, dir))
}

/// Read + parse the frontmatter of `<dir>/PROJECT_INFO.md`. `None` on any
/// failure (missing file, no frontmatter, malformed YAML).
fn read_project_meta(dir: &Path) -> Option<Metadata> {
    let path = dir.join(project_info::RESERVED_FILENAME);
    let body = fs::read_to_string(&path).ok()?;
    let (frontmatter, _) = project_info::split_frontmatter_body(&body)?;
    serde_yaml::from_str::<Metadata>(frontmatter).ok()
}

fn project_from_meta(meta: Metadata, base: &Path, dir: &Path) -> Project {
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

/// Load a base's cache, or `None` if it is absent, unreadable, or not a version
/// this build understands.
///
/// The version check matters more than it looks. Caches are deliberately
/// co-located with the projects so they travel between machines, which means an
/// older fastf will meet caches written by a newer one. Without this, a cache
/// whose shape happened to still deserialize would be *trusted* — and every
/// project it failed to describe would silently vanish from the library.
/// Rejecting an unknown version costs one rescan and cannot hide anything.
fn load_cache(base: &Path) -> Option<Cache> {
    let raw = fs::read_to_string(cache_path(base)).ok()?;
    let cache = serde_json::from_str::<Cache>(&raw).ok()?;
    (cache.version == CACHE_VERSION).then_some(cache)
}

/// Write the cache for `base` atomically. Best-effort: a failure is returned but
/// callers ignore it — the cache is disposable and the folders remain the truth.
///
/// Uses the shared [`crate::util::atomic`] writer, whose temp name carries the
/// process id: two fastf processes refreshing the same base cache no longer
/// collide on a single fixed `.tmp` path.
fn write_cache(base: &Path, projects: &[Project]) -> Result<()> {
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
    let Some(base) = project_dir.parent() else {
        return;
    };
    let project = project_from_meta(meta, base, project_dir);
    cache_upsert(base, &project);
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
// Base display + move
// ---------------------------------------------------------------------------

/// Short display label for a base directory: its last path component (e.g.
/// `01_PROJECTS` for `/mnt/proj/01_PROJECTS`, `cristoc` for `/home/cristoc`).
/// Falls back to the full path for roots like `/`.
pub fn base_label(base: &Path) -> String {
    base.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| base.display().to_string())
}

/// Every distinct tag across `projects`, deduplicated and alphabetically
/// ordered.
///
/// Takes the projects the caller already holds rather than calling `discover`
/// itself: this feeds a per-project "which tag?" picker, and re-scanning every
/// base to populate a suggestion list would make the prompt cost more than the
/// action. The set is therefore only as fresh as the list handed in, which is
/// correct — it is a suggestion, not authority.
pub fn known_tags<'a, I>(projects: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a Project>,
{
    projects
        .into_iter()
        .flat_map(|project| project.tags.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

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

/// The compatibility-library boundary does not own a [`Config`], but it still
/// refuses stale, forged, linked, or non-child project records.
fn revalidate_recorded_project(candidate: &Project) -> Result<Project> {
    let base = candidate
        .base
        .canonicalize()
        .with_context(|| format!("resolving project base {}", candidate.base.display()))?;
    revalidate_project_in_base(candidate, &base)
}

fn revalidate_project_in_base(candidate: &Project, base: &Path) -> Result<Project> {
    assets::require_real_directory(base, "project base")?;
    assets::require_real_directory(&candidate.path, "project source")?;
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

/// The result of a move that may have published successfully while leaving the
/// old source for a later, explicitly supervised cleanup.
#[derive(Debug, Clone)]
pub struct MoveOutcome {
    pub project: Project,
    pub cleanup_pending: bool,
}

/// Move a project folder into another base directory, keeping its folder name.
/// Synchronous convenience wrapper over [`move_project_with`] with throwaway
/// progress/cancel handles — used by `fastf move` (CLI).
pub fn move_project(project: &Project, new_base: &Path) -> Result<Project> {
    let outcome = move_project_outcome(project, new_base)?;
    report_cleanup_pending(&outcome, &project.path);
    Ok(outcome.project)
}

/// Compatibility-level move outcome. Application interfaces should use
/// [`move_project_configured_with_outcome`] so configured-base validation is
/// reloaded under the mutation lock.
pub fn move_project_outcome(project: &Project, new_base: &Path) -> Result<MoveOutcome> {
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    move_project_with_outcome(project, new_base, &progress, &cancel)
}

/// Synchronous configured application wrapper with throwaway progress handles.
pub fn move_project_configured_outcome(project: &Project, new_base: &Path) -> Result<MoveOutcome> {
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    move_project_configured_with_outcome(project, new_base, &progress, &cancel)
}

/// Move a project folder into another base directory, keeping its folder name.
///
/// **Safety invariant: the source is never removed until the destination is
/// fully copied AND verified.** Same-filesystem moves take an instant, atomic
/// `fs::rename`. Cross-filesystem / network moves use a private v2 transaction
/// below the target base, verify exact path/type/size topology plus a second
/// source metadata scan, atomically publish the staging directory, and only
/// then remove the source.
///
/// `progress` drives the UI bar (phase + per-file counts); `cancel` aborts a
/// staged copy cooperatively, cleaning up the staging folder and marker and
/// leaving the source untouched. Returns the relocated [`Project`]. The caller
/// is responsible for ensuring `new_base` is a configured base so the project
/// stays discoverable.
pub fn move_project_with(
    project: &Project,
    new_base: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<Project> {
    let outcome = move_project_with_outcome(project, new_base, progress, cancel)?;
    report_cleanup_pending(&outcome, &project.path);
    Ok(outcome.project)
}

/// Move through the compatibility library API while holding the coarse data
/// lock and revalidating the recorded source base/identity.
pub fn move_project_with_outcome(
    project: &Project,
    new_base: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<MoveOutcome> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let project = revalidate_recorded_project(project)?;
    move_project_unlocked(&project, new_base, progress, cancel)
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
    staged_copy_verify_commit(
        &project,
        &target,
        &new_path,
        &folder.to_string_lossy(),
        &progress,
        &cancel,
    )
}

fn move_project_unlocked(
    project: &Project,
    new_base: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<MoveOutcome> {
    assets::require_real_directory(new_base, "target base")?;
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
    // check below deliberately applies only to the staged fallback.
    // Deliberately NOT `fs_retry::rename`: this call is *expected* to fail on a
    // cross-device move, and that failure is the signal to take the staged path.
    // Retrying would add the full backoff to every cross-drive move for nothing.
    if assets::entry_exists(&new_path)? {
        anyhow::bail!("move target already exists: {}", new_path.display());
    }
    let outcome = match fs::rename(&project.path, &new_path) {
        Ok(()) => {
            let moved = finish_move_bookkeeping(project, &old_base, &new_base, &new_path);
            MoveOutcome {
                project: moved,
                cleanup_pending: false,
            }
        }
        Err(error) if is_cross_device_error(&error) => {
            return staged_copy_verify_commit(
                project,
                &new_base,
                &new_path,
                &folder.to_string_lossy(),
                progress,
                cancel,
            );
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
    set_phase(progress, "finalizing");
    set_phase(progress, "done");
    Ok(outcome)
}

fn is_cross_device_error(error: &std::io::Error) -> bool {
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
        eprintln!(
            "warning: move published at {}, but source cleanup is pending at {}; \
             the move transaction was retained",
            outcome.project.path.display(),
            source.display()
        );
    }
}

/// The staged cross-filesystem move body. All pre-publication state lives in
/// one exclusively-created operation directory; cancellation or an ordinary
/// error before publication removes exactly that directory and leaves the
/// source untouched. Once publication begins cancellation is deliberately too
/// late.
fn staged_copy_verify_commit(
    project: &Project,
    new_base: &Path,
    new_path: &Path,
    _folder_label: &str,
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
            state.phase = "copying".to_string();
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
        set_phase(progress, "verifying");
        manifest.verify_destination(&staging)?;
        manifest.verify_source_unchanged(&project.path)?;
        crate::util::faults::check("move:after-verify")?;
        crate::util::faults::check("move:post-verification")?;

        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("move of '{}' cancelled", project.name);
        }
        set_phase(progress, "finalizing");
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
            eprintln!(
                "warning: move published at {}, but cleanup is pending ({error:#})",
                new_path.display()
            );
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
        eprintln!(
            "warning: move published at {}, but cleanup is pending ({error:#})",
            new_path.display()
        );
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = transaction.set_phase(MovePhase::CleanupPending) {
        eprintln!(
            "warning: move published at {}, but the cleanup phase could not be recorded ({error:#})",
            new_path.display()
        );
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = crate::util::faults::check("move:before-source-cleanup")
        .and_then(|()| crate::util::faults::check("move:source-cleanup"))
    {
        eprintln!(
            "warning: move published at {}, but source cleanup is pending at {} ({error:#})",
            new_path.display(),
            project.path.display()
        );
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = revalidate_recorded_project(project)
        .and_then(|_| manifest.verify_recovery_pair(&project.path, new_path))
        .and_then(|_| crate::util::fs_retry::remove_dir_all(&project.path).map_err(Into::into))
    {
        eprintln!(
            "warning: move published at {}, but source cleanup is pending at {} ({error:#})",
            new_path.display(),
            project.path.display()
        );
        cleanup_pending = true;
        retain_transaction = true;
    } else if let Err(error) = crate::util::faults::check("move:after-source-cleanup") {
        eprintln!(
            "warning: source cleanup completed, but transaction cleanup is pending ({error:#})"
        );
        retain_transaction = true;
    }

    let moved = if cleanup_pending {
        moved_view(project, new_base, new_path)
    } else {
        finish_move_bookkeeping(project, &old_base, new_base, new_path)
    };
    if !retain_transaction && let Err(error) = transaction.remove() {
        eprintln!("warning: could not clear completed move transaction: {error:#}");
    }
    set_phase(progress, "done");
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
        eprintln!("warning: could not update PROJECT_INFO.md after move: {error:#}");
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

fn set_phase(progress: &Mutex<Progress>, phase: &str) {
    if let Ok(mut p) = progress.lock() {
        p.phase = phase.to_string();
        // A phase change is real movement: without it, verifying a large tree
        // looks identical to a dead worker to both `jobs_active` and the
        // frontend's stall notice.
        p.touch();
    }
}

// ---------------------------------------------------------------------------
// Unregister / delete / rename (v1.0)
// ---------------------------------------------------------------------------

/// Unregister a project: remove its `PROJECT_INFO.md` so it stops being a
/// project. The folder and everything else inside it are untouched.
pub fn unregister_project(project: &Project) -> Result<()> {
    let project = revalidate_recorded_project(project)?;
    unregister_project_unlocked(&project)
}

/// Application entry point for unregistering. Configuration and project
/// identity are reloaded while holding the mutation lock, so a stale cache or
/// configuration change cannot authorize removal of a different metadata file.
pub fn unregister_project_configured(project: &Project) -> Result<()> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let config = Config::load()?;
    let project = revalidate_project(&config, project)?;
    unregister_project_unlocked(&project)
}

fn unregister_project_unlocked(project: &Project) -> Result<()> {
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
pub fn delete_project(project: &Project) -> Result<()> {
    let project = revalidate_recorded_project(project)?;
    delete_project_unlocked(&project)
}

/// Application entry point for deletion with configured-base and identity
/// validation performed under the mutation lock.
pub fn delete_project_configured(project: &Project) -> Result<()> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let config = Config::load()?;
    let project = revalidate_project(&config, project)?;
    delete_project_unlocked(&project)
}

fn delete_project_unlocked(project: &Project) -> Result<()> {
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
pub fn rename_project(project: &Project, new_folder: &str) -> Result<Project> {
    let project = revalidate_recorded_project(project)?;
    rename_project_unlocked(&project, new_folder)
}

/// Application entry point for rename with configured-base and identity
/// validation performed under the mutation lock.
pub fn rename_project_configured(project: &Project, new_folder: &str) -> Result<Project> {
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let config = Config::load()?;
    let project = revalidate_project(&config, project)?;
    rename_project_unlocked(&project, new_folder)
}

fn rename_project_unlocked(project: &Project, new_folder: &str) -> Result<Project> {
    let sanitized = naming::sanitize_name(new_folder.trim());
    if sanitized.is_empty() {
        anyhow::bail!("new folder name is empty");
    }
    // Discovery skips dot-prefixed dirs (staging folders) — a dot name would
    // make the project invisible.
    if sanitized.starts_with('.') {
        anyhow::bail!("folder names may not start with '.'");
    }
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
            // Put it back rather than leaving the project under a dot-prefixed
            // name, which discovery skips — that would make it vanish.
            let _ = fs::rename(&staging, &project.path);
            return Err(anyhow::anyhow!(err)
                .context(format!("renaming '{}' to '{}'", project.name, sanitized)));
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
        eprintln!("warning: could not update PROJECT_INFO.md folder/path: {err:#}");
    }

    remove_from_base_cache(project);
    cache_upsert(&base, &renamed);
    Ok(renamed)
}

/// Drop a project's entry from its base cache, best-effort (mirrors the
/// old-side bookkeeping of `move_project_with`).
fn remove_from_base_cache(project: &Project) {
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
    cfg.effective_bases()
        .iter()
        .filter(|base| base.is_dir())
        .map(|base| max_id_in_base(base))
        .max()
        .unwrap_or(0)
}

/// The highest ID held by the projects in **one** base. Read-only, same as
/// [`max_id`]. Separate because a base's own projects are what decide whether
/// its `.fastf-counter.toml` is authoritative or needs raising.
pub fn max_id_in_base(base: &Path) -> u64 {
    read_base_readonly(base)
        .iter()
        .filter_map(|project| naming::id_value(&project.id))
        .max()
        .unwrap_or(0)
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
    use crate::core::provisioning;
    use std::thread::sleep;
    use std::time::Duration;

    /// Write a project folder with a valid `PROJECT_INFO.md` frontmatter block.
    fn write_project(base: &Path, folder: &str, id: &str, template: &str, created: &str) {
        let dir = base.join(folder);
        fs::create_dir_all(&dir).unwrap();
        // Backslashes in a double-quoted YAML scalar are escape sequences —
        // a raw Windows path (`C:\Users\...`) makes the whole frontmatter
        // unparseable, so escape them.
        let path_yaml = dir.display().to_string().replace('\\', "\\\\");
        let fm = format!(
            "---\nid: {id}\ntemplate: {template}\ntemplate_name: \"{template} name\"\n\
             created: \"{created}\"\nfolder: {folder}\npath: \"{path_yaml}\"\nvariables: {{}}\ntags: []\n\
             ---\n\n# Project Info\n"
        );
        fs::write(dir.join(project_info::RESERVED_FILENAME), fm).unwrap();
    }

    fn project_with_tags(id: &str, tags: &[&str]) -> Project {
        Project {
            id: id.to_string(),
            template: "general".to_string(),
            template_name: "General".to_string(),
            name: format!("Folder_{id}"),
            path: PathBuf::from("/tmp").join(id),
            base: PathBuf::from("/tmp"),
            created: "2026-01-01T00:00:00Z".to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            exists: true,
        }
    }

    #[test]
    fn known_tags_dedupes_across_projects_and_sorts() {
        let projects = vec![
            project_with_tags("ID0001", &["draft", "client/Acme"]),
            project_with_tags("ID0002", &["draft", "urgent"]),
        ];
        assert_eq!(
            known_tags(&projects),
            vec!["client/Acme", "draft", "urgent"]
        );
    }

    #[test]
    fn known_tags_is_empty_without_tags() {
        assert!(known_tags(&[] as &[Project]).is_empty());
        let untagged = vec![project_with_tags("ID0001", &[])];
        assert!(known_tags(&untagged).is_empty());
    }

    fn cfg_for(base: &Path, extra: &[&Path]) -> Config {
        Config {
            base_dir: base.display().to_string(),
            bases: extra.iter().map(|p| p.display().to_string()).collect(),
            ..Default::default()
        }
    }

    fn v2_transaction_count(base: &Path) -> usize {
        let root = transactions::transaction_root(base);
        fs::read_dir(root)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0)
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
            base: base.to_path_buf(),
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
    fn discover_populates_base() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

        let cfg = cfg_for(base, &[]);
        let canon = base.canonicalize().unwrap();
        // Fresh scan path.
        let projects = discover(&cfg);
        assert_eq!(projects[0].base, canon);
        // Cached path (second discover reads the cache written by the first).
        let projects = discover(&cfg);
        assert_eq!(projects[0].base, canon);
    }

    /// Renaming only the capitalisation is legitimate and used to be refused:
    /// `exists()` is case-insensitive on Windows, so the target "already
    /// existed" — it was the source.
    #[test]
    fn rename_allows_case_only_change() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let dir = base.join("myproject");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            project_info::pinfo_path(&dir),
            "---\nid: ID0001\ntemplate: t\ntemplate_name: T\n\
             created: 2026-01-01T00:00:00Z\nfolder: myproject\npath: x\n\
             variables: {}\ntags: []\n---\n",
        )
        .unwrap();
        fs::write(dir.join("keep.txt"), "content").unwrap();

        let project = scan_base(base).into_iter().next().unwrap();
        let renamed = rename_project(&project, "MyProject").unwrap();

        assert_eq!(renamed.name, "MyProject");
        assert!(renamed.path.join("keep.txt").is_file(), "content survived");
        // The folder really carries the new casing on disk.
        let on_disk = fs::read_dir(base)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.eq_ignore_ascii_case("myproject"))
            .expect("project folder present");
        assert_eq!(on_disk, "MyProject");
        // No staging folder stranded — a dot-prefixed name is invisible to
        // discovery, so a leftover would make the project disappear.
        assert!(
            !fs::read_dir(base)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().contains("fastf-case")),
            "case-rename staging folder left behind"
        );
        assert_eq!(scan_base(base).len(), 1, "still exactly one project");
    }

    #[test]
    fn move_project_round_trip() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let (old_base, new_base) = (tmp1.path(), tmp2.path());
        write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        // Extra content so the copy fallback path (if hit) is exercised on a tree.
        fs::create_dir_all(old_base.join("proj_a/assets")).unwrap();
        fs::write(old_base.join("proj_a/assets/raw_{x}.txt"), "keep {braces}").unwrap();

        let cfg = cfg_for(old_base, &[new_base]);
        let projects = discover(&cfg);
        assert_eq!(projects.len(), 1);

        let moved = move_project(&projects[0], new_base).unwrap();

        let new_canon = new_base.canonicalize().unwrap();
        assert_eq!(moved.base, new_canon);
        assert_eq!(moved.path, new_canon.join("proj_a"));
        assert!(moved.path.is_dir(), "moved folder should exist");
        assert!(!old_base.join("proj_a").exists(), "source should be gone");
        // Bytes untouched.
        assert_eq!(
            fs::read_to_string(moved.path.join("assets/raw_{x}.txt")).unwrap(),
            "keep {braces}"
        );
        // Metadata `path` patched — in the readable form, not the `\\?\`
        // verbatim one that `canonicalize` hands back on Windows.
        let meta = read_project_meta(&moved.path).unwrap();
        assert_eq!(meta.path, crate::util::paths::display_path(&moved.path));
        assert!(
            !meta.path.starts_with(r"\\?\"),
            "verbatim prefix leaked into metadata: {}",
            meta.path
        );
        assert_eq!(meta.id, "ID0001");
        // Caches on both sides are fresh.
        let old_cache = load_cache(&old_base.canonicalize().unwrap()).unwrap();
        assert!(old_cache.entries.iter().all(|e| e.dir != "proj_a"));
        let new_cache = load_cache(&new_canon).unwrap();
        assert!(new_cache.entries.iter().any(|e| e.dir == "proj_a"));
        // Discovery now finds it under the new base only.
        let after = discover(&cfg);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].base, new_canon);
    }

    #[test]
    fn staged_move_copies_verifies_commits_and_removes_source() {
        // Exercises the cross-filesystem path directly (a same-fs test would take
        // the instant fs::rename fast path and never stage/verify).
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let (old_base, new_base) = (tmp1.path(), tmp2.path());
        write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::create_dir_all(old_base.join("proj_a/assets")).unwrap();
        fs::create_dir_all(old_base.join("proj_a/empty")).unwrap();
        fs::write(old_base.join("proj_a/assets/big.bin"), vec![1u8; 8000]).unwrap();
        fs::write(old_base.join("proj_a/notes_{x}.md"), "keep {braces}").unwrap();
        fs::write(old_base.join("proj_a/real.tmp"), []).unwrap();
        fs::write(old_base.join("proj_a/real.part"), [0_u8, 1, 2, 255]).unwrap();

        let cfg = cfg_for(old_base, &[new_base]);
        let project = discover(&cfg).remove(0);
        let new_path = new_base.join("proj_a");
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        staged_copy_verify_commit(&project, new_base, &new_path, "proj_a", &progress, &cancel)
            .unwrap();

        // Progress must actually advance. The phase and the per-file counter are
        // the only feedback during a multi-minute network copy, so a counter
        // that silently stops updating looks exactly like a hung move.
        {
            let p = progress.lock().unwrap();
            assert_eq!(p.phase, "done", "the phase should have advanced");
            assert!(p.total_files >= 3, "files counted: {}", p.total_files);
            assert_eq!(
                p.done_files, p.total_files,
                "every copied file must be reported done"
            );
            assert!(p.copied_bytes >= 8000, "bytes copied: {}", p.copied_bytes);
        }

        // Copied verbatim, verified, committed, source removed.
        assert_eq!(
            fs::read(new_path.join("assets/big.bin")).unwrap(),
            vec![1u8; 8000]
        );
        assert_eq!(
            fs::read_to_string(new_path.join("notes_{x}.md")).unwrap(),
            "keep {braces}"
        );
        assert_eq!(
            fs::read(new_path.join("real.tmp")).unwrap(),
            Vec::<u8>::new()
        );
        assert_eq!(
            fs::read(new_path.join("real.part")).unwrap(),
            [0_u8, 1, 2, 255]
        );
        assert!(new_path.join("empty").is_dir());
        assert!(
            !old_base.join("proj_a").exists(),
            "source removed only after verify"
        );
        assert_eq!(v2_transaction_count(new_base), 0);
    }

    #[test]
    fn cancelled_staged_move_leaves_source_intact() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let (old_base, new_base) = (tmp1.path(), tmp2.path());
        write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::write(old_base.join("proj_a/data.bin"), vec![9u8; 4096]).unwrap();

        let cfg = cfg_for(old_base, &[new_base]);
        let project = discover(&cfg).remove(0);
        let new_path = new_base.join("proj_a");
        let progress = Mutex::new(Progress::new(&[]));
        // Pre-cancelled → copy aborts on the first chunk.
        let cancel = AtomicBool::new(true);

        let err =
            staged_copy_verify_commit(&project, new_base, &new_path, "proj_a", &progress, &cancel)
                .unwrap_err()
                .to_string();
        assert!(err.contains("cancelled"), "err: {err}");
        assert!(
            old_base.join("proj_a").is_dir(),
            "source untouched on cancel"
        );
        assert!(!new_path.exists(), "no target committed");
        assert_eq!(v2_transaction_count(new_base), 0);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn cleanup_failure_is_a_reported_success_and_retains_the_marker() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        write_project(old.path(), "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::write(old.path().join("proj/payload.bin"), [0_u8, 1, 2, 255]).unwrap();
        let project = scan_base(old.path()).remove(0);
        let final_path = new.path().join("proj");
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        let outcome = crate::util::faults::with_thread_fault("move:source-cleanup", || {
            staged_copy_verify_commit(
                &project,
                new.path(),
                &final_path,
                "proj",
                &progress,
                &cancel,
            )
        })
        .expect("publication remains a successful move");

        assert!(outcome.cleanup_pending);
        assert_eq!(
            fs::read(final_path.join("payload.bin")).unwrap(),
            [0_u8, 1, 2, 255]
        );
        assert!(project.path.is_dir(), "failed cleanup leaves source intact");
        assert_eq!(
            v2_transaction_count(new.path()),
            1,
            "cleanup-pending move must retain its v2 transaction"
        );
    }

    #[test]
    fn conventional_v1_staging_and_marker_are_payload_not_move_authority() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        write_project(old.path(), "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
        let project = scan_base(old.path()).remove(0);
        let final_path = new.path().join("proj");
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        let staging = provisioning::staging_path(new.path(), "proj");
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("sentinel"), b"owned by someone else").unwrap();
        let marker = provisioning::move_marker_path(new.path(), "proj");
        fs::write(&marker, b"foreign marker bytes").unwrap();
        let outcome = staged_copy_verify_commit(
            &project,
            new.path(),
            &final_path,
            "proj",
            &progress,
            &cancel,
        )
        .unwrap();
        assert!(!outcome.cleanup_pending);
        assert_eq!(
            fs::read(staging.join("sentinel")).unwrap(),
            b"owned by someone else"
        );
        assert_eq!(fs::read(marker).unwrap(), b"foreign marker bytes");
    }

    #[test]
    fn only_the_cross_device_error_licenses_copy_fallback() {
        #[cfg(unix)]
        let cross_device = std::io::Error::from_raw_os_error(libc::EXDEV);
        #[cfg(windows)]
        let cross_device = std::io::Error::from_raw_os_error(17);

        assert!(is_cross_device_error(&cross_device));
        assert!(!is_cross_device_error(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied"
        )));
        assert!(!is_cross_device_error(&std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing"
        )));
    }

    #[test]
    fn stale_project_identity_cannot_authorize_deletion() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::write(base.join("proj/sentinel"), b"keep").unwrap();
        let stale = scan_base(base).remove(0);

        project_info::write_frontmatter(&project_info::pinfo_path(&stale.path), |metadata| {
            metadata.id = "ID9999".to_string();
        })
        .unwrap();
        let error = delete_project(&stale).unwrap_err();
        assert!(error.to_string().contains("identity changed"));
        assert_eq!(fs::read(base.join("proj/sentinel")).unwrap(), b"keep");
    }

    #[test]
    fn forged_cached_path_cannot_escape_a_configured_base() {
        let configured = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_project(
            configured.path(),
            "real",
            "ID0001",
            "gen",
            "2026-01-01T00:00:00Z",
        );
        write_project(
            outside.path(),
            "sentinel",
            "ID0001",
            "gen",
            "2026-01-01T00:00:00Z",
        );
        fs::write(outside.path().join("sentinel/keep.bin"), b"keep").unwrap();

        let mut forged = scan_base(configured.path()).remove(0);
        forged.path = outside.path().join("sentinel");
        let config = cfg_for(configured.path(), &[]);
        let error = revalidate_project(&config, &forged).unwrap_err();

        assert!(error.to_string().contains("direct child"), "got: {error}");
        assert_eq!(
            fs::read(outside.path().join("sentinel/keep.bin")).unwrap(),
            b"keep"
        );
    }

    /// Removing a project must drop its cache entry, or `recent` keeps listing
    /// something that is gone until the staleness gate happens to fire.
    #[test]
    fn delete_unregister_and_rename_all_drop_the_old_cache_entry() {
        let cached_dirs = |base: &Path| -> Vec<String> {
            load_cache(base)
                .map(|c| c.entries.into_iter().map(|e| e.dir).collect())
                .unwrap_or_default()
        };

        // delete
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "gone", "ID0001", "gen", "2026-01-01T00:00:00Z");
        write_project(base, "stays", "ID0002", "gen", "2026-01-02T00:00:00Z");
        write_cache(base, &scan_base(base)).unwrap();
        let doomed = scan_base(base)
            .into_iter()
            .find(|p| p.name == "gone")
            .unwrap();
        delete_project(&doomed).unwrap();
        let dirs = cached_dirs(base);
        assert!(!dirs.contains(&"gone".to_string()), "stale entry: {dirs:?}");
        assert!(dirs.contains(&"stays".to_string()), "collateral: {dirs:?}");

        // unregister
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "dropme", "ID0001", "gen", "2026-01-01T00:00:00Z");
        write_cache(base, &scan_base(base)).unwrap();
        let p = scan_base(base).into_iter().next().unwrap();
        unregister_project(&p).unwrap();
        assert!(
            !cached_dirs(base).contains(&"dropme".to_string()),
            "unregister must drop the cache entry"
        );
        assert!(base.join("dropme").is_dir(), "the folder itself stays");

        // rename
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "before", "ID0001", "gen", "2026-01-01T00:00:00Z");
        write_cache(base, &scan_base(base)).unwrap();
        let p = scan_base(base).into_iter().next().unwrap();
        rename_project(&p, "after").unwrap();
        let dirs = cached_dirs(base);
        assert!(!dirs.contains(&"before".to_string()), "old entry: {dirs:?}");
        assert!(dirs.contains(&"after".to_string()), "new entry: {dirs:?}");
    }

    /// `resolve` has three distinct outcomes and each must stay distinguishable:
    /// nothing matched, exactly one, or an ambiguous set.
    #[test]
    fn resolve_distinguishes_no_match_exact_and_ambiguous() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "alpha_one", "ID0001", "gen", "2026-01-01T00:00:00Z");
        write_project(base, "alpha_two", "ID0012", "gen", "2026-01-02T00:00:00Z");
        let cfg = cfg_for(base, &[]);

        // Exactly one → the project.
        assert_eq!(resolve(&cfg, "ID0001").unwrap().name, "alpha_one");
        assert_eq!(resolve(&cfg, "alpha_two").unwrap().id, "ID0012");

        // Nothing matched → a "no project matches" error, not an ambiguity one.
        let err = resolve(&cfg, "nothing_like_this").unwrap_err().to_string();
        assert!(
            err.contains("no project matches"),
            "expected a not-found error, got: {err}"
        );

        // Several matched → an ambiguity error listing the candidates.
        let err = resolve(&cfg, "alpha").unwrap_err().to_string();
        assert!(
            err.contains("ambiguous") && err.contains("ID0001") && err.contains("ID0012"),
            "expected an ambiguity error naming the candidates, got: {err}"
        );

        // An exact ID wins over a prefix that would also match it.
        assert_eq!(resolve(&cfg, "ID0001").unwrap().id, "ID0001");
    }

    /// `max_id` must be read-only — it runs from `plan()`, and a preview that
    /// writes a cache breaks the "dry run touches nothing" guarantee. It must
    /// also see projects a stale cache does not mention.
    #[test]
    fn max_id_is_read_only_and_sees_past_a_stale_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "a", "ID0007", "gen", "2026-01-01T00:00:00Z");
        let cfg = cfg_for(base, &[]);

        // No cache yet: max_id must scan, and must not create one.
        assert_eq!(max_id(&cfg), 7);
        assert!(
            !cache_path(base).exists(),
            "max_id must never write a cache — plan()/preview calls it"
        );

        // With a cache that predates a newly added project, the staleness gate
        // must send it back to the folders rather than under-reporting.
        write_cache(base, &scan_base(base)).unwrap();
        let file = fs::File::options()
            .write(true)
            .open(cache_path(base))
            .unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
            .unwrap();
        drop(file);
        write_project(base, "b", "ID0042", "gen", "2026-01-03T00:00:00Z");
        assert_eq!(
            max_id(&cfg),
            42,
            "a stale cache must not hide a project from the counter floor"
        );
    }

    /// An upsert must leave every *other* entry alone.
    ///
    /// The retain predicate drops the entry being replaced; inverted, it would
    /// drop everything else instead and quietly reduce the cache to a single
    /// project. Discovery would then self-heal on the next staleness check, so
    /// the damage is invisible until someone wonders why `recent` went blank.
    #[test]
    fn cache_upsert_replaces_one_entry_and_preserves_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        for (folder, id) in [("a", "ID0001"), ("b", "ID0002"), ("c", "ID0003")] {
            write_project(base, folder, id, "gen", "2026-01-01T00:00:00Z");
        }
        // Seed a full cache.
        let all = scan_base(base);
        assert_eq!(all.len(), 3);
        write_cache(base, &all).unwrap();

        // Re-upsert one of them with changed metadata.
        let mut updated = all.iter().find(|p| p.name == "b").unwrap().clone();
        updated.tags = vec!["urgent".to_string()];
        cache_upsert(base, &updated);

        let cache = load_cache(base).expect("cache still readable");
        assert_eq!(
            cache.entries.len(),
            3,
            "upsert must not drop the other entries, got {:?}",
            cache.entries.iter().map(|e| &e.dir).collect::<Vec<_>>()
        );
        let names: std::collections::HashSet<&str> =
            cache.entries.iter().map(|e| e.dir.as_str()).collect();
        assert!(names.contains("a") && names.contains("b") && names.contains("c"));

        // Exactly one entry for the upserted project, carrying the new data.
        let b: Vec<_> = cache.entries.iter().filter(|e| e.dir == "b").collect();
        assert_eq!(b.len(), 1, "no duplicate entry for the upserted project");
        assert_eq!(b[0].tags, vec!["urgent".to_string()]);
    }

    /// `refresh_cache` must actually re-read the metadata and write it back —
    /// silently doing nothing would leave `recent`/`search` showing stale tags
    /// after every tag mutation.
    #[test]
    fn refresh_cache_picks_up_edited_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
        write_cache(base, &scan_base(base)).unwrap();
        assert!(
            load_cache(base).unwrap().entries[0].tags.is_empty(),
            "starts untagged"
        );

        let dir = base.join("proj");
        project_info::write_frontmatter(&project_info::pinfo_path(&dir), |meta| {
            meta.tags = vec!["shipped".to_string()];
        })
        .unwrap();

        refresh_cache(&dir);

        let cache = load_cache(base).expect("cache readable");
        assert_eq!(
            cache.entries[0].tags,
            vec!["shipped".to_string()],
            "refresh_cache must write the edited metadata back"
        );
    }

    /// The staleness gate: a cache older than its base must be rescanned, and a
    /// cache newer than its base must be trusted. Getting the comparison wrong
    /// either way costs correctness or a rescan on every command.
    #[test]
    fn cache_staleness_gate_compares_the_right_way_round() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");

        write_cache(base, &scan_base(base)).unwrap();
        let cache_file = cache_path(base);

        // Set the cache's mtime explicitly rather than relying on write order:
        // writing the cache *into* the base bumps the base's own mtime to the
        // same instant, which makes "is it newer?" a coin flip.
        let set_cache_mtime = |offset_secs: i64| {
            let when = if offset_secs >= 0 {
                std::time::SystemTime::now() + std::time::Duration::from_secs(offset_secs as u64)
            } else {
                std::time::SystemTime::now()
                    - std::time::Duration::from_secs(offset_secs.unsigned_abs())
            };
            let file = fs::File::options().write(true).open(&cache_file).unwrap();
            file.set_modified(when).unwrap();
        };

        set_cache_mtime(3600); // cache clearly newer than the base
        assert!(
            !cache_is_stale(base),
            "a cache newer than its base must be trusted"
        );

        set_cache_mtime(-3600); // cache clearly older than the base
        assert!(
            cache_is_stale(base),
            "a cache older than its base must be rescanned"
        );

        // And a missing cache is always stale.
        fs::remove_file(&cache_file).unwrap();
        assert!(cache_is_stale(base));
    }

    /// Metadata with an empty `created` falls back to the folder's own mtime, so
    /// projects still sort sensibly instead of collapsing to one timestamp.
    #[test]
    fn empty_created_falls_back_to_the_folder_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let dir = base.join("proj");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            project_info::pinfo_path(&dir),
            "---\nid: ID0001\ntemplate: t\ntemplate_name: T\ncreated: \"\"\n\
             folder: proj\npath: x\nvariables: {}\ntags: []\n---\n",
        )
        .unwrap();

        let found = scan_base(base);
        assert_eq!(found.len(), 1);
        let created = &found[0].created;
        assert!(!created.is_empty(), "must fall back, not stay blank");
        assert!(
            created.starts_with("20") && created.ends_with('Z'),
            "expected an ISO-8601 UTC timestamp, got {created:?}"
        );
    }

    /// The staged (copying) move must refuse a project containing links.
    ///
    /// Reached through the private pre-flight because the public entry point
    /// only consults it after `fs::rename` fails, and a test cannot conjure a
    /// second filesystem. The same-filesystem path is covered separately in
    /// `tests/windows_semantics.rs`, where the junction is expected to survive.
    #[test]
    fn staged_move_pre_flight_refuses_links() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        // Join components separately: `join("proj_a/linked")` yields a
        // mixed-separator path on Windows (`...\proj_a/linked`), and `cmd` then
        // reads `/linked` as a switch — which is precisely how this test came to
        // "skip" silently while reporting success.
        let link = base.join("proj_a").join("linked");
        let target = base.join("shared");
        fs::create_dir_all(&target).unwrap();

        // A silent skip here would be worse than no test: it reports "ok" while
        // asserting nothing, which is exactly how the mutation run found that
        // `refuse_if_contains_links` could be replaced with `Ok(())` and stay
        // green. Junctions need no elevation on Windows and symlinks work
        // normally on Unix, so failing to create one is a real problem — say so
        // loudly rather than passing.
        #[cfg(windows)]
        {
            let out = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .output()
                .expect("running mklink");
            assert!(
                out.status.success(),
                "could not create a junction (needs no elevation on Windows):\n\
                 stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).expect("creating a symlink");

        let project = scan_base(base).into_iter().next().unwrap();
        let err = MoveManifest::scan(&project.path)
            .expect_err("a copying move cannot reproduce a link and must refuse")
            .to_string();
        assert!(
            err.contains("linked"),
            "the error must name the offending link, got: {err}"
        );
        assert!(project.path.is_dir(), "manifest scanning must be read-only");

        // A project with no links is waved through.
        write_project(base, "proj_b", "ID0002", "gen", "2026-01-02T00:00:00Z");
        let plain = scan_base(base)
            .into_iter()
            .find(|p| p.name == "proj_b")
            .unwrap();
        assert!(MoveManifest::scan(&plain.path).is_ok());
    }

    /// The move invariant, at every failpoint: the source is intact **or** the
    /// destination is complete — never neither, and never a silent half-state.
    ///
    /// The failure is injected rather than raced, so each boundary is hit
    /// deterministically instead of "wherever the kill happened to land". These
    /// go through the private staged path directly: a same-filesystem test would
    /// take the instant `fs::rename` fast path and never stage or verify.
    ///
    /// Debug-only: failpoints are compiled out of release builds.
    #[cfg(debug_assertions)]
    #[test]
    fn interrupted_staged_move_never_loses_data_at_any_failpoint() {
        const MOVE_POINTS: &[&str] = &[
            "move:before-marker-write",
            "move:after-staging",
            "move:after-verify",
            "move:before-commit-rename",
            "move:after-commit-before-source-removal",
        ];

        for point in MOVE_POINTS {
            let tmp1 = tempfile::tempdir().unwrap();
            let tmp2 = tempfile::tempdir().unwrap();
            let (old_base, new_base) = (tmp1.path(), tmp2.path());
            write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
            fs::write(old_base.join("proj_a/payload.bin"), vec![7u8; 4096]).unwrap();

            let cfg = cfg_for(old_base, &[new_base]);
            let project = discover(&cfg).remove(0);
            let new_path = new_base.join("proj_a");
            let progress = Mutex::new(Progress::new(&[]));
            let cancel = AtomicBool::new(false);

            // Armed per-thread, so a move test running in parallel cannot see
            // this fault — an env var would fire inside every one of them.
            let result = crate::util::faults::with_thread_fault(point, || {
                staged_copy_verify_commit(
                    &project, new_base, &new_path, "proj_a", &progress, &cancel,
                )
            });

            if *point == "move:after-commit-before-source-removal" {
                assert!(
                    result.as_ref().is_ok_and(|outcome| outcome.cleanup_pending),
                    "[{point}] publication must be reported as cleanup pending"
                );
            } else {
                assert!(result.is_err(), "[{point}] should have failed");
            }

            // The invariant. `after-commit-before-source-removal` is the one
            // point where the commit already landed, so the destination holds
            // the data and the (still-present) source is redundant.
            let source_ok = old_base.join("proj_a/payload.bin").is_file();
            let dest_ok = new_path.join("payload.bin").is_file();
            assert!(
                source_ok || dest_ok,
                "[{point}] data exists in neither location — this is data loss"
            );

            if *point == "move:after-commit-before-source-removal" {
                assert!(dest_ok, "[{point}] commit landed, destination must hold it");
            } else {
                assert!(
                    source_ok,
                    "[{point}] nothing was committed, so the source must be intact"
                );
                assert!(
                    !new_path.exists(),
                    "[{point}] an uncommitted move must leave no destination"
                );
            }

            // Whatever happened, reconcile must reach a consistent end state
            // with the payload still present exactly once.
            let report = provisioning::reconcile(&cfg);
            let after_source = old_base.join("proj_a/payload.bin").is_file();
            let after_dest = new_path.join("payload.bin").is_file();
            assert!(
                after_source || after_dest,
                "[{point}] reconcile lost the data"
            );
            assert_eq!(
                v2_transaction_count(new_base),
                0,
                "[{point}] reconcile left a transaction behind: {report:?}"
            );
        }
    }

    #[test]
    fn move_project_rejects_same_base_and_collision() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        write_project(
            tmp1.path(),
            "proj_a",
            "ID0001",
            "gen",
            "2026-01-01T00:00:00Z",
        );
        let cfg = cfg_for(tmp1.path(), &[tmp2.path()]);
        let project = discover(&cfg).remove(0);

        // Same base → bail.
        let err = move_project(&project, tmp1.path()).unwrap_err().to_string();
        assert!(err.contains("already in base"), "err: {err}");

        // Target name collision → bail, source untouched.
        fs::create_dir_all(tmp2.path().join("proj_a")).unwrap();
        let err = move_project(&project, tmp2.path()).unwrap_err().to_string();
        assert!(err.contains("already exists"), "err: {err}");
        assert!(project.path.is_dir(), "source must be untouched on bail");
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

    #[test]
    fn rename_sanitizes_and_rejects_bad_names() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        let cfg = cfg_for(base, &[]);
        let project = discover(&cfg).remove(0);

        // Illegal filesystem chars are sanitized, not fatal.
        let renamed = rename_project(&project, "New: Name?").unwrap();
        assert_eq!(renamed.name, "New_ Name_");
        assert!(renamed.path.is_dir());
        assert!(!project.path.exists());

        // Dot-prefixed names would be invisible to discovery → rejected.
        let err = rename_project(&renamed, ".hidden").unwrap_err().to_string();
        assert!(err.contains("may not start with '.'"), "err: {err}");
        // Same-name rename is a no-op error, not a silent success.
        let err = rename_project(&renamed, "New_ Name_")
            .unwrap_err()
            .to_string();
        assert!(err.contains("already the folder's name"), "err: {err}");
        assert!(renamed.path.is_dir(), "folder intact after failed renames");
    }

    #[test]
    fn unregister_and_delete_guard_rails() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::write(base.join("proj_a").join("keep.txt"), "data").unwrap();
        let cfg = cfg_for(base, &[]);
        let project = discover(&cfg).remove(0);

        // Unregister removes only the metadata file.
        unregister_project(&project).unwrap();
        assert!(project.path.join("keep.txt").is_file());
        assert!(!project_info::pinfo_path(&project.path).exists());
        // Double-unregister is a clean error.
        assert!(unregister_project(&project).is_err());

        // Delete refuses a folder without PROJECT_INFO.md (the guard rail).
        let err = delete_project(&project).unwrap_err().to_string();
        assert!(err.contains("no PROJECT_INFO.md"), "err: {err}");
        assert!(project.path.is_dir());

        // Re-register (rewrite metadata) → delete removes the whole folder.
        write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        delete_project(&project).unwrap();
        assert!(!project.path.exists());
    }
}
