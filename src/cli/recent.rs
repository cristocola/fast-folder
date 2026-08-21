use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::core::config::Config;
use crate::core::library::{self, Project};
use crate::util::size_scan::{SizeCell, SizeScanner};

/// How often the guided browser looks for newly measured folder sizes. The same
/// cadence `fastf move` draws its progress at: fast enough to look live, slow
/// enough that a network scan is not competing with the terminal for I/O.
const SIZE_TICK: Duration = Duration::from_millis(200);

/// Width of the Size cell, fixed at the widest value it can hold
/// (`unavailable`). Sizing it to the page's current widest value — which is what
/// the old blocking scan did — reflows every row each time a snapshot lands.
const SIZE_CELL: usize = 11;

/// Shown until a row has been measured. Says what is happening, rather than
/// leaving a gap that reads as "empty folder".
const PENDING_LABEL: &str = "scanning…";

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

    let interactive = !args.plain && std::io::stdout().is_terminal();

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
    let id_w = filtered.iter().map(|p| p.id.len()).max().unwrap_or(4);
    let tmpl_w = filtered.iter().map(|p| p.template.len()).max().unwrap_or(8);
    let base_w = filtered
        .iter()
        .map(|p| library::base_label(&p.base).len())
        .max()
        .unwrap_or(4);
    let date_w = 10; // YYYY-MM-DD

    for p in filtered {
        let date = p.created.get(..date_w).unwrap_or(&p.created);
        let path_str = crate::util::paths::display_path(&p.path);
        let missing = !p.path.exists();
        let marker = if missing { "✗".red() } else { "•".cyan() };
        println!(
            "  {} {:<id_w$}  {:<tmpl_w$}  {}  {:<base_w$}  {}",
            marker,
            p.id.green().bold(),
            p.template.dimmed(),
            date.dimmed(),
            library::base_label(&p.base).cyan(),
            if missing {
                format!("{} {}", p.name, "(missing)".red())
            } else {
                p.name.clone()
            },
            id_w = id_w,
            tmpl_w = tmpl_w,
            base_w = base_w,
        );
        println!("      {} {}", "→".dimmed(), path_str.dimmed());
    }
}

/// Interactive picker shared by `fastf recent` and `fastf search`.
///
/// Displays the projects in a `dialoguer::Select` loop.  Selecting a project
/// enters `project_action_menu`.
/// Clamp a Select item label to the terminal width so dialoguer never has to
/// redraw a soft-wrapped line (the Windows console miscounts wrapped rows,
/// leaving ghosted characters as the selection moves). Budget = columns minus
/// the theme's "> " item prefix minus a last-column safety margin. Labels must
/// stay ANSI-free — `truncate_str` is unicode-width-aware, but styled labels
/// would reintroduce the redraw problem this exists to avoid.
fn clamp_label(label: &str, columns: usize) -> String {
    const PREFIX: usize = 3;
    let budget = columns.saturating_sub(PREFIX);
    if budget == 0 {
        // Width unknown (size() reports 0 off-terminal) — leave untouched.
        return label.to_string();
    }
    dialoguer::console::truncate_str(label, budget, "…").into_owned()
}

fn terminal_columns() -> usize {
    let (_rows, columns) = dialoguer::console::Term::stdout().size();
    columns as usize
}

/// A project picker has several distant columns, so the default one-character
/// cursor is not enough to track the selected row across a wide terminal. Keep
/// the labels themselves ANSI-free (important for clamping/redraw correctness),
/// then apply one terminal-native reverse-video strip at render time.
struct ProjectRowTheme {
    content_width: usize,
}

impl ProjectRowTheme {
    fn new(columns: usize) -> Self {
        // Same budget as `clamp_label`: two prefix columns plus one last-column
        // safety margin prevents a highlighted row from soft-wrapping.
        Self {
            content_width: columns.saturating_sub(3),
        }
    }
}

impl dialoguer::theme::Theme for ProjectRowTheme {
    fn format_select_prompt_item(
        &self,
        f: &mut dyn std::fmt::Write,
        text: &str,
        active: bool,
    ) -> std::fmt::Result {
        if !active {
            return write!(f, "  {text}");
        }

        let padded = if self.content_width == 0 {
            std::borrow::Cow::Borrowed(text)
        } else {
            dialoguer::console::pad_str(
                text,
                self.content_width,
                dialoguer::console::Alignment::Left,
                None,
            )
        };
        let row = format!("> {padded}");
        write!(
            f,
            "{}",
            dialoguer::console::Style::new()
                .for_stderr()
                .reverse()
                .bold()
                .apply_to(row)
        )
    }
}

