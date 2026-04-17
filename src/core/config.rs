use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::util::paths;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    /// Base directory for new projects. Empty = current working directory.
    #[serde(default)]
    pub base_dir: String,

    /// Editor to use for `fastf template edit`. Empty = $EDITOR env var.
    #[serde(default)]
    pub editor: String,

    /// Slug of the default template to use. Empty = always prompt.
    #[serde(default)]
    pub default_template: String,

    /// strftime format for the {date} token. Default: %Y-%m-%d
    #[serde(default = "default_date_format")]
    pub date_format: String,

    /// How many lines of each templated file to show in the rich dry-run preview.
    /// Set to 0 to suppress file-content previews entirely.
    #[serde(default = "default_preview_lines")]
    pub preview_lines: usize,

    /// Default post-create actions applied to every project unless a template
    /// overrides them with its own `post_create` block.
    #[serde(default)]
    pub post_create: crate::core::post_create::PostCreate,
}

fn default_date_format() -> String {
    "%Y-%m-%d".to_string()
}

fn default_preview_lines() -> usize { 8 }

impl Default for Config {
    fn default() -> Self {
        Self {
            base_dir: String::new(),
            editor: String::new(),
            default_template: String::new(),
            date_format: default_date_format(),
            preview_lines: default_preview_lines(),
            post_create: Default::default(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = paths::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self = toml::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::config_path();
        let raw = toml::to_string_pretty(self)
            .context("serializing config")?;
        fs::write(&path, raw)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Resolve base directory: configured path, or current working directory.
    pub fn resolve_base_dir(&self) -> std::path::PathBuf {
        if self.base_dir.is_empty() {
            std::env::current_dir().expect("cannot get current dir")
        } else {
            std::path::PathBuf::from(&self.base_dir)
        }
    }

    /// Resolve editor: configured, or $EDITOR, or fallback.
    pub fn resolve_editor(&self) -> String {
        if !self.editor.is_empty() {
            return self.editor.clone();
        }
        std::env::var("EDITOR").unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "nano".to_string()
            }
        })
    }
}
