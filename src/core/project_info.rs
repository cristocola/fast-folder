//! Per-project metadata file (`PROJECT_INFO.md`).
//!
//! Written into the root of each new project. The file has two layers:
//!
//! 1. **YAML frontmatter** — the source of truth. Structured, parseable
//!    metadata: id, template, created timestamp, folder, path, every template
//!    variable, and any tags.  This is what enables grep / Obsidian /
//!    `fastf search` to query projects after the fact.
//!
//! 2. **Human-readable body** — a markdown table of variables (so the file
//!    reads nicely in any editor) plus a `## Notes` section the user owns,
//!    and a `## Journal` section that grows with timestamped entries.
//!
//! Generation is best-effort: a write failure logs a warning but never fails
//! project creation.
//!
//! Read back two ways:
//!   - [`read`] returns the raw markdown (for `--plain` / fallback display).
//!   - [`read_metadata`] parses the frontmatter into a typed [`Metadata`]
//!     struct (returns `Ok(None)` if the file exists but has no frontmatter,
//!     e.g. older / hand-edited files).
//!
//! Mutation helpers:
//!   - [`write_frontmatter`] reads the file, applies a closure to the parsed
//!     [`Metadata`], re-serialises, and writes back atomically.
//!   - [`append_journal_entry`] appends a timestamped entry to `## Journal`,
//!     creating the section if it doesn't exist yet.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::core::project::ProjectPlan;
use crate::core::template::Template;

/// Canonical filename for the per-project metadata file.
///
/// As of v0.9 the filename is fixed (no config knob): this file IS the
/// project's identity in the filesystem-as-truth model, so it is mandatory and
/// always named `PROJECT_INFO.md`. Reserved across the codebase — templates
/// cannot declare a file entry with this name (case-insensitive), checked in
/// `Template::load_from_file`, `Template::save_to_file`, and the TUI builder.
pub const RESERVED_FILENAME: &str = "PROJECT_INFO.md";

/// Absolute path of a project's metadata file: `<dir>/PROJECT_INFO.md`.
pub fn pinfo_path(dir: &Path) -> std::path::PathBuf {
    dir.join(RESERVED_FILENAME)
}

/// True when `path` (the YAML `files[].path` field) collides with the reserved
/// auto-gen filename. Compared case-insensitively on the final path component
/// so `notes/PROJECT_INFO.md` is fine but `PROJECT_INFO.md` at the root is not.
pub fn path_is_reserved(path: &str) -> bool {
    // Templates always use `/` separators (see CLAUDE.md "Cross-platform paths"),
    // but accept `\` defensively in case a user-edited YAML used backslashes.
    let normalized = path.replace('\\', "/");
    let leaf = normalized.rsplit('/').next().unwrap_or(&normalized);
    // The reservation only kicks in at the project root — templates that want
    // a sub-folder file called PROJECT_INFO.md (rare, but valid) still work.
    leaf.eq_ignore_ascii_case(RESERVED_FILENAME) && !normalized.contains('/')
}

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
    /// Combined literal + auto-derived tags.  `#[serde(default)]` keeps files
    /// written before tagging was introduced valid — they simply get no tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// `true` while the project is still being built.
    ///
    /// Metadata is written *first* now, immediately after the folder is claimed,
    /// so an interrupted create leaves something visible instead of an orphan
    /// folder no fastf command could see. This flag distinguishes "still filling
    /// in" from "finished", and is cleared as the last step of a good create.
    ///
    /// Skipped when false, so a finished project's frontmatter is byte-identical
    /// to what earlier versions wrote — the round-trip tests rely on that.
    #[serde(default, skip_serializing_if = "is_false")]
    pub provisioning: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Metadata {
    /// Build the typed metadata for a freshly-planned project.
    /// `tags` is the combined literal + auto-derived tag list computed in
    /// `project::create()` before writing the file.
    pub fn from_plan(plan: &ProjectPlan, tmpl: &Template, tags: Vec<String>) -> Self {
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
            created: crate::core::library::now_iso8601(),
            folder: plan.folder_name.clone(),
            path: plan.root_path.display().to_string(),
            variables,
            tags,
            provisioning: false,
        }
    }
}

