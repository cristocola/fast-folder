//! What a project *is*, in memory, plus the constants that describe the layout.

use std::path::Path;
use std::path::PathBuf;

/// Filename of the per-base disposable cache, co-located with the projects.
pub const CACHE_FILENAME: &str = ".fastf-index.json";

/// Scan depth beneath each base directory. `1` = direct children only, which
/// matches the user's flat project layouts. Kept as a constant so it could
/// become configurable later without hunting for magic numbers.
pub(crate) const SCAN_DEPTH: usize = 1;

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

/// Short display label for a base directory: its last path component (e.g.
/// `01_PROJECTS` for `/mnt/projects/01_PROJECTS`, `alice` for `/home/alice`).
/// Falls back to the full path for roots like `/`.
pub fn base_label(base: &Path) -> String {
    base.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| base.display().to_string())
}
