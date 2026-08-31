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
    let project = library::resolve(&cfg, query)?;
    report_copy(&project)
}

/// Revalidate, then copy — the shared tail, so `copy` and a picker-driven copy
/// check the same things and say the same words.
pub(crate) fn report_copy(project: &library::Project) -> Result<()> {
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
pub(crate) fn announce(shown: &str) {
    match clipboard::copy(shown) {
        Some(tool) => println!("{}  Copied with {}", "✓".green().bold(), tool.dimmed()),
        None => {
            println!(
                "  {}",
                "no clipboard tool found — here is the path:".dimmed()
            );
            println!("{shown}");
        }
    }
}
