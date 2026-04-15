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
    long_about = "fastf creates structured project folders from YAML templates.\n\
\n\
Templates define a folder structure, placeholder files, and variables (text inputs\n\
or select menus). Each project gets an auto-incrementing ID. Templates, config, and\n\
counters live next to the binary — fully portable, no home directory required.\n\
\n\
Getting started:\n\
  fastf                        # interactive menu\n\
  fastf new                    # pick a template and fill in variables\n\
  fastf template list          # see available templates\n\
  fastf template new           # create a new template interactively\n\
  fastf config show            # view current settings",
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
    #[command(
        after_help = "Examples:\n  \
            fastf new                                    # interactive: pick template, fill vars\n  \
            fastf new music-video                        # use named template, fill vars interactively\n  \
            fastf new music-video --dry-run              # preview without creating anything\n  \
            fastf new music-video --artist=\"Ariana Grande\" --title=Lullaby\n  \
            fastf new music-video --base-dir=/Volumes/Drive/Projects\n\n\
            Variable flags must use = syntax: --artist=\"Bad Bunny\" not --artist \"Bad Bunny\""
    )]
    New {
        /// Template slug to use. Run 'fastf template list' to see available templates.
        /// Prompts interactively if omitted and no default-template is configured.
        template: Option<String>,

        /// Show what would be created without writing anything to disk
        #[arg(long)]
        dry_run: bool,

        /// Override the base directory for this project only (ignores config base-dir)
        #[arg(long)]
        base_dir: Option<String>,

        /// Variable values as --slug=value flags (e.g. --artist="Ariana Grande" --title=Lullaby).
        /// Run 'fastf template show <slug>' to see a template's variables.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },

    /// Manage templates (list, create, edit, delete, import, export)
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },

    /// View and edit fastf configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Manage the global auto-increment project ID counter
    Id {
        #[command(subcommand)]
        action: IdAction,
    },

    /// Print a shell completion script to stdout
    #[command(after_help = "Pipe the output into your shell's completion directory.\n\n\
        Examples:\n  \
            fastf completions bash > /etc/bash_completion.d/fastf\n  \
            fastf completions zsh > ~/.zfunc/_fastf\n  \
            fastf completions fish > ~/.config/fish/completions/fastf.fish")]
    Completions {
        /// Target shell: bash, zsh, fish, or powershell
        shell: String,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// Create a new template step-by-step with an interactive builder
    New,
    /// List all available templates with their slugs and descriptions
    List,
    /// Show full details of a template: variables, folder structure, and placeholder files
    Show {
        /// Template slug (see 'fastf template list')
        slug: String,
    },
    /// Edit an existing template interactively — existing values are pre-filled, press Enter to keep them
    Edit {
        /// Template slug (see 'fastf template list')
        slug: String,
    },
    /// Permanently delete a template (asks for confirmation)
    Delete {
        /// Template slug (see 'fastf template list')
        slug: String,
    },
    /// Import a template from a YAML file into the templates directory
    Import {
        /// Path to the YAML template file to import
        file: String,
    },
    /// Export a template as YAML — to stdout or to a file for sharing or backup
    Export {
        /// Template slug (see 'fastf template list')
        slug: String,
        /// Write output to this file instead of stdout
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Display current configuration and file locations
    Show,
    /// Set a configuration value
    #[command(
        after_help = "Valid keys:\n  \
            base-dir           Directory where new projects are created (default: current directory)\n  \
            editor             Editor command for opening templates (default: $EDITOR)\n  \
            default-template   Slug of template to use without prompting (e.g. music-video)\n  \
            date-format        strftime format for the {date} token (default: %Y-%m-%d)\n\n\
            Path format for base-dir:\n  \
            Linux / macOS      /home/user/Projects  or  /Volumes/Drive/Projects\n  \
            Windows            C:\\Users\\user\\Projects  or  C:/Users/user/Projects\n  \
            (Both slash styles work on Windows)\n\n\
            Examples:\n  \
            fastf config set base-dir /Volumes/Drive/Projects\n  \
            fastf config set base-dir \"C:/Users/Cristo/Projects\"\n  \
            fastf config set default-template music-video\n  \
            fastf config set date-format %d-%m-%Y"
    )]
    Set {
        /// Config key: base-dir, editor, default-template, or date-format
        key: String,
        /// New value to set
        value: String,
    },
}

#[derive(Subcommand)]
enum IdAction {
    /// Show the current global ID counter value and what the next project ID will be
    Show,
    /// Reset the global counter back to 0 (next project will be ID0001)
    Reset,
    /// Set the counter to a specific value (next project will be that value + 1)
    Set {
        /// Counter value to set (e.g. 46 means next project gets ID0047)
        value: u64,
    },
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
