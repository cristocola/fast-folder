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
    // Two counter locations, and the base one is the record — saying only
    // "Counters: <data dir>" made the backup input look authoritative.
    println!(
        "  {:<16} {}",
        "Counter:".green(),
        paths::counters_path().display()
    );
    println!(
        "  {:<16} {}",
        "".dimmed(),
        "this machine's copy — each base also carries .fastf-counter.toml,".dimmed()
    );
    println!(
        "  {:<16} {}",
        "".dimmed(),
        "which is the number both operating systems read (`fastf id show`)".dimmed()
    );
    println!(
        "  {:<16} {}",
        "Templates:".green(),
        paths::templates_dir().display()
    );
    Ok(())
}
