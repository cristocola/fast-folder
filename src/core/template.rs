use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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
        let mut t: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("parsing template {}", path.display()))?;
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
            if entry.is_dir || entry.size > crate::core::assets::TEXT_MAX_BYTES {
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
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        // Defense in depth: never persist a reserved-name file entry.
        let mut snapshot = self.clone();
        snapshot.strip_reserved_files();

        let raw = serde_yaml::to_string(&snapshot).context("serializing template")?;
        fs::write(path, raw).with_context(|| format!("writing {}", path.display()))?;

        // Flush text files into files/. Uses `path`'s parent (authoritative)
        // rather than `self.dir`, which may be unset on an in-memory template.
        let files_dir = dir.join("files");
        for f in &snapshot.files {
            crate::core::naming::ensure_relative_safe_path(&f.path)?;
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
            fs::write(&dest, content).with_context(|| format!("writing {}", dest.display()))?;
        }
        Ok(())
    }

    /// Remove file entries whose path collides with the reserved auto-gen
    /// metadata filename (PROJECT_INFO.md at the project root, case-insensitive).
    /// Called from `load_from_file` and `save_to_file` — fastf always owns that
    /// file, so a template-defined version would just get overwritten.
    pub fn strip_reserved_files(&mut self) {
        self.files
            .retain(|f| !crate::core::project_info::path_is_reserved(&f.path));
    }

    pub fn validate(&self) -> Result<()> {
        if self.slug.is_empty() {
            bail!("template 'slug' is required");
        }
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
            crate::core::naming::ensure_relative_safe_path(&f.path)
                .with_context(|| format!("template '{}' has invalid file path", self.slug))?;
            if !file_paths.insert(&f.path) {
                bail!(
                    "duplicate file path '{}' in template '{}'",
                    f.path,
                    self.slug
                );
            }
        }
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
    let path = paths::template_manifest(slug);
    if !path.exists() {
        bail!(
            "template '{}' not found — run `fastf template list` to see available templates",
            slug
        );
    }
    Template::load_from_file(&path)
}
