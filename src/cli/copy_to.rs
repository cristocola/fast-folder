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
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::core::assets::Progress;
use crate::core::config::Config;
use crate::core::copy_engine::CopyOutcome;
use crate::core::library;

const TICK: Duration = Duration::from_millis(200);

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
            println!("Aborted.");
            return Ok(());
        }
    }

    let outcome = run_with_progress(&project, &destination)?;
    report(&project, &outcome);
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
            "{files} file{}, {}, verified — the original is untouched",
            if files == 1 { "" } else { "s" },
            crate::util::human_bytes::human_bytes(bytes)
        )
        .dimmed()
    );
}

/// The copy on a worker, the progress line on this thread — the same shape
/// `move` uses, and for the same reason: a copy runs for minutes and a silent
/// terminal is indistinguishable from a hung one. Ctrl-C feeds the engine's
/// cancel flag, so an interrupted copy leaves nothing but its own transaction
/// to remove.
fn run_with_progress(project: &library::Project, destination: &Path) -> Result<CopyOutcome> {
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    let live = std::io::stdout().is_terminal();

    let done = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            crate::core::operations::copy_project(project, destination, &progress, &cancel)
        });
        let mut drew = false;
        while !worker.is_finished() {
            if crate::util::interrupt::is_set() {
                cancel.store(true, Ordering::Relaxed);
            }
            if live {
                let snapshot = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if snapshot.total_files > 0 {
                    crate::cli::move_project::draw(&snapshot);
                    drew = true;
                }
            }
            std::thread::sleep(TICK);
        }
        if drew {
            println!();
        }
        worker.join()
    });

    match done {
        Ok(result) => result,
        Err(_) => anyhow::bail!("the copy thread panicked"),
    }
}
