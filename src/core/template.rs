use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::validated::{SafeRelativePath, TemplateSlug};
use crate::util::paths;

// ---------------------------------------------------------------------------
// Template structs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Template {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,

    /// Pattern for the project folder name.
    /// Tokens: {date} {YYYY} {MM} {DD} {id} + any variable slug.
    pub naming_pattern: String,

    #[serde(default)]
    pub id: IdConfig,

    #[serde(default)]
    pub variables: Vec<Variable>,

    #[serde(default)]
    pub structure: Vec<FolderNode>,

    /// Glob patterns (relative to `files/`) whose files are copied **verbatim**
    /// even when they look like text — use this to preserve literal `{braces}`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verbatim: Vec<String>,

    /// Glob patterns (relative to `files/`) that are **never** copied
    /// (e.g. `.DS_Store`, `*.tmp`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,

    /// In-memory snapshot of the template's UTF-8 text files, loaded from the
    /// `files/` subtree. **Not** serialized into `template.yaml` — the `files/`
    /// directory on disk is the single source of truth. This buffer exists only
    /// so the editors/previews can show and round-trip text files; the copy
    /// engine (`core::assets`) always walks the real directory.
    #[serde(skip)]
    pub files: Vec<FileEntry>,

    /// The template's own directory (`templates/<slug>/`). Set at load time so
    /// callers can find the `files/` subtree. Not serialized.
    #[serde(skip)]
    pub dir: PathBuf,

    /// Optional per-template post-create actions (override the global config).
    /// `None` = fall back to `config.toml`'s `post_create` block.
    #[serde(default)]
    pub post_create: Option<crate::core::post_create::PostCreate>,

    /// Literal tags every project from this template receives automatically.
    /// Free-form strings (e.g. `"creative"`, `"music-video"`).
    #[serde(default)]
    pub tags: Vec<String>,

    /// Variable slugs whose resolved values should become hierarchical tags of
    /// the form `slug/value` (e.g. `tag_from: [client_type]` → `client_type/Indie`).
    /// Values that are empty after variable collection are skipped.
    #[serde(default)]
    pub tag_from: Vec<String>,
}

