use anyhow::Result;
use colored::Colorize;
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::core::config::Config;
use crate::core::library::{self, Project};

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
    let cfg = Config::load().unwrap_or_default();
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

pub(crate) fn terminal_columns() -> usize {
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
    let library_tags = library::known_tags(filtered.iter().copied());
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

        // The standalone commands have no session log; the events are dropped.
        let mut events = Vec::new();
        match project_action_menu(filtered[idx], None, &library_tags, false, &mut events)? {
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

/// What an action-menu mutation actually did, so the guided TUI can show it in
/// its session log.
///
/// Only successful mutations produce one — opening a folder or reading metadata
/// changes nothing and reports nothing. The standalone `fastf recent` / `fastf
/// search` commands collect these into a Vec they drop: they have no session to
/// log into, and giving them one would mean printing a summary nobody asked for.
pub struct ActionEvent {
    pub verb: &'static str,
    pub subject: String,
}

impl ActionEvent {
    fn new(verb: &'static str, subject: impl Into<String>) -> Self {
        Self {
            verb,
            subject: subject.into(),
        }
    }
}

/// Guided-TUI project browser. Unlike `fastf recent`, this owns and reloads its
/// full result set and pages it. `load` is called again after every mutation so
/// search predicates and page bounds remain truthful.
///
/// Sizes are measured **on demand**, when a project's action menu opens — never
/// per page. Scanning a page up front stalled the list for as long as the
/// slowest tree took to walk, which on a fuse/network base is seconds of a
/// terminal that looks hung. The measurement is cached for the browser session
/// and invalidated for any path a mutation touched.
pub fn run_paged_browser<F>(
    page_size: usize,
    empty_message: &str,
    mut load: F,
) -> Result<Vec<ActionEvent>>
where
    F: FnMut() -> Vec<Project>,
{
    use dialoguer::Select;

    let page_size = page_size.max(1);
    let mut projects = load();
    let mut page = 0_usize;
    // Browser-session snapshots only. Nothing reaches Project or the cache.
    let mut sizes: HashMap<PathBuf, Option<u64>> = HashMap::new();
    let mut events: Vec<ActionEvent> = Vec::new();

    loop {
        if projects.is_empty() {
            println!("{}", empty_message.dimmed());
            return Ok(events);
        }

        // Recomputed per iteration so a tag added a moment ago is offered as a
        // suggestion on the next visit; `projects` is reloaded after every
        // mutation anyway.
        let library_tags = library::known_tags(projects.iter());

        let page_count = projects.len().div_ceil(page_size);
        page = page.min(page_count - 1);
        let start = page * page_size;
        let end = (start + page_size).min(projects.len());
        let current = &projects[start..end];

        let columns = terminal_columns();
        let mut labels = paged_labels(current, columns);
        let previous_idx = if page > 0 {
            let idx = Some(labels.len());
            labels.push("Previous page".to_string());
            idx
        } else {
            None
        };
        let next_idx = if page + 1 < page_count {
            let idx = Some(labels.len());
            labels.push("Next page".to_string());
            idx
        } else {
            None
        };
        let back_idx = labels.len();
        labels.push("Back".to_string());

        let theme = ProjectRowTheme::new(columns);
        let choice = Select::with_theme(&theme)
            .with_prompt(format!(
                "Projects — Page {}/{} ({} total)",
                page + 1,
                page_count,
                projects.len()
            ))
            .items(&labels)
            .default(0)
            .interact()?;

        if choice < current.len() {
            // Own the selected snapshot so a successful action can reload the
            // backing Vec without keeping a borrow into it.
            let project = current[choice].clone();
            let size = *sizes
                .entry(project.path.clone())
                .or_insert_with(|| measure_size(&project.path));
            match project_action_menu(&project, Some(size), &library_tags, true, &mut events)? {
                ActionLoop::BackToList => {}
                ActionLoop::Changed(paths) => {
                    for path in paths {
                        sizes.remove(&path);
                    }
                    projects = load();
                    // `page` is clamped at the top of the loop if the final row
                    // on the last page was removed or stopped matching search.
                }
                ActionLoop::Quit => return Ok(events),
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
            return Ok(events);
        }
        unreachable!();
    }
}

/// One on-demand size snapshot, with a visible line while the walk runs.
///
/// `directory_size` has no progress callback and no cancellation, so a slow
/// filesystem (ntfs-3g, a network share) shows a static line for the whole
/// walk. That is acceptable now only because this measures **one** project —
/// the project the user just chose — instead of a whole page of them.
fn measure_size(path: &Path) -> Option<u64> {
    const NOTICE: &str = "  measuring folder size…";
    print!("{NOTICE}");
    let _ = std::io::stdout().flush();
    let size = crate::util::tree_size::directory_size(path);
    // Wipe the notice so the action-menu header starts on a clean line.
    print!("\r{}\r", " ".repeat(NOTICE.chars().count()));
    let _ = std::io::stdout().flush();
    size
}

fn paged_labels(projects: &[Project], columns: usize) -> Vec<String> {
    let id_w = projects.iter().map(|p| p.id.len()).max().unwrap_or(4);
    let tmpl_w = projects.iter().map(|p| p.template.len()).max().unwrap_or(8);
    let base_w = projects
        .iter()
        .map(|p| library::base_label(&p.base).len())
        .max()
        .unwrap_or(4);

    projects
        .iter()
        .map(|project| {
            let date = project.created.get(..10).unwrap_or(&project.created);
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
                "{:<id_w$}  {:<tmpl_w$}  {}  {:<base_w$}  {}{}",
                project.id,
                project.template,
                date,
                library::base_label(&project.base),
                project.name,
                tag_str,
                id_w = id_w,
                tmpl_w = tmpl_w,
                base_w = base_w,
            );
            clamp_label(&label, columns)
        })
        .collect()
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

/// `size`: the outer `None` means "this caller does not show a size at all"
/// (`fastf recent` / `fastf search`); `Some(inner)` is a measurement, where the
/// inner `None` is `tree_size`'s honest "unavailable".
///
/// `library_tags`: what "Add tag" offers as suggestions. Supplied by the caller
/// from the project list it already holds — see `library::known_tags`.
///
/// `events`: one entry per successful mutation, for the guided menu's session
/// log. Read-only actions push nothing.
fn project_action_menu(
    project: &Project,
    size: Option<Option<u64>>,
    library_tags: &[String],
    reload_after_change: bool,
    events: &mut Vec<ActionEvent>,
) -> Result<ActionLoop> {
    use dialoguer::{Input, MultiSelect, Select};

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
    if let Some(size) = size {
        println!("    {} {}", "size:".dimmed(), size_label(size));
    }

    // Other configured bases this project could move to (mounted ones only).
    let other_bases: Vec<std::path::PathBuf> = {
        let cfg = Config::load().unwrap_or_default();
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
            "Open in editor",
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
        // Found by name, not by a hard-coded index: the tail actions below have
        // always been index-independent, but this one was a literal that had to
        // be renumbered by hand every time a row was inserted above it.
        let move_idx = items
            .iter()
            .position(|item| *item == "Move to another base")
            .unwrap_or(usize::MAX);
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
                    events.push(ActionEvent::new(
                        "moved",
                        format!("{} → {}", project.id, library::base_label(&moved.base)),
                    ));
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
                    events.push(ActionEvent::new("renamed", renamed.name.clone()));
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
                    events.push(ActionEvent::new("unregistered", project.name.clone()));
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
                    events.push(ActionEvent::new("deleted", project.name.clone()));
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
            // Open in editor
            1 => {
                if !path.exists() {
                    eprintln!(
                        "{} project folder no longer exists at {}",
                        "warning:".yellow().bold(),
                        path_str
                    );
                    continue;
                }
                let cfg = Config::load().unwrap_or_default();
                let editor = cfg.resolve_editor();
                match crate::core::post_create::open_in_editor(&cfg, path) {
                    Ok(()) => println!("  {} opened in {}", "✓".green(), editor),
                    Err(e) => eprintln!(
                        "{} could not open editor '{}': {}",
                        "warning:".yellow().bold(),
                        editor,
                        e
                    ),
                }
            }
            // Show metadata
            2 => {
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
            3 => {
                let current = current_tags(project);
                let offer: Vec<String> = library_tags
                    .iter()
                    .filter(|tag| !current.contains(tag))
                    .cloned()
                    .collect();
                let chosen = prompt_tags_to_add(&offer)?;
                if chosen.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else {
                    match crate::core::operations::add_tags(project, &chosen) {
                        Ok(_) => {
                            println!(
                                "{}  Added {} to {}",
                                "✓".green().bold(),
                                plural_tags(chosen.len()),
                                project.id.green().bold()
                            );
                            events.push(ActionEvent::new(
                                "tagged",
                                format!("{}  +{}", project.id, chosen.join(" +")),
                            ));
                            if reload_after_change {
                                return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                            }
                        }
                        Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                    }
                }
            }
            // Remove tag
            4 => {
                let current = current_tags(project);
                if current.is_empty() {
                    println!("  {} no tags to remove.", "·".dimmed());
                    continue;
                }
                let picks = MultiSelect::new()
                    .with_prompt("Tags to remove (Space to toggle, Enter to confirm)")
                    .items(&current)
                    .interact()?;
                // Indices refer to the list the user just saw; resolve them to
                // values before anything is written, exactly as
                // `edit_postcreate_commands` does.
                let targets: Vec<String> = picks
                    .into_iter()
                    .filter_map(|index| current.get(index).cloned())
                    .collect();
                if targets.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else {
                    match crate::core::operations::remove_tags(project, &targets) {
                        Ok(_) => {
                            println!(
                                "{}  Removed {} from {}",
                                "✓".green().bold(),
                                plural_tags(targets.len()),
                                project.id.green().bold()
                            );
                            events.push(ActionEvent::new(
                                "untagged",
                                format!("{}  -{}", project.id, targets.join(" -")),
                            ));
                            if reload_after_change {
                                return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                            }
                        }
                        Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                    }
                }
            }
            // Add journal note
            5 => {
                let input: String = Input::new().with_prompt("Journal note").interact_text()?;
                let msg = input.trim().to_string();
                if msg.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else {
                    if let Err(e) = crate::core::operations::append_note(project, &msg) {
                        eprintln!("{} {}", "error:".red().bold(), e);
                    } else {
                        println!("{}  Journal entry added.", "✓".green().bold());
                        events.push(ActionEvent::new("noted", project.id.clone()));
                        if reload_after_change {
                            return Ok(ActionLoop::Changed(vec![project.path.clone()]));
                        }
                    }
                }
            }
            // Show journal
            6 => {
                show_journal(path);
            }
            // Move / Back / Quit are handled above the match (dynamic indices).
            _ => unreachable!(),
        }
    }
}

fn plural_tags(count: usize) -> String {
    // "Added 1 tag" / "Removed 3 tags".
    format!("{count} tag{}", if count == 1 { "" } else { "s" })
}

/// This project's tags as they are on disk right now.
///
/// The `Project` came from discovery and goes stale the moment this menu
/// mutates it — `run_picker` in particular never reloads, so a Remove following
/// an Add in the same visit would otherwise offer the pre-Add list. Falls back
/// to the discovered snapshot if the metadata cannot be read, which is the same
/// degrade-never-fail rule the rest of the display code follows.
fn current_tags(project: &Project) -> Vec<String> {
    crate::core::project_info::read_metadata(&project.path)
        .ok()
        .flatten()
        .map(|metadata| metadata.tags)
        .unwrap_or_else(|| project.tags.clone())
}

/// Ask which tags to add. Empty result means "cancelled".
///
/// With nothing to suggest (a fresh library, or a project that already carries
/// every tag) this is the plain free-text prompt it has always been. Otherwise
/// the library's existing tags are offered as checkboxes with a trailing row
/// for typing a new one — picking from a list is what stops `draft`/`drafts`
/// drift, and multi-select means related tags land in one mutation.
fn prompt_tags_to_add(offer: &[String]) -> Result<Vec<String>> {
    use dialoguer::{Input, MultiSelect};

    let ask_freeform = || -> Result<Option<String>> {
        let input: String = Input::new()
            .with_prompt("Tag to add (e.g. draft  or  client/Acme)")
            .interact_text()?;
        let tag = input.trim().to_string();
        Ok((!tag.is_empty()).then_some(tag))
    };

    if offer.is_empty() {
        return Ok(ask_freeform()?.into_iter().collect());
    }

    const NEW_TAG_ROW: &str = "+ type a new tag…";
    let mut items: Vec<&str> = offer.iter().map(String::as_str).collect();
    items.push(NEW_TAG_ROW);
    let sentinel = offer.len();

    let picks = MultiSelect::new()
        .with_prompt("Tags to add (Space to toggle, Enter to confirm)")
        .items(&items)
        .interact()?;

    // Resolve indices to values before anything is written, and never let the
    // sentinel row become a literal tag named "+ type a new tag…".
    let mut chosen: Vec<String> = picks
        .iter()
        .filter(|&&index| index != sentinel)
        .filter_map(|&index| offer.get(index).cloned())
        .collect();
    if picks.contains(&sentinel)
        && let Some(typed) = ask_freeform()?
        && !chosen.contains(&typed)
    {
        chosen.push(typed);
    }
    Ok(chosen)
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
    let cfg = Config::load().unwrap_or_default();
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
    use super::{ProjectRowTheme, clamp_label, size_label};
    use dialoguer::console::measure_text_width;
    use dialoguer::theme::Theme;

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
