//! `fastf cd [query]` — change directory to a project's folder.
//!
//! The binary's half of a two-part verb. A program cannot change the working
//! directory of the shell that ran it, so this command resolves the query
//! exactly as `path` does and prints the bare path, and the shell function
//! `fastf init` prints (`cli::shell_init`) captures that line and enters it.
//! The picker draws on stderr, so an ambiguous query still asks from inside
//! the capture.
//!
//! Run without the function — stdout a terminal, nobody capturing — there is
//! a person in a shell that cannot be moved, and printing a path at them is not
//! `cd`. So fastf does the two things it can (`cli::shell_setup`): the first
//! time, it offers to add the function to that shell's startup file itself, so
//! nobody edits a file by hand; and either way it starts a new shell of the
//! same kind *in* the project, which `exit` leaves. A shell that can never have
//! the function — `cmd.exe`, a PowerShell whose policy runs no profile — gets
//! the new shell every time.
//!
//! An omitted query is the whole library: the resolver's prefix tier matches
//! every ID against `""`, so one project is entered and several are picked
//! from. `fastf cd` alone is "take me to a project".

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result, bail};
use colored::Colorize;

use crate::cli::shell_setup::{self, Host, Setup};
use crate::cli::target::{self, Target};
use crate::core::config::Config;
use crate::core::library;
use crate::util::paths;

/// Resolve `query` and print the project's folder path for the function to
/// enter — or, when nothing is capturing it, enter it the other way.
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
    // A terminal on stdout means no function captured the line, so the user
    // is still where they were: move them the only ways a program can.
    if std::io::stdout().is_terminal() && crate::util::tty::prompt_available() {
        library::revalidate_for_read(&project)
            .with_context(|| format!("project '{}' cannot be entered", project.id))?;
        return enter_without_the_function(&project.path);
    }
    crate::cli::path_cmd::print_path(&project)
}

/// Start a shell in `dir`, having first offered — once — to make the next
/// `fastf cd` move the shell it was typed into instead.
fn enter_without_the_function(dir: &Path) -> Result<()> {
    let shown = paths::display_path(dir);
    let Some(host) = Host::detect() else {
        // Nothing to start: say what a program can say.
        println!("{shown}");
        eprintln!(
            "{} printed, not entered — a program cannot change the directory of the shell that ran \
             it, and this one is not a shell fastf knows.",
            "note:".yellow()
        );
        return Ok(());
    };

    // The window a relaunch opened for the picker is fastf's own; it becomes
    // the shell, as `fastf term`'s does, and asks nothing about startup files.
    let setup = if crate::cli::terminal::window_is_ours() {
        None
    } else {
        Some(shell_setup::inspect(&host))
    };
    let why = match setup {
        None => String::new(),
        Some(Setup::Present(file)) => format!(
            "This terminal started before `fastf cd` was set up in {}, so here is a new {} in \
             the project.",
            paths::display_path(&file),
            host.name()
        ),
        Some(Setup::Unavailable(reason)) => {
            format!("{reason}, so here is a new {} in the project.", host.name())
        }
        Some(Setup::Missing(plan)) if shell_setup::declines::contains(plan.shell) => {
            format!("Here is a new {} in the project.", host.name())
        }
        Some(Setup::Missing(plan)) => {
            eprintln!(
                "`fastf cd` moves a shell through a small function in its startup file. \
                 fastf can add it for you."
            );
            match crate::tui::prompt::confirm(&format!("Add it to {}?", plan.shown()), true)? {
                None => {
                    crate::tui::prompt::report_cancelled("the directory is unchanged");
                    return Ok(());
                }
                Some(true) => {
                    plan.apply()?;
                    format!(
                        "{} Added to {} — new terminals change directory with `fastf cd`. \
                         This one started before that, so here is a new {} in the project.",
                        "✓".green(),
                        plan.shown(),
                        host.name()
                    )
                }
                Some(false) => {
                    shell_setup::declines::remember(plan.shell);
                    format!(
                        "Here is a new {} in the project. (`fastf init` sets `fastf cd` up \
                         whenever you want.)",
                        host.name()
                    )
                }
            }
        }
    };
    if !why.is_empty() {
        eprintln!("{why}");
    }
    eprintln!(
        "{} {}  {}",
        "→".cyan().bold(),
        shown,
        "(`exit` to go back)".dimmed()
    );

    #[cfg(unix)]
    {
        Err(crate::util::term_open::exec_shell_at(
            Some(host.program.as_os_str()),
            dir,
        ))
    }
    #[cfg(windows)]
    {
        let code = crate::util::term_open::run_shell_at(host.program.as_os_str(), dir)?;
        std::process::exit(code)
    }
}
