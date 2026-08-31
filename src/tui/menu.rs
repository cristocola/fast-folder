use anyhow::Result;
use colored::Colorize;
use std::collections::HashMap;

use crate::cli::new::{self, NewArgs};
use crate::cli::register::{self, RegisterArgs};
use crate::cli::{apply, config, id, template};
use crate::core::config::Config;
use crate::core::template as core_template;
use crate::core::{library, query};
use crate::tui::browser;
use crate::tui::pickers::{pick_base, pick_template};
use crate::tui::prompt::{self, TextOpts};

const BANNER: &str = r#"  ___        _      ___    _    _
 | __|_ _ __| |_   | __|__| |__| |___ _ _
 | _/ _` (_-<  _|  | _/ _ \ / _` / -_) '_|
 |_|\__,_/__/\__|  |_|\___/_\__,_\___|_|"#;
const BANNER_WIDTH: usize = 40;

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

/// Say which configured bases are not available, and why, once per list.
///
/// A base that is not mounted is ordinary — an external drive that is not
/// plugged in. One that does not answer is worth naming, because the alternative
/// is a menu that appears to hang.
fn report_unusable_bases(unusable: &[(std::path::PathBuf, crate::util::paths::Probe)]) {
    for (path, probe) in unusable {
        println!(
            "  {} {}{}",
            "·".dimmed(),
            crate::util::paths::display_path(path).dimmed(),
            probe.note().dimmed()
        );
    }
}

