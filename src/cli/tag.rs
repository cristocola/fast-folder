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

use crate::core::library;
use crate::core::{config::Config, project_info};

// ---------------------------------------------------------------------------
// Public entry points (called from main.rs)
// ---------------------------------------------------------------------------

pub fn add(query: &str, new_tags: &[String]) -> Result<()> {
    let cfg = Config::load()?;
    let candidate = library::resolve(&cfg, query)?;

    crate::core::operations::add_tags(&candidate, new_tags)?;

    let n = new_tags.len();
    println!(
        "{}  Added {} tag{} to {}",
        "✓".green().bold(),
        n,
        if n == 1 { "" } else { "s" },
        candidate.id.green().bold()
    );
    Ok(())
}

pub fn remove(query: &str, remove_tags: &[String]) -> Result<()> {
    let cfg = Config::load()?;
    let candidate = library::resolve(&cfg, query)?;

    let before = project_info::read_metadata(&candidate.path)?
        .map(|metadata| metadata.tags.len())
        .unwrap_or(0);
    let after = crate::core::operations::remove_tags(&candidate, remove_tags)?;
    let removed_count = before.saturating_sub(after.len());

    if removed_count == 0 {
        println!(
            "{}  No matching tags found on {} — nothing changed.",
            "i".cyan(),
            candidate.id.green().bold()
        );
    } else {
        println!(
            "{}  Removed {} tag{} from {}",
            "✓".green().bold(),
            removed_count,
            if removed_count == 1 { "" } else { "s" },
            candidate.id.green().bold()
        );
    }
    Ok(())
}

pub fn list(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    let candidate = library::resolve(&cfg, query)?;
    let project = library::revalidate_project(&cfg, &candidate)?;
    let path = project_info::pinfo_path(&project.path);
    if !path.exists() {
        bail!(
            "no {} found for project {} — this project may predate the metadata feature",
            project_info::RESERVED_FILENAME,
            project.id
        );
    }

    let meta = project_info::read_metadata(&project.path)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no YAML frontmatter — cannot read tags",
            path.display()
        )
    })?;

    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        project.id.green().bold(),
        project.name.bold()
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
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, query)?;

    // A registered folder has no template, so there is nothing to re-derive
    // from. Say that, rather than letting the lookup fail with "template
    // '(registered)' not found" — which reads like a broken install.
    if project.template == crate::core::operations::REGISTERED_SLUG {
        bail!(
            "{} was registered without a template, so it has no auto-derived tags to re-derive.\n  \
             Add tags directly with `fastf tag add {} <tag>`, or re-register it with \
             `fastf register <path> --template <slug>`.",
            project.id,
            project.id
        );
    }

    let new_derived = crate::core::operations::replace_auto_tags(&project)?;

    println!(
        "{}  Re-derived {} auto-tag{} for {}",
        "✓".green().bold(),
        new_derived.len(),
        if new_derived.len() == 1 { "" } else { "s" },
        project.id.green().bold()
    );
    Ok(())
}
