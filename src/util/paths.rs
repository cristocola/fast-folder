use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// How the data directory (config, templates, counters) was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirMode {
    /// `FASTF_INSTALL_DIR` environment variable override.
    EnvOverride,
    /// Portable mode: the binary's own directory (it contains `config.toml`
    /// or `templates/`).
    Portable,
    /// Per-user config directory (`~/.config/fastf`, `%APPDATA%\fastf`).
    UserDir,
}

impl DirMode {
    pub fn label(&self) -> &'static str {
        match self {
            DirMode::EnvOverride => "env override (FASTF_INSTALL_DIR)",
            DirMode::Portable => "portable (next to the binary)",
            DirMode::UserDir => "user config directory",
        }
    }
}

/// Resolve the directory where config, templates, and counters live.
///
/// Precedence:
/// 1. `FASTF_INSTALL_DIR` (non-empty) — test hermeticity hatch + power users.
/// 2. Portable mode: the binary's directory, iff it already contains a
///    `config.toml` or a `templates/` dir. Keeps binary-plus-data folders
///    (USB stick, `target/release/`) working exactly as before.
/// 3. The per-user config directory: `$XDG_CONFIG_HOME/fastf` (or
///    `~/.config/fastf`) on Unix, `%APPDATA%\fastf` on Windows — the only
///    option that works when the binary sits in a read-only location like
///    `/usr/bin`.
///
/// No memoization on purpose: tests swap `FASTF_INSTALL_DIR` within one
/// process, and the fallback costs only a couple of `stat` calls.
pub fn try_install_dir() -> Result<(PathBuf, DirMode)> {
    if let Ok(override_dir) = std::env::var("FASTF_INSTALL_DIR")
        && !override_dir.is_empty()
    {
        return Ok((PathBuf::from(override_dir), DirMode::EnvOverride));
    }
    if let Some(dir) = portable_dir() {
        return Ok((dir, DirMode::Portable));
    }
    Ok((user_config_dir()?, DirMode::UserDir))
}

/// Infallible wrapper around [`try_install_dir`] for the ~30 path helpers and
/// their callers. `main()` runs `try_install_dir()?` first thing, so in the
/// binary this can only be reached after a successful resolution; the exit
/// branch is belt-and-braces for library consumers (e.g. UI server threads).
pub fn install_dir() -> PathBuf {
    match try_install_dir() {
        Ok((dir, _)) => dir,
        Err(err) => {
            eprintln!(
                "fastf: cannot determine data directory: {err}. \
                 Set FASTF_INSTALL_DIR to choose one."
            );
            std::process::exit(2);
        }
    }
}

/// Portable-mode probe: the canonicalized directory of the running binary,
/// iff it already holds fastf data (`config.toml` or `templates/`).
fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    if is_portable_data_dir(dir) {
        Some(dir.to_path_buf())
    } else {
        None
    }
}

fn is_portable_data_dir(dir: &Path) -> bool {
    dir.join("config.toml").is_file() || dir.join("templates").is_dir()
}

/// The user's home directory (`%USERPROFILE%` on Windows, `$HOME` elsewhere).
/// Hand-rolled like `user_config_dir` — no `dirs` crate.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Per-user config directory, hand-rolled (no `dirs` crate — two env lookups).
#[cfg(windows)]
fn user_config_dir() -> Result<PathBuf> {
    user_config_dir_from(
        std::env::var("APPDATA").ok().as_deref(),
        std::env::var("USERPROFILE").ok().as_deref(),
    )
}

#[cfg(windows)]
fn user_config_dir_from(appdata: Option<&str>, profile: Option<&str>) -> Result<PathBuf> {
    if let Some(appdata) = appdata
        && !appdata.is_empty()
    {
        return Ok(PathBuf::from(appdata).join("fastf"));
    }
    if let Some(profile) = profile
        && !profile.is_empty()
    {
        return Ok(PathBuf::from(profile)
            .join("AppData")
            .join("Roaming")
            .join("fastf"));
    }
    bail!("neither %APPDATA% nor %USERPROFILE% is set")
}

#[cfg(not(windows))]
fn user_config_dir() -> Result<PathBuf> {
    user_config_dir_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

#[cfg(not(windows))]
fn user_config_dir_from(xdg: Option<&str>, home: Option<&str>) -> Result<PathBuf> {
    if let Some(xdg) = xdg
        && !xdg.is_empty()
        && Path::new(xdg).is_absolute()
    {
        return Ok(PathBuf::from(xdg).join("fastf"));
    }
    if let Some(home) = home
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home).join(".config").join("fastf"));
    }
    bail!("neither $XDG_CONFIG_HOME nor $HOME is set")
}

