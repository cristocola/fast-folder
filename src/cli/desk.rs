//! `fastf desk`: every project as one line, grouped by the owner of its next
//! open todo.
//!
//! Each line shows the project's first open todo, its phase, how long the
//! project has been quiet, and whether a program is working in it. The
//! groups are: yours (the hat's owner name), each other owner, projects
//! whose next todo has no owner, and projects with no open todo. The flags
//! narrow the list to one group, to the quiet projects, or to one random
//! line.
//!
//! ```bash
//! fastf desk                        # every group, yours first
//! fastf desk --mine                 # only the projects whose next todo is yours
//! fastf desk --owner claude         # only the projects whose next todo is @claude's
//! fastf desk --no-owner             # only the projects whose next todo has no owner
//! fastf desk --quiet 7              # only the projects nothing has written in for a week
//! fastf desk --random               # one project, picked at random
//! fastf desk --json
//! ```
//!
//! On a terminal, `fastf desk` opens the app on its desk tab, carrying the
//! group, the order, the hat and the random pick. `--plain` and `--json`
//! print the list instead, as do the flags the tab has no key for
//! (`--quiet`, `--template`, `--tag`, `--base`) and a pipe.
//!
//! What it reads: every project's `PROJECT_INFO.md` (the todos), the file's
//! and the folder's times (`touched`), and on Linux `/proc` for which
//! programs are working in which folder. `core::desk` does the grouping;
//! this module reads and prints.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use colored::Colorize;
use serde::Serialize;

use crate::core::config::Config;
use crate::core::desk::{self, Assignee, Card, Group, Order};
use crate::core::library::{self, Project};

pub struct DeskArgs {
    /// Use this owner name as the hat for this run, instead of the configured one.
    pub as_hat: Option<String>,
    /// Only the projects whose next todo is yours (its owner is the hat).
    pub mine: bool,
    /// Only the projects whose next todo has this owner.
    pub owner: Option<String>,
    /// Only the projects whose next todo has no owner.
    pub no_owner: bool,
    /// Only the projects with no open todo.
    pub no_open_todos: bool,
    /// Only projects nothing has written in for this many days.
    pub quiet: Option<u64>,
    /// One line, picked at random from what the other flags leave.
    pub random: bool,
    pub template: Option<String>,
    pub tag: Option<String>,
    pub base: Option<String>,
    /// `quiet` (the default), `recent`, `name` or `id`.
    pub order: Option<String>,
    pub plain: bool,
    pub json: bool,
}

/// One card as `--json` prints it.
#[derive(Serialize)]
struct CardJson {
    id: String,
    name: String,
    path: String,
    base_label: String,
    group: &'static str,
    owner: Option<String>,
    next_todo: Option<NextTodoJson>,
    open: usize,
    done: usize,
    quiet_since: Option<String>,
    quiet_for: Option<String>,
    active: Vec<String>,
}

#[derive(Serialize)]
struct NextTodoJson {
    /// The number `fastf todo done` takes, counted from one.
    number: usize,
    text: String,
    phase: Option<String>,
    owner: Option<String>,
    added: Option<String>,
}

