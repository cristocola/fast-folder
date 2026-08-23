use anyhow::{Result, bail};
use std::collections::HashMap;

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
            let transformed = crate::core::template::apply_transform(&value, &variable.transform);
            (
                variable.slug.clone(),
                crate::core::naming::sanitize_name(&transformed),
            )
        })
        .collect())
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
