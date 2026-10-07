//! What a project *is*, in memory, plus the constants that describe the layout.

use std::path::Path;
use std::path::PathBuf;

/// Filename of the per-base disposable cache, co-located with the projects.
pub const CACHE_FILENAME: &str = ".fastf-index.json";

/// Scan depth beneath each base directory. `1` = direct children only, which
/// matches the user's flat project layouts. Only `1` is implemented: the scan
/// asserts it, and a cache entry's `dir` is valid only as one component.
pub(crate) const SCAN_DEPTH: usize = 1;

/// A discovered project — the in-memory view built either from a freshly-read
/// `PROJECT_INFO.md` or from a cache entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Authoritative ID from the `PROJECT_INFO.md` frontmatter.
    pub id: String,
    /// The number behind that id, when the project records one. Read through
    /// [`Project::number`], never directly — see it for why.
    pub id_number: Option<u64>,
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
    /// The one line that says what the project is; empty when none was written.
    /// Read from the index like `tags`, so a list never opens a file for it,
    /// and a hand edit shows once the project is opened or reindexed.
    pub description: String,
    pub tags: Vec<String>,
    /// `true` for freshly-scanned projects; cache entries are checked against
    /// the base's listing and only surface when their folder is in it, so this
    /// is effectively always `true` for returned projects (the field exists so
    /// future callers can render a transient "missing" state without a
    /// signature change).
    pub exists: bool,
}

impl Project {
    /// **The one way to ask what number a project is.**
    ///
    /// Prefers the number recorded in `PROJECT_INFO.md`, and falls back to
    /// reading the trailing digits of the id string for every project written
    /// before that field existed.
    ///
    /// The fallback is a guess, and for one shape of template a bad one:
    /// `Counters::format_id` is lossy, so a digits-only `id.prefix` renders
    /// project 1 as `2001` and the parse reads two thousand and one back.
    /// The guess feeds the counter's self-heal floor, and the counter never
    /// descends — so a single create would renumber a whole library.
    /// `fastf reindex` backfills the recorded number into older files.
    pub fn number(&self) -> Option<u64> {
        self.id_number
            .or_else(|| crate::core::naming::id_value(&self.id))
    }
}

/// Short display label for a base directory: its last path component (e.g.
/// `01_PROJECTS` for `/mnt/projects/01_PROJECTS`, `alice` for `/home/alice`).
/// Falls back to the full path for roots like `/` — as it reads, so a drive
/// root is `S:\`, not the canonical `\\?\S:\`.
pub fn base_label(base: &Path) -> String {
    base.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| crate::util::paths::display_path(base))
}

/// When the project was last written: the newer of its `PROJECT_INFO.md`'s
/// and its folder's modification time, as the fixed-width UTC stamp
/// `created` uses, so the two sort as text beside each other.
///
/// **Computed, never stored.** Every write fastf makes to the file — a note,
/// a tick, a tag, a move's bookkeeping — moves it, and so does a hand edit;
/// the folder's time moves when something lands in or leaves its top level.
/// An edit deeper in the folder is invisible here, which is why a note is
/// still how work is said to have happened. Two stats, nothing read; `None`
/// where neither answers.
pub fn touched(project: &Project) -> Option<String> {
    let info = std::fs::metadata(crate::core::project_info::pinfo_path(&project.path))
        .and_then(|meta| meta.modified())
        .ok();
    let dir = std::fs::metadata(&project.path)
        .and_then(|meta| meta.modified())
        .ok();
    touched_from(info, dir)
}

/// [`touched`] over times already read: the pane stats both for its own
/// freshness check and need not ask again.
pub fn touched_from(
    info: Option<std::time::SystemTime>,
    dir: Option<std::time::SystemTime>,
) -> Option<String> {
    let newest = match (info, dir) {
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    Some(crate::util::time::iso8601_of(newest))
}
