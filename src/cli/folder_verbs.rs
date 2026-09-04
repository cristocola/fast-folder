//! `fastf rename`, `fastf unregister`, `fastf delete` — the three verbs on a
//! project's folder that the guided app had and the command line did not.
//!
//! Each resolves a query the way `open`/`copy`/`path` do (an ambiguous one
//! gets the picker), asks its question the way the app asks it — a name to
//! type, a yes/no, the word `delete` — and takes `--yes` for a script. The
//! engine underneath is `core::operations`, the same calls the app makes.

use anyhow::Result;
use colored::Colorize;

use crate::cli::target::{self, Target};
use crate::core::config::Config;
use crate::core::library::Project;
use crate::core::operations;
use crate::tui::prompt;
use crate::tui::validators;
use crate::util::paths::display_path;

/// Resolve `query` to one project, or say why there is none to act on.
fn one(query: &str, verb: &str, prompt: &str, nothing: &str) -> Result<Option<Project>> {
    let cfg = Config::load()?;
    match target::one_project(&cfg, query, prompt, &target::full_id_hint(verb))? {
        Target::Project(project) => Ok(Some(*project)),
        Target::Cancelled => {
            prompt::report_cancelled(nothing);
            Ok(None)
        }
        // A terminal is running this again; it will ask.
        Target::HandedOff => Ok(None),
    }
}

/// `fastf rename <query> [name]`: the folder's new name, asked for when it is
/// not given, checked the way the app checks it.
pub fn rename(query: &str, name: Option<String>, yes: bool) -> Result<()> {
    let Some(project) = one(
        query,
        "rename",
        "Which project to rename?",
        "nothing was renamed",
    )?
    else {
        return Ok(());
    };
    let name = match name {
        Some(name) => {
            validators::folder_name(&name).map_err(|error| anyhow::anyhow!(error))?;
            name
        }
        None => {
            crate::util::tty::require_tty(
                "ask for the new name",
                "give it as an argument: `fastf rename <query> <name>`",
            )?;
            let answer = prompt::text(
                validators::RENAME_PROMPT,
                prompt::TextOpts {
                    initial: Some(project.name.clone()),
                    default: None,
                    allow_empty: false,
                    validator: Some(Box::new(validators::folder_name)),
                },
            )?;
            match answer {
                Some(name) => name,
                None => {
                    prompt::report_cancelled("nothing was renamed");
                    return Ok(());
                }
            }
        }
    };
    if name == project.name {
        println!("{}", "Already called that — nothing to do.".dimmed());
        return Ok(());
    }
    if !yes && !confirm(&format!("Rename '{}' to '{name}'?", project.name))? {
        prompt::report_cancelled("nothing was renamed");
        return Ok(());
    }
    let renamed = operations::rename(&project, &name)?;
    println!(
        "{} Renamed {} to {}",
        "✓".green(),
        project.id,
        display_path(&renamed.path)
    );
    Ok(())
}

/// `fastf unregister <query>`: forget the project, keep the files.
pub fn unregister(query: &str, yes: bool) -> Result<()> {
    let Some(project) = one(
        query,
        "unregister",
        "Which project to unregister?",
        "nothing was unregistered",
    )?
    else {
        return Ok(());
    };
    if !yes
        && !confirm(&validators::unregister_prompt(std::slice::from_ref(
            &project.name,
        )))?
    {
        prompt::report_cancelled("nothing was unregistered");
        return Ok(());
    }
    operations::unregister(&project)?;
    println!(
        "{} Unregistered {} — the files stay at {}",
        "✓".green(),
        project.id,
        display_path(&project.path)
    );
    Ok(())
}

/// `fastf delete <query>`: the folder and everything inside it, after the
/// word `delete` — the same confirmation the app asks for.
pub fn delete(query: &str, yes: bool) -> Result<()> {
    let Some(project) = one(
        query,
        "delete",
        "Which project to delete?",
        "nothing was deleted",
    )?
    else {
        return Ok(());
    };
    if !yes {
        crate::util::tty::require_tty("confirm", "pass --yes to delete without confirming")?;
        let typed = prompt::text(
            &validators::delete_prompt(std::slice::from_ref(&project.name)),
            prompt::TextOpts {
                initial: None,
                default: None,
                allow_empty: true,
                validator: None,
            },
        )?;
        let confirmed = typed
            .as_deref()
            .is_some_and(|word| word.trim().eq_ignore_ascii_case(validators::DELETE_WORD));
        if !confirmed {
            println!("{}", validators::DELETE_MISMATCH.dimmed());
            return Ok(());
        }
    }
    let path = display_path(&project.path);
    operations::delete(&project)?;
    println!("{} Deleted {} ({})", "✓".green(), project.id, path);
    Ok(())
}

/// A yes/no that needs a terminal, and says which flag answers it otherwise.
fn confirm(question: &str) -> Result<bool> {
    crate::util::tty::require_tty("confirm", "pass --yes to skip the question")?;
    Ok(prompt::confirm(question, false)?.unwrap_or(false))
}
