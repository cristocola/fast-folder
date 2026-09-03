//! The reads the workers perform: the summary, a discovery, one project's
//! detail. Plain functions returning data, so a worker is a thread that calls
//! one and sends the answer.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::core::config::Config;
use crate::core::library::{self, Project};
use crate::core::project_info::{self, Metadata};
use crate::core::{provisioning, template};
use crate::tui::app::data::{BaseInfo, Entry, ProjectDetail, Summary, TemplateCard};
use crate::util::paths;

/// The header, from the indexes: no base is scanned to draw it. Each base is
/// probed with a timeout rather than `is_dir`-ed, so a dead network mount costs
/// `PROBE_TIMEOUT` once instead of a frozen screen.
pub fn summary() -> Result<Summary> {
    let cfg = Config::load()?;
    let bases = cfg.effective_bases();
    let default_base = cfg.resolve_base_dir();
    let probed = paths::probe_dirs(&bases, paths::PROBE_TIMEOUT);

    let mut summary = Summary::default();
    for (base, probe) in probed {
        let index = probe
            .usable()
            .then(|| library::index_summary(&base))
            .flatten();
        if let Some(index) = &index {
            summary.projects += index.projects;
            if let Some(id) = &index.max_id
                && summary.max_id.as_ref().is_none_or(|held| {
                    crate::core::naming::id_value(held) < crate::core::naming::id_value(id)
                })
            {
                summary.max_id = Some(id.clone());
            }
            if summary.newest.is_none() {
                summary.newest = index.newest.clone();
            }
        }
        summary.bases.push(BaseInfo {
            label: library::base_label(&base),
            is_default: base == default_base,
            indexed: index.map(|i| i.projects),
            path: base,
            probe,
        });
    }

    summary.templates = match template::load_all() {
        Ok(templates) => templates.iter().map(template_card).collect(),
        Err(err) => {
            crate::util::diag::warn(format!("templates could not be listed: {err:#}"));
            Vec::new()
        }
    };
    summary.attention = provisioning::list_incomplete(&cfg).len();
    Ok(summary)
}

fn template_card(t: &template::Template) -> TemplateCard {
    fn count(nodes: &[template::FolderNode]) -> usize {
        nodes.iter().map(|n| 1 + count(&n.children)).sum()
    }
    TemplateCard {
        slug: t.slug.clone(),
        name: t.name.clone(),
        description: t.description.clone(),
        variables: t.variables.len(),
        folders: count(&t.structure),
        naming_pattern: t.naming_pattern.clone(),
    }
}

/// Every project, newest first, through the caches.
pub fn discover() -> Result<Vec<Project>> {
    let cfg = Config::load()?;
    Ok(library::discover(&cfg))
}

/// Read one project's `PROJECT_INFO.md` for a query that needs its variables.
pub fn metadata(paths: &[PathBuf]) -> Vec<(PathBuf, Option<Metadata>)> {
    paths
        .iter()
        .map(|path| {
            let meta = project_info::read_metadata(path).ok().flatten();
            (path.clone(), meta)
        })
        .collect()
}

/// How many entries of a folder the pane lists.
const LISTING_LIMIT: usize = 200;
/// How many journal entries the pane keeps.
const JOURNAL_LIMIT: usize = 5;
/// How many lines of the notes section the pane keeps.
const NOTES_LIMIT: usize = 8;

/// The detail pane's reads for one project.
pub fn detail(path: &Path) -> ProjectDetail {
    let mut detail = ProjectDetail::default();

    match project_info::read_metadata(path) {
        Ok(meta) => detail.meta = meta,
        Err(err) => detail.error = Some(format!("{err:#}")),
    }

    if let Ok(entries) = project_info::read_journal_entries(path) {
        detail.journal_count = entries.len();
        detail.journal = entries
            .iter()
            .rev()
            .take(JOURNAL_LIMIT)
            .map(|entry| {
                (
                    entry
                        .timestamp
                        .get(..10)
                        .unwrap_or(&entry.timestamp)
                        .to_string(),
                    entry.message.clone(),
                )
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    if let Ok(content) = project_info::read(path) {
        detail.notes = notes_section(&content);
    }

    detail.listing = listing(path);
    detail
}

/// The first lines of `## Notes`, up to the next heading.
fn notes_section(content: &str) -> Vec<String> {
    let body = project_info::split_frontmatter_body(content)
        .map(|(_, body)| body)
        .unwrap_or(content);
    let Some(start) = body.find("## Notes") else {
        return Vec::new();
    };
    body[start..]
        .lines()
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .take(NOTES_LIMIT)
        .map(str::to_string)
        .collect()
}

/// Directories first, then files, both sorted; the metadata file hidden.
fn listing(path: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == project_info::RESERVED_FILENAME {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(Entry { name, is_dir })
        })
        .take(LISTING_LIMIT)
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    entries
}

#[cfg(test)]
mod tests {
    use super::notes_section;

    #[test]
    fn the_notes_section_stops_at_the_next_heading() {
        let content = "---\nid: ID0001\n---\n# Project Info\n\n## Notes\n\nfirst cut due Friday\n\n## Journal\n- entry\n";
        assert_eq!(
            notes_section(content),
            vec!["first cut due Friday".to_string()]
        );
        assert!(notes_section("# nothing").is_empty());
    }
}
