//! `fastf copy-to <query> <destination>` — copy a project to a folder outside
//! the library, keeping its ID.
//!
//! Named `copy-to` because `fastf copy` is the clipboard verb and has been
//! since v2.1.0: it exists to be instant from a launcher, and taking its name
//! for something that copies gigabytes would be the worst possible pun. The
//! guided app calls this `Copy to…` on `C`, so the two surfaces read the same.
//!
//! The copy keeps its `PROJECT_INFO.md` byte for byte — it is the same project
//! on another drive. `copy_engine::resolve_destination` is what refuses a
//! destination inside a configured base, and says why.

use anyhow::Result;
use colored::Colorize;
use std::path::Path;

use crate::core::config::Config;
use crate::core::copy_engine::CopyOutcome;
use crate::core::library;

pub struct CopyToArgs {
    /// Project query — exact ID, ID prefix, or name substring.
    pub query: String,
    /// The folder to copy into. `~` is expanded; it must exist.
    pub destination: String,
    /// Skip the confirmation prompt.
    pub yes: bool,
}

pub fn run(args: CopyToArgs) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, &args.query)?;

    // The same expansion every stored path gets, so `~/backups` means the same
    // thing here as it does in `config set bases`.
    let destination = crate::core::config::expand_base_path(&args.destination)?;
    // Refused now rather than after the confirmation: a question about a copy
    // that cannot happen is a question that should not be asked.
    let target = crate::core::copy_engine::resolve_destination(&cfg, &project, &destination)?;

    if !args.yes {
        crate::util::tty::require_tty("confirm", "pass --yes to copy without confirming")?;
        println!(
            "  {} {}  {} {}",
            "copy".dimmed(),
            project.name.bold(),
            "→".cyan(),
            crate::util::paths::display_path(&target)
        );
        println!(
            "  {}",
            format!(
                "the copy keeps {} — add this folder as a base later and both will list",
                project.id
            )
            .dimmed()
        );
        let ok = crate::tui::prompt::confirm("Copy this project?", true)?.unwrap_or(false);
        if !ok {
            crate::tui::prompt::report_cancelled("nothing was copied");
            return Ok(());
        }
    }

    let outcome = run_with_progress(&project, &destination).inspect_err(|error| {
        crate::cli::log::keep(
            crate::util::messages::Level::Error,
            format!(
                "the copy of {} {} failed: {error:#}",
                project.id, project.name
            ),
        )
    })?;
    report(&project, &outcome);
    crate::cli::log::keep(
        crate::util::messages::Level::Good,
        format!(
            "copied {} {} to {}; the original is untouched",
            project.id,
            project.name,
            crate::util::paths::display_path(&outcome.path)
        ),
    );
    Ok(())
}

fn report(project: &library::Project, outcome: &CopyOutcome) {
    let (files, bytes) = outcome.copied;
    println!(
        "{}  Copied {} {}",
        "✓".green().bold(),
        project.id.green().bold(),
        project.name.bold()
    );
    println!(
        "   {} {}",
        "to".dimmed(),
        crate::util::paths::display_path(&outcome.path)
    );
    println!(
        "   {}",
        format!(
            "{}, verified — the original is untouched",
            crate::core::transactions::copied_summary(files, outcome.links, bytes)
        )
        .dimmed()
    );
    for note in &outcome.link_notes {
        eprintln!("{} {note}", "note:".cyan().bold());
    }
}

/// The copy on a worker, the progress on this thread — the same shape `move`
/// uses, and for the same reason: a copy runs for minutes and a silent
/// terminal is indistinguishable from a hung one. Ctrl-C feeds the engine's
/// cancel flag, so an interrupted copy leaves nothing but its own transaction
/// to remove.
fn run_with_progress(project: &library::Project, destination: &Path) -> Result<CopyOutcome> {
    crate::cli::progress::run_watched("copy", |progress, cancel| {
        crate::core::operations::copy_project(project, destination, progress, cancel)
    })
}
