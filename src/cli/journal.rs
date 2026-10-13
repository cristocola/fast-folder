//! `fastf journal` — every dated note across every project, newest first,
//! and with `--todos` every todo's dated events among them.
//!
//! The notes are the work log, and a log that can only be read one project
//! at a time answers "what is this" and never "what happened last week". This
//! reads every project's notes, keeps the dated ones, and prints them as one
//! timeline; `--grep` is how a note written to be found later (`lesson: …`)
//! is found. A todo ticked is an event with a date of its own (`done:` on
//! its line), and `--todos` puts it on the timeline where it happened, which
//! is how a morning reads what the agents finished.
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

/// Which of the todos' events the timeline carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Todos {
    /// The notes alone.
    None,
    /// Every todo ticked, by its `done:` date.
    Done,
    /// Those, and every todo added, by its `added:` date.
    All,
}

pub struct JournalArgs {
    /// `None` = [`DEFAULT_LIMIT`].
    pub limit: Option<usize>,
    /// Only entries dated on or after this — the *entry's* date, never the
    /// project's.
    pub since: Option<String>,
    pub template: Option<String>,
    pub tag: Option<String>,
    pub base: Option<String>,
    /// A case-insensitive substring of the entry's text.
    pub grep: Option<String>,
    pub todos: Todos,
    pub json: bool,
}

/// What an entry is: a note, or a todo's event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Note,
    Done,
    Added,
}

impl Kind {
    /// The word a JSON row and the plain list say.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Note => "note",
            Kind::Done => "done",
            Kind::Added => "added",
        }
    }
}

/// One dated entry, with the project it belongs to.
pub struct Entry<'a> {
    pub project: &'a Project,
    pub timestamp: String,
    pub text: String,
    pub kind: Kind,
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
    // One filter for a note and a todo's event alike: dated on or after
    // `--since`, and saying what `--grep` looks for.
    let keep = |timestamp: &str, text: &str| {
        !args.since.as_deref().is_some_and(|since| timestamp < since)
            && !needle
                .as_deref()
                .is_some_and(|needle| !text.to_lowercase().contains(needle))
    };
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
            if !keep(&timestamp, &note.text) {
                continue;
            }
            entries.push(Entry {
                project,
                timestamp,
                text: note.text,
                kind: Kind::Note,
            });
        }
        if args.todos == Todos::None {
            continue;
        }
        let todos = crate::core::body::read_todos(&project.path).unwrap_or_default();
        for todo in todos {
            // The event's text names the owner, since who finished it is
            // what a morning asks.
            let text = match &todo.owner {
                Some(owner) => format!("{} @{owner}", todo.text),
                None => todo.text.clone(),
            };
            let mut events = Vec::new();
            if todo.done
                && let Some(done) = &todo.done_at
            {
                events.push((done.clone(), Kind::Done));
            }
            if args.todos == Todos::All
                && let Some(added) = &todo.added
            {
                events.push((added.clone(), Kind::Added));
            }
            for (timestamp, kind) in events {
                if keep(&timestamp, &text) {
                    entries.push(Entry {
                        project,
                        timestamp,
                        text: text.clone(),
                        kind,
                    });
                }
            }
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
                kind: entry.kind.word(),
            })
            .collect();
        return crate::cli::json::print(&rows);
    }

    let what = if args.todos == Todos::None {
        "notes"
    } else {
        "entries"
    };
    if entries.is_empty() {
        let filtered = args.since.is_some()
            || args.grep.is_some()
            || args.template.is_some()
            || args.tag.is_some()
            || args.base.is_some();
        println!(
            "{}",
            if projects.is_empty() {
                "No projects yet — create one with `fastf new`.".to_string()
            } else if filtered {
                format!("No {what} match those filters.")
            } else {
                "No notes yet — `fastf note add` writes the first.".to_string()
            }
            .dimmed()
        );
        return Ok(());
    }

    print_plain(&entries, args.todos != Todos::None);
    println!();
    let shown = entries.len();
    println!(
        "  {}",
        if shown < total {
            format!("{shown} of {total} {what} — -n {total} for every one")
        } else if args.todos == Todos::None {
            format!("{shown} note{}", crate::util::plural::s(shown))
        } else {
            format!(
                "{shown} {}",
                crate::util::plural::of(shown, "entry", "entries")
            )
        }
        .dimmed()
    );
    Ok(())
}

/// One entry per block: the day and the time, the project's id, the first
/// line; the rest of a note under its text. The id column is as wide as the
/// widest id shown, so the text lines up; with the todos' events among the
/// notes, a column before the text says which each is (`done`, `added`), and
/// a note's is blank.
fn print_plain(entries: &[Entry], with_kinds: bool) {
    let id_w = entries
        .iter()
        .map(|entry| entry.project.id.len())
        .max()
        .unwrap_or(0);
    let kind_w = if with_kinds { 7 } else { 0 };
    for entry in entries {
        let when = crate::util::time::local_readable(&entry.timestamp);
        // `local_readable` gives `YYYY-MM-DD HH:MM:SS`; the minute is enough
        // here, and a timestamp it could not parse is shown as written — in
        // the same sixteen columns, or a day written by hand (`done:2026-10-05`)
        // pulls its row's id out of line with the others.
        let when = format!("{:<16}", when.get(..16).unwrap_or(&when));
        let kind = match (with_kinds, entry.kind) {
            (false, _) => String::new(),
            (true, Kind::Note) => format!("{:<kind_w$}", ""),
            (true, kind) => format!("{:<kind_w$}", kind.word()),
        };
        let mut lines = entry.text.lines();
        println!(
            "  {} {}  {:<id_w$}  {}{}",
            "•".dimmed(),
            when.dimmed(),
            entry.project.id.green().bold(),
            kind.dimmed(),
            lines.next().unwrap_or("")
        );
        for line in lines {
            println!(
                "  {:<w$}  {line}",
                "",
                w = 2 + when.len() + 2 + id_w + kind_w
            );
        }
    }
}
