//! `fastf todo` — a project's task list from the command line.
//!
//! The tasks are the `- [ ]` and `- [x]` lines of the `## Todo` section of
//! `PROJECT_INFO.md`, and a `###` line inside that section labels the
//! **phase** every task under it belongs to. The app's detail pane shows the
//! same list; this is the surface a script or an agent uses.
//!
//! ```bash
//! fastf todo list ID0047                              # open ones, done ones counted
//! fastf todo list ID0047 --all                        # every one, in file order
//! fastf todo add ID0047 "colour grade"                # at the end
//! fastf todo add ID0047 "cut" --phase "Main Edit"     # under a label
//! fastf todo add ID0047 "cut @claude"                 # owner: @claude
//! fastf todo add ID0047 "call back" --mine            # in the hat's
//! fastf todo owner ID0047 3 claude                    # hand number 3 over
//! fastf todo owner ID0047 3 --none                    # to nobody
//! fastf todo done ID0047 3                            # tick number 3
//! fastf todo done ID0047 3 --undo                     # untick it again
//! fastf todo edit ID0047 3 "grade the colour"         # reword number 3
//! fastf todo remove ID0047 3                          # take it out (or `rm`)
//! ```
//!
//! A task's number is its place in the file, counted from one over the
//! tasks alone: a label is not a task and takes no number. `list` prints the
//! open ones' numbers and folds each run's done ones into `✓ n done`, and
//! `list --all` prints every number; showing the open ones first renumbers
//! nothing. A verb that takes a number reads the list first and hands the
//! core the text it read there, so a file that changed in between is
//! refused rather than edited on the wrong line.

use anyhow::{Context, Result, bail};
use colored::Colorize;

use crate::core::body::{Todo, TodoGroup};
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

/// The task `number` names — its place in the file, counted from one — or
/// the refusal that says where the numbers come from. One helper, so every
/// verb that takes a number refuses a wrong one in the same words.
fn numbered<'a>(project: &Project, todos: &'a [Todo], number: usize) -> Result<&'a Todo> {
    if number == 0 {
        bail!(
            "the todos are numbered from 1 — `fastf todo list {} --all` prints every number",
            project.id
        );
    }
    let Some(todo) = todos.get(number - 1) else {
        bail!(
            "{} has {} — `fastf todo list {} --all` prints every number",
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

/// What `list` prints of each run of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListShow {
    /// The open tasks, and one `✓ n done` line for the done ones.
    Folded,
    /// The open tasks alone.
    Open,
    /// Every task, in file order.
    All,
}

pub struct ListArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    pub show: ListShow,
}

pub fn list(args: ListArgs) -> Result<()> {
    let (_cfg, project) = project(&args.query)?;
    let (todos, labels) = crate::core::body::read_todo_list(&project.path)?;

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
    let groups = crate::core::body::todo_groups(&todos, &labels);
    // What each run prints: its open tasks, or every task. A number is the
    // task's place in the file whichever is shown.
    let rows_of = |group: &TodoGroup| -> Vec<usize> {
        match args.show {
            ListShow::Folded | ListShow::Open => group.open.clone(),
            ListShow::All => group.tasks.clone().collect(),
        }
    };
    let number_w = todos.len().to_string().len();
    let shown: Vec<&Todo> = groups
        .iter()
        .flat_map(rows_of)
        .map(|ordinal| &todos[ordinal])
        .collect();
    let widths = trailer_widths(&shown, number_w, crate::tui::rows::print_columns());
    for group in &groups {
        let rows = rows_of(group);
        if args.show == ListShow::Open && rows.is_empty() {
            continue;
        }
        // The label over the run it names, the way the pane draws it, an
        // empty one too: it is where `add --phase` puts a task.
        if let Some(name) = &group.phase {
            println!("  {}", name.dimmed());
        }
        for ordinal in rows {
            let todo = &todos[ordinal];
            let marker = if todo.done { "[x]" } else { "[ ]" };
            let number = format!("{:>number_w$}.", ordinal + 1);
            let trailer = trailer_columns(todo, widths);
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
        if args.show == ListShow::Folded && !group.done.is_empty() {
            // In the column the boxes are in, so the count reads as one
            // more row of the run.
            println!(
                "  {}{}",
                " ".repeat(number_w + 2),
                format!("✓ {} done", group.done.len()).dimmed()
            );
        }
    }
    if args.show == ListShow::Open && shown.is_empty() {
        println!("  {}", "(everything is done)".dimmed());
    }
    println!();
    println!("  {}", format!("{done}/{} done", todos.len()).dimmed());
    Ok(())
}

/// The columns a list's trailers are aligned in: the text column's width,
/// and the owner column's after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrailerWidths {
    text: usize,
    owner: usize,
}

/// The trailer columns for the todos a list shows, measured over those
/// alone — an `--open` list is not padded for a done task it leaves out —
/// with the text column capped so an aligned line fits `columns`: the
/// indent, a number `number_w` wide and its box, the text, then the owner
/// and the day, each two spaces after what precedes it.
fn trailer_widths(shown: &[&Todo], number_w: usize, columns: usize) -> TrailerWidths {
    use unicode_width::UnicodeWidthStr;
    let owner = shown
        .iter()
        .filter_map(|t| t.owner.as_deref())
        .map(|o| o.width() + 1)
        .max()
        .unwrap_or(0);
    let day = shown
        .iter()
        .filter_map(|t| day_shown(t))
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0);
    // `  12. [ ] `
    let lead = number_w + 8;
    let gapped = |w: usize| if w > 0 { w + 2 } else { 0 };
    let widest = shown.iter().map(|t| t.text.width()).max().unwrap_or(0);
    TrailerWidths {
        text: widest.min(columns.saturating_sub(lead + gapped(owner) + gapped(day))),
        owner,
    }
}

