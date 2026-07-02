use fastf::{bootstrap, cli, tui, ui};

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;

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
    #[command(after_help = "Examples:\n  \
            fastf new                                    # interactive: pick template, fill vars\n  \
            fastf new music-video                        # use named template, fill vars interactively\n  \
            fastf new music-video --dry-run              # preview without creating anything\n  \
            fastf new music-video --artist=\"Ariana Grande\" --title=Lullaby\n  \
            fastf new music-video --base-dir=/Volumes/Drive/Projects\n  \
            fastf new music-video --yes --artist=\"Bad Bunny\"   # flags + vars in any order\n\n\
            Variable flags must use = syntax: --artist=\"Bad Bunny\" not --artist \"Bad Bunny\".\n\
            Flags (--yes, --dry-run, --no-preview, --no-post, --base-dir=...) may appear\n\
            before OR after the template slug — fastf lifts them out automatically.")]
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

        /// Suppress file-content previews in the dry-run / confirm output
        #[arg(long)]
        no_preview: bool,

        /// Skip post-create actions (git init / reveal / editor / custom commands)
        #[arg(long)]
        no_post: bool,

        /// Skip the confirmation prompt (for scripts). Implies --no-preview is honored.
        #[arg(short = 'y', long)]
        yes: bool,

        /// Variable values as --slug=value flags (e.g. --artist="Ariana Grande" --title=Lullaby).
        /// Run 'fastf template show <slug>' to see a template's variables.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },

    /// Manage templates (list, create, edit, delete, from-folder)
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

    /// List recent projects — opens an interactive picker by default
    #[command(
        after_help = "By default, `fastf recent` opens an interactive picker so you can\n\
        select a project and choose between opening its folder, viewing metadata,\n\
        adding tags, or appending a journal note. Pass --plain (or pipe stdout)\n\
        to get the non-interactive list output for scripts.\n\n\
        Examples:\n  \
            fastf recent                           # interactive picker\n  \
            fastf recent --plain                   # non-interactive list (script-friendly)\n  \
            fastf recent --plain --limit 5\n  \
            fastf recent --template music-video\n  \
            fastf recent --since 2026-01-01\n  \
            fastf recent --tag draft               # only projects with this tag\n  \
            fastf recent --prune                   # remove index entries whose folder is gone"
    )]
    Recent {
        /// Max number of projects to show (default: from config recent_default_limit, or 20)
        #[arg(long)]
        limit: Option<usize>,

        /// Only show projects created from this template slug
        #[arg(long)]
        template: Option<String>,

        /// Only show projects created on or after this date (YYYY-MM-DD)
        #[arg(long)]
        since: Option<String>,

        /// Only show projects that have this exact tag
        #[arg(long)]
        tag: Option<String>,

        /// Delete records whose folder no longer exists on disk (does not touch folders)
        #[arg(long)]
        prune: bool,

        /// Print the plain list and exit instead of entering the interactive picker.
        /// Auto-engages when stdout is not a TTY (e.g. piping to grep or a file).
        #[arg(long)]
        plain: bool,
    },

    /// Open a previously created project folder in the system file manager
    #[command(
        after_help = "The query is matched against (in order): exact ID, ID prefix,\n\
        then case-insensitive substring of the project name.\n\n\
        Examples:\n  \
            fastf open ID0047\n  \
            fastf open 0047                        # ID prefix match\n  \
            fastf open lullaby                     # name substring match"
    )]
    Open {
        /// Project ID (e.g. ID0047), ID prefix, or name substring
        query: String,
    },

    /// Onboard an existing folder into fastf's index (no folder is created)
    #[command(
        about = "Onboard an existing folder into fastf's index (no folder is created)",
        long_about = "Adopt a pre-existing folder into fastf — write PROJECT_INFO.md, append a\n\
            record to projects.jsonl, and bump the global ID counter. Use this for\n\
            retroactively indexing projects that started before fastf, or projects\n\
            created outside it.\n\n\
            With --template: prompts for that template's variables, writes a full\n\
            metadata file (frontmatter + tags incl. tag_from auto-derivation), and\n\
            optionally fills missing template structure (--apply) or renames the\n\
            folder to the template's naming_pattern (--rename).\n\n\
            Without --template: writes a minimal metadata file and appends a record\n\
            with template = \"(registered)\". The folder is otherwise untouched.\n\n\
            The `created` timestamp defaults to the folder's filesystem creation\n\
            time (modification time on filesystems without birth-time, e.g. ext4).\n\
            Override with `--use-today` (now) or `--created YYYY-MM-DD` (explicit).",
        after_help = "Examples:\n  \
            fastf register ./old-project                         # minimal, no template\n  \
            fastf register ./old-project --template music-video --artist=X --title=Y\n  \
            fastf register ./old-project -t music-video --apply  # also fill template structure\n  \
            fastf register ./old-project -t music-video --rename # rename to naming_pattern\n  \
            fastf register ./old-project --use-today             # ignore folder mtime\n  \
            fastf register ./old-project --created 2024-06-15    # historical date\n\n\
            Batch import (with the --yes ordering fix you can pipe these):\n  \
            for d in ~/old-work/*/ ; do fastf register \"$d\" --yes ; done"
    )]
    Register {
        /// Path to an existing folder to onboard into fastf's index
        path: String,

        /// Template slug to attach (enables --apply and --rename). Omit for a minimal record.
        #[arg(short = 't', long)]
        template: Option<String>,

        /// After registering, run apply-style fill-in of missing template folders/files
        /// (requires --template). Existing files are never overwritten.
        #[arg(long, requires = "template")]
        apply: bool,

        /// Standardize the folder name by renaming on disk. With --template:
        /// renders the template's naming_pattern. Without --template: uses
        /// config.register_naming_pattern (default "{date}_{name}_{id}", where
        /// {name} is the sanitized current folder name). Confirms before
        /// moving unless --yes.
        #[arg(long)]
        rename: bool,

        /// Use today's date as the project's `created` timestamp
        /// (overrides the folder's filesystem time).
        #[arg(long, conflicts_with = "created")]
        use_today: bool,

        /// Explicit `created` date as YYYY-MM-DD (e.g. 2024-06-15).
        #[arg(long, value_name = "YYYY-MM-DD")]
        created: Option<String>,

        /// Skip confirmation prompts (PROJECT_INFO.md overwrite, rename).
        #[arg(short = 'y', long)]
        yes: bool,

        /// Variable values as --slug=value flags when --template is set.
        /// Same parsing contract as `fastf new`: vars use =; flags may appear in any order.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },

    /// Re-apply a template to an existing folder, adding missing folders/files
    #[command(after_help = "Existing files are never overwritten — only missing\n\
        folders and files are added.\n\n\
        Examples:\n  \
            fastf apply music-video ./old-project --dry-run\n  \
            fastf apply rust-project ./my-crate --artist=\"\" -y")]
    Apply {
        /// Template slug (see 'fastf template list')
        template: String,

        /// Target folder to augment
        target: String,

        /// Preview what would be added without writing anything
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,

        /// Variable values as --slug=value flags (only used when templated files need interpolation)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },

    /// Add, remove, list, or re-derive tags on a project
    #[command(after_help = "Tags live in the YAML frontmatter of PROJECT_INFO.md.\n\
        Free-form tags are arbitrary strings.  Auto-derived tags follow\n\
        the `slug/value` convention (set via `tag_from:` in a template).\n\n\
        Examples:\n  \
            fastf tag add ID0047 draft urgent       # add free-form tags\n  \
            fastf tag remove ID0047 draft\n  \
            fastf tag list ID0047\n  \
            fastf tag reauto ID0047                 # re-derive from template tag_from")]
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },

    /// Search projects by metadata fields and tags
    #[command(
        after_help = "Default mode: a bare term (no operator) does a case-insensitive\n\
        substring match across all variable values, tags, folder name,\n\
        template slug, template display name, and ID.  Multiple bare terms\n\
        AND together.  `path` is intentionally excluded.\n\n\
        Explicit operators (each clause ANDs with the rest):\n\
        \n  \
            key=value        exact match (case-insensitive)\n  \
            key=prefix*      prefix/glob match\n  \
            key>date         field is lexicographically after date\n  \
            key<date         field is lexicographically before date\n  \
            tag:value        exact tag match\n  \
            tag:prefix*      tag prefix/glob match\n\n\
        Field names: id  template  template_name  created  folder  name  path\n\
        plus any template variable slug (e.g. artist=Aria*)\n\n\
        Examples:\n  \
            fastf search ariana                       # default: substring across fields\n  \
            fastf search ariana lullaby               # both terms must appear somewhere\n  \
            fastf search tag:draft\n  \
            fastf search tag:client/*\n  \
            fastf search template=music-video tag:draft\n  \
            fastf search artist=Aria* created>2026-01-01\n  \
            fastf search ariana template=music-video  # mix free + explicit\n  \
            fastf search tag:draft --plain"
    )]
    Search {
        /// Query clauses (e.g. tag:draft template=music-video artist=Aria*)
        #[arg(required = true)]
        terms: Vec<String>,

        /// Print non-interactive list (auto-engages when stdout is not a TTY)
        #[arg(long)]
        plain: bool,
    },

    /// Append a timestamped journal note to a project
    #[command(
        name = "note",
        after_help = "Three ways to supply the message:\n  \
            fastf note add ID0047 \"finished final mix\"   # inline\n  \
            fastf note add ID0047 -                       # read from stdin\n  \
            fastf note add ID0047                         # open $EDITOR"
    )]
    Note {
        #[command(subcommand)]
        action: NoteAction,
    },

    /// Show journal entries for a project
    #[command(
        name = "notes",
        after_help = "Examples:\n  \
            fastf notes ID0047\n  \
            fastf notes ID0047 --since 2026-04-01"
    )]
    Notes {
        /// Project ID, ID prefix, or name substring
        query: String,

        /// Only show entries on or after this date (YYYY-MM-DD or ISO-8601)
        #[arg(long)]
        since: Option<String>,
    },

    /// Print a shell completion script to stdout
    #[command(
        after_help = "Pipe the output into your shell's completion directory.\n\n\
        Examples:\n  \
            fastf completions bash > /etc/bash_completion.d/fastf\n  \
            fastf completions zsh > ~/.zfunc/_fastf\n  \
            fastf completions fish > ~/.config/fish/completions/fastf.fish"
    )]
    Completions {
        /// Target shell: bash, zsh, fish, or powershell
        shell: String,
    },

    /// Launch the local browser UI for Fast Folder
    #[command(
        after_help = "Starts a small loopback-only HTTP server and opens the Fast Folder\n\
        UI in your browser. The UI shares the same templates, config, counter,\n\
        and project index as the CLI. Stop it with Ctrl-C.\n\n\
        Examples:\n  \
            fastf ui                              # serve + open the default browser\n  \
            fastf ui --app                        # open a dedicated app window (Chromium/Chrome)\n  \
            fastf ui --no-open                    # serve only, don't open a browser\n  \
            fastf ui --address 127.0.0.1:47840    # bind a different loopback port"
    )]
    Ui {
        /// Address to bind (loopback only — do not expose to a network).
        #[arg(long, default_value = ui::DEFAULT_ADDRESS)]
        address: String,

        /// Start the server but do not open a browser.
        #[arg(long)]
        no_open: bool,

        /// Open in a dedicated app window (Chromium/Chrome) instead of the default browser.
        #[arg(long)]
        app: bool,
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
    /// Generate a template from an existing folder tree (structure + file contents, opt-in assets)
    #[command(
        after_help = "Walks the folder, turning every directory into a FolderNode and every\n\
            text file ≤ 64 KB into a reproduced file. Binary and large files are\n\
            skipped by default; pass --bundle-assets to copy them byte-for-byte into\n\
            the template (it confirms the total size first). Common noise dirs\n\
            (.git, node_modules, target, __pycache__, .venv, dist, build, .idea, .vscode)\n\
            are skipped automatically.\n\n\
            Examples:\n  \
                fastf template from-folder ./my-crate rust-project\n  \
                fastf template from-folder ./delivery-kit client-kit --bundle-assets\n  \
                fastf template from-folder ./existing-video video-project --force"
    )]
    FromFolder {
        /// Source folder to scan
        path: String,
        /// Slug for the new template (letters, digits, '-', '_')
        slug: String,
        /// Overwrite existing template with the same slug
        #[arg(long)]
        force: bool,
        /// Bundle binary/large files byte-for-byte (default: text files only)
        #[arg(long)]
        bundle_assets: bool,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Display current configuration and file locations
    Show,
    /// Set a configuration value
    #[command(after_help = "Valid keys:\n  \
            base-dir                    Directory where new projects are created (default: current directory)\n  \
            editor                      Editor command for opening templates (default: $EDITOR)\n  \
            default-template            Slug of template to use without prompting (e.g. music-video)\n  \
            date-format                 strftime format for the {date} token (default: %Y-%m-%d)\n  \
            preview-lines               Lines per file in dry-run preview (default: 8, 0 = none)\n  \
            prompt-open-after-create    Ask 'Open project folder?' after `fastf new` (default: true)\n  \
            confirm-create              Ask 'Create this project?' in `fastf new` (default: true)\n  \
            show-banner                 Show ASCII banner in TUI menu (default: true)\n  \
            project-info-enabled        Write PROJECT_INFO.md (YAML frontmatter + variables table) into each new project (default: true)\n  \
            project-info-filename       Filename for project metadata (default: PROJECT_INFO.md)\n  \
            recent-default-limit        Default --limit for `fastf recent` (default: 20)\n  \
            register-naming-pattern     Pattern for `fastf register --rename` w/o a template (default: \"{date}_{name}_{id}\")\n  \
            post_create.git_init        Run `git init` automatically (default: false)\n  \
            post_create.reveal          Open folder in file manager automatically (default: false)\n  \
            post_create.open_in_editor  Open folder in $EDITOR automatically (default: false)\n  \
            post_create.print_path      Print absolute path on stdout (default: false)\n\n\
            Booleans accept: true/false, on/off, yes/no, 1/0\n\n\
            Path format for base-dir:\n  \
            Linux / macOS               /home/user/Projects  or  /Volumes/Drive/Projects\n  \
            Windows                     C:\\Users\\user\\Projects  or  C:/Users/user/Projects\n  \
            (Both slash styles work on Windows)\n\n\
            Examples:\n  \
            fastf config set base-dir /Volumes/Drive/Projects\n  \
            fastf config set default-template music-video\n  \
            fastf config set date-format %d-%m-%Y\n  \
            fastf config set prompt-open-after-create false\n  \
            fastf config set project-info-filename .fastf-info.md\n  \
            fastf config set post_create.reveal true")]
    Set {
        /// Config key (run `fastf config set --help` for the full list)
        key: String,
        /// New value to set
        value: String,
    },
}

