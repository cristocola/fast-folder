use anyhow::{Result, bail};
use chrono::Local;
use colored::Colorize;

use crate::core::config::Config;
use crate::util::paths;

pub fn show() -> Result<()> {
    let config = Config::load()?;
    let base = config.resolve_base_dir();
    let editor = config.resolve_editor();

    println!("{}", "fastf config:".bold());
    println!(
        "  {:<26} {}",
        "Config file:".dimmed(),
        paths::config_path().display()
    );
    println!(
        "  {:<26} {}",
        "Templates dir:".dimmed(),
        paths::templates_dir().display()
    );
    println!(
        "  {:<26} {}",
        "Counters file:".dimmed(),
        paths::counters_path().display()
    );
    println!();
    println!(
        "  {:<26} {}",
        "base_dir:".green(),
        if config.base_dir.is_empty() {
            format!("{} (current directory)", base.display())
        } else {
            base.display().to_string()
        }
    );
    println!(
        "  {:<26} {}",
        "editor:".green(),
        if config.editor.is_empty() {
            format!("{} (from $EDITOR)", editor)
        } else {
            editor
        }
    );
    println!(
        "  {:<26} {}",
        "default_template:".green(),
        if config.default_template.is_empty() {
            "(always prompt)".to_string()
        } else {
            config.default_template.clone()
        }
    );
    println!("  {:<26} {}", "date_format:".green(), config.date_format);
    println!(
        "  {:<26} {}",
        "preview_lines:".green(),
        config.preview_lines
    );
    println!();
    println!(
        "  {:<26} {}",
        "prompt_open_after_create:".green(),
        bool_label(config.prompt_open_after_create)
    );
    println!(
        "  {:<26} {}",
        "confirm_create:".green(),
        bool_label(config.confirm_create)
    );
    println!(
        "  {:<26} {}",
        "show_banner:".green(),
        bool_label(config.show_banner)
    );
    println!(
        "  {:<26} {}",
        "project_info_enabled:".green(),
        bool_label(config.project_info_enabled)
    );
    println!(
        "  {:<26} {}",
        "project_info_filename:".green(),
        config.project_info_filename
    );
    println!(
        "  {:<26} {}",
        "recent_default_limit:".green(),
        config.recent_default_limit
    );
    println!();
    println!("  {}", "post_create defaults:".bold());
    println!(
        "    {:<24} {}",
        "git_init".dimmed(),
        bool_label(config.post_create.git_init)
    );
    println!(
        "    {:<24} {}",
        "reveal".dimmed(),
        bool_label(config.post_create.reveal)
    );
    println!(
        "    {:<24} {}",
        "open_in_editor".dimmed(),
        bool_label(config.post_create.open_in_editor)
    );
    println!(
        "    {:<24} {}",
        "print_path".dimmed(),
        bool_label(config.post_create.print_path)
    );
    let cmd_count = config.post_create.commands.len();
    println!(
        "    {:<24} {} command{}",
        "commands".dimmed(),
        cmd_count,
        if cmd_count == 1 { "" } else { "s" }
    );

    Ok(())
}

fn bool_label(b: bool) -> colored::ColoredString {
    if b { "on".green() } else { "off".dimmed() }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" | "on" | "yes" | "y" | "1" => Ok(true),
        "false" | "off" | "no" | "n" | "0" => Ok(false),
        other => bail!(
            "expected a boolean (true/false, on/off, yes/no, 1/0); got '{}'",
            other
        ),
    }
}

fn parse_usize(value: &str) -> Result<usize> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("expected a non-negative integer; got '{}'", value))
}

pub fn set(key: &str, value: &str) -> Result<()> {
    let mut config = Config::load()?;
    let normalized = key.replace('-', "_");
    match normalized.as_str() {
        "base_dir" => {
            config.base_dir = value.to_string();
            println!("Set base_dir = {}", value);
        }
        "editor" => {
            config.editor = value.to_string();
            println!("Set editor = {}", value);
        }
        "default_template" => {
            config.default_template = value.to_string();
            println!("Set default_template = {}", value);
        }
        "date_format" => {
            let preview = Local::now().format(value).to_string();
            if preview.is_empty() {
                bail!(
                    "invalid date format '{}' — must be a valid strftime string (e.g. %Y-%m-%d)",
                    value
                );
            }
            config.date_format = value.to_string();
            println!("Set date_format = {}  (today: {})", value, preview);
        }
        "preview_lines" => {
            config.preview_lines = parse_usize(value)?;
            println!("Set preview_lines = {}", config.preview_lines);
        }
        "prompt_open_after_create" => {
            config.prompt_open_after_create = parse_bool(value)?;
            println!(
                "Set prompt_open_after_create = {}",
                config.prompt_open_after_create
            );
        }
        "confirm_create" => {
            config.confirm_create = parse_bool(value)?;
            println!("Set confirm_create = {}", config.confirm_create);
        }
        "show_banner" => {
            config.show_banner = parse_bool(value)?;
            println!("Set show_banner = {}", config.show_banner);
        }
        // `pinfo_*` are kept as aliases (parse-only) for the v0.2-interim
        // config files that used the old name. They write into the renamed
        // fields and serialize back under the new names on save.
        "project_info_enabled" | "pinfo_enabled" => {
            config.project_info_enabled = parse_bool(value)?;
            println!("Set project_info_enabled = {}", config.project_info_enabled);
        }
        "project_info_filename" | "pinfo_filename" => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("project_info_filename cannot be empty");
            }
            if trimmed.contains('/') || trimmed.contains('\\') {
                bail!("project_info_filename must be a bare filename, not a path");
            }
            config.project_info_filename = trimmed.to_string();
            println!(
                "Set project_info_filename = {}",
                config.project_info_filename
            );
        }
        "recent_default_limit" => {
            let n = parse_usize(value)?;
            if n == 0 {
                bail!("recent_default_limit must be at least 1");
            }
            config.recent_default_limit = n;
            println!("Set recent_default_limit = {}", config.recent_default_limit);
        }
        "post_create.git_init" => {
            config.post_create.git_init = parse_bool(value)?;
            println!("Set post_create.git_init = {}", config.post_create.git_init);
        }
        "post_create.reveal" => {
            config.post_create.reveal = parse_bool(value)?;
            println!("Set post_create.reveal = {}", config.post_create.reveal);
        }
        "post_create.open_in_editor" => {
            config.post_create.open_in_editor = parse_bool(value)?;
            println!(
                "Set post_create.open_in_editor = {}",
                config.post_create.open_in_editor
            );
        }
        "post_create.print_path" => {
            config.post_create.print_path = parse_bool(value)?;
            println!(
                "Set post_create.print_path = {}",
                config.post_create.print_path
            );
        }
        other => bail!(
            "unknown config key '{}'. Valid keys: base-dir, editor, default-template, date-format, \
             preview-lines, prompt-open-after-create, confirm-create, show-banner, \
             project-info-enabled, project-info-filename, recent-default-limit, \
             post_create.git_init, post_create.reveal, post_create.open_in_editor, post_create.print_path",
            other
        ),
    }
    config.save()?;
    Ok(())
}
