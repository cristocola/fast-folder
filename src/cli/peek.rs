//! `fastf peek <query>`: a short summary of one project, without opening
//! its folder.
//!
//! It prints the next open todo and its owner, the todo counts,
//! which programs are working in the folder right now, the newest file under
//! it, the repository's head when the folder is one, the last note, and how
//! long the project has been quiet. `fastf show` prints everything fastf
//! knows about the project; `peek` prints what changed and what is next.

use std::sync::atomic::AtomicBool;

use anyhow::Result;
use colored::Colorize;

use crate::core::config::Config;
use crate::core::desk::{self, Card};
use crate::core::library;
use crate::core::project_info;

pub fn run(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, query)?;
    if !project_info::pinfo_path(&project.path).exists() {
        anyhow::bail!(
            "{}",
            crate::cli::tag::no_metadata_message(&project.id, &project.path)
        );
    }
    let now = crate::util::time::now_iso8601();
    let todos = crate::core::body::read_todos(&project.path).unwrap_or_default();
    let notes = project_info::read_journal_entries(&project.path).unwrap_or_default();
    let description = project_info::read_metadata(&project.path)
        .ok()
        .flatten()
        .map(|meta| crate::core::validated::Description::one_line(&meta.description))
        .unwrap_or_else(|| project.description.clone());
    // Asked only where the platform can answer: elsewhere `nobody` would be
    // a guess, and the row is left out.
    let working: Option<Vec<String>> = crate::core::holders::WORKING_KNOWN.then(|| {
        crate::core::holders::in_tree(&project.path)
            .working_in
            .iter()
            .map(|holder| holder.who())
            .collect()
    });
    let facts =
        crate::util::tree_size::directory_facts_until(&project.path, &AtomicBool::new(false));
    let head = crate::core::glance::git_head(&project.path);
    let touched = library::touched(&project);
    // Quiet since the newest file under the folder when the walk found one
    // newer than the top level, since an edit deeper in the folder is work
    // too.
    let quiet_since = match (&touched, facts.as_ref().and_then(|f| f.newest_stamp())) {
        (Some(a), Some(b)) => Some(a.clone().max(b)),
        (Some(a), None) => Some(a.clone()),
        (None, b) => b,
    };
    let card = Card::of(project.clone(), &todos, Vec::new(), quiet_since.clone());

    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        project.id.green().bold(),
        project.name.bold()
    );
    println!(
        "    {}",
        crate::util::paths::display_path(&project.path).dimmed()
    );
    if !description.is_empty() {
        println!("    {description}");
    }
    println!();
    let field = |label: &str, value: String| println!("  {:<11} {value}", label.dimmed());

    let assignee = card.assignee();
    match &card.next_todo {
        Some(next) => {
            let mut line = format!(
                "{}  {}",
                card.next_todo_line(" · "),
                format!("({})", assignee.label(cfg.resolve_hat().as_deref())).dimmed()
            );
            if let Some(day) = next.todo.added_day() {
                line.push_str(&format!("  {}", format!("added {day}").dimmed()));
            }
            line.push_str(&format!(
                "  {}",
                format!("todo {}", next.index + 1).dimmed()
            ));
            field("next todo", line);
        }
        None => field("next todo", "no open todos".dimmed().to_string()),
    }
    field(
        "todos",
        format!(
            "{} open · {} done{}",
            card.open,
            card.done,
            owners_line(&todos)
        ),
    );
    if let Some(working) = &working {
        field(
            "working",
            if working.is_empty() {
                "nobody".dimmed().to_string()
            } else {
                working.join(", ")
            },
        );
    }
    field(
        "last moved",
        match facts.as_ref().and_then(|f| f.newest.as_ref()) {
            Some((at, path)) => format!(
                "{}  {}",
                crate::util::paths::display_path(path),
                crate::util::time::iso8601_of(*at).dimmed()
            ),
            None => "nothing read".dimmed().to_string(),
        },
    );
    if let Some(head) = head {
        field(
            "head",
            match &head.when {
                Some(when) => format!("{}  {}", head.line(), when.dimmed()),
                None => head.line(),
            },
        );
    }
    if let Some(note) = notes.iter().rev().find(|note| note.is_dated()) {
        field(
            "last note",
            format!(
                "{}  {}",
                note.day().unwrap_or("").dimmed(),
                note.text.lines().next().unwrap_or("")
            ),
        );
    }
    match &quiet_since {
        Some(since) => field(
            "quiet for",
            format!(
                "{}  {}",
                desk::age_label(since, &now),
                format!("since {since}").dimmed()
            ),
        ),
        None => field("quiet for", "unknown".dimmed().to_string()),
    }
    Ok(())
}

/// ` · @claude 2 · no owner 1`: how many open todos each owner has, after
/// the counts.
fn owners_line(todos: &[crate::core::body::Todo]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for todo in todos.iter().filter(|t| !t.done) {
        let key = match &todo.owner {
            Some(owner) => format!("@{owner}"),
            None => "no owner".to_string(),
        };
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => counts.push((key, 1)),
        }
    }
    counts
        .iter()
        .map(|(key, n)| format!(" · {key} {n}"))
        .collect()
}
