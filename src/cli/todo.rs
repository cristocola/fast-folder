//! `fastf todo` — a project's task list from the command line.
//!
//! The tasks are the `- [ ]` and `- [x]` lines of the `## Todo` section of
//! `PROJECT_INFO.md`, and a `###` line inside that section labels the
//! **phase** every task under it belongs to. The app's detail pane shows the
//! same list; this is the surface a script or an agent uses.
//!
//! ```bash
//! fastf todo list ID0047                              # the list, numbered
//! fastf todo add ID0047 "colour grade"                # at the end
//! fastf todo add ID0047 "cut" --phase "Main Edit"     # under a label
//! fastf todo done ID0047 3                            # tick number 3
//! fastf todo done ID0047 3 --undo                     # untick it again
//! fastf todo edit ID0047 3 "grade the colour"         # reword number 3
//! fastf todo remove ID0047 3                          # take it out (or `rm`)
//! ```
//!
//! The numbers are the ones `list` prints, counted from one over the tasks
//! alone: a label is not a task and takes no number. A verb that takes a
//! number reads the list first and hands the core the text it read there,
//! so a file that changed in between is refused rather than edited on the
//! wrong line.

use anyhow::{Context, Result, bail};
use colored::Colorize;

use crate::core::body::Todo;
use crate::core::library::{self, Project};
use crate::core::{config::Config, project_info};

/// The project a todo command works on, refusing a folder fastf owns no
/// metadata for with the sentence `tag` and `note` give for the same thing.
fn project(query: &str) -> Result<(Config, Project)> {
    let cfg = Config::load()?;
    let candidate = library::resolve(&cfg, query)?;
    if !project_info::pinfo_path(&candidate.path).exists() {
        bail!(
            "{}",
            crate::cli::tag::no_metadata_message(&candidate.id, &candidate.path)
        );
    }
    Ok((cfg, candidate))
}

/// The task `number` names — counted from one, as `list` prints them — or
/// the refusal that says where the numbers come from. One helper, so every
/// verb that takes a number refuses a wrong one in the same words.
fn numbered<'a>(project: &Project, todos: &'a [Todo], number: usize) -> Result<&'a Todo> {
    if number == 0 {
        bail!("the todos are numbered from 1 — `fastf todo list` prints them");
    }
    let Some(todo) = todos.get(number - 1) else {
        bail!(
            "{} has {} — `fastf todo list {}` prints them",
            project.id,
            match todos.len() {
                0 => "no todos".to_string(),
                1 => "one todo".to_string(),
                n => format!("{n} todos"),
            },
            project.id
        );
    };
    Ok(todo)
}

pub struct ListArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// Leave out the tasks already done.
    pub open: bool,
}

pub fn list(args: ListArgs) -> Result<()> {
    let (_cfg, project) = project(&args.query)?;
    let todos = crate::core::body::read_todos(&project.path)?;

    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        project.id.green().bold(),
        project.name.bold()
    );

    if todos.is_empty() {
        println!(
            "    {}",
            "(no todos yet — use `fastf todo add` to add one)".dimmed()
        );
        return Ok(());
    }

    println!();
    let done = todos.iter().filter(|t| t.done).count();
    let width = todos.len().to_string().len();
    let mut phase: Option<&str> = None;
    let mut shown = 0;
    for (index, todo) in todos.iter().enumerate() {
        if todo.done && args.open {
            continue;
        }
        // The label over the run it names, the way the pane draws it.
        if todo.phase.as_deref() != phase {
            phase = todo.phase.as_deref();
            if let Some(name) = phase {
                println!("  {}", name.dimmed());
            }
        }
        shown += 1;
        let marker = if todo.done { "[x]" } else { "[ ]" };
        let number = format!("{:>width$}.", index + 1, width = width);
        if todo.done {
            println!(
                "  {} {} {}",
                number.dimmed(),
                marker.dimmed(),
                todo.text.dimmed()
            );
        } else {
            println!("  {} {} {}", number.dimmed(), marker.green(), todo.text);
        }
    }
    if shown == 0 {
        println!("  {}", "(everything is done)".dimmed());
    }
    println!();
    println!("  {}", format!("{done}/{} done", todos.len()).dimmed());
    Ok(())
}

