//! `fastf reconcile` — resume interrupted background copies and finish/roll back
//! interrupted staged moves.
//!
//! Large asset copies during `fastf new` (UI) and cross-filesystem moves leave a
//! durable marker so a crash mid-copy is never silent data loss. This command
//! (also run automatically when `fastf ui` launches) walks every base, resumes
//! pending create copies, and either finishes an already-committed move's source
//! removal or rolls back an uncommitted one — always leaving the source intact
//! when nothing was verified.

use anyhow::Result;
use colored::Colorize;

use crate::core::config::Config;
use crate::core::provisioning;

pub fn run() -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let report = provisioning::reconcile(&cfg);

    if report.is_empty() {
        println!(
            "{}  Nothing to reconcile — all projects fully provisioned.",
            "✓".green().bold()
        );
        return Ok(());
    }

    println!("{}  Reconcile complete.", "✓".green().bold());
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
    if !report.unrecoverable.is_empty() {
        println!(
            "   {} {} item(s) could not be recovered:",
            "unrecoverable".yellow().bold(),
            report.unrecoverable.len()
        );
        for item in &report.unrecoverable {
            println!("     - {}", item.yellow());
        }
    }
    Ok(())
}
