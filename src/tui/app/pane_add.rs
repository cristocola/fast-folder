//! Adding to the pane: a todo where it will land, a phase with its first todo,
//! a paste of several, and the answers that come back.

use super::App;
use super::pane::*;
use crate::core::body::TodoPlace;
use crate::tui::app::modal::Modal;
use crate::tui::command::{self, CommandId};
use crate::tui::effect::{Action, Effect};
use crate::tui::validators;

impl App {
    /// `+` in the pane: one more of whatever the cursor is among — a tag, a
    /// note, or a todo, typed on a line where it will land: in the phase the
    /// cursor is in, with the loose tasks when it is on one, else at the end.
    pub(super) fn pane_add(&mut self) -> Vec<Effect> {
        let rows = self.pane_rows();
        // One more of what the cursor is on: on "add a phase", a phase.
        if rows.get(self.pane_cursor) == Some(&PaneRow::AddPhase) {
            return self.run(CommandId::AddPhase);
        }
        match section_at(&rows, self.pane_cursor) {
            PaneSection::Tags => self.open_add_tag(),
            PaneSection::Notes => self.run(CommandId::NoteInline),
            _ => {
                // A todo goes to one list: with marks, which one is a guess,
                // and the verb says so rather than choosing.
                if let command::Availability::Disabled(reason) =
                    (command::find(CommandId::AddTodo).available)(self)
                {
                    self.warn(format!("Add a todo: {reason}"));
                    return Vec::new();
                }
                let place = place_at(&rows, self.pane_cursor);
                self.start_adding(place)
            }
        }
    }

    /// Open the line a new todo is typed on, in the pane, with the focus
    /// there. Falls back to the prompt when the pane cannot show the list —
    /// switched off, or its record not read yet.
    pub(super) fn start_adding(&mut self, place: TodoPlace) -> Vec<Effect> {
        let read = self
            .library
            .selected()
            .and_then(|p| self.details.get(&p.path));
        let Some(detail) = read else {
            return self.prompt_for_a_todo(place);
        };
        if !self.pane_live() || self.screen != super::Screen::Library {
            return self.prompt_for_a_todo(place);
        }
        // Drawn in from a heading when the todo will sit under one — at the
        // end of a list whose last run is a phase, too.
        let phased = match &place {
            TodoPlace::Phase(_) => true,
            TodoPlace::Loose => false,
            TodoPlace::End => !labels_of(detail).is_empty(),
        };
        // Closing the line comes back to the row `+` was pressed on — or, from
        // the list, to the row that adds a todo.
        let from = if self.focus == super::Focus::Detail {
            target_at(&self.pane_rows(), self.pane_cursor).unwrap_or(PaneTarget::AddTodo)
        } else {
            PaneTarget::AddTodo
        };
        self.set_focus(super::Focus::Detail);
        self.close_pane_edit();
        self.pane_edit = Some(PaneEdit::Line {
            row: 0,
            input: crate::tui::widgets::input::LineEdit::new(),
            target: EditTarget::NewTodo {
                place,
                phased,
                from,
                sending: Vec::new(),
                queued: Vec::new(),
                closing: false,
                landed: false,
            },
            error: None,
            pending: false,
        });
        self.pane_anchor = Some(PaneTarget::Adding);
        self.refind_pane();
        Vec::new()
    }

    /// The one-line prompt for a todo — what adding is where the pane cannot
    /// show the list it goes into.
    pub(super) fn prompt_for_a_todo(&mut self, place: TodoPlace) -> Vec<Effect> {
        let title = match &place {
            TodoPlace::Phase(name) => validators::add_todo_in_prompt(name),
            TodoPlace::End | TodoPlace::Loose => validators::ADD_TODO_PROMPT.to_string(),
        };
        self.modals
            .push(Modal::TextPrompt(super::actions::TextPrompt::new(
                title,
                super::actions::TextThen::AddTodo(place),
            )));
        Vec::new()
    }

    /// A new phase: its name typed on a line where its heading will be
    /// written, then its todos on the line under it. Falls back to a prompt
    /// for the name when the pane cannot show the list.
    pub(super) fn start_phase(&mut self) -> Vec<Effect> {
        let read = self
            .library
            .selected()
            .is_some_and(|p| self.details.contains_key(&p.path));
        if !read || !self.pane_live() || self.screen != super::Screen::Library {
            self.modals
                .push(Modal::TextPrompt(super::actions::TextPrompt::new(
                    validators::ADD_PHASE_PROMPT,
                    super::actions::TextThen::AddPhase,
                )));
            return Vec::new();
        }
        let from = if self.focus == super::Focus::Detail {
            target_at(&self.pane_rows(), self.pane_cursor).unwrap_or(PaneTarget::AddPhase)
        } else {
            PaneTarget::AddPhase
        };
        self.set_focus(super::Focus::Detail);
        self.close_pane_edit();
        self.pane_edit = Some(PaneEdit::Line {
            row: 0,
            input: crate::tui::widgets::input::LineEdit::new(),
            target: EditTarget::NewPhase { from },
            error: None,
            pending: false,
        });
        self.pane_anchor = Some(PaneTarget::Adding);
        self.refind_pane();
        Vec::new()
    }