fn default_version() -> String {
    "1".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IdConfig {
    #[serde(default = "default_id_prefix")]
    pub prefix: String,
    #[serde(default = "default_id_digits")]
    pub digits: usize,
}

fn default_id_prefix() -> String {
    "ID".to_string()
}
fn default_id_digits() -> usize {
    4
}

impl Default for IdConfig {
    fn default() -> Self {
        Self {
            prefix: default_id_prefix(),
            digits: default_id_digits(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Variable {
    pub slug: String,
    pub label: String,
    #[serde(rename = "type", default = "default_var_type")]
    pub var_type: VarType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub transform: Transform,
}

fn default_var_type() -> VarType {
    VarType::Text
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VarType {
    Text,
    Select,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    #[default]
    None,
    TitleUnderscore,
    UpperUnderscore,
    LowerUnderscore,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FolderNode {
    pub name: String,
    #[serde(default)]
    pub children: Vec<FolderNode>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    /// Inline template content with {token} interpolation.
    #[serde(default)]
    pub template: String,
    /// Raw content (no interpolation).
    #[serde(default)]
    pub content: String,
}

// ---------------------------------------------------------------------------
// Load / save / list
// ---------------------------------------------------------------------------

impl Template {
    /// Every top-level `template.yaml` key this struct is authoritative for.
    ///
    /// Two of them are never serialized and are listed anyway. `files` is a
    /// pre-v0.8 flat block: since the `files/` directory became the spec, such a
    /// block is ignored on load, and it must keep being *dropped* on save rather
    /// than surviving as an unknown key that no longer means anything. `dir` is
    /// an in-memory convenience that never belonged in a manifest. Kept honest by
    /// `owned_keys_covers_every_serialized_field`.
    pub const OWNED_KEYS: &'static [&'static str] = &[
        "name",
        "slug",
        "description",
        "version",
        "naming_pattern",
        "id",
        "variables",
        "structure",
        "verbatim",
        "exclude",
        "post_create",
        "tags",
        "tag_from",
        "files",
        "dir",
    ];

    /// The `files/` subtree of this template (the spec reproduced into projects).
    pub fn files_dir(&self) -> PathBuf {
        self.dir.join("files")
    }

    /// Load a template from its `template.yaml` manifest. The manifest holds
    /// metadata only; the sibling `files/` directory holds the actual spec. The
    /// UTF-8 text files under `files/` are scanned into the in-memory `files`
    /// buffer (for editors/previews); binaries stay on disk only.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("reading template {}", path.display()))?;
        // Strip a UTF-8 BOM. Notepad, PowerShell's `Out-File -Encoding utf8`,
        // and plenty of other Windows editors add one by default, and serde_yaml
        // then fails with a thoroughly misleading `missing field \`slug\``
        // pointing at line 1 column 2 — while `slug` is sitting right there.
        // `project_info::split_frontmatter_body` has stripped it for years; the
        // template loader simply never got the same treatment.
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        let mut t: Self = serde_yaml::from_str(raw).with_context(|| {
            format!(
                "parsing template {}\n  (if you edited this file on Windows, \
                 check it is saved as UTF-8 without a BOM)",
                path.display()
            )
        })?;
        t.dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        t.scan_files();
        // Silently strip file entries colliding with the reserved auto-gen
        // filename. fastf always owns PROJECT_INFO.md.
        t.strip_reserved_files();
        t.validate()?;
        Ok(t)
    }

    /// Read the UTF-8 text files under `files/` into the `files` buffer. Binary
    /// or oversize files are left on disk only (the copy engine handles them).
    fn scan_files(&mut self) {
        self.files.clear();
        let files_dir = self.files_dir();
        let entries = match crate::core::assets::walk(&files_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries {
            // Only plain files carry editable text; dirs, links and special
            // files have nothing to scan into the buffer.
            if !entry.is_file() || entry.size > crate::core::assets::TEXT_MAX_BYTES {
                continue;
            }
            if let Ok(text) = fs::read_to_string(files_dir.join(&entry.rel)) {
                self.files.push(FileEntry {
                    path: entry.rel,
                    template: text,
                    content: String::new(),
                });
            }
        }
    }

    /// Persist a template in folder form: write `template.yaml` (metadata only)
    /// at `path` and flush the in-memory text `files` buffer into the sibling
    /// `files/` directory. Binaries already on disk are untouched.
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        // Defense in depth: never persist a reserved-name file entry.
        let mut snapshot = self.clone();
        snapshot.strip_reserved_files();
        // Validation must precede even directory creation. A caller commonly
        // derives `path` from the slug, so creating its parent first would let
        // an invalid `../slug` cause a filesystem side effect before rejection.
        snapshot.validate()?;

        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        // `template.yaml` is a file the user owns and may have keys in it that
        // this build knows nothing about. Merge onto what is already there so an
        // editor save is an edit, not a rewrite that silently deletes them.
        let raw = match fs::read_to_string(path) {
            Ok(existing) => crate::util::yaml::to_string_preserving_unknown(
                &snapshot,
                &existing,
                Template::OWNED_KEYS,
            )
            .context("serializing template")?,
            // No readable manifest yet: this is a new template, or the old file
            // is unreadable and there is nothing to preserve from it.
            Err(_) => serde_yaml::to_string(&snapshot).context("serializing template")?,
        };
        // Atomic: a manifest truncated by a crash is a template that no longer
        // loads, and `load_all` is what every create reads.
        crate::util::atomic::write(path, raw)
            .with_context(|| format!("writing {}", path.display()))?;
        crate::util::faults::check("template:mid-save")?;

        // Flush text files into files/. Uses `path`'s parent (authoritative)
        // rather than `self.dir`, which may be unset on an in-memory template.
        let files_dir = dir.join("files");
        for f in &snapshot.files {
            crate::core::validated::SafeRelativePath::parse(&f.path)?;
            let dest = files_dir.join(&f.path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            let content = if !f.template.is_empty() {
                &f.template
            } else {
                &f.content
            };
            crate::util::atomic::write(&dest, content)
                .with_context(|| format!("writing {}", dest.display()))?;
        }
        Ok(())
    }

    /// Remove file entries whose path collides with the reserved auto-gen
    /// metadata filename (PROJECT_INFO.md at the project root, case-insensitive).
    /// Called from `load_from_file` and `save_to_file` — fastf always owns that
    /// file, so a template-defined version would just get overwritten.
    pub(crate) fn strip_reserved_files(&mut self) {
        self.files
            .retain(|f| !crate::core::project_info::path_is_reserved(&f.path));
    }

    pub fn validate(&self) -> Result<()> {
        TemplateSlug::parse(&self.slug).context("template has invalid slug")?;
        if self.name.is_empty() {
            bail!("template 'name' is required");
        }
        if self.naming_pattern.is_empty() {
            bail!("template 'naming_pattern' is required");
        }
        // Check for duplicate variable slugs
        let mut seen = std::collections::HashSet::new();
        for v in &self.variables {
            if !seen.insert(&v.slug) {
                bail!("duplicate variable slug '{}'", v.slug);
            }
        }
        // Reject file paths that escape the project root (absolute, `..`, etc.).
        let mut file_paths = std::collections::HashSet::new();
        for f in &self.files {
            SafeRelativePath::parse(&f.path)
                .with_context(|| format!("template '{}' has invalid file path", self.slug))?;
            if !file_paths.insert(&f.path) {
                bail!(
                    "duplicate file path '{}' in template '{}'",
                    f.path,
                    self.slug
                );
            }
        }
        validate_structure(&self.structure, &self.slug)?;
        // tag_from entries must reference declared variable slugs.
        let var_slugs: std::collections::HashSet<&str> =
            self.variables.iter().map(|v| v.slug.as_str()).collect();
        for slug in &self.tag_from {
            if !var_slugs.contains(slug.as_str()) {
                bail!(
                    "tag_from slug '{}' is not a declared variable in template '{}'",
                    slug,
                    self.slug
                );
            }
        }
        Ok(())
    }

    /// Path to this template's manifest: `templates/<slug>/template.yaml`.
    pub fn file_path(&self) -> PathBuf {
        paths::template_manifest(&self.slug)
    }
}

/// Load all templates from the templates directory. Each template is a folder
/// `templates/<slug>/` containing a `template.yaml` manifest.
pub fn load_all() -> Result<Vec<Template>> {
    let dir = paths::templates_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut templates = Vec::new();
    for entry in
        fs::read_dir(&dir).with_context(|| format!("reading templates dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("template.yaml");
        if !manifest.exists() {
            continue;
        }
        match Template::load_from_file(&manifest) {
            Ok(t) => templates.push(t),
            Err(e) => eprintln!("warning: skipping {}: {}", manifest.display(), e),
        }
    }
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(templates)
}

/// Find a template by slug.
pub fn find_by_slug(slug: &str) -> Result<Template> {
    let slug = TemplateSlug::parse(slug)?;
    let path = paths::template_manifest(slug.as_str());
    if !path.exists() {
        bail!(
            "template '{}' not found — run `fastf template list` to see available templates",
            slug
        );
    }
    Template::load_from_file(&path)
}

fn validate_structure(nodes: &[FolderNode], template_slug: &str) -> Result<()> {
    for node in nodes {
        SafeRelativePath::parse(&node.name).with_context(|| {
            format!(
                "template '{}' has invalid structure path '{}'",
                template_slug, node.name
            )
        })?;
        validate_structure(&node.children, template_slug)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field added to `Template` without being added to `OWNED_KEYS` would be
    /// preserved from the old manifest instead of updated, so an edit made in the
    /// TUI builder or the browser editor would appear to save and change nothing.
    #[test]
    fn owned_keys_covers_every_serialized_field() {
        // Populated so nothing is skipped: `verbatim` and `exclude` are omitted
        // when empty, and the two `#[serde(skip)]` fields never appear at all.
        let tmpl = Template {
            name: "T".to_string(),
            slug: "t".to_string(),
            naming_pattern: "{id}".to_string(),
            verbatim: vec!["*.png".to_string()],
            exclude: vec!["*.tmp".to_string()],
            ..Template::default()
        };
        let serialized = crate::util::yaml::serialized_keys(&tmpl);
        for key in &serialized {
            assert!(
                Template::OWNED_KEYS.contains(&key.as_str()),
                "`{key}` is serialized into template.yaml but missing from OWNED_KEYS"
            );
        }
        // The two deliberately unserialized keys, listed so that a stale flat
        // `files:` block keeps being dropped rather than preserved.
        for key in ["files", "dir"] {
            assert!(Template::OWNED_KEYS.contains(&key));
            assert!(!serialized.contains(&key.to_string()));
        }
    }
}
