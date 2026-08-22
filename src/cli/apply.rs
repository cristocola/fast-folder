use anyhow::{Result, bail};
use colored::Colorize;
use dialoguer::Confirm;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::cli::extra::Recognized;
use crate::core::config::Config;
use crate::core::project;
use crate::core::template;
use crate::core::template::FolderNode;
use crate::core::vars::collect_vars;
use crate::util::tty;

/// Returns true if any folder name in the structure contains a `{token}` placeholder.
/// Used to decide whether to prompt for variables during `fastf apply`.
fn structure_has_tokens(nodes: &[FolderNode]) -> bool {
    nodes
        .iter()
        .any(|n| n.name.contains('{') || structure_has_tokens(&n.children))
}

/// Collect variable values, but only if this template actually interpolates
/// anything — a template of plain folders needs no answers.
///
/// `pub` so the TUI can collect **once** and hand the same values to a dry run
/// and the real run. It used to call `apply::run` twice with an empty map, which
/// meant answering every prompt a second time to confirm what you had just
/// previewed.
pub fn collect_if_needed(
    tmpl: &crate::core::template::Template,
    provided: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let needs_vars =
        tmpl.files.iter().any(|f| !f.template.is_empty()) || structure_has_tokens(&tmpl.structure);
    if needs_vars {
        collect_vars(tmpl, provided)
    } else {
        Ok(HashMap::new())
    }
}

pub struct ApplyArgs {
    pub template_slug: String,
    pub target: String,
    pub dry_run: bool,
    pub vars: HashMap<String, String>,
    pub yes: bool,
}

/// Apply the flags recovered from clap's trailing bucket. See
/// [`crate::cli::new::apply_extra`] for why the fallback arm exists.
pub fn apply_extra(args: &mut ApplyArgs, recognized: Vec<Recognized>) -> Result<()> {
    for flag in recognized {
        match flag.name.as_str() {
            "yes" => args.yes = true,
            "dry-run" => args.dry_run = true,
            other => bail!("flag `--{other}` is declared but not handled after the target"),
        }
    }
    Ok(())
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

    let raw_vars = collect_if_needed(&tmpl, &args.vars)?;

    let actions = project::apply_plan(&tmpl, &target, &raw_vars, &config.date_format)?;

    if args.dry_run {
        project::print_apply_plan(&actions, project::PreviewKind::DryRun);
        return Ok(());
    }

    project::print_apply_plan(&actions, project::PreviewKind::BeforeCommit);

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
        tty::require_tty("confirm", "pass --yes to apply without confirming")?;
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
    crate::core::operations::apply(&tmpl.slug, &target, &raw_vars)?;
    println!("\n{}  {}", "✓".green().bold(), "Template applied".bold());
    Ok(())
}
