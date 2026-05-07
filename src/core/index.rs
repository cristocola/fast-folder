use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::util::paths;

/// One line per project in `projects.jsonl` — append-only log of everything
/// ever created by `fastf new`. Used by `fastf recent` and `fastf open`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectRecord {
    pub id: String,
    pub template: String,
    pub path: String,
    pub name: String,
    pub created_at: String,
}

/// Append a record to `projects.jsonl` (creates the file if missing).
/// Failures are logged to stderr but do not fail the caller — the project
/// was successfully created on disk; the index is a convenience.
pub fn append(record: &ProjectRecord) {
    if let Err(e) = try_append(record) {
        eprintln!(
            "warning: failed to update project index ({}): {}",
            paths::projects_index_path().display(),
            e
        );
    }
}

fn try_append(record: &ProjectRecord) -> Result<()> {
    let line = serde_json::to_string(record).context("serializing record")?;
    let path = paths::projects_index_path();
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(f, "{}", line).with_context(|| format!("writing to {}", path.display()))?;
    Ok(())
}

/// Read all records from `projects.jsonl`. Malformed lines are skipped with a warning.
pub fn load_all() -> Result<Vec<ProjectRecord>> {
    let path = paths::projects_index_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let f = fs::File::open(&path).with_context(|| format!("opening {}", path.display()))?;
    let reader = BufReader::new(f);
    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("reading {} line {}", path.display(), idx + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ProjectRecord>(&line) {
            Ok(r) => out.push(r),
            Err(e) => eprintln!("warning: skipping malformed index line {}: {}", idx + 1, e),
        }
    }
    Ok(out)
}

/// Atomically rewrite the index with the given records.
/// Used by `fastf recent --prune`.
pub fn rewrite(records: &[ProjectRecord]) -> Result<()> {
    let final_path = paths::projects_index_path();
    let tmp_path = final_path.with_extension("jsonl.tmp");
    {
        let mut f = fs::File::create(&tmp_path)
            .with_context(|| format!("creating {}", tmp_path.display()))?;
        for r in records {
            let line = serde_json::to_string(r).context("serializing record")?;
            writeln!(f, "{}", line).with_context(|| format!("writing {}", tmp_path.display()))?;
        }
    }
    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "renaming {} -> {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

/// Return the current UTC timestamp in ISO-8601 format (seconds precision).
pub fn now_iso8601() -> String {
    use chrono::SecondsFormat;
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[allow(dead_code)]
pub fn index_path_is(path: &Path) -> bool {
    paths::projects_index_path() == path
}

/// Resolve a project record by query string.
///
/// Resolution order: exact ID → ID prefix → case-insensitive name substring.
/// Returns an error when the query is ambiguous or matches nothing.
pub fn resolve_project(query: &str) -> Result<ProjectRecord> {
    let records = load_all()?;
    if records.is_empty() {
        anyhow::bail!("project index is empty — create a project with `fastf new` first");
    }

    // 1. Exact ID match.
    let mut matches: Vec<&ProjectRecord> = records.iter().filter(|r| r.id == query).collect();

    // 2. ID prefix.
    if matches.is_empty() {
        matches = records.iter().filter(|r| r.id.starts_with(query)).collect();
    }

    // 3. Name substring (case-insensitive).
    if matches.is_empty() {
        let q = query.to_lowercase();
        matches = records
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&q))
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
            for r in matches.iter().take(10) {
                msg.push_str(&format!("  {}  {}  ({})\n", r.id, r.name, r.template));
            }
            anyhow::bail!("{}", msg.trim_end())
        }
    }
}
