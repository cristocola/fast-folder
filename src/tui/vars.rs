//! The interactive half of variable collection.
//!
//! It lived in `core::vars`, which meant `core` imported `dialoguer` and printed
//! to the terminal. The noninteractive boundary (`core::vars::validated_raw_values`)
//! stayed where it was: it is what create, apply, register, and the browser UI
//! all validate against.

use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::core::template::{Template, VarType};
use crate::core::vars::validated_raw_values;
use crate::tui::prompt::{self, TextOpts};

/// Collect variable values for a template, preferring CLI-provided values
/// and falling back to interactive prompts for anything missing.
/// Shared between `fastf new` and `fastf apply`.
pub fn collect_vars(
    tmpl: &Template,
    cli_vars: &HashMap<String, String>,
) -> Result<Option<HashMap<String, String>>> {
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
        if !crate::util::tty::prompt_available() {
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
                // The template's default keeps `dialoguer`'s `[default]`
                // contract: shown in the prompt, taken by a bare Enter, replaced
                // by whatever is typed instead.
                let mut opts = TextOpts::new();
                if !var.default.is_empty() {
                    opts = opts.default_value(var.default.clone());
                }
                if !var.required {
                    opts = opts.allow_empty();
                }
                match prompt::text(&var.label, opts)? {
                    Some(value) => value,
                    None => return Ok(None),
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
                match prompt::select(&var.label, &var.options, default_idx)? {
                    Some(idx) => var.options[idx].clone(),
                    None => return Ok(None),
                }
            }
        };

        result.insert(var.slug.clone(), value);
    }

    validated_raw_values(tmpl, &result).map(Some)
}