/// Build the full markdown body — frontmatter + variables table + Notes section.
pub fn render(plan: &ProjectPlan, tmpl: &Template, tags: &[String]) -> String {
    let meta = Metadata::from_plan(plan, tmpl, tags.to_vec());

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

/// Write `<root>/PROJECT_INFO.md`. Metadata is mandatory in v0.9 (the file is
/// the project's identity), so there is no "disabled" path.
pub fn write(plan: &ProjectPlan, tmpl: &Template, tags: &[String]) -> Result<()> {
    let path = pinfo_path(&plan.root_path);
    let body = render(plan, tmpl, tags);
    // Atomic: this file *is* the project's identity, so a half-written one would
    // make the project unreadable rather than merely stale.
    crate::util::atomic::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

/// Flag a project as still being built. Set immediately after the folder is
/// claimed so an interrupted create leaves a *visible, labelled* partial project
/// instead of an orphan folder that discovery cannot see.
pub fn mark_provisioning(project_root: &Path) -> Result<()> {
    write_frontmatter(&pinfo_path(project_root), |meta| meta.provisioning = true)
}

/// Clear the in-progress flag — the project is complete. Last step of a
/// successful create, after every file has landed.
pub fn clear_provisioning(project_root: &Path) -> Result<()> {
    write_frontmatter(&pinfo_path(project_root), |meta| meta.provisioning = false)
}

/// True when a project's metadata says it was never finished being built.
/// Cheap enough for a depth-1 sweep; unreadable metadata reports `false` so a
/// hand-edited file is never mistaken for a broken create.
pub fn is_provisioning(project_root: &Path) -> bool {
    read_metadata(project_root)
        .ok()
        .flatten()
        .is_some_and(|meta| meta.provisioning)
}

/// Read the raw markdown body for the project's metadata file.
/// Errors with a friendly message when missing.
pub fn read(project_root: &Path) -> Result<String> {
    let path = pinfo_path(project_root);
    if !path.exists() {
        anyhow::bail!("no {} found at {}", RESERVED_FILENAME, path.display());
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
pub fn read_metadata(project_root: &Path) -> Result<Option<Metadata>> {
    let body = read(project_root)?;
    let Some((frontmatter, _)) = split_frontmatter_body(&body) else {
        return Ok(None);
    };
    let meta: Metadata = serde_yaml::from_str(frontmatter)
        .with_context(|| format!("parsing YAML frontmatter in {}", RESERVED_FILENAME))?;
    Ok(Some(meta))
}

// ---------------------------------------------------------------------------
// Mutation helpers
// ---------------------------------------------------------------------------

/// Atomically rewrite the frontmatter of an existing `PROJECT_INFO.md`.
///
/// Reads the file, parses the YAML frontmatter, applies `mutator` to the
/// typed [`Metadata`], re-serialises, recombines with the original body bytes
/// unchanged, then writes via a `.tmp` + rename for atomicity.
///
/// Returns an error when:
/// - The file cannot be read or written.
/// - No YAML frontmatter block is present — the caller gets a named error.
/// - The frontmatter cannot be parsed or re-serialised.
pub fn write_frontmatter(path: &Path, mutator: impl FnOnce(&mut Metadata)) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let (frontmatter_yaml, body) = split_frontmatter_body(&content).ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no YAML frontmatter — cannot update metadata (was it created by fastf new?)",
            path.display()
        )
    })?;

    let mut meta: Metadata = serde_yaml::from_str(frontmatter_yaml)
        .with_context(|| format!("parsing YAML frontmatter in {}", path.display()))?;

    mutator(&mut meta);

    let new_yaml = serde_yaml::to_string(&meta).context("re-serialising metadata")?;

    let new_content = format!("---\n{}---\n{}", new_yaml, body);

    atomic_write(path, new_content.as_bytes())
}

