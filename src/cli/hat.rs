//! `fastf hat` — the hat worn: the owner whose court is yours.
//!
//! ```bash
//! fastf hat                 # which hat, if any
//! fastf hat sales           # wear one: @sales
//! fastf hat --off           # wear none
//! ```
//!
//! The hat is the `hat` configuration key, so `config show` prints it and the
//! settings screen has its row; this is the short way to change it. `FASTF_HAT`
//! wears one for a shell without writing anything, and wins while it is set.

use anyhow::Result;
use colored::Colorize;

use crate::core::config::Config;

pub struct HatArgs {
    /// The hat to wear, with or without its `@`. `None` prints the one worn.
    pub name: Option<String>,
    /// Take the hat off.
    pub off: bool,
}

pub fn run(args: HatArgs) -> Result<()> {
    if args.off {
        let said = crate::cli::config::set_quietly("hat", "")?;
        println!("{}  {said}", "✓".green().bold());
        return Ok(());
    }
    match args.name {
        Some(name) => {
            let said = crate::cli::config::set_quietly("hat", &name)?;
            println!("{}  {said}", "✓".green().bold());
        }
        None => {
            let cfg = Config::load()?;
            match cfg.resolve_hat() {
                Some(hat) if std::env::var_os("FASTF_HAT").is_some() => {
                    println!("@{}  {}", hat.bold(), "(from $FASTF_HAT)".dimmed());
                }
                Some(hat) => println!("@{}", hat.bold()),
                None => println!("{}", "no hat — `fastf hat <name>` wears one".dimmed()),
            }
        }
    }
    Ok(())
}
