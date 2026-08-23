use anyhow::{Result, bail};
use colored::Colorize;
use std::collections::HashMap;

use crate::cli::extra::Recognized;
use crate::cli::render;
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::project;
use crate::core::template::{self, Template};
use crate::tui::vars::collect_vars;
use crate::util::tty;

/// Arguments passed to `fastf new`.
pub struct NewArgs {
    pub template_slug: Option<String>,
    pub vars: HashMap<String, String>,
    pub dry_run: bool,
    pub base_dir_override: Option<String>,
    pub no_preview: bool,
    pub no_post: bool,
    pub yes: bool,
}

pub fn run(args: NewArgs) -> Result<()> {
    let mut config = Config::load()?;
    if let Some(ref dir) = args.base_dir_override {
        config.base_dir = crate::util::paths::storable(
            &crate::core::config::resolve_base_dir_input(dir)?,
            "the base directory",
        )?;
    }
    if args.no_preview {
        config.preview_lines = 0;
    }

    // Resolve template
    let tmpl = resolve_template(args.template_slug.as_deref(), &config)?;

    // Warn about CLI var keys that don't match any template variable
    let known_slugs: std::collections::HashSet<&str> =
        tmpl.variables.iter().map(|v| v.slug.as_str()).collect();
    for key in args.vars.keys() {
        if !known_slugs.contains(key.as_str()) {
            eprintln!(
                "{} unknown variable '--{}' — not defined in template '{}'",
                "warning:".yellow().bold(),
                key,
                tmpl.slug
            );
        }
    }

    // Collect variable values (flags → interactive fallback). Esc at any
    // variable cancels the whole create: no folder, no ID.
    let Some(raw_vars) = collect_vars(&tmpl, &args.vars)? else {
        crate::tui::prompt::report_cancelled("nothing was created");
        return Ok(());
    };

    // Preview plan — read-only, so no lock is taken. The ID shown here is
    // advisory: the committed one is allocated under the lock below, and only
    // differs if another fastf creates a project while the prompt is open.
    let counters = Counters::load()?;
    let plan = project::plan(&tmpl, &raw_vars, &config, &counters)?;

    if args.dry_run {
        render::print_dry_run(&plan, &tmpl, &config, render::PreviewKind::DryRun);
        return Ok(());
    }

    // Show preview and confirm (unless --yes or confirm_create disabled globally)
    render::print_dry_run(&plan, &tmpl, &config, render::PreviewKind::BeforeCommit);
    if !args.yes && config.confirm_create {
        tty::require_tty(
            "confirm",
            "pass --yes to create without confirming\n  \
             (or set `fastf config set confirm-create false` to stop asking)",
        )?;
        println!();
        // Esc is a No here: the question is whether to create, and cancelling
        // it means not creating.
        let ok = crate::tui::prompt::confirm("Create this project?", true)?.unwrap_or(false);

        if !ok {
            crate::tui::prompt::report_cancelled("nothing was created");
            return Ok(());
        }
    }

    // Allocate the ID and claim the folder under the cross-process data lock.
    // The counter is re-read and the plan recomputed inside the lock: another
    // fastf may have taken an ID while the confirmation prompt was open, and
    // reusing the previewed value is exactly how duplicate IDs were minted.
    // Post-create runs after the lock is released — see `run_post_create`.
    let mut created = crate::core::operations::create(crate::core::operations::CreateOptions {
        template_slug: tmpl.slug.clone(),
        variables: raw_vars,
        base_dir_override: args.base_dir_override.clone(),
        defer_over: None,
    })?;
    drop(created.take_mutation_lock());
    let plan = created.plan;
    let tmpl = created.template;
    let config = created.config;
    render::print_success(&plan, &tmpl);

    if !args.no_post {
        let root = plan
            .root_path
            .canonicalize()
            .unwrap_or_else(|_| plan.root_path.clone());
        let notes = project::run_post_create(&root, &tmpl, &config);
        crate::cli::render::print_post_create_notes(&notes);
    }

    // "Open project folder?" prompt — skip in non-interactive / headless modes
    // and when `reveal` would already run as a post-create action (avoid double-open).
    if should_prompt_open(&args, &tmpl, &config) {
        let abs_path = plan
            .root_path
            .canonicalize()
            .unwrap_or_else(|_| plan.root_path.clone());
        println!();
        if let Err(e) = prompt_and_reveal(&abs_path) {
            eprintln!(
                "{} could not open folder: {}",
                "warning:".yellow().bold(),
                e
            );
        }
    }

    Ok(())
}

fn should_prompt_open(args: &NewArgs, tmpl: &Template, config: &Config) -> bool {
    if args.yes || args.no_post {
        return false;
    }
    if !config.prompt_open_after_create {
        return false;
    }
    if !tty::prompt_available() {
        return false;
    }
    // If reveal will already run as a post-create action, don't double-open.
    let resolved = project::resolve_post_create(tmpl, config);
    if resolved.reveal {
        return false;
    }
    true
}

fn resolve_template(slug: Option<&str>, config: &Config) -> Result<Template> {
    // If slug provided directly, use it
    if let Some(s) = slug {
        return template::find_by_slug(s);
    }

    // If default_template is configured, use it
    if !config.default_template.is_empty() {
        return template::find_by_slug(&config.default_template);
    }

    // Otherwise prompt
    pick_template_interactively()
}

pub fn pick_template_interactively() -> Result<Template> {
    let picked = crate::tui::pickers::pick_template(
        "Select template",
        "name it instead: `fastf new <slug>`\n  \
         (or set one with `fastf config set default-template <slug>`)",
    )?;
    match picked {
        Some(tmpl) => Ok(tmpl),
        None => bail!("no template chosen"),
    }
}

// ---------------------------------------------------------------------------
// Flags lifted out of clap's trailing bucket
// ---------------------------------------------------------------------------

/// Apply the flags [`crate::cli::extra::classify_extra`] recovered for `new`.
///
/// The `_ =>` arm is the guard: a flag declared in clap but not handled here is
/// a build-time-visible bug rather than a flag that silently stops working when
/// typed after the slug. `main.rs`'s exhaustiveness test calls this with every
/// long `new` declares.
pub fn apply_extra(args: &mut NewArgs, recognized: Vec<Recognized>) -> Result<()> {
    for flag in recognized {
        match flag.name.as_str() {
            "yes" => args.yes = true,
            "dry-run" => args.dry_run = true,
            "no-preview" => args.no_preview = true,
            "no-post" => args.no_post = true,
            "base-dir" => args.base_dir_override = flag.value,
            other => bail!("flag `--{other}` is declared but not handled after the slug"),
        }
    }
    Ok(())
}

/// Ask "Open project folder? [Y/n]" and reveal on Yes.
///
/// The caller has already filtered out the cases where the prompt should not
/// fire (`--yes`, `--no-post`, `prompt_open_after_create=false`, a resolved
/// `post_create` that already reveals, no terminal). This owns the prompt and
/// the reveal call, and it lives here rather than in `core::post_create`
/// because `core` may not prompt: the same module runs for `fastf ui`, where
/// there is nobody at a terminal to answer.
fn prompt_and_reveal(path: &std::path::Path) -> Result<()> {
    let open = crate::tui::prompt::confirm("Open project folder?", true)?.unwrap_or(false);
    if open {
        crate::core::post_create::reveal_folder(path)?;
    }
    Ok(())
}
