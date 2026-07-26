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

/// Recover a numeric ID from a folder name by locating a `<prefix><digits>`
/// token (e.g. `parse_id_token("2026-04-19_Foo_ID0030", "ID")` → `Some(30)`).
///
/// Zero-padding is ignored — the value is what matters, so `ID007` and `ID0030`
/// yield `7` and `30`. Scans left-to-right and returns the first occurrence of
/// the prefix that is immediately followed by at least one ASCII digit. Returns
/// `None` when the prefix is empty or no digit-bearing token is found.
///
/// This is the **only** place folder names still influence identity — used by
/// `fastf register` to seed a metadata-less folder's ID. Discovery never calls it.
pub fn parse_id_token(name: &str, prefix: &str) -> Option<u64> {
    if prefix.is_empty() {
        return None;
    }
    let mut search_start = 0;
    while let Some(rel) = name[search_start..].find(prefix) {
        let after = search_start + rel + prefix.len();
        let digits: String = name[after..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return digits.parse::<u64>().ok();
        }
        // Prefix without trailing digits — keep scanning past this occurrence.
        search_start = after;
        if search_start >= name.len() {
            break;
        }
    }
    None
}

/// Extract the numeric value from a formatted ID string by reading its trailing
/// run of ASCII digits (e.g. `id_value("ID0030")` → `Some(30)`). Prefix-agnostic
/// so it works across templates with different ID prefixes — used to compute the
/// counter self-heal floor (`library::max_id`). Returns `None` when the string
/// has no trailing digits.
pub fn id_value(id: &str) -> Option<u64> {
    let bytes = id.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == bytes.len() {
        return None;
    }
    id[i..].parse::<u64>().ok()
}

/// MS-DOS device names, still reserved by Win32 today. A file or folder called
/// any of these — with or without an extension — cannot be created, and the
/// error Windows returns says nothing useful about why.
const RESERVED_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

fn is_reserved_device_name(stem: &str) -> bool {
    RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Sanitize a string for use as a folder/file name component.
///
/// Beyond the outright-illegal characters, this covers the two Windows
/// behaviours that corrupt a name silently rather than failing loudly:
///
/// - **Trailing dots and spaces** are stripped by the filesystem itself, so
///   `"Draft ."` lands on disk as `"Draft"`. fastf would then have recorded a
///   folder name that does not match reality, and every later lookup, rename or
///   move against it would miss.
/// - **Reserved device names** (`CON`, `NUL`, `COM1`, …) cannot be used at all,
///   extension or not, so an underscore is appended.
///
/// Applied on every platform, not only Windows: a project created on Linux and
/// later opened on Windows needs a name that works in both places, and a
/// template should not produce different folder names depending on where it runs.
pub fn sanitize_name(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            // Illegal in names on Windows, unreadable everywhere else.
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();

    // Doing the trim here keeps the recorded name and the name on disk
    // identical. It also reduces ".." to the empty string, which callers such as
    // `rename_project` already reject.
    let trimmed = replaced.trim_end_matches([' ', '.']);

    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_reserved_device_name(stem) {
        // `CON` → `CON_`, `CON.txt` → `CON_.txt`.
        return format!("{}_{}", stem, &trimmed[stem.len()..]);
    }
    trimmed.to_string()
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
    fn parse_id_token_ignores_padding() {
        assert_eq!(parse_id_token("2026-04-19_Foo_ID0030", "ID"), Some(30));
        assert_eq!(parse_id_token("ID007_bar", "ID"), Some(7));
        assert_eq!(parse_id_token("proj_ID0068_final", "ID"), Some(68));
    }

    #[test]
    fn parse_id_token_none_when_absent() {
        assert_eq!(parse_id_token("no_id_here", "ID"), None);
        // Prefix present but no trailing digits.
        assert_eq!(parse_id_token("this_is_IDentity", "ID"), None);
        assert_eq!(parse_id_token("anything", ""), None);
    }

    #[test]
    fn parse_id_token_skips_prefix_without_digits() {
        // First "ID" has no digits; the second one does.
        assert_eq!(parse_id_token("IDeas_then_ID0042", "ID"), Some(42));
    }

    #[test]
    fn id_value_reads_trailing_digits() {
        assert_eq!(id_value("ID0030"), Some(30));
        assert_eq!(id_value("ID007"), Some(7));
        assert_eq!(id_value("T042"), Some(42));
        assert_eq!(id_value("no-digits"), None);
        assert_eq!(id_value(""), None);
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize_name("hello/world"), "hello_world");
        assert_eq!(sanitize_name("a:b*c"), "a_b_c");
        // Ordinary names are untouched — including interior dots.
        assert_eq!(sanitize_name("Release v1.2.3"), "Release v1.2.3");
        assert_eq!(sanitize_name("Ariana_Grande"), "Ariana_Grande");
    }

    #[test]
    fn sanitize_defuses_reserved_device_names() {
        // Windows cannot create any of these, with or without an extension.
        assert_eq!(sanitize_name("CON"), "CON_");
        assert_eq!(sanitize_name("nul"), "nul_");
        assert_eq!(sanitize_name("COM1"), "COM1_");
        assert_eq!(sanitize_name("LPT9"), "LPT9_");
        assert_eq!(sanitize_name("CON.txt"), "CON_.txt");
        assert_eq!(sanitize_name("aux.tar.gz"), "aux_.tar.gz");
        // Names that merely start with a reserved word are fine.
        assert_eq!(sanitize_name("CONTENT"), "CONTENT");
        assert_eq!(sanitize_name("COM10"), "COM10");
        assert_eq!(sanitize_name("console"), "console");
    }

    #[test]
    fn sanitize_strips_trailing_dots_and_spaces() {
        // Windows drops these silently, so the name fastf records would not
        // match the folder that actually appears on disk.
        assert_eq!(sanitize_name("Draft."), "Draft");
        assert_eq!(sanitize_name("Draft "), "Draft");
        assert_eq!(sanitize_name("Draft . . "), "Draft");
        // Leading whitespace is legal and left alone.
        assert_eq!(sanitize_name(" Draft"), " Draft");
        // Dot-only names collapse to empty; callers reject that explicitly.
        assert_eq!(sanitize_name(".."), "");
        assert_eq!(sanitize_name("..."), "");
    }

    #[test]
    fn sanitize_removes_control_characters() {
        assert_eq!(sanitize_name("bad\u{0}name"), "bad_name");
        assert_eq!(sanitize_name("tab\tname"), "tab_name");
        assert_eq!(sanitize_name("nl\nname"), "nl_name");
    }

    #[test]
    fn sanitize_is_idempotent() {
        // plan() sanitizes each variable and then the assembled folder name, so
        // applying it twice must not keep changing the result.
        for input in [
            "CON",
            "Draft.",
            "a:b*c",
            "..",
            "Release v1.2.3",
            "CON.txt",
            "tab\tname",
        ] {
            let once = sanitize_name(input);
            assert_eq!(sanitize_name(&once), once, "not idempotent for {input:?}");
        }
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
