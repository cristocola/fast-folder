//! `fastf hat`: print or set the hat, the owner name the desk and
//! `todo add --mine` treat as yours.
//!
//! ```bash
//! fastf hat                 # print the hat, if one is set
//! fastf hat sales           # set it to @sales
//! fastf hat --off           # unset it
//! ```
//!
//! The hat is the `hat` configuration key, so `config show` prints it and the
//! settings screen has its row; this is the short way to change it. `FASTF_HAT`
//! sets it for one shell without writing the config file, and wins while it
//! is set.

use anyhow::Result;
use colored::Colorize;

use crate::core::config::Config;

pub struct HatArgs {
    /// The owner name to set, with or without its `@`. `None` prints the
    /// current hat.
    pub name: Option<String>,
    /// Unset the hat.
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
                None => println!("{}", "no hat is set: `fastf hat <name>` sets one".dimmed()),
            }
        }
    }
    Ok(())
}
