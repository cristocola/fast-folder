//! `fastf agent <query>` — start the configured agent in a project's folder.
//!
//! The sibling of `term`: where `term` puts a shell at the project, `agent`
//! puts the agent there and does nothing more. The `agent` configuration key
//! is one shell line, the shape a `post_create` command has, and it runs the
//! way one does — the project as its working folder, `FASTF_PROJECT_PATH`
//! set, `{path}` rewritten — in the terminal this was typed in, and exits
//! with the agent's status. What the agent does with `PROJECT_INFO.md` is
//! between the user and the agent's own prompt: the folder is whole, and the
//! raw file is all it needs to leave its trace.
//!
//! From a desktop launcher there is no terminal, so the same hand-off that
//! opens `term`'s window opens one running this command again, which then
//! runs in place inside it. The app's `x` opens such a window for a project
//! while it keeps its own screen (`cli::terminal::open_terminal_running`).

use anyhow::{Context, Result};
use colored::Colorize;

use crate::core::config::Config;
use crate::core::library;
use crate::util::paths;

/// The refusal with no agent configured, in one place: the command and the
/// app's dimmed verb say the same thing.
pub const NO_AGENT: &str = "no agent configured — `fastf config set agent claude` names one";

/// Resolve `query` and run the agent in the project's folder.
pub fn run(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    let Some(line) = cfg.resolve_agent() else {
        anyhow::bail!("{NO_AGENT}");
    };
    // From a launcher nothing printed is seen and the agent would talk to
    // nobody: hand the whole command to a terminal, which runs it again.
    if crate::cli::terminal::hand_off_to_a_terminal(&cfg, false) {
        return Ok(());
    }
    let project = match crate::cli::target::one_project(
        &cfg,
        query,
        "Start the agent in which project?",
        &crate::cli::target::full_id_hint("agent"),
    )? {
        crate::cli::target::Target::Project(project) => *project,
        crate::cli::target::Target::Cancelled => {
            crate::tui::prompt::report_cancelled("no agent was started");
            return Ok(());
        }
        crate::cli::target::Target::HandedOff => return Ok(()),
    };

    // A discovered path is a hint until it has been looked at, and this one
    // is about to be a program's working directory.
    library::revalidate_for_read(&project)
        .with_context(|| format!("project '{}' cannot be opened", project.id))?;

    let shown = paths::display_path(&project.path);
    eprintln!(
        "{} {} in {}",
        "→".cyan().bold(),
        line.bold(),
        shown.dimmed()
    );
    let status = crate::core::post_create::run_line_in(&line, &project.path)
        .with_context(|| format!("starting `{line}`"))?;
    if !status.success() {
        anyhow::bail!("the agent exited with {status}");
    }
    Ok(())
}
