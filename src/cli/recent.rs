use anyhow::{Context, Result};
use colored::Colorize;
use std::io::IsTerminal;

use crate::core::config::Config;
use crate::core::library::{self, Project};
use crate::tui::rows::{RowWidths, date_cell};

pub struct RecentArgs {
    /// None = use Config::recent_default_limit.
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
        browse(
            filtered.into_iter().cloned().collect(),
            "No projects to show.",
        )
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

/// Open the guided browser over an already-filtered list.
///
/// `fastf recent` and `fastf search` used to have a second, size-less picker of
/// their own (`run_picker`), so the same library looked different depending on
/// which door you came through and only one of the two showed folder sizes.
/// There is one browser now; the shell entry points differ only in their last
/// row, which says Quit rather than Back to main menu.
pub fn browse(projects: Vec<Project>, empty_message: &str) -> Result<()> {
    let page_size = Config::load()?.recent_default_limit.max(1);
    // The list is already filtered and already read: the loader hands back the
    // same rows rather than discovering again.
    let mut initial = Some(projects);
    crate::tui::browser::run_paged_browser(
        page_size,
        empty_message,
        "Quit",
        move || Ok(initial.take().unwrap_or_default()),
        |_| true,
    )
}

/// `fastf open <query>` — resolve a project and reveal it in the file manager.
pub fn open(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    // Ambiguity on a terminal is a question, not a failure — and the answer
    // opens the project, rather than dropping into the action menu.
    let Some(project) = crate::cli::target::one_project(
        &cfg,
        query,
        "Which project?",
        &crate::cli::target::full_id_hint("open"),
    )?
    else {
        crate::tui::prompt::report_cancelled("nothing was opened");
        return Ok(());
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
