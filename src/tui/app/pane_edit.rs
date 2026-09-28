//! `update`'s side of an edit in the pane: opening one on the row the cursor
//! is on, the keys it takes, and sending what was typed.

use super::App;
use super::pane::*;
use crate::core::body::TodoPlace;
use crate::tui::app::modal::{Modal, PickItem, PickState, Then};
use crate::tui::command::{self, CommandId, Context, Key};
use crate::tui::effect::{Action, Effect};
use crate::tui::validators;

impl App {
    /// A pane edit is open: the field has first refusal on anything typed,
    /// the registry answers the rest (`Enter`, `Esc`, `Ctrl-S`), and what
    /// neither takes goes to the field — which is how Enter in the notes is a
    /// new line: `PaneEditConfirm` is hidden there, so the text area gets it.
    pub(super) fn on_pane_edit_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::PaneEdit, key, self)
        {
            return self.run(id);
        }
        let Some(edit) = &mut self.pane_edit else {
            return Vec::new();
        };
        if edit.pending() {
            return Vec::new();
        }
        let changed = match edit {
            PaneEdit::Line { input, .. } => input.apply(&key),
            PaneEdit::Note { area, .. } => area.apply(&key),
        };
        if changed {
            edit.clear_error();
        }
        Vec::new()
    }

    /// Enter on a pane row: what the row is decides what opens — or, for a
    /// todo, what is written at once, since a toggle has nothing to type.
    pub(super) fn pane_edit_start(&mut self) -> Vec<Effect> {
        let rows = self.pane_rows();
        let Some(row) = rows.get(self.pane_cursor).cloned() else {
            return Vec::new();
        };
        let at = self.pane_cursor;
        match row {
            PaneRow::Name(_) => self.run(CommandId::Rename),
            PaneRow::AddTag => self.open_add_tag(),
            PaneRow::AddNote => self.run(CommandId::NoteInline),
            PaneRow::EarlierNotes(_) => self.run(CommandId::ShowJournal),
            PaneRow::AddTodo => self.run(CommandId::AddTodo),
            PaneRow::AddPhase => self.run(CommandId::AddPhase),
            PaneRow::Todo { ordinal, .. } => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                // The whole todo as it was read — the row holds only what fits.
                let Some(was) = self
                    .details
                    .get(&project.path)
                    .and_then(|detail| detail.todos.get(ordinal))
                    .map(|todo| todo.text.clone())
                else {
                    return Vec::new();
                };
                let effects = self.run_action(
                    "writing…",
                    Action::ToggleTodo {
                        project: Box::new(project),
                        ordinal,
                        was,
                    },
                );
                if !effects.is_empty() {
                    self.pane_pending = Some(PaneTarget::Todo(ordinal));
                }
                effects
            }
            PaneRow::Tag(tag) => {
                self.pane_edit = Some(PaneEdit::Line {
                    row: at,
                    input: crate::tui::widgets::input::LineEdit::with_text(tag.clone()),
                    target: EditTarget::Tag(tag),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            PaneRow::Variable {
                slug,
                label,
                kind: VarKind::Select(options),
                value,
            } => {
                // A select offers its options and nothing else — the picker
                // is the one shape that cannot hold anything outside them.
                let items: Vec<PickItem> = options
                    .into_iter()
                    .map(|option| PickItem {
                        detail: if option == value {
                            "current".to_string()
                        } else {
                            String::new()
                        },
                        label: option.clone(),
                        value: option,
                    })
                    .collect();
                self.modals.push(Modal::Pick(PickState::new(
                    format!(" {label} "),
                    items,
                    Then::PaneVariable(slug),
                )));
                Vec::new()
            }
            PaneRow::Variable {
                slug,
                kind: VarKind::Text,
                value,
                ..
            } => {
                self.pane_edit = Some(PaneEdit::Line {
                    row: at,
                    input: crate::tui::widgets::input::LineEdit::with_text(value),
                    target: EditTarget::Variable(slug),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            PaneRow::Note { ordinal, .. } => {
                let text = self
                    .library
                    .selected()
                    .and_then(|project| self.details.get(&project.path))
                    .and_then(|detail| detail.notes.get(ordinal))
                    .map(|note| note.text.clone())
                    .unwrap_or_default();
                self.pane_edit = Some(PaneEdit::Note {
                    row: at,
                    ordinal,
                    was: text.clone(),
                    area: Box::new(crate::tui::widgets::text_area::TextArea::with_text(&text)),
                    error: None,
                    pending: false,
                });
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Enter on the pane's line editor: what was typed goes to the project —
    /// unless nothing changed, which is a cancel, or a tag is not a tag, which
    /// is a refusal under the line.
    pub(super) fn pane_edit_confirm(&mut self) -> Vec<Effect> {
        if self.pane_edit.as_ref().is_some_and(PaneEdit::is_adding) {
            return self.add_line_enter();
        }
        if matches!(
            self.pane_edit,
            Some(PaneEdit::Line {
                target: EditTarget::NewPhase { .. },
                ..
            })
        ) {
            return self.phase_line_enter();
        }
        let Some(PaneEdit::Line { target, input, .. }) = &self.pane_edit else {
            return Vec::new();
        };
        let text = input.text().trim().to_string();
        let Some(project) = self.library.selected().cloned() else {
            self.pane_edit = None;
            return Vec::new();
        };
        let action = match target {
            EditTarget::Variable(slug) => Action::SetVariable {
                project: Box::new(project),
                slug: slug.clone(),
                value: text,
            },
            EditTarget::Todo { ordinal, was } => {
                // Unchanged is a cancel — except a todo with no words, where
                // the empty line kept is how it is removed.
                if text == *was && !text.is_empty() {
                    self.close_pane_edit();
                    return Vec::new();
                }
                Action::ReplaceTodo {
                    project: Box::new(project),
                    ordinal: *ordinal,
                    was: was.clone(),
                    text,
                }
            }
            EditTarget::NewTodo { .. } | EditTarget::NewPhase { .. } => return Vec::new(),
            EditTarget::Tag(from) => {
                if text == *from {
                    self.close_pane_edit();
                    return Vec::new();
                }
                let to = if text.is_empty() {
                    None
                } else {
                    match validators::tag(&text) {
                        Ok(tag) => Some(tag),
                        Err(error) => {
                            if let Some(edit) = &mut self.pane_edit {
                                edit.fail(error);
                            }
                            return Vec::new();
                        }
                    }
                };
                Action::ReplaceTag {
                    project: Box::new(project),
                    from: from.clone(),
                    to,
                }
            }
        };
        self.send_pane_edit(action)
    }

    /// F2 on a pane row: its text, opened in place. On a todo that is the
    /// rewording Enter never does — Enter ticks it — and emptied, the todo
    /// goes; on the name, a tag, a variable or a note it is what Enter opens.
    pub(super) fn pane_edit_text(&mut self) -> Vec<Effect> {
        let rows = self.pane_rows();
        let Some(PaneRow::Todo { ordinal, .. }) = rows.get(self.pane_cursor) else {
            return self.pane_edit_start();
        };
        let ordinal = *ordinal;
        // The whole todo as it was read — the row holds only what fits.
        let Some(was) = self
            .library
            .selected()
            .and_then(|project| self.details.get(&project.path))
            .and_then(|detail| detail.todos.get(ordinal))
            .map(|todo| todo.text.clone())
        else {
            return Vec::new();
        };
        self.pane_edit = Some(PaneEdit::Line {
            row: self.pane_cursor,
            input: crate::tui::widgets::input::LineEdit::with_text(was.clone()),
            target: EditTarget::Todo { ordinal, was },
            error: None,
            pending: false,
        });
        Vec::new()
    }

    /// Close the edit open in the pane, leaving its row as it was. The add
    /// line gives the cursor back to where it came from — the row `+` was
    /// pressed on, or the last todo it added — rather than to its own row,
    /// which goes with it; a write of it still on its way settles the cursor
    /// when it lands; and anything entered that could not be sent yet is
    /// named rather than dropped in silence.
    pub(super) fn close_pane_edit(&mut self) {
        let Some(edit) = self.pane_edit.take() else {
            return;
        };
        match edit {
            PaneEdit::Line {
                target:
                    EditTarget::NewTodo {
                        place,
                        from,
                        sending,
                        queued,
                        landed,
                        ..
                    },
                ..
            } => {
                if !sending.is_empty() {
                    self.pane_pending = Some(PaneTarget::Adding);
                }
                if !queued.is_empty() {
                    self.warn(format!("not added: {}", queued.join(", ")));
                } else if sending.is_empty()
                    && !landed
                    && let TodoPlace::Phase(name) = &place
                    && !self.has_phase(name)
                {
                    // The heading was only drawn: a phase is written with
                    // its first todo, and none was typed.
                    self.info(validators::PHASE_NOT_WRITTEN);
                }
                self.pane_anchor = Some(from);
            }
            PaneEdit::Line {
                target: EditTarget::NewPhase { from },
                ..
            } => self.pane_anchor = Some(from),
            _ => {}
        }
        self.refind_pane();
    }

    /// Whether the selected project's list, as last read, has a phase of this
    /// name — ignoring case, as the writer matches one.
    fn has_phase(&self, name: &str) -> bool {
        let wanted = name.to_lowercase();
        self.library
            .selected()
            .and_then(|project| self.details.get(&project.path))
            .is_some_and(|detail| {
                detail.todos.iter().any(|todo| {
                    todo.phase
                        .as_ref()
                        .is_some_and(|p| p.to_lowercase() == wanted)
                })
            })
    }

    /// `Ctrl-S` on the pane's note editor: the note, rewritten — or, emptied,
    /// removed. Unchanged text is a cancel.
    pub(super) fn pane_edit_save(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Note {
            area, ordinal, was, ..
        }) = &self.pane_edit
        else {
            return Vec::new();
        };
        let text = area.text();
        if text.trim() == was.trim() {
            self.pane_edit = None;
            return Vec::new();
        }
        let (ordinal, was) = (*ordinal, was.clone());
        let Some(project) = self.library.selected().cloned() else {
            self.pane_edit = None;
            return Vec::new();
        };
        self.send_pane_edit(Action::ReplaceNote {
            project: Box::new(project),
            ordinal,
            was,
            text,
        })
    }

    /// The edit stays open, marked pending, until the worker answers: an
    /// `Ok` closes it and pulses the row, an `Err` lands on it.
    pub(super) fn send_pane_edit(&mut self, action: Action) -> Vec<Effect> {
        if let Some(edit) = &mut self.pane_edit {
            edit.set_pending(true);
        }
        let effects = self.run_action("writing…", action);
        if effects.is_empty()
            && let Some(edit) = &mut self.pane_edit
        {
            // Refused before it left — something else is still running.
            edit.set_pending(false);
        }
        effects
    }
}
