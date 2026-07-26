//! `fastf move <query> [base]` — move a project folder into another
//! configured base.
//!
//! Targets are restricted to the effective bases (`base_dir` + config `bases`)
//! so a moved project always stays discoverable. Cross-filesystem moves (e.g.
//! btrfs `~` → NTFS `/mnt/proj`) fall back to a verbatim copy + remove inside
//! `library::move_project`.

use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::library;

pub struct MoveArgs {
    /// Project query — exact ID, ID prefix, or name substring.
    pub query: String,
    /// Target base directory. Omit on a TTY to pick interactively.
    pub base: Option<String>,
}

pub fn run(args: MoveArgs) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, &args.query)?;

    let current = project
        .base
        .canonicalize()
        .unwrap_or_else(|_| project.base.clone());
    // Mounted configured bases the project could move to.
    let candidates: Vec<PathBuf> = cfg
        .effective_bases()
        .into_iter()
        .filter(|b| b.is_dir() && *b != current)
        .collect();

    if candidates.is_empty() {
        anyhow::bail!(
            "no other bases configured — add one with `fastf config set bases <dir,...>` \
             or in Settings → Library bases"
        );
    }

    let target = match &args.base {
        Some(raw) => {
            let wanted = PathBuf::from(raw);
            let wanted = wanted.canonicalize().unwrap_or(wanted);
            if wanted == current {
                anyhow::bail!(
                    "'{}' is already in base {}",
                    project.name,
                    current.display()
                );
            }
            // Accept a full path or a base's short label (its folder name).
            candidates
                .iter()
                .find(|b| **b == wanted || library::base_label(b) == raw.trim_end_matches('/'))
                .cloned()
                .ok_or_else(|| {
                    let list = candidates
                        .iter()
                        .map(|b| format!("  {}", b.display()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    anyhow::anyhow!(
                        "'{}' is not a configured base. Valid targets:\n{}",
                        raw,
                        list
                    )
                })?
        }
        None => {
            if !std::io::stdout().is_terminal() {
                anyhow::bail!("no target base given — usage: fastf move <query> <base>");
            }
            let labels: Vec<String> = candidates
                .iter()
                .map(|b| format!("{}  ({})", library::base_label(b), b.display()))
                .collect();
            let idx = dialoguer::Select::new()
                .with_prompt(format!("Move '{}' to which base?", project.name))
                .items(&labels)
                .default(0)
                .interact()?;
            candidates[idx].clone()
        }
    };

    let moved = library::move_project(&project, &target)?;
    println!(
        "{}  Moved {} {}",
        "✓".green().bold(),
        moved.id.green().bold(),
        moved.name.bold()
    );
    println!(
        "   {} {}",
        "from".dimmed(),
        crate::util::paths::display_path(&project.path).dimmed()
    );
    println!(
        "   {} {}",
        "to  ".dimmed(),
        crate::util::paths::display_path(&moved.path)
    );
    Ok(())
}
