//! The template picker and the base picker, once each.
//!
//! There were two template pickers with different labels and different "no
//! templates" errors, and three base pickers of which one clamped its labels and
//! one marked the default. A picker is a picker: these are the two.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

use crate::core::template::{self, Template};
use crate::tui::rows::{base_row, clamp_label, terminal_columns};
use crate::util::tty;

/// Ask which template to use.
///
/// `how` is the hint `require_tty` prints when there is no terminal: it must
/// name the flag or setting that answers the same question without a prompt.
/// `Ok(None)` is a cancelled pick, never an error.
pub fn pick_template(prompt: &str, how: &str) -> Result<Option<Template>> {
    let templates = template::load_all()?;
    if templates.is_empty() {
        bail!("no templates found — run `fastf template new` to create one");
    }
    tty::require_tty("pick a template", how)?;

    let columns = terminal_columns();
    let labels: Vec<String> = templates
        .iter()
        .map(|t| {
            // The slug is what every other surface addresses a template by, so
            // it belongs in the label; the description is what tells them apart.
            let head = format!("{} ({})", t.name, t.slug);
            if t.description.is_empty() {
                head
            } else {
                format!("{head} — {}", t.description)
            }
        })
        .map(|label| clamp_label(&label, columns))
        .collect();

    let Some(idx) = crate::tui::prompt::select(prompt, &labels, 0)? else {
        return Ok(None);
    };

    Ok(Some(templates[idx].clone()))
}

/// Ask which base to use.
///
/// `default_base` is marked `(default)` when it appears in `bases`; pass `None`
/// where the notion does not apply. With `offer_cancel`, a trailing `[Cancel]`
/// row returns `Ok(None)`. Labels are clamped, because a base label carries a
/// full path and paths are long.
pub fn pick_base(
    prompt: &str,
    bases: &[PathBuf],
    default_base: Option<&Path>,
    how: &str,
    offer_cancel: bool,
) -> Result<Option<PathBuf>> {
    if bases.is_empty() {
        bail!("no configured bases are mounted");
    }
    tty::require_tty("pick a base", how)?;

    let columns = terminal_columns();
    let mut labels: Vec<String> = bases
        .iter()
        .map(|base| {
            let is_default = default_base.is_some_and(|d| d == base.as_path());
            clamp_label(&base_row(base, is_default), columns)
        })
        .collect();
    if offer_cancel {
        labels.push("[Cancel]".to_string());
    }

    let Some(idx) = crate::tui::prompt::select(prompt, &labels, 0)? else {
        return Ok(None);
    };

    if idx >= bases.len() {
        return Ok(None);
    }
    Ok(Some(bases[idx].clone()))
}
