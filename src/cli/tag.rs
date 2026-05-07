//! `fastf tag` subcommands — add, remove, list, reauto.
//!
//! Tags live in the YAML frontmatter of each project's `PROJECT_INFO.md`.
//! Free-form tags are arbitrary strings (e.g. `draft`, `urgent`).
//! Auto-derived tags follow the `slug/value` convention (e.g. `artist/Ariana_Grande`).
//!
//! `tag reauto` re-derives the auto tags from the current frontmatter variables
//! while leaving free-form tags untouched.

use anyhow::{Result, bail};
use colored::Colorize;
use std::path::Path;

use crate::core::{config::Config, index, project_info, template};

// ---------------------------------------------------------------------------
// Public entry points (called from main.rs)
// ---------------------------------------------------------------------------

pub fn add(query: &str, new_tags: &[String]) -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let record = index::resolve_project(query)?;
    let path = pinfo_path(&record.path, &cfg);

    project_info::write_frontmatter(&path, |meta| {
        for tag in new_tags {
            if !meta.tags.contains(tag) {
                meta.tags.push(tag.clone());
            }
        }
    })?;

    let n = new_tags.len();
    println!(
        "{}  Added {} tag{} to {}",
        "✓".green().bold(),
        n,
        if n == 1 { "" } else { "s" },
        record.id.green().bold()
    );
    Ok(())
}

pub fn remove(query: &str, remove_tags: &[String]) -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let record = index::resolve_project(query)?;
    let path = pinfo_path(&record.path, &cfg);

    let mut removed_count = 0usize;
    project_info::write_frontmatter(&path, |meta| {
        let before = meta.tags.len();
        meta.tags.retain(|t| !remove_tags.contains(t));
        removed_count = before - meta.tags.len();
    })?;

    if removed_count == 0 {
        println!(
            "{}  No matching tags found on {} — nothing changed.",
            "i".cyan(),
            record.id.green().bold()
        );
    } else {
        println!(
            "{}  Removed {} tag{} from {}",
            "✓".green().bold(),
            removed_count,
            if removed_count == 1 { "" } else { "s" },
            record.id.green().bold()
        );
    }
    Ok(())
}

pub fn list(query: &str) -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let record = index::resolve_project(query)?;
    let path = pinfo_path(&record.path, &cfg);

    if !path.exists() {
        bail!(
            "no {} found for project {} — this project may predate the metadata feature",
            cfg.project_info_filename,
            record.id
        );
    }

    let meta = project_info::read_metadata(Path::new(&record.path), &cfg)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no YAML frontmatter — cannot read tags",
            path.display()
        )
    })?;

    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        record.id.green().bold(),
        record.name.bold()
    );

    if meta.tags.is_empty() {
        println!("    {}", "(no tags)".dimmed());
    } else {
        for tag in &meta.tags {
            println!("    {} {}", "•".yellow(), tag.yellow());
        }
    }
    Ok(())
}

/// Re-derive auto-tags from the current frontmatter variables, replacing any
/// previously derived tags (identified by `slug/` prefix for slugs in
/// `template.tag_from`) while keeping free-form tags intact.
pub fn reauto(query: &str) -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let record = index::resolve_project(query)?;
    let path = pinfo_path(&record.path, &cfg);

    // Load the current metadata to get variables and current tags.
    let meta = project_info::read_metadata(Path::new(&record.path), &cfg)?
        .ok_or_else(|| anyhow::anyhow!("{} has no YAML frontmatter", path.display()))?;

    // Load the template to get tag_from.
    let tmpl = template::find_by_slug(&record.template)?;

    // Compute new derived tags from the current variable values.
    let new_derived: Vec<String> = tmpl
        .tag_from
        .iter()
        .filter_map(|slug| {
            let value = meta.variables.get(slug)?;
            if value.is_empty() {
                None
            } else {
                Some(format!("{slug}/{value}"))
            }
        })
        .collect();

    // The set of prefixes to remove (slug/ patterns owned by tag_from).
    let owned_prefixes: Vec<String> = tmpl.tag_from.iter().map(|s| format!("{s}/")).collect();

    project_info::write_frontmatter(&path, |m| {
        // Keep tags that are NOT derived from tag_from slugs.
        m.tags
            .retain(|t| !owned_prefixes.iter().any(|pfx| t.starts_with(pfx.as_str())));
        // Also remove stale literal tags that were part of template.tags
        // (keep free-form tags the user added manually via `fastf tag add`).
        m.tags.extend(new_derived.iter().cloned());
    })?;

    println!(
        "{}  Re-derived {} auto-tag{} for {}",
        "✓".green().bold(),
        new_derived.len(),
        if new_derived.len() == 1 { "" } else { "s" },
        record.id.green().bold()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the full path to the project's metadata file.
fn pinfo_path(project_path: &str, cfg: &Config) -> std::path::PathBuf {
    Path::new(project_path).join(&cfg.project_info_filename)
}
