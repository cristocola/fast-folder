use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::path::Path;

use crate::core::index::{self, ProjectRecord};

pub struct RecentArgs {
    pub limit: usize,
    pub template: Option<String>,
    pub since: Option<String>,
    pub prune: bool,
}

pub fn run(args: RecentArgs) -> Result<()> {
    let records = index::load_all()?;

    if args.prune {
        return prune(&records);
    }

    if records.is_empty() {
        println!(
            "{}",
            "No projects yet — create one with `fastf new`.".dimmed()
        );
        return Ok(());
    }

    // Records are append-only so newest is at the end. Reverse + filter + take.
    let mut filtered: Vec<&ProjectRecord> = records
        .iter()
        .rev()
        .filter(|r| {
            if let Some(ref slug) = args.template
                && &r.template != slug
            {
                return false;
            }
            if let Some(ref since) = args.since
                && r.created_at.as_str() < since.as_str()
            {
                return false;
            }
            true
        })
        .collect();

    filtered.truncate(args.limit);

    if filtered.is_empty() {
        println!("{}", "No projects match those filters.".dimmed());
        return Ok(());
    }

    // Column widths
    let id_w = filtered.iter().map(|r| r.id.len()).max().unwrap_or(4);
    let tmpl_w = filtered.iter().map(|r| r.template.len()).max().unwrap_or(8);
    let date_w = 10; // YYYY-MM-DD

    for r in filtered {
        let date = r.created_at.get(..date_w).unwrap_or(&r.created_at);
        let missing = !Path::new(&r.path).exists();
        let marker = if missing {
            "✗".red()
        } else {
            "•".cyan()
        };
        println!(
            "  {} {:<id_w$}  {:<tmpl_w$}  {}  {}",
            marker,
            r.id.green().bold(),
            r.template.dimmed(),
            date.dimmed(),
            if missing {
                format!("{} {}", r.name, "(missing)".red()).to_string()
            } else {
                r.name.clone()
            },
            id_w = id_w,
            tmpl_w = tmpl_w,
        );
        println!("      {} {}", "→".dimmed(), r.path.dimmed());
    }

    Ok(())
}

fn prune(records: &[ProjectRecord]) -> Result<()> {
    let (keep, dropped): (Vec<_>, Vec<_>) = records
        .iter()
        .cloned()
        .partition(|r| Path::new(&r.path).exists());

    if dropped.is_empty() {
        println!("{}", "Index is already clean — no missing projects.".dimmed());
        return Ok(());
    }

    index::rewrite(&keep).context("rewriting index")?;
    println!(
        "{} Removed {} stale record{}.",
        "✓".green().bold(),
        dropped.len(),
        if dropped.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

pub fn open(query: &str) -> Result<()> {
    let records = index::load_all()?;
    if records.is_empty() {
        bail!("project index is empty — create a project with `fastf new` first");
    }

    // Resolution order: exact id → id prefix → substring on name.
    let mut matches: Vec<&ProjectRecord> = records.iter().filter(|r| r.id == query).collect();
    if matches.is_empty() {
        matches = records.iter().filter(|r| r.id.starts_with(query)).collect();
    }
    if matches.is_empty() {
        let q = query.to_lowercase();
        matches = records
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&q))
            .collect();
    }

    match matches.len() {
        0 => bail!("no project matches '{}' — try `fastf recent`", query),
        1 => {
            let r = matches[0];
            let path = Path::new(&r.path);
            if !path.exists() {
                bail!(
                    "project '{}' no longer exists on disk at {} — run `fastf recent --prune` to clean up",
                    r.id,
                    r.path
                );
            }
            println!("{} Opening {} ({})", "→".cyan().bold(), r.name.bold(), r.path.dimmed());
            open_folder(path)
        }
        _ => {
            eprintln!("{} '{}' is ambiguous — pick a specific ID:", "error:".red().bold(), query);
            for r in matches {
                eprintln!("  {}  {}  ({})", r.id.green(), r.name, r.template.dimmed());
            }
            bail!("{} matches", records.len())
        }
    }
}

#[cfg(windows)]
fn open_folder(path: &Path) -> Result<()> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", &path.display().to_string()])
        .status()
        .context("spawning cmd /c start")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_folder(path: &Path) -> Result<()> {
    std::process::Command::new("open").arg(path).status().context("spawning open")?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_folder(path: &Path) -> Result<()> {
    std::process::Command::new("xdg-open").arg(path).status().context("spawning xdg-open")?;
    Ok(())
}
