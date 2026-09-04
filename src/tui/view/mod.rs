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
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::App;
use crate::tui::layout;

pub fn view(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if layout::too_small(area) {
        render_too_small(app, frame, area);
        return;
    }
    let regions = layout::regions(area, app.detail_open, app.table_min_width());

    dashboard::header(app, frame, regions.header);
    // The two tabs share every band but the middle one, so the chrome — the
    // name, the tabs, the bases, the status line, the keys — stays where it is
    // when you switch, and only the work changes.
    let search_caret = match app.screen {
        crate::tui::app::Screen::Library => {
            let caret = dashboard::search_bar(app, frame, regions.search);
            projects::table(app, frame, regions.table);
            if let Some(detail) = regions.detail {
                projects::detail(app, frame, detail);
            }
            caret
        }
        crate::tui::app::Screen::Templates => {
            let caret = templates::bar(app, frame, regions.search);
            let body = Rect::new(
                regions.table.x,
                regions.table.y,
                area.width,
                regions.table.height,
            );
            templates::screen(app, frame, body);
            caret
        }
    };
    dashboard::status(app, frame, regions.status);
    dashboard::hints(app, frame, regions.hints);

    let modal_caret = modals::render(app, frame, area);
    modals::render_move_progress(app, frame, area);
    modals::render_job(app, frame, area);

    if let Some(caret) = modal_caret {
        frame.set_cursor_position(caret);
    } else if app.modals.is_empty()
        && app.search.editing
        && let Some(caret) = search_caret
    {
        frame.set_cursor_position(caret);
    }
}

fn render_too_small(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let text = vec![
        Line::from(Span::styled(
            format!(
                "fastf needs at least {}×{} — this window is {}×{}",
                layout::MIN_WIDTH,
                layout::MIN_HEIGHT,
                area.width,
                area.height
            ),
            theme.warn(),
        )),
        Line::from(Span::styled(
            "make it bigger, or press q to quit",
            theme.dim(),
        )),
    ];
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
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

/// A line with `left` at the start and `right` at the end, `right` winning
/// the space when both cannot fit.
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
        let left_text: String = left.iter().map(|s| s.content.as_ref()).collect();
        let style = left.first().map(|s| s.style).unwrap_or_default();
        spans.push(Span::styled(fit(&left_text, room, ellipsis), style));

        spans.push(Span::raw(" "));
        spans.extend(right);
    } else {
        spans.extend(left);
    }
    Line::from(spans)
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
