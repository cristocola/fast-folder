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

pub fn run() -> Result<()> {
    let report = crate::core::operations::reconcile()?;

    if report.is_empty() {
        println!(
            "{}  Nothing to reconcile — all projects fully provisioned.",
            "✓".green().bold()
        );
        return Ok(());
    }

    println!("{}  Reconcile report complete.", "✓".green().bold());
    if report.resumed > 0 {
        println!(
            "   {} {} interrupted copy job(s) finished",
            "resumed".dimmed(),
            report.resumed
        );
    }
    if report.completed > 0 {
        println!(
            "   {} {} move(s) committed (source removed)",
            "completed".dimmed(),
            report.completed
        );
    }
    if report.rolled_back > 0 {
        println!(
            "   {} {} uncommitted move(s) — source left intact",
            "rolled back".dimmed(),
            report.rolled_back
        );
    }
    if report.swept > 0 {
        println!(
            "   {} {} abandoned temporary file(s) removed",
            "swept".dimmed(),
            report.swept
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
    if !report.unrecoverable.is_empty() {
        println!(
            "   {} {} item(s) could not be inspected:",
            "unrecoverable".yellow().bold(),
            report.unrecoverable.len()
        );
        for item in &report.unrecoverable {
            println!("     - {}", item.yellow());
        }
    }
    Ok(())
}
