use anyhow::{bail, Result};
use dialoguer::{Input, Select};
use std::collections::HashMap;

use crate::core::template::{Template, VarType};

/// Collect variable values for a template, preferring CLI-provided values
/// and falling back to interactive prompts for anything missing.
/// Shared between `fastf new` and `fastf apply`.
pub fn collect_vars(
    tmpl: &Template,
    cli_vars: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();

    for var in &tmpl.variables {
        if let Some(val) = cli_vars.get(&var.slug) {
            result.insert(var.slug.clone(), val.clone());
            continue;
        }

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