/// The day a todo's line shows: the day it was added, or the day it was
/// done once it is.
fn day_shown(todo: &Todo) -> Option<&str> {
    if todo.done {
        todo.done_day().or_else(|| todo.added_day())
    } else {
        todo.added_day()
    }
}

/// The owner and the day after a todo's text, padded into their columns:
/// `  @claude  2026-10-12`. Empty when the todo has neither, so a plain list
/// stays plain. A text wider than its column keeps them two spaces after it,
/// with no column's padding: aligning past a long text is a gap, not a
/// column.
fn trailer_columns(todo: &Todo, widths: TrailerWidths) -> String {
    use unicode_width::UnicodeWidthStr;
    let owner = todo.owner.as_deref().map(|o| format!("@{o}"));
    let day = day_shown(todo);
    if owner.is_none() && day.is_none() {
        return String::new();
    }
    let text_w = todo.text.width();
    let mut out = String::new();
    if text_w > widths.text {
        for part in owner.iter().map(String::as_str).chain(day) {
            out.push_str("  ");
            out.push_str(part);
        }
        return out;
    }
    out.push_str(&" ".repeat(widths.text - text_w));
    if widths.owner > 0 {
        let owner = owner.unwrap_or_default();
        out.push_str("  ");
        out.push_str(&owner);
        out.push_str(&" ".repeat(widths.owner.saturating_sub(owner.width())));
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
    /// Its owner, over any `@owner` typed at the end of the text.
    pub owner: Option<String>,
    /// Make the hat its owner.
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
    /// The todo's place in the file, counted from one: the number
    /// `fastf todo list` prints beside it (`--all` for a done one).
    pub number: usize,
    /// The new owner, with or without its `@`. `None` with `--none` takes
    /// the todo away from everybody; `None` without it prints the owner.
    pub owner: Option<String>,
    pub none: bool,
    /// Make the hat its owner.
    pub mine: bool,
}

/// `fastf todo owner`: print a todo's owner, or set it.
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
    /// The todo's place in the file, counted from one: the number
    /// `fastf todo list` prints beside it (`--all` for a done one).
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
    /// The todo's place in the file, counted from one: the number
    /// `fastf todo list` prints beside it (`--all` for a done one).
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
    /// The todo's place in the file, counted from one: the number
    /// `fastf todo list` prints beside it (`--all` for a done one).
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

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(text: &str, owner: Option<&str>, added: Option<&str>) -> Todo {
        Todo {
            done: false,
            text: text.to_string(),
            phase: None,
            owner: owner.map(str::to_string),
            added: added.map(str::to_string),
            done_at: None,
        }
    }

    #[test]
    fn the_text_column_is_the_widest_shown_text_within_the_width() {
        let short = todo("cut", Some("claude"), Some("2026-10-12T10:00:00Z"));
        let longer = todo("grade the colour", None, None);
        // `  1. [ ] ` is 9 columns, `  @claude` 9, `  2026-10-12` 12.
        assert_eq!(
            trailer_widths(&[&short, &longer], 1, 100),
            TrailerWidths { text: 16, owner: 7 }
        );
        assert_eq!(
            trailer_widths(&[&short, &longer], 1, 40),
            TrailerWidths { text: 10, owner: 7 },
            "capped at what the width leaves after the lead and the trailer"
        );
        assert_eq!(
            trailer_widths(&[&longer], 1, 100),
            TrailerWidths { text: 16, owner: 0 },
            "a list nobody owns anything in has no owner column"
        );
        assert_eq!(
            trailer_widths(&[], 1, 100),
            TrailerWidths { text: 0, owner: 0 }
        );
    }

    #[test]
    fn a_text_wider_than_its_column_keeps_its_trailer_two_spaces_after_it() {
        let widths = TrailerWidths { text: 10, owner: 7 };
        let owned = todo("cut", Some("claude"), Some("2026-10-12T10:00:00Z"));
        assert_eq!(
            trailer_columns(&owned, widths),
            "         @claude  2026-10-12",
            "a short text is padded into the columns"
        );
        let nobodys = todo("cut", None, Some("2026-10-12T10:00:00Z"));
        assert_eq!(
            trailer_columns(&nobodys, widths),
            "                  2026-10-12",
            "an owner-less one keeps the day in its column"
        );
        let long = todo("a task far wider than its column", None, Some("2026-10-12"));
        assert_eq!(
            trailer_columns(&long, widths),
            "  2026-10-12",
            "no owner column's padding after a long text, though others own theirs"
        );
        let long_owned = todo("a task far wider than its column", Some("sales"), None);
        assert_eq!(trailer_columns(&long_owned, widths), "  @sales");
        let plain = todo("a task far wider than its column", None, None);
        assert_eq!(trailer_columns(&plain, widths), "");
    }
}
