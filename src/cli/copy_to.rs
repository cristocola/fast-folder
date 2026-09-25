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

use crate::core::config::Config;
use crate::core::library;

pub struct CopyToArgs {
    /// Project query — exact ID, ID prefix, or name substring.
    pub query: String,
    /// The folder to copy into. `~` is expanded; it must exist.
    pub destination: String,
    /// Skip the confirmation prompt.
    pub yes: bool,
    /// Start the copy and return; `fastf jobs` follows it.
    pub detach: bool,
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

    let item = crate::core::jobs::JobItem::of(&project, Some(&destination))?;
    match crate::cli::jobs::start(crate::core::jobs::JobKind::Copy, vec![item], args.detach)? {
        crate::cli::jobs::Followed::Ended(state) => crate::cli::jobs::finish(&state),
        _ => Ok(()),
    }
}
