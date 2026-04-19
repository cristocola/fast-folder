//! Per-project metadata file (`PROJECT_INFO.md`).
//!
//! Written into the root of each new project. The file has two layers:
//!
//! 1. **YAML frontmatter** — the source of truth. Structured, parseable
//!    metadata: id, template, created timestamp, folder, path, and every
//!    template variable (regardless of whether it appears in the folder name).
//!    This is what enables grep / Obsidian / a future `fastf search` to query
//!    projects after the fact.
//!
//! 2. **Human-readable body** — a markdown table of variables (so the file
//!    reads nicely in any editor) plus a `## Notes` section the user owns.
//!
//! Generation is best-effort: a write failure logs a warning but never fails
//! project creation.
//!
//! Read back two ways:
//!   - [`read`] returns the raw markdown (for `--plain` / fallback display).
//!   - [`read_metadata`] parses the frontmatter into a typed [`Metadata`]
//!     struct (returns `Ok(None)` if the file exists but has no frontmatter,
//!     e.g. older / hand-edited files).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::core::config::Config;
use crate::core::project::ProjectPlan;
use crate::core::template::Template;

/// Typed view of the YAML frontmatter — the structured / queryable layer.
///
/// `BTreeMap` (vs `HashMap`) keeps `variables` in deterministic alphabetical
/// order on serialize, so the file is diff-friendly across runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub id: String,
    pub template: String,
    pub template_name: String,
    pub created: String,
    pub folder: String,
    pub path: String,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

impl Metadata {
    /// Build the typed metadata for a freshly-planned project.
    pub fn from_plan(plan: &ProjectPlan, tmpl: &Template) -> Self {
        // Drop the synthetic "id" entry — it's already a top-level field.
        let variables: BTreeMap<String, String> = tmpl
            .variables
            .iter()
            .map(|v| {
                let value = plan.vars.get(&v.slug).cloned().unwrap_or_default();
                (v.slug.clone(), value)
            })
            .collect();

        Self {
            id: plan.id_str.clone(),
            template: tmpl.slug.clone(),
            template_name: tmpl.name.clone(),
            created: crate::core::index::now_iso8601(),
            folder: plan.folder_name.clone(),
            path: plan.root_path.display().to_string(),
            variables,
        }
    }
}

/// Build the full markdown body — frontmatter + variables table + Notes section.
pub fn render(plan: &ProjectPlan, tmpl: &Template) -> String {
    let meta = Metadata::from_plan(plan, tmpl);

    // Serialize frontmatter via serde_yaml so colons, quotes, multibyte values,
    // etc. all escape correctly. serde_yaml's output already ends with `\n`
    // and starts with no leading separator, so we wrap it in `---` lines.
    let yaml =
        serde_yaml::to_string(&meta).unwrap_or_else(|e| format!("# yaml-serialize-error: {e}\n"));

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n\n");

    out.push_str("# Project Info\n\n");

    if !tmpl.variables.is_empty() {
        // Variables table (labels from template, values from plan — post-transform).
        // Column widths sized to the longest label / value so it renders cleanly
        // in any monospace viewer.
        let label_w = tmpl
            .variables
            .iter()
            .map(|v| v.label.chars().count())
            .max()
            .unwrap_or(8)
            .max("Variable".len());
        let value_w = tmpl
            .variables
            .iter()
            .map(|v| {
                let raw = plan.vars.get(&v.slug).cloned().unwrap_or_default();
                let display = if raw.is_empty() {
                    "_(empty)_".to_string()
                } else {
                    raw
                };
                display.chars().count()
            })
            .max()
            .unwrap_or(5)
            .max("Value".len());

        out.push_str(&format!(
            "| {:<lw$} | {:<vw$} |\n",
            "Variable",
            "Value",
            lw = label_w,
            vw = value_w
        ));
        out.push_str(&format!(
            "|{:-<lw$}|{:-<vw$}|\n",
            "",
            "",
            lw = label_w + 2,
            vw = value_w + 2
        ));
        for var in &tmpl.variables {
            let raw = plan.vars.get(&var.slug).cloned().unwrap_or_default();
            let display = if raw.is_empty() {
                "_(empty)_".to_string()
            } else {
                raw
            };
            out.push_str(&format!(
                "| {:<lw$} | {:<vw$} |\n",
                var.label,
                display,
                lw = label_w,
                vw = value_w
            ));
        }
        out.push('\n');
    }

    out.push_str("## Notes\n\n");
    out
}

