use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::util::paths;

// ---------------------------------------------------------------------------
// Template structs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
#[derive(Default)]
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

    #[serde(default)]
    pub files: Vec<FileEntry>,
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
    #[serde(default = "default_true")]
    pub auto_increment: bool,
}

fn default_id_prefix() -> String { "ID".to_string() }
fn default_id_digits() -> usize  { 4 }
fn default_true() -> bool        { true }

impl Default for IdConfig {
    fn default() -> Self {
        Self {
            prefix: default_id_prefix(),
            digits: default_id_digits(),
            auto_increment: default_true(),
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

fn default_var_type() -> VarType { VarType::Text }

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
    pub fn load_from_file(path: &PathBuf) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("reading template {}", path.display()))?;
        let t: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("parsing template {}", path.display()))?;
        t.validate()?;
        Ok(t)
    }

    #[allow(dead_code)]
    pub fn save_to_file(&self, path: &PathBuf) -> Result<()> {
        let raw = serde_yaml::to_string(self)
            .context("serializing template")?;
        fs::write(path, raw)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    fn validate(&self) -> Result<()> {
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
        Ok(())
    }

    /// Path where this template is stored on disk.
    #[allow(dead_code)]
    pub fn file_path(&self) -> PathBuf {
        paths::templates_dir().join(format!("{}.yaml", self.slug))
    }
}

/// Load all templates from the templates directory.
pub fn load_all() -> Result<Vec<Template>> {
    let dir = paths::templates_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut templates = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("reading templates dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            match Template::load_from_file(&path) {
                Ok(t) => templates.push(t),
                Err(e) => eprintln!("warning: skipping {}: {}", path.display(), e),
            }
        }
    }
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(templates)
}

/// Find a template by slug.
pub fn find_by_slug(slug: &str) -> Result<Template> {
    let path = paths::templates_dir().join(format!("{}.yaml", slug));
    if !path.exists() {
        bail!("template '{}' not found", slug);
    }
    Template::load_from_file(&path)
}
