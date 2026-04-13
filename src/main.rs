mod bootstrap;
mod cli;
mod core;
mod tui;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "fastf",
    about = "Fast Folder Creator — template-driven project folder generator",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new project from a template
    New {
        /// Template slug (e.g. music-video). Prompts if omitted.
        template: Option<String>,

        /// Preview what would be created without writing anything
        #[arg(long)]
        dry_run: bool,

        /// Override base directory for this project only
        #[arg(long)]
        base_dir: Option<String>,

        /// Variable values: --var-slug=value (e.g. --artist="Ariana Grande")
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },

    /// Manage templates
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },

    /// View and edit settings
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Manage auto-increment ID counters
    Id {
        #[command(subcommand)]
        action: IdAction,
    },

    /// Generate shell completion scripts
    Completions {
        /// Shell: bash, zsh, fish, powershell
        shell: String,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// Create a new template interactively
    New,
    /// List all available templates
    List,
    /// Show details of a template
    Show { slug: String },
    /// Edit a template interactively
    Edit { slug: String },
    /// Delete a template
    Delete { slug: String },
    /// Import a template from a YAML file
    Import { file: String },
    /// Export a template to stdout or a file
    Export {
        slug: String,
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Display current configuration
    Show,
    /// Set a configuration value (keys: base-dir, editor, default-template, date-format)
    Set { key: String, value: String },
}

#[derive(Subcommand)]
enum IdAction {
    /// Show the global ID counter
    Show,
    /// Reset the global counter to 0
    Reset,
    /// Set the global counter to a specific value
    Set { value: u64 },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    if let Err(e) = run() {
        eprintln!("{} {:#}", colored::Colorize::red("error:"), e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    // Bootstrap on every run (idempotent — no-op after first run)
    bootstrap::ensure_bootstrapped()?;

    let cli = Cli::parse();

    match cli.command {
        // No subcommand → interactive TUI
        None => tui::menu::run(),

        Some(Commands::New { template, dry_run, base_dir, extra }) => {
            let vars = parse_extra_vars(&extra);
            cli::new::run(cli::new::NewArgs {
                template_slug: template,
                vars,
                dry_run,
                base_dir_override: base_dir,
            })
        }

        Some(Commands::Template { action }) => match action {
            TemplateAction::New              => cli::template::new_interactive(),
            TemplateAction::List             => cli::template::list(),
            TemplateAction::Show { slug }    => cli::template::show(&slug),
            TemplateAction::Edit { slug }    => cli::template::edit(&slug),
            TemplateAction::Delete { slug }  => cli::template::delete(&slug),
            TemplateAction::Import { file }  => cli::template::import(&file),
            TemplateAction::Export { slug, output } => {
                cli::template::export(&slug, output.as_deref())
            }
        },

        Some(Commands::Config { action }) => match action {
            ConfigAction::Show               => cli::config::show(),
            ConfigAction::Set { key, value } => cli::config::set(&key, &value),
        },

        Some(Commands::Id { action }) => match action {
            IdAction::Show         => cli::id::show(),
            IdAction::Reset        => cli::id::reset(),
            IdAction::Set { value} => cli::id::set(value),
        },

        Some(Commands::Completions { shell }) => generate_completions(&shell),
    }
}

/// Parse trailing args like --artist="Ariana Grande" --title=Lullaby
/// into HashMap { "artist" => "Ariana Grande", "title" => "Lullaby" }.
/// Hyphens in keys are normalized to underscores.
fn parse_extra_vars(extra: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for arg in extra {
        let s = arg.trim_start_matches('-');
        if let Some((key, val)) = s.split_once('=') {
            let key = key.replace('-', "_");
            map.insert(key, val.to_string());
        }
    }
    map
}

fn generate_completions(shell: &str) -> Result<()> {
    use clap::CommandFactory;
    use clap_complete::{generate, shells};
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    match shell.to_lowercase().as_str() {
        "bash"             => { generate(shells::Bash,       &mut cmd, &name, &mut std::io::stdout()); Ok(()) }
        "zsh"              => { generate(shells::Zsh,        &mut cmd, &name, &mut std::io::stdout()); Ok(()) }
        "fish"             => { generate(shells::Fish,       &mut cmd, &name, &mut std::io::stdout()); Ok(()) }
        "powershell" | "ps"=> { generate(shells::PowerShell, &mut cmd, &name, &mut std::io::stdout()); Ok(()) }
        other => anyhow::bail!("unknown shell '{}'. Valid: bash, zsh, fish, powershell", other),
    }
}
