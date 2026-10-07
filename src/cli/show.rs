//! `fastf show` — one project, whole.
//!
//! What `recent` and `search` print per row, plus everything only the file
//! knows: the template variables, the notes and the task list. `--json` for a
//! script; without it, a compact summary a person can read.

use anyhow::Result;
use colored::Colorize;

use crate::cli::json::{NoteJson, ProjectDetailJson, ProjectJson, TodoJson};
use crate::core::{body, config::Config, library, project_info};

pub struct ShowArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// Print JSON instead of the summary.
    pub json: bool,
}

pub fn run(args: ShowArgs) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, &args.query)?;

    // A folder fastf discovered has a `PROJECT_INFO.md`, but it may have been
    // edited since; everything below it is best-effort so `show` never fails
    // on a file it can still partly read.
    let meta = project_info::read_metadata(&project.path).unwrap_or(None);
    let notes = project_info::read_journal_entries(&project.path).unwrap_or_default();
    let todos = body::read_todos(&project.path).unwrap_or_default();
    // The file's description, not the index's: the index is a hint, and an
    // older fastf may have rewritten it without the field.
    let description = meta
        .as_ref()
        .map(|m| crate::core::validated::Description::one_line(&m.description))
        .unwrap_or_else(|| project.description.clone());
    let variables = meta.map(|m| m.variables).unwrap_or_default();
    let last_note = notes
        .iter()
        .filter(|note| note.is_dated())
        .filter_map(|note| note.timestamp.clone())
        .max();

    if args.json {
        let mut row = ProjectJson::of(&project);
        row.description = description;
        return crate::cli::json::print(&ProjectDetailJson {
            project: row,
            last_note,
            variables,
            notes: notes
                .iter()
                .map(|n| NoteJson {
                    timestamp: n.timestamp.clone(),
                    text: n.text.clone(),
                })
                .collect(),
            todos: todos
                .iter()
                .map(|t| TodoJson {
                    done: t.done,
                    text: t.text.clone(),
                    phase: t.phase.clone(),
                })
                .collect(),
        });
    }

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
    let field = |label: &str, value: String| println!("  {:<10} {value}", label.dimmed());
    field("template", project.template.clone());
    field("base", library::base_label(&project.base));
    field("created", project.created.clone());
    if let Some(touched) = library::touched(&project) {
        field("touched", touched);
    }
    if let Some(last_note) = &last_note {
        field("last note", last_note.clone());
    }
    if !project.tags.is_empty() {
        field("tags", project.tags.join(", "));
    }
    for (slug, value) in &variables {
        field(slug, value.clone());
    }
    let done = todos.iter().filter(|t| t.done).count();
    println!();
    println!(
        "  {}",
        format!(
            "{} note{}   ·   {done}/{} todo{} done",
            notes.len(),
            crate::util::plural::s(notes.len()),
            todos.len(),
            crate::util::plural::s(todos.len())
        )
        .dimmed()
    );
    Ok(())
}
