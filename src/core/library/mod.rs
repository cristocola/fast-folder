//! Filesystem-as-truth project library.
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
//! (`/mnt/projects/...`) is valid when the same base is read on Windows (`D:\\...`).
//! There is no manual prune: the "missing" state is transient and self-heals.
//!
//! **This module is a facade.** Every path callers used before the split still
//! resolves — `library::discover`, `library::move_project`, `library::resolve` —
//! while the implementations live in focused submodules. The move engine left
//! the library entirely (`core::move_engine`): it depends on transactions,
//! staged copies and progress reporting, which nothing else here does.

mod cache;
mod discovery;
mod guard;
mod lifecycle;
mod model;
mod resolve;

pub use cache::*;
pub use discovery::*;
pub use guard::*;
pub use lifecycle::*;
pub use model::*;
pub use resolve::*;

pub(crate) use crate::core::move_engine::finish_recovered_move;
/// Debug builds only, like the failpoints it exists beside.
#[cfg(debug_assertions)]
pub use crate::core::move_engine::move_project_staged_for_test;
pub use crate::core::move_engine::{
    MoveOutcome, move_project, move_project_configured_with_outcome,
};

/// Compatibility re-export: the clock moved to [`crate::util::time`], which is
/// where a timestamp belongs. `library` was importing nothing else from
/// `project_info` or `provisioning`, and both of them imported this.
pub use crate::util::time::now_iso8601;

#[cfg(test)]
mod tests;
