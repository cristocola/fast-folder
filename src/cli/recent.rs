use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

use crate::core::config::Config;
use crate::core::library::{self, Project};
use crate::tui::actions::{ActionLoop, project_action_menu};
use crate::tui::rows::{
    ProjectRowTheme, RowWidths, clamp_label, date_cell, project_row, terminal_columns,
};

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
        run_picker(&filtered)
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

/// Interactive picker shared by `fastf recent` and `fastf search`.
///
/// Displays the projects in a `dialoguer::Select` loop. Selecting a project
/// enters `tui::actions::project_action_menu`.
pub fn run_picker(filtered: &[&Project]) -> Result<()> {
    use dialoguer::Select;

    let columns = terminal_columns();
    let theme = ProjectRowTheme::new(columns);
    let widths = RowWidths::measure(filtered.iter().copied());

    loop {
        let labels: Vec<String> = filtered
            .iter()
            // Tags come straight from discovery (cache/scan) — no reload.
            .map(|p| clamp_label(&project_row(p, &widths, None, true), columns))
            .chain(std::iter::once("[Quit]".to_string()))
            .collect();

        let idx = Select::with_theme(&theme)
            .with_prompt(format!("Projects ({} shown) — pick one", filtered.len()))
            .items(&labels)
            .default(0)
            .interact()?;

        if idx == filtered.len() {
            return Ok(());
        }

        match project_action_menu(filtered[idx], None, false)? {
            ActionLoop::BackToList => continue,
            // Preserve the command picker's existing behaviour: it returns to
            // its current list after a mutation. The guided browser passes
            // `reload_after_change = true` and refreshes its owned rows.
            ActionLoop::Changed(_) => continue,
            ActionLoop::Quit => return Ok(()),
        }
    }
}

/// `fastf open <query>` — resolve a project and reveal it in the file manager.
pub fn open(query: &str) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, query)?;

    if !project.path.exists() {
        anyhow::bail!(
            "project '{}' no longer exists on disk at {}",
            project.id,
            crate::util::paths::display_path(&project.path)
        );
    }
    println!(
        "{} Opening {} ({})",
        "→".cyan().bold(),
        project.name.bold(),
        crate::util::paths::display_path(&project.path).dimmed()
    );
    crate::core::post_create::reveal_folder(&project.path)
}
