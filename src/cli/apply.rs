use anyhow::{Result, bail};
use colored::Colorize;
use dialoguer::Confirm;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::config::Config;
use crate::core::project;
use crate::core::template;
use crate::core::vars::collect_vars;

pub struct ApplyArgs {
    pub template_slug: String,
    pub target: String,
    pub dry_run: bool,
    pub vars: HashMap<String, String>,
    pub yes: bool,
}

pub fn run(args: ApplyArgs) -> Result<()> {
    let config = Config::load()?;
    let tmpl = template::find_by_slug(&args.template_slug)?;

    let target = PathBuf::from(&args.target);
    if !target.exists() {
        bail!("target folder does not exist: {}", target.display());
    }
    if !target.is_dir() {
        bail!("target is not a directory: {}", target.display());
    }

    // Warn on unknown --vars
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

    // Only collect variables if at least one templated file needs them.
    let needs_vars = tmpl.files.iter().any(|f| !f.template.is_empty());
    let raw_vars = if needs_vars {
        collect_vars(&tmpl, &args.vars)?
    } else {
        HashMap::new()
    };

    let actions = project::apply_plan(&tmpl, &target);

    if args.dry_run {
        project::print_apply_plan(&actions);
        return Ok(());
    }

    project::print_apply_plan(&actions);

    // Short-circuit if nothing to do
    let will_create = actions.iter().any(|a| {
        matches!(
            a,
            project::ApplyAction::CreateFolder(_) | project::ApplyAction::CreateFile(_)
        )
    });
    if !will_create {
        println!(
            "\n{}",
            "Nothing to apply — every folder and file already exists.".dimmed()
        );
        return Ok(());
    }

    if !args.yes {
        println!();
        let ok = Confirm::new()
            .with_prompt(format!(
                "Apply template '{}' to {}?",
                tmpl.slug,
                target.display()
            ))
            .default(true)
            .interact()?;
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    println!();
    project::apply(&tmpl, &target, &raw_vars, &config)?;
    println!("\n{}  {}", "✓".green().bold(), "Template applied".bold());
    Ok(())
}
