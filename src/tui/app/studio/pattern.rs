//! The naming pattern as the builder reads it: the tokens it holds, what is
//! wrong with it, and the folder name it would make.

use super::*;

/// The built-in tokens every naming pattern understands.
pub const BUILT_IN_TOKENS: [&str; 5] = ["date", "YYYY", "MM", "DD", "id"];

/// The `{token}` names a string mentions, in the order they appear.
///
/// A plain scan rather than a regex: a `{` with no `}` after it is not a
/// token, and neither is an empty `{}`.
pub fn tokens_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = &after[..close];
        if !name.is_empty() && !found.iter().any(|seen| seen == name) {
            found.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    found
}

/// What is wrong with a naming pattern, in one sentence, or `None`.
///
/// Two mistakes, and neither is something `Template::validate` can refuse —
/// both produce a template that loads and saves perfectly and then names every
/// project wrongly, which is why they have to be said here:
///
/// - **A token no variable answers.** `{clientname}` typed for a variable
///   called `client_name` is left in the folder name verbatim, braces and all.
/// - **A variable the pattern never uses.** This is the one that costs a
///   first template: declare `artist` and `title`, leave the suggested
///   `{date}_{id}` alone, and every project is called `2026-09-07_ID0001`. The
///   questions are asked, the answers are recorded in `PROJECT_INFO.md`, and
///   the folder names are identical. Nothing says so until the first create.
pub fn pattern_warning(template: &Template) -> Option<String> {
    let declared: Vec<&str> = template.variables.iter().map(|v| v.slug.as_str()).collect();
    pattern_warning_of(&template.naming_pattern, &declared)
}

/// The same check over a pattern being typed, before it has been committed to
/// the scratch template — so the form can say it on the keystroke that caused
/// it rather than after the section is closed.
pub fn pattern_warning_of(pattern: &str, declared: &[&str]) -> Option<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return None;
    }
    let used = tokens_in(pattern);

    let unknown: Vec<String> = used
        .iter()
        .filter(|token| {
            !BUILT_IN_TOKENS.contains(&token.as_str()) && !declared.contains(&token.as_str())
        })
        .map(|token| format!("{{{token}}}"))
        .collect();
    if !unknown.is_empty() {
        return Some(format!(
            "{} matches no variable — it stays in the folder name as written",
            unknown.join(" ")
        ));
    }

    let unused: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|slug| !used.iter().any(|token| token == slug))
        .collect();
    if !unused.is_empty() {
        return Some(format!(
            "{} not in the pattern — every project gets the same folder name",
            unused
                .iter()
                .map(|slug| format!("{{{slug}}}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    None
}

/// The folder name this template would produce, with an illustrative answer
/// for every question it asks.
///
/// **Pure, and deliberately not "now".** `naming::RenderContext`'s four fields
/// are the whole of its state, so a sample context is a struct literal — no
/// clock, which is what lets `update` and a snapshot test both call this. The
/// date is a fixed 31 January because it is the one day of the year where
/// `{YYYY}`, `{MM}` and `{DD}` are three visibly different numbers, so a person
/// reading the example can tell which token produced which digits.
///
/// An answer is the variable's own default when it has one, else its slug in
/// the shape its transform would give it — so the example shows the transform
/// working rather than describing it.
pub fn sample_folder_name(template: &Template) -> String {
    let ctx = crate::core::naming::RenderContext {
        date: "2026-01-31".to_string(),
        yyyy: "2026".to_string(),
        mm: "01".to_string(),
        dd: "31".to_string(),
        // The sample renders `{id}` from the variable map below, the way the
        // sample renders every other token: one place, one answer.
        id: None,
    };
    let mut vars: std::collections::HashMap<String, String> = template
        .variables
        .iter()
        .map(|v| {
            let raw = if v.default.trim().is_empty() {
                sample_answer(&v.slug)
            } else {
                v.default.clone()
            };
            (
                v.slug.clone(),
                crate::core::template::apply_transform(&raw, &v.transform),
            )
        })
        .collect();
    vars.insert(
        "id".to_string(),
        Counters::format_id(&template.id.prefix, template.id.digits, 1),
    );
    crate::core::naming::interpolate_name_with(&template.naming_pattern, &vars, &ctx)
}

/// A plausible answer to a question nobody has answered: the slug as words.
/// `client_name` reads back as `Client Name`, which is what a transform is
/// then visibly applied to.
fn sample_answer(slug: &str) -> String {
    slug.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `{token}`s a template understands: its own variables, then the built-ins.
pub fn tokens(template: &Template) -> Vec<String> {
    template
        .variables
        .iter()
        .map(|v| format!("{{{}}}", v.slug))
        .chain(
            ["date", "YYYY", "MM", "DD", "id"]
                .iter()
                .map(|t| format!("{{{t}}}")),
        )
        .collect()
}

/// Which of a template's tokens a body actually uses — the check that catches
/// `{clientname}` typed for a variable called `client_name`, before saving.
pub fn tokens_used(body: &str, template: &Template) -> Vec<String> {
    tokens(template)
        .into_iter()
        .filter(|token| body.contains(token.as_str()))
        .collect()
}
