use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use std::collections::HashMap;

use crate::cli::new::{self, NewArgs};
use crate::cli::{apply, config, id, recent, template};
use crate::core::config::Config;

const BANNER: &str = r#"
  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|
                       by Cristo Cola
"#;

pub fn run() -> Result<()> {
    // Banner is shown once based on the first config load. Honors show_banner.
    let initial = Config::load().unwrap_or_default();
    if initial.show_banner {
        println!("{}", BANNER.cyan().bold());
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
                "Apply template to existing folder",
                "Manage templates",
                "View / edit settings",
                "View ID counters",
                "Quit",
            ])
            .default(0)
            .interact()?;

        match choice {
            0 => menu_create()?,
            1 => menu_recent()?,
            2 => menu_apply()?,
            3 => menu_templates()?,
            4 => menu_settings()?,
            5 => {
                id::show()?;
                println!();
                let reset = Confirm::new()
                    .with_prompt("Reset global ID counter?")
                    .default(false)
                    .interact()?;
                if reset {
                    id::reset()?;
                }
                println!();
            }
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
        base_dir_override: None,
        no_preview: false,
        no_post: false,
        yes: false,
    };
    new::run(args)?;
    println!();
    Ok(())
}

fn menu_recent() -> Result<()> {
    // Interactive picker is now the default for `fastf recent`, so just delegate.
    recent::run(recent::RecentArgs {
        limit: None,
        template: None,
        since: None,
        prune: false,
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

fn menu_templates() -> Result<()> {
    loop {
        let choice = Select::new()
            .with_prompt("Templates")
            .items(&[
                "Create new template",
                "Generate template from existing folder",
                "Edit a template",
                "List templates",
                "Show template details",
                "Delete a template",
                "Import template from file",
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
                template::from_folder(&path, &slug, force)?;
                println!();
            }
            2 => {
                let slug = prompt_template_slug("Edit template")?;
                template::edit(&slug)?;
                println!();
            }
            3 => {
                template::list()?;
                println!();
            }
            4 => {
                let slug = prompt_template_slug("Show template")?;
                template::show(&slug)?;
                println!();
            }
            5 => {
                let slug = prompt_template_slug("Delete template")?;
                template::delete(&slug)?;
                println!();
            }
            6 => {
                let path: String = Input::new()
                    .with_prompt("Path to .yaml file")
                    .interact_text()?;
                template::import(&path)?;
                println!();
            }
            7 => break,
            _ => unreachable!(),
        }
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
                "Project metadata  (PROJECT_INFO.md enabled / filename)",
                "Recent projects  (default limit)",
                "Post-create actions  (git / reveal / editor / path / commands)",
                "Back",
            ])
            .default(0)
            .interact()?;

        match choice {
            0 => menu_settings_basics()?,
            1 => menu_settings_workflow()?,
            2 => menu_settings_project_info()?,
            3 => menu_settings_recent()?,
            4 => menu_settings_postcreate()?,
            5 => break,
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

fn menu_settings_project_info() -> Result<()> {
    loop {
        let cfg = Config::load().unwrap_or_default();
        let items = [
            label_toggle(
                "Generate PROJECT_INFO.md on new project",
                cfg.project_info_enabled,
            ),
            format!("Metadata filename  [{}]", cfg.project_info_filename),
            "Back".to_string(),
        ];
        let choice = Select::new()
            .with_prompt("Project metadata")
            .items(&items)
            .default(0)
            .interact()?;

        match choice {
            0 => toggle_setting("project-info-enabled", cfg.project_info_enabled)?,
            1 => {
                let val: String = Input::new()
                    .with_prompt("Filename (e.g. PROJECT_INFO.md, .fastf-info.md)")
                    .default(cfg.project_info_filename.clone())
                    .interact_text()?;
                config::set("project-info-filename", &val)?;
            }
            2 => break,
            _ => unreachable!(),
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
