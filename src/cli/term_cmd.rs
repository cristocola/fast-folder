//! `fastf term <query>` — open a terminal window at a project's folder.
//!
//! The third of the launcher verbs: `open` reveals the folder, `copy` puts the
//! path on the clipboard, `term` puts a shell there. From a desktop launcher an
//! unambiguous query spawns the window directly; an ambiguous one relaunches
//! into a terminal to show the picker, and after the pick that window *becomes*
//! the shell rather than spawning a second one.
//!
//! (Module named `term_cmd` on the `path_cmd` precedent: `cli::term` would read
//! too much like `cli::terminal`, the Config↔relaunch seam next door.)

use anyhow::{Context, Result};
use colored::Colorize;

use crate::core::config::Config;
use crate::core::library;
use crate::util::paths;

/// Resolve `query` and open a terminal at the project's folder.
pub fn run(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    let project = match crate::cli::target::one_project(
        &cfg,
        query,
        "Open a terminal at which project?",
        &crate::cli::target::full_id_hint("term"),
    )? {
        crate::cli::target::Target::Project(project) => *project,
        crate::cli::target::Target::Cancelled => {
            crate::tui::prompt::report_cancelled("no terminal was opened");
            return Ok(());
        }
        // A terminal is running this again; its picker will finish the job.
        crate::cli::target::Target::HandedOff => return Ok(()),
    };

    // Same check as `open`: a discovered path is a hint until it has been
    // looked at, and this one is about to become a shell's working directory.
    library::revalidate_for_read(&project).with_context(|| {
        format!(
            "project '{}' cannot be opened at {}",
            project.id,
            paths::display_path(&project.path)
        )
    })?;

    // The relaunched-picker case: fastf already owns a fresh window that exists
    // only because there was a picker to show. Becoming the shell there *is*
    // the feature — a second window would strand this one — and `exec` also
    // sidesteps the pause a relaunched window takes before closing.
    #[cfg(unix)]
    if crate::cli::terminal::window_is_ours() {
        return Err(crate::util::term_open::exec_shell_at(&project.path));
    }

    let shown = paths::display_path(&project.path);
    println!(
        "{} Opening terminal at {}",
        "→".cyan().bold(),
        shown.dimmed()
    );

    match crate::cli::terminal::open_terminal_at(&cfg, &project.path) {
        Ok(()) => {
            // From a launcher the line above went to the journal; the window is
            // its own feedback, but say so where it can be seen anyway.
            if crate::cli::terminal::headless_gui() {
                crate::cli::terminal::notify("Terminal opened", &shown);
            }
            Ok(())
        }
        Err(e) => {
            // Headless, the propagated error is journal-only — the
            // notification is the one channel left. Best-effort, like the
            // relaunch's own failure path.
            if crate::cli::terminal::headless_gui() {
                crate::cli::terminal::notify("fastf could not open a terminal", &format!("{e:#}"));
            }
            Err(e)
        }
    }
}
