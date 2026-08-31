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
    if let Ok((_, mode)) = paths::try_install_dir() {
        println!("  {:<26} {}", "Data dir mode:".dimmed(), mode.label());
    }
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
    // The data-dir counter is a backup input, not the record — each base carries
    // its own `.fastf-counter.toml`. `fastf id show` lists them.
    println!(
        "  {:<26} {}",
        "Counter (this machine):".dimmed(),
        paths::counters_path().display()
    );
    println!(
        "  {:<26} {}",
        "Counter (per base):".dimmed(),
        "<base>/.fastf-counter.toml  — see `fastf id show`".dimmed()
    );
    println!();
    println!(
        "  {:<26} {}",
        "base_dir:".green(),
        if config.base_dir.is_empty() {
            format!("{} (home directory — not configured)", base.display())
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
        "terminal:".green(),
        match config.resolve_terminal() {
            crate::core::config::TerminalPreference::Disabled =>
                "none (never opens a window)".to_string(),
            crate::core::config::TerminalPreference::Named(name) =>
                if config.terminal.trim().is_empty() {
                    format!("{name} (from $TERMINAL)")
                } else {
                    name
                },
            crate::core::config::TerminalPreference::Probe =>
                "(probe: konsole, gnome-terminal, kitty, …)".to_string(),
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
        "show_frame:".green(),
        bool_label(config.show_frame)
    );
    println!(
        "  {:<26} {}",
        "recent_default_limit:".green(),
        config.recent_default_limit
    );
    println!(
        "  {:<26} {}",
        "bases:".green(),
        if config.bases.is_empty() {
            "(none)".dimmed().to_string()
        } else {
            config.bases.join(", ")
        }
    );
    println!(
        "  {:<26} {}",
        "register_naming_pattern:".green(),
        config.register_naming_pattern
    );
    println!(
        "  {:<26} {}  {}",
        "on_name_collision:".green(),
        config.on_name_collision,
        if config.suffix_on_name_collision() {
            "(a taken folder name gets _2, _3, …)".dimmed()
        } else {
            "(a taken folder name is refused)".dimmed()
        }
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

/// Validate one extra base directory and return the string to store.
///
/// Each entry gets the same `~`-expansion and absolute-path check as `base_dir`
/// (`config::expand_base_path`), but an entry that does not *exist* is only a
/// note: an unmounted drive is a legitimate base, and discovery already treats
/// an absent one as honestly empty. `expand_base_path` deliberately creates
/// nothing — conjuring a missing base would plant an empty directory over a
/// mount point and shadow the drive it stands for.
///
/// Shared with the TUI's Library bases menu so there is one validator rather
/// than two that can drift.
pub fn normalize_base_entry(raw: &str) -> Result<String> {
    let expanded = crate::core::config::expand_base_path(raw)?;
    if !expanded.is_dir() {
        eprintln!(
            "{} {} is not mounted right now — keeping it; it will be indexed when it appears",
            "note:".yellow(),
            crate::util::paths::display_path(&expanded)
        );
    }
    crate::util::paths::storable(&expanded, "the base directory")
}

pub fn set(key: &str, value: &str) -> Result<()> {
    // Load-mutate-save is a read-modify-write, so it needs the same
    // cross-process lock as ID allocation. Without it, two concurrent
    // `config set` calls each write back their own copy of the whole file and
    // one update is silently lost. A release-mode test caught this; the debug
    // build happened to be slow enough to serialize the processes by luck.
    let normalized = key.replace('-', "_");
    crate::core::operations::update_config(|config| {
        match normalized.as_str() {
            "base_dir" => {
                // Same validation as first-run onboarding — see
                // `config::resolve_base_dir_input`. Storing the raw string let a
                // quoted `~/Projects` become a literal directory named `~`, and a
                // relative path scatter projects wherever the command ran.
                let resolved = crate::core::config::resolve_base_dir_input(value)?;
                config.base_dir = crate::util::paths::storable(&resolved, "the base directory")?;
                println!(
                    "Set base_dir = {}",
                    crate::util::paths::display_path(&resolved)
                );
            }
            "editor" => {
                config.editor = value.to_string();
                println!("Set editor = {}", value);
            }
            "terminal" => {
                config.terminal = value.to_string();
                println!("Set terminal = {}", value);
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
            "show_frame" => {
                config.show_frame = parse_bool(value)?;
                println!("Set show_frame = {}", config.show_frame);
            }
            "bases" => {
                // Comma-separated list of extra base directories to index. Empty
                // value clears the list.
                let mut resolved = Vec::new();
                for raw in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    resolved.push(normalize_base_entry(raw)?);
                }
                config.bases = resolved;
                if config.bases.is_empty() {
                    println!("Cleared bases");
                } else {
                    println!("Set bases = {}", config.bases.join(", "));
                }
            }
            "recent_default_limit" => {
                let n = parse_usize(value)?;
                if n == 0 {
                    bail!("recent_default_limit must be at least 1");
                }
                config.recent_default_limit = n;
                println!("Set recent_default_limit = {}", config.recent_default_limit);
            }
            "register_naming_pattern" => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    bail!("register_naming_pattern cannot be empty");
                }
                // `{name}` and `{id}` are the safety net — without them the pattern
                // would silently rename multiple registered folders to the same path.
                if !trimmed.contains("{id}") {
                    bail!(
                        "register_naming_pattern must contain {{id}} so registered folders get unique names; got '{}'",
                        trimmed
                    );
                }
                config.register_naming_pattern = trimmed.to_string();
                println!(
                    "Set register_naming_pattern = {}",
                    config.register_naming_pattern
                );
            }
            "on_name_collision" => {
                config.on_name_collision = match value.trim().to_lowercase().as_str() {
                    "suffix" => crate::core::config::NameCollision::Suffix,
                    "error" => crate::core::config::NameCollision::Error,
                    // The *setter* is still strict: a typo typed at the command
                    // line is a mistake to report, where a typo already sitting
                    // in a config file must not stop every command.
                    other => bail!("expected 'suffix' or 'error'; got '{other}'"),
                };
                println!(
                    "Set on_name_collision = {}  ({})",
                    config.on_name_collision,
                    if config.suffix_on_name_collision() {
                        "a taken folder name gets _2, _3, …"
                    } else {
                        "a taken folder name is refused"
                    }
                );
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
                "unknown config key '{}'. Valid keys: base-dir, bases, editor, terminal, default-template, date-format, \
             preview-lines, prompt-open-after-create, confirm-create, show-banner, show-frame, \
             recent-default-limit, register-naming-pattern, on-name-collision, \
             post_create.git_init, post_create.reveal, post_create.open_in_editor, post_create.print_path",
                other
            ),
        }
        Ok(())
    })?;
    Ok(())
}
