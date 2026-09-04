//! The prompt texts and validation messages for the app's native text prompts,
//! kept verbatim from the prompt-at-a-time flows they replace. Keeping the
//! strings here means a message change is one edit, and the pty suite's anchors
//! do not drift when a flow moves.

use crate::core::validated::ProjectFolderName;

/// `New folder name`, as the rename prompt always asked.
pub const RENAME_PROMPT: &str = "New folder name";

/// `Tag to add (e.g. draft  or  client/Acme)`.
pub const ADD_TAG_PROMPT: &str = "Tag to add (e.g. draft  or  client/Acme)";

/// `Journal note`, for one project or for every marked one.
pub fn note_prompt(count: usize) -> String {
    if count <= 1 {
        "Journal note".to_string()
    } else {
        format!("Journal note, appended to {count} projects")
    }
}

/// The word that confirms a delete. One word, the same every time: the
/// prompt names what is being deleted, so the confirmation is the decision
/// and not a typing test of a long folder name.
pub const DELETE_WORD: &str = "delete";

/// `Type delete to confirm — '…' and everything inside it will be gone`, or
/// the same over the named folders.
pub fn delete_prompt(names: &[String]) -> String {
    match names {
        [] => format!("Type {DELETE_WORD} to confirm"),
        [name] => {
            format!(
                "Type {DELETE_WORD} to confirm — '{name}' and everything inside it will be gone"
            )
        }
        many => format!(
            "Type {DELETE_WORD} to confirm — these {} projects and everything inside them will be gone: {}",
            many.len(),
            name_list(many, 6)
        ),
    }
}

/// The message a mismatched delete confirmation answers with.
pub const DELETE_MISMATCH: &str = "type delete to confirm — nothing deleted";

/// `Remove PROJECT_INFO.md from '…'? The files stay on disk; fastf just forgets
/// the project`, or the same over the named folders.
pub fn unregister_prompt(names: &[String]) -> String {
    match names {
        [name] => format!(
            "Remove PROJECT_INFO.md from '{name}'? The files stay on disk; fastf just forgets the project"
        ),
        many => format!(
            "Remove PROJECT_INFO.md from these {} projects? The files stay on disk; fastf just forgets them: {}",
            many.len(),
            name_list(many, 6)
        ),
    }
}

/// `A, B, C … and 2 more`: the first `shown` names, then the count of the
/// rest, so a confirmation over fifty marks still fits a dialog.
pub fn name_list(names: &[String], shown: usize) -> String {
    let head = names
        .iter()
        .take(shown)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    match names.len().saturating_sub(shown) {
        0 => head,
        rest => format!("{head} … and {rest} more"),
    }
}

/// The first-run question, and what skipping it says. The words are the
/// onboarding flow's own.
pub const ONBOARDING_PROMPT: &str = "Where should your projects live?";
pub const ONBOARDING_SKIPPED: &str = "Skipped — set it anytime in Settings → Base directory.";

/// `Raise the counter to (the next project will be this + 1)`, with the floor
/// it cannot go below named, because that refusal is the one people meet.
pub fn raise_counter_prompt(floor: u64) -> String {
    format!("Raise the counter to (the next project will be this + 1; it is {floor} now)")
}

/// A rename's answer must be a folder name the filesystem can actually hold.
/// The message is `ProjectFolderName::parse`'s, verbatim.
pub fn folder_name(value: &str) -> Result<(), String> {
    ProjectFolderName::parse(value)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{delete_prompt, folder_name, name_list, unregister_prompt};

    #[test]
    fn a_confirmation_names_what_it_is_about_and_stops_at_six() {
        let names: Vec<String> = (1..=8).map(|i| format!("P{i}")).collect();
        assert_eq!(name_list(&names[..2], 6), "P1, P2");
        assert_eq!(name_list(&names, 6), "P1, P2, P3, P4, P5, P6 … and 2 more");
        assert!(delete_prompt(&names[..1]).contains("'P1' and everything inside it"));
        assert!(delete_prompt(&names).contains("these 8 projects"));
        assert!(unregister_prompt(&names[..3]).contains("these 3 projects"));
        assert!(unregister_prompt(&names[..1]).contains("from 'P1'"));
    }

    #[test]
    fn an_empty_or_unusable_name_is_refused_with_the_core_message() {
        assert!(folder_name("").is_err());
        assert!(folder_name("  ").is_err());
        assert!(folder_name("....").is_err());
        assert!(
            folder_name("a/b").is_ok(),
            "separators are sanitised, not refused"
        );
        assert!(folder_name("Draft").is_ok());
    }
}
