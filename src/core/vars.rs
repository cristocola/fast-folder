use anyhow::{Result, bail};
use dialoguer::{Input, Select};
use std::collections::HashMap;
use std::io::IsTerminal;

use crate::core::template::{Template, VarType};

/// Resolve defaults and validate required/select values without applying name
/// transforms. CLI prompts, browser requests, previews, create, register, and
/// apply all use this boundary so an input cannot be accepted by one interface
/// and rejected by another.
pub fn validated_raw_values(
    tmpl: &Template,
    supplied: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    tmpl.validate()?;
    let mut resolved = HashMap::new();
    for variable in &tmpl.variables {
        let value = supplied
            .get(&variable.slug)
            .cloned()
            .unwrap_or_else(|| variable.default.clone());
        if variable.required && value.trim().is_empty() {
            bail!("variable '{}' is required", variable.label);
        }
        if variable.var_type == VarType::Select {
            if variable.options.is_empty() {
                bail!(
                    "variable '{}' is type 'select' but has no options",
                    variable.slug
                );
            }
            if !value.is_empty() && !variable.options.iter().any(|option| option == &value) {
                bail!(
                    "{} must be one of: {}",
                    variable.label,
                    variable.options.join(", ")
                );
            }
        }
        resolved.insert(variable.slug.clone(), value);
    }
    Ok(resolved)
}

/// Resolve defaults, validate, then apply the template transform and filesystem
/// sanitization used by rendered names and stored metadata.
pub fn rendered_values(
    tmpl: &Template,
    supplied: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let raw = validated_raw_values(tmpl, supplied)?;
    Ok(tmpl
        .variables
        .iter()
        .map(|variable| {
            let value = raw.get(&variable.slug).cloned().unwrap_or_default();
            let transformed = crate::core::naming::apply_transform(&value, &variable.transform);
            (
                variable.slug.clone(),
                crate::core::naming::sanitize_name(&transformed),
            )
        })
        .collect())
}

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

        // Without a terminal the prompt below fails with dialoguer's bare
        // "IO error: not a terminal", which tells a script author nothing about
        // what to do. Name the variable that is missing and the flag that
        // supplies it. (Optional variables need `--slug=` too — the prompt runs
        // for them as well.)
        if !std::io::stdout().is_terminal() {
            anyhow::bail!(
                "no terminal to prompt on, and '{}' was not supplied.\n  \
                 Pass it as a flag: --{}=<value>   (use --{}= for an empty value)\n  \
                 Every variable of template '{}' must be given this way in a script.",
                var.label,
                var.slug,
                var.slug,
                tmpl.slug
            );
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
                    bail!(
                        "variable '{}' is type 'select' but has no options",
                        var.slug
                    );
                }
                let default_idx = var
                    .options
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

    validated_raw_values(tmpl, &result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::template::{Transform, Variable};

    fn template_with_select() -> Template {
        Template {
            name: "Test".to_string(),
            slug: "test".to_string(),
            naming_pattern: "{kind}".to_string(),
            variables: vec![Variable {
                slug: "kind".to_string(),
                label: "Kind".to_string(),
                var_type: VarType::Select,
                required: true,
                default: "alpha".to_string(),
                options: vec!["alpha".to_string(), "beta".to_string()],
                transform: Transform::UpperUnderscore,
            }],
            ..Template::default()
        }
    }

    #[test]
    fn defaults_select_validation_and_transforms_are_shared() {
        let template = template_with_select();
        let raw = validated_raw_values(&template, &HashMap::new()).unwrap();
        assert_eq!(raw.get("kind").unwrap(), "alpha");
        let rendered = rendered_values(&template, &HashMap::new()).unwrap();
        assert_eq!(rendered.get("kind").unwrap(), "ALPHA");

        let invalid = HashMap::from([("kind".to_string(), "gamma".to_string())]);
        assert!(validated_raw_values(&template, &invalid).is_err());
    }
}
