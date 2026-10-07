//! The dialogs that ask: a line of text, a note, a yes or no, each sized to
//! what it holds.

use super::*;

pub(super) fn render_text_prompt(
    app: &App,
    prompt: &TextPrompt,
    frame: &mut Frame,
    area: Rect,
) -> Position {
    use crate::tui::app::actions::TextThen;

    let theme = &app.theme;
    let verb = match prompt.then {
        TextThen::Rename(_) => "rename",
        TextThen::AddTag(_) => "add a tag",
        TextThen::AddTodo { .. } => "add a todo",
        TextThen::AddPhase(_) => "add a phase",
        TextThen::Describe(_) => "describe",
        TextThen::Delete(_) => "delete",
        TextThen::RaiseCounter => "ID counter",
        TextThen::CopyTo(_) => "copy to",
        TextThen::DiscardAttention(_) => "discard",
    };
    // The box grows with its question: a confirmation over six marked
    // folders names all six.
    let (width, prompt_rows) = question_size(area, &prompt.title, 62, 6);
    let area = centered_fixed(area, width, prompt_rows + 6);
    super::clear(frame, area, &app.theme);
    let block = frame_block(app, format!(" {} ", verb), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The prompt itself, wrapped, above the input line — inset a column, so
    // every wrapped line sits where the first one does.
    let prompt_area = inset(Rect::new(inner.x, inner.y, inner.width, prompt_rows));
    frame.render_widget(
        Paragraph::new(Span::styled(prompt.title.clone(), theme.dim())).wrap(Wrap { trim: false }),
        prompt_area,
    );

    let input_area = Rect::new(inner.x, inner.y + prompt_rows + 1, inner.width, 1);
    let caret = prompt
        .input
        .render_line(input_area, frame.buffer_mut(), Span::raw(" "), theme.text())
        .unwrap_or(Position::new(inner.x, input_area.y));
    if let Some(error) = &prompt.error {
        let error_area = Rect::new(inner.x, input_area.y + 1, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {error}"), theme.warn())),
            error_area,
        );
    }
    caret
}

/// One column of padding on the left: wrapped text drawn here keeps every
/// line where the first one starts, which a leading space cannot do.
pub(super) fn inset(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(1),
        area.height,
    )
}

/// How many rows `text` takes when word-wrapped at `width` columns — the
/// greedy wrap `Paragraph::wrap` does, a word at a time and a long word
/// broken where it must be — so a box can be sized to its question.
pub(super) fn wrapped_rows(text: &str, width: usize) -> usize {
    let width = width.max(1);
    text.lines()
        .map(|line| {
            let mut rows = 1usize;
            let mut used = 0usize;
            for word in line.split(' ') {
                let w = word.width();
                if used == 0 {
                    used = w;
                } else if used + 1 + w <= width {
                    used += 1 + w;
                } else {
                    rows += 1;
                    used = w;
                }
                while used > width {
                    rows += 1;
                    used -= width;
                }
            }
            rows
        })
        .sum()
}

/// The quick note: a small text area over the dashboard. Enter saves,
/// Alt-Enter breaks a line, and a pasted paragraph lands whole.
pub(super) fn render_note(
    app: &App,
    note: &crate::tui::app::actions::NoteState,
    frame: &mut Frame,
    area: Rect,
) -> Position {
    let theme = &app.theme;
    let g = theme.glyphs;
    let rows = (note.area.lines().len() as u16).clamp(3, 8);
    let area = centered_fixed(area, 62, rows + 5);
    super::clear(frame, area, &app.theme);
    let title = if note.count() > 1 {
        format!(" note {} {} projects ", g.sep, note.count())
    } else {
        " note ".to_string()
    };
    let block = frame_block(app, title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", crate::tui::validators::note_prompt(note.count())),
            theme.dim(),
        )),
        prompt_area,
    );
    let text_area = Rect::new(
        inner.x + 1,
        inner.y + 1,
        inner.width.saturating_sub(1),
        rows,
    );
    let caret = note
        .area
        .render(text_area, frame.buffer_mut(), theme.text())
        .unwrap_or(Position::new(text_area.x, text_area.y));
    let keys_area = Rect::new(inner.x, inner.y + 1 + rows, inner.width, 1);
    // Cut at a whole pair, the way out before the extra: half a pair names a
    // key that is not one.
    frame.render_widget(
        Paragraph::new(crate::tui::view::builder::key_line(
            theme,
            &[
                ("Enter", "save"),
                ("Esc", "cancel"),
                ("Alt-Enter", "new line"),
            ],
            inner.width as usize,
        )),
        keys_area,
    );
    caret
}

/// The width a question's box will get, and how many rows its text takes there.
///
/// **Measured at the width it will actually be drawn at**: rows counted at a
/// wanted width that `centered_fixed` then narrows to the screen are too few,
/// and the tail of the question is cut.
///
/// The row ceiling is the screen too, not a constant. `validators::delete_prompt`
/// over six long folder names goes past eight wrapped rows easily, and a
/// destructive confirmation must never hide part of what it is about
/// (`src/tui/CLAUDE.md` › Layout).
fn question_size(area: Rect, question: &str, wanted: u16, chrome: u16) -> (u16, u16) {
    let width = wanted.min(area.width);
    let rows = wrapped_rows(question, usize::from(width).saturating_sub(3)) as u16;
    // The screen is the only ceiling.
    let ceiling = area.height.saturating_sub(chrome);
    (
        width,
        crate::tui::layout::fit_between(rows, 1, ceiling.max(1)),
    )
}

pub(super) fn render_confirm(
    app: &App,
    confirm: &Confirm,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    // Sized to its question, which names every folder it is about.
    let (width, rows) = question_size(area, &confirm.prompt, 64, 5);
    let area = centered_fixed(area, width, rows + 5);
    super::clear(frame, area, &app.theme);
    let block = frame_block(app, " confirm ".to_string(), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt_area = inset(Rect::new(inner.x, inner.y, inner.width, rows));
    frame.render_widget(
        Paragraph::new(Span::styled(confirm.prompt.clone(), theme.text()))
            .wrap(Wrap { trim: false }),
        prompt_area,
    );
    let keys_area = Rect::new(inner.x, inner.y + rows + 1, inner.width, 1);
    frame.render_widget(
        Paragraph::new(crate::tui::view::builder::key_line(
            theme,
            &[("y / Enter", "yes"), ("n", "no"), ("Esc", "cancel")],
            inner.width as usize,
        )),
        keys_area,
    );
    None
}
