//! `fastf reconcile` — recover scoped v2 work and report obsolete v1 markers.
//!
//! Version-2 create journals contain validated relative paths, while move
//! transactions live below the configured target base and derive every owned
//! path from that location. The core reconciler can therefore resume or discard
//! only scoped work after verifying its identity and state.
//!
//! Version-1 markers contain arbitrary absolute paths, so this command never
//! parses them, follows them, copies through them, or deletes anything they name.
//! It reports their own paths for manual inspection and leaves all bytes alone.
//! Reconciliation is explicit, idempotent, and also available through the UI.

use anyhow::Result;
use colored::Colorize;

use crate::core::config::Config;

/// `fastf reconcile --list`: what is unfinished, by who finishes it, and
/// for what needs you, the commands that settle it. Changes nothing.
pub fn list() -> Result<()> {
    use crate::core::attention::State;
    let cfg = Config::load()?;
    let attention = crate::core::attention::attention(&cfg);
    if attention.items.is_empty() {
        println!("{}  Nothing unfinished.", "✓".green().bold());
        return Ok(());
    }
    let groups = [
        (State::NeedsYou, "needs you".yellow().bold()),
        (State::Auto, "fastf finishes".bold()),
        (State::Waiting, "waiting".bold()),
    ];
    for (state, title) in groups {
        let items: Vec<_> = attention
            .items
            .iter()
            .filter(|item| item.state == state)
            .collect();
        if items.is_empty() {
            continue;
        }
        println!("{title} ({})", items.len());
        for item in items {
            let shown = crate::util::paths::display_path(&item.path);
            match &item.project {
                Some(project) => println!("  {} — {} {}", item.what, project, shown.dimmed()),
                None => println!("  {} — {}", item.what, shown.dimmed()),
            }
            println!("    {}", item.reason);
            for action in &item.actions {
                println!(
                    "    {}  {}",
                    format!(
                        "fastf reconcile --resolve {} {}",
                        shell_quoted(&shown),
                        action.word()
                    )
                    .cyan(),
                    action.label().dimmed()
                );
            }
        }
    }
    if attention.auto() > 0 {
        println!(
            "{}",
            "`fastf reconcile` finishes what fastf can; the app starts one by itself.".dimmed()
        );
    }
    Ok(())
}

/// `fastf reconcile --resolve <path> <action>`: settle one item that needs
/// you, as you chose. Discarding asks for the word first.
pub fn resolve(path: &str, action: &str, yes: bool) -> Result<()> {
    use crate::core::attention::Action;
    let Some(action) = Action::from_word(action) else {
        anyhow::bail!(
            "'{action}' is not an action; one of: keep-moved, take-old, put-back, discard, finish"
        );
    };
    let path = std::path::PathBuf::from(path);
    if action == Action::Discard && !yes {
        crate::util::tty::require_tty("confirm", "pass --yes to discard without confirming")?;
        let typed = crate::tui::prompt::text(
            &format!(
                "Discard {}? It goes for good. Type discard to confirm",
                crate::util::paths::display_path(&path)
            ),
            crate::tui::prompt::TextOpts {
                initial: None,
                default: None,
                allow_empty: true,
                validator: None,
            },
        )?;
        if !typed
            .as_deref()
            .is_some_and(|word| word.trim().eq_ignore_ascii_case("discard"))
        {
            println!("{}", "not discarded: the word did not match".dimmed());
            return Ok(());
        }
    }
    let said = crate::core::operations::resolve_attention(&path, action)?;
    println!("{}  {said}", "✓".green().bold());
    Ok(())
}

