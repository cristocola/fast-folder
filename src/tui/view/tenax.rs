//! Tenax's box at the top of the sidebar: his face on the left, what he last
//! said on the right, the key that talks to him on the bottom border.
//!
//! Never focused — the accent says what has the keys, and he never does. His
//! text is cut, never wrapped past the box: the whole of it is in the chat.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::app::App;
use crate::tui::app::pane::wrap_columns;
use crate::tui::app::tenax::Mood;
use crate::tui::command::{self, CommandId};
use crate::tui::sprite;
use crate::tui::view::fit;

/// Columns from the box's inner edge to his text: a space, his face, two
/// spaces.
const TEXT_AT: u16 = 1 + sprite::WIDTH + 2;

/// Draw the box into `area`.
pub fn panel(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let talk = format!(" {} talk ", command::key_of(CommandId::Tenax));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Tenax ", theme.dim()))
        .title_bottom(Line::from(Span::styled(talk, theme.dim())).right_aligned())
        .border_style(theme.border(false));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < sprite::WIDTH + 1 || inner.height == 0 {
        return;
    }

    let face = app.tenax_frame();
    for (row, line) in face.iter().enumerate().take(inner.height as usize) {
        frame.render_widget(
            Paragraph::new(Span::styled(*line, theme.text())),
            Rect::new(inner.x + 1, inner.y + row as u16, sprite::WIDTH, 1),
        );
    }

    if inner.width <= TEXT_AT + 4 {
        return;
    }
    let text_area = Rect::new(
        inner.x + TEXT_AT,
        inner.y,
        inner.width - TEXT_AT - 1,
        inner.height,
    );
    let lines = panel_lines(app, text_area.width as usize, text_area.height as usize);
    frame.render_widget(Paragraph::new(lines), text_area);
}

/// What the box says, as at most `rows` lines of `width` columns.
pub fn panel_lines(app: &App, width: usize, rows: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let g = theme.glyphs;
    if rows == 0 || width == 0 {
        return Vec::new();
    }
    // The last line, when there is one, says what he is doing or waiting on;
    // the lines above it what he said.
    let (status, status_style) = if app.tenax.thinking() {
        (
            Some(format!("{} thinking{}", g.spin(app.elapsed_ms), g.ellipsis)),
            theme.dim(),
        )
    } else if app.tenax.asking.is_some() || app.tenax.held.is_some() {
        (
            Some(format!(
                "{} a change for you: {} to answer",
                g.warn,
                command::key_of(CommandId::Tenax)
            )),
            theme.warn(),
        )
    } else {
        (None, theme.dim())
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let room = rows - usize::from(status.is_some());
    match &app.tenax.mood {
        Mood::Trouble(sentence) if !app.tenax.thinking() => {
            push_wrapped(&mut lines, sentence, width, room, theme.warn(), g.ellipsis);
        }
        _ => {
            if let Some(said) = app.tenax.last_said() {
                push_wrapped(&mut lines, said, width, room, theme.text(), g.ellipsis);
            } else if let Some(brief) = &app.tenax.brief {
                let start = brief
                    .start
                    .as_ref()
                    .filter(|start| !start.project.is_empty());
                let say_rows = if start.is_some() && room > 1 {
                    room - 1
                } else {
                    room
                };
                push_wrapped(
                    &mut lines,
                    &brief.say,
                    width,
                    say_rows,
                    theme.text(),
                    g.ellipsis,
                );
                if let Some(start) = start
                    && lines.len() < room
                {
                    let line = format!("{} {} {} {}", g.cursor, start.project, g.sep, start.todo);
                    lines.push(Line::from(Span::styled(
                        fit(&line, width, g.ellipsis),
                        theme.dim(),
                    )));
                }
            } else {
                let hello = format!(
                    "Ready when you are. {} to talk.",
                    command::key_of(CommandId::Tenax)
                );
                push_wrapped(&mut lines, &hello, width, room, theme.dim(), g.ellipsis);
            }
        }
    }
    if let Some(status) = status {
        while lines.len() < rows - 1 {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            fit(&status, width, g.ellipsis),
            status_style,
        )));
    }
    lines
}

/// `text` wrapped to `width`, at most `rows` lines, the last cut with the
/// ellipsis when there was more.
fn push_wrapped(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    rows: usize,
    style: ratatui::style::Style,
    ellipsis: &str,
) {
    let wrapped: Vec<String> = text
        .split('\n')
        .filter(|paragraph| !paragraph.trim().is_empty())
        .flat_map(|paragraph| wrap_columns(paragraph.trim(), width))
        .collect();
    let cut = wrapped.len() > rows;
    for (at, row) in wrapped.into_iter().take(rows).enumerate() {
        let row = if cut && at + 1 == rows {
            let mut shown = row;
            while !shown.is_empty()
                && unicode_width::UnicodeWidthStr::width(shown.as_str())
                    + unicode_width::UnicodeWidthStr::width(ellipsis)
                    > width
            {
                shown.pop();
            }
            format!("{}{ellipsis}", shown.trim_end())
        } else {
            row
        };
        lines.push(Line::from(Span::styled(row, style)));
    }
}
