//! One keystroke, normalised, and how it is written on a key line.

use super::*;

/// One keystroke, normalised: shift is folded into the character, Ctrl and Alt
/// are flags. `KeyCode::Char('c')` with the control flag is Ctrl-C.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl Key {
    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
        }
    }

    pub const fn ch(c: char) -> Self {
        Self::plain(KeyCode::Char(c))
    }

    pub const fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            ctrl: true,
            alt: false,
        }
    }

    /// The text the hint bar and the help overlay print for this key, in the
    /// Unicode alphabet. Anything drawn on screen asks [`Key::label_in`].
    pub fn label(&self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "Shift-Tab".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Del".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        };
        self.with_modifiers(base)
    }

    /// The label in `g`'s alphabet. Where the terminal has no arrows to draw
    /// — a console on the ASCII alphabet — an arrow key is the word printed
    /// on it, not `->`, which reads as a key of its own.
    pub fn label_in(&self, g: &crate::tui::theme::Glyphs) -> String {
        if !g.is_ascii() {
            return self.label();
        }
        let word = match self.code {
            KeyCode::Up => "Up",
            KeyCode::Down => "Down",
            KeyCode::Left => "Left",
            KeyCode::Right => "Right",
            _ => return self.label(),
        };
        self.with_modifiers(word.to_string())
    }

    fn with_modifiers(&self, base: String) -> String {
        match (self.ctrl, self.alt) {
            (true, true) => format!("Ctrl-Alt-{base}"),
            (true, false) => format!("Ctrl-{base}"),
            (false, true) => format!("Alt-{base}"),
            (false, false) => base,
        }
    }

    /// A printable character with no modifier — what a text field inserts.
    pub fn typed(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt && !c.is_control() => Some(c),
            _ => None,
        }
    }
}

impl From<KeyEvent> for Key {
    fn from(event: KeyEvent) -> Self {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        let code = match event.code {
            // Ctrl-Shift-P and Ctrl-p are the same chord; a terminal reports
            // either casing depending on the protocol it speaks.
            KeyCode::Char(c) if ctrl => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        Self { code, ctrl, alt }
    }
}
