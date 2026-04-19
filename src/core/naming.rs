use chrono::Local;

use crate::core::template::Transform;

/// Apply a transform to a raw string value.
pub fn apply_transform(value: &str, transform: &Transform) -> String {
    match transform {
        Transform::None => value.to_string(),
        Transform::TitleUnderscore => to_title_underscore(value),
        Transform::UpperUnderscore => value.replace(' ', "_").to_uppercase(),
        Transform::LowerUnderscore => value.replace(' ', "_").to_lowercase(),
    }
}

/// "ariana grande" or "Ariana Grande" → "Ariana_Grande"
fn to_title_underscore(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
            }
        })
        .collect::<Vec<_>>()
        .join("_")
}

/// Substitute `{token}` placeholders in `pattern`. Built-in tokens
/// (`{date}`, `{YYYY}`, `{MM}`, `{DD}`) resolve automatically; everything else
/// comes from `vars`. Unrecognized tokens are left literal.
///
/// This is the raw form, used for file contents where `__` sequences (e.g.
/// Python's `__init__`, `__version__`) must be preserved exactly.
pub fn interpolate(
    pattern: &str,
    vars: &std::collections::HashMap<String, String>,
    date_format: &str,
) -> String {
    let now = Local::now();
    let date_str = now.format(date_format).to_string();
    let yyyy = now.format("%Y").to_string();
    let mm = now.format("%m").to_string();
    let dd = now.format("%d").to_string();

    let mut result = pattern.to_string();

    // Built-in tokens
    result = result.replace("{date}", &date_str);
    result = result.replace("{YYYY}", &yyyy);
    result = result.replace("{MM}", &mm);
    result = result.replace("{DD}", &dd);

    // Variable tokens
    for (key, value) in vars {
        result = result.replace(&format!("{{{}}}", key), value);
    }

    result
}

/// Interpolate a *name* — identical to `interpolate`, then collapse consecutive
/// underscores left behind by empty variables and trim leading/trailing
/// underscores. Use this for folder and file *names*, not for file contents.
pub fn interpolate_name(
    pattern: &str,
    vars: &std::collections::HashMap<String, String>,
    date_format: &str,
) -> String {
    let mut result = interpolate(pattern, vars, date_format);
    while result.contains("__") {
        result = result.replace("__", "_");
    }
    result.trim_matches('_').to_string()
}

/// Sanitize a string for use as a folder/file name component.
/// Strips characters that are problematic on Windows, macOS, or Linux.
pub fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect()
}

/// Reject file paths that would escape the project root.
/// Refuses absolute paths, paths containing `..`, Windows drive letters, and
/// leading path separators. Callers see the error at template load time (via
/// `Template::validate`) and again defensively at create time.
pub fn ensure_relative_safe_path(raw: &str) -> anyhow::Result<()> {
    if raw.is_empty() {
        anyhow::bail!("file path is empty");
    }
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        anyhow::bail!("file path '{}' must be relative (no leading slash)", raw);
    }
    // Reject Windows-style drive letters (C:/..., D:\...).
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        anyhow::bail!("file path '{}' must not contain a drive letter", raw);
    }
    for segment in normalized.split('/') {
        if segment == ".." {
            anyhow::bail!("file path '{}' must not contain '..'", raw);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_underscore() {
        assert_eq!(to_title_underscore("ariana grande"), "Ariana_Grande");
        assert_eq!(to_title_underscore("Ariana Grande"), "Ariana_Grande");
        assert_eq!(to_title_underscore("ARIANA GRANDE"), "Ariana_Grande");
        assert_eq!(to_title_underscore("single"), "Single");
    }

    #[test]
    fn test_empty_token_collapses_underscores() {
        use std::collections::HashMap;
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Project".to_string());
        vars.insert("title".to_string(), "".to_string());
        vars.insert("id".to_string(), "001".to_string());
        let result = interpolate_name("{name}_{title}_{id}", &vars, "%Y-%m-%d");
        assert_eq!(result, "Project_001");
    }

    #[test]
    fn test_interpolate_preserves_double_underscores() {
        // File content must preserve `__` sequences so Python's __version__,
        // __init__ etc. don't get mangled.
        use std::collections::HashMap;
        let vars = HashMap::new();
        let result = interpolate("__version__ = \"0.1.0\"", &vars, "%Y-%m-%d");
        assert_eq!(result, "__version__ = \"0.1.0\"");
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize_name("hello/world"), "hello_world");
        assert_eq!(sanitize_name("a:b*c"), "a_b_c");
    }

    #[test]
    fn rejects_parent_escape() {
        assert!(ensure_relative_safe_path("../evil.txt").is_err());
        assert!(ensure_relative_safe_path("a/../b.txt").is_err());
        assert!(ensure_relative_safe_path("a/b/../../c.txt").is_err());
    }

    #[test]
    fn rejects_absolute_path() {
        assert!(ensure_relative_safe_path("/etc/passwd").is_err());
        assert!(ensure_relative_safe_path("\\windows\\evil").is_err());
        assert!(ensure_relative_safe_path("C:/evil").is_err());
        assert!(ensure_relative_safe_path("D:\\evil").is_err());
    }

    #[test]
    fn accepts_normal_paths() {
        assert!(ensure_relative_safe_path("README.md").is_ok());
        assert!(ensure_relative_safe_path("src/lib.rs").is_ok());
        assert!(ensure_relative_safe_path("deeply/nested/file.txt").is_ok());
    }
}
