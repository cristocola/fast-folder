//! The prompt texts and validation messages for the app's native text prompts,
//! kept verbatim from the bridged dialoguer flows they replace (`tui::actions`
//! and `tui::prompt`). Keeping the strings here means a message change is one
//! edit, and the pty suite's anchors do not drift when a flow moves native.

use crate::core::validated::ProjectFolderName;

/// `New folder name`, as the rename prompt always asked.
pub const RENAME_PROMPT: &str = "New folder name";

/// `Tag to add (e.g. draft  or  client/Acme)`.
pub const ADD_TAG_PROMPT: &str = "Tag to add (e.g. draft  or  client/Acme)";

/// `Journal note`.
pub const NOTE_PROMPT: &str = "Journal note";

/// `Type the folder name '…' to confirm`.
pub fn delete_prompt(name: &str) -> String {
    format!("Type the folder name '{name}' to confirm")
}

/// The message a mismatched delete confirmation answers with.
pub const DELETE_MISMATCH: &str = "name did not match — nothing deleted";

/// `Remove PROJECT_INFO.md from '…'? The files stay on disk; fastf just forgets
/// the project`.
pub fn unregister_prompt(name: &str) -> String {
    format!(
        "Remove PROJECT_INFO.md from '{name}'? The files stay on disk; fastf just forgets the project"
    )
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
    use super::folder_name;

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