/// Render a path for humans, stripping Windows' `\\?\` extended-length prefix.
///
/// `Path::canonicalize` returns the verbatim form on Windows, so every path that
/// had been through it surfaced as `\\?\C:\Users\...` — in the create success
/// line, in `recent`, in `move`, and baked into every project's
/// `PROJECT_INFO.md`. It is a valid path, but not one anyone wants to read or
/// paste, and it reads as a bug.
///
/// **Display only.** The verbatim form is what makes paths beyond `MAX_PATH`
/// work, and long-path support without it is an opt-in system setting that is
/// off on many machines — so filesystem calls keep the canonical path and only
/// the rendering is cleaned up.
///
/// - `\\?\C:\foo`            → `C:\foo`
/// - `\\?\UNC\server\share`  → `\\server\share`
/// - anything else           → unchanged
pub fn display_path(path: &Path) -> String {
    strip_verbatim(&path.display().to_string())
}

/// The string half of [`display_path`], split out so Windows-shaped inputs can
/// be unit-tested on any platform.
fn strip_verbatim(raw: &str) -> String {
    const VERBATIM: &str = r"\\?\";
    const VERBATIM_UNC: &str = r"\\?\UNC\";

    if let Some(rest) = raw.strip_prefix(VERBATIM_UNC) {
        // `\\?\UNC\server\share` is really `\\server\share`.
        return format!(r"\\{rest}");
    }
    let Some(rest) = raw.strip_prefix(VERBATIM) else {
        return raw.to_string();
    };
    // Only unwrap a plain drive path (`C:\...`). Anything else behind the prefix
    // — a device path like `\\?\Volume{guid}\` — means something specific and
    // has to be shown as it is.
    let bytes = rest.as_bytes();
    let is_drive_path = bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'\\');
    if is_drive_path {
        rest.to_string()
    } else {
        raw.to_string()
    }
}

pub fn config_path() -> PathBuf {
    install_dir().join("config.toml")
}

pub fn counters_path() -> PathBuf {
    install_dir().join("counters.toml")
}

pub fn templates_dir() -> PathBuf {
    install_dir().join("templates")
}

/// Directory holding a single template (folder form): `templates/<slug>/`.
/// Contains `template.yaml` (metadata) and a `files/` subtree (the spec).
pub fn template_dir(slug: &str) -> PathBuf {
    templates_dir().join(slug)
}

/// The metadata manifest for a template: `templates/<slug>/template.yaml`.
pub fn template_manifest(slug: &str) -> PathBuf {
    template_dir(slug).join("template.yaml")
}

/// The bundled-files subtree for a template: `templates/<slug>/files/`.
/// Everything here is reproduced into new projects (names + text interpolated).
pub fn template_files_dir(slug: &str) -> PathBuf {
    template_dir(slug).join("files")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_verbatim_prefix_for_display() {
        // The exact shape that leaked into create/recent/move output.
        assert_eq!(
            strip_verbatim(r"\\?\C:\Users\Cristo\Projects\2026_Thing_ID0001"),
            r"C:\Users\Cristo\Projects\2026_Thing_ID0001"
        );
        assert_eq!(strip_verbatim(r"\\?\E:\"), r"E:\");
        // UNC round-trips to the familiar double-backslash form.
        assert_eq!(
            strip_verbatim(r"\\?\UNC\server\share\proj"),
            r"\\server\share\proj"
        );
        // Device paths mean something specific — leave them alone.
        assert_eq!(
            strip_verbatim(r"\\?\Volume{9f3a}\data"),
            r"\\?\Volume{9f3a}\data"
        );
        // Ordinary paths are untouched, on either platform.
        assert_eq!(strip_verbatim(r"C:\already\plain"), r"C:\already\plain");
        assert_eq!(strip_verbatim("/home/user/projects"), "/home/user/projects");
        assert_eq!(strip_verbatim(""), "");
    }

    #[test]
    fn portable_marker_detection() {
        let tmp = std::env::temp_dir().join(format!("fastf-paths-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!is_portable_data_dir(&tmp));
        std::fs::write(tmp.join("config.toml"), "").unwrap();
        assert!(is_portable_data_dir(&tmp));
        std::fs::remove_file(tmp.join("config.toml")).unwrap();
        std::fs::create_dir_all(tmp.join("templates")).unwrap();
        assert!(is_portable_data_dir(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(windows))]
    #[test]
    fn user_config_dir_precedence() {
        // Absolute XDG_CONFIG_HOME wins.
        assert_eq!(
            user_config_dir_from(Some("/tmp/xdg-test"), Some("/home/testuser")).unwrap(),
            PathBuf::from("/tmp/xdg-test/fastf")
        );
        // Relative or empty XDG_CONFIG_HOME is ignored (per the XDG spec).
        assert_eq!(
            user_config_dir_from(Some("relative/dir"), Some("/home/testuser")).unwrap(),
            PathBuf::from("/home/testuser/.config/fastf")
        );
        assert_eq!(
            user_config_dir_from(Some(""), Some("/home/testuser")).unwrap(),
            PathBuf::from("/home/testuser/.config/fastf")
        );
        assert_eq!(
            user_config_dir_from(None, Some("/home/testuser")).unwrap(),
            PathBuf::from("/home/testuser/.config/fastf")
        );
        // Nothing set → error, not a panic.
        assert!(user_config_dir_from(None, None).is_err());
        assert!(user_config_dir_from(None, Some("")).is_err());
    }
}
