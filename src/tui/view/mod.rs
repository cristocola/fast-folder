//! Drawing the app. `view` takes `&App` — it cannot change state, so what a
//! frame shows is exactly what `update` left behind, and a snapshot test can
//! render any state it can construct.

pub mod builder;
pub mod dashboard;
pub mod modals;
pub mod projects;
pub mod templates;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::App;
use crate::tui::layout;

pub fn view(app: &App, frame: &mut Frame) {
    draw(app, frame);
    paint_canvas(frame.buffer_mut(), &app.theme);
}

/// **A theme with a canvas paints it here, once, after everything is drawn.**
/// Every cell still on the terminal's own background gets the canvas, and every
/// cell still in the terminal's own text colour gets the theme's — so a
/// `Clear` under a dialog, a gap between widgets or a widget added later can
/// never leave a hole in it. Only Doom One has a canvas; the other palettes
/// leave the terminal's colours alone and this does nothing.
pub fn paint_canvas(buffer: &mut ratatui::buffer::Buffer, theme: &crate::tui::theme::Theme) {
    use ratatui::style::Color;
    if theme.canvas == Color::Reset {
        return;
    }
    for cell in buffer.content.iter_mut() {
        if cell.bg == Color::Reset {
            cell.bg = theme.canvas;
        }
        if cell.fg == Color::Reset {
            cell.fg = theme.text;
        }
    }
}

/// Blank `area` for a dialog: `Clear`, then the theme's `surface` under it,
/// so a dialog on a painted canvas sits on a shade of its own (Doom's popups
/// wear `bg-alt`). Where there is no canvas the terminal's background stays.
pub(crate) fn clear(frame: &mut Frame, area: Rect, theme: &crate::tui::theme::Theme) {
    frame.render_widget(ratatui::widgets::Clear, area);
    if theme.surface != ratatui::style::Color::Reset {
        frame.render_widget(
            ratatui::widgets::Block::default().style(Style::default().bg(theme.surface)),
            area,
        );
    }
}

fn draw(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if layout::too_small(area) {
        render_too_small(app, frame, area);
        return;
    }
    let regions = layout::regions(area, app.pane_live(), app.table_needs());

    dashboard::header(app, frame, regions.header);
    // The two tabs share every band but the middle one, so the chrome — the
    // name, the tabs, the bases, the status line, the keys — stays where it is
    // when you switch, and only the work changes.
    let (search_caret, pane_caret) = match app.screen {
        crate::tui::app::Screen::Library => {
            let caret = dashboard::search_bar(app, frame, regions.search);
            // The pane in the list's place is drawn only while it has the
            // focus, and then instead of the table; beside or under the
            // table, it is drawn with it.
            let over = regions.placement == Some(layout::Placement::Over);
            let pane_caret = match regions.detail {
                Some(pane) if over && app.focus == crate::tui::app::Focus::Detail => {
                    projects::detail(app, frame, pane)
                }
                Some(pane) if !over => {
                    projects::table(app, frame, regions.table);
                    projects::detail(app, frame, pane)
                }
                _ => {
                    projects::table(app, frame, regions.table);
                    None
                }
            };
            (caret, pane_caret)
        }
        crate::tui::app::Screen::Templates => {
            let caret = templates::bar(app, frame, regions.search);
            templates::screen(app, frame, regions.body);
            (caret, None)
        }
    };
    dashboard::status(app, frame, regions.status);
    dashboard::hints(app, frame, regions.hints);

    let modal_caret = modals::render(app, frame, area);
    modals::render_move_progress(app, frame, area);
    modals::render_job(app, frame, area);

    // The terminal's own cursor goes where typing lands: a dialog's field, the
    // search bar while it is being typed into, or an edit open in the pane —
    // one at a time, since a field under a dialog is not the one being typed.
    let caret = if modal_caret.is_some() {
        modal_caret
    } else if !app.modals.is_empty() {
        None
    } else if app.search.editing {
        search_caret
    } else if app.pane_edit.is_some() {
        pane_caret
    } else {
        None
    };
    if let Some(caret) = caret {
        frame.set_cursor_position(caret);
    }
}