pub fn run_picker(filtered: &[&Project]) -> Result<()> {
    use dialoguer::Select;

    let columns = terminal_columns();
    let theme = ProjectRowTheme::new(columns);
    let id_w = filtered.iter().map(|p| p.id.len()).max().unwrap_or(4);
    let tmpl_w = filtered.iter().map(|p| p.template.len()).max().unwrap_or(8);
    let base_w = filtered
        .iter()
        .map(|p| library::base_label(&p.base).len())
        .max()
        .unwrap_or(4);

    loop {
        let labels: Vec<String> = filtered
            .iter()
            .map(|p| {
                let date = p.created.get(..10).unwrap_or(&p.created);
                let missing = !p.path.exists();
                let suffix = if missing { "  (missing)" } else { "" };
                // Tags come straight from discovery (cache/scan) — no reload.
                let tag_str = if p.tags.is_empty() {
                    String::new()
                } else {
                    let truncated: Vec<&str> = p.tags.iter().map(|t| t.as_str()).take(3).collect();
                    let extra = p.tags.len().saturating_sub(3);
                    if extra > 0 {
                        format!("  [{}  +{}]", truncated.join("  "), extra)
                    } else {
                        format!("  [{}]", truncated.join("  "))
                    }
                };
                format!(
                    "{:<id_w$}  {:<tmpl_w$}  {}  {:<base_w$}  {}{}{}",
                    p.id,
                    p.template,
                    date,
                    library::base_label(&p.base),
                    p.name,
                    suffix,
                    tag_str,
                    id_w = id_w,
                    tmpl_w = tmpl_w,
                    base_w = base_w,
                )
            })
            .map(|label| clamp_label(&label, columns))
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

enum ActionLoop {
    BackToList,
    Changed(Vec<PathBuf>),
    Quit,
}

/// Guided-TUI project browser. Unlike `fastf recent`, this owns and reloads its
/// full result set, pages it, and shows live folder sizes for the current page.
/// `load` is called again after every mutation so search predicates and page
/// bounds remain truthful, and its failure ends the browser: a library that
/// cannot be resolved is not a library to page through.
///
/// Sizes come from `SizeScanner`'s worker threads, and the list is drawn before
/// any of them has answered. That is the whole point: walking a page of project
/// trees takes seconds on a network share, and it used to happen inline, so the
/// list only appeared once every visible row had been measured.
///
/// While the list is up, `util::live_select` owns the terminal, so
/// nothing in here may print — which is why the scan has no progress output of
/// its own, and why the scanner threads are silent by construction.
pub fn run_paged_browser<F>(page_size: usize, empty_message: &str, mut load: F) -> Result<()>
where
    F: FnMut() -> Result<Vec<Project>>,
{
    let page_size = page_size.max(1);
    let mut projects = load()?;
    let mut page = 0_usize;
    // Browser-session snapshots only. Nothing reaches Project or the cache.
    let scanner = SizeScanner::new();

    loop {
        if projects.is_empty() {
            println!("{}", empty_message.dimmed());
            return Ok(());
        }

        let page_count = projects.len().div_ceil(page_size);
        page = page.min(page_count - 1);
        let start = page * page_size;
        let end = (start + page_size).min(projects.len());
        let current = &projects[start..end];
        let paths: Vec<PathBuf> = current.iter().map(|p| p.path.clone()).collect();

        let mut nav: Vec<String> = Vec::new();
        let previous_idx = if page > 0 {
            nav.push("Previous page".to_string());
            Some(current.len() + nav.len() - 1)
        } else {
            None
        };
        let next_idx = if page + 1 < page_count {
            nav.push("Next page".to_string());
            Some(current.len() + nav.len() - 1)
        } else {
            None
        };
        nav.push("Back".to_string());
        let back_idx = current.len() + nav.len() - 1;

        let columns = terminal_columns();
        let theme = ProjectRowTheme::new(columns);
        let prompt = format!(
            "Projects — Page {}/{} ({} total)",
            page + 1,
            page_count,
            projects.len()
        );

        let choice = crate::util::live_select::select_live(&prompt, 0, &theme, SIZE_TICK, |sel| {
            // Re-declare the whole visible page every frame, selected row first:
            // it is the one the user is about to open, and `request` replaces the
            // queue rather than extending it, so moving the selection or turning
            // the page reprioritises straight away.
            scanner.request(&scan_order(&paths, sel));
            let mut labels = paged_labels(current, &scanner.cells_for(&paths), columns);
            labels.extend(nav.iter().cloned());
            labels
        })?;

        if choice < current.len() {
            // Own the selected snapshot so a successful action can reload the
            // backing Vec without keeping a borrow into it.
            let project = current[choice].clone();
            let cell = scanner.cells_for(std::slice::from_ref(&project.path))[0];
            match project_action_menu(&project, Some(cell), true)? {
                ActionLoop::BackToList => {}
                ActionLoop::Changed(paths) => {
                    for path in paths {
                        scanner.forget(&path);
                    }
                    projects = load()?;
                    // `page` is clamped at the top of the loop if the final row
                    // on the last page was removed or stopped matching search.
                }
                ActionLoop::Quit => return Ok(()),
            }
            continue;
        }
        if previous_idx == Some(choice) {
            page -= 1;
            continue;
        }
        if next_idx == Some(choice) {
            page += 1;
            continue;
        }
        if choice == back_idx {
            return Ok(());
        }
        unreachable!();
    }
}

/// The visible page's paths with the selected row first, so the row the user is
/// pointing at is measured next. A navigation row leaves display order alone.
fn scan_order(paths: &[PathBuf], sel: usize) -> Vec<PathBuf> {
    let mut ordered = Vec::with_capacity(paths.len());
    if let Some(selected) = paths.get(sel) {
        ordered.push(selected.clone());
    }
    ordered.extend(
        paths
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != sel)
            .map(|(_, path)| path.clone()),
    );
    ordered
}

/// One label per project, in display order, with `sizes` positionally matching.
///
/// Every column width here is derived from the projects alone, never from the
/// sizes, so a label only ever changes in its own Size cell — the table cannot
/// reflow under the reader as snapshots land.
fn paged_labels(projects: &[Project], sizes: &[SizeCell], columns: usize) -> Vec<String> {
    let id_w = projects.iter().map(|p| p.id.len()).max().unwrap_or(4);
    let tmpl_w = projects.iter().map(|p| p.template.len()).max().unwrap_or(8);
    let base_w = projects
        .iter()
        .map(|p| library::base_label(&p.base).len())
        .max()
        .unwrap_or(4);

    projects
        .iter()
        .enumerate()
        .map(|(idx, project)| {
            let date = project.created.get(..10).unwrap_or(&project.created);
            let size = cell_label(sizes.get(idx).copied().unwrap_or(SizeCell::Pending));
            let tag_str = if project.tags.is_empty() {
                String::new()
            } else {
                let shown: Vec<&str> = project.tags.iter().map(String::as_str).take(3).collect();
                let extra = project.tags.len().saturating_sub(3);
                if extra > 0 {
                    format!("  [{}  +{}]", shown.join("  "), extra)
                } else {
                    format!("  [{}]", shown.join("  "))
                }
            };
            let label = format!(
                "{:<id_w$}  {:<tmpl_w$}  {}  {:<base_w$}  Size {:>size_w$}  {}{}",
                project.id,
                project.template,
                date,
                library::base_label(&project.base),
                size,
                project.name,
                tag_str,
                id_w = id_w,
                tmpl_w = tmpl_w,
                base_w = base_w,
                size_w = SIZE_CELL,
            );
            clamp_label(&label, columns)
        })
        .collect()
}

/// The Size cell for one row. Its fixed width belongs to the caller's format
/// string, not here.
fn cell_label(cell: SizeCell) -> String {
    match cell {
        SizeCell::Pending => PENDING_LABEL.to_string(),
        SizeCell::Known(bytes) => size_label(bytes),
    }
}

fn size_label(size: Option<u64>) -> String {
    let Some(bytes) = size else {
        return "unavailable".to_string();
    };
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes_f < MIB {
        format!("{:.1} KB", bytes_f / KIB)
    } else if bytes_f < GIB {
        format!("{:.1} MB", bytes_f / MIB)
    } else if bytes_f < TIB {
        format!("{:.1} GB", bytes_f / GIB)
    } else {
        format!("{:.1} TB", bytes_f / TIB)
    }
}

/// `size` is `None` for surfaces that show no Size column (`fastf recent` and
/// `fastf search`), and the browser's current cell otherwise.
fn project_action_menu(
    project: &Project,
    size: Option<SizeCell>,
    reload_after_change: bool,
) -> Result<ActionLoop> {
    use dialoguer::{Input, Select};

    let path = project.path.as_path();
    let path_str = crate::util::paths::display_path(path);

    println!();
    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        project.id.green().bold(),
        project.name.bold()
    );
    println!(
        "    {} {}  {} {}  {}",
        "template:".dimmed(),
        project.template,
        "base:".dimmed(),
        library::base_label(&project.base).cyan(),
        path_str.dimmed()
    );
    match size {
        Some(SizeCell::Known(bytes)) => {
            println!("    {} {}", "size:".dimmed(), size_label(bytes))
        }
        // Say a scan is outstanding rather than silently dropping the line: this
        // header is printed once and never repainted, so it must not pretend the
        // size is unknowable.
        Some(SizeCell::Pending) => {
            println!("    {} {}", "size:".dimmed(), PENDING_LABEL.dimmed())
        }
        None => {}
    }

    // Other configured bases this project could move to (mounted ones only).
    let other_bases: Vec<std::path::PathBuf> = {
        let cfg = Config::load()?;
        let current = project
            .base
            .canonicalize()
            .unwrap_or_else(|_| project.base.clone());
        cfg.effective_bases()
            .into_iter()
            .filter(|b| b.is_dir() && *b != current)
            .collect()
    };

    loop {
        let mut items = vec![
            "Open project folder",
            "Show project metadata",
            "Add tag",
            "Remove tag",
            "Add journal note",
            "Show journal",
        ];
        // Only offer a move when there is somewhere to move to.
        if !other_bases.is_empty() {
            items.push("Move to another base");
        }
        items.push("Rename folder");
        items.push("Unregister (keep files)");
        items.push("Delete folder permanently");
        items.push("Back to list");
        items.push("Quit");
        let move_idx = if other_bases.is_empty() {
            usize::MAX
        } else {
            6
        };
        let rename_idx = items.len() - 5;
        let unregister_idx = items.len() - 4;
        let delete_idx = items.len() - 3;
        let back_idx = items.len() - 2;
        let quit_idx = items.len() - 1;

        let choice = Select::new()
            .with_prompt("What would you like to do?")
            .items(&items)
            .default(0)
            .interact()?;

        if choice == move_idx {
            let columns = terminal_columns();
            let mut labels: Vec<String> = other_bases
                .iter()
                .map(|b| {
                    clamp_label(
                        &format!("{}  ({})", library::base_label(b), b.display()),
                        columns,
                    )
                })
                .collect();
            labels.push("[Cancel]".to_string());
            let sel = Select::new()
                .with_prompt("Move to which base?")
                .items(&labels)
                .default(0)
                .interact()?;
            if sel == other_bases.len() {
                continue;
            }
            let progress = std::sync::Mutex::new(crate::core::assets::Progress::new(&[]));
            let cancel = std::sync::atomic::AtomicBool::new(false);
            match crate::core::operations::move_project(
                project,
                &other_bases[sel],
                &progress,
                &cancel,
            ) {
                Ok(outcome) => {
                    let moved = outcome.project;
                    println!(
                        "{}  Moved to {}",
                        "✓".green().bold(),
                        crate::util::paths::display_path(&moved.path).bold()
                    );
                    if outcome.cleanup_pending {
                        eprintln!(
                            "{} destination is complete, but cleanup is pending at {}",
                            "warning:".yellow().bold(),
                            crate::util::paths::display_path(&project.path)
                        );
                    }
                    return Ok(ActionLoop::Changed(vec![project.path.clone(), moved.path]));
                }
                Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
            }
            continue;
        }
        if choice == rename_idx {
            let new_name: String = Input::new()
                .with_prompt("New folder name")
                .with_initial_text(project.name.clone())
                .interact_text()?;
            match crate::core::operations::rename(project, &new_name) {
                Ok(renamed) => {
                    println!("{}  Renamed to {}", "✓".green().bold(), renamed.name.bold());
                    return Ok(ActionLoop::Changed(vec![
                        project.path.clone(),
                        renamed.path,
                    ]));
                }
                Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
            }
            continue;
        }
        if choice == unregister_idx {
            let confirmed = dialoguer::Confirm::new()
                .with_prompt(format!(
                    "Remove PROJECT_INFO.md from '{}'? The files stay on disk; fastf just forgets the project",
                    project.name
                ))
                .default(false)
                .interact()?;
            if !confirmed {
                continue;
            }
            match crate::core::operations::unregister(project) {
                Ok(()) => {
                    println!(
                        "{}  Unregistered {}",
                        "✓".green().bold(),
                        project.name.bold()
                    );
                    return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                }
                Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
            }
            continue;
        }
        if choice == delete_idx {
            println!(
                "  {} this permanently deletes {} and everything inside it.",
                "warning:".red().bold(),
                path_str.bold()
            );
            let typed: String = Input::new()
                .with_prompt(format!(
                    "Type the folder name '{}' to confirm",
                    project.name
                ))
                .allow_empty(true)
                .interact_text()?;
            if typed.trim() != project.name {
                eprintln!(
                    "{} name did not match — nothing deleted",
                    "cancelled:".yellow()
                );
                continue;
            }
            match crate::core::operations::delete(project) {
                Ok(()) => {
                    println!("{}  Deleted {}", "✓".green().bold(), path_str.bold());
                    return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                }
                Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
            }
            continue;
        }
        if choice == back_idx {
            return Ok(ActionLoop::BackToList);
        }
        if choice == quit_idx {
            return Ok(ActionLoop::Quit);
        }

        match choice {
            // Open folder
            0 => {
                if !path.exists() {
                    eprintln!(
                        "{} project folder no longer exists at {}",
                        "warning:".yellow().bold(),
                        path_str
                    );
                    continue;
                }
                if let Err(e) = crate::core::post_create::reveal_folder(path) {
                    eprintln!(
                        "{} could not open folder: {}",
                        "warning:".yellow().bold(),
                        e
                    );
                }
            }
            // Show metadata
            1 => {
                if !path.exists() {
                    eprintln!(
                        "{} project folder no longer exists at {}",
                        "warning:".yellow().bold(),
                        path_str
                    );
                    continue;
                }
                show_metadata(path);
            }
            // Add tag
            2 => {
                let input: String = Input::new()
                    .with_prompt("Tag to add (e.g. draft  or  client/Acme)")
                    .interact_text()?;
                let tag = input.trim().to_string();
                if tag.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else {
                    match crate::core::operations::add_tags(project, &[tag]) {
                        Ok(_) => {
                            println!(
                                "{}  Added 1 tag to {}",
                                "✓".green().bold(),
                                project.id.green().bold()
                            );
                            if reload_after_change {
                                return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                            }
                        }
                        Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                    }
                }
            }
            // Remove tag
            3 => {
                let input: String = Input::new().with_prompt("Tag to remove").interact_text()?;
                let tag = input.trim().to_string();
                if tag.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else {
                    match crate::core::operations::remove_tags(project, &[tag]) {
                        Ok(_) => {
                            println!(
                                "{}  Removed 1 tag from {}",
                                "✓".green().bold(),
                                project.id.green().bold()
                            );
                            if reload_after_change {
                                return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                            }
                        }
                        Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                    }
                }
            }
            // Add journal note
            4 => {
                let input: String = Input::new().with_prompt("Journal note").interact_text()?;
                let msg = input.trim().to_string();
                if msg.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else {
                    if let Err(e) = crate::core::operations::append_note(project, &msg) {
                        eprintln!("{} {}", "error:".red().bold(), e);
                    } else {
                        println!("{}  Journal entry added.", "✓".green().bold());
                        if reload_after_change {
                            return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                        }
                    }
                }
            }
            // Show journal
            5 => {
                show_journal(path);
            }
            // Move / Back / Quit are handled above the match (dynamic indices).
            _ => unreachable!(),
        }
    }
}

