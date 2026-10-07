//! `fastf journal` — every dated note across every project, newest first.
//!
//! The notes are the work log, and a log that can only be read one project
//! at a time answers "what is this" and never "what happened last week". This
//! reads every project's notes, keeps the dated ones, and prints them as one
//! timeline; `--grep` is how a note written to be found later (`lesson: …`)
//! is found.
//!
//! It never goes interactive — a list of notes has no picker to offer — so
//! the only question the terminal decides is the launcher hand-off: from a
//! desktop launcher nothing printed is seen, and `recent` and `search`
//! answer that the same way, before any refusal.

use anyhow::Result;
use colored::Colorize;

use crate::cli::json::JournalEntryJson;
use crate::core::library::{self, Project};
use crate::core::{config::Config, project_info};

/// The default length of the timeline. Thirty is a few days of work in one
/// screen, and `-n` is there for more.
pub const DEFAULT_LIMIT: usize = 30;

pub struct JournalArgs {
    /// `None` = [`DEFAULT_LIMIT`].
    pub limit: Option<usize>,
    /// Only notes written on or after this date — the *note's* date, never
    /// the project's.
    pub since: Option<String>,
    pub template: Option<String>,
    pub tag: Option<String>,
    pub base: Option<String>,
    /// A case-insensitive substring of the note's text.
    pub grep: Option<String>,
    pub json: bool,
}

/// One dated note, with the project it belongs to.
pub struct Entry<'a> {
    pub project: &'a Project,
    pub timestamp: String,
    pub text: String,
}

pub fn run(args: JournalArgs) -> Result<()> {
    let cfg = Config::load()?;
    if crate::cli::terminal::hand_off_to_a_terminal(&cfg, args.json) {
        return Ok(());
    }

    // Every refusal is below the hand-off, as in `recent`: from a launcher
    // the message would go to a socket nobody reads.
    if args.limit == Some(0) {
        anyhow::bail!("--limit must be at least 1");
    }
    if let Some(since) = &args.since {
        crate::cli::recent::check_since(since)?;
    }
    crate::cli::recent::validate_scope(&cfg, args.template.as_deref(), args.base.as_deref())?;
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).max(1);

    let projects = library::discover(&cfg);
    // The project filters, and none of the note's: `--since` is about the
    // note, and a new note on an old project is exactly what a timeline is
    // for.
    let scoped = crate::cli::recent::filter_projects(
        &projects,
        &args.template,
        &None,
        &args.tag,
        &args.base,
        usize::MAX,
    );

    let needle = args.grep.as_deref().map(str::to_lowercase);
    let mut entries: Vec<Entry> = Vec::new();
    for project in scoped {
        // A file that will not read is named once by the warning discovery
        // already gives; here it is a project with no notes to show.
        let notes = project_info::read_journal_entries(&project.path).unwrap_or_default();
        for note in notes {
            if !note.is_dated() {
                continue;
            }
            let Some(timestamp) = note.timestamp else {
                continue;
            };
            if args
                .since
                .as_deref()
                .is_some_and(|since| timestamp.as_str() < since)
            {
                continue;
            }
            if needle
                .as_deref()
                .is_some_and(|needle| !note.text.to_lowercase().contains(needle))
            {
                continue;
            }
            entries.push(Entry {
                project,
                timestamp,
                text: note.text,
            });
        }
    }
    // Newest first; the stamps fastf writes are fixed-width UTC and sort as
    // text, and a hand-written date without a time sorts among its day.
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    let total = entries.len();
    entries.truncate(limit);

    if args.json {
        let rows: Vec<JournalEntryJson> = entries
            .iter()
            .map(|entry| JournalEntryJson {
                id: entry.project.id.clone(),
                name: entry.project.name.clone(),
                path: crate::util::paths::display_path(&entry.project.path),
                timestamp: entry.timestamp.clone(),
                text: entry.text.clone(),
            })
            .collect();
        return crate::cli::json::print(&rows);
    }

    if entries.is_empty() {
        let filtered = args.since.is_some()
            || args.grep.is_some()
            || args.template.is_some()
            || args.tag.is_some()
            || args.base.is_some();
        println!(
            "{}",
            if projects.is_empty() {
                "No projects yet — create one with `fastf new`."
            } else if filtered {
                "No notes match those filters."
            } else {
                "No notes yet — `fastf note add` writes the first."
            }
            .dimmed()
        );
        return Ok(());
    }

    print_plain(&entries);
    println!();
    let shown = entries.len();
    println!(
        "  {}",
        if shown < total {
            format!("{shown} of {total} notes — -n {total} for every one")
        } else {
            format!("{shown} note{}", crate::util::plural::s(shown))
        }
        .dimmed()
    );
    Ok(())
}

/// One note per block: the day and the time, the project's id, the first
/// line; the rest of the note under its text. The id column is as wide as the
/// widest id shown, so the text lines up.
fn print_plain(entries: &[Entry]) {
    let id_w = entries
        .iter()
        .map(|entry| entry.project.id.len())
        .max()
        .unwrap_or(0);
    for entry in entries {
        let when = crate::util::time::local_readable(&entry.timestamp);
        // `local_readable` gives `YYYY-MM-DD HH:MM:SS`; the minute is enough
        // here, and a timestamp it could not parse is shown as written.
        let when = when.get(..16).unwrap_or(&when).to_string();
        let mut lines = entry.text.lines();
        println!(
            "  {} {}  {:<id_w$}  {}",
            "•".dimmed(),
            when.dimmed(),
            entry.project.id.green().bold(),
            lines.next().unwrap_or("")
        );
        for line in lines {
            println!("  {:<w$}  {line}", "", w = 2 + when.len() + 2 + id_w);
        }
    }
}
