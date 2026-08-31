//! `fastf copy <query>` — put a project's path on the clipboard.
//!
//! The command-line half of the TUI action menu's **Copy path**, and the verb a
//! desktop launcher can actually use: it needs no terminal, so `Alt+Space`,
//! `fastf copy lullaby`, Enter leaves the folder on the clipboard with nothing
//! to read. `fastf path` is its scriptable sibling — see that module.

use anyhow::{Context, Result};
use colored::Colorize;

use crate::core::config::Config;
use crate::core::library;
use crate::util::{clipboard, paths};

/// Resolve `query` and copy the project's folder path.
pub fn run(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    let project = match crate::cli::target::one_project(
        &cfg,
        query,
        "Which project's path?",
        &crate::cli::target::full_id_hint("copy"),
    )? {
        crate::cli::target::Target::Project(project) => *project,
        crate::cli::target::Target::Cancelled => {
            crate::tui::prompt::report_cancelled("nothing was copied");
            return Ok(());
        }
        // A terminal is running this again; it will do the copying.
        crate::cli::target::Target::HandedOff => return Ok(()),
    };
    report_copy(&project)
}

/// Revalidate, then copy — the tail both a direct and a picker-driven `copy`
/// arrive at, so they check the same things and say the same words.
fn report_copy(project: &library::Project) -> Result<()> {
    // `resolve` may have answered from a cache, and a cache is a file that
    // travels with the projects. Check what the path names before handing it to
    // another program, exactly as `open` does.
    library::revalidate_for_read(project).with_context(|| {
        format!(
            "project '{}' cannot be copied at {}",
            project.id,
            paths::display_path(&project.path)
        )
    })?;

    let shown = paths::display_path(&project.path);
    announce(&shown);
    Ok(())
}

/// Say what happened, always.
///
/// There is no portable clipboard, so "no clipboard tool here" is an ordinary
/// answer: the path goes on its own line instead, which is what a terminal
/// selection wants anyway. A Copy that silently did nothing is the worst
/// possible version of this. The wording matches the TUI's Copy path.
fn announce(shown: &str) {
    let tool = clipboard::copy(shown);
    match tool {
        Some(tool) => println!("{}  Copied with {}", "✓".green().bold(), tool.dimmed()),
        None => {
            println!(
                "  {}",
                "no clipboard tool found — here is the path:".dimmed()
            );
            println!("{shown}");
        }
    }

    // Launched from a desktop launcher, every line above went to the journal.
    // The copy itself is the useful part and has already happened, so all that
    // is left is to say so where it can actually be seen. Best-effort: a system
    // without `notify-send` gets a silent success, which is still the thing the
    // user asked for.
    if crate::cli::terminal::headless_gui() {
        let summary = if tool.is_some() {
            "Path copied"
        } else {
            "fastf: no clipboard tool"
        };
        crate::cli::terminal::notify(summary, shown);
    }
}
