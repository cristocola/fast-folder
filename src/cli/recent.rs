use anyhow::{Context, Result};
use colored::Colorize;
use std::io::IsTerminal;

use crate::core::config::Config;
use crate::core::library::{self, Project};
use crate::tui::rows::{RowWidths, date_cell};

pub struct RecentArgs {
    /// None = use the configured `recent-limit`.
    pub limit: Option<usize>,
    pub template: Option<String>,
    pub since: Option<String>,
    /// Only show projects that have this tag.
    pub tag: Option<String>,
    /// Force the plain (non-interactive) list output. Auto-engages when stdout
    /// is not a TTY.
    pub plain: bool,
}

pub fn run(args: RecentArgs) -> Result<()> {
    let cfg = Config::load()?;
    // `--limit 0` used to be clamped to 1, quietly showing a project the user
    // asked not to see. Refuse instead — a zero-length list is not what anyone
    // means by it.
    if args.limit == Some(0) {
        anyhow::bail!("--limit must be at least 1");
    }
    let limit = args.limit.unwrap_or(cfg.recent_default_limit).max(1);

    // Nothing below this line can be read from a desktop launcher: stdout and
    // stderr are journald sockets there, and the picker has no terminal to draw
    // on. Rather than working into the void, open a terminal and run this again
    // inside it. In every other context — a shell, a pipe, cron, CI — this is
    // false and nothing changes.
    if crate::cli::terminal::hand_off_to_a_terminal(&cfg, args.plain) {
        return Ok(());
    }

    // Filesystem-as-truth: discover projects from their PROJECT_INFO.md across
    // all bases (cache-accelerated). Already sorted newest-first.
    let projects = library::discover(&cfg);

    if projects.is_empty() {
        println!(
            "{}",
            "No projects yet — create one with `fastf new`.".dimmed()
        );
        return Ok(());
    }

    let filtered = filter_projects(&projects, &args.template, &args.since, &args.tag, limit);

    if filtered.is_empty() {
        println!("{}", "No projects match those filters.".dimmed());
        return Ok(());
    }

    // Two questions, both of which must say yes: stdout decides the *format*
    // (a pipe gets the plain list), and stderr decides whether the picker can
    // be drawn and answered at all. Without the second, `2>/dev/null` launched
    // a picker nobody could see and waited for a key.
    let interactive =
        !args.plain && std::io::stdout().is_terminal() && crate::util::tty::prompt_available();

    if interactive {
        crate::tui::run(crate::tui::Entry::Recent {
            preset: crate::tui::Preset {
                template: args.template.clone(),
                since: args.since.clone(),
                tag: args.tag.clone(),
                limit: args.limit,
            },
            initial: filtered.into_iter().cloned().collect(),
        })
    } else {
        print_plain(&filtered);
        Ok(())
    }
}

fn filter_projects<'a>(
    projects: &'a [Project],
    template: &Option<String>,
    since: &Option<String>,
    tag: &Option<String>,
    limit: usize,
) -> Vec<&'a Project> {
    // `discover` already returns newest-first; just filter + take.
    let mut filtered: Vec<&Project> = projects
        .iter()
        .filter(|p| {
            if let Some(slug) = template
                && &p.template != slug
            {
                return false;
            }
            if let Some(since) = since
                && p.created.as_str() < since.as_str()
            {
                return false;
            }
            if let Some(want_tag) = tag
                && !p.tags.iter().any(|t| t == want_tag)
            {
                return false;
            }
            true
        })
        .collect();

    filtered.truncate(limit);
    filtered
}

/// Plain (non-interactive) list output. Shared by `fastf recent` and
/// `fastf search` — keep the two commands' output identical.
pub fn print_plain(filtered: &[&Project]) {
    let widths = RowWidths::measure(filtered.iter().copied());

    for p in filtered {
        let path_str = crate::util::paths::display_path(&p.path);
        let missing = !p.path.exists();
        let marker = if missing { "✗".red() } else { "•".cyan() };
        println!(
            "  {} {:<id_w$}  {:<tmpl_w$}  {}  {:<base_w$}  {}",
            marker,
            p.id.green().bold(),
            p.template.dimmed(),
            date_cell(&p.created).dimmed(),
            library::base_label(&p.base).cyan(),
            if missing {
                format!("{} {}", p.name, "(missing)".red())
            } else {
                p.name.clone()
            },
            id_w = widths.id,
            tmpl_w = widths.template,
            base_w = widths.base,
        );
        println!("      {} {}", "→".dimmed(), path_str.dimmed());
    }
}

/// `fastf open <query>` — resolve a project and reveal it in the file manager.
pub fn open(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    // Ambiguity on a terminal is a question, not a failure — and the answer
    // opens the project, rather than dropping into the action menu.
    let project = match crate::cli::target::one_project(
        &cfg,
        query,
        "Which project?",
        &crate::cli::target::full_id_hint("open"),
    )? {
        crate::cli::target::Target::Project(project) => *project,
        crate::cli::target::Target::Cancelled => {
            crate::tui::prompt::report_cancelled("nothing was opened");
            return Ok(());
        }
        // A terminal is running this again; it will open the project.
        crate::cli::target::Target::HandedOff => return Ok(()),
    };

    // `resolve` may have answered from a cache, and a cache is a file that
    // travels with the projects — a synced folder or an unpacked archive can
    // bring one along. Check what the path names before spawning the system
    // file manager on it.
    library::revalidate_for_read(&project).with_context(|| {
        format!(
            "project '{}' cannot be opened at {}",
            project.id,
            crate::util::paths::display_path(&project.path)
        )
    })?;
    println!(
        "{} Opening {} ({})",
        "→".cyan().bold(),
        project.name.bold(),
        crate::util::paths::display_path(&project.path).dimmed()
    );
    crate::core::post_create::reveal_folder(&project.path)
}
