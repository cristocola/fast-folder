use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use std::collections::HashMap;

use crate::cli::new::{self, NewArgs};
use crate::cli::register::{self, RegisterArgs};
use crate::cli::{apply, config, id, recent, template};
use crate::core::config::Config;
use crate::core::template as core_template;
use crate::core::{library, query};
use crate::tui::dashboard::{Entry, SessionState};

const BANNER: &str = r#"  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|"#;
const BANNER_WIDTH: usize = 40;

// Escape semantics.
//
// Every `Select`/`Confirm`/`MultiSelect` here is driven through `interact_opt`,
// which returns `Ok(None)` on **Esc or `q`**. What that means is decided per
// prompt — Back in a submenu, Quit at the top level, No on a confirmation,
// cancel inside an action — but it is never an error, so containment
// (`contain`/`is_fatal`) is untouched: a prompt that genuinely cannot run still
// fails with `dialoguer::Error` and still ends the session.
//
// `Input` has no opt variant in dialoguer 0.11 and swallows Esc, so text
// prompts cancel on an empty answer instead. The three Settings values where
// empty is itself meaningful (base dir, default template, editor) cannot be
// escaped at all; their prompts say what empty does rather than pretend.

/// Index of the main menu's Quit row. Esc resolves to it, so the two exits are
/// literally the same code path.
const QUIT: usize = 6;

/// Should this error end the session, or just be reported?
///
/// A mistyped path or an out-of-range setting is an ordinary part of using a
/// menu; letting it unwind to `main` closed the whole TUI and threw away every
/// answer the user had already given.
///
/// The one thing that must never be contained is a failure of the **prompt
/// itself** — no terminal, or stdin at EOF. Containing that would return to a
/// loop that immediately prompts again and fails again, forever. `dialoguer`
/// returns exactly one error type and only from prompt calls, so that is the
/// discriminator.
///
/// Deliberately NOT "does the chain contain an `io::Error`": a bad path fails
/// with `canonicalize`'s `NotFound` wrapped in context, so that rule would
/// propagate the very case this exists to catch.
fn is_fatal(err: &anyhow::Error) -> bool {
    crate::util::interrupt::is_set() || err.downcast_ref::<dialoguer::Error>().is_some()
}

/// Report a recoverable error and stay in the menu; propagate a fatal one.
fn contain(result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(err) if is_fatal(&err) => Err(err),
        Err(err) => {
            eprintln!("{} {:#}", "error:".red().bold(), err);
            println!();
            Ok(())
        }
    }
}

