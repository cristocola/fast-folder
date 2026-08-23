//! The one place fastf asks a question.
//!
//! Every prompt in the tool goes through here, and `tests/layering.rs` fails the
//! build if any other module under `src/tui` or `src/cli` names
//! `dialoguer::Select`, `MultiSelect`, `Confirm`, or `Input`. That is the
//! enforcement, because the defect this replaces was not a wrong behaviour but
//! an inconsistent one: an earlier attempt moved twenty-nine prompts to
//! `interact_opt` by hand and missed several, so Esc backed out of some menus
//! and was swallowed by others, which is worse than Esc never working.
//!
//! **`Ok(None)` is a cancelled prompt. It is never an error**, so
//! `tui::menu::is_fatal` keeps classifying a *broken* prompt (no terminal, stdin
//! at EOF) as fatal and a cancelled one as an ordinary answer.
//!
//! Two of the four are hand-rolled on `dialoguer::console::Term`:
//!
//! * `confirm`, because `Confirm::interact_opt` makes Esc set a pending value
//!   that still needs Enter, and because its `interact` variant answers a bare
//!   `y`/`n` with no Enter — a contract worth keeping and one `interact_opt`
//!   drops.
//! * `text`, because `dialoguer::Input` has no `Key::Escape` arm at all. There
//!   is no way to cancel a text prompt through it.
//!
//! Rendering mirrors `dialoguer`'s `SimpleTheme` exactly, so the prompt strings
//! the pty suite anchors on do not move.

use anyhow::Result;
use colored::Colorize;
use dialoguer::console::{Key, Term};
use dialoguer::theme::{SimpleTheme, Theme};

use crate::util::tty;

/// What a text prompt starts with and what it will accept.
#[derive(Default)]
pub struct TextOpts<'a> {
    /// Editable starting text, not a hint: the cursor lands at its end and
    /// Backspace works into it. This is what a retry uses, so a value rejected
    /// by a validator is corrected rather than retyped.
    pub initial: Option<String>,
    /// A value an empty answer means, shown as `prompt [default]:` on an empty
    /// line — `dialoguer::Input::default`'s contract, kept because "type the new
    /// number" is a different gesture from "edit the old one".
    pub default: Option<String>,
    /// Whether an empty answer submits.
    pub allow_empty: bool,
    /// Runs on Enter. `Err` keeps the text on the line and prints the message
    /// below it, so a rejected answer is corrected rather than retyped.
    #[allow(clippy::type_complexity)]
    pub validator: Option<Box<dyn Fn(&str) -> std::result::Result<(), String> + 'a>>,
}

impl<'a> TextOpts<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn initial(mut self, text: impl Into<String>) -> Self {
        self.initial = Some(text.into());
        self
    }

    pub fn default_value(mut self, text: impl Into<String>) -> Self {
        self.default = Some(text.into());
        self
    }

    pub fn allow_empty(mut self) -> Self {
        self.allow_empty = true;
        self
    }

    pub fn validate(mut self, f: impl Fn(&str) -> std::result::Result<(), String> + 'a) -> Self {
        self.validator = Some(Box::new(f));
        self
    }
}

/// Guard shared by all four: a prompt that cannot be drawn is refused with a
/// message, never left to fail inside the terminal library.
///
/// Call sites that know a flag which answers the same question call
/// `tty::require_tty` themselves first, with that flag named; this is the
/// backstop for the ones that do not.
fn ready() -> Result<()> {
    tty::require_tty(
        "prompt",
        "run the command with its flags instead of interactively",
    )
}

/// Pick one item. `Ok(None)` is Esc or `q`.
pub fn select(prompt: &str, items: &[String], default: usize) -> Result<Option<usize>> {
    select_with_theme(prompt, items, default, &SimpleTheme)
}

/// `select` with a caller-supplied theme — the project lists use one that
/// highlights the whole row.
pub fn select_with_theme(
    prompt: &str,
    items: &[String],
    default: usize,
    theme: &dyn Theme,
) -> Result<Option<usize>> {
    ready()?;
    Ok(dialoguer::Select::with_theme(theme)
        .with_prompt(prompt)
        .items(items)
        .default(default.min(items.len().saturating_sub(1)))
        .interact_opt()?)
}

/// Pick any number of items. `Ok(None)` is Esc or `q`; `Ok(Some(vec![]))` is a
/// deliberate empty selection.
pub fn multi_select(
    prompt: &str,
    items: &[String],
    checked: &[bool],
) -> Result<Option<Vec<usize>>> {
    ready()?;
    Ok(dialoguer::MultiSelect::new()
        .with_prompt(prompt)
        .items(items)
        .defaults(checked)
        .interact_opt()?)
}

/// Reorder items by dragging. `Ok(None)` is Esc or `q`.
pub fn sort(prompt: &str, items: &[String]) -> Result<Option<Vec<usize>>> {
    ready()?;
    Ok(dialoguer::Sort::new()
        .with_prompt(prompt)
        .items(items)
        .interact_opt()?)
}

