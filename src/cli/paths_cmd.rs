//! `fastf paths` — show where fastf keeps its data and how that was decided.
//! (Module can't be named `paths` inside cli/ without shadowing util::paths
//! in imports; `paths_cmd` keeps call sites unambiguous.)

use anyhow::Result;
use colored::Colorize;

use crate::util::paths;

pub fn run() -> Result<()> {
    let (dir, mode) = paths::try_install_dir()?;

    println!("{}", "fastf data locations:".bold());
    println!("  {:<16} {}", "Data dir:".dimmed(), dir.display());
    println!("  {:<16} {}", "Resolved via:".dimmed(), mode.label());
    println!();
    println!(
        "  {:<16} {}",
        "Config:".green(),
        paths::config_path().display()
    );
    println!(
        "  {:<16} {}",
        "Counters:".green(),
        paths::counters_path().display()
    );
    println!(
        "  {:<16} {}",
        "Templates:".green(),
        paths::templates_dir().display()
    );
    Ok(())
}
