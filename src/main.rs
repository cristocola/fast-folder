use fastf::{bootstrap, cli, tui};

use anyhow::Result;
use clap::{Parser, Subcommand};

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
counters live in one data folder: next to the binary when a config.toml sits there\n\
(portable mode), otherwise in your user config directory. See `fastf paths`.\n\
\n\
Getting started:\n\
  fastf                        # interactive menu\n\
  fastf new                    # pick a template and fill in variables\n\
  fastf template list          # see available templates\n\
  fastf template new           # create a new template interactively\n\
  fastf paths                  # where fastf keeps its data\n\
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
            fastf new music-video --base-dir=/mnt/projects/clients   # create in another base\n  \
            fastf new music-video --yes --artist=\"Bad Bunny\"   # flags + vars in any order\n\n\
            Variables must use = syntax: --artist=\"Bad Bunny\", not --artist \"Bad Bunny\".\n\
            Every flag above works before OR after the template slug, and a --word that is\n\
            neither a declared flag nor a --key=value pair is refused, not ignored.")]
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
            fastf recent --tag draft               # only projects with this tag"
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

    /// Move a project folder into another configured base
    #[command(
        name = "move",
        about = "Move a project folder into another configured base directory",
        long_about = "Move a project's folder from its current base into another configured base\n\
            (base_dir or one of the `bases` list), keeping the folder name. Targets are\n\
            restricted to configured bases so the moved project stays discoverable.\n\n\
            Only the operating system's cross-device error enables the copy fallback.\n\
            It uses a private target-base transaction, copies every regular file and\n\
            empty directory, verifies exact paths/types/byte lengths plus unchanged\n\
            source metadata, publishes atomically, and only then removes the source.\n\
            Keep the project untouched while this copy is running.",
        after_help = "Examples:\n  \
            fastf move ID0047 /mnt/projects/archive   # by full base path\n  \
            fastf move ID0047 01_PROJECTS             # by base folder name\n  \
            fastf move lullaby                        # interactive base picker (TTY)"
    )]
    Move {
        /// Project ID (e.g. ID0047), ID prefix, or name substring
        query: String,

        /// Target base — full path or its folder name. Omit to pick interactively.
        base: Option<String>,

        /// Skip the confirmation prompt (for scripts).
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Rebuild the project-library cache by rescanning every base
    #[command(
        about = "Force a full rescan of every base and rewrite its .fastf-index.json cache",
        long_about = "Projects are discovered from each folder's PROJECT_INFO.md and accelerated\n\
            by a per-base .fastf-index.json cache that self-heals automatically. Run\n\
            `fastf reindex` only after EXTERNAL changes fastf can't observe — folders\n\
            moved or metadata hand-edited on another machine — to refresh the caches."
    )]
    Reindex,

    /// Recover scoped v2 operations and report obsolete pre-v2 markers
    #[command(
        about = "Recover scoped v2 operations and report obsolete pre-v2 markers",
        long_about = "Scoped v2 journals let `fastf reconcile` safely resume deferred creates,\n\
            discard unpublished move staging, or finish source cleanup after a\n\
            verified publication. Identity, configured-base, path, and manifest\n\
            checks must all pass before it mutates anything.\n\n\
            Pre-v2 markers contain arbitrary absolute paths, so reconcile never\n\
            parses them, copies through them, or deletes any path they name. It\n\
            lists each obsolete marker for manual inspection and leaves it plus\n\
            all related paths untouched. Reconciliation is explicit and idempotent."
    )]
    Reconcile,

    /// Onboard an existing folder by writing its PROJECT_INFO.md (no folder is created)
    #[command(
        about = "Onboard an existing folder by writing its PROJECT_INFO.md (no folder is created)",
        long_about = "Adopt a pre-existing folder into fastf by writing a PROJECT_INFO.md, which\n\
            makes it discoverable (filesystem-as-truth). Use this for projects that\n\
            started before fastf, or were created outside it.\n\n\
            The ID is recovered from an `ID####` token in the folder name when present\n\
            (so a folder named `..._ID0030` keeps ID 30); otherwise a fresh ID is\n\
            minted from the self-healing counter.\n\n\
            With --template: prompts for that template's variables, writes a full\n\
            metadata file (frontmatter + tags incl. tag_from auto-derivation), and\n\
            optionally fills missing template structure (--apply) or renames the\n\
            folder to the template's naming_pattern (--rename).\n\n\
            Without --template: writes a minimal metadata file with\n\
            template = \"(registered)\". The folder is otherwise untouched.\n\n\
            With --recursive: writes a PROJECT_INFO.md into every direct child of\n\
            <path> that lacks one (use --dry-run to preview). The `created` timestamp\n\
            defaults to the folder's filesystem time; override with --use-today or\n\
            --created YYYY-MM-DD.",
        after_help = "Examples:\n  \
            fastf register ./old-project                         # minimal, no template\n  \
            fastf register ./old-project --template music-video --artist=X --title=Y\n  \
            fastf register ./old-project -t music-video --apply  # also fill template structure\n  \
            fastf register ./old-project -t music-video --rename # rename to naming_pattern\n  \
            fastf register ./old-project --use-today             # ignore folder mtime\n  \
            fastf register ~/Projects --recursive --dry-run      # preview a bulk import\n  \
            fastf register ~/Projects --recursive                # onboard every child"
    )]
    Register {
        /// Path to an existing folder to onboard (writes a PROJECT_INFO.md so it
        /// becomes discoverable). With --recursive, a base whose direct children
        /// are onboarded.
        path: String,

        /// Register every direct child of <path> that lacks a PROJECT_INFO.md.
        #[arg(long)]
        recursive: bool,

        /// Preview which folders would be registered, writing nothing.
        /// Only meaningful with --recursive (a single folder has nothing to preview).
        #[arg(long, requires = "recursive")]
        dry_run: bool,

        /// Template slug to attach (enables --apply and --rename). Omit for a minimal record.
        #[arg(short = 't', long)]
        template: Option<String>,

        /// After registering, run apply-style fill-in of missing template folders/files
        /// (requires --template). Existing files are never overwritten.
        #[arg(long, requires = "template", conflicts_with = "recursive")]
        apply: bool,

        /// Standardize the folder name by renaming on disk. With --template:
        /// renders the template's naming_pattern. Without --template: uses
        /// config.register_naming_pattern (default "{date}_{name}_{id}", where
        /// {name} is the sanitized current folder name). Confirms before
        /// moving unless --yes.
        #[arg(long, conflicts_with = "recursive")]
        rename: bool,

        /// Use today's date as the project's `created` timestamp
        /// (overrides the folder's filesystem time).
        #[arg(long, conflicts_with = "created")]
        use_today: bool,

        /// Explicit `created` date as YYYY-MM-DD (e.g. 2024-06-15).
        #[arg(long, value_name = "YYYY-MM-DD", conflicts_with = "recursive")]
        created: Option<String>,

        /// Skip confirmation prompts (PROJECT_INFO.md overwrite, rename).
        #[arg(short = 'y', long, conflicts_with = "recursive")]
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
        after_help = "Write the output into your shell's completion directory.\n\n\
        Examples:\n  \
            fastf completions bash > ~/.local/share/bash-completion/completions/fastf\n  \
            fastf completions zsh > ~/.zfunc/_fastf          # ~/.zfunc must be on $fpath\n  \
            fastf completions fish > ~/.config/fish/completions/fastf.fish"
    )]
    Completions {
        /// Target shell: bash, zsh, fish, or powershell
        shell: String,
    },

    /// Show where fastf keeps its data (config, templates, counters) and why
    #[command(after_help = "fastf resolves its data directory in this order:\n  \
            1. FASTF_INSTALL_DIR environment variable (if set)\n  \
            2. Portable mode: the binary's own directory, if it already contains\n     \
               a config.toml or templates/ folder\n  \
            3. Your user config directory (~/.config/fastf on Linux,\n     \
               %APPDATA%\\fastf on Windows)")]
    Paths,

    /// Generate man pages into a directory (used by packaging)
    #[command(hide = true)]
    Mangen {
        /// Output directory for the generated .1 files
        dir: std::path::PathBuf,
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
        /// Skip the confirmation prompt (for scripts)
        #[arg(short = 'y', long)]
        yes: bool,
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
                fastf template from-folder ./delivery-kit client-kit --dry-run\n  \
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
        /// Accept the bundle-size confirmation without asking (for scripts)
        #[arg(short = 'y', long)]
        yes: bool,
        /// Print what would be generated and write nothing
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Display current configuration and file locations
    Show,
    /// Set a configuration value
    #[command(after_help = "Valid keys:\n  \
            base-dir                    Directory where new projects are created (default: home directory)\n  \
            bases                       Extra project folders to index, comma-separated (empty value clears the list)\n  \
            editor                      Editor command for opening templates (default: $EDITOR)\n  \
            default-template            Slug of template to use without prompting (e.g. music-video)\n  \
            date-format                 strftime format for the {date} token (default: %Y-%m-%d)\n  \
            preview-lines               Lines per file in dry-run preview (default: 8, 0 = none)\n  \
            prompt-open-after-create    Ask 'Open project folder?' after `fastf new` (default: true)\n  \
            confirm-create              Ask 'Create this project?' in `fastf new` (default: true)\n  \
            show-banner                 Show ASCII banner in TUI menu (default: true)\n  \
            recent-default-limit        TUI Projects page size and default `fastf recent --limit` (default: 20)\n  \
            register-naming-pattern     Pattern for `fastf register --rename` w/o a template (default: \"{date}_{name}_{id}\")\n  \
            on-name-collision           What to do when the folder name is taken: suffix (add _2, _3…) or error (default: suffix)\n  \
            post_create.git_init        Run `git init` automatically (default: false)\n  \
            post_create.reveal          Open folder in file manager automatically (default: false)\n  \
            post_create.open_in_editor  Open folder in $EDITOR automatically (default: false)\n  \
            post_create.print_path      Print absolute path on stdout (default: false)\n\n\
            Booleans accept: true/false, on/off, yes/no, 1/0\n\n\
            Path format for base-dir and bases:\n  \
            Linux / macOS               /home/user/Projects  or  /Volumes/Drive/Projects\n  \
            Windows                     C:\\Users\\user\\Projects  or  C:/Users/user/Projects\n  \
            (Both slash styles work on Windows)\n\n\
            Examples:\n  \
            fastf config set base-dir /Volumes/Drive/Projects\n  \
            fastf config set bases \"/mnt/projects/clients,/srv/archive\"\n  \
            fastf config set default-template music-video\n  \
            fastf config set date-format %d-%m-%Y\n  \
            fastf config set prompt-open-after-create false\n  \
            fastf config set on-name-collision error\n  \
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
    /// Make every base agree on the highest ID seen anywhere
    #[command(
        long_about = "The counter is the highest ID seen in any base's counter file, this\n\
            machine's data directory, or the projects themselves — and every base\n\
            converges on that one number, so both operating systems of a dual-boot\n\
            machine hand out the same next ID.\n\n\
            This happens automatically on every create and every `fastf id show`.\n\
            Run `sync` explicitly after an external change: a base mounted for the\n\
            first time, or projects copied in from another machine."
    )]
    Sync,
    /// Raise the counter (it can never be lowered — see `fastf id sync`)
    #[command(
        after_help = "The counter only moves up: it is the highest ID seen anywhere, so a\n\
        lower value would hand out an ID that already exists. Values at or below\n\
        the current floor are refused with an explanation.\n\n\
        Examples:\n  \
            fastf id set 100        # next project becomes ID0101"
    )]
    Set {
        /// Counter value to set (e.g. 46 means next project gets ID0047)
        value: u64,
    },
    /// Removed — the counter cannot be reset. Use `fastf id sync`.
    #[command(hide = true)]
    Reset,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    // Die quietly when stdout closes early (`fastf recent --plain | head`)
    // instead of panicking — restore the default SIGPIPE disposition that the
    // Rust runtime masks on startup.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // Turn Ctrl-C into a flag that long-running work polls, so an interrupted
    // create unwinds and rolls its partial folder back rather than being killed
    // part-way through copying a template's assets.
    fastf::util::interrupt::install();

    if let Err(e) = run() {
        // A prompt that failed or was unwound past left the cursor hidden.
        fastf::util::interrupt::restore_terminal();

        // An interrupt is the user's choice, not a failure. Say so, and exit
        // 130 (the shell convention for SIGINT) so scripts can tell them apart.
        //
        // Deliberately says nothing about a partial project: this fires wherever
        // Ctrl-C lands, including the main menu with nothing in flight. The
        // create path prints its own notice when it actually rolls a folder back.
        if fastf::util::interrupt::is_set() {
            eprintln!("{}", colored::Colorize::yellow("aborted."));
            std::process::exit(130);
        }
        let rendered = format!("{e:#}");
        eprintln!("{} {}", colored::Colorize::red("error:"), rendered);
        if is_config_parse_failure(&rendered) {
            // The only command that could repair the file also refuses to run
            // until it is repaired, so say what actually gets the user moving.
            eprintln!("  hint: fix the file, or delete it to start over with defaults");
        }
        std::process::exit(1);
    }
}

