//! The command line's prompts: a few rows at the cursor, in the app's palette.
//!
//! A prompt here is **inline**. It reserves the rows it needs where the cursor
//! already is, redraws them in place, and takes them back when it is answered,
//! leaving one line of transcript behind. It never switches to the alternate
//! screen, so `fastf new`'s preview stays above the question that follows it and
//! the shell keeps the whole run — which is exactly the difference between a
//! prompt and the guided app.
//!
//! It replaces `dialoguer`. The line editor, the palette and the glyphs are the
//! app's (`widgets::input::LineEdit`, `theme::Theme`), which is what makes
//! `fastf copy lullaby`'s picker and the guided app look like one tool.
//!
//! **Every movement is relative, and the cursor position is never queried.**
//! ratatui's `Viewport::Inline` is the obvious way to do this and the wrong one:
//! it asks the terminal where the cursor is (`ESC [ 6 n`) and waits up to two
//! seconds for an answer. A pty under test never sends one — `cargo test`
//! failed on it immediately — and neither does every real terminal. The same
//! trap already cost this codebase `Terminal::clear` (`src/tui/CLAUDE.md`), and
//! it would be worse here: a query on the command-line path would put a stall
//! in front of `fastf copy`, which exists to be instant. So the block is
//! reserved by printing newlines, and every repaint is `move up n`, draw, and
//! nothing else.
//!
//! **Everything is drawn on stderr**, the stream fastf has always prompted on,
//! so `cd "$(fastf path lullaby)"` still gets a picker and stdout still carries
//! nothing but the path.
//!
//! `Ok(None)` is a cancelled prompt and never an error: `cli` classifies a
//! *broken* prompt (no terminal, stdin at EOF) as fatal and a cancelled one as
//! an ordinary answer.

use std::io::{self, Write};

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::style::{Color, Modifier, Style};

use crate::tui::command::Key;
use crate::tui::theme::Theme;
use crate::tui::view::fit;
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;

/// The most rows a picker takes, however many candidates there are. The block
/// scrolls the terminal to make room, so a tall one pushes away the output the
/// question is about.
const MAX_ROWS: usize = 12;

/// One drawn row: styled runs, left to right.
type Row = Vec<(String, Style)>;

/// The configured `theme`, if the config can be read. A prompt is a cosmetic
/// surface and the command that opened it has already loaded — and would
/// already have stopped on — a config that cannot be parsed, so nothing is
/// masked by falling back to the environment alone here.
fn theme_preference() -> Option<String> {
    crate::core::config::Config::load()
        .ok()
        .map(|config| config.theme)
}

/// The alphabet the command line's prompts draw with — the app's own choice,
/// so `FASTF_ASCII=1` reaches the pickers too.
pub fn glyphs() -> crate::tui::theme::Glyphs {
    Theme::detect_with(theme_preference().as_deref()).glyphs
}

/// The terminal, for as long as one prompt lasts.
///
/// Raw mode and the reserved rows are taken on the way in and given back on the
/// way out — including on the error path, which is why this is a guard and not
/// a pair of calls.
struct Inline {
    theme: Theme,
    /// How many rows the block occupies. Fixed for the prompt's lifetime, so a
    /// repaint always knows exactly what to take back.
    height: usize,
    columns: usize,
    /// Whether raw mode and the rows are still ours.
    open: bool,
}

impl Inline {
    fn open(height: usize) -> Result<Self> {
        enable_raw_mode().context("putting the terminal into raw mode")?;
        let columns = ratatui::crossterm::terminal::size()
            .map(|(columns, _)| columns as usize)
            .unwrap_or(80)
            .max(20);
        let inline = Self {
            theme: Theme::detect_with(theme_preference().as_deref()),
            height,
            columns,
            open: true,
        };

        // Reserve the rows by scrolling them into existence, then come back to
        // the top of the block. Nothing is asked of the terminal.
        let mut out = String::from("\x1b[?25l");
        for _ in 0..height {
            out.push_str("\r\n");
        }
        out.push_str(&up(height));
        inline.emit(&out);
        Ok(inline)
    }