/// A folder that must already exist, checked at the prompt that asks for it.
///
/// **Validate at the prompt, not at the operation.** A path typed wrong used to
/// be rejected by the core operation *after* three more questions had been
/// answered, and all four answers went with it. Rejecting it here keeps the
/// text on the line to be corrected.
fn existing_directory(raw: &str) -> std::result::Result<(), String> {
    let path = std::path::Path::new(raw.trim());
    if raw.trim().is_empty() {
        return Err("enter a folder path".to_string());
    }
    if !path.exists() {
        return Err(format!("no such folder: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("not a folder: {}", path.display()));
    }
    Ok(())
}

/// Draw one menu and return the **label** that was chosen.
///
/// `Ok(None)` is Esc, and every menu treats that as its own Back item — the
/// visible row stays because it is the discoverable path; Esc is the shortcut.
///
/// Labels rather than indices: every submenu used to `match` on a raw index with
/// a trailing `unreachable!()`, so inserting a row silently reassigned the ones
/// below it, and `move_idx` was a hard-coded `6`.
fn menu(prompt: &str, items: &[&str], default: usize) -> Result<Option<String>> {
    let owned: Vec<String> = items.iter().map(|s| (*s).to_string()).collect();
    Ok(prompt::select(prompt, &owned, default)?.map(|index| owned[index].clone()))
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

pub fn run() -> Result<()> {
    // Checked before the banner: a menu that cannot be driven should not put
    // decoration on stdout for a session that never happens.
    crate::util::tty::require_tty(
        "show the menu",
        "run a subcommand instead — see `fastf --help`",
    )?;
    // Banner is shown once based on the first config load. Honors show_banner.
    let initial = Config::load()?;
    if initial.show_banner {
        println!();
        println!("{}", BANNER.cyan().bold());
        let tagline = format!("project scaffolder · v{}", env!("CARGO_PKG_VERSION"));
        let pad = BANNER_WIDTH.saturating_sub(tagline.chars().count());
        println!("{}{}", " ".repeat(pad), tagline.dimmed());
        println!();
    }
    onboard_first_run(&initial)?;

    loop {
        // Reload config each iteration so changes in settings are reflected immediately
        let cfg = Config::load()?;
        crate::tui::frame::print(&cfg);

        let choice = menu(
            "What would you like to do?",
            &[
                "Create new project",
                "Project list",
                "Search projects",
                "Register existing folder",
                "Manage templates",
                "View / edit settings",
                "Quit",
            ],
            0,
        )?;

        // Every arm is contained: a typo in a path or a setting returns to this
        // menu instead of ending the session.
        // Esc at the top level is Quit — there is no parent to go back to.
        match choice.as_deref() {
            Some("Create new project") => contain(menu_create())?,
            Some("Project list") => contain(menu_projects())?,
            Some("Search projects") => contain(menu_search())?,
            Some("Register existing folder") => contain(menu_register())?,
            Some("Manage templates") => contain(menu_templates())?,
            Some("View / edit settings") => contain(menu_settings())?,
            Some("Quit") | None => {
                println!("Goodbye.");
                break;
            }
            Some(other) => anyhow::bail!("unhandled menu item '{other}'"),
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
        let answer = prompt::text(
            "Projects base folder (empty to skip)",
            TextOpts::new().initial(suggested.clone()).allow_empty(),
        )?
        .unwrap_or_default();
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

fn menu_create() -> Result<()> {
    let Some(tmpl) = pick_template("Select template", "name it instead: `fastf new <slug>`")?
    else {
        prompt::report_cancelled("nothing was created");
        println!();
        return Ok(());
    };
    let Some(base_dir_override) = pick_base_interactively()? else {
        prompt::report_cancelled("nothing was created");
        println!();
        return Ok(());
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
    println!();
    Ok(())
}

/// When more than one base is configured, ask which one the new project should
/// be created in. Returns `None` (= config default) when there's only one base
/// or the default entry is chosen.
/// The outer `Option` is the cancel: `None` means Esc, and the create is
/// abandoned. The inner one is the answer, where `None` means "the default base"
/// and lets `config.base_dir` resolution do its thing.
fn pick_base_interactively() -> Result<Option<Option<String>>> {
    let cfg = Config::load()?;
    let (bases, unusable) = crate::util::paths::mounted_bases(&cfg.effective_bases());
    report_unusable_bases(&unusable);
    if bases.len() <= 1 {
        return Ok(Some(None));
    }
    let default_base = bases.first().cloned();
    let picked = pick_base(
        "Create the project in which base?",
        &bases,
        default_base.as_deref(),
        "name it instead: `fastf new <slug> --base-dir=<path>`",
        false,
    )?;
    match picked {
        Some(base) if Some(&base) == default_base.as_ref() => Ok(Some(None)),
        Some(base) => Ok(Some(Some(base.display().to_string()))),
        None => Ok(None),
    }
}

fn menu_projects() -> Result<()> {
    let page_size = Config::load()?.recent_default_limit.max(1);
    browser::run_paged_browser(
        page_size,
        "No projects yet — create one with `fastf new`.",
        "Back to main menu",
        || {
            let cfg = Config::load()?;
            Ok(library::discover(&cfg))
        },
        // Every project belongs in the full library.
        |_| true,
    )?;
    println!();
    Ok(())
}

/// Search, with the query kept across a miss.
///
/// A query that matched nothing used to drop straight back to the main menu,
/// so the only way to fix a typo in `tag:draft` was to type the whole thing
/// again. It now comes back in the field, editable.
fn menu_search() -> Result<()> {
    let mut previous = String::new();
    loop {
        let mut opts = TextOpts::new().allow_empty();
        if !previous.is_empty() {
            opts = opts.initial(previous.clone());
        }
        let Some(query) = prompt::text(
            "Search query (e.g. tag:draft  template=music-video  artist=Aria*)",
            opts,
        )?
        else {
            return Ok(());
        };
        let query = query.trim().to_string();
        if query.is_empty() {
            println!("{}", "  (cancelled)".dimmed());
            return Ok(());
        }

        let terms: Vec<String> = query.split_whitespace().map(|s| s.to_string()).collect();
        let predicates = query::parse(&terms);
        let cfg = Config::load()?;
        let matches = crate::cli::search::matching_projects(&cfg, &predicates);
        if matches.is_empty() {
            println!("{}", "No projects match that query.".dimmed());
            previous = query;
            continue;
        }

        let page_size = cfg.recent_default_limit.max(1);
        browser::run_paged_browser(
            page_size,
            "No projects match that query.",
            "Back to main menu",
            || {
                let cfg = Config::load()?;
                Ok(crate::cli::search::matching_projects(&cfg, &predicates))
            },
            |project| crate::cli::search::still_matches(project, &predicates),
        )?;
        println!();
        return Ok(());
    }
}

fn menu_apply() -> Result<()> {
    let Some(slug) = prompt_template_slug("Template to apply")? else {
        return Ok(());
    };
    // Before the dry-run question and before every variable: `apply` used to
    // reject the target only after all of them had been answered.
    let Some(target) = prompt::text(
        "Target folder",
        TextOpts::new().validate(existing_directory),
    )?
    else {
        return Ok(());
    };
    let Some(dry_run) = prompt::confirm("Dry run first (preview only)?", true)? else {
        return Ok(());
    };

    // Collect the variables once and reuse them for both passes. Running apply
    // twice with an empty map meant answering every prompt again just to confirm
    // the preview you had already approved.
    let tmpl = core_template::find_by_slug(&slug)?;
    let Some(vars) = apply::collect_if_needed(&tmpl, &HashMap::new())? else {
        prompt::report_cancelled("nothing was applied");
        return Ok(());
    };

    apply::run(apply::ApplyArgs {
        template_slug: slug.clone(),
        target: target.clone(),
        dry_run,
        yes: false,
        vars: vars.clone(),
    })?;

    if dry_run {
        let proceed = prompt::confirm("Apply for real now?", false)?.unwrap_or(false);
        if proceed {
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

fn menu_register() -> Result<()> {
    let Some(scope) = menu(
        "Register what?",
        &["One folder", "Every unregistered folder in a base", "Back"],
        0,
    )?
    else {
        return Ok(());
    };
    match scope.as_str() {
        "One folder" => menu_register_one(),
        "Every unregistered folder in a base" => menu_register_recursive(),
        _ => Ok(()),
    }
}

/// Bulk onboarding: preview first, then commit. The preview is the same
/// renderer `fastf register --recursive --dry-run` uses.
fn menu_register_recursive() -> Result<()> {
    let Some(base) = prompt::text(
        "Base folder whose children to register",
        TextOpts::new().validate(existing_directory),
    )?
    else {
        return Ok(());
    };
    let Some(template_slug) = ask_optional_template()? else {
        return Ok(());
    };
    let Some(use_today) = ask_created_is_today()? else {
        return Ok(());
    };

    register::run_recursive(register::RecursiveArgs {
        base: std::path::PathBuf::from(base.trim()),
        template_slug: template_slug.clone(),
        vars: HashMap::new(),
        use_today,
        dry_run: true,
    })?;

    let Some(true) = prompt::confirm("Register these folders now?", false)? else {
        prompt::report_cancelled("nothing was registered");
        println!();
        return Ok(());
    };

    register::run_recursive(register::RecursiveArgs {
        base: std::path::PathBuf::from(base.trim()),
        template_slug,
        vars: HashMap::new(),
        use_today,
        dry_run: false,
    })?;
    println!();
    Ok(())
}

/// "Attach a template?" then the picker. The outer `Option` is the cancel.
fn ask_optional_template() -> Result<Option<Option<String>>> {
    let Some(use_template) = prompt::confirm(
        "Attach a template (enables tags + variable capture)?",
        false,
    )?
    else {
        return Ok(None);
    };
    if !use_template {
        return Ok(Some(None));
    }
    match core_template::load_all() {
        Ok(ts) if !ts.is_empty() => Ok(prompt_template_slug("Template to attach")?.map(Some)),
        Ok(_) => {
            println!(
                "  {} no templates available — continuing without one.",
                "·".dimmed()
            );
            Ok(Some(None))
        }
        Err(e) => {
            println!(
                "  {} could not load templates ({e}) — continuing without one.",
                "warning:".yellow().bold()
            );
            Ok(Some(None))
        }
    }
}

/// Which date a registered project should claim as its creation date.
///
/// `register` defaults to the folder's own timestamp, which is almost always
/// what a folder that predates fastf should keep. The menu could not say
/// otherwise at all before.
fn ask_created_is_today() -> Result<Option<bool>> {
    let Some(choice) = menu(
        "Created date for these projects",
        &["The folder's own date", "Today"],
        0,
    )?
    else {
        return Ok(None);
    };
    Ok(Some(choice == "Today"))
}

fn menu_register_one() -> Result<()> {
    // 1. Folder path.
    // Validated here, before the three questions that follow it.
    let Some(path) = prompt::text(
        "Existing folder to register",
        TextOpts::new().validate(existing_directory),
    )?
    else {
        return Ok(());
    };
    let path = path.trim();

    // 2. Optional template — first ask, then pick if Yes. Skipping is fully
    //    supported: register writes a minimal record with template "(registered)".
    let Some(use_template) = prompt::confirm(
        "Attach a template (enables tags + variable capture)?",
        false,
    )?
    else {
        return Ok(());
    };

    let template_slug = if use_template {
        match core_template::load_all() {
            Ok(ts) if !ts.is_empty() => match prompt_template_slug("Template to attach")? {
                Some(slug) => Some(slug),
                None => return Ok(()),
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
    let Some(rename) = prompt::confirm("Standardize folder name (rename to match pattern)?", true)?
    else {
        return Ok(());
    };

    let Some(use_today) = ask_created_is_today()? else {
        return Ok(());
    };

    // 4. Optional --apply (only meaningful with a template).
    let apply_structure = if template_slug.is_some() {
        match prompt::confirm("Fill in missing template folders/files?", false)? {
            Some(answer) => answer,
            None => return Ok(()),
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
        use_today,
        created_override: None,
        yes: false,
    })?;
    println!();
    Ok(())
}

fn menu_templates() -> Result<()> {
    loop {
        let Some(choice) = menu(
            "Templates",
            &[
                "Create new template",
                "Generate template from existing folder",
                "Edit a template",
                "Apply template to existing folder",
                "List templates",
                "Show template details",
                "Delete a template",
                "Back",
            ],
            0,
        )?
        else {
            break;
        };

        // Each arm yields a Result rather than using `?` inline, so one
        // contained failure (an unreadable template, a missing source folder)
        // returns to this menu instead of unwinding out of the TUI.
        let outcome = match choice.as_str() {
            "Create new template" => template::new_interactive(),
            "Generate template from existing folder" => template_from_folder_flow(),
            "Edit a template" => on_template("Edit template", template::edit),
            "Apply template to existing folder" => menu_apply(),
            "List templates" => template::list(),
            "Show template details" => on_template("Show template", template::show),
            "Delete a template" => on_template("Delete template", |s| template::delete(s, false)),
            "Back" => break,
            other => anyhow::bail!("unhandled menu item '{other}'"),
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

/// Ask which template, then run `action` on its slug. `None` from the picker is
/// a cancel, which is an answer, not a failure.
fn on_template(prompt: &str, action: impl FnOnce(&str) -> Result<()>) -> Result<()> {
    match prompt_template_slug(prompt)? {
        Some(slug) => action(&slug),
        None => Ok(()),
    }
}

fn template_from_folder_flow() -> Result<()> {
    let Some(path) = prompt::text(
        "Source folder to scan",
        TextOpts::new().validate(existing_directory),
    )?
    else {
        return Ok(());
    };
    let Some(slug) = prompt::text(
        "Slug for the new template",
        TextOpts::new().validate(|raw| {
            crate::core::validated::TemplateSlug::parse(raw.trim())
                .map(|_| ())
                .map_err(|e| format!("{e:#}"))
        }),
    )?
    else {
        return Ok(());
    };
    let Some(force) = prompt::confirm("Overwrite if a template with this slug exists?", false)?
    else {
        return Ok(());
    };
    // The menu used to hard-code `bundle_assets: false`, so binary files in the
    // source folder were silently left out of the template and there was no way
    // to ask for them without dropping to the command line. `run_from_folder`
    // does its own size confirmation, so answering yes here does not commit to
    // anything: `yes: false` leaves that second question in place.
    let Some(bundle_assets) = prompt::confirm(
        "Bundle binary and large files into the template (copied byte-for-byte)?",
        false,
    )?
    else {
        return Ok(());
    };
    template::run_from_folder(template::FromFolderArgs {
        path,
        slug,
        force,
        bundle_assets,
        yes: false,
        dry_run: false,
    })
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
        // Default stays on Back: this menu's other two items write.
        let Some(choice) = menu(
            "ID counter",
            &[
                "Raise counter value",
                "Sync every base to the highest ID",
                "Back",
            ],
            2,
        )?
        else {
            break;
        };

        let outcome = match choice.as_str() {
            "Raise counter value" => {
                match prompt::text(
                    "Raise counter to (next project will be this + 1)",
                    TextOpts::new().validate(|value| {
                        value
                            .trim()
                            .parse::<u64>()
                            .map(|_| ())
                            .map_err(|_| format!("expected a number, got '{}'", value.trim()))
                    }),
                )? {
                    Some(val) => id::set(val.trim().parse::<u64>().unwrap_or_default()),
                    None => Ok(()),
                }
            }
            "Sync every base to the highest ID" => id::sync(),
            "Back" => break,
            other => anyhow::bail!("unhandled menu item '{other}'"),
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
        let Some(choice) = menu(
            "Settings",
            &[
                "Project basics  (base dir / template / date / editor)",
                "Workflow prompts  (open prompt / confirm / banner / preview)",
                "Library bases  (extra folders to index)",
                "Project list (page size)  (TUI page size / CLI recent limit)",
                "Post-create actions  (git / reveal / editor / path / commands)",
                "ID counter",
                "Maintenance  (reindex / check and recover / data locations)",
                "Back",
            ],
            0,
        )?
        else {
            break;
        };

        let outcome = match choice.as_str() {
            "Project basics  (base dir / template / date / editor)" => menu_settings_basics(),
            "Workflow prompts  (open prompt / confirm / banner / preview)" => {
                menu_settings_workflow()
            }
            "Library bases  (extra folders to index)" => menu_settings_bases(),
            "Project list (page size)  (TUI page size / CLI recent limit)" => {
                menu_settings_recent()
            }
            "Post-create actions  (git / reveal / editor / path / commands)" => {
                menu_settings_postcreate()
            }
            "ID counter" => menu_id(),
            "Maintenance  (reindex / check and recover / data locations)" => menu_maintenance(),
            "Back" => break,
            other => anyhow::bail!("unhandled menu item '{other}'"),
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

/// Ask for a config value and write it. Esc leaves the setting alone, which is
/// what "Esc in a settings field → the settings submenu, value unchanged" means.
fn set_from_prompt(key: &str, prompt_text: &str, opts: TextOpts<'_>) -> Result<()> {
    match prompt::text(prompt_text, opts)? {
        Some(value) => config::set(key, &value),
        None => Ok(()),
    }
}

/// Reindex, recovery, and where fastf keeps its files — the three commands a TUI
/// user had to leave the menu for.
///
/// Every arm is the CLI command's own function, so the output is the output, not
/// a second rendering of it that can drift.
fn menu_maintenance() -> Result<()> {
    loop {
        let Some(choice) = menu(
            "Maintenance",
            &[
                "Reindex  (rescan every base)",
                "Check and recover  (finish or roll back interrupted work)",
                "Show data locations",
                "Back",
            ],
            0,
        )?
        else {
            break;
        };

        let outcome = match choice.as_str() {
            "Reindex  (rescan every base)" => crate::cli::reindex::run(),
            "Check and recover  (finish or roll back interrupted work)" => {
                crate::cli::reconcile::run()
            }
            "Show data locations" => crate::cli::paths_cmd::run(),
            _ => break,
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

fn menu_settings_basics() -> Result<()> {
    loop {
        let Some(choice) = menu(
            "Project basics",
            &[
                "Set base directory",
                "Set default template",
                "Set date format",
                "Set editor",
                "Set terminal",
                "Set register naming pattern",
                "Back",
            ],
            0,
        )?
        else {
            break;
        };

        let outcome = match choice.as_str() {
            "Set base directory" => {
                println!(
                    "  {}  Linux/macOS: /home/user/Projects  ·  Windows: C:\\Users\\user\\Projects or C:/Users/user/Projects",
                    "Hint:".yellow()
                );
                // Empty falls back to the HOME directory, not the cwd — that
                // the home directory, and this hint used to say otherwise.
                set_from_prompt(
                    "base-dir",
                    "Base directory (empty = your home directory)",
                    TextOpts::new().allow_empty().validate(|raw| {
                        if raw.trim().is_empty() {
                            return Ok(());
                        }
                        crate::core::config::expand_base_path(raw)
                            .map(|_| ())
                            .map_err(|e| format!("{e:#}"))
                    }),
                )
            }
            "Set default template" => set_from_prompt(
                "default-template",
                "Default template slug (empty = always prompt)",
                TextOpts::new().allow_empty(),
            ),
            "Set date format" => set_from_prompt(
                "date-format",
                "Date format (strftime, e.g. %Y-%m-%d)",
                TextOpts::new().default_value("%Y-%m-%d"),
            ),
            "Set editor" => set_from_prompt(
                "editor",
                "Editor command (e.g. nvim, code, nano)",
                TextOpts::new().allow_empty(),
            ),
            "Set terminal" => {
                println!(
                    "  {}  the emulator to open when fastf is launched without a terminal (a desktop launcher);",
                    "Hint:".yellow()
                );
                println!(
                    "        empty = $TERMINAL, else probe the known ones. \"none\" never opens a window."
                );
                set_from_prompt(
                    "terminal",
                    "Terminal command (e.g. konsole, kitty, none)",
                    TextOpts::new().allow_empty(),
                )
            }
            "Set register naming pattern" => {
                println!(
                    "  {}  used by `register --rename` when no template is attached; must contain {{id}}",
                    "Hint:".yellow()
                );
                let current = Config::load()?.register_naming_pattern.clone();
                set_from_prompt(
                    "register-naming-pattern",
                    "Register naming pattern",
                    TextOpts::new()
                        .default_value(current)
                        // The same rule `config set` enforces: without {id},
                        // registering two folders with the same {name} renames
                        // them both to the same target.
                        .validate(|raw| {
                            if raw.contains("{id}") {
                                Ok(())
                            } else {
                                Err("the pattern must contain {id}".to_string())
                            }
                        }),
                )
            }
            "Back" => break,
            other => anyhow::bail!("unhandled menu item '{other}'"),
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
        let cfg = Config::load()?;
        let items = [
            label_toggle(
                "\"Open project folder?\" prompt after create",
                cfg.prompt_open_after_create,
            ),
            label_toggle("\"Create this project?\" confirmation", cfg.confirm_create),
            label_toggle("ASCII banner in main menu", cfg.show_banner),
            label_toggle("Library summary in main menu", cfg.show_frame),
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
        let Some(choice) = prompt::select("Workflow prompts", &items, 0)? else {
            break;
        };

        let outcome = match choice {
            0 => toggle_setting("prompt-open-after-create", cfg.prompt_open_after_create),
            1 => toggle_setting("confirm-create", cfg.confirm_create),
            2 => toggle_setting("show-banner", cfg.show_banner),
            3 => toggle_setting("show-frame", cfg.show_frame),
            4 => set_from_prompt(
                "preview-lines",
                "Lines per file in dry-run (0 = none)",
                TextOpts::new()
                    .default_value(cfg.preview_lines.to_string())
                    .validate(|raw| {
                        raw.trim()
                            .parse::<usize>()
                            .map(|_| ())
                            .map_err(|_| format!("expected a number, got '{}'", raw.trim()))
                    }),
            ),
            // Not a bool on disk — it stores "suffix" or "error".
            5 => config::set(
                "on-name-collision",
                if cfg.suffix_on_name_collision() {
                    "error"
                } else {
                    "suffix"
                },
            ),
            _ => break,
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

/// Edit the list of extra base directories the project library indexes (beyond
/// `base_dir`). An absent/unmounted base is simply skipped at scan time.
///
/// **Prompt first, then lock, then reload** — the same rule
/// [`edit_postcreate_commands`] follows. This used to rebuild the whole `bases`
/// list from the copy loaded before the prompt, so a base added by the browser
/// UI or another `fastf config set` while the prompt sat open was silently
/// reverted. Removal matches on the base's text rather than the position it had
/// in the list the user saw, for the same reason.
fn menu_settings_bases() -> Result<()> {
    loop {
        let cfg = Config::load()?;
        println!();
        if cfg.bases.is_empty() {
            println!(
                "  {}",
                "No extra bases. base_dir is always indexed on its own.".dimmed()
            );
        } else {
            println!("  {}", "Extra indexed bases (besides base_dir):".bold());
            // Probed, so a base that is configured but unplugged or hanging says
            // so here rather than looking identical to a working one.
            let entries: Vec<std::path::PathBuf> =
                cfg.bases.iter().map(std::path::PathBuf::from).collect();
            for (path, probe) in
                crate::util::paths::probe_dirs(&entries, crate::util::paths::PROBE_TIMEOUT)
            {
                println!(
                    "    {} {}{}",
                    "•".cyan(),
                    path.display(),
                    probe.note().dimmed()
                );
            }
        }
        println!();

        let mut items = vec!["Add a base directory".to_string()];
        for b in &cfg.bases {
            items.push(format!("Remove  {b}"));
        }
        items.push("Back".to_string());

        let Some(choice) = prompt::select("Library bases", &items, 0)? else {
            break;
        };

        if choice == items.len() - 1 {
            break;
        }

        // What the user asked for, decided entirely outside the lock.
        let outcome = if choice == 0 {
            match prompt::text(
                "Base directory to add (absolute path)",
                TextOpts::new().allow_empty().validate(|raw| {
                    if raw.trim().is_empty() {
                        return Ok(());
                    }
                    config::normalize_base_entry(raw)
                        .map(|_| ())
                        .map_err(|e| format!("{e:#}"))
                }),
            )? {
                Some(val) => add_base(val.trim()),
                None => Ok(()),
            }
        } else {
            // The label carries the text, never the index: the list may have
            // changed while the menu was open, and removing by position would
            // then delete a base the user never pointed at.
            remove_base(&cfg.bases[choice - 1])
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

/// Append one extra base, validated at the prompt and committed against the
/// configuration as it is under the lock.
fn add_base(raw: &str) -> Result<()> {
    if raw.is_empty() {
        return Ok(());
    }
    let entry = config::normalize_base_entry(raw)?;
    let mut already = false;
    crate::core::operations::update_config(|fresh| {
        if fresh.bases.iter().any(|b| b == &entry) {
            already = true;
        } else {
            fresh.bases.push(entry.clone());
        }
        Ok(())
    })?;
    if already {
        println!("  {} {} is already indexed.", "·".dimmed(), entry);
    } else {
        println!("  {} added {}", "✓".green(), entry);
    }
    Ok(())
}

/// Drop one extra base by text, reporting rather than guessing when it has
/// already gone elsewhere.
fn remove_base(entry: &str) -> Result<()> {
    let target = entry.to_string();
    let mut removed = false;
    crate::core::operations::update_config(|fresh| {
        if let Some(position) = fresh.bases.iter().position(|b| b == &target) {
            fresh.bases.remove(position);
            removed = true;
        }
        Ok(())
    })?;
    if removed {
        println!("  {} removed {}", "✓".green(), entry);
    } else {
        println!(
            "  {} {} had already been removed elsewhere — left alone.",
            "note:".yellow(),
            entry
        );
    }
    Ok(())
}

fn menu_settings_recent() -> Result<()> {
    loop {
        let cfg = Config::load()?;
        let items = [
            format!("Projects page size  [{}]", cfg.recent_default_limit),
            "Back".to_string(),
        ];
        let Some(choice) = prompt::select("Project list (page size)", &items, 0)? else {
            break;
        };

        let outcome = match choice {
            0 => set_from_prompt(
                "recent-default-limit",
                "TUI page size and default --limit for `fastf recent`",
                TextOpts::new()
                    .default_value(cfg.recent_default_limit.to_string())
                    .validate(|raw| match raw.trim().parse::<usize>() {
                        Ok(n) if n >= 1 => Ok(()),
                        Ok(_) => Err("must be at least 1".to_string()),
                        Err(_) => Err(format!("expected a number, got '{}'", raw.trim())),
                    }),
            ),
            _ => break,
        };
        contain(outcome)?;
        println!();
    }
    Ok(())
}

fn menu_settings_postcreate() -> Result<()> {
    loop {
        let cfg = Config::load()?;
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
        let Some(choice) =
            prompt::select("Post-create actions (default for new projects)", &items, 0)?
        else {
            break;
        };

        let outcome = match choice {
            0 => toggle_setting("post_create.git_init", pc.git_init),
            1 => toggle_setting("post_create.reveal", pc.reveal),
            2 => toggle_setting("post_create.open_in_editor", pc.open_in_editor),
            3 => toggle_setting("post_create.print_path", pc.print_path),
            4 => edit_postcreate_commands(),
            _ => break,
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
/// unattended, and then write the whole struct back — reverting anything
/// another `fastf config set` had changed in the meantime, with no
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

    let Some(choice) = menu(
        "Manage commands",
        &["Add a command", "Remove commands", "Back"],
        0,
    )?
    else {
        return Ok(());
    };

    // What the user asked for, decided entirely outside the lock.
    enum Edit {
        Add(String),
        Remove(Vec<usize>),
        Nothing,
    }

    let edit = match choice.as_str() {
        "Add a command" => {
            println!(
                "  {}  Use {{path}} as a placeholder for the absolute project path.",
                "Hint:".yellow()
            );
            let cmd =
                prompt::text("Shell command", TextOpts::new().allow_empty())?.unwrap_or_default();
            if cmd.trim().is_empty() {
                Edit::Nothing
            } else {
                Edit::Add(cmd)
            }
        }
        "Remove commands" => {
            if cmds.is_empty() {
                println!("  {} nothing to remove.", "·".dimmed());
                return Ok(());
            }
            let labels: Vec<String> = cmds.to_vec();
            let checked = vec![false; labels.len()];
            match prompt::multi_select(
                "Select commands to remove (Space to toggle, Enter to confirm)",
                &labels,
                &checked,
            )? {
                Some(picks) => Edit::Remove(picks),
                None => Edit::Nothing,
            }
        }
        _ => Edit::Nothing,
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

/// The guided menu addresses templates by slug; the picker itself is shared with
/// `fastf new`.
fn prompt_template_slug(prompt: &str) -> Result<Option<String>> {
    Ok(pick_template(
        prompt,
        "run the command directly: `fastf template show <slug>`",
    )?
    .map(|tmpl| tmpl.slug))
}

#[cfg(test)]
mod tests {
    use super::is_fatal;

    /// `is_fatal` reads the process-global interrupt flag, and
    /// `util::interrupt`'s own tests raise it. Without taking the lock that
    /// lives beside that state, a test here can be running while the flag is up
    /// and every "recoverable" assertion flips. Pre-existing race, made visible
    /// by adding lib tests elsewhere; the rule is in `tests/CLAUDE.md`.
    fn no_interrupt<R>(body: impl FnOnce() -> R) -> R {
        let _guard = crate::util::interrupt::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        body()
    }

    /// A mistyped path fails with `canonicalize`'s `NotFound` wrapped in
    /// context. This is THE case containment exists for, and the obvious
    /// "propagate anything with an io::Error in the chain" rule would have got
    /// it exactly backwards.
    #[test]
    fn a_wrapped_io_error_is_recoverable() {
        no_interrupt(|| {
            let io = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory");
            let err = anyhow::Error::new(io).context("path does not exist or is not accessible");
            assert!(!is_fatal(&err));
        });
    }

    /// A validation failure from `config::set` is likewise a correction to make,
    /// not a reason to end the session.
    #[test]
    fn a_plain_message_is_recoverable() {
        no_interrupt(|| {
            let err = anyhow::anyhow!("recent_default_limit must be at least 1");
            assert!(!is_fatal(&err));
        });
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
