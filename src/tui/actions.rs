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
use crate::tui::prompt::{self, TextOpts};
use crate::tui::rows::{PENDING_LABEL, size_label};
use crate::util::size_scan::SizeCell;

/// What a project action asks its list to do next.
///
/// The distinction that matters is `Patched` versus `Reload`. Every mutation
/// used to be reported as one boolean, and the guided browser answered it by
/// re-running `library::discover` across every base — so adding a tag to one
/// project re-read every `PROJECT_INFO.md` in the library, and in a search
/// browser it re-evaluated the query against all of them too. A content
/// mutation changes one row, and the list already holds that row.
///
/// `stale` names the paths whose size snapshot must be dropped: the new location
/// always, and the old one as well when the folder moved or was renamed.
pub enum ActionLoop {
    BackToList,
    /// One row's content changed; the library's shape did not.
    Patched {
        project: Project,
        stale: Vec<PathBuf>,
    },
    /// The row is gone from the library.
    Removed {
        path: PathBuf,
    },
    /// Something the list cannot reason about. Re-run the loader.
    Reload,
    Quit,
}

/// `size` is `None` for surfaces that show no Size column (`fastf recent` and
/// `fastf search`), and the browser's current cell otherwise.
pub(crate) fn project_action_menu(
    project: &Project,
    size: Option<SizeCell>,
    reload_after_change: bool,
) -> Result<ActionLoop> {
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
        // Probed once when the action menu opens, not `is_dir`-ed per base every
        // time a list is built.
        let (mounted, _unusable) = crate::util::paths::mounted_bases(&all);
        (
            mounted.into_iter().filter(|b| *b != current).collect(),
            default_base,
        )
    };

    loop {
        // Labels, not indices. The move row appears only when there is somewhere
        // to move to, so every index below it used to shift — `move_idx` was a
        // hard-coded `6` that only happened to be right.
        let mut items: Vec<&str> = vec![
            "Open project folder",
            "Show project metadata",
            "Add tag",
            "Remove tag",
            "Add journal note",
            "Show journal",
        ];
        if !other_bases.is_empty() {
            items.push("Move to another base");
        }
        items.push("Rename folder");
        items.push("Unregister (keep files)");
        items.push("Delete folder permanently");
        items.push("Back to list");
        items.push("Back to main menu");

        let labels: Vec<String> = items.iter().map(|item| (*item).to_string()).collect();
        // Esc is "Back to list": the parent of this menu is the list it opened from.
        let Some(choice) = prompt::select("What would you like to do?", &labels, 0)? else {
            return Ok(ActionLoop::BackToList);
        };

        match items[choice] {
            "Open project folder" => {
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
            "Show project metadata" => {
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
            "Add tag" => {
                let Some(input) = prompt::text(
                    "Tag to add (e.g. draft  or  client/Acme)",
                    TextOpts::new().allow_empty(),
                )?
                else {
                    continue;
                };
                let tag = input.trim().to_string();
                if tag.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                    continue;
                }
                match crate::core::operations::add_tags(project, &[tag]) {
                    Ok(tags) => {
                        println!(
                            "{}  Added 1 tag to {}",
                            "✓".green().bold(),
                            project.id.green().bold()
                        );
                        if reload_after_change {
                            return Ok(retagged(project, tags));
                        }
                    }
                    Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                }
            }
            "Remove tag" => {
                let Some(input) = prompt::text("Tag to remove", TextOpts::new().allow_empty())?
                else {
                    continue;
                };
                let tag = input.trim().to_string();
                if tag.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                    continue;
                }
                match crate::core::operations::remove_tags(project, &[tag]) {
                    Ok(tags) => {
                        println!(
                            "{}  Removed 1 tag from {}",
                            "✓".green().bold(),
                            project.id.green().bold()
                        );
                        if reload_after_change {
                            return Ok(retagged(project, tags));
                        }
                    }
                    Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                }
            }
            "Add journal note" => {
                let Some(input) = prompt::text("Journal note", TextOpts::new().allow_empty())?
                else {
                    continue;
                };
                let msg = input.trim().to_string();
                if msg.is_empty() {
                    println!("{}", "  (cancelled)".dimmed());
                    continue;
                }
                if let Err(e) = crate::core::operations::append_note(project, &msg) {
                    eprintln!("{} {}", "error:".red().bold(), e);
                } else {
                    println!("{}  Journal entry added.", "✓".green().bold());
                    if reload_after_change {
                        // The journal is not shown in a row, but the row's size
                        // is now wrong, so the snapshot has to go.
                        return Ok(ActionLoop::Patched {
                            project: project.clone(),
                            stale: vec![project.path.clone()],
                        });
                    }
                }
            }
            "Show journal" => show_journal(path),
            "Move to another base" => {
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
                        let stale = vec![project.path.clone(), moved.path.clone()];
                        return Ok(ActionLoop::Patched {
                            project: moved,
                            stale,
                        });
                    }
                    Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                }
            }
            "Rename folder" => {
                let Some(new_name) = prompt::text(
                    "New folder name",
                    TextOpts::new().initial(project.name.clone()),
                )?
                else {
                    continue;
                };
                match crate::core::operations::rename(project, &new_name) {
                    Ok(renamed) => {
                        println!("{}  Renamed to {}", "✓".green().bold(), renamed.name.bold());
                        let stale = vec![project.path.clone(), renamed.path.clone()];
                        return Ok(ActionLoop::Patched {
                            project: renamed,
                            stale,
                        });
                    }
                    Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                }
            }
            "Unregister (keep files)" => {
                let confirmed = prompt::confirm(
                    &format!(
                        "Remove PROJECT_INFO.md from '{}'? The files stay on disk; fastf just forgets the project",
                        project.name
                    ),
                    false,
                )?
                .unwrap_or(false);
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
                        return Ok(ActionLoop::Removed {
                            path: project.path.clone(),
                        });
                    }
                    Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                }
            }
            "Delete folder permanently" => {
                println!(
                    "  {} this permanently deletes {} and everything inside it.",
                    "warning:".red().bold(),
                    path_str.bold()
                );
                let Some(typed) = prompt::text(
                    &format!("Type the folder name '{}' to confirm", project.name),
                    TextOpts::new().allow_empty(),
                )?
                else {
                    continue;
                };
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
                        return Ok(ActionLoop::Removed {
                            path: project.path.clone(),
                        });
                    }
                    Err(e) => eprintln!("{} {}", "error:".red().bold(), e),
                }
            }
            "Back to list" => return Ok(ActionLoop::BackToList),
            "Back to main menu" => return Ok(ActionLoop::Quit),
            other => anyhow::bail!("unhandled action '{other}'"),
        }
    }
}

/// The same row with its tag list replaced.
///
/// The tags are the only field a tag mutation can change, and `mutate_tags`
/// already returns the new list, so the row is patched from what the operation
/// reported rather than by reading the file back.
fn retagged(project: &Project, tags: Vec<String>) -> ActionLoop {
    let mut patched = project.clone();
    patched.tags = tags;
    ActionLoop::Patched {
        // The tag was written into `PROJECT_INFO.md`, so the folder is a few
        // bytes bigger than the snapshot says. Measure it again.
        stale: vec![project.path.clone()],
        project: patched,
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