fn render_too_small(app: &App, frame: &mut Frame, area: Rect) {
    use crate::tui::app::actions::{Confirm, ConfirmThen};
    use crate::tui::app::modal::Modal;
    use crate::tui::command::{CommandId, key_of};

    let theme = &app.theme;
    // Which side is short, and by how much — a person dragging a corner wants
    // to know which way, and a split pane is short on one side only.
    let columns = layout::MIN_WIDTH.saturating_sub(area.width);
    let rows = layout::MIN_HEIGHT.saturating_sub(area.height);
    let short = |n: u16, one: &str, many: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    };
    let what = match (columns, rows) {
        (0, rows) => format!("{} short", short(rows, "row", "rows")),
        (columns, 0) => format!("{} too narrow", short(columns, "column", "columns")),
        (columns, rows) => format!(
            "{} too narrow and {} short",
            short(columns, "column", "columns"),
            short(rows, "row", "rows")
        ),
    };
    let mut text = vec![
        Line::from(Span::styled(
            format!(
                "fastf needs {}×{} or more",
                layout::MIN_WIDTH,
                layout::MIN_HEIGHT
            ),
            theme.warn(),
        )),
        Line::from(Span::styled(
            format!("this window is {}×{}: {what}", area.width, area.height),
            theme.dim(),
        )),
    ];
    // A dialog cannot be drawn here, so a question it is waiting on is asked
    // in words: the quit gesture works on this screen, and a template worked
    // on must not be thrown away by a second key nobody could see the
    // question for.
    let quit = key_of(CommandId::Quit);
    match app.modals.top() {
        Some(Modal::Confirm(Confirm {
            then: ConfirmThen::DiscardTemplate { then_quit: Some(_) },
            ..
        })) => text.push(Line::from(Span::styled(
            format!(
                "a template has unsaved changes — make the window bigger to keep it, \
                 or press {quit} again to throw it away and quit"
            ),
            theme.warn(),
        ))),
        Some(Modal::Builder(builder)) if builder.is_dirty() => {
            text.push(Line::from(Span::styled(
                "a template has unsaved changes — make the window bigger to keep working on it",
                theme.warn(),
            )));
            text.push(Line::from(Span::styled(
                format!("or press {quit} to be asked about it"),
                theme.dim(),
            )));
        }
        _ => text.push(Line::from(Span::styled(
            format!("make it bigger, or press {quit} to quit"),
            theme.dim(),
        ))),
    }
    // Centred, and no box: a frame around three lines costs a small window
    // two of its rows and four of its columns, and says nothing.
    let lines = text_rows(&text, area.width as usize).min(area.height as usize) as u16;
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: true })
        .alignment(ratatui::layout::Alignment::Center);
    let at = Rect::new(
        area.x,
        area.y + (area.height - lines) / 2,
        area.width,
        lines,
    );
    frame.render_widget(paragraph, at);
}

/// How many rows `lines` take wrapped at `width`, as `Wrap` wraps them.
fn text_rows(lines: &[Line], width: usize) -> usize {
    lines
        .iter()
        .map(|line| {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            crate::tui::command::wrap_words(&text, width).len().max(1)
        })
        .sum()
}

/// The one scrollbar, for every list and pane that outgrows its box: on the
/// right border, no end arrows, and drawn in the theme's alphabet — a console
/// with no block elements gets `|` and `#` rather than replacement boxes.
pub fn scrollbar(g: &crate::tui::theme::Glyphs) -> ratatui::widgets::Scrollbar<'static> {
    let bar =
        ratatui::widgets::Scrollbar::new(ratatui::widgets::ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
    if g.is_ascii() {
        bar.track_symbol(Some("|")).thumb_symbol("#")
    } else {
        bar
    }
}

/// Cut `text` to `width` display columns, ending in `ellipsis` when it had to.
pub fn fit(text: &str, width: usize, ellipsis: &str) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let ellipsis_width = ellipsis.width();
    if width <= ellipsis_width {
        return ellipsis.chars().take(width).collect();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width - ellipsis_width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push_str(ellipsis);
    out
}

/// One [`crate::tui::guide::Note`] as the lines it draws, wrapped to `width`.
///
/// The guide overlay and the editor's explanation panel both render through
/// this, so there is one styling vocabulary for explanatory text rather than
/// two that drift. Prose wraps; a literal block never does — a wrapped folder
/// tree is not a folder tree, so it is cut at the edge instead.
///
/// **A list item wears no glyph.** The five the theme has each mean one thing
/// already, and a sixth invented for decoration is the thing this app's look
/// rules out. An item starts two columns in and its continuations four, so
/// where one ends and the next begins is visible from the alignment alone.
pub fn note_lines<'a>(
    notes: &[crate::tui::guide::Note],
    theme: &crate::tui::theme::Theme,
    width: usize,
) -> Vec<Line<'a>> {
    use crate::tui::command::wrap_words;
    use crate::tui::guide::Note;

    let width = width.max(1);
    let mut out: Vec<Line> = Vec::new();
    let prose = |out: &mut Vec<Line>, text: &str, style: Style, first: usize, rest: usize| {
        let room = width.saturating_sub(rest).max(1);
        for (at, part) in wrap_words(text, room).into_iter().enumerate() {
            let indent = if at == 0 { first } else { rest };
            out.push(Line::from(Span::styled(
                format!("{}{part}", " ".repeat(indent)),
                style,
            )));
        }
    };
    for note in notes {
        match note {
            Note::Head(text) => prose(&mut out, text, theme.accent(), 0, 0),
            Note::Text(text) => prose(&mut out, text, theme.text(), 0, 0),
            Note::Aside(text) => prose(&mut out, text, theme.dim(), 0, 0),
            Note::Warn(text) => prose(&mut out, text, theme.warn(), 0, 0),
            Note::Bullet(text) => prose(&mut out, text, theme.text(), 2, 4),
            Note::Code(text) => out.push(Line::from(Span::styled(
                fit(text, width, theme.glyphs.ellipsis),
                theme.dim(),
            ))),
            Note::Blank => out.push(Line::from("")),
        }
    }
    out
}

