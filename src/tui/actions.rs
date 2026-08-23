//! The project action menu, and the two read-only views it opens.
//!
//! Reached from every project list: the guided browser, `fastf recent`, and
//! `fastf search`.

use anyhow::Result;
use colored::Colorize;
use std::path::{Path, PathBuf};

use crate::core::config::Config;
use crate::core::library::{self, Project};
use crate::tui::pickers::pick_base;
use crate::tui::rows::{PENDING_LABEL, size_label};
use crate::util::size_scan::SizeCell;

/// What a project action asks its list to do next. `Changed` carries every path
/// whose contents moved, so the caller can drop stale size snapshots.
pub enum ActionLoop {
    BackToList,
    Changed(Vec<PathBuf>),
    Quit,
}

/// `size` is `None` for surfaces that show no Size column (`fastf recent` and
/// `fastf search`), and the browser's current cell otherwise.
pub(crate) fn project_action_menu(
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
    let (other_bases, default_base): (Vec<PathBuf>, Option<PathBuf>) = {
        let cfg = Config::load()?;
        let current = project
            .base
            .canonicalize()
            .unwrap_or_else(|_| project.base.clone());
        let all = cfg.effective_bases();
        let default_base = all.first().cloned();
        (
            all.into_iter()
                .filter(|b| b.is_dir() && *b != current)
                .collect(),
            default_base,
        )
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
            let Some(target) = pick_base(
                "Move to which base?",
                &other_bases,
                default_base.as_deref(),
                "name the target instead: `fastf move <query> <base>`",
                true,
            )?
            else {
                continue;
            };
            let progress = std::sync::Mutex::new(crate::core::assets::Progress::new(&[]));
            let cancel = std::sync::atomic::AtomicBool::new(false);
            match crate::core::operations::move_project(project, &target, &progress, &cancel) {
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
