//! The single-project actions as native dialogs: the action-menu modal,
//! the text prompt, the yes/no confirm, the multi-pick for tags, and the pure
//! lookups that feed them. The verbs themselves are declared once in
//! `command.rs`; this module holds the modal state a verb opens and the lists
//! its pickers show, so `update` stays a function of data and not of closures.

use std::path::PathBuf;

use super::App;
use crate::tui::command::{Availability, CommandId, Context};
use crate::tui::widgets::input::LineEdit;

/// The picker's "type your own" entry.
pub const NEW_TAG: &str = "New tag…";

/// The action menu: the selected project's verbs, chosen from the registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionsState {
    pub selected: usize,
    pub offset: usize,
}

impl ActionsState {
    pub fn step(&mut self, len: usize, delta: isize) {
        self.selected =
            crate::tui::widgets::nav::step(Some(self.selected), len, delta).unwrap_or(0);
    }

    pub fn clamp_viewport(&mut self, len: usize, rows: usize) {
        self.offset =
            crate::tui::widgets::nav::viewport_offset(self.offset, Some(self.selected), len, rows);
    }
}

/// What a text prompt's answer does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextThen {
    /// Rename the project the prompt named, carried by path.
    ///
    /// **The dialog carries its target rather than re-reading the selection.**
    /// The prompt text is built once from the row under the cursor, and the
    /// action used to be built from whatever was selected when Enter landed —
    /// so a discovery arriving under an open dialog, which moves the cursor
    /// when the named row is no longer in the snapshot, could point a
    /// destructive verb at a different project from the one on screen.
    Rename(std::path::PathBuf),
    AddTag,
    /// A todo for the selected project, from the pane's add row.
    AddTodo,
    /// Type the word `delete` to confirm; nothing else deletes. The prompt
    /// names the folder — or the folders, over marks — so what is being
    /// confirmed is on screen, and the word is the same every time.
    ///
    /// Carries the single project's path for the same reason `Rename` does;
    /// a batch delete goes by the marks, which are kept by path already.
    Delete(std::path::PathBuf),
    /// Raise the global ID counter to the number typed.
    RaiseCounter,
    /// The folder to copy into. Refused by the engine rather than here, so the
    /// command line and the app say the same words about the same rule.
    CopyTo,
}

/// The quick note: a few lines typed where you are. Enter saves, Alt-Enter
/// (or a pasted newline) breaks a line — so a pasted paragraph is one note,
/// and never a run of keystrokes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteState {
    pub area: crate::tui::widgets::text_area::TextArea,
    /// How many projects the note goes to: the marks, or the one selected.
    pub count: usize,
}

impl NoteState {
    pub fn new(count: usize) -> Self {
        Self {
            area: crate::tui::widgets::text_area::TextArea::new(),
            count,
        }
    }
}

/// A single-line prompt drawn over the dashboard. Esc cancels; Enter submits
/// the text and `update` interprets `then`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPrompt {
    pub title: String,
    pub input: LineEdit,
    /// A validation message shown under the line; the text stays for editing.
    pub error: Option<String>,
    pub then: TextThen,
}

impl TextPrompt {
    pub fn new(title: impl Into<String>, then: TextThen) -> Self {
        Self {
            title: title.into(),
            input: LineEdit::new(),
            error: None,
            then,
        }
    }
}

/// What a yes/no confirm answers. A bare `y`/`n` answers without Enter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmThen {
    /// Unregister the project the question named, carried by path — see
    /// [`TextThen::Rename`].
    Unregister(std::path::PathBuf),
    /// Delete the named template and everything bundled with it.
    DeleteTemplate(String),
    /// Leave the builder, throwing away a template that has been worked on.
    ///
    /// `then_quit` is set when the gesture was a quit rather than a close, so
    /// answering the question does what was asked instead of stopping one
    /// level short.
    DiscardTemplate {
        then_quit: Option<crate::tui::effect::Exit>,
    },
    /// Delete every marked project (the marks are the batch).
    DeleteBatch,
    /// Unregister every marked project (the marks are the batch).
    UnregisterBatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub prompt: String,
    pub then: ConfirmThen,
}

/// What a multi-pick answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiThen {
    RemoveTags,
}

/// A list where Space toggles and Enter confirms the picked set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiPick {
    pub title: String,
    pub items: Vec<String>,
    pub picked: Vec<bool>,
    pub selected: usize,
    pub then: MultiThen,
}

impl MultiPick {
    pub fn new(title: impl Into<String>, items: Vec<String>, then: MultiThen) -> Self {
        Self {
            title: title.into(),
            picked: vec![false; items.len()],
            items,
            selected: 0,
            then,
        }
    }

    pub fn chosen(&self) -> Vec<String> {
        self.items
            .iter()
            .zip(&self.picked)
            .filter(|(_, picked)| **picked)
            .map(|(item, _)| item.clone())
            .collect()
    }
}

/// The commands the action menu lists: every project verb that fires in the
/// `Actions` context and is not hidden, with its availability, in registry
/// order. Hidden means the verb makes no sense here (Move with no other
/// mounted base), so it is not listed at all. The menu's own navigation —
/// Enter, the arrows, Esc — fires there too and is not a row.
pub fn action_entries(app: &App) -> Vec<(CommandId, Availability)> {
    crate::tui::command::COMMANDS
        .iter()
        .filter(|command| {
            command.contexts.contains(&Context::Actions)
                && command.category == crate::tui::command::Category::Project
        })
        .map(|command| (command.id, (command.available)(app)))
        .filter(|(_, availability)| *availability != Availability::Hidden)
        .collect()
}

/// The mounted bases the selected project could move to, in summary order.
pub fn move_targets(app: &App) -> Vec<PathBuf> {
    let Some(project) = app.library.selected() else {
        return Vec::new();
    };
    app.summary
        .as_ref()
        .map(|summary| {
            summary
                .bases
                .iter()
                .filter(|base| base.probe.usable() && base.path != project.base)
                .map(|base| base.path.clone())
                .collect()
        })
        .unwrap_or_default()
}
