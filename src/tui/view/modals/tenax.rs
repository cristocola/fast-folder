//! The chat with Tenax: the transcript over a line to type into.
//!
//! `transcript_lines` is the one answer to "what rows does the transcript
//! have at this width", read by the view and by `update`, which clamps the
//! scroll to it — a scroll counted any other way runs past the end.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::App;
use crate::tui::app::pane::wrap_columns;
use crate::tui::app::tenax::{Chat, Said};
use crate::tui::command::{self, CommandId, Context};

/// The labels' column: `tenax` and a space.
const LABEL: usize = 7;

/// Where the transcript is drawn inside the chat's dialog: inside the border,
/// a column of padding each side, above the footer, the key line, the rule
/// and the input line.
pub fn transcript_rect(dialog: Rect) -> Rect {
    Rect::new(
        dialog.x.saturating_add(2),
        dialog.y.saturating_add(1),
        dialog.width.saturating_sub(4),
        dialog.height.saturating_sub(6),
    )
}

/// The transcript as drawn rows at `width` columns: what he and the person
/// said, what came of each change he proposed, and his brief at the top when
/// nothing has been said yet.
pub fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let text_width = width.saturating_sub(LABEL).max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut entry = |label: &str, label_style, text: &str, style| {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let mut first = true;
        for paragraph in text.split('\n') {
            for row in wrap_columns(paragraph, text_width) {
                let lead = if first {
                    format!("{label:<LABEL$}")
                } else {
                    " ".repeat(LABEL)
                };
                first = false;
                lines.push(Line::from(vec![
                    Span::styled(lead, label_style),
                    Span::styled(row, style),
                ]));
            }
        }
    };
    if app.tenax.transcript.is_empty() {
        match &app.tenax.brief {
            Some(brief) => {
                let mut said = brief.say.clone();
                if let Some(start) = &brief.start
                    && !start.project.is_empty()
                {
                    said.push_str(&format!(
                        "\n{} {} {} {} {}",
                        g.cursor, start.project, start.folder, g.sep, start.todo
                    ));
                }
                entry("tenax", theme.dim(), &said, theme.text());
            }
            None => entry("tenax", theme.dim(), INTRO, theme.text()),
        }
    }
    for said in &app.tenax.transcript {
        match said {
            Said::You(text) => entry("you", theme.dim(), text, theme.text()),
            Said::Tenax(text) => entry("tenax", theme.dim(), text, theme.text()),
            Said::Done(text) => entry(g.check, theme.good(), text, theme.dim()),
            Said::Declined(text) => entry(
                g.cross,
                theme.dim(),
                &format!("declined: {text}"),
                theme.dim(),
            ),
            Said::Trouble(text) => entry(g.warn, theme.warn(), text, theme.warn()),
        }
    }
    if app.tenax.chat_request.is_some() {
        entry(
            g.spin(app.elapsed_ms),
            theme.dim(),
            &format!("thinking{}", g.ellipsis),
            theme.dim(),
        );
    }
    lines
}

/// What the chat says before anything has been.
const INTRO: &str = "Ask me what to start on, how to do something in fastf, or to add a todo or a note. \
I ask before I change anything.";

/// Draw the chat. Returns where the caret is.
pub fn render_chat(app: &App, chat: &Chat, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    let dialog = crate::tui::layout::tenax_chat(area);
    let (body, footer, keys) =
        crate::tui::view::builder::frame_parts(app, " Tenax ".to_string(), frame, dialog)?;
    if body.height < 3 {
        return None;
    }
    let transcript = transcript_rect(dialog);
    let lines = transcript_lines(app, transcript.width as usize);
    let rows = transcript.height as usize;
    let max = lines.len().saturating_sub(rows);
    let scroll = if chat.follow {
        max
    } else {
        chat.scroll.min(max)
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), transcript);
    if max > 0 {
        let mut state = ratatui::widgets::ScrollbarState::new(max)
            .position(scroll)
            .viewport_content_length(rows);
        frame.render_stateful_widget(
            crate::tui::view::scrollbar(&theme.glyphs),
            Rect::new(
                dialog.x + dialog.width - 1,
                transcript.y,
                1,
                transcript.height,
            ),
            &mut state,
        );
    }

    let rule_y = body.y + body.height - 2;
    let rule = Rect::new(body.x, rule_y, body.width, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(
            theme.glyphs.rule.repeat(body.width as usize),
            theme.dim(),
        )),
        rule,
    );
    let input = Rect::new(body.x + 1, rule_y + 1, body.width.saturating_sub(2), 1);
    let caret = chat.input.render_line(
        input,
        frame.buffer_mut(),
        Span::styled(format!("{} ", theme.glyphs.cursor), theme.accent()),
        theme.text(),
    );

    let note = if app.tenax.held.is_some() {
        format!(
            " {} he has a change to put to you — clear the line and it comes up",
            theme.glyphs.warn
        )
    } else {
        format!(" {} at high effort", app.tenax.prefs.model)
    };
    crate::tui::view::builder::footer_line(
        frame,
        footer,
        &crate::tui::view::fit(&note, footer.width as usize, theme.glyphs.ellipsis),
        theme.dim(),
    );
    let mut pairs: Vec<(String, String)> = vec![
        (
            command::key_of(CommandId::PromptConfirm),
            "send".to_string(),
        ),
        (
            command::key_of(CommandId::PromptCancel),
            "close".to_string(),
        ),
        (
            command::key_of(CommandId::TenaxNewChat),
            "new chat".to_string(),
        ),
    ];
    if let Some((keys, what)) = command::movement_pair(Context::Chat, &theme.glyphs) {
        pairs.push((keys, what.to_string()));
    }
    frame.render_widget(
        Paragraph::new(crate::tui::view::builder::key_line(
            theme,
            &pairs,
            keys.width as usize,
        )),
        keys,
    );
    caret
}