/// Write `<root>/<cfg.project_info_filename>`. No-op when disabled.
pub fn write(plan: &ProjectPlan, tmpl: &Template, cfg: &Config) -> Result<()> {
    if !cfg.project_info_enabled {
        return Ok(());
    }
    let path = plan.root_path.join(&cfg.project_info_filename);
    let body = render(plan, tmpl);
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Read the raw markdown body for the project's metadata file.
/// Errors with a friendly message when missing.
pub fn read(project_root: &Path, cfg: &Config) -> Result<String> {
    let path = project_root.join(&cfg.project_info_filename);
    if !path.exists() {
        anyhow::bail!(
            "no {} found at {} — this project predates the metadata feature",
            cfg.project_info_filename,
            path.display()
        );
    }
    fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
}

/// Parse the YAML frontmatter into a typed [`Metadata`].
///
/// - `Ok(Some(meta))` — frontmatter found and parsed cleanly.
/// - `Ok(None)` — file exists but has no `---` frontmatter block (older /
///   hand-edited file). Caller should fall back to displaying [`read`] output
///   verbatim.
/// - `Err(_)` — file missing, IO error, or malformed YAML.
pub fn read_metadata(project_root: &Path, cfg: &Config) -> Result<Option<Metadata>> {
    let body = read(project_root, cfg)?;
    let Some(frontmatter) = extract_frontmatter(&body) else {
        return Ok(None);
    };
    let meta: Metadata = serde_yaml::from_str(frontmatter)
        .with_context(|| format!("parsing YAML frontmatter in {}", cfg.project_info_filename))?;
    Ok(Some(meta))
}

/// Slice the YAML frontmatter out of a markdown string. Returns `None` when
/// the file does not start with a `---` line followed by a closing `---` line.
fn extract_frontmatter(body: &str) -> Option<&str> {
    // Strip optional UTF-8 BOM so a hand-edited file from Notepad still parses.
    let body = body.strip_prefix('\u{feff}').unwrap_or(body);

    // Must open with `---` at line 0.
    let rest = body
        .strip_prefix("---\n")
        .or_else(|| body.strip_prefix("---\r\n"))?;

    // Find the closing `---` line.
    // Accept both `\n---\n` and `\n---\r\n`, and also a trailing close at EOF.
    let close_lf = rest.find("\n---\n");
    let close_crlf = rest.find("\n---\r\n");
    let end = match (close_lf, close_crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }?;

    Some(&rest[..end + 1]) // include the trailing `\n` of the last YAML line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_frontmatter() {
        let body = "---\nid: ID0001\ntemplate: foo\n---\n\n# Body\n";
        let fm = extract_frontmatter(body).expect("frontmatter present");
        assert!(fm.contains("id: ID0001"));
        assert!(fm.contains("template: foo"));
    }

    #[test]
    fn no_frontmatter_returns_none() {
        let body = "# Just a markdown file\n\nNo YAML here.\n";
        assert!(extract_frontmatter(body).is_none());
    }

    #[test]
    fn handles_crlf_line_endings() {
        let body = "---\r\nid: ID0002\r\n---\r\n\r\n# Body\r\n";
        let fm = extract_frontmatter(body).expect("frontmatter present");
        assert!(fm.contains("id: ID0002"));
    }

    #[test]
    fn unterminated_frontmatter_returns_none() {
        let body = "---\nid: ID0003\n# never closed\n";
        assert!(extract_frontmatter(body).is_none());
    }
}
