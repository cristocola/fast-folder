use anyhow::{Result, bail};
use colored::Colorize;
use dialoguer::{Confirm, Select};
use std::collections::HashMap;

use crate::cli::extra::Recognized;
use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::project;
use crate::core::template::{self, Template};
use crate::core::vars::collect_vars;
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
        config.base_dir = crate::core::config::resolve_base_dir_input(dir)?
            .display()
            .to_string();
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

    // Collect variable values (flags → interactive fallback)
    let raw_vars = collect_vars(&tmpl, &args.vars)?;

    // Preview plan — read-only, so no lock is taken. The ID shown here is
    // advisory: the committed one is allocated under the lock below, and only
    // differs if another fastf creates a project while the prompt is open.
    let counters = Counters::load()?;
    let plan = project::plan(&tmpl, &raw_vars, &config, &counters)?;

    if args.dry_run {
        project::print_dry_run(&plan, &tmpl, &config, project::PreviewKind::DryRun);
        return Ok(());
    }

    // Show preview and confirm (unless --yes or confirm_create disabled globally)
    project::print_dry_run(&plan, &tmpl, &config, project::PreviewKind::BeforeCommit);
    if !args.yes && config.confirm_create {
        tty::require_tty(
            "confirm",
            "pass --yes to create without confirming\n  \
             (or set `fastf config set confirm-create false` to stop asking)",
        )?;
        println!();
        let ok = Confirm::new()
            .with_prompt("Create this project?")
            .default(true)
            .interact()?;

        if !ok {
            println!("Aborted.");
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
    project::print_success(&plan, &tmpl);

    if !args.no_post {
        let root = plan
            .root_path
            .canonicalize()
            .unwrap_or_else(|_| plan.root_path.clone());
        project::run_post_create(&root, &tmpl, &config);
    }

    // "Open project folder?" prompt — skip in non-interactive / headless modes
    // and when `reveal` would already run as a post-create action (avoid double-open).
    if should_prompt_open(&args, &tmpl, &config) {
        let abs_path = plan
            .root_path
            .canonicalize()
            .unwrap_or_else(|_| plan.root_path.clone());
        println!();
        if let Err(e) = crate::core::post_create::prompt_and_reveal(&abs_path) {
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
    let templates = template::load_all()?;
    if templates.is_empty() {
        bail!("no templates found — run `fastf template new` to create one");
    }
    tty::require_tty(
        "pick a template",
        "name it instead: `fastf new <slug>`\n  \
         (or set one with `fastf config set default-template <slug>`)",
    )?;

    let labels: Vec<String> = templates
        .iter()
        .map(|t| {
            if t.description.is_empty() {
                t.name.clone()
            } else {
                format!("{} — {}", t.name, t.description)
            }
        })
        .collect();

    let idx = Select::new()
        .with_prompt("Select template")
        .items(&labels)
        .default(0)
        .interact()?;

    Ok(templates[idx].clone())
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
