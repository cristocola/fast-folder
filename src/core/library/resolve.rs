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

/// What a query resolved to, as data rather than as a `Result`.
///
/// [`resolve`] collapses this into a `Result<Project>` for the callers that
/// only ever want one project. The surfaces that can *offer* a choice — the
/// verbs that may open an ambiguity picker — match on this instead, because an
/// error string cannot be shown in a picker.
#[derive(Debug)]
pub enum Resolution {
    /// The library is empty — no base holds a project at all.
    NoProjects,
    /// Projects exist; none matched the query.
    NoMatch,
    /// Exactly one match. **Boxed**: `Project` is large, and the Windows clippy
    /// leg fires `large_enum_variant` on the unboxed form where Linux does not.
    One(Box<Project>),
    /// Several matches, in discovery order (newest first). The full list, not
    /// the truncated one the ambiguity *message* shows.
    Many(Vec<Project>),
}

/// Resolve a query against the library, reporting the candidates as data.
///
/// Tiers, first non-empty wins: exact ID → **numeric** → ID prefix →
/// case-insensitive name substring.
///
/// The numeric tier is what makes `fastf open 37` find `ID0037`: an all-digits
/// query is read as an ID *number* and compared with [`naming::id_value`], so
/// it is prefix-agnostic and immune to padding width. It sits *below* the exact
/// tier because a template may declare a digits-only ID prefix, which makes an
/// all-digits string a legal complete ID; it sits *above* the prefix tier
/// because otherwise `4` matches everything from ID0040 to ID0049.
pub fn resolve_matches(cfg: &Config, query: &str) -> Resolution {
    let projects = discover(cfg);
    if projects.is_empty() {
        return Resolution::NoProjects;
    }

    // 1. Exact ID.
    let mut matches: Vec<&Project> = projects.iter().filter(|p| p.id == query).collect();
    // 2. ID number.
    if matches.is_empty()
        && let Some(n) = numeric_query(query)
    {
        matches = projects
            .iter()
            .filter(|p| naming::id_value(&p.id) == Some(n))
            .collect();
    }
    // 3. ID prefix.
    if matches.is_empty() {
        matches = projects
            .iter()
            .filter(|p| p.id.starts_with(query))
            .collect();
    }
    // 4. Name substring (case-insensitive).
    if matches.is_empty() {
        let q = query.to_lowercase();
        matches = projects
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&q))
            .collect();
    }

    match matches.len() {
        0 => Resolution::NoMatch,
        1 => Resolution::One(Box::new(matches[0].clone())),
        _ => Resolution::Many(matches.into_iter().cloned().collect()),
    }
}

/// An all-ASCII-digits query as a number, or `None` when it is not one — which
/// includes a digit run too long for `u64`, so an absurd query falls through to
/// the ordinary tiers instead of matching nothing forever.
fn numeric_query(query: &str) -> Option<u64> {
    if query.is_empty() || !query.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    query.parse::<u64>().ok()
}

/// Resolve a project by query, or fail with the message that names the reason.
///
/// A thin wrapper over [`resolve_matches`] and the three error builders below —
/// which is where the messages live, exactly once each, so a picker-driven
/// caller and a piped one report identical text.
pub fn resolve(cfg: &Config, query: &str) -> Result<Project> {
    match resolve_matches(cfg, query) {
        Resolution::NoProjects => Err(no_projects_error()),
        Resolution::NoMatch => Err(no_match_error(query)),
        Resolution::One(project) => Ok(*project),
        Resolution::Many(candidates) => Err(ambiguous_error(query, &candidates)),
    }
}

/// The library is empty.
pub(crate) fn no_projects_error() -> anyhow::Error {
    anyhow::anyhow!("no projects found — create one with `fastf new` first")
}

/// Projects exist, but none matched.
pub(crate) fn no_match_error(query: &str) -> anyhow::Error {
    anyhow::anyhow!("no project matches '{}' — try `fastf recent`", query)
}

/// Several matched, and the caller could not (or would not) ask which.
///
/// The listing is capped at ten in the *text* only; `Resolution::Many` carries
/// the whole set, because a picker can scroll and an error message cannot.
pub(crate) fn ambiguous_error(query: &str, candidates: &[Project]) -> anyhow::Error {
    // **When every candidate is the same project, the full ID is not the
    // answer** — it is what was typed. `fastf copy-to` puts a project on a
    // backup drive keeping its id, and adding that drive as a base lists both;
    // telling the reader to be more specific about an id that is already exact
    // sends them looking for a distinction that is not there. The base is.
    let one_id = candidates
        .first()
        .is_some_and(|first| candidates.iter().all(|p| p.id == first.id));
    let mut msg = if one_id {
        format!(
            "'{}' is in {} bases — name the base, or open it from `fastf recent`:\n",
            query,
            candidates.len()
        )
    } else {
        format!(
            "'{}' is ambiguous — {} matches. Specify a full ID:\n",
            query,
            candidates.len()
        )
    };
    for p in candidates.iter().take(10) {
        if one_id {
            msg.push_str(&format!(
                "  {}  {}  in {}\n",
                p.id,
                p.name,
                super::base_label(&p.base)
            ));
        } else {
            msg.push_str(&format!("  {}  {}  ({})\n", p.id, p.name, p.template));
        }
    }
    anyhow::anyhow!("{}", msg.trim_end())
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
            .filter_map(|entry| entry.into_project(base))
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
