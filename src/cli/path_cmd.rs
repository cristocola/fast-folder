//! `fastf path <query>` — print a project's folder path, and nothing else.
//!
//! The scriptable half of the pair: `cd "$(fastf path api)"`. Its whole
//! contract is that one line, so nothing here colours, decorates or explains —
//! `fastf copy` is where a human is being spoken to.
//!
//! (Module can't be named `path` inside cli/ without reading like
//! `std::path` at every call site; `path_cmd` follows `paths_cmd`.)

use anyhow::{Context, Result};

use crate::core::config::Config;
use crate::core::library;
use crate::util::paths;

/// Resolve `query` and print the project's folder path.
pub fn run(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    // An ambiguous query asks, when there is a terminal on stderr to ask on —
    // which `cd "$(fastf path lullaby)"` has, its stdout being a pipe. The
    // picker never writes to stdout, so the line below stays the only thing
    // there. With no terminal at all it is the error it has always been.
    let project = match crate::cli::target::one_project(
        &cfg,
        query,
        "Which project's path?",
        &crate::cli::target::full_id_hint("path"),
    )? {
        crate::cli::target::Target::Project(project) => *project,
        crate::cli::target::Target::Cancelled => {
            crate::tui::prompt::report_cancelled("no path printed");
            return Ok(());
        }
        // A terminal is running this again; it will print the path.
        crate::cli::target::Target::HandedOff => return Ok(()),
    };
    print_path(&project)
}

/// Revalidate, then print the bare line.
fn print_path(project: &library::Project) -> Result<()> {
    // Same check as `open` and `copy`: a discovered path is a hint until it has
    // been looked at, and this one is about to be pasted into another command.
    library::revalidate_for_read(project).with_context(|| {
        format!(
            "project '{}' has no folder at {}",
            project.id,
            paths::display_path(&project.path)
        )
    })?;

    // No `colored` call, deliberately: the bare line is the script contract.
    let shown = paths::display_path(&project.path);
    println!("{shown}");

    // From a launcher that line went to the journal, where it is a trace and
    // not an answer. Degrade to what `copy` does — the clipboard and a
    // notification — rather than to nothing. The print stays: it costs nothing
    // and it is the one record of what the command decided.
    if crate::cli::terminal::headless_gui() {
        crate::util::clipboard::copy(&shown);
        crate::cli::terminal::notify("Path copied", &shown);
    }
    Ok(())
}