    fn emit(&self, text: &str) {
        let mut stderr = io::stderr();
        let _ = stderr.write_all(text.as_bytes());
        let _ = stderr.flush();
    }

    /// Repaint the block. `caret` is `(row, column)` within it, and the cursor
    /// is shown there — a text field with no visible insertion point is the
    /// regression that cost a release.
    fn paint(&self, rows: &[Row], caret: Option<(usize, usize)>) {
        let mut out = String::from("\x1b[?25l");
        for index in 0..self.height {
            out.push('\r');
            // Clear to the end of the line before drawing it: a shorter row
            // must not leave the tail of a longer one behind.
            out.push_str("\x1b[K");
            if let Some(row) = rows.get(index) {
                let mut used = 0usize;
                for (text, style) in row {
                    let room = self.columns.saturating_sub(used + 1);
                    if room == 0 {
                        break;
                    }
                    let shown = fit(text, room, self.theme.glyphs.ellipsis);
                    used += unicode_width::UnicodeWidthStr::width(shown.as_str());
                    out.push_str(&paint_span(&shown, *style));
                }
            }
            if index + 1 < self.height {
                out.push_str("\r\n");
            }
        }
        // Back to the top of the block, then down to where the caret goes.
        out.push_str(&up(self.height.saturating_sub(1)));
        match caret {
            Some((row, column)) => {
                out.push('\r');
                if row > 0 {
                    out.push_str(&format!("\x1b[{row}B"));
                }
                if column > 0 {
                    out.push_str(&format!("\x1b[{}C", column.min(self.columns - 1)));
                }
                out.push_str("\x1b[?25h");
            }
            None => out.push('\r'),
        }
        self.emit(&out);
    }

    /// One keystroke, normalised the way the app normalises them. Anything that
    /// is not a key press — a resize, a paste, a mouse report — is skipped.
    fn key(&self) -> Result<Key> {
        loop {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => return Ok(key.into()),
                _ => continue,
            }
        }
    }

    /// Take the rows back and leave `line` behind as the record of what was
    /// answered. A cancelled prompt says so rather than vanishing: a transcript
    /// that skips a question nobody answered reads like it was never asked.
    fn close(mut self, line: Row) {
        self.open = false;
        let mut out = String::from("\r\x1b[J");
        for (text, style) in &line {
            out.push_str(&paint_span(text, *style));
        }
        out.push_str("\r\n\x1b[?25h");
        let _ = disable_raw_mode();
        self.emit(&out);
    }
}

impl Drop for Inline {
    fn drop(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        // The error path: give the rows and the cursor back whatever happened.
        self.emit("\r\x1b[J\x1b[?25h");
        let _ = disable_raw_mode();
    }
}

/// `n` rows up, or nothing at all — `ESC [ 0 A` is one row on some terminals.
fn up(n: usize) -> String {
    if n == 0 {
        String::new()
    } else {
        format!("\x1b[{n}A")
    }
}

/// One styled run as SGR. Only what the theme actually uses: a foreground
/// colour and the three modifiers, then a reset.
fn paint_span(text: &str, style: Style) -> String {
    let mut codes: Vec<String> = Vec::new();
    if let Some(color) = style.fg {
        codes.push(foreground(color));
    }
    if style.add_modifier.contains(Modifier::BOLD) {
        codes.push("1".to_string());
    }
    if style.add_modifier.contains(Modifier::DIM) {
        codes.push("2".to_string());
    }
    if style.add_modifier.contains(Modifier::REVERSED) {
        codes.push("7".to_string());
    }
    if codes.is_empty() {
        return text.to_string();
    }
    format!("\x1b[{}m{text}\x1b[0m", codes.join(";"))
}

fn foreground(color: Color) -> String {
    match color {
        Color::Reset => "39".to_string(),
        Color::Black => "30".to_string(),
        Color::Red => "31".to_string(),
        Color::Green => "32".to_string(),
        Color::Yellow => "33".to_string(),
        Color::Blue => "34".to_string(),
        Color::Magenta => "35".to_string(),
        Color::Cyan => "36".to_string(),
        Color::Gray => "37".to_string(),
        Color::DarkGray => "90".to_string(),
        Color::LightRed => "91".to_string(),
        Color::LightGreen => "92".to_string(),
        Color::LightYellow => "93".to_string(),
        Color::LightBlue => "94".to_string(),
        Color::LightMagenta => "95".to_string(),
        Color::LightCyan => "96".to_string(),
        Color::White => "97".to_string(),
        Color::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
        Color::Indexed(i) => format!("38;5;{i}"),
    }
}

