//! Turning a query into a project, and reading the library's highest ID.

use anyhow::Result;
use std::path::Path;

use crate::core::config::Config;
use crate::core::naming;

use super::cache::*;
use super::discovery::*;
use super::model::*;

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
pub(crate) fn max_id_in_base(base: &Path) -> u64 {
    read_base_readonly(base)
        .iter()
        .filter_map(|project| naming::id_value(&project.id))
        .max()
        .unwrap_or(0)
}

/// Read a base's projects **without** writing the cache: use a fresh cache if
/// one is present, else scan the directory. Never mutates disk — safe for the
/// preview/plan path.
pub(crate) fn read_base_readonly(base: &Path) -> Vec<Project> {
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
