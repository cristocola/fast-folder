use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::util::paths;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    /// Base directory for new projects. Empty = the user's home directory.
    #[serde(default)]
    pub base_dir: String,

    /// Additional directories to index for the project library (beyond
    /// `base_dir`). Each holds project folders whose `PROJECT_INFO.md` makes
    /// them discoverable. `effective_bases()` unions these with `base_dir`.
    /// An unmounted/absent base is simply skipped at scan time.
    #[serde(default)]
    pub bases: Vec<String>,

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

    /// Show the "Open project folder?" prompt after a successful `fastf new`.
    /// Independent of `post_create.reveal` (which runs unconditionally when set);
    /// the prompt auto-skips when reveal is already enabled to avoid double-open.
    #[serde(default = "default_true")]
    pub prompt_open_after_create: bool,

    /// Page size for the guided TUI's Projects browser and default `--limit`
    /// for `fastf recent`. The key name is retained for compatibility.
    #[serde(default = "default_recent_limit")]
    pub recent_default_limit: usize,

    /// Show the "Create this project?" confirm prompt in `fastf new`.
    /// When `false`, behaves as if `--yes` were always passed.
    #[serde(default = "default_true")]
    pub confirm_create: bool,

    /// Show the ASCII banner at the top of the TUI main menu.
    #[serde(default = "default_true")]
    pub show_banner: bool,

    /// Pattern used by `fastf register --rename` when no `--template` is set.
    /// Tokens: `{date}` (uses `date_format`), `{YYYY}`, `{MM}`, `{DD}`, `{id}`,
    /// and `{name}` — the sanitized basename of the existing folder.
    /// Default `"{date}_{name}_{id}"` produces names like
    /// `2026-05-11_my_video_ID0048`. With `--template`, the template's
    /// `naming_pattern` is used instead and this setting is ignored.
    #[serde(default = "default_register_naming_pattern")]
    pub register_naming_pattern: String,

    /// What to do when the resolved folder name is already taken:
    /// `"suffix"` (default) appends `_2`, `_3`… , `"error"` refuses.
    ///
    /// Rarely reached with the bundled patterns, which end in a unique
    /// `{id}` — but a pattern need not contain one, and then two projects
    /// created the same day from the same answers collide for real. Appending a
    /// suffix is what every file manager does. `"error"` restores the old
    /// refuse-a-duplicate behaviour for anyone who would rather be stopped.
    #[serde(default = "default_on_name_collision")]
    pub on_name_collision: String,
}

fn default_date_format() -> String {
    "%Y-%m-%d".to_string()
}

