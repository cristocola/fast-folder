use anyhow::Result;
use colored::Colorize;
use dialoguer::Select;
use std::collections::HashMap;

use crate::cli::new::{self, NewArgs};
use crate::cli::{config, id, template};
use crate::core::config::Config;

const BANNER: &str = r#"
  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|
                       by Cristo Cola
"#;

pub fn run() -> Result<()> {
    println!("{}", BANNER.cyan().bold());

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
                "Manage templates",
                "View / edit settings",
                "View ID counters",
                "Quit",
            ])
            .default(0)
            .interact()?;

        match choice {
            0 => menu_create()?,
            1 => menu_templates()?,
            2 => menu_settings()?,
            3 => {
                id::show()?;
                println!();
                // Offer reset option
                let reset = dialoguer::Confirm::new()
                    .with_prompt("Reset global ID counter?")
                    .default(false)
                    .interact()?;
                if reset {
                    id::reset()?;
                }
                println!();
            }
            4 => {
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
    };
    new::run(args)?;
    println!();
    Ok(())
}

fn menu_templates() -> Result<()> {
    loop {
        let choice = Select::new()
            .with_prompt("Templates")
            .items(&[
                "Create new template",
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
                let slug = prompt_template_slug("Edit template")?;
                template::edit(&slug)?;
                println!();
            }
            2 => {
                template::list()?;
                println!();
            }
            3 => {
                let slug = prompt_template_slug("Show template")?;
                template::show(&slug)?;
                println!();
            }
            4 => {
                let slug = prompt_template_slug("Delete template")?;
                template::delete(&slug)?;
                println!();
            }
            5 => {
                let path: String = dialoguer::Input::new()
                    .with_prompt("Path to .yaml file")
                    .interact_text()?;
                template::import(&path)?;
                println!();
            }
            6 => break,
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn menu_settings() -> Result<()> {
    loop {
        config::show()?;
        println!();
        let choice = Select::new()
            .with_prompt("Settings")
            .items(&[
                "Set base directory",
                "Set default template",
                "Set date format",
                "Set editor",
                "Back",
            ])
            .default(4)
            .interact()?;

        match choice {
            0 => {
                println!(
                    "  {}  Linux/macOS: /home/user/Projects  ·  Windows: C:\\Users\\user\\Projects or C:/Users/user/Projects",
                    "Hint:".yellow()
                );
                let val: String = dialoguer::Input::new()
                    .with_prompt("Base directory (empty = current dir)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("base-dir", &val)?;
            }
            1 => {
                let val: String = dialoguer::Input::new()
                    .with_prompt("Default template slug (empty = always prompt)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("default-template", &val)?;
            }
            2 => {
                let val: String = dialoguer::Input::new()
                    .with_prompt("Date format (strftime, e.g. %Y-%m-%d)")
                    .default("%Y-%m-%d".to_string())
                    .interact_text()?;
                config::set("date-format", &val)?;
            }
            3 => {
                let val: String = dialoguer::Input::new()
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