/// Does this error chain say that the configuration file itself is unreadable?
///
/// `Config::load` wraps the TOML failure with `parsing <path>`, so both halves
/// are present in the rendered chain. Keyed on the pair rather than on either
/// alone: fastf parses templates and metadata too, and offering to delete one
/// of those would be terrible advice.
fn is_config_parse_failure(rendered: &str) -> bool {
    rendered.contains("parsing") && rendered.contains("config.toml")
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // Fail early with a clean error if no data directory can be resolved
    // (e.g. $HOME unset) — everything downstream assumes it exists.
    fastf::util::paths::try_install_dir()?;

    // Bootstrap on every run (idempotent — no-op after first run). Skipped for
    // completions/mangen so packaging steps never write to the user's home.
    if !matches!(
        cli.command,
        Some(Commands::Completions { .. }) | Some(Commands::Mangen { .. })
    ) {
        bootstrap::ensure_bootstrapped()?;
    }

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
            let classified = classify_for("new", extra)?;
            let mut args = cli::new::NewArgs {
                template_slug: template,
                vars: classified.vars,
                dry_run,
                base_dir_override: base_dir,
                no_preview,
                no_post,
                yes,
            };
            cli::new::apply_extra(&mut args, classified.recognized)?;
            cli::new::run(args)
        }

        Some(Commands::Template { action }) => match action {
            TemplateAction::New => cli::template::new_interactive(),
            TemplateAction::List => cli::template::list(),
            TemplateAction::Show { slug } => cli::template::show(&slug),
            TemplateAction::Edit { slug } => cli::template::edit(&slug),
            TemplateAction::Delete { slug, yes } => cli::template::delete(&slug, yes),
            TemplateAction::FromFolder {
                path,
                slug,
                force,
                bundle_assets,
                yes,
                dry_run,
            } => cli::template::run_from_folder(cli::template::FromFolderArgs {
                path,
                slug,
                force,
                bundle_assets,
                yes,
                dry_run,
            }),
        },

        Some(Commands::Config { action }) => match action {
            ConfigAction::Show => cli::config::show(),
            ConfigAction::Set { key, value } => cli::config::set(&key, &value),
        },

        Some(Commands::Id { action }) => match action {
            IdAction::Show => cli::id::show(),
            IdAction::Sync => cli::id::sync(),
            IdAction::Set { value } => cli::id::set(value),
            IdAction::Reset => cli::id::reset(),
        },

        Some(Commands::Recent {
            limit,
            template,
            since,
            tag,
            plain,
        }) => cli::recent::run(cli::recent::RecentArgs {
            limit,
            template,
            since,
            tag,
            plain,
        }),

        Some(Commands::Open { query }) => cli::recent::open(&query),
        Some(Commands::Move { query, base, yes }) => {
            cli::move_project::run(cli::move_project::MoveArgs { query, base, yes })
        }

        Some(Commands::Reindex) => cli::reindex::run(),
        Some(Commands::Reconcile) => cli::reconcile::run(),

        Some(Commands::Register {
            path,
            recursive,
            dry_run,
            template,
            apply,
            rename,
            use_today,
            created,
            yes,
            extra,
        }) => {
            let classified = classify_for("register", extra)?;
            // clap's `requires`/`conflicts_with` only see flags written *before*
            // the path; `trailing_var_arg` swallows anything after it. So the
            // flags are merged first and the constraints checked on the merged
            // set — `fastf register X --dry-run` used to be dropped silently and
            // the folder written for real.
            let mut flags = cli::register::RegisterFlags {
                recursive,
                dry_run,
                template,
                apply,
                rename,
                use_today,
                created,
                yes,
            };
            flags.apply_extra(classified.recognized)?;
            flags.validate()?;
            if flags.recursive {
                cli::register::run_recursive(cli::register::RecursiveArgs {
                    base: std::path::PathBuf::from(path),
                    template_slug: flags.template,
                    vars: classified.vars,
                    use_today: flags.use_today,
                    dry_run: flags.dry_run,
                })
            } else {
                cli::register::run(cli::register::RegisterArgs {
                    path: std::path::PathBuf::from(path),
                    template_slug: flags.template,
                    vars: classified.vars,
                    apply_structure: flags.apply,
                    rename: flags.rename,
                    use_today: flags.use_today,
                    created_override: flags.created,
                    yes: flags.yes,
                })
            }
        }

        Some(Commands::Apply {
            template,
            target,
            dry_run,
            yes,
            extra,
        }) => {
            let classified = classify_for("apply", extra)?;
            let mut args = cli::apply::ApplyArgs {
                template_slug: template,
                target,
                dry_run,
                yes,
                vars: classified.vars,
            };
            cli::apply::apply_extra(&mut args, classified.recognized)?;
            cli::apply::run(args)
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
        Some(Commands::Paths) => cli::paths_cmd::run(),
        Some(Commands::Mangen { dir }) => generate_man_pages(&dir),
    }
}

