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
                Some(first) => {
                    first.to_uppercase().to_string() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<_>>()
        .join("_")
}

/// Interpolate a pattern string, replacing {token} with values from the map.
/// Built-in tokens ({date}, {YYYY}, {MM}, {DD}) are resolved automatically.
/// Variable tokens are looked up in `vars`.
pub fn interpolate(
    pattern: &str,
    vars: &std::collections::HashMap<String, String>,
    date_format: &str,
) -> String {
    let now = Local::now();
    let date_str = now.format(date_format).to_string();
    let yyyy = now.format("%Y").to_string();
    let mm   = now.format("%m").to_string();
    let dd   = now.format("%d").to_string();

    let mut result = pattern.to_string();

    // Built-in tokens
    result = result.replace("{date}", &date_str);
    result = result.replace("{YYYY}", &yyyy);
    result = result.replace("{MM}",   &mm);
    result = result.replace("{DD}",   &dd);

    // Variable tokens
    for (key, value) in vars {
        result = result.replace(&format!("{{{}}}", key), value);
    }

    result
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
    fn test_sanitize() {
        assert_eq!(sanitize_name("hello/world"), "hello_world");
        assert_eq!(sanitize_name("a:b*c"), "a_b_c");
    }
}
