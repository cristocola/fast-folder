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

    let outcome = run_with_progress(&project, &target).inspect_err(|error| {
        crate::cli::log::keep(
            crate::util::messages::Level::Error,
            format!(
                "the move of {} {} failed: {error:#}",
                project.id, project.name
            ),
        )
    })?;
    let moved = &outcome.project;

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
    // Which kind of move it was. Without it a rename and a two-hundred-gigabyte
    // staged copy print the same three lines, and the instant one reads as
    // though nothing happened.
    println!(
        "   {}",
        match outcome.copied {
            Some((files, bytes)) => format!(
                "copied {}, verified",
                crate::core::transactions::copied_summary(files, outcome.links, bytes)
            ),
            None => "renamed on the same filesystem, nothing copied".to_string(),
        }
        .dimmed()
    );
    for note in &outcome.link_notes {
        eprintln!("{} {note}", "note:".cyan().bold());
    }
    let warning = outcome.source.warning(&project.path);
    if let Some(warning) = &warning {
        eprintln!("{} {warning}", "warning:".yellow().bold());
    }
    crate::cli::log::keep(
        if warning.is_some() {
            crate::util::messages::Level::Warn
        } else {
            crate::util::messages::Level::Good
        },
        match &warning {
            Some(warning) => format!(
                "moved {} {} to {}; {warning}",
                moved.id,
                moved.name,
                crate::util::paths::display_path(&moved.path)
            ),
            None => format!(
                "moved {} {} to {}",
                moved.id,
                moved.name,
                crate::util::paths::display_path(&moved.path)
            ),
        },
    );
    Ok(())
}

/// Run the move on a worker thread and report progress from this one.
///
/// A cross-filesystem move can copy for minutes and remove for longer;
/// without this the command line would sit silent for the whole of it.
/// `cli::progress` draws it the way the app does.
fn run_with_progress(
    project: &library::Project,
    target: &std::path::Path,
) -> Result<library::MoveOutcome> {
    crate::cli::progress::run_watched("move", |progress, cancel| {
        crate::core::operations::move_project(project, target, progress, cancel)
    })
}
