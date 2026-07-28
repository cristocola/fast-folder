use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;
use std::path::Path;

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

fn terminal_columns() -> usize {
    let (_rows, columns) = dialoguer::console::Term::stdout().size();
    columns as usize
}

pub fn run_picker(filtered: &[&Project]) -> Result<()> {
    use dialoguer::Select;

    let columns = terminal_columns();
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

        let idx = Select::new()
            .with_prompt(format!("Projects ({} shown) — pick one", filtered.len()))
            .items(&labels)
            .default(0)
            .interact()?;

        if idx == filtered.len() {
            return Ok(());
        }

        match project_action_menu(filtered[idx])? {
            ActionLoop::BackToList => continue,
            ActionLoop::Quit => return Ok(()),
        }
    }
}

enum ActionLoop {
    BackToList,
    Quit,
}

fn project_action_menu(project: &Project) -> Result<ActionLoop> {
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
            match library::move_project(project, &other_bases[sel]) {
                Ok(moved) => {
                    println!(
                        "{}  Moved to {}",
                        "✓".green().bold(),
                        crate::util::paths::display_path(&moved.path).bold()
                    );
                    // The in-memory list still shows the old path — go back so
                    // the user re-enters from a fresh `recent`/`search`.
                    return Ok(ActionLoop::BackToList);
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
            match library::rename_project(project, &new_name) {
                Ok(renamed) => {
                    println!("{}  Renamed to {}", "✓".green().bold(), renamed.name.bold());
                    // The in-memory list still shows the old name — go back so
                    // the user re-enters from a fresh `recent`/`search`.
                    return Ok(ActionLoop::BackToList);
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
            match library::unregister_project(project) {
                Ok(()) => {
                    println!(
                        "{}  Unregistered {}",
                        "✓".green().bold(),
                        project.name.bold()
                    );
                    return Ok(ActionLoop::BackToList);
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
            match library::delete_project(project) {
                Ok(()) => {
                    println!("{}  Deleted {}", "✓".green().bold(), path_str.bold());
                    return Ok(ActionLoop::BackToList);
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
                } else if let Err(e) = crate::cli::tag::add(&project.id, &[tag]) {
                    eprintln!("{} {}", "error:".red().bold(), e);
                }
            }
            // Remove tag
            3 => {
                let input: String = Input::new().with_prompt("Tag to remove").interact_text()?;
                let tag = input.trim().to_string();
                if tag.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else if let Err(e) = crate::cli::tag::remove(&project.id, &[tag]) {
                    eprintln!("{} {}", "error:".red().bold(), e);
                }
            }
            // Add journal note
            4 => {
                let input: String = Input::new().with_prompt("Journal note").interact_text()?;
                let msg = input.trim().to_string();
                if msg.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                } else {
                    let pinfo = crate::core::project_info::pinfo_path(path);
                    if let Err(e) = crate::core::project_info::append_journal_entry(&pinfo, &msg) {
                        eprintln!("{} {}", "error:".red().bold(), e);
                    } else {
                        println!("{}  Journal entry added.", "✓".green().bold());
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
    use super::clamp_label;
    use dialoguer::console::measure_text_width;

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
}