/// Render a project's metadata to stdout.
fn show_metadata(project_root: &Path) {
    use crate::core::project_info;

    println!();
    let banner = "─────  Project metadata  ─────";
    println!("{}", banner.dimmed());

    match project_info::read_metadata(project_root) {
        Ok(Some(meta)) => print_structured_metadata(&meta, project_root),
        Ok(None) => match project_info::read(project_root) {
            Ok(raw) => {
                println!(
                    "{}",
                    "(no YAML frontmatter — showing raw file contents)".dimmed()
                );
                println!();
                print!("{}", raw);
            }
            Err(e) => println!("  {}", e.to_string().yellow()),
        },
        Err(e) => println!("  {}", e.to_string().yellow()),
    }

    println!("{}", "─".repeat(banner.chars().count()).dimmed());
}

/// Aligned `key  value` printer for parsed frontmatter.
fn print_structured_metadata(meta: &crate::core::project_info::Metadata, project_root: &Path) {
    // Base = the project folder's parent (depth-1 discovery). Derived live, not
    // from frontmatter, so it stays truthful after external moves.
    let base = project_root
        .parent()
        .map(|b| b.display().to_string())
        .unwrap_or_default();
    // Top-level scalar fields, in a readable order (not alphabetical — id first).
    let scalars: [(&str, &str); 7] = [
        ("id", &meta.id),
        ("template", &meta.template),
        ("template_name", &meta.template_name),
        ("created", &meta.created),
        ("folder", &meta.folder),
        ("base", &base),
        ("path", &meta.path),
    ];

    let scalar_w = scalars
        .iter()
        .map(|(k, _)| k.len())
        .chain(std::iter::once("variables".len()))
        .chain(std::iter::once("tags".len()))
        .max()
        .unwrap_or(8);

    for (k, v) in scalars {
        println!("{:<w$}  {}", k.cyan(), v, w = scalar_w);
    }

    if !meta.tags.is_empty() {
        println!();
        println!("{}", "tags:".cyan());
        for tag in &meta.tags {
            println!("  {} {}", "•".yellow(), tag.yellow());
        }
    }

    if !meta.variables.is_empty() {
        println!();
        println!("{}", "variables:".cyan());
        let var_w = meta.variables.keys().map(|k| k.len()).max().unwrap_or(8);
        for (k, v) in &meta.variables {
            let display = if v.is_empty() {
                "(empty)".dimmed().to_string()
            } else {
                v.clone()
            };
            println!("  {:<w$}  {}", k, display, w = var_w);
        }
    }
}