/// Append a timestamped journal entry to `## Journal` in the file.
///
/// If the file has no `## Journal` section one is created before EOF.
/// Entries are appended in chronological order (oldest first).
/// The write is atomic: tmp-file + rename.
pub fn append_journal_entry(path: &Path, message: &str) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    // Require frontmatter — this is a structured project file.
    split_frontmatter_body(&content).ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no YAML frontmatter — cannot append journal entry",
            path.display()
        )
    })?;

    let timestamp = crate::core::library::now_iso8601();
    let entry_line = format!("- {} — {}\n", timestamp, message);

    let new_content = if content.contains("## Journal") {
        // Section exists — append at end of file (chronological order).
        if content.ends_with('\n') {
            format!("{}{}", content, entry_line)
        } else {
            format!("{}\n{}", content, entry_line)
        }
    } else {
        // No section yet — add it at end of file.
        if content.ends_with('\n') {
            format!("{}## Journal\n\n{}", content, entry_line)
        } else {
            format!("{}\n\n## Journal\n\n{}", content, entry_line)
        }
    };

    atomic_write(path, new_content.as_bytes())
}

/// Read back only the journal lines from the metadata file.
///
/// Parses entries of the form `- <timestamp> — <message>` from the
/// `## Journal` section of the body.  Returns an empty vec when there is no
/// journal section.
pub fn read_journal_entries(project_root: &Path) -> Result<Vec<JournalEntry>> {
    let body = read(project_root)?;
    Ok(parse_journal_entries(&body))
}

/// A single timestamped journal entry.
pub struct JournalEntry {
    pub timestamp: String,
    pub message: String,
}