    /// Enter on the line a phase is named on: the line becomes the one its
    /// todos are typed on, under the heading. Nothing is written yet. An
    /// empty Enter is a cancel.
    pub(super) fn phase_line_enter(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Line {
            input,
            error,
            target: EditTarget::NewPhase { from },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        if input.text().trim().is_empty() {
            self.close_pane_edit();
            return Vec::new();
        }
        let Some(name) = crate::core::body::phase_label(input.text()) else {
            *error = Some(validators::PHASE_NAMELESS.to_string());
            return Vec::new();
        };
        let from = from.clone();
        self.pane_edit = Some(PaneEdit::Line {
            row: 0,
            input: crate::tui::widgets::input::LineEdit::new(),
            target: EditTarget::NewTodo {
                place: TodoPlace::Phase(name),
                phased: true,
                from,
                sending: Vec::new(),
                queued: Vec::new(),
                closing: false,
                landed: false,
            },
            error: None,
            pending: false,
        });
        self.pane_anchor = Some(PaneTarget::Adding);
        self.refind_pane();
        Vec::new()
    }

    /// Enter on the add line: what was typed goes to the file, and the line
    /// empties for the next — at once, so the keys that follow land in it
    /// while the write is on its way. Entered while one is, it waits its turn.
    /// An empty Enter is done: now, or once what is waiting has landed.
    pub(super) fn add_line_enter(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Line {
            input,
            error,
            target:
                EditTarget::NewTodo {
                    sending,
                    queued,
                    closing,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        let text = input.text().trim().to_string();
        if text.is_empty() {
            if sending.is_empty() && queued.is_empty() {
                self.close_pane_edit();
            } else {
                *closing = true;
            }
            return Vec::new();
        }
        *error = None;
        input.set_text("");
        if !sending.is_empty() {
            queued.push(text);
            return Vec::new();
        }
        self.send_adds(vec![text])
    }

    /// A paste onto the add line goes in at the caret, as into any field; a
    /// paste of several lines makes that many todos, in one write, the list
    /// markers a checklist is copied with taken off — and what was typed on
    /// the line before it is the first of them. A single line is the field's,
    /// markers taken off when the line was empty. `None` when this is not the
    /// add line's to take.
    pub(super) fn paste_todos(&mut self, text: &str) -> Option<Vec<Effect>> {
        let Some(PaneEdit::Line {
            input,
            error,
            target: EditTarget::NewTodo {
                sending, queued, ..
            },
            ..
        }) = &mut self.pane_edit
        else {
            return None;
        };
        if !text.contains('\n') {
            if input.is_empty() {
                input.set_text(todo_text_of(text));
                *error = None;
                return Some(Vec::new());
            }
            return None;
        }
        let typed = input.text().to_string();
        let at = typed
            .char_indices()
            .nth(input.cursor())
            .map_or(typed.len(), |(byte, _)| byte);
        let whole = format!("{}{text}{}", &typed[..at], &typed[at..]);
        let texts: Vec<String> = whole
            .lines()
            .map(todo_text_of)
            .filter(|line| !line.is_empty())
            .collect();
        input.set_text("");
        *error = None;
        if texts.is_empty() {
            return Some(Vec::new());
        }
        if !sending.is_empty() {
            queued.extend(texts);
            return Some(Vec::new());
        }
        Some(self.send_adds(texts))
    }

    /// Send `texts` from the add line. Refused before it left — something
    /// else is being written — they wait their turn instead, and go when that
    /// has landed (`flush_adds`).
    fn send_adds(&mut self, texts: Vec<String>) -> Vec<Effect> {
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        let Some(PaneEdit::Line {
            target: EditTarget::NewTodo { place, sending, .. },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        let place = place.clone();
        sending.clone_from(&texts);
        let effects = self.run_action(
            "writing…",
            Action::AddTodos {
                project: Box::new(project),
                texts: texts.clone(),
                place,
            },
        );
        if effects.is_empty()
            && let Some(PaneEdit::Line {
                target:
                    EditTarget::NewTodo {
                        sending, queued, ..
                    },
                ..
            }) = &mut self.pane_edit
        {
            sending.clear();
            let later = std::mem::take(queued);
            *queued = texts;
            queued.extend(later);
        }
        effects
    }

    /// After a write has landed: send what was entered while it was on its
    /// way, or close the line if that was asked for meanwhile.
    pub(super) fn flush_adds(&mut self) -> Vec<Effect> {
        let Some(PaneEdit::Line {
            target:
                EditTarget::NewTodo {
                    sending,
                    queued,
                    closing,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        else {
            return Vec::new();
        };
        if !sending.is_empty() || self.busy.is_some() {
            return Vec::new();
        }
        if !queued.is_empty() {
            let texts = std::mem::take(queued);
            return self.send_adds(texts);
        }
        if *closing {
            self.close_pane_edit();
        }
        Vec::new()
    }

    /// The add line's write has landed at `ordinal` (the last todo written):
    /// the line stays open, and closing it now comes back to that todo.
    pub(super) fn adds_landed(&mut self, ordinal: Option<usize>) {
        if let Some(PaneEdit::Line {
            target:
                EditTarget::NewTodo {
                    sending,
                    from,
                    landed,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        {
            sending.clear();
            *landed = true;
            if let Some(ordinal) = ordinal {
                *from = PaneTarget::Todo(ordinal);
            }
        }
    }

    /// The add line's write was refused. What was sent comes back to the
    /// field when nothing was typed after it; otherwise the refusal names what
    /// was not written, so nothing entered is lost without a word. What was
    /// waiting behind it is not sent: the refusal is the person's to read.
    pub(super) fn adds_refused(&mut self, message: String) {
        let Some(PaneEdit::Line {
            input,
            error,
            target:
                EditTarget::NewTodo {
                    sending,
                    queued,
                    closing,
                    ..
                },
            ..
        }) = &mut self.pane_edit
        else {
            return;
        };
        let unsent: Vec<String> = sending.drain(..).chain(queued.drain(..)).collect();
        *closing = false;
        if unsent.len() == 1 && input.is_empty() {
            input.set_text(unsent[0].clone());
            *error = Some(message);
        } else {
            *error = Some(format!("{message} — not added: {}", unsent.join(", ")));
        }
    }
}
