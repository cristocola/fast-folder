//! `fastf cd [query]` — change directory to a project's folder.
//!
//! The binary's half of a two-part verb. A program cannot change the working
//! directory of the shell that ran it, so this command resolves the query
//! exactly as `path` does and prints the bare path, and the shell function
//! `fastf init` prints (`cli::shell_init`) captures that line and enters it.
//! The picker draws on stderr, so an ambiguous query still asks from inside
//! the capture.
//!
//! Run without the function — stdout a terminal, nobody capturing — it prints
//! the path anyway, which is still an answer, and says on stderr how to add
//! the function, for the shell `$SHELL` says the user is in.
//!
//! An omitted query is the whole library: the resolver's prefix tier matches
//! every ID against `""`, so one project is entered and several are picked
//! from. `fastf cd` alone is "take me to a project".

use std::io::IsTerminal;

use anyhow::{Result, bail};
use colored::Colorize;

use crate::cli::shell_init::Shell;
use crate::cli::target::{self, Target};
use crate::core::config::Config;

/// Resolve `query` and print the project's folder path, then — when nothing
/// is capturing it — say why that is all a binary can do.
pub fn run(query: Option<&str>) -> Result<()> {
    let cfg = Config::load()?;
    let query = query.unwrap_or("");
    // The empty query means "every project", which is a picker or nothing.
    // Said here rather than as `'' is ambiguous — 12 matches`, which is true
    // and useless.
    if query.is_empty() && !crate::util::tty::prompt_available() {
        bail!(
            "`fastf cd` with no query picks from every project, which needs a terminal — \
             {}",
            target::full_id_hint("cd")
        );
    }
    let project = match target::one_project(
        &cfg,
        query,
        "Change directory to which project?",
        &target::full_id_hint("cd"),
    )? {
        Target::Project(project) => *project,
        Target::Cancelled => {
            // On stdout, like every cancellation. The shell function prints
            // it, since it is not a directory.
            crate::tui::prompt::report_cancelled("the directory is unchanged");
            return Ok(());
        }
        // A terminal is running this again; it will print the path.
        Target::HandedOff => return Ok(()),
    };
    crate::cli::path_cmd::print_path(&project)?;

    // A terminal on stdout means no function captured the line, so the user
    // is still where they were. Say so once, on the stream for saying things,
    // and name the fix for the shell they are most likely in.
    if std::io::stdout().is_terminal() {
        explain_the_hook();
    }
    Ok(())
}

fn explain_the_hook() {
    eprintln!(
        "{} printed, not entered — a program cannot change the directory of the shell that ran it.",
        "note:".yellow()
    );
    match Shell::current() {
        Some(shell) => {
            let (line, file) = shell.setup();
            eprintln!("      Put {} in {file}, and `fastf cd` will.", line.bold());
        }
        None => eprintln!("      `fastf init --help` shows the shell function that can."),
    }
}