pub fn run(args: DeskArgs) -> Result<()> {
    let cfg = Config::load()?;
    if crate::cli::terminal::hand_off_to_a_terminal(&cfg, args.plain || args.json) {
        return Ok(());
    }
    crate::cli::recent::validate_scope(&cfg, args.template.as_deref(), args.base.as_deref())?;
    let order = match &args.order {
        None => Order::default(),
        Some(word) => Order::parse(word).ok_or_else(|| {
            anyhow::anyhow!(
                "--order takes quiet, recent, name or id; got '{}'",
                word.trim()
            )
        })?,
    };
    let hat = match &args.as_hat {
        Some(hat) => Some(
            crate::core::validated::Owner::parse(hat)
                .map(crate::core::validated::Owner::into_string)?,
        ),
        None => cfg.resolve_hat(),
    };
    if args.mine && hat.is_none() {
        anyhow::bail!("--mine needs a hat: `fastf hat <name>` sets one, or pass --as <name>");
    }
    let owner_cut = args
        .owner
        .as_deref()
        .map(crate::core::validated::Owner::parse)
        .transpose()?
        .map(crate::core::validated::Owner::into_string);
    let cuts = [
        args.mine,
        owner_cut.is_some(),
        args.no_owner,
        args.no_open_todos,
    ]
    .iter()
    .filter(|on| **on)
    .count();
    if cuts > 1 {
        anyhow::bail!(
            "--mine, --owner, --no-owner and --no-open-todos each show one group; pass one of them"
        );
    }

    // Two checks, both of which must pass, the same ones `recent` makes:
    // stdout decides the format (a pipe gets the list), and stderr whether
    // the app can be drawn at all. A flag the tab has no key for prints the
    // list too.
    let printing = args.plain
        || args.json
        || args.quiet.is_some()
        || args.template.is_some()
        || args.tag.is_some()
        || args.base.is_some();
    let interactive = !printing
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
        && crate::util::tty::prompt_available();
    if interactive {
        let group = if args.mine {
            hat.clone().map(Assignee::Owner)
        } else if let Some(owner) = &owner_cut {
            Some(Assignee::Owner(owner.clone()))
        } else if args.no_owner {
            Some(Assignee::NoOwner)
        } else if args.no_open_todos {
            Some(Assignee::NoOpenTodo)
        } else {
            None
        };
        return crate::tui::run(crate::tui::Entry::Desk {
            group,
            order: args.order.as_deref().map(|_| order),
            hat: args.as_hat.as_ref().and(hat),
            random: args.random,
        });
    }

    let projects = library::discover(&cfg);
    if projects.is_empty() {
        println!(
            "{}",
            "No projects yet. Create one with `fastf new`.".dimmed()
        );
        return Ok(());
    }
    let scoped = crate::cli::recent::filter_projects(
        &projects,
        &args.template,
        &None,
        &args.tag,
        &args.base,
        usize::MAX,
    );
    let now = crate::util::time::now_iso8601();
    let cards = read_cards(&scoped);
    let mut groups = desk::group(cards, hat.as_deref(), order);

    // The group flags.
    let wanted = |assignee: &Assignee| -> bool {
        if args.mine {
            return assignee.owner() == hat.as_deref();
        }
        if let Some(owner) = &owner_cut {
            return assignee.owner() == Some(owner.as_str());
        }
        if args.no_owner {
            return *assignee == Assignee::NoOwner;
        }
        if args.no_open_todos {
            return *assignee == Assignee::NoOpenTodo;
        }
        true
    };
    groups.retain(|group| wanted(&group.assignee));
    if let Some(days) = args.quiet {
        for group in &mut groups {
            group
                .cards
                .retain(|card| desk::quiet_for_days(card.quiet_since.as_deref(), &now, days));
        }
        groups.retain(|group| !group.cards.is_empty());
    }
    if args.random {
        groups = pick_one(groups);
    }

    if args.json {
        let now = now.as_str();
        let rows: Vec<CardJson> = groups
            .iter()
            .flat_map(|group| {
                group
                    .cards
                    .iter()
                    .map(move |card| card_json(card, group, now))
            })
            .collect();
        return crate::cli::json::print(&rows);
    }

    if groups.is_empty() {
        println!(
            "{}",
            if cuts > 0
                || args.quiet.is_some()
                || args.template.is_some()
                || args.tag.is_some()
                || args.base.is_some()
            {
                "No project matches those flags."
            } else {
                "No project has a todo yet."
            }
            .dimmed()
        );
        return Ok(());
    }
    print_plain(&groups, hat.as_deref(), &now);
    Ok(())
}

/// Every project's card. The todos are read on the pool, several files at
/// once, which a network base needs; which programs work in each folder
/// comes from one pass over the processes. A file that will not read is a
/// project with no open todo, and discovery's own warning names it once.
fn read_cards(projects: &[&Project]) -> Vec<Card> {
    let paths: Vec<PathBuf> = projects.iter().map(|p| p.path.clone()).collect();
    let active = crate::core::holders::working_in_all(&paths);
    let width = projects
        .first()
        .map_or(1, |p| crate::util::pool::width_for(&p.base));
    let read: Mutex<Vec<(usize, Card)>> = Mutex::new(Vec::new());
    let items: Vec<(usize, &Project)> = projects.iter().copied().enumerate().collect();
    let _ = crate::util::pool::run(width, items, |(index, project): (usize, &Project)| {
        let todos = crate::core::body::read_todos(&project.path).unwrap_or_default();
        let working = active.get(&project.path).cloned().unwrap_or_default();
        let card = Card::of(project.clone(), &todos, working, library::touched(project));
        read.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((index, card));
        Ok::<(), std::convert::Infallible>(())
    });
    let mut cards = read.into_inner().unwrap_or_else(|e| e.into_inner());
    cards.sort_by_key(|(index, _)| *index);
    cards.into_iter().map(|(_, card)| card).collect()
}

