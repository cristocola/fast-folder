//! The bands around the table: the header, the search bar, the status line
//! and the hint bar.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::tui::app::{App, StatusLevel};
use crate::tui::command;
use crate::tui::view::{fit, split_line};

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub fn header(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = area.width as usize;

    // Line 1: the counts.
    let mut left = vec![Span::styled(" fastf ", theme.accent())];
    let projects = if app.library.loaded {
        app.library.snapshot.len()
    } else {
        app.summary.as_ref().map(|s| s.projects).unwrap_or(0)
    };
    left.push(Span::styled(
        format!(" {} {projects} projects", g.projects),
        theme.bold(),
    ));
    if !app.library.loaded {
        left.push(Span::styled(
            format!(" (from index) {}", SPINNER[(app.ticks % 4) as usize]),
            theme.dim(),
        ));
    }
    left.push(Span::styled(
        format!(
            " {} {} {} templates",
            g.sep,
            g.templates,
            app.templates.cards.len()
        ),
        theme.text(),
    ));
    if let Some(summary) = &app.summary {
        left.push(Span::styled(
            format!(" {} {} {} bases", g.sep, g.bases, summary.bases.len()),
            theme.text(),
        ));
        if let Some(id) = &summary.max_id {
            left.push(Span::styled(
                format!(" {} {} {id}", g.sep, g.highest),
                theme.dim(),
            ));
        }
        if summary.attention > 0 {
            left.push(Span::styled(
                format!(
                    " {} {} {} needs attention",
                    g.sep, g.warn, summary.attention
                ),
                theme.warn(),
            ));
        }
    }
    let right = if app.session.is_empty() {
        Vec::new()
    } else {
        vec![
            Span::styled("this session: ", theme.dim()),
            Span::styled(app.session.join(&format!("  {}  ", g.sep)), theme.dim()),
            Span::raw(" "),
        ]
    };
    let mut lines = vec![split_line(left, right, width)];

    // Line 2: bases on a tall header, the pulse on a compact one.
    let bases_line = |app: &App| -> Line<'static> {
        let mut spans = vec![Span::styled(" bases  ", theme.dim())];
        match &app.summary {
            Some(summary) => {
                for (i, base) in summary.bases.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::styled(format!("  {}  ", g.sep), theme.dim()));
                    }
                    if base.is_default {
                        spans.push(Span::styled(format!("{} ", g.arrow), theme.dim()));
                    }
                    spans.push(Span::styled(base.label.clone(), theme.accent()));
                    spans.push(Span::styled(format!(" ({})", base.note()), theme.dim()));
                }
            }
            None => spans.push(Span::styled("probing…", theme.dim())),
        }
        Line::from(spans)
    };
    let pulse_line = |app: &App, room: usize| -> Line<'static> {
        let mut spans = vec![Span::styled(
            format!(" {}{} pulse {}{} ", g.rule, g.rule, g.rule, g.rule),
            theme.dim(),
        )];
        let pulse = app.templates.pulse(4);
        let max = pulse.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
        let mut used = spans[0].width();
        for (slug, count) in pulse {
            let bar = g.bar.repeat(((count * 8) / max).max(1));
            let cell = format!(" {slug} {bar} {count}  ");
            if used + cell.width() > room {
                break;
            }
            used += cell.width();
            spans.push(Span::styled(format!(" {slug} "), theme.dim()));
            spans.push(Span::styled(
                bar,
                ratatui::style::Style::default().fg(theme.tag_color(slug)),
            ));
            spans.push(Span::styled(format!(" {count}  "), theme.text()));
        }
        Line::from(spans)
    };

    if area.height >= 4 {
        lines.push(bases_line(app));
        lines.push(pulse_line(app, width));
        let newest = app
            .summary
            .as_ref()
            .and_then(|s| s.newest.as_ref())
            .map(|(id, name)| format!("{id} {name}"))
            .unwrap_or_else(|| "—".to_string());
        lines.push(Line::from(vec![
            Span::styled(" newest ", theme.dim()),
            Span::styled(
                fit(&newest, width.saturating_sub(9), g.ellipsis),
                theme.text(),
            ),
        ]));
    } else {
        lines.push(pulse_line(app, width));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// The search bar. Returns where the caret is when the bar is being edited.
pub fn search_bar(app: &App, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = area.width as usize;

    let mut right = vec![Span::styled(
        format!("{}/{}", app.library.len(), app.library.snapshot.len()),
        theme.text(),
    )];
    right.push(Span::styled(
        format!(
            " {} {}",
            g.sep,
            app.library.effective_sort(&app.search.query).label()
        ),
        theme.dim(),
    ));
    if let Some(slug) = &app.library.template_filter {
        right.push(Span::styled(
            format!(" {} template={slug}", g.sep),
            theme.accent_alt(),
        ));
    }
    if !app.library.marks.is_empty() {
        right.push(Span::styled(
            format!(" {} {} {}", g.sep, app.library.marks.len(), g.mark),
            theme.warn(),
        ));
    }
    right.push(Span::raw(" "));
    let right_width: usize = right.iter().map(|s| s.width()).sum();

    let mut prefix = format!(" {} ", g.search);
    if let Some(preset) = &app.library.preset {
        prefix.push_str(&format!("[{}] ", preset.label()));
    }
    let prefix_span = Span::styled(
        prefix.clone(),
        if app.search.editing {
            theme.accent()
        } else {
            theme.dim()
        },
    );

    let text_room = width.saturating_sub(right_width + 1);
    let text_area = Rect::new(area.x, area.y, text_room as u16, 1);

    let caret = if app.search.editing || !app.search.input.is_empty() {
        app.search
            .input
            .render_line(text_area, frame.buffer_mut(), prefix_span, theme.text())
    } else {
        let placeholder =
            "/ to search  ·  words match fuzzily, tag:x  template=y  created>date match exactly";
        let line = Line::from(vec![
            prefix_span,
            Span::styled(
                fit(
                    placeholder,
                    text_room.saturating_sub(prefix.len()),
                    g.ellipsis,
                ),
                theme.dim(),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), text_area);
        None
    };

    let right_area = Rect::new(
        area.x + (width - right_width) as u16,
        area.y,
        right_width as u16,
        1,
    );
    frame.render_widget(Paragraph::new(Line::from(right)), right_area);
    caret.filter(|_| app.search.editing)
}

pub fn status(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let line = if let Some(what) = app.busy {
        Line::from(vec![
            Span::styled(
                format!(" {} ", SPINNER[(app.ticks % 4) as usize]),
                theme.accent(),
            ),
            Span::styled(what, theme.text()),
        ])
    } else if !app.status.text.is_empty() {
        let style = match app.status.level {
            StatusLevel::Info => theme.text(),
            StatusLevel::Good => theme.good(),
            StatusLevel::Warn => theme.warn(),
            StatusLevel::Error => theme.bad(),
        };
        Line::from(Span::styled(
            format!(
                " {}",
                fit(
                    &app.status.text,
                    area.width.saturating_sub(1) as usize,
                    g.ellipsis
                )
            ),
            style,
        ))
    } else if !app.library.loaded {
        Line::from(Span::styled(" reading the library…", theme.dim()))
    } else if let Some(error) = &app.library.error {
        Line::from(Span::styled(format!(" {error}"), theme.bad()))
    } else {
        let idle = if app.library.is_empty() && app.library.snapshot.is_empty() {
            "no projects yet — press n to create one, or e to register a folder".to_string()
        } else if app.library.is_empty() {
            "no matches — loosen the query, or press F to clear the template filter".to_string()
        } else {
            format!(
                "{} of {} projects  {}  ? for help",
                app.library.len(),
                app.library.snapshot.len(),
                g.sep
            )
        };
        Line::from(Span::styled(format!(" {idle}"), theme.dim()))
    };
    frame.render_widget(Paragraph::new(line), area);
}

pub fn hints(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let mut spans = vec![Span::raw(" ")];
    let ctx = app.context();
    let pairs = match ctx {
        crate::tui::command::Context::SearchEdit => vec![
            ("Enter".to_string(), "keep"),
            ("Esc".to_string(), "clear / leave"),
            ("↑↓".to_string(), "move"),
        ],
        crate::tui::command::Context::Palette => vec![
            ("↑↓".to_string(), "move"),
            ("Enter".to_string(), "run"),
            ("Esc".to_string(), "close"),
            ("#".to_string(), "projects only"),
        ],
        crate::tui::command::Context::Modal => {
            vec![("Esc".to_string(), "close"), ("↑↓".to_string(), "scroll")]
        }
        other => command::hints(other, app, area.width.saturating_sub(2) as usize),
    };
    for (key, title) in pairs {
        spans.push(Span::styled(key, theme.key()));
        spans.push(Span::styled(format!(" {title}  "), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
