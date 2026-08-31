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
    let project = library::resolve(&cfg, query)?;
    print_path(&project)
}

/// Revalidate, then print the bare line — the shared tail with the picker path.
pub(crate) fn print_path(project: &library::Project) -> Result<()> {
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
    println!("{}", paths::display_path(&project.path));
    Ok(())
}