/// Sort one subcommand's trailing bucket, using **that subcommand's own clap
/// declarations** as the list of flags to recognize.
///
/// Reading the list from clap is the point: the hand-written recognizer knew
/// five flags, `register` declares none of them, and every register flag typed
/// after the path was reported "unrecognized" and dropped.
fn classify_for(subcommand: &str, extra: Vec<String>) -> Result<cli::extra::ClassifiedExtra> {
    use clap::CommandFactory;
    let command = Cli::command();
    let sub = command
        .find_subcommand(subcommand)
        .expect("subcommand is declared on Cli");
    cli::extra::classify_extra(extra, sub)
}

/// Generate the full man-page set (fastf.1 + one page per subcommand) into
/// `dir`. Reached only via the hidden `fastf mangen` subcommand — release
/// packaging (GitHub workflow, PKGBUILD) is the intended caller.
fn generate_man_pages(dir: &std::path::Path) -> Result<()> {
    use clap::CommandFactory;
    std::fs::create_dir_all(dir)?;
    clap_mangen::generate_to(Cli::command(), dir)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{Cli, is_config_parse_failure};
    use clap::CommandFactory;
    use fastf::cli::extra::Recognized;

    /// Every flag a subcommand declares, as `classify_extra` would report it.
    fn every_declared_flag(subcommand: &str) -> Vec<Recognized> {
        let command = Cli::command();
        let sub = command.find_subcommand(subcommand).expect("declared");
        sub.get_arguments()
            .filter(|arg| arg.get_long().is_some())
            .filter(|arg| !matches!(arg.get_id().as_str(), "help" | "version"))
            .map(|arg| Recognized {
                name: arg.get_long().unwrap().to_string(),
                value: arg.get_action().takes_values().then(|| "x".to_string()),
            })
            .collect()
    }

    /// The guard that replaces the old "three coordinated edits" rule: declare a
    /// flag in clap, handle it in that command's `apply_extra`, and this test
    /// catches the case you forget. Before it, a flag added to clap kept working
    /// before the positional and silently did nothing after it.
    #[test]
    fn every_declared_flag_is_handled_after_the_positional() {
        let mut new_args = fastf::cli::new::NewArgs {
            template_slug: None,
            vars: Default::default(),
            dry_run: false,
            base_dir_override: None,
            no_preview: false,
            no_post: false,
            yes: false,
        };
        fastf::cli::new::apply_extra(&mut new_args, every_declared_flag("new"))
            .expect("every `new` flag must be handled after the slug");

        let mut apply_args = fastf::cli::apply::ApplyArgs {
            template_slug: String::new(),
            target: String::new(),
            dry_run: false,
            vars: Default::default(),
            yes: false,
        };
        fastf::cli::apply::apply_extra(&mut apply_args, every_declared_flag("apply"))
            .expect("every `apply` flag must be handled after the target");

        let mut register_flags = fastf::cli::register::RegisterFlags::default();
        register_flags
            .apply_extra(every_declared_flag("register"))
            .expect("every `register` flag must be handled after the path");
    }

    /// The whole point of the hint: a config that exists but does not parse
    /// stops every command, and no command can repair it.
    #[test]
    fn a_config_parse_failure_earns_the_hint() {
        assert!(is_config_parse_failure(
            "parsing /home/u/.config/fastf/config.toml: TOML parse error at line 9, column 1"
        ));
    }

    /// "Delete it to start over with defaults" would be ruinous advice about a
    /// template or a project's metadata.
    #[test]
    fn other_parse_failures_do_not() {
        assert!(!is_config_parse_failure(
            "parsing /home/u/.config/fastf/templates/general/template.yaml: mapping values are not allowed"
        ));
        assert!(!is_config_parse_failure(
            "reading /home/u/.config/fastf/config.toml: permission denied"
        ));
    }
}