/// Yes or no. A bare `y`/`n` answers without Enter, Enter takes the default,
/// Esc or `q` cancels.
pub fn confirm(prompt: &str, default: bool) -> Result<Option<bool>> {
    ready()?;
    let term = Term::stderr();
    let theme = SimpleTheme;

    let mut line = String::new();
    theme
        .format_confirm_prompt(&mut line, prompt, Some(default))
        .map_err(std::io::Error::other)?;

    // Written without a newline, the way dialoguer draws it, so the answer keys
    // land after the question rather than under it.
    term.write_str(&line)?;
    term.flush()?;
    term.hide_cursor()?;

    let answer = loop {
        match term.read_key() {
            Ok(Key::Char('y') | Key::Char('Y')) => break Some(true),
            Ok(Key::Char('n') | Key::Char('N')) => break Some(false),
            Ok(Key::Enter) => break Some(default),
            Ok(Key::Escape | Key::Char('q')) => break None,
            Ok(_) => continue,
            Err(e) => {
                let _ = term.show_cursor();
                term.write_line("")?;
                return Err(e.into());
            }
        }
    };

    term.show_cursor()?;
    // Take the question back the way `Confirm` does, then leave nothing behind.
    term.clear_line()?;
    term.flush()?;
    Ok(answer)
}

/// Read a line. `Ok(None)` is Esc.
pub fn text(prompt: &str, opts: TextOpts<'_>) -> Result<Option<String>> {
    ready()?;
    let term = Term::stderr();
    let mut editor = LineEditor::new(
        prompt,
        opts.initial.clone().unwrap_or_default(),
        opts.default.clone(),
    );

    term.hide_cursor()?;
    let outcome = run_editor(&term, &mut editor, &opts);
    let _ = term.show_cursor();
    let _ = term.flush();
    outcome
}

fn run_editor(term: &Term, editor: &mut LineEditor, opts: &TextOpts<'_>) -> Result<Option<String>> {
    let mut error: Option<String> = None;
    loop {
        editor.render(term, term.size().1 as usize, error.as_deref())?;
        match term.read_key()? {
            Key::Escape => {
                editor.erase(term)?;
                return Ok(None);
            }
            Key::Enter => {
                let mut value = editor.text.clone();
                if value.is_empty()
                    && let Some(fallback) = &opts.default
                {
                    value = fallback.clone();
                }
                if value.is_empty() && !opts.allow_empty {
                    error = Some("a value is required (Esc to cancel)".to_string());
                    continue;
                }
                if let Some(validator) = &opts.validator
                    && let Err(message) = validator(&value)
                {
                    // The text stays on the line. Losing it is the defect Phase 8
                    // is named after, and a validator is where it happened most.
                    error = Some(message);
                    continue;
                }
                editor.erase(term)?;
                return Ok(Some(value));
            }
            key => {
                editor.apply(key);
                error = None;
            }
        }
    }
}

/// A single-line editor: printable insert at the cursor, Backspace, Delete,
/// Left/Right, Home/End.
///
/// The cursor position is a **char index**, never a byte offset — a value can
/// hold any character a folder name can, and slicing one mid-character panics.
struct LineEditor {
    prompt: String,
    /// Rendered as `prompt [default]:`, the way `dialoguer::Input` shows one.
    default: Option<String>,
    text: String,
    /// Chars before the cursor.
    cursor: usize,
    /// Lines currently on screen, so a repaint knows what to take back.
    drawn: usize,
}

impl LineEditor {
    fn new(prompt: &str, initial: String, default: Option<String>) -> Self {
        let cursor = initial.chars().count();
        Self {
            prompt: prompt.to_string(),
            default,
            text: initial,
            cursor,
            drawn: 0,
        }
    }

