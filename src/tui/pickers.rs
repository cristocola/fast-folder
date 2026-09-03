//! The template picker, the base picker and the project picker, once each.
//!
//! There were two template pickers with different labels and different "no
//! templates" errors, and three base pickers of which one clamped its labels and
//! one marked the default. A picker is a picker: these are the three.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

use crate::core::library::Project;
use crate::core::template::{self, Template};
use crate::tui::rows::{RowWidths, base_row, clamp_label, project_row, terminal_columns};
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

    // Re-loaded rather than returned from the list: `load_all` skips the text
    // buffer, and the caller goes on to preview the template's files with it.
    Ok(Some(template::find_by_slug(&templates[idx].slug)?))
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

/// Ask which of several projects was meant.
///
/// This is the ambiguity picker: `fastf copy lullaby` matching three projects
/// shows them and copies the one chosen. It is deliberately **not** the project
/// browser — the browser's Enter opens the full action menu, and a picker that
/// interrupted a verb must serve that verb and nothing else. `fastf` and
/// `fastf recent` are how you reach the action menu.
///
/// `how` is the hint `require_tty` prints when there is no terminal, and must
/// name the way to answer the same question without being asked. `Ok(None)` is
/// a cancelled pick, never an error.
///
/// Not `live_select`: the candidate list is static — already narrowed by the
/// query, with no sizes landing later — and `live_select` carries three
/// load-bearing caller obligations this list has no use for.
pub fn pick_project(prompt: &str, candidates: &[Project], how: &str) -> Result<Option<Project>> {
    if candidates.is_empty() {
        bail!("no projects to choose from");
    }
    tty::require_tty("pick a project", how)?;

    let columns = terminal_columns();
    let widths = RowWidths::measure(candidates);
    let labels: Vec<String> = candidates
        .iter()
        .map(|p| clamp_label(&project_row(p, &widths, None, true), columns))
        .collect();

    // One picker, one look: the selected row is highlighted whole by the same
    // list widget the app draws, so the project picker no longer needs a theme
    // of its own to say "this row".
    let Some(idx) = crate::tui::prompt::select(prompt, &labels, 0)? else {
        return Ok(None);
    };
    Ok(Some(candidates[idx].clone()))
}