pub struct AddArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// The task, one line.
    pub text: String,
    /// The `###` label to put it under, opening one if the list has none.
    pub phase: Option<String>,
}

pub fn add(args: AddArgs) -> Result<()> {
    let (_cfg, project) = project(&args.query)?;
    crate::core::operations::add_todo_in(&project, &args.text, args.phase.as_deref())
        .with_context(|| format!("adding a todo to {}", project.id))?;
    match &args.phase {
        Some(phase) => println!(
            "{}  Todo added to {} under {}",
            "✓".green().bold(),
            project.id.green().bold(),
            phase.bold()
        ),
        None => println!(
            "{}  Todo added to {}",
            "✓".green().bold(),
            project.id.green().bold()
        ),
    }
    Ok(())
}

pub struct DoneArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// The number `fastf todo list` printed, counted from one.
    pub number: usize,
    /// Mark it open again instead.
    pub undo: bool,
}

pub fn done(args: DoneArgs) -> Result<()> {
    let (_cfg, project) = project(&args.query)?;
    let todos = crate::core::body::read_todos(&project.path)?;
    let todo = numbered(&project, &todos, args.number)?;
    let wanted = !args.undo;
    if todo.done == wanted {
        println!(
            "{}  {} is already {}: {}",
            "·".dimmed(),
            format!("todo {}", args.number).bold(),
            if wanted { "done" } else { "open" },
            todo.text.dimmed()
        );
        return Ok(());
    }
    // `expected` is the text just read, so a file that changed under us is
    // refused rather than mis-ticked.
    crate::core::operations::toggle_todo(&project, args.number - 1, &todo.text)
        .with_context(|| format!("toggling a todo of {}", project.id))?;
    println!(
        "{}  {} {}: {}",
        "✓".green().bold(),
        if wanted { "Done" } else { "Reopened" },
        format!("in {}", project.id).green().bold(),
        todo.text
    );
    Ok(())
}

pub struct EditArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// The number `fastf todo list` printed, counted from one.
    pub number: usize,
    /// The new text, one line.
    pub text: String,
}

pub fn edit(args: EditArgs) -> Result<()> {
    let (_cfg, project) = project(&args.query)?;
    let todos = crate::core::body::read_todos(&project.path)?;
    let todo = numbered(&project, &todos, args.number)?;
    let text = args.text.trim();
    // The pane removes a todo emptied in place, because it has no other way
    // to say so; the command line has a verb for it, and an empty argument is
    // more often a script's unset variable than a wish to delete.
    if text.is_empty() {
        bail!(
            "the new text is empty — `fastf todo remove {} {}` removes a todo",
            project.id,
            args.number
        );
    }
    if text == todo.text {
        println!(
            "{}  {} already reads: {}",
            "·".dimmed(),
            format!("todo {}", args.number).bold(),
            todo.text.dimmed()
        );
        return Ok(());
    }
    crate::core::operations::replace_todo(&project, args.number - 1, &todo.text, text)
        .with_context(|| format!("rewording a todo of {}", project.id))?;
    println!(
        "{}  Reworded {}: {}",
        "✓".green().bold(),
        format!("in {}", project.id).green().bold(),
        text
    );
    Ok(())
}

pub struct RemoveArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// The number `fastf todo list` printed, counted from one.
    pub number: usize,
}

pub fn remove(args: RemoveArgs) -> Result<()> {
    let (_cfg, project) = project(&args.query)?;
    let todos = crate::core::body::read_todos(&project.path)?;
    let todo = numbered(&project, &todos, args.number)?;
    // An empty text is the core's removal.
    crate::core::operations::replace_todo(&project, args.number - 1, &todo.text, "")
        .with_context(|| format!("removing a todo of {}", project.id))?;
    println!(
        "{}  Removed {}: {}",
        "✓".green().bold(),
        format!("from {}", project.id).green().bold(),
        todo.text
    );
    Ok(())
}
