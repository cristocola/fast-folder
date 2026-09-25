//! `fastf move <query> [base]` — move a project folder into another
//! configured base.
//!
//! Targets are restricted to the effective bases (`base_dir` + config `bases`)
//! so a moved project always stays discoverable. Only EXDEV enables the private
//! v2 copy transaction; it verifies topology/lengths and source stability before
//! publication, then removes the source.

use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::library;

pub struct MoveArgs {
    /// Project query — exact ID, ID prefix, or name substring.
    pub query: String,
    /// Target base directory. Omit on a TTY to pick interactively.
    pub base: Option<String>,
    /// Skip the confirmation prompt.
    pub yes: bool,
    /// Start the move and return; `fastf jobs` follows it.
    pub detach: bool,
}

pub fn run(args: MoveArgs) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, &args.query)?;

    let current =
        crate::util::paths::canonical(&project.base).unwrap_or_else(|_| project.base.clone());
    // Mounted configured bases the project could move to. Probed rather than
    // `is_dir`-ed: a dead network mount answers `is_dir()` only after the
    // operating system's own timeout, and nothing on screen says why.
    let (mounted, unusable) = crate::util::paths::mounted_bases(&cfg.effective_bases());
    for (path, probe) in &unusable {
        eprintln!(
            "{} skipping base {}{}",
            "note:".yellow(),
            crate::util::paths::display_path(path),
            probe.note()
        );
    }
    let candidates: Vec<PathBuf> = mounted.into_iter().filter(|b| *b != current).collect();

    if candidates.is_empty() {
        anyhow::bail!(
            "no other bases configured — add one with `fastf config set bases <dir,...>` \
             or in Settings → Library bases"
        );
    }

    let target = match &args.base {
        Some(raw) => {
            let wanted = PathBuf::from(raw);
            let wanted = crate::util::paths::canonical(&wanted).unwrap_or(wanted);
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
            let default_base = cfg.effective_bases().first().cloned();
            let picked = crate::tui::pickers::pick_base(
                &format!("Move '{}' to which base?", project.name),
                &candidates,
                default_base.as_deref(),
                "name the target instead: `fastf move <query> <base>`",
                true,
            )?;
            match picked {
                Some(base) => base,
                None => {
                    println!("{}", "Cancelled — nothing moved.".dimmed());
                    return Ok(());
                }
            }
        }
    };

    // Confirm before touching anything. Deliberately no size figure: getting one
    // means walking the whole tree, which is wasted on the same-filesystem
    // rename that handles most moves, and slow over NTFS. The progress line
    // below reports real numbers once there is actually something to copy.
    if !args.yes {
        // A confirmation that cannot be shown is not a confirmation. Skipping it
        // here moved the folder on the strength of a question nobody was asked.
        crate::util::tty::require_tty("confirm", "pass --yes to move without confirming")?;
        println!(
            "  {} {}  {} {}",
            "move".dimmed(),
            project.name.bold(),
            "→".cyan(),
            crate::util::paths::display_path(&target)
        );
        let ok = crate::tui::prompt::confirm("Move this project?", true)?.unwrap_or(false);
        if !ok {
            crate::tui::prompt::report_cancelled("nothing moved");
            return Ok(());
        }
    }

    // A job of its own: this terminal follows it, and closing it or killing
    // this process leaves the move to finish (`cli::jobs`).
    let item = crate::core::jobs::JobItem::of(&project, Some(&target))?;
    match crate::cli::jobs::start(crate::core::jobs::JobKind::Move, vec![item], args.detach)? {
        crate::cli::jobs::Followed::Ended(state) => crate::cli::jobs::finish(&state),
        _ => Ok(()),
    }
}
