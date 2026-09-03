//! A multi-line editor: the template builder's folder list and file contents.
//!
//! Written here rather than taken from `tui-textarea`, which the plan named as
//! a candidate. Its current release pins `ratatui 0.29`, and a widget built
//! against a different ratatui does not implement *our* `Widget` trait at all —
//! adding it pulls a second copy of ratatui into the tree, or fails to resolve,
//! which is what it does here. The plan's own condition for a widget crate is
//! that it build against the ratatui in `Cargo.toml`; this one does not, so the
//! piece is ours.
//!
//! It is [`LineEdit`](super::input::LineEdit) with a second dimension, and the
//! same rule holds: **the cursor is a char index, never a byte offset**, on
//! both axes. A folder name can hold any character a filesystem can, and
//! slicing one mid-character panics.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::tui::command::Key;
use crate::tui::widgets::input::visible_window;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextArea {
    lines: Vec<String>,
    /// Which line the caret is on.
    row: usize,
    /// Chars before the caret on that line.
    column: usize,
    /// First visible line.
    offset: usize,
}

impl Default for TextArea {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            column: 0,
            offset: 0,
        }
    }
}

impl TextArea {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start with `text`, the caret at the end. An empty string is one empty
    /// line, never zero lines — there is always somewhere to type.
    pub fn with_text(text: &str) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(str::to_string).collect()
        };
        let row = lines.len() - 1;
        let column = lines[row].chars().count();
        Self {
            lines,
            row,
            column,
            offset: 0,
        }
    }

    /// Every line, joined with `\n`. No trailing newline is added: a caller
    /// that wants one says so.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The lines with anything blank dropped and the rest trimmed — what a
    /// list of folder paths is, as opposed to a document.
    pub fn entries(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.is_empty())
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.column)
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map(|l| l.chars().count()).unwrap_or(0)
    }

    fn insert(&mut self, c: char) {
        let at = byte_index(&self.lines[self.row], self.column);
        self.lines[self.row].insert(at, c);
        self.column += 1;
    }

    fn split_line(&mut self) {
        let at = byte_index(&self.lines[self.row], self.column);
        let rest = self.lines[self.row].split_off(at);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.column = 0;
    }

    /// Backspace: within a line it removes a char; at the start of one it
    /// joins that line to the one above, with the caret where they met.
    fn backspace(&mut self) -> bool {
        if self.column > 0 {
            let at = byte_index(&self.lines[self.row], self.column - 1);
            self.lines[self.row].remove(at);
            self.column -= 1;
            return true;
        }
        if self.row == 0 {
            return false;
        }
        let removed = self.lines.remove(self.row);
        self.row -= 1;
        self.column = self.line_len(self.row);
        self.lines[self.row].push_str(&removed);
        true
    }

    fn delete(&mut self) -> bool {
        if self.column < self.line_len(self.row) {
            let at = byte_index(&self.lines[self.row], self.column);
            self.lines[self.row].remove(at);
            return true;
        }
        if self.row + 1 >= self.lines.len() {
            return false;
        }
        let next = self.lines.remove(self.row + 1);
        self.lines[self.row].push_str(&next);
        true
    }

    pub fn paste(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => self.split_line(),
                '\r' => {}
                c if c.is_control() => {}
                c => self.insert(c),
            }
        }
    }

    /// Apply one key. `true` when the text changed.
    ///
    /// Enter is a newline here, so the caller cannot use it to submit: an
    /// editor that ends on Enter cannot hold two lines. The builder's sections
    /// commit with Ctrl-S and cancel with Esc, and say so.
    pub fn apply(&mut self, key: &Key) -> bool {
        if let Some(c) = key.typed() {
            self.insert(c);
            return true;
        }
        match (key.code, key.ctrl) {
            (KeyCode::Enter, false) => {
                self.split_line();
                true
            }
            (KeyCode::Backspace, false) => self.backspace(),
            (KeyCode::Delete, false) => self.delete(),
            (KeyCode::Left, false) => {
                if self.column > 0 {
                    self.column -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.column = self.line_len(self.row);
                }
                false
            }
            (KeyCode::Right, false) => {
                if self.column < self.line_len(self.row) {
                    self.column += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.column = 0;
                }
                false
            }
            (KeyCode::Up, false) => {
                self.row = self.row.saturating_sub(1);
                self.column = self.column.min(self.line_len(self.row));
                false
            }
            (KeyCode::Down, false) => {
                self.row = (self.row + 1).min(self.lines.len() - 1);
                self.column = self.column.min(self.line_len(self.row));
                false
            }
            (KeyCode::Home, false) | (KeyCode::Char('a'), true) => {
                self.column = 0;
                false
            }
            (KeyCode::End, false) | (KeyCode::Char('e'), true) => {
                self.column = self.line_len(self.row);
                false
            }
            (KeyCode::Char('u'), true) => {
                // The line, not the document: Ctrl-U in a shell clears a line.
                let changed = !self.lines[self.row].is_empty();
                self.lines[self.row].clear();
                self.column = 0;
                changed
            }
            (KeyCode::Char('k'), true) => {
                // Kill the line entirely, which is how a folder path is dropped.
                if self.lines.len() == 1 {
                    let changed = !self.lines[0].is_empty();
                    self.lines[0].clear();
                    self.column = 0;
                    return changed;
                }
                self.lines.remove(self.row);
                self.row = self.row.min(self.lines.len() - 1);
                self.column = self.column.min(self.line_len(self.row));
                true
            }
            _ => false,
        }
    }

    fn clamp_viewport(&mut self, rows: usize) {
        self.offset =
            super::nav::viewport_offset(self.offset, Some(self.row), self.lines.len(), rows);
    }

    /// Draw into `area` and report where the caret is. Takes `&mut self` for
    /// the viewport alone — the same bargain `LibraryState.offset` makes:
    /// scrolling is state, and re-deriving it every frame throws it away.
    pub fn render(&mut self, area: Rect, buffer: &mut Buffer, style: Style) -> Option<Position> {
        if area.height == 0 || area.width == 0 {
            return None;
        }
        self.clamp_viewport(area.height as usize);
        let width = area.width as usize;
        let rendered: Vec<Line> = self
            .lines
            .iter()
            .skip(self.offset)
            .take(area.height as usize)
            .map(|line| {
                let (shown, _) = visible_window(line, line.chars().count(), width, 0);
                Line::from(Span::styled(shown, style))
            })
            .collect();
        Paragraph::new(rendered).render(area, buffer);

        let row = self.row.checked_sub(self.offset)?;
        if row >= area.height as usize {
            return None;
        }
        let (_, caret) = visible_window(&self.lines[self.row], self.column, width, 0);
        Some(Position::new(
            area.x + caret.min(width.saturating_sub(1)) as u16,
            area.y + row as u16,
        ))
    }
}

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyCode;

    fn key(code: KeyCode) -> Key {
        Key::plain(code)
    }

    #[test]
    fn enter_splits_a_line_and_backspace_joins_it_back() {
        let mut area = TextArea::with_text("01_Assets");
        assert!(area.apply(&key(KeyCode::Enter)));
        assert!(area.apply(&Key::ch('0')));
        assert_eq!(area.text(), "01_Assets\n0");
        assert_eq!(area.cursor(), (1, 1));

        assert!(area.apply(&key(KeyCode::Backspace)));
        assert!(area.apply(&key(KeyCode::Backspace)));
        assert_eq!(area.text(), "01_Assets");
        assert_eq!(area.cursor(), (0, 9));
    }

    #[test]
    fn entries_are_the_non_blank_lines_trimmed() {
        let area = TextArea::with_text("01_Assets\n\n  02_Edit  \n");
        assert_eq!(
            area.entries(),
            vec!["01_Assets".to_string(), "02_Edit".to_string()]
        );
        assert_eq!(area.lines().len(), 4);
    }

    #[test]
    fn a_pasted_block_becomes_lines() {
        let mut area = TextArea::new();
        area.paste("a\nb/c\r\nd");
        assert_eq!(area.text(), "a\nb/c\nd");
    }

    #[test]
    fn the_caret_never_leaves_the_text() {
        let mut area = TextArea::with_text("ab\nlonger line");
        area.apply(&key(KeyCode::End));
        area.apply(&key(KeyCode::Up));
        assert_eq!(area.cursor(), (0, 2), "clamped to the shorter line");
        area.apply(&key(KeyCode::Down));
        area.apply(&key(KeyCode::Down));
        assert_eq!(area.cursor().0, 1, "the last line is the last line");
        for _ in 0..40 {
            area.apply(&key(KeyCode::Left));
        }
        assert_eq!(area.cursor(), (0, 0));
    }

    #[test]
    fn ctrl_k_drops_the_line_and_never_the_last_one() {
        let mut area = TextArea::with_text("one\ntwo");
        assert!(area.apply(&Key::ctrl('k')));
        assert_eq!(area.text(), "one");
        assert!(area.apply(&Key::ctrl('k')));
        assert_eq!(area.text(), "");
        assert_eq!(area.lines().len(), 1, "there is always somewhere to type");
    }

    #[test]
    fn multibyte_text_is_edited_by_character() {
        let mut area = TextArea::with_text("日本語");
        assert!(area.apply(&key(KeyCode::Backspace)));
        assert_eq!(area.text(), "日本");
        area.apply(&key(KeyCode::Home));
        assert!(area.apply(&Key::ch('a')));
        assert_eq!(area.text(), "a日本");
    }
}
