//! The asset copy engine for folder-form templates.
//!
//! A template is a folder whose `files/` subtree IS the spec: every file and
//! directory under `files/` is reproduced into each new project. Names and
//! UTF-8 text contents get `{token}` interpolation; binaries (and anything
//! matched by a `verbatim` glob, or larger than [`TEXT_MAX_BYTES`], or not
//! valid UTF-8) are copied byte-for-byte. `exclude` globs are never copied.
//!
//! There is no per-file manifest — convention over configuration. This module
//! walks the real directory and is the single source of truth for `fastf new`
//! and `fastf apply` (CLI and UI share it).

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::naming::interpolate_name;

/// Text files at or below this size are candidates for `{token}` interpolation.
/// Anything larger is copied verbatim — interpolating a 200 MB file makes no
/// sense and would blow up memory.
pub const TEXT_MAX_BYTES: u64 = 1024 * 1024;

/// One physical entry discovered under a template's `files/` directory.
pub struct AssetEntry {
    /// Path relative to `files/`, forward-slash separated, **uninterpolated**.
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Recursively list every entry under `files_dir` (directories included so that
/// deliberately-empty folders are reproduced). Returns an empty vec when the
/// directory does not exist. Results are sorted so parents precede children.
pub fn walk(files_dir: &Path) -> Result<Vec<AssetEntry>> {
    let mut out = Vec::new();
    if !files_dir.exists() {
        return Ok(out);
    }
    walk_inner(files_dir, files_dir, &mut out)?;
    // Lexicographic sort puts a parent ("a") before its children ("a/b").
    out.sort_by(|x, y| x.rel.cmp(&y.rel));
    Ok(out)
}

fn walk_inner(root: &Path, current: &Path, out: &mut Vec<AssetEntry>) -> Result<()> {
    for entry in fs::read_dir(current).with_context(|| format!("reading {}", current.display()))? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if ft.is_dir() {
            out.push(AssetEntry {
                rel,
                is_dir: true,
                size: 0,
            });
            walk_inner(root, &path, out)?;
        } else if ft.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(AssetEntry {
                rel,
                is_dir: false,
                size,
            });
        }
        // symlinks / fifos / etc. are intentionally skipped
    }
    Ok(())
}

/// Interpolate a relative path segment-by-segment, so empty variables collapse
/// underscores *within* each name component without touching the `/` separators.
pub fn interp_rel(rel: &str, vars: &HashMap<String, String>, date_format: &str) -> String {
    rel.split('/')
        .map(|segment| interpolate_name(segment, vars, date_format))
        .collect::<Vec<_>>()
        .join("/")
}

/// Match a glob (`*` = any run, `?` = one char) against `text`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Iterative wildcard match with backtracking on `*`.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A glob with no `/` is matched against the basename; a glob containing `/`
/// is matched against the full relative path.
fn matches_any(rel: &str, patterns: &[String]) -> bool {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    patterns.iter().any(|pat| {
        if pat.contains('/') {
            glob_match(pat, rel)
        } else {
            glob_match(pat, base)
        }
    })
}

/// True when `rel` matches an `exclude` glob (never copied).
pub fn is_excluded(rel: &str, exclude: &[String]) -> bool {
    matches_any(rel, exclude)
}

/// True when `rel` matches a `verbatim` glob (copied literally even if text).
pub fn is_verbatim(rel: &str, verbatim: &[String]) -> bool {
    matches_any(rel, verbatim)
}

/// Copy one file, atomically (`.part` temp + rename).
///
/// When `force_verbatim` is false the source is read as UTF-8 and, if that
/// succeeds, `{token}` interpolation is applied to its contents. A read failure
/// (binary) transparently falls back to a byte copy. `force_verbatim` short-
/// circuits straight to the byte copy (used for `verbatim` globs and oversize
/// files).
pub fn copy_file(
    src: &Path,
    dest: &Path,
    force_verbatim: bool,
    vars: &HashMap<String, String>,
    date_format: &str,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dirs for {}", dest.display()))?;
    }

    let mut tmp_os = dest.as_os_str().to_owned();
    tmp_os.push(".part");
    let tmp = PathBuf::from(tmp_os);

    let interpolated = if force_verbatim {
        None
    } else {
        // Try to read as text; a non-UTF-8 file yields Err → verbatim copy.
        fs::read_to_string(src).ok()
    };

    match interpolated {
        Some(text) => {
            let rendered = crate::core::naming::interpolate(&text, vars, date_format);
            fs::write(&tmp, rendered).with_context(|| format!("writing {}", tmp.display()))?;
        }
        None => {
            fs::copy(src, &tmp)
                .with_context(|| format!("copying {} → {}", src.display(), tmp.display()))?;
        }
    }

    fs::rename(&tmp, dest)
        .with_context(|| format!("finalizing {}", dest.display()))
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches() {
        assert!(glob_match("*.svg", "logo.svg"));
        assert!(!glob_match("*.svg", "logo.png"));
        assert!(glob_match(".DS_Store", ".DS_Store"));
        assert!(glob_match("*.tmp", "a.b.tmp"));
        assert!(glob_match("docs/*.md", "docs/readme.md"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn verbatim_and_exclude_scope() {
        assert!(is_verbatim("assets/logo.svg", &["*.svg".into()]));
        assert!(is_excluded(".DS_Store", &[".DS_Store".into()]));
        assert!(!is_excluded("keep.txt", &["*.tmp".into()]));
    }

    #[test]
    fn interp_rel_is_per_segment() {
        let mut vars = HashMap::new();
        vars.insert("client".to_string(), String::new());
        vars.insert("name".to_string(), "Aurora".to_string());
        // Empty segment variable collapses within the segment, slash preserved.
        let out = interp_rel("05_Delivery/Note_{name}.md", &vars, "%Y-%m-%d");
        assert_eq!(out, "05_Delivery/Note_Aurora.md");
    }
}