fn default_preview_lines() -> usize {
    8
}
fn default_true() -> bool {
    true
}
fn default_recent_limit() -> usize {
    20
}
fn default_register_naming_pattern() -> String {
    "{date}_{name}_{id}".to_string()
}
fn default_on_name_collision() -> String {
    "suffix".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_dir: String::new(),
            bases: Vec::new(),
            editor: String::new(),
            default_template: String::new(),
            date_format: default_date_format(),
            preview_lines: default_preview_lines(),
            post_create: Default::default(),
            prompt_open_after_create: true,
            recent_default_limit: default_recent_limit(),
            confirm_create: true,
            show_banner: true,
            register_naming_pattern: default_register_naming_pattern(),
            on_name_collision: default_on_name_collision(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = paths::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    /// Persist the config atomically — a crash mid-write must never truncate it.
    pub fn save(&self) -> Result<()> {
        let path = paths::config_path();
        let raw = toml::to_string_pretty(self).context("serializing config")?;
        crate::util::atomic::write(&path, raw)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Whether a taken folder name should get a `_2` suffix rather than fail.
    ///
    /// Anything other than an explicit `"error"` means suffix, so a typo in the
    /// config leaves creates working instead of blocking them.
    pub fn suffix_on_name_collision(&self) -> bool {
        !self.on_name_collision.trim().eq_ignore_ascii_case("error")
    }

    /// Resolve base directory: configured path, or the user's home directory.
    /// Home (not the cwd) is the unconfigured fallback so an empty `base_dir`
    /// never scatters projects or `.fastf-index.json` caches into whatever
    /// directory a command happens to run from. The cwd remains only as a
    /// last resort when the home env var itself is missing.
    pub fn resolve_base_dir(&self) -> std::path::PathBuf {
        if self.base_dir.is_empty() {
            paths::home_dir()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        } else {
            std::path::PathBuf::from(&self.base_dir)
        }
    }

    /// The full set of directories the project library indexes: `base_dir`
    /// unioned with `bases`, each normalized (canonicalized when it exists) and
    /// de-duplicated. Order is stable: `base_dir` first, then `bases` as listed.
    /// Non-existent paths are kept (not canonicalizable) so callers can decide
    /// to skip them — discovery does, treating an absent base as honestly empty.
    pub fn effective_bases(&self) -> Vec<std::path::PathBuf> {
        let mut candidates = vec![self.resolve_base_dir()];
        for b in &self.bases {
            if !b.trim().is_empty() {
                candidates.push(std::path::PathBuf::from(b));
            }
        }

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for c in candidates {
            let norm = c.canonicalize().unwrap_or(c);
            if seen.insert(norm.clone()) {
                out.push(norm);
            }
        }
        out
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

/// The conventional default projects base offered by first-run onboarding:
/// `<home>/Projects` (`C:\Users\<user>\Projects` on Windows).
pub fn suggested_base_dir() -> Option<std::path::PathBuf> {
    paths::home_dir().map(|home| home.join("Projects"))
}

/// Expand a leading `~` and require an absolute path. Creates nothing.
///
/// **Takes no lock and saves nothing** — that split is mandatory, not cosmetic.
/// `DataLock` is not reentrant, so a validator that locked could not be called
/// from `config::set`, which already holds it. Every entry point for a base path
/// goes through here or [`resolve_base_dir_input`]: onboarding, `fastf config
/// set base-dir` / `bases`, and the same keys in TUI Settings. When only
/// onboarding validated, `config set base-dir '~/Projects'` stored a literal `~`
/// and a relative path was accepted outright — which scattered projects, index
/// caches and a counter file into whatever directory the command happened to
/// run from.
pub fn expand_base_path(raw: &str) -> Result<std::path::PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("Choose a folder for your projects first");
    }
    let expanded = if raw == "~" {
        paths::home_dir().context("cannot expand '~': no home directory found")?
    } else if let Some(rest) = raw
        .strip_prefix("~/")
        .or_else(|| raw.strip_prefix("~\\"))
        .filter(|rest| !rest.is_empty())
    {
        paths::home_dir()
            .context("cannot expand '~': no home directory found")?
            .join(rest)
    } else {
        std::path::PathBuf::from(raw)
    };
    if !expanded.is_absolute() {
        bail!(
            "The base folder must be an absolute path (got '{raw}').\n  \
             A relative path would depend on where the command was run, scattering \
             projects across directories."
        );
    }
    Ok(expanded)
}

/// [`expand_base_path`] plus "make it exist": creates the folder if missing and
/// returns the canonical path. This is for `base_dir`, the one folder new
/// projects are written into.
///
/// Extra bases deliberately do **not** go through here. Creating a missing one
/// would plant an empty directory at an unmounted mount point, shadowing the
/// drive it stands for — an absent base is meant to be skipped, not conjured.
pub fn resolve_base_dir_input(raw: &str) -> Result<std::path::PathBuf> {
    let expanded = expand_base_path(raw)?;
    fs::create_dir_all(&expanded).with_context(|| format!("creating {}", expanded.display()))?;
    // Stored canonical, rendered readable at the display sites. Keeping the
    // verbatim form is what preserves long-path support when this base is later
    // used for filesystem work.
    Ok(expanded.canonicalize().unwrap_or(expanded))
}

/// First-run onboarding core, shared by the TUI prompt and the web UI's
/// `/api/base/init`: validate via [`resolve_base_dir_input`] and persist the
/// result as `base_dir`. Returns the resolved path.
pub fn init_base_dir(raw: &str) -> Result<std::path::PathBuf> {
    let resolved = resolve_base_dir_input(raw)?;
    // A load-mutate-save, so it takes the same cross-process lock as
    // `config set`. No caller holds the lock already (the lock is not
    // reentrant): the web UI's `/api/base/init` takes only `WRITE_LOCK`, and the
    // TUI's onboarding runs before anything else.
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    let mut config = Config::load()?;
    config.base_dir = resolved.display().to_string();
    config.save()?;
    Ok(resolved)
}