    fn apply(&mut self, key: Key) {
        match key {
            Key::Char(c) if !c.is_control() => {
                let at = byte_index(&self.text, self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            Key::Backspace if self.cursor > 0 => {
                let at = byte_index(&self.text, self.cursor - 1);
                self.text.remove(at);
                self.cursor -= 1;
            }
            Key::Del if self.cursor < self.text.chars().count() => {
                let at = byte_index(&self.text, self.cursor);
                self.text.remove(at);
            }
            Key::ArrowLeft => self.cursor = self.cursor.saturating_sub(1),
            Key::ArrowRight => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = self.text.chars().count(),
            _ => {}
        }
    }

    fn render(&mut self, term: &Term, columns: usize, error: Option<&str>) -> std::io::Result<()> {
        self.erase(term)?;

        let mut head = String::new();
        SimpleTheme
            .format_input_prompt(&mut head, &self.prompt, self.default.as_deref())
            .map_err(std::io::Error::other)?;

        let window = visible_window(&self.text, self.cursor, columns, head.chars().count());
        term.write_line(&format!("{head}{window}"))?;
        self.drawn = 1;
        if let Some(message) = error {
            let mut line = String::new();
            SimpleTheme
                .format_error(&mut line, message)
                .map_err(std::io::Error::other)?;
            term.write_line(&line)?;
            self.drawn += 1;
        }
        term.flush()
    }

    fn erase(&mut self, term: &Term) -> std::io::Result<()> {
        if self.drawn > 0 {
            term.clear_last_lines(self.drawn)?;
            self.drawn = 0;
        }
        Ok(())
    }
}

/// Byte offset of char `n`, saturating at the end.
fn byte_index(text: &str, n: usize) -> usize {
    text.char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

/// The slice of `text` to show when it is longer than the line has room for.
///
/// Windowing rather than wrapping is deliberate: a soft-wrapped line is what
/// ghosts on the legacy Windows console, and it is also what makes
/// `clear_last_lines` take back the wrong number of rows. The window always
/// contains the cursor, and prefers to keep the end of the text visible, which
/// is where a person typing is looking.
fn visible_window(text: &str, cursor: usize, columns: usize, prompt_width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    // One column of headroom so the cursor position itself never lands in the
    // last cell, which is where terminals disagree about wrapping.
    let room = columns.saturating_sub(prompt_width + 1);
    if room == 0 || chars.len() <= room {
        return text.to_string();
    }
    let end = (cursor + 1).max(room).min(chars.len());
    let start = end - room;
    chars[start..end].iter().collect()
}

/// The one sentence a cancelled flow prints before returning to where it came
/// from. `what` completes "Cancelled — _": say what did *not* happen, since the
/// reassurance is the point ("nothing was created", not "aborted").
pub fn report_cancelled(what: &str) {
    println!("{}", format!("Cancelled — {what}.").dimmed());
}

#[cfg(test)]
mod tests {
    use super::{LineEditor, byte_index, visible_window};
    use dialoguer::console::Key;

    fn editor(text: &str) -> LineEditor {
        LineEditor::new("Name", text.to_string(), None)
    }

    #[test]
    fn initial_text_is_editable_and_the_cursor_starts_after_it() {
        let mut e = editor("Draft");
        assert_eq!(e.cursor, 5);
        e.apply(Key::Backspace);
        e.apply(Key::Char('t'));
        e.apply(Key::Char('e'));
        e.apply(Key::Char('d'));
        assert_eq!(e.text, "Drafted");
    }

    #[test]
    fn the_cursor_is_a_char_index_not_a_byte_offset() {
        // Four chars, ten bytes. A byte-offset cursor slices mid-character and
        // panics on the first one of these.
        let mut e = editor("日本語だ");
        assert_eq!(e.cursor, 4);
        e.apply(Key::ArrowLeft);
        e.apply(Key::Char('!'));
        assert_eq!(e.text, "日本語!だ");
        e.apply(Key::Home);
        e.apply(Key::Del);
        assert_eq!(e.text, "本語!だ");
        e.apply(Key::End);
        e.apply(Key::Backspace);
        assert_eq!(e.text, "本語!");
    }

    #[test]
    fn the_edges_hold() {
        let mut e = editor("");
        e.apply(Key::Backspace);
        e.apply(Key::Del);
        e.apply(Key::ArrowLeft);
        assert_eq!(e.text, "");
        assert_eq!(e.cursor, 0);

        let mut e = editor("ab");
        e.apply(Key::ArrowRight);
        e.apply(Key::ArrowRight);
        e.apply(Key::ArrowRight);
        assert_eq!(e.cursor, 2);
        e.apply(Key::Del);
        assert_eq!(e.text, "ab");
    }

    #[test]
    fn control_characters_are_not_inserted() {
        let mut e = editor("a");
        e.apply(Key::Char('\t'));
        e.apply(Key::Char('\u{7f}'));
        assert_eq!(e.text, "a");
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
        assert_eq!(visible_window("hello", 5, 80, 6), "hello");
        // No room at all: better to show the text than nothing.
        assert_eq!(visible_window("hello", 5, 4, 6), "hello");
    }

    #[test]
    fn a_long_line_windows_around_the_cursor_instead_of_wrapping() {
        let text: String = ('a'..='z').collect();
        // Room for 10 chars (20 columns minus a 9-char prompt minus headroom).
        let at_end = visible_window(&text, 26, 20, 9);
        assert_eq!(at_end.chars().count(), 10);
        assert_eq!(at_end, "qrstuvwxyz", "the end of the text stays visible");

        // Cursor moved back into the middle: the window follows it.
        let middle = visible_window(&text, 12, 20, 9);
        assert_eq!(middle.chars().count(), 10);
        assert!(
            middle.contains('l') && middle.contains('m'),
            "the window must contain the cursor: {middle}"
        );

        // And at the very start.
        let start = visible_window(&text, 0, 20, 9);
        assert_eq!(start, "abcdefghij");
    }
}
