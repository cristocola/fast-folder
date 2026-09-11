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

/// A folder name a project may actually be created under.
///
/// **The one validator for "what may a project folder be called".** `plan`,
/// `rename_project_inner` and `register`'s `--rename` all go through it, so a
/// name that is refused in one of them is refused in all three.
///
/// [`crate::core::naming::sanitize_name`] does the character-level work — it maps the
/// characters no filesystem accepts and trims the trailing dots and spaces
/// Windows strips silently. What it deliberately does not do is *refuse*: it
/// returns `""` for `".."` and leaves a leading `.` alone, because it has no
/// opinion about whether an empty or hidden name is meaningful to its caller.
/// This type has that opinion:
///
/// - **Empty** cannot be joined onto a base. `base.join("")` is `base` itself,
///   which `exists()` answers yes to — that is how `--name=..` came to claim a
///   folder named `_2` beside the base rather than inside it.
/// - **Dot-prefixed** would be invisible: discovery skips dot-prefixed
///   directories (they are fastf's own staging), so the project would show up
///   once from the write-through cache and then vanish at the next rescan.
/// - **More than one path component** would put the project somewhere other
///   than the base it was planned for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectFolderName(String);

impl ProjectFolderName {
    pub fn parse(raw: &str) -> Result<Self> {
        let sanitized = crate::core::naming::sanitize_name(raw.trim());

        if sanitized.is_empty() {
            // Two different empties, and saying "every character in it is one a
            // folder name may not contain" about `""` is nonsense.
            if raw.trim().is_empty() {
                bail!("a folder name cannot be empty");
            }
            bail!(
                "'{}' leaves no usable folder name: nothing survives trimming the \
                 trailing dots and spaces a filesystem would strip anyway",
                raw.trim()
            );
        }
        if sanitized.starts_with('.') {
            bail!(
                "a folder name may not start with '.': fastf would not see the project \
                 (got '{sanitized}')"
            );
        }
        // Belt and braces. `sanitize_name` maps both separators to `_`, so this
        // cannot fire today — but it is the rule the type exists to state, and a
        // future change to the character map must not quietly repeal it.
        if sanitized.contains('/') || sanitized.contains('\\') {
            bail!("a folder name must be a single path component (got '{sanitized}')");
        }

        Ok(Self(sanitized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ProjectFolderName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A tag: one word, or a `namespace/value` path of words.
///
/// **The one validator for what a tag may be**, at the one door every tag
/// comes through (`operations::add_tags`, and so the CLI, the app's prompt
/// and the pane alike). Tags were "arbitrary strings you add yourself", which
/// admitted a paragraph, a newline — which is a second YAML list item on the
/// way back in — and a tag that was only spaces. A tag is something you filter
/// by and something `tag:x` has to be able to spell in the search bar, so:
/// trimmed, non-empty, one line, no whitespace, at most [`Tag::MAX_LEN`]
/// characters, letters and digits and `- _ . /`, and a `/` only *between*
/// parts — `client/Acme` is the convention `tag_from` writes, and a bare
/// `client/` is what it refuses to write.
///
/// Every refusal names the rule, because the person who typed it is looking
/// at a field they can correct.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tag(String);

impl Tag {
    /// Long enough for `department/sub-project/2026-q3`, short enough that
    /// a tag stays a tag and not a sentence.
    pub const MAX_LEN: usize = 64;

    pub fn parse(raw: &str) -> Result<Self> {
        let tag = raw.trim();
        if tag.is_empty() {
            bail!("a tag cannot be empty");
        }
        if tag.chars().any(char::is_whitespace) {
            bail!("a tag is one word — use '-' or '_' instead of a space (got '{tag}')");
        }
        if tag.chars().count() > Self::MAX_LEN {
            bail!(
                "a tag is at most {} characters (got {})",
                Self::MAX_LEN,
                tag.chars().count()
            );
        }
        if let Some(bad) = tag
            .chars()
            .find(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
        {
            bail!(
                "a tag may use letters, digits, '-', '_', '.' and '/' — not '{bad}' (got '{tag}')"
            );
        }
        if tag.starts_with('/') || tag.ends_with('/') || tag.contains("//") {
            bail!("a '/' in a tag goes between two parts, as in client/Acme (got '{tag}')");
        }
        Ok(Self(tag.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Tag {
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
    fn a_tag_is_one_word_of_letters_digits_and_a_few_marks() {
        for ok in ["draft", "client/Acme", "v2.1", "a-b_c", "Ärger", "x/y/z"] {
            assert_eq!(Tag::parse(&format!("  {ok} ")).unwrap().as_str(), ok);
        }
        for (bad, names) in [
            ("", "empty"),
            ("   ", "empty"),
            ("two words", "one word"),
            ("a\nb", "one word"),
            ("client/", "between two parts"),
            ("/client", "between two parts"),
            ("a//b", "between two parts"),
            ("no,commas", "not ','"),
            ("quote\"d", "not '\"'"),
        ] {
            let err = Tag::parse(bad).unwrap_err().to_string();
            assert!(err.contains(names), "{bad:?}: {err}");
        }
        let long = "x".repeat(Tag::MAX_LEN + 1);
        assert!(
            Tag::parse(&long)
                .unwrap_err()
                .to_string()
                .contains("at most")
        );
        assert!(Tag::parse(&"x".repeat(Tag::MAX_LEN)).is_ok());
    }

    #[test]
    fn a_project_folder_name_survives_sanitizing_and_stays_visible() {
        for (raw, expected) in [
            ("Spring Campaign", "Spring Campaign"),
            ("  padded  ", "padded"),
            // Sanitizing still happens; it just no longer has the last word.
            ("a/b", "a_b"),
            ("a\\b", "a_b"),
            ("Draft .", "Draft"),
            ("CON", "CON_"),
        ] {
            assert_eq!(
                ProjectFolderName::parse(raw).unwrap().as_str(),
                expected,
                "parsing {raw:?}"
            );
        }
    }

    /// The two shapes that used to reach `create_dir` and should not.
    ///
    /// Note what is *not* here: a name of purely illegal characters. `?*|`
    /// sanitizes to `___`, which is a real, visible, findable folder — silly,
    /// but not a defect. Only names that sanitize away to nothing are refused.
    #[test]
    fn a_project_folder_name_refuses_empty_and_hidden_names() {
        for raw in ["", "   "] {
            let error = ProjectFolderName::parse(raw)
                .expect_err("an empty name must be refused")
                .to_string();
            assert!(error.contains("cannot be empty"), "{raw:?} gave: {error}");
        }

        for raw in ["..", ".", "...", ". . ."] {
            let error = ProjectFolderName::parse(raw)
                .expect_err("a name that sanitizes away must be refused")
                .to_string();
            assert!(
                error.contains("leaves no usable folder name"),
                "{raw:?} gave: {error}"
            );
        }

        for raw in [".hidden", ".fastf-transactions", " .git"] {
            let error = ProjectFolderName::parse(raw)
                .expect_err("expected a dot-prefixed name to be refused")
                .to_string();
            assert!(
                error.contains("may not start with '.'"),
                "{raw:?} gave: {error}"
            );
        }
    }

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
        assert!(SafeRelativePath::parse("README.md").is_ok());
        assert!(SafeRelativePath::parse("deeply/nested/file.txt").is_ok());
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
            "a/b/../../c.txt",
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