/// Render the journal section for a project.
fn show_journal(project_root: &Path) {
    use crate::core::project_info;

    println!();
    let banner = "─────  Journal  ─────";
    println!("{}", banner.dimmed());

    match project_info::read_journal_entries(project_root) {
        Ok(entries) if entries.is_empty() => {
            println!("  {}", "(no journal entries yet)".dimmed());
        }
        Ok(entries) => {
            for entry in &entries {
                // See `cli::note::notes` — a hand-edited timestamp must not
                // panic the picker.
                let date = entry.timestamp.get(..10).unwrap_or(&entry.timestamp);
                println!("  {} {}  {}", "•".dimmed(), date.dimmed(), entry.message);
            }
        }
        Err(e) => println!("  {}", e.to_string().yellow()),
    }

    println!("{}", "─".repeat(banner.chars().count()).dimmed());
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

#[cfg(test)]
mod tests {
    use super::{
        PENDING_LABEL, Project, ProjectRowTheme, SizeCell, clamp_label, paged_labels, scan_order,
        size_label,
    };
    use dialoguer::console::measure_text_width;
    use dialoguer::theme::Theme;
    use std::path::PathBuf;

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.to_string(),
            template: "general".to_string(),
            template_name: "General".to_string(),
            name: name.to_string(),
            path: PathBuf::from("/base").join(name),
            base: PathBuf::from("/base"),
            created: "2026-08-18T00:00:00Z".to_string(),
            tags: Vec::new(),
            exists: true,
        }
    }

    #[test]
    fn clamp_leaves_short_labels_unchanged() {
        assert_eq!(
            clamp_label("ID0001  general  proj", 80),
            "ID0001  general  proj"
        );
    }

    #[test]
    fn clamp_elides_long_labels_within_budget() {
        let label = "x".repeat(200);
        let out = clamp_label(&label, 40);
        assert!(out.ends_with('…'));
        assert!(measure_text_width(&out) <= 37);
    }

    #[test]
    fn clamp_is_wide_char_safe() {
        // CJK chars are double-width; the clamp must count display columns,
        // not chars, and never split a wide char in half.
        let label = "プロジェクト".repeat(20);
        let out = clamp_label(&label, 30);
        assert!(out.ends_with('…'));
        assert!(measure_text_width(&out) <= 27);
    }

    #[test]
    fn clamp_passes_through_when_width_unknown() {
        let label = "y".repeat(200);
        assert_eq!(clamp_label(&label, 0), label);
    }

    #[test]
    fn size_labels_cover_bytes_through_terabytes() {
        assert_eq!(size_label(Some(0)), "0 B");
        assert_eq!(size_label(Some(1024)), "1.0 KB");
        assert_eq!(size_label(Some(1024_u64.pow(2))), "1.0 MB");
        assert_eq!(size_label(Some(1024_u64.pow(3))), "1.0 GB");
        assert_eq!(size_label(Some(1024_u64.pow(4))), "1.0 TB");
        assert_eq!(size_label(None), "unavailable");
    }

    /// The reason the Size cell is a fixed width. The old browser sized the
    /// column to the page's widest value, so every row shifted sideways each time
    /// a background snapshot landed — unreadable while a page fills in.
    #[test]
    fn a_landing_size_does_not_reflow_the_row() {
        let projects = [project("ID0001", "Alpha"), project("ID0002", "Beta")];
        let pending = paged_labels(&projects, &[SizeCell::Pending; 2], 200);
        let known = paged_labels(
            &projects,
            &[
                SizeCell::Known(Some(2048)),
                // The widest cell there is, and the one most likely to stretch a
                // column that was measured from its contents.
                SizeCell::Known(None),
            ],
            200,
        );

        // Compared in display columns, not bytes: the pending cell's "…" is three
        // bytes wide and one column wide, and it is the column that has to line
        // up. (Rust pads to a char count, which equals the column count for every
        // character these cells can hold.)
        for (before, after) in pending.iter().zip(known.iter()) {
            assert_eq!(
                name_column(before),
                name_column(after),
                "the name column moved when a size landed:\n{before}\n{after}"
            );
            assert_eq!(measure_text_width(before), measure_text_width(after));
        }
        assert!(pending[0].contains(PENDING_LABEL));
        assert!(known[0].contains("2.0 KB"));
        assert!(known[1].contains("unavailable"));
    }

    /// Which terminal column a row's project name starts at.
    fn name_column(label: &str) -> usize {
        let at = label
            .find("Alpha")
            .or_else(|| label.find("Beta"))
            .expect("label carries a project name");
        measure_text_width(&label[..at])
    }

    /// The row the user is pointing at is the one they are about to open, so it
    /// must be measured next rather than in display order.
    #[test]
    fn the_selected_row_is_measured_first() {
        let paths: Vec<PathBuf> = ["a", "b", "c"].iter().map(PathBuf::from).collect();

        assert_eq!(
            scan_order(&paths, 2),
            vec![PathBuf::from("c"), PathBuf::from("a"), PathBuf::from("b"),]
        );
        assert_eq!(scan_order(&paths, 0), paths);
        // A navigation row: nothing is promoted, and nothing is lost.
        assert_eq!(scan_order(&paths, 7), paths);
    }

    #[test]
    fn selected_project_row_highlight_spans_the_safe_terminal_width() {
        let theme = ProjectRowTheme::new(24);
        let mut rendered = String::new();
        theme
            .format_select_prompt_item(&mut rendered, "ID001  Project", true)
            .unwrap();

        let plain = dialoguer::console::strip_ansi_codes(&rendered);
        assert!(plain.starts_with("> ID001  Project"));
        assert_eq!(measure_text_width(&plain), 23);
        assert!(plain.ends_with(' '), "selected row should fill the row");

        let mut inactive = String::new();
        theme
            .format_select_prompt_item(&mut inactive, "ID001  Project", false)
            .unwrap();
        assert_eq!(inactive, "  ID001  Project");
    }
}
