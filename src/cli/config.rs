use anyhow::{bail, Result};
use chrono::Local;
use colored::Colorize;

use crate::core::config::Config;
use crate::util::paths;

pub fn show() -> Result<()> {
    let config = Config::load()?;
    let base = config.resolve_base_dir();
    let editor = config.resolve_editor();

    println!("{}", "fastf config:".bold());
    println!("  {:<18} {}", "Config file:".dimmed(), paths::config_path().display());
    println!("  {:<18} {}", "Templates dir:".dimmed(), paths::templates_dir().display());
    println!("  {:<18} {}", "Counters file:".dimmed(), paths::counters_path().display());
    println!();
    println!("  {:<18} {}", "base_dir:".green(), if config.base_dir.is_empty() {
        format!("{} (current directory)", base.display())
    } else {
        base.display().to_string()
    });
    println!("  {:<18} {}", "editor:".green(), if config.editor.is_empty() {
        format!("{} (from $EDITOR)", editor)
    } else {
        editor
    });
    println!("  {:<18} {}", "default_template:".green(), if config.default_template.is_empty() {
        "(always prompt)".to_string()
    } else {
        config.default_template.clone()
    });
    println!("  {:<18} {}", "date_format:".green(), config.date_format);

    Ok(())
}

pub fn set(key: &str, value: &str) -> Result<()> {
    let mut config = Config::load()?;
    match key {
        "base-dir" | "base_dir" => {
            config.base_dir = value.to_string();
            println!("Set base_dir = {}", value);
        }
        "editor" => {
            config.editor = value.to_string();
            println!("Set editor = {}", value);
        }
        "default-template" | "default_template" => {
            config.default_template = value.to_string();
            println!("Set default_template = {}", value);
        }
        "date-format" | "date_format" => {
            let preview = Local::now().format(value).to_string();
            if preview.is_empty() {
                bail!("invalid date format '{}' — must be a valid strftime string (e.g. %Y-%m-%d)", value);
            }
            config.date_format = value.to_string();
            println!("Set date_format = {}  (today: {})", value, preview);
        }
        other => bail!(
            "unknown config key '{}'. Valid keys: base-dir, editor, default-template, date-format",
            other
        ),
    }
    config.save()?;
    Ok(())
}