/// `contain`, plus a `✗` line in the session log. Used by the top-level arms,
/// which are the ones with a session to report into.
fn contain_logged(result: Result<()>, session: &mut SessionState, what: &str) -> Result<()> {
    if let Err(err) = &result
        && !is_fatal(err)
    {
        session.log(Entry::failed(what, format!("{err:#}")));
    }
    contain(result)
}

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
    onboard_first_run(&initial)?;

    // Computed once here, then refreshed only by the arms that can change it —
    // see `dashboard`. A per-iteration refresh walks every base.
    let mut session = SessionState::new(&Config::load().unwrap_or_default());

    loop {
        // Reload config each iteration so changes in settings are reflected immediately
        let cfg = Config::load().unwrap_or_default();
        session.render(&cfg);

        // Esc (or `q`) quits from the top level, exactly like the Quit row.
        let choice = Select::new()
            .with_prompt("What would you like to do?")
            .items(&[
                "Create new project",
                "Projects",
                "Search projects",
                "Register existing folder",
                "Manage templates",
                "View / edit settings",
                "Quit",
            ])
            .default(0)
            .interact_opt()?
            .unwrap_or(QUIT);

        // Every arm is contained: a typo in a path or a setting returns to this
        // menu instead of ending the session. Each arm builds its outcome
        // first and hands it to `contain` — using `?` inline would silently opt
        // that path out of containment.
        match choice {
            0 => {
                let outcome = menu_create(&mut session);
                contain_logged(outcome, &mut session, "create")?;
            }
            1 => {
                let outcome = menu_projects(&mut session);
                contain_logged(outcome, &mut session, "browse")?;
            }
            2 => {
                let outcome = menu_search(&mut session);
                contain_logged(outcome, &mut session, "search")?;
            }
            3 => {
                let outcome = menu_register(&mut session);
                contain_logged(outcome, &mut session, "register")?;
            }
            // Templates change none of the numbers in the header.
            4 => contain(menu_templates())?,
            5 => {
                let outcome = menu_settings();
                contain_logged(outcome, &mut session, "settings")?;
                // Bases, page size and the counter all live under Settings.
                session.refresh(&Config::load().unwrap_or_default());
            }
            QUIT => {
                println!("Goodbye.");
                break;
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

/// First-run onboarding, mirroring the web UI's welcome dialog: when no base
/// is configured anywhere, ask where projects should live (defaulting to the
/// conventional `<home>/Projects`) and create + persist it via the shared
/// `config::init_base_dir`. Enter accepts the suggestion; an empty answer
/// skips (the prompt returns on the next launch until a base is set).
fn onboard_first_run(cfg: &Config) -> Result<()> {
    if !cfg.base_dir.trim().is_empty() || !cfg.bases.is_empty() {
        return Ok(());
    }
    let suggested = crate::core::config::suggested_base_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    println!(
        "  {}",
        "Welcome! Let's pick a home for your projects.".bold()
    );
    println!(
        "  {}",
        "Every new project is created inside this base folder.".dimmed()
    );
    println!(
        "  {}",
        "You can add more bases later (a second drive, a network share)".dimmed()
    );
    println!("  {}", "under Settings → Library bases.".dimmed());
    println!();
    loop {
        let answer: String = Input::new()
            .with_prompt("Projects base folder (empty to skip)")
            .with_initial_text(&suggested)
            .allow_empty(true)
            .interact_text()?;
        if answer.trim().is_empty() {
            println!(
                "  {}",
                "Skipped — set it anytime in Settings → Project basics.".dimmed()
            );
            println!();
            return Ok(());
        }
        match crate::core::config::init_base_dir(&answer) {
            Ok(resolved) => {
                println!(
                    "{}  Projects base set to {}",
                    "✓".green().bold(),
                    resolved.display().to_string().bold()
                );
                println!();
                return Ok(());
            }
            Err(error) => println!("{} {}", "error:".red().bold(), error),
        }
    }
}

fn menu_create(session: &mut SessionState) -> Result<()> {
    let tmpl = new::pick_template_interactively()?;
    let base_dir_override = match pick_base_interactively()? {
        Some(BaseChoice::Default) => None,
        Some(BaseChoice::Explicit(path)) => Some(path),
        None => {
            println!("{}", "  (cancelled)".dimmed());
            return Ok(());
        }
    };
    let args = NewArgs {
        template_slug: Some(tmpl.slug.clone()),
        vars: HashMap::new(),
        dry_run: false,
        base_dir_override,
        no_preview: false,
        no_post: false,
        yes: false,
    };
    new::run(args)?;
    // The diff is what tells the log the folder name — and, when the user
    // declined at the confirmation, that nothing happened at all.
    let diff = session.refresh(&Config::load().unwrap_or_default());
    session.log_added("created", &diff);
    println!();
    Ok(())
}

/// Which base a new project goes into.
///
/// `Default` and a cancellation are different answers, which is why this is not
/// an `Option<String>`: `None` used to mean "fall through to `config.base_dir`",
/// so reusing it for Esc would silently create the project instead of
/// abandoning it.
enum BaseChoice {
    Default,
    Explicit(String),
}

/// When more than one base is configured, ask which one the new project should
/// be created in. `Ok(None)` is a cancellation; `BaseChoice::Default` (also the
/// answer when there is only one base) defers to `config.base_dir`.
fn pick_base_interactively() -> Result<Option<BaseChoice>> {
    let cfg = Config::load().unwrap_or_default();
    let bases: Vec<std::path::PathBuf> = cfg
        .effective_bases()
        .into_iter()
        .filter(|b| b.is_dir())
        .collect();
    if bases.len() <= 1 {
        return Ok(Some(BaseChoice::Default));
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
    let Some(idx) = Select::new()
        .with_prompt("Create the project in which base?")
        .items(&labels)
        .default(0)
        .interact_opt()?
    else {
        return Ok(None);
    };
    if idx == 0 {
        // The default base — let config.base_dir resolution do its thing.
        return Ok(Some(BaseChoice::Default));
    }
    Ok(Some(BaseChoice::Explicit(bases[idx].display().to_string())))
}

fn menu_projects(session: &mut SessionState) -> Result<()> {
    let page_size = Config::load()
        .unwrap_or_default()
        .recent_default_limit
        .max(1);
    let events = recent::run_paged_browser(
        page_size,
        "No projects yet — create one with `fastf new`.",
        || {
            let cfg = Config::load().unwrap_or_default();
            library::discover(&cfg)
        },
    )?;
    absorb(session, events);
    println!();
    Ok(())
}

/// Fold a browser session's mutations into the log, and recompute the header
/// numbers only if there were any.
fn absorb(session: &mut SessionState, events: Vec<recent::ActionEvent>) {
    if events.is_empty() {
        return;
    }
    for event in events {
        session.log(Entry::done(event.verb, event.subject));
    }
    session.refresh(&Config::load().unwrap_or_default());
}

fn menu_search(session: &mut SessionState) -> Result<()> {
    let query: String = Input::new()
        .with_prompt("Search query (e.g. tag:draft  template=music-video  artist=Aria*)")
        .allow_empty(true)
        .interact_text()?;
    let query = query.trim().to_string();
    if query.is_empty() {
        return cancelled();
    }
    let terms: Vec<String> = query.split_whitespace().map(|s| s.to_string()).collect();
    let predicates = query::parse(&terms);
    let page_size = Config::load()
        .unwrap_or_default()
        .recent_default_limit
        .max(1);
    let events = recent::run_paged_browser(page_size, "No projects match that query.", || {
        let cfg = Config::load().unwrap_or_default();
        crate::cli::search::matching_projects(&cfg, &predicates)
    })?;
    absorb(session, events);
    println!();
    Ok(())
}

fn menu_apply() -> Result<()> {
    let Some(slug) = prompt_template_slug("Template to apply")? else {
        return cancelled();
    };
    let target: String = Input::new()
        .with_prompt("Target folder (empty to cancel)")
        .allow_empty(true)
        .interact_text()?;
    if target.trim().is_empty() {
        return cancelled();
    }
    let Some(dry_run) = Confirm::new()
        .with_prompt("Dry run first (preview only)?")
        .default(true)
        .interact_opt()?
    else {
        return cancelled();
    };

    // Collect the variables once and reuse them for both passes. Running apply
    // twice with an empty map meant answering every prompt again just to confirm
    // the preview you had already approved.
    let tmpl = core_template::find_by_slug(&slug)?;
    let vars = apply::collect_if_needed(&tmpl, &HashMap::new())?;

    apply::run(apply::ApplyArgs {
        template_slug: slug.clone(),
        target: target.clone(),
        dry_run,
        yes: false,
        vars: vars.clone(),
    })?;

    if dry_run {
        // Esc here is No — the preview already ran, so there is nothing to undo.
        let proceed = Confirm::new()
            .with_prompt("Apply for real now?")
            .default(false)
            .interact_opt()?;
        if proceed == Some(true) {
            apply::run(apply::ApplyArgs {
                template_slug: slug,
                target,
                dry_run: false,
                // Same answers — `collect_vars` finds them all and prompts for none.
                yes: true,
                vars,
            })?;
        }
    }
    println!();
    Ok(())
}

fn menu_register(session: &mut SessionState) -> Result<()> {
    // 1. Folder path.
    let path: String = Input::new()
        .with_prompt("Existing folder to register (empty to cancel)")
        .allow_empty(true)
        .interact_text()?;
    let path = path.trim();
    if path.is_empty() {
        return cancelled();
    }

    // 2. Optional template — first ask, then pick if Yes. Skipping is fully
    //    supported: register writes a minimal record with template "(registered)".
    let Some(use_template) = Confirm::new()
        .with_prompt("Attach a template (enables tags + variable capture)?")
        .default(false)
        .interact_opt()?
    else {
        return cancelled();
    };

    let template_slug = if use_template {
        match core_template::load_all() {
            Ok(ts) if !ts.is_empty() => match prompt_template_slug("Template to attach")? {
                Some(slug) => Some(slug),
                None => return cancelled(),
            },
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
    let Some(rename) = Confirm::new()
        .with_prompt("Standardize folder name (rename to match pattern)?")
        .default(true)
        .interact_opt()?
    else {
        return cancelled();
    };

    // 4. Optional --apply (only meaningful with a template).
    let apply_structure = if template_slug.is_some() {
        match Confirm::new()
            .with_prompt("Fill in missing template folders/files?")
            .default(false)
            .interact_opt()?
        {
            Some(answer) => answer,
            None => return cancelled(),
        }
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
    let diff = session.refresh(&Config::load().unwrap_or_default());
    session.log_added("registered", &diff);
    println!();
    Ok(())
}

fn menu_templates() -> Result<()> {
    loop {
        let Some(choice) = Select::new()
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
            .interact_opt()?
        else {
            break;
        };

        // Each arm yields a Result rather than using `?` inline, so one
        // contained failure (an unreadable template, a missing source folder)
        // returns to this menu instead of unwinding out of the TUI.
        let outcome = match choice {
            0 => template::new_interactive(),
            1 => template_from_folder_flow(),
            2 => with_template_slug("Edit template", template::edit),
            3 => menu_apply(),
            4 => template::list(),
            5 => with_template_slug("Show template", template::show),
            6 => with_template_slug("Delete template", |slug| template::delete(slug, false)),
            7 => break,
            _ => unreachable!(),
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

fn template_from_folder_flow() -> Result<()> {
    let path: String = Input::new()
        .with_prompt("Source folder to scan (empty to cancel)")
        .allow_empty(true)
        .interact_text()?;
    if path.trim().is_empty() {
        return cancelled();
    }
    let slug: String = Input::new()
        .with_prompt("Slug for the new template (empty to cancel)")
        .allow_empty(true)
        .interact_text()?;
    if slug.trim().is_empty() {
        return cancelled();
    }
    let Some(force) = Confirm::new()
        .with_prompt("Overwrite if a template with this slug exists?")
        .default(false)
        .interact_opt()?
    else {
        return cancelled();
    };
    template::run_from_folder(&path, &slug, force, false)
}

/// Report an abandoned action and return to the menu. Every cancellation says
/// the same thing in the same place, so an escaped prompt never looks like a
/// silent no-op.
fn cancelled() -> Result<()> {
    println!("{}", "  (cancelled)".dimmed());
    Ok(())
}

// ---------------------------------------------------------------------------
// ID counter
// ---------------------------------------------------------------------------

fn menu_id() -> Result<()> {
    loop {
        // `show` converges the counter across bases, so it can fail on an
        // unwritable base — report it and stay rather than dropping the user out.
        contain(id::show())?;
        println!();
        let Some(choice) = Select::new()
            .with_prompt("ID counter")
            .items(&[
                "Raise counter value",
                "Sync every base to the highest ID",
                "Back",
            ])
            .default(2)
            .interact_opt()?
        else {
            break;
        };

        let outcome = match choice {
            0 => {
                let val: String = Input::new()
                    .with_prompt(
                        "Raise counter to (next project will be this + 1, empty to cancel)",
                    )
                    .allow_empty(true)
                    .interact_text()?;
                match val.trim() {
                    "" => cancelled(),
                    trimmed => match trimmed.parse::<u64>() {
                        Ok(n) => id::set(n),
                        Err(_) => Err(anyhow::anyhow!("expected a number, got '{trimmed}'")),
                    },
                }
            }
            1 => id::sync(),
            2 => break,
            _ => unreachable!(),
        };
        // A refusal ("cannot go below 82") is information, not a reason to quit.
        contain(outcome)?;
        println!();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings — grouped submenus
// ---------------------------------------------------------------------------

fn menu_settings() -> Result<()> {
    loop {
        contain(config::show())?;
        println!();
        let Some(choice) = Select::new()
            .with_prompt("Settings")
            .items(&[
                "Project basics  (base dir / template / date / editor)",
                "Workflow prompts  (open prompt / confirm / banner / preview)",
                "Library bases  (extra folders to index)",
                "Projects  (TUI page size / CLI recent limit)",
                "Post-create actions  (git / reveal / editor / path / commands)",
                "ID counter",
                "Back",
            ])
            .default(0)
            .interact_opt()?
        else {
            break;
        };

        let outcome = match choice {
            0 => menu_settings_basics(),
            1 => menu_settings_workflow(),
            2 => menu_settings_bases(),
            3 => menu_settings_recent(),
            4 => menu_settings_postcreate(),
            5 => menu_id(),
            6 => break,
            _ => unreachable!(),
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

fn menu_settings_basics() -> Result<()> {
    loop {
        let Some(choice) = Select::new()
            .with_prompt("Project basics")
            .items(&[
                "Set base directory",
                "Set default template",
                "Set date format",
                "Set editor",
                "Back",
            ])
            .default(0)
            .interact_opt()?
        else {
            break;
        };

        let outcome = match choice {
            0 => {
                println!(
                    "  {}  Linux/macOS: /home/user/Projects  ·  Windows: C:\\Users\\user\\Projects or C:/Users/user/Projects",
                    "Hint:".yellow()
                );
                let val: String = Input::new()
                    // Empty falls back to the HOME directory, not the cwd — that
                    // changed in v1.0.2 and this hint kept saying otherwise.
                    .with_prompt("Base directory (empty = your home directory)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("base-dir", &val)
            }
            1 => {
                println!(
                    "  {}  `fastf new` uses it without asking; this menu preselects it in the picker.",
                    "Hint:".yellow()
                );
                let val: String = Input::new()
                    .with_prompt("Default template slug (empty = no default)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("default-template", &val)
            }
            2 => {
                let val: String = Input::new()
                    .with_prompt("Date format (strftime, e.g. %Y-%m-%d)")
                    .default("%Y-%m-%d".to_string())
                    .interact_text()?;
                config::set("date-format", &val)
            }
            3 => {
                let val: String = Input::new()
                    .with_prompt("Editor command (e.g. nvim, code, nano)")
                    .allow_empty(true)
                    .interact_text()?;
                config::set("editor", &val)
            }
            4 => break,
            _ => unreachable!(),
        };
        // A rejected value (relative base dir, unknown template) is a correction
        // to make here, not a reason to leave the menu.
        contain(outcome)?;
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
            format!(
                "Duplicate folder name  [{}]",
                if cfg.suffix_on_name_collision() {
                    "add _2 suffix"
                } else {
                    "refuse"
                }
            ),
            "Back".to_string(),
        ];
        let Some(choice) = Select::new()
            .with_prompt("Workflow prompts")
            .items(&items)
            .default(0)
            .interact_opt()?
        else {
            break;
        };

        let outcome = match choice {
            0 => toggle_setting("prompt-open-after-create", cfg.prompt_open_after_create),
            1 => toggle_setting("confirm-create", cfg.confirm_create),
            2 => toggle_setting("show-banner", cfg.show_banner),
            3 => {
                let val: String = Input::new()
                    .with_prompt("Lines per file in dry-run (0 = none)")
                    .default(cfg.preview_lines.to_string())
                    .interact_text()?;
                config::set("preview-lines", &val)
            }
            // Not a bool on disk — it stores "suffix" or "error".
            4 => config::set(
                "on-name-collision",
                if cfg.suffix_on_name_collision() {
                    "error"
                } else {
                    "suffix"
                },
            ),
            5 => break,
            _ => unreachable!(),
        };
        contain(outcome)?;
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

        let Some(choice) = Select::new()
            .with_prompt("Library bases")
            .items(&items)
            .default(0)
            .interact_opt()?
        else {
            break;
        };

        let outcome = if choice == 0 {
            let val: String = Input::new()
                .with_prompt("Base directory to add (absolute path)")
                .allow_empty(true)
                .interact_text()?;
            let val = val.trim();
            if val.is_empty() {
                Ok(())
            } else {
                let mut bases = cfg.bases.clone();
                if !bases.iter().any(|b| b == val) {
                    bases.push(val.to_string());
                }
                // `config::set` validates each entry (absolute, `~` expanded)
                // and only warns about one that is merely absent — an unmounted
                // drive is a legitimate base.
                config::set("bases", &bases.join(","))
            }
        } else if choice == items.len() - 1 {
            break;
        } else {
            let mut bases = cfg.bases.clone();
            let idx = choice - 1;
            if idx < bases.len() {
                bases.remove(idx);
            }
            config::set("bases", &bases.join(","))
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

fn menu_settings_recent() -> Result<()> {
    loop {
        let cfg = Config::load().unwrap_or_default();
        let items = [
            format!("Projects page size  [{}]", cfg.recent_default_limit),
            "Back".to_string(),
        ];
        let Some(choice) = Select::new()
            .with_prompt("Projects")
            .items(&items)
            .default(0)
            .interact_opt()?
        else {
            break;
        };

        let outcome = match choice {
            0 => {
                let val: String = Input::new()
                    .with_prompt("TUI page size and default --limit for `fastf recent`")
                    .default(cfg.recent_default_limit.to_string())
                    .interact_text()?;
                config::set("recent-default-limit", &val)
            }
            1 => break,
            _ => unreachable!(),
        };
        contain(outcome)?;
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
        let Some(choice) = Select::new()
            .with_prompt("Post-create actions (default for new projects)")
            .items(&items)
            .default(0)
            .interact_opt()?
        else {
            break;
        };

        let outcome = match choice {
            0 => toggle_setting("post_create.git_init", pc.git_init),
            1 => toggle_setting("post_create.reveal", pc.reveal),
            2 => toggle_setting("post_create.open_in_editor", pc.open_in_editor),
            3 => toggle_setting("post_create.print_path", pc.print_path),
            4 => edit_postcreate_commands(),
            5 => break,
            _ => unreachable!(),
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

/// Add or remove post-create shell commands.
///
/// **Collect every answer first, then take the lock, reload, mutate and save.**
/// This used to load the config, sit on a prompt for as long as the terminal was
/// unattended, and then write the whole struct back — reverting anything the
/// browser UI or another `fastf config set` had changed in the meantime, with no
/// sign that it had happened. `cli::new::run` is the same shape: decide outside
/// the lock, commit inside it.
fn edit_postcreate_commands() -> Result<()> {
    let cfg = Config::load()?;
    let cmds = &cfg.post_create.commands;

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
        .interact_opt()?
        .unwrap_or(2); // Esc = Done.

    // What the user asked for, decided entirely outside the lock.
    enum Edit {
        Add(String),
        Remove(Vec<usize>),
        Nothing,
    }

    let edit = match choice {
        0 => {
            println!(
                "  {}  Use {{path}} as a placeholder for the absolute project path.",
                "Hint:".yellow()
            );
            let cmd: String = Input::new()
                .with_prompt("Shell command")
                .allow_empty(true)
                .interact_text()?;
            if cmd.trim().is_empty() {
                Edit::Nothing
            } else {
                Edit::Add(cmd)
            }
        }
        1 => {
            if cmds.is_empty() {
                println!("  {} nothing to remove.", "·".dimmed());
                return Ok(());
            }
            let labels: Vec<&str> = cmds.iter().map(String::as_str).collect();
            Edit::Remove(
                MultiSelect::new()
                    .with_prompt("Select commands to remove (Space to toggle, Enter to confirm)")
                    .items(&labels)
                    .interact_opt()?
                    .unwrap_or_default(), // Esc = remove nothing.
            )
        }
        2 => Edit::Nothing,
        _ => unreachable!(),
    };

    if matches!(edit, Edit::Nothing) {
        return Ok(());
    }

    match edit {
        Edit::Add(cmd) => {
            crate::core::operations::update_config(move |fresh| {
                fresh.post_create.commands.push(cmd);
                Ok(())
            })?;
            println!("  {} command added.", "✓".green());
        }
        Edit::Remove(picks) => {
            // Indices refer to the list the user saw. If another process edited
            // the commands meanwhile, removing by position would delete the
            // wrong one — so match on the text and report anything that moved.
            let mut missed = picks
                .iter()
                .filter(|&&index| cmds.get(index).is_none())
                .count();
            let targets = picks
                .into_iter()
                .filter_map(|index| cmds.get(index).cloned())
                .collect::<Vec<_>>();
            crate::core::operations::update_config(|fresh| {
                for target in targets {
                    if let Some(position) = fresh
                        .post_create
                        .commands
                        .iter()
                        .position(|command| command == &target)
                    {
                        fresh.post_create.commands.remove(position);
                    } else {
                        missed += 1;
                    }
                }
                Ok(())
            })?;
            if missed > 0 {
                println!(
                    "  {} {missed} command(s) had already changed elsewhere and were left alone.",
                    "note:".yellow()
                );
            }
            println!("  {} updated.", "✓".green());
        }
        Edit::Nothing => unreachable!(),
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

/// `Ok(None)` means the user escaped out of the picker — the caller abandons
/// whatever it was collecting.
fn prompt_template_slug(prompt: &str) -> Result<Option<String>> {
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
        .interact_opt()?;
    Ok(idx.map(|idx| templates[idx].slug.clone()))
}

/// Run `action` on a picked template slug, or do nothing when the pick was
/// escaped. Keeps the Templates menu arms one line each.
fn with_template_slug(prompt: &str, action: impl FnOnce(&str) -> Result<()>) -> Result<()> {
    match prompt_template_slug(prompt)? {
        Some(slug) => action(&slug),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::is_fatal;

    /// A mistyped path fails with `canonicalize`'s `NotFound` wrapped in
    /// context. This is THE case containment exists for, and the obvious
    /// "propagate anything with an io::Error in the chain" rule would have got
    /// it exactly backwards.
    #[test]
    fn a_wrapped_io_error_is_recoverable() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory");
        let err = anyhow::Error::new(io).context("path does not exist or is not accessible");
        assert!(!is_fatal(&err));
    }

    /// A validation failure from `config::set` is likewise a correction to make,
    /// not a reason to end the session.
    #[test]
    fn a_plain_message_is_recoverable() {
        let err = anyhow::anyhow!("recent_default_limit must be at least 1");
        assert!(!is_fatal(&err));
    }

    /// A prompt that cannot run must end the session: containing it would return
    /// to a loop that prompts again and fails again, forever.
    #[test]
    fn a_failed_prompt_is_fatal() {
        let err = anyhow::Error::new(dialoguer::Error::IO(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "not a terminal",
        )));
        assert!(is_fatal(&err));
    }

    /// …including when a command wrapped it in context on the way up.
    #[test]
    fn a_failed_prompt_stays_fatal_under_context() {
        let err = anyhow::Error::new(dialoguer::Error::IO(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "not a terminal",
        )))
        .context("collecting template variables");
        assert!(is_fatal(&err));
    }
}
