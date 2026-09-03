//! A single-line editor: printable insert at the cursor, Backspace, Delete,
//! Left/Right, Home/End, Ctrl-U (clear), Ctrl-W (delete the word before the
//! cursor), paste.
//!
//! Ported from `tui::prompt`'s `LineEditor`, which still drives the CLI's text
//! prompts until they move here too. The cursor is a **char index**, never a
//! byte offset — a value can hold any character a folder name can, and slicing
//! one mid-character panics. A long line is *windowed* around the cursor rather
//! than wrapped, and the caret is reported as a cell so the frame can park the
//! terminal's real cursor in the text.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::tui::command::Key;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEdit {
    text: String,
    /// Chars before the cursor.
    cursor: usize,
}

impl LineEdit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start with editable text, the cursor after it.
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn insert(&mut self, c: char) {
        let at = byte_index(&self.text, self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn paste(&mut self, pasted: &str) {
        for c in pasted.chars().filter(|c| !c.is_control()) {
            self.insert(c);
        }
    }

    /// Apply one key. `true` when the text changed.
    pub fn apply(&mut self, key: &Key) -> bool {
        if let Some(c) = key.typed() {
            self.insert(c);
            return true;
        }
        match (key.code, key.ctrl) {
            (KeyCode::Backspace, false) if self.cursor > 0 => {
                let at = byte_index(&self.text, self.cursor - 1);
                self.text.remove(at);
                self.cursor -= 1;
                true
            }
            (KeyCode::Delete, false) if self.cursor < self.len() => {
                let at = byte_index(&self.text, self.cursor);
                self.text.remove(at);
                true
            }
            (KeyCode::Left, false) => {
                self.cursor = self.cursor.saturating_sub(1);
                false
            }
            (KeyCode::Right, false) => {
                self.cursor = (self.cursor + 1).min(self.len());
                false
            }
            (KeyCode::Home, false) | (KeyCode::Char('a'), true) => {
                self.cursor = 0;
                false
            }
            (KeyCode::End, false) | (KeyCode::Char('e'), true) => {
                self.cursor = self.len();
                false
            }
            (KeyCode::Char('u'), true) => {
                let changed = !self.text.is_empty();
                self.clear();
                changed
            }
            (KeyCode::Char('w'), true) => {
                if self.cursor == 0 {
                    return false;
                }
                let chars: Vec<char> = self.text.chars().collect();
                let mut start = self.cursor;
                while start > 0 && chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                let kept: String = chars[..start]
                    .iter()
                    .chain(chars[self.cursor..].iter())
                    .collect();
                self.text = kept;
                self.cursor = start;
                true
            }
            _ => false,
        }
    }

    /// The slice to show in `columns` cells after a `prompt_width`-wide prefix,
    /// and the caret's offset within it.
    pub fn window(&self, columns: usize, prompt_width: usize) -> (String, usize) {
        visible_window(&self.text, self.cursor, columns, prompt_width)
    }

    /// Draw `prefix` then the text into `area`, and say where the caret is.
    /// `None` when the area has no room for a caret.
    pub fn render_line(
        &self,
        area: Rect,
        buf: &mut Buffer,
        prefix: Span<'_>,
        text_style: Style,
    ) -> Option<Position> {
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let prefix_width = prefix.width();
        let (window, offset) = self.window(area.width as usize, prefix_width);
        Line::from(vec![prefix, Span::styled(window, text_style)]).render(area, buf);
        let column = (prefix_width + offset).min(area.width.saturating_sub(1) as usize);
        Some(Position::new(area.x + column as u16, area.y))
    }
}

/// Byte offset of char `n`, saturating at the end.
fn byte_index(text: &str, n: usize) -> usize {
    text.char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

/// The slice of `text` to show when it is longer than the line has room for,
/// and the cursor's char offset **within that slice** — which is what the caret
/// is drawn at, and is not the cursor index once the window has scrolled.
///
/// Windowing rather than wrapping is deliberate: a soft-wrapped line is what
/// ghosts on the legacy Windows console. The window always contains the
/// cursor, and prefers to keep the end of the text visible, which is where a
/// person typing is looking.
pub fn visible_window(
    text: &str,
    cursor: usize,
    columns: usize,
    prompt_width: usize,
) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    // One column of headroom so the cursor position itself never lands in the
    // last cell, which is where terminals disagree about wrapping.
    let room = columns.saturating_sub(prompt_width + 1);
    if room == 0 || chars.len() <= room {
        return (text.to_string(), cursor.min(chars.len()));
    }
    let end = (cursor + 1).max(room).min(chars.len());
    let start = end - room;
    (
        chars[start..end].iter().collect(),
        cursor.saturating_sub(start),
    )
}

#[cfg(test)]
mod tests {
    use super::{LineEdit, byte_index, visible_window};
    use crate::tui::command::Key;
    use ratatui::crossterm::event::KeyCode;

    fn editor(text: &str) -> LineEdit {
        LineEdit::with_text(text)
    }

    #[test]
    fn initial_text_is_editable_and_the_cursor_starts_after_it() {
        let mut e = editor("Draft");
        assert_eq!(e.cursor(), 5);
        e.apply(&Key::plain(KeyCode::Backspace));
        e.apply(&Key::ch('t'));
        e.apply(&Key::ch('e'));
        e.apply(&Key::ch('d'));
        assert_eq!(e.text(), "Drafted");
    }

    #[test]
    fn the_cursor_is_a_char_index_not_a_byte_offset() {
        // Four chars, ten bytes. A byte-offset cursor slices mid-character and
        // panics on the first one of these.
        let mut e = editor("日本語だ");
        assert_eq!(e.cursor(), 4);
        e.apply(&Key::plain(KeyCode::Left));
        e.apply(&Key::ch('!'));
        assert_eq!(e.text(), "日本語!だ");
        e.apply(&Key::plain(KeyCode::Home));
        e.apply(&Key::plain(KeyCode::Delete));
        assert_eq!(e.text(), "本語!だ");
        e.apply(&Key::plain(KeyCode::End));
        e.apply(&Key::plain(KeyCode::Backspace));
        assert_eq!(e.text(), "本語!");
    }

    #[test]
    fn the_edges_hold() {
        let mut e = editor("");
        e.apply(&Key::plain(KeyCode::Backspace));
        e.apply(&Key::plain(KeyCode::Delete));
        e.apply(&Key::plain(KeyCode::Left));
        assert_eq!(e.text(), "");
        assert_eq!(e.cursor(), 0);

        let mut e = editor("ab");
        e.apply(&Key::plain(KeyCode::Right));
        e.apply(&Key::plain(KeyCode::Right));
        e.apply(&Key::plain(KeyCode::Right));
        assert_eq!(e.cursor(), 2);
        e.apply(&Key::plain(KeyCode::Delete));
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn control_characters_are_not_inserted() {
        let mut e = editor("a");
        e.apply(&Key::ch('\t'));
        e.apply(&Key::ch('\u{7f}'));
        e.apply(&Key::ctrl('x'));
        assert_eq!(e.text(), "a");
    }

    #[test]
    fn ctrl_w_deletes_the_word_before_the_cursor_and_ctrl_u_clears() {
        let mut e = editor("tag:draft lullaby  ");
        assert!(e.apply(&Key::ctrl('w')));
        assert_eq!(e.text(), "tag:draft ");
        e.apply(&Key::plain(KeyCode::Home));
        assert!(!e.apply(&Key::ctrl('w')), "nothing before the cursor");
        assert!(e.apply(&Key::ctrl('u')));
        assert_eq!(e.text(), "");
    }

    #[test]
    fn paste_inserts_at_the_cursor_and_drops_control_characters() {
        let mut e = editor("ab");
        e.apply(&Key::plain(KeyCode::Left));
        e.paste("x\ty\n");
        assert_eq!(e.text(), "axyb");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn byte_index_saturates_at_the_end() {
        assert_eq!(byte_index("abc", 0), 0);
        assert_eq!(byte_index("abc", 3), 3);
        assert_eq!(byte_index("abc", 99), 3);
        assert_eq!(byte_index("日本", 1), 3);
    }

    #[test]
    fn a_short_line_is_shown_whole() {
        assert_eq!(visible_window("hello", 5, 80, 6), ("hello".to_string(), 5));
        // No room at all: better to show the text than nothing.
        assert_eq!(visible_window("hello", 5, 4, 6), ("hello".to_string(), 5));
    }

    #[test]
    fn a_long_line_windows_around_the_cursor_instead_of_wrapping() {
        let text: String = ('a'..='z').collect();
        // Room for 10 chars (20 columns minus a 9-char prompt minus headroom).
        let (at_end, offset) = visible_window(&text, 26, 20, 9);
        assert_eq!(at_end.chars().count(), 10);
        assert_eq!(at_end, "qrstuvwxyz", "the end of the text stays visible");
        assert_eq!(offset, 10, "the caret sits after the last visible char");

        // Cursor moved back into the middle: the window follows it.
        let (middle, offset) = visible_window(&text, 12, 20, 9);
        assert_eq!(middle.chars().count(), 10);
        assert!(
            middle.contains('l') && middle.contains('m'),
            "the window must contain the cursor: {middle}"
        );
        assert_eq!(
            middle.chars().nth(offset),
            Some('m'),
            "the caret offset points at the cursor's own char, not its index \
             into the whole text"
        );

        // And at the very start.
        let (start, offset) = visible_window(&text, 0, 20, 9);
        assert_eq!(start, "abcdefghij");
        assert_eq!(offset, 0);
    }

    #[test]
    fn the_caret_column_always_lands_inside_the_line() {
        // Whatever the text, the window and the cursor's place in it, the column
        // the caret is moved to must stay on the row. A caret parked past the
        // last cell is where terminals disagree about wrapping.
        let text: String = ('a'..='z').collect();
        let prompt_width = 9;
        for columns in [12usize, 20, 26, 80] {
            for cursor in 0..=text.chars().count() {
                let (window, offset) = visible_window(&text, cursor, columns, prompt_width);
                assert!(
                    offset <= window.chars().count(),
                    "offset {offset} outside window {window:?}"
                );
                if columns > prompt_width + 1 {
                    assert!(
                        prompt_width + offset < columns,
                        "caret at column {} in a {columns}-column line",
                        prompt_width + offset
                    );
                }
            }
        }
    }

    #[test]
    fn the_rendered_caret_sits_after_the_text() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Style;
        use ratatui::text::Span;
        let e = editor("abc");
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        let caret = e
            .render_line(area, &mut buf, Span::raw("> "), Style::default())
            .unwrap();
        assert_eq!((caret.x, caret.y), (5, 0));
        assert_eq!(buf[(0, 0)].symbol(), ">");
        assert_eq!(buf[(2, 0)].symbol(), "a");
    }
}