/// One card, at random, under its group. The clock's nanoseconds are as
/// random as a pick needs.
fn pick_one(groups: Vec<Group>) -> Vec<Group> {
    let total: usize = groups.iter().map(|g| g.cards.len()).sum();
    if total == 0 {
        return groups;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as usize);
    let mut at = nanos % total;
    for group in groups {
        if at < group.cards.len() {
            let card = group.cards.into_iter().nth(at).expect("in range");
            return vec![Group {
                assignee: group.assignee,
                cards: vec![card],
            }];
        }
        at -= group.cards.len();
    }
    Vec::new()
}

fn card_json(card: &Card, group: &Group, now: &str) -> CardJson {
    CardJson {
        id: card.project.id.clone(),
        name: card.project.name.clone(),
        path: crate::util::paths::display_path(&card.project.path),
        base_label: library::base_label(&card.project.base),
        group: group.assignee.kind(),
        owner: group.assignee.owner().map(str::to_string),
        next_todo: card.next_todo.as_ref().map(|next| NextTodoJson {
            number: next.index + 1,
            text: next.todo.text.clone(),
            phase: next.todo.phase.clone(),
            owner: next.todo.owner.clone(),
            added: next.todo.added.clone(),
        }),
        open: card.open,
        done: card.done,
        quiet_since: card.quiet_since.clone(),
        quiet_for: card
            .quiet_since
            .as_deref()
            .map(|since| desk::age_label(since, now)),
        active: card.active.clone(),
    }
}

/// One heading per group with its count, one row per project under it:
/// the id, the name, the next todo (its phase before it), how long the
/// project has been quiet, and `active` when a program is working in it.
fn print_plain(groups: &[Group], hat: Option<&str>, now: &str) {
    use unicode_width::UnicodeWidthStr;
    let cards = groups.iter().flat_map(|g| g.cards.iter());
    let id_w = cards
        .clone()
        .map(|c| c.project.id.width())
        .max()
        .unwrap_or(0);
    let name_w = cards
        .clone()
        .map(|c| c.project.name.width())
        .max()
        .unwrap_or(0);
    let todo_w = cards
        .map(|c| c.next_todo_line(" · ").width())
        .max()
        .unwrap_or(0);
    let mut first = true;
    for group in groups {
        if !first {
            println!();
        }
        first = false;
        println!(
            "  {} {}",
            group.assignee.label(hat).bold(),
            format!("· {}", group.cards.len()).dimmed()
        );
        for card in &group.cards {
            let next = card.next_todo_line(" · ");
            let age = card
                .quiet_since
                .as_deref()
                .map(|since| desk::age_label(since, now))
                .unwrap_or_default();
            let active = if card.active.is_empty() {
                String::new()
            } else {
                format!("  active: {}", card.active.join(", "))
            };
            println!(
                "    {:<id_w$}  {:<name_w$}  {:<todo_w$}  {:>6}{}",
                card.project.id.green().bold(),
                card.project.name,
                next,
                age.dimmed(),
                active.cyan(),
                id_w = id_w,
                name_w = name_w,
                todo_w = todo_w,
            );
        }
    }
}

/// Every owner the cards name, each with how many projects have their next
/// todo under it. `peek` prints these counts.
pub fn assignees_of(cards: &[Card]) -> Vec<(Assignee, usize)> {
    let mut counts: HashMap<Assignee, usize> = HashMap::new();
    for card in cards {
        *counts.entry(card.assignee()).or_default() += 1;
    }
    let mut out: Vec<(Assignee, usize)> = counts.into_iter().collect();
    out.sort();
    out
}
