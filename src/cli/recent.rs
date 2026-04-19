use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::io::IsTerminal;
use std::path::Path;

use crate::core::config::Config;
use crate::core::index::{self, ProjectRecord};

pub struct RecentArgs {
    /// None = use Config::recent_default_limit.
    pub limit: Option<usize>,
    pub template: Option<String>,
    pub since: Option<String>,
    pub prune: bool,
    /// Force the plain (non-interactive) list output. Auto-engages when stdout
    /// is not a TTY.
    pub plain: bool,
}

pub fn run(args: RecentArgs) -> Result<()> {
    let cfg = Config::load().unwrap_or_default();
    let limit = args.limit.unwrap_or(cfg.recent_default_limit).max(1);

    let records = index::load_all()?;

    if args.prune {
        return prune(&records);
    }

    if records.is_empty() {
        println!(
            "{}",
            "No projects yet — create one with `fastf new`.".dimmed()
        );
        return Ok(());
    }

    let filtered = filter_records(&records, &args.template, &args.since, limit);

    if filtered.is_empty() {
        println!("{}", "No projects match those filters.".dimmed());
        return Ok(());
    }

    let interactive = !args.plain && std::io::stdout().is_terminal();

    if interactive {
        interactive_picker(&filtered, &cfg)
    } else {
        print_plain(&filtered);
        Ok(())
    }
}

fn filter_records<'a>(
    records: &'a [ProjectRecord],
    template: &Option<String>,
    since: &Option<String>,
    limit: usize,
) -> Vec<&'a ProjectRecord> {
    // Records are append-only so newest is at the end. Reverse + filter + take.
    let mut filtered: Vec<&ProjectRecord> = records
        .iter()
        .rev()
        .filter(|r| {
            if let Some(slug) = template
                && &r.template != slug
            {
                return false;
            }
            if let Some(since) = since
                && r.created_at.as_str() < since.as_str()
            {
                return false;
            }
            true
        })
        .collect();

    filtered.truncate(limit);
    filtered
}

fn print_plain(filtered: &[&ProjectRecord]) {
    let id_w = filtered.iter().map(|r| r.id.len()).max().unwrap_or(4);
    let tmpl_w = filtered.iter().map(|r| r.template.len()).max().unwrap_or(8);
    let date_w = 10; // YYYY-MM-DD

    for r in filtered {
        let date = r.created_at.get(..date_w).unwrap_or(&r.created_at);
        let missing = !Path::new(&r.path).exists();
        let marker = if missing { "✗".red() } else { "•".cyan() };
        println!(
            "  {} {:<id_w$}  {:<tmpl_w$}  {}  {}",
            marker,
            r.id.green().bold(),
            r.template.dimmed(),
            date.dimmed(),
            if missing {
                format!("{} {}", r.name, "(missing)".red())
            } else {
                r.name.clone()
            },
            id_w = id_w,
            tmpl_w = tmpl_w,
        );
        println!("      {} {}", "→".dimmed(), r.path.dimmed());
    }
}

fn interactive_picker(filtered: &[&ProjectRecord], cfg: &Config) -> Result<()> {
    use dialoguer::Select;

    let id_w = filtered.iter().map(|r| r.id.len()).max().unwrap_or(4);
    let tmpl_w = filtered.iter().map(|r| r.template.len()).max().unwrap_or(8);

    loop {
        let labels: Vec<String> = filtered
            .iter()
            .map(|r| {
                let date = r.created_at.get(..10).unwrap_or(&r.created_at);
                let missing = !Path::new(&r.path).exists();
                let suffix = if missing { "  (missing)" } else { "" };
                format!(
                    "{:<id_w$}  {:<tmpl_w$}  {}  {}{}",
                    r.id,
                    r.template,
                    date,
                    r.name,
                    suffix,
                    id_w = id_w,
                    tmpl_w = tmpl_w,
                )
            })
            .chain(std::iter::once("[Quit]".to_string()))
            .collect();

        let idx = Select::new()
            .with_prompt(format!(
                "Recent projects ({} shown) — pick one",
                filtered.len()
            ))
            .items(&labels)
            .default(0)
            .interact()?;

        if idx == filtered.len() {
            return Ok(());
        }

        match project_action_menu(filtered[idx], cfg)? {
            ActionLoop::BackToList => continue,
            ActionLoop::Quit => return Ok(()),
        }
    }
}

enum ActionLoop {
    BackToList,
    Quit,
}

