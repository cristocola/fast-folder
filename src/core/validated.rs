//! Validated identifiers and paths used at filesystem boundaries.
//!
//! Keeping these as types makes it difficult for an untrusted CLI/UI string to
//! reach `Path::join` accidentally.  Template paths use `/` in their portable
//! representation and are converted to native [`PathBuf`]s only after they have
//! passed validation.

use anyhow::{Result, bail};
use std::fmt;
use std::path::{Path, PathBuf};

/// A template directory slug (`letters`, `numbers`, `-`, and `_` only).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemplateSlug(String);

impl TemplateSlug {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() {
            bail!("template slug cannot be empty");
        }
        if !raw
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            bail!(
                "template slug '{}' contains invalid characters; use letters, numbers, '-' or '_'",
                raw
            );
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TemplateSlug {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A non-empty relative path whose components cannot escape its eventual root.
///
/// Both slash styles are accepted at the boundary. The stored representation
/// always uses `/`, which is the portable syntax used by template manifests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SafeRelativePath(String);

impl SafeRelativePath {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() {
            bail!("relative path is empty");
        }
        if raw.contains('\0') {
            bail!("relative path contains a NUL byte");
        }

        let normalized = raw.replace('\\', "/");
        if normalized.starts_with('/') {
            bail!("path '{}' must be relative (no leading slash)", raw);
        }

        // `Path::is_absolute` is host-specific. Check drive-qualified Windows
        // paths explicitly so a hostile template is rejected on Linux before
        // it can later be carried to Windows.
        let bytes = normalized.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            bail!("path '{}' must not contain a drive letter", raw);
        }

        for component in normalized.split('/') {
            match component {
                "" => bail!("path '{}' contains an empty component", raw),
                "." => bail!("path '{}' must not contain '.'", raw),
                ".." => bail!("path '{}' must not contain '..'", raw),
                _ => {}
            }
        }

        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Convert the portable representation into a native relative path.
    pub fn to_path_buf(&self) -> PathBuf {
        self.0.split('/').collect()
    }

    /// Join this validated path beneath `root`.
    pub fn join_to(&self, root: &Path) -> PathBuf {
        root.join(self.to_path_buf())
    }
}

impl fmt::Display for SafeRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_slug_accepts_only_one_safe_component() {
        for slug in ["general", "music-video", "client_2", "T42"] {
            assert_eq!(TemplateSlug::parse(slug).unwrap().as_str(), slug);
        }
        for slug in ["", ".", "../general", "a/b", "a\\b", "/tmp/x", "C:\\x"] {
            assert!(TemplateSlug::parse(slug).is_err(), "accepted {slug:?}");
        }
    }

    #[test]
    fn safe_relative_path_preserves_nested_paths() {
        let path = SafeRelativePath::parse("src/components/button.rs").unwrap();
        assert_eq!(path.as_str(), "src/components/button.rs");
        assert_eq!(
            path.join_to(Path::new("root")),
            Path::new("root").join("src/components/button.rs")
        );
    }

    #[test]
    fn safe_relative_path_rejects_host_independent_escapes() {
        for path in [
            "",
            ".",
            "..",
            "../outside",
            "inside/../../outside",
            "/etc/passwd",
            "\\server\\share",
            "C:/Windows",
            "D:\\Windows",
            "src//main.rs",
            "src/./main.rs",
        ] {
            assert!(SafeRelativePath::parse(path).is_err(), "accepted {path:?}");
        }
    }
}