// ---------------------------------------------------------------------------
// The picker
// ---------------------------------------------------------------------------

/// Pick one of `items`. `Ok(None)` is Esc or `q`.
///
/// Deliberately not filterable. This is the picker a verb interrupted —
/// `fastf copy lullaby` matching three projects — and its whole job is to be
/// answered in one or two keystrokes over a list a query has already narrowed.
/// Fuzzy search lives in the app, where there is a library to search.
pub fn select(prompt: &str, items: &[String], default: usize) -> Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }
    let rows = items.len().min(MAX_ROWS);
    let inline = Inline::open(rows + 1)?;
    let theme = inline.theme.clone();
    let g = theme.glyphs;
    let mut selected = default.min(items.len() - 1);
    let mut offset = 0usize;

    let answer = loop {
        offset = nav::viewport_offset(offset, Some(selected), items.len(), rows);
        let mut drawn: Vec<Row> = vec![vec![
            (format!("{} ", g.search), theme.accent()),
            (prompt.to_string(), theme.bold()),
        ]];
        for (index, item) in items.iter().enumerate().skip(offset).take(rows) {
            let chosen = index == selected;
            drawn.push(vec![
                (
                    format!("{} ", if chosen { g.cursor } else { " " }),
                    theme.accent(),
                ),
                (
                    item.clone(),
                    if chosen {
                        theme.selection
                    } else {
                        theme.text()
                    },
                ),
            ]);
        }
        inline.paint(&drawn, None);

        let key = inline.key()?;
        match key.code {
            KeyCode::Enter => break Some(selected),
            KeyCode::Esc | KeyCode::Char('q') => break None,
            KeyCode::Up | KeyCode::Char('k') => {
                selected = nav::wrap_step(Some(selected), items.len(), -1).unwrap_or(0);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                selected = nav::wrap_step(Some(selected), items.len(), 1).unwrap_or(0);
            }
            KeyCode::Home => selected = 0,
            KeyCode::End => selected = items.len() - 1,
            KeyCode::PageUp => {
                selected =
                    nav::clamp_jump(Some(selected), items.len(), -(rows as isize)).unwrap_or(0);
            }
            KeyCode::PageDown => {
                selected = nav::clamp_jump(Some(selected), items.len(), rows as isize).unwrap_or(0);
            }
            _ => {}
        }
    };

    inline.close(answered(
        &theme,
        prompt,
        answer.map(|at| items[at].as_str()),
    ));
    Ok(answer)
}

/// The one line a prompt leaves behind: the question, and what it was answered
/// with.
fn answered(theme: &Theme, prompt: &str, chosen: Option<&str>) -> Row {
    match chosen {
        Some(value) => vec![
            (format!("{} ", theme.glyphs.check), theme.good()),
            (format!("{prompt} "), theme.dim()),
            (value.to_string(), theme.text()),
        ],
        None => vec![
            (format!("{} ", theme.glyphs.cross), theme.dim()),
            (format!("{prompt} "), theme.dim()),
            ("cancelled".to_string(), theme.dim()),
        ],
    }
}

// ---------------------------------------------------------------------------
// Yes or no
// ---------------------------------------------------------------------------

/// A bare `y`/`n` answers without Enter, Enter takes the default, Esc or `q`
/// cancels. The bare-key contract is worth keeping: it is what makes
/// `fastf new`'s confirmation one keystroke.
pub fn confirm(prompt: &str, default: bool) -> Result<Option<bool>> {
    let inline = Inline::open(1)?;
    let theme = inline.theme.clone();
    let suffix = if default { "[Y/n]" } else { "[y/N]" };

    let answer = loop {
        inline.paint(
            &[vec![
                (format!("{prompt} "), theme.bold()),
                (suffix.to_string(), theme.dim()),
            ]],
            None,
        );
        let key = inline.key()?;
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => break Some(true),
            KeyCode::Char('n') | KeyCode::Char('N') => break Some(false),
            KeyCode::Enter => break Some(default),
            KeyCode::Esc | KeyCode::Char('q') => break None,
            _ => {}
        }
    };

    inline.close(answered(
        &theme,
        prompt,
        answer.map(|yes| if yes { "yes" } else { "no" }),
    ));
    Ok(answer)
}