fn project_action_menu(record: &ProjectRecord, cfg: &Config) -> Result<ActionLoop> {
    use dialoguer::Select;

    println!();
    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        record.id.green().bold(),
        record.name.bold()
    );
    println!(
        "    {} {}  {}",
        "template:".dimmed(),
        record.template,
        record.path.dimmed()
    );

    loop {
        let items = [
            "Open project folder",
            "Show project metadata",
            "Back to list",
            "Quit",
        ];
        let choice = Select::new()
            .with_prompt("What would you like to do?")
            .items(&items)
            .default(0)
            .interact()?;

        match choice {
            0 => {
                let path = Path::new(&record.path);
                if !path.exists() {
                    eprintln!(
                        "{} project folder no longer exists at {} — run `fastf recent --prune`",
                        "warning:".yellow().bold(),
                        record.path
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
            1 => {
                let path = Path::new(&record.path);
                if !path.exists() {
                    eprintln!(
                        "{} project folder no longer exists at {}",
                        "warning:".yellow().bold(),
                        record.path
                    );
                    continue;
                }
                show_metadata(path, cfg);
            }
            2 => return Ok(ActionLoop::BackToList),
            3 => return Ok(ActionLoop::Quit),
            _ => unreachable!(),
        }
    }
}

fn prune(records: &[ProjectRecord]) -> Result<()> {
    let (keep, dropped): (Vec<_>, Vec<_>) = records
        .iter()
        .cloned()
        .partition(|r| Path::new(&r.path).exists());

    if dropped.is_empty() {
        println!(
            "{}",
            "Index is already clean — no missing projects.".dimmed()
        );
        return Ok(());
    }

    index::rewrite(&keep).context("rewriting index")?;
    println!(
        "{} Removed {} stale record{}.",
        "✓".green().bold(),
        dropped.len(),
        if dropped.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

/// Render a project's metadata to stdout.
///
/// Three cases, in order of preference:
///
/// 1. **Frontmatter present.** Pretty-print as aligned `key  value` pairs and
///    a separate `variables:` block — no markdown, no bolding. Looks like
///    `id3v2 -l` / `exiftool` output, easy to scan.
/// 2. **File exists, no frontmatter.** Hand-edited or pre-feature file: dump
///    the raw markdown so the user still sees what's there.
/// 3. **File missing.** Friendly yellow warning explaining why.
fn show_metadata(project_root: &Path, cfg: &Config) {
    use crate::core::project_info;

    println!();
    let banner = "─────  Project metadata  ─────";
    println!("{}", banner.dimmed());

    match project_info::read_metadata(project_root, cfg) {
        Ok(Some(meta)) => print_structured_metadata(&meta),
        Ok(None) => match project_info::read(project_root, cfg) {
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
fn print_structured_metadata(meta: &crate::core::project_info::Metadata) {
    // Top-level scalar fields, in a readable order (not alphabetical — id first).
    let scalars: [(&str, &str); 6] = [
        ("id", &meta.id),
        ("template", &meta.template),
        ("template_name", &meta.template_name),
        ("created", &meta.created),
        ("folder", &meta.folder),
        ("path", &meta.path),
    ];

    let scalar_w = scalars
        .iter()
        .map(|(k, _)| k.len())
        .chain(std::iter::once("variables".len()))
        .max()
        .unwrap_or(8);

    for (k, v) in scalars {
        println!("{:<w$}  {}", k.cyan(), v, w = scalar_w);
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

pub fn open(query: &str) -> Result<()> {
    let records = index::load_all()?;
    if records.is_empty() {
        bail!("project index is empty — create a project with `fastf new` first");
    }

    // Resolution order: exact id → id prefix → substring on name.
    let mut matches: Vec<&ProjectRecord> = records.iter().filter(|r| r.id == query).collect();
    if matches.is_empty() {
        matches = records.iter().filter(|r| r.id.starts_with(query)).collect();
    }
    if matches.is_empty() {
        let q = query.to_lowercase();
        matches = records
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&q))
            .collect();
    }

    match matches.len() {
        0 => bail!("no project matches '{}' — try `fastf recent`", query),
        1 => {
            let r = matches[0];
            let path = Path::new(&r.path);
            if !path.exists() {
                bail!(
                    "project '{}' no longer exists on disk at {} — run `fastf recent --prune` to clean up",
                    r.id,
                    r.path
                );
            }
            println!(
                "{} Opening {} ({})",
                "→".cyan().bold(),
                r.name.bold(),
                r.path.dimmed()
            );
            crate::core::post_create::reveal_folder(path)
        }
        _ => {
            eprintln!(
                "{} '{}' is ambiguous — pick a specific ID:",
                "error:".red().bold(),
                query
            );
            for r in matches {
                eprintln!("  {}  {}  ({})", r.id.green(), r.name, r.template.dimmed());
            }
            bail!("{} matches", records.len())
        }
    }
}