fn parse_journal_entries(content: &str) -> Vec<JournalEntry> {
    // Only process lines after the `## Journal` header.
    let Some(journal_start) = content.find("## Journal") else {
        return vec![];
    };
    let journal_section = &content[journal_start..];

    // Find where the next `##` section starts (if any) and stop there.
    let section_body = if let Some(next_h2) = journal_section[2..].find("\n##") {
        &journal_section[..next_h2 + 2] // stop before next ##
    } else {
        journal_section
    };

    let mut entries = Vec::new();
    for line in section_body.lines() {
        let line = line.trim();
        if !line.starts_with("- ") {
            continue;
        }
        let rest = &line[2..];
        if let Some((ts, msg)) = rest.split_once(" — ") {
            entries.push(JournalEntry {
                timestamp: ts.to_string(),
                message: msg.to_string(),
            });
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Split a `PROJECT_INFO.md` body into its YAML frontmatter and the markdown
/// body that follows the closing `---` line.
///
/// Returns `None` when the content does not start with a valid `---` block.
///
/// The returned `frontmatter` slice includes the trailing `\n` of the last
/// YAML line (so `"---\n" + frontmatter + "---\n"` rebuilds the header).
/// The returned `body` slice starts immediately after the closing `---\n` line.
pub fn split_frontmatter_body(content: &str) -> Option<(&str, &str)> {
    // Strip optional UTF-8 BOM so hand-edited files from Notepad still parse.
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    // Must open with `---` at column 0.
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;

    // Locate the closing `---` line, accepting both LF and CRLF.
    let close_lf = rest.find("\n---\n").map(|i| (i, "\n---\n".len()));
    let close_crlf = rest.find("\n---\r\n").map(|i| (i, "\n---\r\n".len()));

    let (close_pos, close_len) = match (close_lf, close_crlf) {
        (Some((a, la)), Some((b, lb))) => {
            if a <= b {
                (a, la)
            } else {
                (b, lb)
            }
        }
        (Some((a, la)), None) => (a, la),
        (None, Some((b, lb))) => (b, lb),
        (None, None) => return None,
    };

    // +1 to include the trailing \n of the last YAML line.
    let frontmatter_yaml = &rest[..close_pos + 1];
    let body = &rest[close_pos + close_len..];

    Some((frontmatter_yaml, body))
}

/// Write `bytes` to `path` atomically via a `.tmp` sibling + rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("md.tmp");
    fs::write(&tmp, bytes).with_context(|| format!("writing tmp {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_simple_frontmatter() {
        let body = "---\nid: ID0001\ntemplate: foo\n---\n\n# Body\n";
        let (fm, body_part) = split_frontmatter_body(body).expect("should split");
        assert_eq!(fm, "id: ID0001\ntemplate: foo\n");
        assert_eq!(body_part, "\n# Body\n");
    }

    #[test]
    fn split_no_frontmatter_returns_none() {
        let body = "# Just a markdown file\n\nNo YAML here.\n";
        assert!(split_frontmatter_body(body).is_none());
    }

    #[test]
    fn split_handles_crlf() {
        let body = "---\r\nid: ID0002\r\n---\r\n\r\n# Body\r\n";
        let (fm, _body) = split_frontmatter_body(body).expect("should split");
        assert!(fm.contains("id: ID0002"));
    }

    #[test]
    fn split_unterminated_returns_none() {
        let body = "---\nid: ID0003\n# never closed\n";
        assert!(split_frontmatter_body(body).is_none());
    }

    #[test]
    fn split_body_byte_identical_after_roundtrip() {
        let original = "---\nid: ID0004\ntags: []\n---\n\n# Body\nSome notes here.\n";
        let (fm, body) = split_frontmatter_body(original).expect("split");
        let rebuilt = format!("---\n{}---\n{}", fm, body);
        assert_eq!(original, rebuilt);
    }

    #[test]
    fn extracts_simple_frontmatter() {
        // Legacy-style test — kept for regression
        let body = "---\nid: ID0001\ntemplate: foo\n---\n\n# Body\n";
        let (fm, _) = split_frontmatter_body(body).expect("frontmatter present");
        assert!(fm.contains("id: ID0001"));
        assert!(fm.contains("template: foo"));
    }

    #[test]
    fn parse_journal_entries_basic() {
        let content = "---\nid: x\n---\n\n## Notes\n\n## Journal\n\n- 2026-01-01T00:00:00Z — first entry\n- 2026-01-02T00:00:00Z — second entry\n";
        let entries = parse_journal_entries(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp, "2026-01-01T00:00:00Z");
        assert_eq!(entries[0].message, "first entry");
        assert_eq!(entries[1].message, "second entry");
    }

    #[test]
    fn parse_journal_entries_no_section() {
        let content = "---\nid: x\n---\n\n## Notes\n\nSome notes.\n";
        let entries = parse_journal_entries(content);
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_journal_entries_stops_at_next_section() {
        let content = "---\nid: x\n---\n\n## Journal\n\n- 2026-01-01T00:00:00Z — entry\n\n## Notes\n\nNot a journal entry.\n";
        let entries = parse_journal_entries(content);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn path_is_reserved_matches_root_project_info() {
        assert!(path_is_reserved("PROJECT_INFO.md"));
        assert!(path_is_reserved("project_info.md"));
        assert!(path_is_reserved("Project_Info.MD"));
    }

    #[test]
    fn path_is_reserved_allows_subfolder_collision() {
        // Templates that put PROJECT_INFO.md in a subfolder aren't fighting
        // for the root auto-gen slot — let those through.
        assert!(!path_is_reserved("docs/PROJECT_INFO.md"));
        assert!(!path_is_reserved("notes\\PROJECT_INFO.md"));
    }

    #[test]
    fn path_is_reserved_allows_other_filenames() {
        assert!(!path_is_reserved("NOTES.md"));
        assert!(!path_is_reserved("README.md"));
        assert!(!path_is_reserved("project-info.md")); // hyphen, not underscore
        assert!(!path_is_reserved(".fastf-info.md"));
    }
}
