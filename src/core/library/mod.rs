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
//!   - No cache, no recorded names, a names-only listing of the base that
//!     differs from the names the cache recorded, or a base dir whose mtime
//!     is newer than the cache → rescan + rewrite.
//!   - Otherwise → load the cache, dropping (and rewriting away) any entry
//!     whose folder the listing no longer holds.
//!
//! Cache entries are **base-relative** (`dir`), so a cache written on Linux
//! (`/mnt/projects/...`) is valid when the same base is read on Windows (`D:\\...`).
//! There is no manual prune: the "missing" state is transient and self-heals.
//!
//! **This module is a facade**: callers name `library::discover`,
//! `library::move_project`, `library::resolve`, and the implementations live in
//! focused submodules. The move engine is `core::move_engine`, re-exported
//! here: it depends on transactions, staged copies and progress reporting,
//! which nothing else here does.

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

/// Compatibility re-export: the clock lives in [`crate::util::time`], which is
/// where a timestamp belongs.
pub use crate::util::time::now_iso8601;

#[cfg(test)]
mod tests;
