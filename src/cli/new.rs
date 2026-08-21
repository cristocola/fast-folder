use anyhow::{Result, bail};
use colored::Colorize;
use dialoguer::{Confirm, Select};
use std::collections::HashMap;
use std::io::IsTerminal;

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
    if !args.yes && config.confirm_create && !std::io::stdout().is_terminal() {
        bail!(
            "no terminal to confirm on — pass --yes to create without confirming\n  \
             (or set `fastf config set confirm-create false` to stop asking)"
        );
    }
    if !args.yes && config.confirm_create {
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
    if !std::io::stdout().is_terminal() {
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
// `extra` classifier — shared by main.rs's New / Apply / Register arms.
//
// clap's `trailing_var_arg = true` captures every token after the positional
// slug into a `Vec<String>`. Without further parsing, bare flags like `--yes`
// placed after the slug get silently dropped. This helper lifts recognized
// fastf flags out into `ExtraFlags`, splits `--key=value` pairs into the
// `vars` map (with hyphens normalised to underscores), and surfaces unknown
// `--foo` tokens so the caller can warn.
// ---------------------------------------------------------------------------

/// fastf-level flags that may appear inside clap's `trailing_var_arg` bucket.
/// Each `Commands` arm picks the fields it cares about — Apply/Register
/// ignore the ones that don't apply to them (no breaking change vs. the
/// pre-fix silent drop).
#[derive(Default, Debug, PartialEq)]
pub struct ExtraFlags {
    pub yes: bool,
    pub dry_run: bool,
    pub no_preview: bool,
    pub no_post: bool,
    pub base_dir: Option<String>,
}

/// Result of lifting recognized flags out of clap's `extra` bucket.
#[derive(Default, Debug)]
pub struct ClassifiedExtra {
    pub flags: ExtraFlags,
    /// `--key=value` pairs that did not match any recognized flag — these
    /// flow into the template variable map. Keys have hyphens normalised
    /// to underscores to match the on-disk template slug shape.
    pub vars: HashMap<String, String>,
    /// `--something` tokens without `=` that aren't a recognized fastf flag.
    /// Callers should warn (they used to be silently dropped).
    pub unknown: Vec<String>,
}

/// Walk clap's trailing `extra` once and classify each token.
///
/// Recognized boolean flags: `--yes` / `-y`, `--dry-run`, `--no-preview`,
/// `--no-post`. Recognized value flag (must use `=` form): `--base-dir=PATH`.
/// Everything else shaped like `--key=value` becomes a variable; everything
/// else shaped like `--foo` lands in `unknown`.
pub fn classify_extra(extra: Vec<String>) -> ClassifiedExtra {
    let mut out = ClassifiedExtra::default();
    for arg in extra {
        // Recognized boolean flags (exact match).
        match arg.as_str() {
            "--yes" | "-y" => {
                out.flags.yes = true;
                continue;
            }
            "--dry-run" => {
                out.flags.dry_run = true;
                continue;
            }
            "--no-preview" => {
                out.flags.no_preview = true;
                continue;
            }
            "--no-post" => {
                out.flags.no_post = true;
                continue;
            }
            _ => {}
        }

        // --key=value forms.
        if let Some(stripped) = arg.strip_prefix("--") {
            if let Some((key, val)) = stripped.split_once('=') {
                if key == "base-dir" {
                    out.flags.base_dir = Some(val.to_string());
                } else {
                    let key = key.replace('-', "_");
                    out.vars.insert(key, val.to_string());
                }
                continue;
            }
            // Bare `--foo` we don't recognize — surface it.
            out.unknown.push(arg);
            continue;
        }

        // Anything else (positional residue, single-dash unrecognized) —
        // also unknown.  Keeps the user from wondering where it went.
        out.unknown.push(arg);
    }
    out
}

#[cfg(test)]
mod extra_tests {
    use super::*;

    #[test]
    fn classify_extra_recognizes_yes_after_slug() {
        let c = classify_extra(vec!["--yes".to_string()]);
        assert!(c.flags.yes);
        assert!(c.vars.is_empty());
        assert!(c.unknown.is_empty());
    }

    #[test]
    fn classify_extra_dash_y_short() {
        let c = classify_extra(vec!["-y".to_string()]);
        assert!(c.flags.yes);
    }

    #[test]
    fn classify_extra_keeps_vars() {
        let c = classify_extra(vec![
            "--artist=Bad Bunny".to_string(),
            "--title=Lullaby".to_string(),
        ]);
        assert_eq!(c.vars.get("artist"), Some(&"Bad Bunny".to_string()));
        assert_eq!(c.vars.get("title"), Some(&"Lullaby".to_string()));
        assert_eq!(c.flags, ExtraFlags::default());
    }

    #[test]
    fn classify_extra_mixed() {
        let c = classify_extra(vec![
            "--yes".to_string(),
            "--artist=foo".to_string(),
            "--no-preview".to_string(),
        ]);
        assert!(c.flags.yes);
        assert!(c.flags.no_preview);
        assert!(!c.flags.dry_run);
        assert_eq!(c.vars.get("artist"), Some(&"foo".to_string()));
    }

    #[test]
    fn classify_extra_base_dir_with_equals() {
        let c = classify_extra(vec!["--base-dir=/tmp/x".to_string()]);
        assert_eq!(c.flags.base_dir.as_deref(), Some("/tmp/x"));
        assert!(c.vars.is_empty());
    }

    #[test]
    fn classify_extra_unknown_flag_isolated() {
        let c = classify_extra(vec!["--bogus".to_string()]);
        assert_eq!(c.unknown, vec!["--bogus".to_string()]);
        assert!(c.vars.is_empty());
        assert_eq!(c.flags, ExtraFlags::default());
    }

    #[test]
    fn classify_extra_hyphen_in_var_key_normalised() {
        let c = classify_extra(vec!["--client-name=Acme".to_string()]);
        assert_eq!(c.vars.get("client_name"), Some(&"Acme".to_string()));
    }

    #[test]
    fn classify_extra_dry_run_after_slug() {
        let c = classify_extra(vec!["--dry-run".to_string()]);
        assert!(c.flags.dry_run);
    }
}
