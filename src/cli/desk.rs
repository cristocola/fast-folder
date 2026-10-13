//! `fastf desk` — the work across every project, one line each, grouped by
//! whose court the ball is in.
//!
//! The overseer's question is not "what are all the todos" but "what is
//! waiting on me, what is moving without me, and what has gone quiet". So
//! each project is one line — its first open todo, who holds it, how long
//! the project has been quiet, whether a program is working in it — under
//! the court the ball is in: yours, an agent's, somebody you are waiting
//! on, nobody's, or nothing planned at all. The cuts narrow it: one court,
//! the quiet ones, a random pick.
//!
//! ```bash
//! fastf desk                        # every court, the hat's own first
//! fastf desk --mine                 # the hat's court alone
//! fastf desk --owner claude         # one court
//! fastf desk --unowned              # the pile to assign from
//! fastf desk --quiet 7              # nothing written for a week
//! fastf desk --random               # pick me one
//! fastf desk --json
//! ```
//!
//! On a terminal, `fastf desk` opens the app on its desk tab, carrying the
//! court, the order, the hat and the random pick; `--plain` and `--json`
//! keep the list in the terminal, as do the cuts the tab has no key for
//! (`--quiet`, `--template`, `--tag`, `--base`) and a pipe.
//!
//! What it reads: every project's `PROJECT_INFO.md` (the todos), the file's
//! and the folder's times (`touched`), and on Linux `/proc` for who is
//! working where. `core::desk` is the grouping, pure; this is the reading
//! and the printing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use colored::Colorize;
use serde::Serialize;

use crate::core::config::Config;
use crate::core::desk::{self, Card, Court, Group, Order};
use crate::core::library::{self, Project};

pub struct DeskArgs {
    /// Wear this hat for the run, over the configured one.
    pub as_hat: Option<String>,
    /// The hat's court alone.
    pub mine: bool,
    /// One owner's court alone.
    pub owner: Option<String>,
    /// The unowned pile alone.
    pub unowned: bool,
    /// The projects with nothing planned alone.
    pub nothing_planned: bool,
    /// Only projects nothing has written in for this many days.
    pub quiet: Option<u64>,
    /// One line, picked at random from what the cuts leave.
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
    court: &'static str,
    owner: Option<String>,
    ball: Option<BallJson>,
    open: usize,
    done: usize,
    quiet_since: Option<String>,
    quiet_for: Option<String>,
    active: Vec<String>,
}

#[derive(Serialize)]
struct BallJson {
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
        anyhow::bail!("--mine needs a hat — `fastf hat <name>` wears one, or pass --as <name>");
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
        args.unowned,
        args.nothing_planned,
    ]
    .iter()
    .filter(|on| **on)
    .count();
    if cuts > 1 {
        anyhow::bail!(
            "--mine, --owner, --unowned and --nothing-planned each show one court — pass one"
        );
    }

    // Two questions, both of which must say yes, as `recent` asks them:
    // stdout decides the format (a pipe gets the list), and stderr whether
    // the app can be drawn at all. A cut the tab has no key for keeps the
    // list in the terminal too.
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
        let court = if args.mine {
            hat.clone().map(Court::Owner)
        } else if let Some(owner) = &owner_cut {
            Some(Court::Owner(owner.clone()))
        } else if args.unowned {
            Some(Court::Unowned)
        } else if args.nothing_planned {
            Some(Court::NothingPlanned)
        } else {
            None
        };
        return crate::tui::run(crate::tui::Entry::Desk {
            court,
            order: args.order.as_deref().map(|_| order),
            hat: args.as_hat.as_ref().and(hat),
            random: args.random,
        });
    }

    let projects = library::discover(&cfg);
    if projects.is_empty() {
        println!(
            "{}",
            "No projects yet — create one with `fastf new`.".dimmed()
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

    // The cuts.
    let wanted = |court: &Court| -> bool {
        if args.mine {
            return court.owner() == hat.as_deref();
        }
        if let Some(owner) = &owner_cut {
            return court.owner() == Some(owner.as_str());
        }
        if args.unowned {
            return *court == Court::Unowned;
        }
        if args.nothing_planned {
            return *court == Court::NothingPlanned;
        }
        true
    };
    groups.retain(|group| wanted(&group.court));
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
                "Nothing on the desk matches those cuts."
            } else {
                "Nothing on the desk."
            }
            .dimmed()
        );
        return Ok(());
    }
    print_plain(&groups, hat.as_deref(), &now);
    Ok(())
}

/// Every project's card: its todos read on the pool — several files at
/// once, which is what a network base needs — and who is working in it,
/// from one pass over the processes. A file that will not read is a
/// project with nothing planned, named once by discovery's own warning.
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

/// One card, at random, under its court: the one thing that beats a
/// stalled eye. The clock's nanoseconds are as random as a pick needs.
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
                court: group.court,
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
        court: group.court.kind(),
        owner: group.court.owner().map(str::to_string),
        ball: card.ball.as_ref().map(|ball| BallJson {
            number: ball.index + 1,
            text: ball.todo.text.clone(),
            phase: ball.todo.phase.clone(),
            owner: ball.todo.owner.clone(),
            added: ball.todo.added.clone(),
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

/// One heading per court with its count, one row per project under it:
/// the id, the name, the ball (its phase before it), how long the project
/// has been quiet, and `active` when a program is working in it.
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
    let ball_w = cards.map(|c| c.ball_line(" · ").width()).max().unwrap_or(0);
    let mut first = true;
    for group in groups {
        if !first {
            println!();
        }
        first = false;
        println!(
            "  {} {}",
            group.court.label(hat).bold(),
            format!("· {}", group.cards.len()).dimmed()
        );
        for card in &group.cards {
            let ball = card.ball_line(" · ");
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
                "    {:<id_w$}  {:<name_w$}  {:<ball_w$}  {:>6}{}",
                card.project.id.green().bold(),
                card.project.name,
                ball,
                age.dimmed(),
                active.cyan(),
                id_w = id_w,
                name_w = name_w,
                ball_w = ball_w,
            );
        }
    }
}

/// Which owners the open todos of `cards` name, each with how many balls it
/// holds across the desk — the courts' counts, for `peek` and a heading.
pub fn courts_of(cards: &[Card]) -> Vec<(Court, usize)> {
    let mut counts: HashMap<Court, usize> = HashMap::new();
    for card in cards {
        *counts.entry(card.court()).or_default() += 1;
    }
    let mut out: Vec<(Court, usize)> = counts.into_iter().collect();
    out.sort();
    out
}
