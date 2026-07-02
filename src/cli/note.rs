//! `fastf note add` and `fastf notes` — per-project journal.
//!
//! Entries are timestamped lines in the `## Journal` section of
//! `PROJECT_INFO.md`.  They are append-only — fastf never edits or deletes
//! existing entries.
//!
//! # Adding entries
//! ```bash
//! fastf note add ID0047 "finished final mix"    # inline message
//! fastf note add ID0047 -                        # read from stdin
//! fastf note add ID0047                          # open $EDITOR
//! ```
//!
//! # Viewing entries
//! ```bash
//! fastf notes ID0047                             # all entries
//! fastf notes ID0047 --since 2026-04-01          # entries on/after a date
//! ```

use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::io::{self, Read};

use crate::core::library;
use crate::core::{config::Config, project_info};

// ---------------------------------------------------------------------------
// Add a journal entry
// ---------------------------------------------------------------------------

pub struct NoteAddArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// Inline message text, `-` to read from stdin, or `None` to open $EDITOR.
    pub message: Option<String>,
}

pub fn add(args: NoteAddArgs) -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let project = library::resolve(&cfg, &args.query)?;
    let pinfo = project_info::pinfo_path(&project.path);

    if !pinfo.exists() {
        bail!(
            "no {} found for project {}",
            project_info::RESERVED_FILENAME,
            project.id
        );
    }

    let message = resolve_message(args.message.as_deref(), &cfg.editor)?;
    let message = message.trim().to_string();

    if message.is_empty() {
        bail!("journal entry is empty — nothing written");
    }

    project_info::append_journal_entry(&pinfo, &message)
        .with_context(|| format!("appending journal entry to {}", pinfo.display()))?;

    println!(
        "{}  Journal entry added to {}",
        "✓".green().bold(),
        project.id.green().bold()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// List / view journal entries
// ---------------------------------------------------------------------------

pub struct NotesArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// Only show entries on or after this ISO-8601 date prefix (e.g. `2026-04-01`).
    pub since: Option<String>,
}

pub fn notes(args: NotesArgs) -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let project = library::resolve(&cfg, &args.query)?;

    let entries = project_info::read_journal_entries(&project.path)?;

    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        project.id.green().bold(),
        project.name.bold()
    );

    let filtered: Vec<_> = entries
        .iter()
        .filter(|e| {
            if let Some(since) = &args.since {
                e.timestamp.as_str() >= since.as_str()
            } else {
                true
            }
        })
        .collect();

    if filtered.is_empty() {
        if args.since.is_some() {
            println!("    {}", "(no entries since that date)".dimmed());
        } else {
            println!(
                "    {}",
                "(no journal entries yet — use `fastf note add` to add one)".dimmed()
            );
        }
        return Ok(());
    }

    println!();
    for entry in &filtered {
        let date = &entry.timestamp[..entry.timestamp.len().min(10)]; // YYYY-MM-DD
        println!("  {} {}  {}", "•".dimmed(), date.dimmed(), entry.message);
    }
    println!();
    println!(
        "  {}",
        format!(
            "{} entr{}",
            filtered.len(),
            if filtered.len() == 1 { "y" } else { "ies" }
        )
        .dimmed()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the message text from the three input modes.
fn resolve_message(raw: Option<&str>, editor: &str) -> Result<String> {
    match raw {
        // stdin sentinel
        Some("-") => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .context("reading from stdin")?;
            Ok(buf)
        }
        // inline text
        Some(text) => Ok(text.to_string()),
        // open editor
        None => open_in_editor(editor),
    }
}

/// Open the configured editor and return what the user wrote.
fn open_in_editor(editor: &str) -> Result<String> {
    use std::fs;

    let tmp = tempfile_path();
    // Write a prompt comment so the editor opens with some context.
    fs::write(
        &tmp,
        "# Enter your journal note. Lines starting with # are ignored.\n",
    )
    .context("writing editor temp file")?;

    let status = std::process::Command::new(editor)
        .arg(&tmp)
        .status()
        .with_context(|| format!("launching editor '{}'", editor))?;

    if !status.success() {
        bail!("editor exited with non-zero status");
    }

    let raw = fs::read_to_string(&tmp).context("reading editor temp file")?;
    let _ = fs::remove_file(&tmp);

    // Strip comment lines
    let message: String = raw
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(message)
}

fn tempfile_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("fastf-note-{}.txt", std::process::id()))
}
