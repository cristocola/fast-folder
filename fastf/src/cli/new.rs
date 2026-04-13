use anyhow::{bail, Result};
use dialoguer::{Confirm, Input, Select};
use std::collections::HashMap;

use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::project;
use crate::core::template::{self, Template, VarType};

/// Arguments passed to `fastf new`.
pub struct NewArgs {
    pub template_slug: Option<String>,
    pub vars: HashMap<String, String>,
    pub dry_run: bool,
    pub base_dir_override: Option<String>,
}

pub fn run(args: NewArgs) -> Result<()> {
    let mut config = Config::load()?;
    if let Some(ref dir) = args.base_dir_override {
        config.base_dir = dir.clone();
    }

    // Resolve template
    let tmpl = resolve_template(args.template_slug.as_deref(), &config)?;

    // Collect variable values (flags → interactive fallback)
    let raw_vars = collect_vars(&tmpl, &args.vars)?;

    // Load counters
    let mut counters = Counters::load()?;

    // Build plan
    let plan = project::plan(&tmpl, &raw_vars, &config, &counters)?;

    if args.dry_run {
        project::print_dry_run(&plan, &tmpl);
        return Ok(());
    }

    // Show preview and confirm
    project::print_dry_run(&plan, &tmpl);
    println!();
    let ok = Confirm::new()
        .with_prompt("Create this project?")
        .default(true)
        .interact()?;

    if !ok {
        println!("Aborted.");
        return Ok(());
    }

    project::create(&plan, &tmpl, &mut counters, &config)?;
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

/// Collect variable values, preferring CLI-provided values, falling back to prompts.
fn collect_vars(tmpl: &Template, cli_vars: &HashMap<String, String>) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();

    for var in &tmpl.variables {
        // If provided via CLI flag, use it directly
        if let Some(val) = cli_vars.get(&var.slug) {
            result.insert(var.slug.clone(), val.clone());
            continue;
        }

        // Otherwise prompt interactively
        let value = match var.var_type {
            VarType::Text => {
                if var.required {
                    loop {
                        let mut input = Input::<String>::new().with_prompt(&var.label);
                        if !var.default.is_empty() {
                            input = input.default(var.default.clone());
                        }
                        let v: String = input.interact_text()?;
                        if !v.is_empty() {
                            break v;
                        }
                        eprintln!("  '{}' is required — please enter a value", var.label);
                    }
                } else {
                    let mut input = Input::<String>::new()
                        .with_prompt(&var.label)
                        .allow_empty(true);
                    if !var.default.is_empty() {
                        input = input.default(var.default.clone());
                    }
                    input.interact_text()?
                }
            }
            VarType::Select => {
                if var.options.is_empty() {
                    bail!("variable '{}' is type 'select' but has no options", var.slug);
                }
                let default_idx = var.options
                    .iter()
                    .position(|o| o == &var.default)
                    .unwrap_or(0);
                let idx = Select::new()
                    .with_prompt(&var.label)
                    .items(&var.options)
                    .default(default_idx)
                    .interact()?;
                var.options[idx].clone()
            }
        };

        result.insert(var.slug.clone(), value);
    }

    Ok(result)
}
