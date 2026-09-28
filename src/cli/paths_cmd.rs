//! `fastf paths` — show where fastf keeps its data and how that was decided.
//! (Module can't be named `paths` inside cli/ without shadowing util::paths
//! in imports; `paths_cmd` keeps call sites unambiguous.)

use anyhow::Result;
use colored::Colorize;

use crate::util::paths;

pub fn run() -> Result<()> {
    let (dir, mode) = paths::try_install_dir()?;

    println!("{}", "fastf data locations:".bold());
    println!(
        "  {:<16} {}",
        "Data dir:".dimmed(),
        crate::util::paths::display_path(&dir)
    );
    println!("  {:<16} {}", "Resolved via:".dimmed(), mode.label());
    println!();
    println!(
        "  {:<16} {}",
        "Config:".green(),
        crate::util::paths::display_path(&paths::config_path())
    );
    // Two counter locations, and the base one is the record — naming only the
    // data dir's would make the backup input look authoritative.
    println!(
        "  {:<16} {}",
        "Counter:".green(),
        crate::util::paths::display_path(&paths::counters_path())
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
        crate::util::paths::display_path(&paths::templates_dir())
    );
    Ok(())
}
