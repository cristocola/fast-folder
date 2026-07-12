use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use std::collections::HashMap;

use crate::cli::new::{self, NewArgs};
use crate::cli::register::{self, RegisterArgs};
use crate::cli::{apply, config, id, recent, search, template};
use crate::core::config::Config;
use crate::core::template as core_template;

const BANNER: &str = r#"  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|"#;
const BANNER_WIDTH: usize = 40;

pub fn run() -> Result<()> {
    // Banner is shown once based on the first config load. Honors show_banner.
    let initial = Config::load().unwrap_or_default();
    if initial.show_banner {
        println!();
        println!("{}", BANNER.cyan().bold());
        let tagline = format!("project scaffolder · v{}", env!("CARGO_PKG_VERSION"));
        let pad = BANNER_WIDTH.saturating_sub(tagline.chars().count());
        println!("{}{}", " ".repeat(pad), tagline.dimmed());
        println!();
    }

    loop {
        // Reload config each iteration so changes in settings are reflected immediately
        let cfg = Config::load().unwrap_or_default();
        let base = cfg.resolve_base_dir();

        let parent = base
            .parent()
            .map(|p| format!("{}{}", p.display(), std::path::MAIN_SEPARATOR))
            .unwrap_or_default();
        let name = base
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| base.to_string_lossy().into_owned());

        println!(
            "  {}  {}{}",
            "project base  →".dimmed(),
            parent.dimmed(),
            name.cyan().bold()
        );
        println!();

        let choice = Select::new()
            .with_prompt("What would you like to do?")
            .items(&[
                "Create new project",
                "Recent projects",
                "Search projects",
                "Register existing folder",
                "Manage templates",
                "View / edit settings",
                "Quit",
            ])
            .default(0)
            .interact()?;

        match choice {
            0 => menu_create()?,
            1 => menu_recent()?,
            2 => menu_search()?,
            3 => menu_register()?,
            4 => menu_templates()?,
            5 => menu_settings()?,
            6 => {
                println!("Goodbye.");
                break;
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

fn menu_create() -> Result<()> {
    let tmpl = new::pick_template_interactively()?;
    let args = NewArgs {
        template_slug: Some(tmpl.slug.clone()),
        vars: HashMap::new(),
        dry_run: false,
        base_dir_override: pick_base_interactively()?,
        no_preview: false,
        no_post: false,
        yes: false,
    };
    new::run(args)?;
    println!();
    Ok(())
}

/// When more than one base is configured, ask which one the new project should
/// be created in. Returns `None` (= config default) when there's only one base
/// or the first (default) entry is chosen.
fn pick_base_interactively() -> Result<Option<String>> {
    let cfg = Config::load().unwrap_or_default();
    let bases: Vec<std::path::PathBuf> = cfg
        .effective_bases()
        .into_iter()
        .filter(|b| b.is_dir())
        .collect();
    if bases.len() <= 1 {
        return Ok(None);
    }
    let labels: Vec<String> = bases
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let default = if i == 0 { "  (default)" } else { "" };
            format!(
                "{}  ({}){}",
                crate::core::library::base_label(b),
                b.display(),
                default
            )
        })
        .collect();
    let idx = Select::new()
        .with_prompt("Create the project in which base?")
        .items(&labels)
        .default(0)
        .interact()?;
    if idx == 0 {
        // The default base — let config.base_dir resolution do its thing.
        return Ok(None);
    }
    Ok(Some(bases[idx].display().to_string()))
}

fn menu_recent() -> Result<()> {
    // Interactive picker is the default for `fastf recent`, so just delegate.
    recent::run(recent::RecentArgs {
        limit: None,
        template: None,
        since: None,
        tag: None,
        plain: false,
    })?;
    println!();
    Ok(())
}

fn menu_search() -> Result<()> {
    let query: String = Input::new()
        .with_prompt("Search query (e.g. tag:draft  template=music-video  artist=Aria*)")
        .interact_text()?;
    let query = query.trim().to_string();
    if query.is_empty() {
        println!("{}", "  (cancelled)".dimmed());
        return Ok(());
    }
    let terms: Vec<String> = query.split_whitespace().map(|s| s.to_string()).collect();
    search::run(search::SearchArgs {
        terms,
        plain: false,
    })?;
    println!();
    Ok(())
}

fn menu_apply() -> Result<()> {
    let slug = prompt_template_slug("Template to apply")?;
    let target: String = Input::new().with_prompt("Target folder").interact_text()?;
    let dry_run = Confirm::new()
        .with_prompt("Dry run first (preview only)?")
        .default(true)
        .interact()?;

    apply::run(apply::ApplyArgs {
        template_slug: slug.clone(),
        target: target.clone(),
        dry_run,
        yes: false,
        vars: HashMap::new(),
    })?;

    if dry_run {
        let proceed = Confirm::new()
            .with_prompt("Apply for real now?")
            .default(false)
            .interact()?;
        if proceed {
            apply::run(apply::ApplyArgs {
                template_slug: slug,
                target,
                dry_run: false,
                yes: false,
                vars: HashMap::new(),
            })?;
        }
    }
    println!();
    Ok(())
}

fn menu_register() -> Result<()> {
    // 1. Folder path.
    let path: String = Input::new()
        .with_prompt("Existing folder to register")
        .interact_text()?;
    let path = path.trim();
    if path.is_empty() {
        println!("{}", "  (cancelled)".dimmed());
        return Ok(());
    }

    // 2. Optional template — first ask, then pick if Yes. Skipping is fully
    //    supported: register writes a minimal record with template "(registered)".
    let use_template = Confirm::new()
        .with_prompt("Attach a template (enables tags + variable capture)?")
        .default(false)
        .interact()?;

    let template_slug = if use_template {
        match core_template::load_all() {
            Ok(ts) if !ts.is_empty() => Some(prompt_template_slug("Template to attach")?),
            Ok(_) => {
                println!(
                    "  {} no templates available — continuing without one.",
                    "·".dimmed()
                );
                None
            }
            Err(e) => {
                println!(
                    "  {} could not load templates ({e}) — continuing without one.",
                    "warning:".yellow().bold()
                );
                None
            }
        }
    } else {
        None
    };

    // 3. Standardize folder name. Default Yes; the actual fs::rename inside
    //    register::run() prompts again before moving, so the user has a second
    //    chance to back out once they see the proposed new name.
    let rename = Confirm::new()
        .with_prompt("Standardize folder name (rename to match pattern)?")
        .default(true)
        .interact()?;

    // 4. Optional --apply (only meaningful with a template).
    let apply_structure = if template_slug.is_some() {
        Confirm::new()
            .with_prompt("Fill in missing template folders/files?")
            .default(false)
            .interact()?
    } else {
        false
    };

    register::run(RegisterArgs {
        path: std::path::PathBuf::from(path),
        template_slug,
        vars: HashMap::new(),
        apply_structure,
        rename,
        use_today: false,
        created_override: None,
        yes: false,
    })?;
    println!();
    Ok(())
}

fn menu_templates() -> Result<()> {
    loop {
        let choice = Select::new()
            .with_prompt("Templates")
            .items(&[
                "Create new template",
                "Generate template from existing folder",
                "Edit a template",
                "Apply template to existing folder",
                "List templates",
                "Show template details",
                "Delete a template",
                "Back",
            ])
            .default(0)
            .interact()?;

        match choice {
            0 => {
                template::new_interactive()?;
                println!();
            }
            1 => {
                let path: String = Input::new()
                    .with_prompt("Source folder to scan")
                    .interact_text()?;
                let slug: String = Input::new()
                    .with_prompt("Slug for the new template")
                    .interact_text()?;
                let force = Confirm::new()
                    .with_prompt("Overwrite if a template with this slug exists?")
                    .default(false)
                    .interact()?;
                template::run_from_folder(&path, &slug, force, false)?;
                println!();
            }
            2 => {
                let slug = prompt_template_slug("Edit template")?;
                template::edit(&slug)?;
                println!();
            }
            3 => {
                menu_apply()?;
            }
            4 => {
                template::list()?;
                println!();
            }
            5 => {
                let slug = prompt_template_slug("Show template")?;
                template::show(&slug)?;
                println!();
            }
            6 => {
                let slug = prompt_template_slug("Delete template")?;
                template::delete(&slug)?;
                println!();
            }
            7 => break,
            _ => unreachable!(),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ID counter
// ---------------------------------------------------------------------------

fn menu_id() -> Result<()> {
    loop {
        id::show()?;
        println!();
        let choice = Select::new()
            .with_prompt("ID counter")
            .items(&["Set counter value", "Reset to 0", "Back"])
            .default(2)
            .interact()?;

        match choice {
            0 => {
                let val: String = Input::new()
                    .with_prompt("Set counter to (next project will be this + 1)")
                    .interact_text()?;
                match val.trim().parse::<u64>() {
                    Ok(n) => {
                        id::set(n)?;
                    }
                    Err(_) => {
                        eprintln!("{} expected a number, got '{}'", "error:".red().bold(), val);
                    }
                }
            }
            1 => {
                id::reset()?;
            }
            2 => break,
            _ => unreachable!(),
        }
        println!();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings — grouped submenus
// ---------------------------------------------------------------------------

fn menu_settings() -> Result<()> {
    loop {
        config::show()?;
        println!();
        let choice = Select::new()
            .with_prompt("Settings")
            .items(&[
                "Project basics  (base dir / template / date / editor)",
                "Workflow prompts  (open prompt / confirm / banner / preview)",
                "Library bases  (extra folders to index)",
                "Recent projects  (default limit)",
                "Post-create actions  (git / reveal / editor / path / commands)",
                "ID counter",
                "Back",
            ])
            .default(0)
            .interact()?;

        match choice {
            0 => menu_settings_basics()?,
            1 => menu_settings_workflow()?,
            2 => menu_settings_bases()?,
            3 => menu_settings_recent()?,
            4 => menu_settings_postcreate()?,
            5 => menu_id()?,
            6 => break,
            _ => unreachable!(),
        }
        println!();
    }
    Ok(())
}

fn menu_settings_basics() -> Result<()> {
    loop {
        let choice = Select::new()
            .with_prompt("Project basics")
            .items(&[
                "Set base directory",
                "Set default template",
                "Set date format",
                "Set editor",
                "Back",
            ])
            .default(0)
            .interact()?;

        match choice {
            0 => {
                println!(
                    "  {}  Linux/macOS: /home/user/Projects  ·  Windows: C:\\Users\\user\\Projects or C:/Users/user/Projects",
                    "Hint:".yellow()
                );
                let val: String = Input::new()
                    .with_prompt("Base directory (empty = current dir)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("base-dir", &val)?;
            }
            1 => {
                let val: String = Input::new()
                    .with_prompt("Default template slug (empty = always prompt)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("default-template", &val)?;
            }
            2 => {
                let val: String = Input::new()
                    .with_prompt("Date format (strftime, e.g. %Y-%m-%d)")
                    .default("%Y-%m-%d".to_string())
                    .interact_text()?;
                config::set("date-format", &val)?;
            }
            3 => {
                let val: String = Input::new()
                    .with_prompt("Editor command (e.g. nvim, code, nano)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("editor", &val)?;
            }
            4 => break,
            _ => unreachable!(),
        }
        println!();
    }
    Ok(())
}

fn menu_settings_workflow() -> Result<()> {
    loop {
        let cfg = Config::load().unwrap_or_default();
        let items = [
            label_toggle(
                "\"Open project folder?\" prompt after create",
                cfg.prompt_open_after_create,
            ),
            label_toggle("\"Create this project?\" confirmation", cfg.confirm_create),
            label_toggle("ASCII banner in main menu", cfg.show_banner),
            format!("Dry-run preview lines  [{}]", cfg.preview_lines),
            "Back".to_string(),
        ];
        let choice = Select::new()
            .with_prompt("Workflow prompts")
            .items(&items)
            .default(0)
            .interact()?;

        match choice {
            0 => toggle_setting("prompt-open-after-create", cfg.prompt_open_after_create)?,
            1 => toggle_setting("confirm-create", cfg.confirm_create)?,
            2 => toggle_setting("show-banner", cfg.show_banner)?,
            3 => {
                let val: String = Input::new()
                    .with_prompt("Lines per file in dry-run (0 = none)")
                    .default(cfg.preview_lines.to_string())
                    .interact_text()?;
                config::set("preview-lines", &val)?;
            }
            4 => break,
            _ => unreachable!(),
        }
        println!();
    }
    Ok(())
}

/// Edit the list of extra base directories the project library indexes (beyond
/// `base_dir`). An absent/unmounted base is simply skipped at scan time.
fn menu_settings_bases() -> Result<()> {
    loop {
        let cfg = Config::load().unwrap_or_default();
        println!();
        if cfg.bases.is_empty() {
            println!(
                "  {}",
                "No extra bases. base_dir is always indexed on its own.".dimmed()
            );
        } else {
            println!("  {}", "Extra indexed bases (besides base_dir):".bold());
            for b in &cfg.bases {
                println!("    {} {}", "•".cyan(), b);
            }
        }
        println!();

        let mut items = vec!["Add a base directory".to_string()];
        for b in &cfg.bases {
            items.push(format!("Remove  {b}"));
        }
        items.push("Back".to_string());

        let choice = Select::new()
            .with_prompt("Library bases")
            .items(&items)
            .default(0)
            .interact()?;

        if choice == 0 {
            let val: String = Input::new()
                .with_prompt("Base directory to add (absolute path)")
                .allow_empty(true)
                .interact_text()?;
            let val = val.trim();
            if !val.is_empty() {
                let mut bases = cfg.bases.clone();
                if !bases.iter().any(|b| b == val) {
                    bases.push(val.to_string());
                }
                config::set("bases", &bases.join(","))?;
            }
        } else if choice == items.len() - 1 {
            break;
        } else {
            let mut bases = cfg.bases.clone();
            let idx = choice - 1;
            if idx < bases.len() {
                bases.remove(idx);
            }
            config::set("bases", &bases.join(","))?;
        }
        println!();
    }
    Ok(())
}

fn menu_settings_recent() -> Result<()> {
    loop {
        let cfg = Config::load().unwrap_or_default();
        let items = [
            format!("Default list limit  [{}]", cfg.recent_default_limit),
            "Back".to_string(),
        ];
        let choice = Select::new()
            .with_prompt("Recent projects")
            .items(&items)
            .default(0)
            .interact()?;

        match choice {
            0 => {
                let val: String = Input::new()
                    .with_prompt("Default --limit for `fastf recent`")
                    .default(cfg.recent_default_limit.to_string())
                    .interact_text()?;
                config::set("recent-default-limit", &val)?;
            }
            1 => break,
            _ => unreachable!(),
        }
        println!();
    }
    Ok(())
}

fn menu_settings_postcreate() -> Result<()> {
    loop {
        let cfg = Config::load().unwrap_or_default();
        let pc = &cfg.post_create;
        let items = [
            label_toggle("Run `git init`", pc.git_init),
            label_toggle("Reveal folder in file manager", pc.reveal),
            label_toggle("Open in configured editor", pc.open_in_editor),
            label_toggle("Print absolute path on stdout", pc.print_path),
            format!(
                "Edit extra shell commands  [{} configured]",
                pc.commands.len()
            ),
            "Back".to_string(),
        ];
        let choice = Select::new()
            .with_prompt("Post-create actions (default for new projects)")
            .items(&items)
            .default(0)
            .interact()?;

        match choice {
            0 => toggle_setting("post_create.git_init", pc.git_init)?,
            1 => toggle_setting("post_create.reveal", pc.reveal)?,
            2 => toggle_setting("post_create.open_in_editor", pc.open_in_editor)?,
            3 => toggle_setting("post_create.print_path", pc.print_path)?,
            4 => edit_postcreate_commands()?,
            5 => break,
            _ => unreachable!(),
        }
        println!();
    }
    Ok(())
}

fn edit_postcreate_commands() -> Result<()> {
    let mut cfg = Config::load()?;
    let cmds = &mut cfg.post_create.commands;

    if cmds.is_empty() {
        println!("  {} no commands configured yet.", "·".dimmed());
    } else {
        println!("  {} current commands:", "·".dimmed());
        for (i, c) in cmds.iter().enumerate() {
            println!("    {}. {}", i + 1, c.as_str().dimmed());
        }
    }

    let choice = Select::new()
        .with_prompt("Manage commands")
        .items(&["Add a command", "Remove commands", "Done"])
        .default(0)
        .interact()?;

    match choice {
        0 => {
            println!(
                "  {}  Use {{path}} as a placeholder for the absolute project path.",
                "Hint:".yellow()
            );
            let cmd: String = Input::new()
                .with_prompt("Shell command")
                .allow_empty(true)
                .interact_text()?;
            if !cmd.trim().is_empty() {
                cmds.push(cmd);
                cfg.save()?;
                println!("  {} command added.", "✓".green());
            }
        }
        1 => {
            if cmds.is_empty() {
                println!("  {} nothing to remove.", "·".dimmed());
                return Ok(());
            }
            let labels: Vec<&str> = cmds.iter().map(String::as_str).collect();
            let picks = MultiSelect::new()
                .with_prompt("Select commands to remove (Space to toggle, Enter to confirm)")
                .items(&labels)
                .interact()?;
            // Remove in reverse so indices stay valid
            for i in picks.into_iter().rev() {
                cmds.remove(i);
            }
            cfg.save()?;
            println!("  {} updated.", "✓".green());
        }
        2 => {}
        _ => unreachable!(),
    }
    Ok(())
}

fn label_toggle(label: &str, on: bool) -> String {
    let state = if on { "on" } else { "off" };
    format!("{}  [{}]", label, state)
}

fn toggle_setting(key: &str, current: bool) -> Result<()> {
    let new_val = !current;
    config::set(key, if new_val { "true" } else { "false" })?;
    Ok(())
}

fn prompt_template_slug(prompt: &str) -> Result<String> {
    use crate::core::template;
    let templates = template::load_all()?;
    if templates.is_empty() {
        anyhow::bail!("no templates found");
    }
    let labels: Vec<String> = templates
        .iter()
        .map(|t| format!("{} ({})", t.name, t.slug))
        .collect();
    let idx = Select::new()
        .with_prompt(prompt)
        .items(&labels)
        .default(0)
        .interact()?;
    Ok(templates[idx].slug.clone())
}