#[derive(Subcommand)]
enum TagAction {
    /// Add one or more tags to a project
    Add {
        /// Project ID, ID prefix, or name substring
        query: String,
        /// Tags to add (space-separated)
        #[arg(required = true)]
        tags: Vec<String>,
    },
    /// Remove one or more tags from a project
    Remove {
        /// Project ID, ID prefix, or name substring
        query: String,
        /// Tags to remove (space-separated)
        #[arg(required = true)]
        tags: Vec<String>,
    },
    /// List tags on a project
    List {
        /// Project ID, ID prefix, or name substring
        query: String,
    },
    /// Re-derive auto tags from the template's tag_from variables
    Reauto {
        /// Project ID, ID prefix, or name substring
        query: String,
    },
}

#[derive(Subcommand)]
enum NoteAction {
    /// Add a timestamped journal entry
    Add {
        /// Project ID, ID prefix, or name substring
        query: String,
        /// Message text, or `-` to read from stdin, or omit to open $EDITOR
        message: Option<String>,
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

        Some(Commands::New {
            template,
            dry_run,
            base_dir,
            no_preview,
            no_post,
            yes,
            extra,
        }) => {
            let classified = cli::new::classify_extra(extra);
            warn_unknown(&classified.unknown);
            cli::new::run(cli::new::NewArgs {
                template_slug: template,
                vars: classified.vars,
                dry_run: dry_run || classified.flags.dry_run,
                base_dir_override: base_dir.or(classified.flags.base_dir),
                no_preview: no_preview || classified.flags.no_preview,
                no_post: no_post || classified.flags.no_post,
                yes: yes || classified.flags.yes,
            })
        }

        Some(Commands::Template { action }) => match action {
            TemplateAction::New => cli::template::new_interactive(),
            TemplateAction::List => cli::template::list(),
            TemplateAction::Show { slug } => cli::template::show(&slug),
            TemplateAction::Edit { slug } => cli::template::edit(&slug),
            TemplateAction::Delete { slug } => cli::template::delete(&slug),
            TemplateAction::FromFolder {
                path,
                slug,
                force,
                bundle_assets,
            } => cli::template::run_from_folder(&path, &slug, force, bundle_assets),
        },

        Some(Commands::Config { action }) => match action {
            ConfigAction::Show => cli::config::show(),
            ConfigAction::Set { key, value } => cli::config::set(&key, &value),
        },

        Some(Commands::Id { action }) => match action {
            IdAction::Show => cli::id::show(),
            IdAction::Reset => cli::id::reset(),
            IdAction::Set { value } => cli::id::set(value),
        },

        Some(Commands::Recent {
            limit,
            template,
            since,
            tag,
            prune,
            plain,
        }) => cli::recent::run(cli::recent::RecentArgs {
            limit,
            template,
            since,
            tag,
            prune,
            plain,
        }),

        Some(Commands::Open { query }) => cli::recent::open(&query),

        Some(Commands::Register {
            path,
            template,
            apply,
            rename,
            use_today,
            created,
            yes,
            extra,
        }) => {
            let classified = cli::new::classify_extra(extra);
            warn_unknown(&classified.unknown);
            cli::register::run(cli::register::RegisterArgs {
                path: std::path::PathBuf::from(path),
                template_slug: template,
                vars: classified.vars,
                apply_structure: apply,
                rename,
                use_today,
                created_override: created,
                yes: yes || classified.flags.yes,
            })
        }

        Some(Commands::Apply {
            template,
            target,
            dry_run,
            yes,
            extra,
        }) => {
            let classified = cli::new::classify_extra(extra);
            warn_unknown(&classified.unknown);
            cli::apply::run(cli::apply::ApplyArgs {
                template_slug: template,
                target,
                dry_run: dry_run || classified.flags.dry_run,
                yes: yes || classified.flags.yes,
                vars: classified.vars,
            })
        }

        Some(Commands::Tag { action }) => match action {
            TagAction::Add { query, tags } => cli::tag::add(&query, &tags),
            TagAction::Remove { query, tags } => cli::tag::remove(&query, &tags),
            TagAction::List { query } => cli::tag::list(&query),
            TagAction::Reauto { query } => cli::tag::reauto(&query),
        },

        Some(Commands::Search { terms, plain }) => {
            cli::search::run(cli::search::SearchArgs { terms, plain })
        }

        Some(Commands::Note { action }) => match action {
            NoteAction::Add { query, message } => {
                cli::note::add(cli::note::NoteAddArgs { query, message })
            }
        },

        Some(Commands::Notes { query, since }) => {
            cli::note::notes(cli::note::NotesArgs { query, since })
        }

        Some(Commands::Completions { shell }) => generate_completions(&shell),

        Some(Commands::Ui {
            address,
            no_open,
            app,
        }) => cli::ui::run(cli::ui::UiArgs {
            address,
            no_open,
            app,
        }),
    }
}

/// Emit a `warning:` line for every token that came out of `classify_extra`'s
/// unknown bucket. Used by the New / Apply / Register arms.
fn warn_unknown(unknown: &[String]) {
    for u in unknown {
        eprintln!(
            "{} unrecognized flag '{}' — ignored",
            "warning:".yellow().bold(),
            u
        );
    }
}

fn generate_completions(shell: &str) -> Result<()> {
    use clap::CommandFactory;
    use clap_complete::{generate, shells};
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    match shell.to_lowercase().as_str() {
        "bash" => {
            generate(shells::Bash, &mut cmd, &name, &mut std::io::stdout());
            Ok(())
        }
        "zsh" => {
            generate(shells::Zsh, &mut cmd, &name, &mut std::io::stdout());
            Ok(())
        }
        "fish" => {
            generate(shells::Fish, &mut cmd, &name, &mut std::io::stdout());
            Ok(())
        }
        "powershell" | "ps" => {
            generate(shells::PowerShell, &mut cmd, &name, &mut std::io::stdout());
            Ok(())
        }
        other => anyhow::bail!(
            "unknown shell '{}'. Valid: bash, zsh, fish, powershell",
            other
        ),
    }
}
