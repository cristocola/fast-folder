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
//! fastf todo add ID0047 "cut @claude"                 # assigned to @claude
//! fastf todo add ID0047 "call back" --mine            # in the hat's
//! fastf todo owner ID0047 3 claude                    # hand number 3 over
//! fastf todo owner ID0047 3 --none                    # to nobody
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

/// The hat, or the refusal `--mine` gets without one.
fn hat_or_refuse(cfg: &Config) -> Result<String> {
    cfg.resolve_hat().ok_or_else(|| {
        anyhow::anyhow!(
            "no hat is set: `fastf hat <name>` sets one, or name the owner with --owner"
        )
    })
}

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
    // The owner and the day in two columns after the text, each as wide as
    // the widest shown, so the eye reads down them; a list with neither has
    // neither.
    let text_w = todos
        .iter()
        .map(|t| unicode_width::UnicodeWidthStr::width(t.text.as_str()))
        .max()
        .unwrap_or(0);
    let owner_w = todos
        .iter()
        .filter_map(|t| t.owner.as_deref())
        .map(|o| unicode_width::UnicodeWidthStr::width(o) + 1)
        .max()
        .unwrap_or(0);
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
        let trailer = trailer_columns(todo, text_w, owner_w);
        if todo.done {
            println!(
                "  {} {} {}{}",
                number.dimmed(),
                marker.dimmed(),
                todo.text.dimmed(),
                trailer.dimmed()
            );
        } else {
            println!(
                "  {} {} {}{}",
                number.dimmed(),
                marker.green(),
                todo.text,
                trailer.dimmed()
            );
        }
    }
    if shown == 0 {
        println!("  {}", "(everything is done)".dimmed());
    }
    println!();
    println!("  {}", format!("{done}/{} done", todos.len()).dimmed());
    Ok(())
}

/// The owner and the day after a todo's text, padded into their columns:
/// `  @claude  2026-10-12` — the day it was added, or the day it was done
/// once it is. Empty when the todo has neither, so a plain list stays plain.
fn trailer_columns(todo: &Todo, text_w: usize, owner_w: usize) -> String {
    let owner = todo.owner.as_deref().map(|o| format!("@{o}"));
    let day = if todo.done {
        todo.done_day().or_else(|| todo.added_day())
    } else {
        todo.added_day()
    };
    if owner.is_none() && day.is_none() {
        return String::new();
    }
    let pad = text_w.saturating_sub(unicode_width::UnicodeWidthStr::width(todo.text.as_str()));
    let mut out = " ".repeat(pad);
    if owner_w > 0 {
        let owner = owner.unwrap_or_default();
        let pad = owner_w.saturating_sub(unicode_width::UnicodeWidthStr::width(owner.as_str()));
        out.push_str("  ");
        out.push_str(&owner);
        out.push_str(&" ".repeat(pad));
    }
    if let Some(day) = day {
        out.push_str("  ");
        out.push_str(day);
    }
    out.trim_end().to_string()
}

pub struct AddArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// The task, one line.
    pub text: String,
    /// The `###` label to put it under, opening one if the list has none.
    pub phase: Option<String>,
    /// Who it is assigned to, over any `@owner` typed at the end of the text.
    pub owner: Option<String>,
    /// Assign it to the hat.
    pub mine: bool,
}

pub fn add(args: AddArgs) -> Result<()> {
    let (cfg, project) = project(&args.query)?;
    let owner = match (&args.owner, args.mine) {
        (Some(_), true) => bail!("--owner and --mine name two owners — pass one"),
        (Some(owner), false) => Some(owner.clone()),
        (None, true) => Some(hat_or_refuse(&cfg)?),
        (None, false) => None,
    };
    let owner = crate::core::operations::add_todo_owned(
        &project,
        &args.text,
        owner.as_deref(),
        args.phase.as_deref(),
    )
    .with_context(|| format!("adding a todo to {}", project.id))?;
    let assigned = owner
        .map(|owner| format!(" for @{owner}"))
        .unwrap_or_default();
    match &args.phase {
        Some(phase) => println!(
            "{}  Todo added to {} under {}{assigned}",
            "✓".green().bold(),
            project.id.green().bold(),
            phase.bold()
        ),
        None => println!(
            "{}  Todo added to {}{assigned}",
            "✓".green().bold(),
            project.id.green().bold()
        ),
    }
    Ok(())
}

pub struct OwnerArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// The number `fastf todo list` printed, counted from one.
    pub number: usize,
    /// The new owner, with or without its `@`. `None` with `--none` takes
    /// the todo away from everybody; `None` without it prints the owner.
    pub owner: Option<String>,
    pub none: bool,
    /// Assign it to the hat.
    pub mine: bool,
}

/// `fastf todo owner`: who a todo is assigned to, or assign it to somebody.
pub fn owner(args: OwnerArgs) -> Result<()> {
    let (cfg, project) = project(&args.query)?;
    let todos = crate::core::body::read_todos(&project.path)?;
    let todo = numbered(&project, &todos, args.number)?;
    let wanted: Option<Option<String>> = match (&args.owner, args.none, args.mine) {
        (Some(_), true, _) | (Some(_), _, true) | (None, true, true) => {
            bail!("name one owner, --mine or --none — not two of them")
        }
        (Some(owner), false, false) => Some(Some(owner.clone())),
        (None, true, false) => Some(None),
        (None, false, true) => Some(Some(hat_or_refuse(&cfg)?)),
        (None, false, false) => None,
    };
    let Some(wanted) = wanted else {
        match &todo.owner {
            Some(owner) => println!("@{}  {}", owner.bold(), todo.text.dimmed()),
            None => println!("{}  {}", "(nobody's)".dimmed(), todo.text.dimmed()),
        }
        return Ok(());
    };
    let owner = crate::core::operations::set_todo_owner(
        &project,
        args.number - 1,
        &todo.text,
        wanted.as_deref(),
    )
    .with_context(|| format!("setting a todo's owner in {}", project.id))?;
    match owner {
        Some(owner) => println!(
            "{}  {} is now @{}'s: {}",
            "✓".green().bold(),
            format!("todo {}", args.number).bold(),
            owner.bold(),
            todo.text
        ),
        None => println!(
            "{}  {} is nobody's now: {}",
            "✓".green().bold(),
            format!("todo {}", args.number).bold(),
            todo.text
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
