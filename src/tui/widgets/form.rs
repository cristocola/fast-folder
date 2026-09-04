//! A form: labelled fields on one screen, moved through with Tab and the
//! arrows, edited in place, submitted with Enter.
//!
//! It replaces a run of one-at-a-time prompts, and the difference is the point. A
//! sequence of prompts can only ask one thing at a time, so an answer given
//! three questions ago is invisible and unreachable, and a value rejected at
//! the end takes every earlier answer with it. A form shows every answer at
//! once, lets any of them be corrected without retyping the rest, and puts a
//! rejection on the field that caused it.
//!
//! **A field never validates against a disk here.** `update` performs no I/O,
//! so a path that must exist is checked by the worker that builds the preview,
//! and its answer comes back as an [`Field::error`] on the field that was
//! wrong (`Form::fail`). The typed text stays where it was.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Position, Rect};
use ratatui::text::Span;
use ratatui::widgets::{List, ListItem, ListState, StatefulWidget};

use crate::tui::command::Key;
use crate::tui::theme::Theme;
use crate::tui::view::{fit, pad};
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;

/// What a field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text(LineEdit),
    Toggle(bool),
    /// One of a fixed list. `←`/`→` step through it; Space opens a picker over
    /// the same options, which is what makes a twenty-template list usable.
    Choice {
        options: Vec<String>,
        selected: usize,
    },
}

/// One row of a form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Stable name, so a worker can say which field its refusal belongs to and
    /// a caller can read a value back without counting rows.
    pub key: String,
    pub label: String,
    /// The dimmed line the footer shows while this field has the cursor.
    pub hint: String,
    pub kind: FieldKind,
    /// A refusal, shown under the form and beside the field.
    pub error: Option<String>,
    /// Hidden fields are not drawn and never take the cursor — a question that
    /// does not apply to the answers already given (bulk registration never
    /// renames) is absent rather than greyed, because there is nothing to say
    /// about it.
    pub hidden: bool,
    /// Whether a person has changed this field. A value the form suggests from
    /// another answer keeps following it until then, and never afterwards.
    pub touched: bool,
}

impl Field {
    pub fn text(key: &str, label: &str, hint: &str, initial: impl Into<String>) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            hint: hint.to_string(),
            kind: FieldKind::Text(LineEdit::with_text(initial.into())),
            error: None,
            hidden: false,
            touched: false,
        }
    }

    pub fn toggle(key: &str, label: &str, hint: &str, value: bool) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            hint: hint.to_string(),
            kind: FieldKind::Toggle(value),
            error: None,
            hidden: false,
            touched: false,
        }
    }

    pub fn choice(key: &str, label: &str, hint: &str, options: Vec<String>, at: usize) -> Self {
        let selected = at.min(options.len().saturating_sub(1));
        Self {
            key: key.to_string(),
            label: label.to_string(),
            hint: hint.to_string(),
            kind: FieldKind::Choice { options, selected },
            error: None,
            hidden: false,
            touched: false,
        }
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// The value as text: a line's contents, `yes`/`no`, or the chosen option.
    pub fn value(&self) -> String {
        match &self.kind {
            FieldKind::Text(edit) => edit.text().to_string(),
            FieldKind::Toggle(on) => if *on { "yes" } else { "no" }.to_string(),
            FieldKind::Choice { options, selected } => {
                options.get(*selected).cloned().unwrap_or_default()
            }
        }
    }

    pub fn is_on(&self) -> bool {
        matches!(self.kind, FieldKind::Toggle(true))
    }

    /// Put text in a text field. For a value that is computed rather than
    /// typed — a default recovered when a template changes.
    pub fn set_text(&mut self, value: impl Into<String>) {
        if let FieldKind::Text(edit) = &mut self.kind {
            edit.set_text(value);
        }
    }

    /// Bracketed paste into a text field; anything else ignores it.
    pub fn paste(&mut self, text: &str) {
        if let FieldKind::Text(edit) = &mut self.kind {
            edit.paste(text);
            self.error = None;
        }
    }

    /// Point a choice at `value`, if it is one of the options.
    pub fn select(&mut self, value: &str) -> bool {
        let FieldKind::Choice { options, selected } = &mut self.kind else {
            return false;
        };
        match options.iter().position(|option| option == value) {
            Some(at) => {
                *selected = at;
                true
            }
            None => false,
        }
    }

    /// Step a toggle or a choice; `true` when the value moved.
    pub fn step(&mut self, delta: isize) -> bool {
        match &mut self.kind {
            FieldKind::Toggle(on) => {
                *on = !*on;
                true
            }
            FieldKind::Choice { options, selected } => {
                if options.len() < 2 {
                    return false;
                }
                let next = nav::wrap_step(Some(*selected), options.len(), delta).unwrap_or(0);
                let changed = next != *selected;
                *selected = next;
                changed
            }
            FieldKind::Text(_) => false,
        }
    }
}