/// `1 base`, `2 bases`: a count with its noun.
pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Pad `text` to `width` display columns.
pub fn pad(text: &str, width: usize) -> String {
    let w = text.width();
    if w >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - w))
    }
}

/// `text` as spans with the characters at `hits` (char offsets) in `hit`.
pub fn highlighted<'a>(text: &'a str, hits: &[usize], base: Style, hit: Style) -> Vec<Span<'a>> {
    if hits.is_empty() {
        return vec![Span::styled(text, base)];
    }
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_hit = false;
    for (i, c) in text.chars().enumerate() {
        let is_hit = hits.contains(&i);
        if is_hit != run_hit && !run.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut run),
                if run_hit { hit } else { base },
            ));
        }
        run_hit = is_hit;
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_hit { hit } else { base }));
    }
    spans
}

/// `spans` cut to `width` display columns, each keeping its own style, the
/// cut marked with `ellipsis` in the style of the span it fell in. A line of
/// several styles collapsed into the first one's — a tab's underline, a
/// base's colour — says less than the same line cut short.
pub fn fit_spans<'a>(spans: Vec<Span<'a>>, width: usize, ellipsis: &str) -> Vec<Span<'a>> {
    let total: usize = spans.iter().map(|s| s.width()).sum();
    if total <= width {
        return spans;
    }
    let ellipsis_width = ellipsis.width();
    if width <= ellipsis_width {
        return vec![Span::raw(ellipsis.chars().take(width).collect::<String>())];
    }
    let room = width - ellipsis_width;
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let w = span.width();
        if used + w <= room {
            used += w;
            out.push(span);
            continue;
        }
        let mut cut = String::new();
        for c in span.content.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if used + cw > room {
                break;
            }
            used += cw;
            cut.push(c);
        }
        let style = span.style;
        if !cut.is_empty() {
            out.push(Span::styled(cut, style));
        }
        out.push(Span::styled(ellipsis.to_string(), style));
        break;
    }
    out
}

/// A line with `left` at the start and `right` at the end, `right` winning
/// the space when both cannot fit — and `left` cut with its styles kept.
pub fn split_line<'a>(
    left: Vec<Span<'a>>,
    right: Vec<Span<'a>>,
    width: usize,
    ellipsis: &str,
) -> Line<'a> {
    let left_width: usize = left.iter().map(|s| s.width()).sum();
    let right_width: usize = right.iter().map(|s| s.width()).sum();
    let mut spans = Vec::new();
    if left_width + right_width < width {
        spans.extend(left);
        spans.push(Span::raw(" ".repeat(width - left_width - right_width)));
        spans.extend(right);
    } else if right_width < width {
        let room = width - right_width - 1;
        spans.extend(fit_spans(left, room, ellipsis));
        spans.push(Span::raw(" "));
        spans.extend(right);
    } else {
        spans.extend(fit_spans(left, width, ellipsis));
    }
    Line::from(spans)
}

/// The first of `candidates` — `(left, right)` pairs, most complete first —
/// whose halves fit `width` side by side; the last one's left half, cut with
/// its styles kept, when none does. What a narrow window gives up first is
/// whatever the caller left out of the earlier candidates.
pub fn first_that_fits<'a>(
    candidates: Vec<(Vec<Span<'a>>, Vec<Span<'a>>)>,
    width: usize,
    ellipsis: &str,
) -> Line<'a> {
    let last = candidates.len().saturating_sub(1);
    for (at, (left, right)) in candidates.into_iter().enumerate() {
        let wide: usize = left.iter().chain(&right).map(|s| s.width()).sum();
        if wide < width || at == last {
            return if wide < width {
                split_line(left, right, width, ellipsis)
            } else {
                Line::from(fit_spans(left, width, ellipsis))
            };
        }
    }
    Line::default()
}

#[cfg(test)]
mod tests {
    use super::{fit, highlighted, pad};
    use ratatui::style::{Modifier, Style};

    #[test]
    fn fit_counts_display_columns() {
        assert_eq!(fit("hello", 10, "…"), "hello");
        assert_eq!(fit("hello world", 6, "…"), "hello…");
        assert_eq!(fit("日本語だ", 5, "…"), "日本…");
        assert_eq!(pad("ab", 4), "ab  ");
    }

    #[test]
    fn hits_split_the_text_into_runs() {
        let hit = Style::default().add_modifier(Modifier::BOLD);
        let spans = highlighted("lullaby", &[0, 1, 5], Style::default(), hit);
        let text: Vec<&str> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, vec!["lu", "lla", "b", "y"]);
        assert_eq!(spans[0].style, hit);
        assert_eq!(spans[1].style, Style::default());
    }
}
