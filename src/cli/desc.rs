//! `fastf desc` — a project's one-line description: print it, set it, clear it.
//!
//! The line is read from the file, not the index: the index is what a list
//! shows, and a hand edit reaches it only when the project is opened or the
//! base reindexed. The write goes through `operations::set_description`, the
//! one door every surface uses, so the refusals are the same sentence here,
//! in the pane and in the wizard.

use anyhow::Result;
use colored::Colorize;

use crate::cli::tag::no_metadata_message;
use crate::core::validated::Description;
use crate::core::{config::Config, library, project_info};

pub struct DescArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// The new description; `None` prints the current one.
    pub text: Option<String>,
    /// Remove the description.
    pub clear: bool,
}

pub fn run(args: DescArgs) -> Result<()> {
    let cfg = Config::load()?;
    let candidate = library::resolve(&cfg, &args.query)?;
    let pinfo = project_info::pinfo_path(&candidate.path);
    if !pinfo.is_file() {
        anyhow::bail!(no_metadata_message(&candidate.id, &candidate.path));
    }

    if args.clear {
        crate::core::operations::set_description(&candidate, "")?;
        println!(
            "{}  Description cleared on {}",
            "✓".green().bold(),
            candidate.id.green().bold()
        );
        return Ok(());
    }

    match args.text {
        Some(text) => {
            let meta = crate::core::operations::set_description(&candidate, &text)?;
            if meta.description.is_empty() {
                println!(
                    "{}  Description cleared on {}",
                    "✓".green().bold(),
                    candidate.id.green().bold()
                );
            } else {
                println!(
                    "{}  Description set on {}",
                    "✓".green().bold(),
                    candidate.id.green().bold()
                );
                println!("    {}", meta.description);
            }
        }
        None => {
            let description = project_info::read_metadata(&candidate.path)?
                .map(|meta| Description::one_line(&meta.description))
                .unwrap_or_default();
            if description.is_empty() {
                println!("{}", "(no description)".dimmed());
            } else {
                println!("{description}");
            }
        }
    }
    Ok(())
}
