use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::{Confirm, Select};
use std::collections::HashMap;

use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::project;
use crate::core::template::{self, Template};
use crate::core::vars::collect_vars;

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
        config.base_dir = dir.clone();
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

    // Load counters
    let mut counters = Counters::load()?;

    // Build plan
    let plan = project::plan(&tmpl, &raw_vars, &config, &counters)?;

    if args.dry_run {
        project::print_dry_run(&plan, &tmpl, &config);
        return Ok(());
    }

    // Show preview and confirm (unless --yes)
    project::print_dry_run(&plan, &tmpl, &config);
    if !args.yes {
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

    project::create(&plan, &tmpl, &mut counters, &config, !args.no_post)?;
    project::print_success(&plan, &tmpl);

    Ok(())
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