// ---------------------------------------------------------------------------
// A line of text
// ---------------------------------------------------------------------------

/// What a text prompt starts with and what it will accept.
#[derive(Default)]
pub struct TextOpts<'a> {
    /// Editable starting text, not a hint: the cursor lands at its end and
    /// Backspace works into it. This is what a retry uses, so a value rejected
    /// by a validator is corrected rather than retyped.
    pub initial: Option<String>,
    /// A value an empty answer means, shown as `prompt [default]:` — a
    /// different gesture from editing the old one.
    pub default: Option<String>,
    /// Whether an empty answer submits.
    pub allow_empty: bool,
    /// Runs on Enter. `Err` keeps the text on the line and shows the message
    /// under it.
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

/// Read a line. `Ok(None)` is Esc.
///
/// The caret is parked in the text being edited. Leaving it at the start of the
/// line — or hidden — is the regression that cost a release: the text moved as
/// you typed and nothing said where the next character would land.
pub fn text(prompt: &str, opts: TextOpts<'_>) -> Result<Option<String>> {
    let inline = Inline::open(2)?;
    let theme = inline.theme.clone();
    let mut input = LineEdit::with_text(opts.initial.clone().unwrap_or_default());
    let head = match &opts.default {
        Some(default) => format!("{prompt} [{default}]: "),
        None => format!("{prompt}: "),
    };
    let head_width = unicode_width::UnicodeWidthStr::width(head.as_str());
    let mut error: Option<String> = None;

    let answer = loop {
        // Windowed, not wrapped: a soft-wrapped line is what ghosts on the
        // legacy Windows console, and it is also what would make the block
        // taller than the rows reserved for it.
        let (shown, caret) = crate::tui::widgets::input::visible_window(
            input.text(),
            input.cursor(),
            inline.columns,
            head_width,
        );
        let mut rows: Vec<Row> = vec![vec![(head.clone(), theme.bold()), (shown, theme.text())]];
        rows.push(match &error {
            Some(message) => vec![(format!("{} {message}", theme.glyphs.warn), theme.warn())],
            None => Vec::new(),
        });
        inline.paint(&rows, Some((0, head_width + caret)));

        let key = inline.key()?;
        match key.code {
            KeyCode::Esc if !key.ctrl => break None,
            KeyCode::Enter => {
                let mut value = input.text().to_string();
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
                    // The text stays on the line: a rejected answer is
                    // corrected, never retyped.
                    error = Some(message);
                    continue;
                }
                break Some(value);
            }
            _ => {
                if input.apply(&key) {
                    error = None;
                }
            }
        }
    };

    inline.close(answered(&theme, prompt, answer.as_deref()));
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::{foreground, paint_span, up};
    use ratatui::style::{Color, Modifier, Style};

    #[test]
    fn an_unstyled_run_is_written_as_it_is() {
        assert_eq!(paint_span("plain", Style::default()), "plain");
    }

    #[test]
    fn a_styled_run_carries_its_colour_and_its_modifiers() {
        let style = Style::default()
            .fg(Color::Rgb(1, 2, 3))
            .add_modifier(Modifier::BOLD);
        assert_eq!(
            paint_span("x", style),
            "\x1b[38;2;1;2;3;1mx\x1b[0m",
            "truecolor first, then bold, then a reset"
        );
        assert_eq!(foreground(Color::Indexed(9)), "38;5;9");
    }

    #[test]
    fn moving_up_nothing_moves_nothing() {
        // `ESC [ 0 A` is one row on some terminals, which would eat the line
        // above a one-row prompt on every repaint.
        assert_eq!(up(0), "");
        assert_eq!(up(3), "\x1b[3A");
    }
}