/// What one key did to the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormEvent {
    /// A value changed.
    Changed,
    /// The cursor moved, or the key was swallowed with nothing to show for it.
    Moved,
    /// Enter: the caller validates and commits.
    Submit,
    /// Esc.
    Cancel,
    /// Space on a `Choice`: the caller opens a picker over its options.
    Pick,
    /// Not the form's key.
    Ignored,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub selected: usize,
    pub offset: usize,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Self {
        let mut form = Self {
            fields,
            selected: 0,
            offset: 0,
        };
        form.selected = form.first_visible().unwrap_or(0);
        form
    }

    fn first_visible(&self) -> Option<usize> {
        self.fields.iter().position(|field| !field.hidden)
    }

    pub fn visible(&self) -> impl Iterator<Item = (usize, &Field)> {
        self.fields
            .iter()
            .enumerate()
            .filter(|(_, field)| !field.hidden)
    }

    /// Where the cursor sits among the *visible* rows, which is what the list
    /// widget counts in.
    pub fn cursor_row(&self) -> usize {
        self.visible()
            .position(|(index, _)| index == self.selected)
            .unwrap_or(0)
    }

    pub fn rows(&self) -> usize {
        self.visible().count()
    }

    pub fn focused(&self) -> Option<&Field> {
        self.fields.get(self.selected)
    }

    pub fn focused_mut(&mut self) -> Option<&mut Field> {
        self.fields.get_mut(self.selected)
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.key == key)
    }

    pub fn field_mut(&mut self, key: &str) -> Option<&mut Field> {
        self.fields.iter_mut().find(|field| field.key == key)
    }

    /// A field's value, or the empty string when it is absent.
    pub fn value(&self, key: &str) -> String {
        self.field(key).map(Field::value).unwrap_or_default()
    }

    pub fn is_on(&self, key: &str) -> bool {
        self.field(key).is_some_and(Field::is_on)
    }

    pub fn set_hidden(&mut self, key: &str, hidden: bool) {
        if let Some(field) = self.field_mut(key) {
            field.hidden = hidden;
        }
        if self.fields.get(self.selected).is_some_and(|f| f.hidden) {
            self.selected = self.first_visible().unwrap_or(0);
        }
    }

    /// Put `message` on `key`'s field and move the cursor to it, so the
    /// refusal and the text that caused it are in the same place.
    pub fn fail(&mut self, key: Option<&str>, message: impl Into<String>) {
        self.clear_errors();
        let message = message.into();
        let at = key.and_then(|key| self.fields.iter().position(|field| field.key == key));
        match at {
            Some(at) => {
                self.fields[at].error = Some(message);
                if !self.fields[at].hidden {
                    self.selected = at;
                }
            }
            None => {
                if let Some(field) = self.focused_mut() {
                    field.error = Some(message);
                }
            }
        }
    }

    pub fn clear_errors(&mut self) {
        for field in &mut self.fields {
            field.error = None;
        }
    }

    /// The error to show under the form: the focused field's, else the first.
    pub fn error(&self) -> Option<&str> {
        self.focused()
            .and_then(|field| field.error.as_deref())
            .or_else(|| {
                self.fields
                    .iter()
                    .find_map(|field| field.error.as_deref().filter(|_| !field.hidden))
            })
    }

    pub fn step(&mut self, delta: isize) {
        let visible: Vec<usize> = self.visible().map(|(index, _)| index).collect();
        if visible.is_empty() {
            return;
        }
        let at = visible
            .iter()
            .position(|index| *index == self.selected)
            .unwrap_or(0);
        let next = nav::wrap_step(Some(at), visible.len(), delta).unwrap_or(0);
        self.selected = visible[next];
    }

    pub fn clamp_viewport(&mut self, rows: usize) {
        self.offset = nav::viewport_offset(self.offset, Some(self.cursor_row()), self.rows(), rows);
    }

    /// Apply one key.
    ///
    /// Up/Down and Tab move between fields; everything else belongs to the
    /// field that has the cursor, which is why `←`/`→` edit a text field's
    /// caret and change a choice. Enter always submits: a form is answered as
    /// a whole, and a field that wanted Enter for itself would make "am I done"
    /// depend on where the cursor happens to be.
    pub fn apply(&mut self, key: &Key) -> FormEvent {
        match key.code {
            KeyCode::Esc if !key.ctrl => return FormEvent::Cancel,
            KeyCode::Enter => return FormEvent::Submit,
            KeyCode::Tab | KeyCode::Down => {
                self.step(1);
                return FormEvent::Moved;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.step(-1);
                return FormEvent::Moved;
            }
            _ => {}
        }
        let Some(field) = self.fields.get_mut(self.selected) else {
            return FormEvent::Ignored;
        };
        let is_text = matches!(field.kind, FieldKind::Text(_));
        let event = match (&mut field.kind, key.code) {
            (FieldKind::Text(edit), _) => {
                if edit.apply(key) {
                    FormEvent::Changed
                } else {
                    FormEvent::Moved
                }
            }
            (FieldKind::Choice { .. }, KeyCode::Char(' ')) => FormEvent::Pick,
            (_, KeyCode::Left) => {
                if field.step(-1) {
                    FormEvent::Changed
                } else {
                    FormEvent::Moved
                }
            }
            (_, KeyCode::Right) | (_, KeyCode::Char(' ')) => {
                if field.step(1) {
                    FormEvent::Changed
                } else {
                    FormEvent::Moved
                }
            }
            (FieldKind::Toggle(on), KeyCode::Char('y')) => {
                *on = true;
                FormEvent::Changed
            }
            (FieldKind::Toggle(on), KeyCode::Char('n')) => {
                *on = false;
                FormEvent::Changed
            }
            _ => FormEvent::Ignored,
        };
        // A key that changed something clears the refusal it is answering; a
        // caret moved inside a text field has not answered anything yet.
        if event == FormEvent::Changed || (is_text && event == FormEvent::Moved) {
            self.fields[self.selected].error = None;
        }
        if event == FormEvent::Changed {
            self.fields[self.selected].touched = true;
        }
        event
    }

    /// Draw the fields into `area`, one row each, and report where the caret
    /// of a focused text field is.
    pub fn render(
        &self,
        area: Rect,
        buffer: &mut Buffer,
        theme: &Theme,
        label_width: usize,
    ) -> Option<Position> {
        let g = theme.glyphs;
        let width = area.width as usize;
        let value_x = area.x + label_width as u16 + 3;
        let value_width = width.saturating_sub(label_width + 3);
        let items: Vec<ListItem> = self
            .visible()
            .map(|(index, field)| {
                let focused = index == self.selected;
                let marker = if focused { g.cursor } else { " " };
                let label_style = if field.error.is_some() {
                    theme.warn()
                } else if focused {
                    theme.accent()
                } else {
                    theme.dim()
                };
                let value_style = if focused { theme.text() } else { theme.dim() };
                // No decoration on a choice: the footer says how to change it,
                // which costs no glyph and no width in every unfocused row.
                ListItem::new(ratatui::text::Line::from(vec![
                    Span::styled(format!("{marker} "), theme.accent()),
                    Span::styled(pad(&field.label, label_width), label_style),
                    Span::raw(" "),
                    Span::styled(fit(&field.value(), value_width, g.ellipsis), value_style),
                ]))
            })
            .collect();
        let mut state = ListState::default().with_offset(self.offset);
        StatefulWidget::render(List::new(items), area, buffer, &mut state);

        // The focused text field draws itself again over its own row, so the
        // caret lands in the text rather than at the start of the line.
        let row = self.cursor_row().checked_sub(self.offset)?;
        if row >= area.height as usize {
            return None;
        }
        let field = self.focused()?;
        let FieldKind::Text(edit) = &field.kind else {
            return None;
        };
        let line = Rect::new(value_x, area.y + row as u16, value_width as u16, 1);
        edit.render_line(line, buffer, Span::raw(""), theme.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyCode;

    fn form() -> Form {
        Form::new(vec![
            Field::text("path", "Folder", "where", "/mnt/projects/Legacy"),
            Field::toggle("rename", "Standardize name", "rename it", false),
            Field::choice(
                "created",
                "Created",
                "which date",
                vec!["the folder's own date".into(), "today".into()],
                0,
            ),
        ])
    }

    #[test]
    fn tab_moves_and_typing_edits_the_focused_field() {
        let mut form = form();
        assert_eq!(form.apply(&Key::ch('X')), FormEvent::Changed);
        assert_eq!(form.value("path"), "/mnt/projects/LegacyX");
        assert_eq!(form.apply(&Key::plain(KeyCode::Tab)), FormEvent::Moved);
        assert_eq!(form.apply(&Key::ch('y')), FormEvent::Changed);
        assert!(form.is_on("rename"));
        assert_eq!(form.apply(&Key::ch('n')), FormEvent::Changed);
        assert!(!form.is_on("rename"));
    }

    #[test]
    fn a_choice_steps_with_the_arrows_and_offers_a_picker_on_space() {
        let mut form = form();
        form.selected = 2;
        assert_eq!(form.apply(&Key::plain(KeyCode::Right)), FormEvent::Changed);
        assert_eq!(form.value("created"), "today");
        assert_eq!(form.apply(&Key::plain(KeyCode::Left)), FormEvent::Changed);
        assert_eq!(form.value("created"), "the folder's own date");
        assert_eq!(form.apply(&Key::ch(' ')), FormEvent::Pick);
    }

    #[test]
    fn a_hidden_field_is_skipped_and_never_holds_the_cursor() {
        let mut form = form();
        form.set_hidden("rename", true);
        form.step(1);
        assert_eq!(form.focused().unwrap().key, "created");
        form.selected = 1;
        form.set_hidden("rename", true);
        assert_ne!(form.focused().unwrap().key, "rename");
        assert_eq!(form.rows(), 2);
    }

    #[test]
    fn a_refusal_lands_on_its_own_field_and_the_next_edit_clears_it() {
        let mut form = form();
        form.selected = 1;
        form.fail(Some("path"), "no such folder: /nope");
        assert_eq!(form.focused().unwrap().key, "path");
        assert_eq!(form.error(), Some("no such folder: /nope"));
        form.apply(&Key::plain(KeyCode::Backspace));
        assert_eq!(form.error(), None);
    }

    #[test]
    fn enter_submits_and_esc_cancels_wherever_the_cursor_is() {
        let mut form = form();
        assert_eq!(form.apply(&Key::plain(KeyCode::Enter)), FormEvent::Submit);
        form.selected = 2;
        assert_eq!(form.apply(&Key::plain(KeyCode::Esc)), FormEvent::Cancel);
    }
}