/// A path as a shell reads it back, when it needs quoting.
fn shell_quoted(text: &str) -> String {
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-~:".contains(c))
    {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

pub fn run(detach: bool) -> Result<()> {
    // A job of its own, like a move: its removals on a cloud mount can take
    // minutes, and closing this terminal must not stop one part of the way.
    Config::load()?;
    let state =
        match crate::cli::jobs::start(crate::core::jobs::JobKind::Reconcile, Vec::new(), detach)? {
            crate::cli::jobs::Followed::Ended(state) => state,
            _ => return Ok(()),
        };
    let Some(report) = state.reconcile.clone() else {
        anyhow::bail!("{}", state.summary);
    };

    if report.is_empty() {
        println!(
            "{}  Nothing to reconcile — all projects fully provisioned.",
            "✓".green().bold()
        );
        return Ok(());
    }

    // Not a green tick over a report that may be nothing but "could not
    // inspect": the tick means something worked, and here it only means the
    // pass ran.
    let clean = report.unrecoverable.is_empty() && report.leftovers.is_empty() && !report.cancelled;
    if report.cancelled {
        println!(
            "{}  Reconcile stopped when asked. What it finished is below; what it had \
             not reached is as it was — run `fastf reconcile` again to finish it.",
            "⚠".yellow().bold()
        );
    }
    println!(
        "{}  Reconcile report complete.",
        if clean {
            "✓".green().bold()
        } else {
            "⚠".yellow().bold()
        }
    );
    if report.resumed > 0 {
        println!(
            "   {} {} interrupted copy job(s) finished",
            "resumed".dimmed(),
            report.resumed
        );
    }
    if report.completed > 0 {
        println!(
            "   {} {} move(s) finished (original removed)",
            "completed".dimmed(),
            report.completed
        );
    }
    if report.cleared > 0 {
        println!(
            "   {} {} old folder(s) of deleted projects removed",
            "cleared".dimmed(),
            report.cleared
        );
    }
    if report.restored > 0 {
        println!(
            "   {} {} project(s) put back where an interrupted rename was taking them",
            "restored".dimmed(),
            report.restored
        );
    }
    if report.rolled_back > 0 {
        println!(
            "   {} {} uncommitted move(s) — source left intact",
            "rolled back".dimmed(),
            report.rolled_back
        );
    }
    if !report.incomplete.is_empty() {
        println!(
            "   {} {} project(s) were never finished being created:",
            "incomplete".yellow().bold(),
            report.incomplete.len()
        );
        for item in &report.incomplete {
            println!("     - {}", item.yellow());
        }
        println!(
            "     {}",
            "These cannot be rebuilt automatically (the values you typed are gone). \
             Delete the folder and run `fastf new` again."
                .dimmed()
        );
    }
    if !report.obsolete.is_empty() {
        println!(
            "   {} {} pre-v2 marker(s) were left untouched:",
            "obsolete".yellow().bold(),
            report.obsolete.len()
        );
        for item in &report.obsolete {
            println!("     - {}", item.yellow());
        }
        println!(
            "     {}",
            "Inspect the source and destination yourself. Remove a marker only after \
             you have confirmed which copy is authoritative."
                .dimmed()
        );
    }
    if !report.leftovers.is_empty() {
        println!(
            "   {} {} old folder(s) not removed yet:",
            "leftover".yellow().bold(),
            report.leftovers.len()
        );
        for item in &report.leftovers {
            println!("     - {}", item.yellow());
        }
        println!(
            "     {}",
            "Hidden folders that projects which have left the library left behind. \
             Each line says whether fastf can remove it."
                .dimmed()
        );
    }
    if !report.waiting.is_empty() {
        println!(
            "   {} {} item(s) waiting for a base to answer:",
            "waiting".cyan().bold(),
            report.waiting.len()
        );
        for item in &report.waiting {
            println!("     - {}", item.cyan());
        }
        println!(
            "     {}",
            "fastf changed nothing about these and finishes them once the base answers \
             again: mount it, or wait for the connection to come back."
                .dimmed()
        );
    }
    if !report.unrecoverable.is_empty() {
        println!(
            "   {} {} item(s) need a look:",
            "attention".yellow().bold(),
            report.unrecoverable.len()
        );
        for item in &report.unrecoverable {
            println!("     - {}", item.yellow());
        }
        // The other two blocks each end with a sentence saying what to do.
        // This one — the category the user can do least about — ended with the
        // bare list.
        println!(
            "     {}",
            "Each line says what is on disk and what fastf left alone. Where it names a \
             step, that step finishes it; otherwise look at the paths yourself."
                .dimmed()
        );
    }
    Ok(())
}
