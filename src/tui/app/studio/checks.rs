//! What a field of the builder may hold, as the sentence that refuses it.

use super::*;

// ---------------------------------------------------------------------------
// Validation `update` can perform — no disk, no template load
// ---------------------------------------------------------------------------

pub fn check_slug(value: &str) -> Result<(), String> {
    crate::core::validated::TemplateSlug::parse(value.trim())
        .map(|_| ())
        .map_err(|error| format!("{error:#}"))
}

pub fn check_naming_pattern(value: &str) -> Result<(), String> {
    if value.trim_start().starts_with('.') {
        // Discovery skips dot-prefixed directories, so such a pattern names
        // projects fastf cannot see. Same rule as `Template::validate`.
        Err(
            "a naming pattern may not start with '.' — fastf would not see the projects it names"
                .to_string(),
        )
    } else if value.trim().is_empty() {
        Err("a naming pattern is required".to_string())
    } else {
        Ok(())
    }
}

pub fn check_digits(value: &str) -> Result<(), String> {
    match value.trim().parse::<usize>() {
        Ok(n) if (1..=MAX_ID_DIGITS).contains(&n) => Ok(()),
        Ok(_) => Err(format!("expected a number between 1 and {MAX_ID_DIGITS}")),
        Err(_) => Err(format!("expected a number, got '{}'", value.trim())),
    }
}

/// The first field of `form` a rule refuses, as `(key, message)`.
pub fn check_metadata(form: &Form) -> Option<(&'static str, String)> {
    if form.value("name").trim().is_empty() {
        return Some(("name", "a template needs a name".to_string()));
    }
    if let Err(error) = check_slug(&form.value("slug")) {
        return Some(("slug", error));
    }
    if let Err(error) = check_naming_pattern(&form.value("naming_pattern")) {
        return Some(("naming_pattern", error));
    }
    None
}

pub fn check_id(form: &Form) -> Option<(&'static str, String)> {
    if form.value("prefix").trim().is_empty() {
        return Some((
            "prefix",
            "an empty prefix matches any trailing digits — register could not recover an ID"
                .to_string(),
        ));
    }
    if let Err(error) = check_digits(&form.value("digits")) {
        return Some(("digits", error));
    }
    None
}
