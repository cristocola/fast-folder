//! The bands around the table: the header, the search bar, the status line
//! and the hint bar.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::{App, StatusLevel};
use crate::tui::command;
use crate::tui::view::{fit, split_line};

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub fn header(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = area.width as usize;
    let gap = "   ";

    // Line 1: the name, the counts, the highest id.
    let projects = if app.library.loaded {
        app.library.snapshot.len()
    } else {
        app.summary.as_ref().map(|s| s.projects).unwrap_or(0)
    };
    let mut left = vec![
        Span::styled(
            " fastf",
            theme.accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw(gap),
    ];
    left.push(Span::styled(format!("{projects} projects"), theme.text()));
    if !app.library.loaded {
        left.push(Span::styled(
            format!(" (from index) {}", SPINNER[(app.ticks % 4) as usize]),
            theme.dim(),
        ));
    }
    left.push(Span::styled(
        format!("{gap}{} templates", app.templates.cards.len()),
        theme.text(),
    ));
    if let Some(summary) = &app.summary {
        left.push(Span::styled(
            format!("{gap}{} bases", summary.bases.len()),
            theme.text(),
        ));
    }
    let right = match app.summary.as_ref().and_then(|s| s.max_id.as_ref()) {
        Some(id) => vec![
            Span::styled("highest ", theme.dim()),
            Span::styled(id.clone(), theme.text()),
            Span::raw(" "),
        ],
        None => Vec::new(),
    };
    let mut lines = vec![split_line(left, right, width, g.ellipsis)];

    // Line 2: the bases, and on the right whatever needs attention — else
    // what this session did.
    let mut bases = vec![Span::raw(" ")];
    match &app.summary {
        Some(summary) => {
            for (i, base) in summary.bases.iter().enumerate() {
                if i > 0 {
                    bases.push(Span::raw(gap));
                }
                if base.is_default {
                    bases.push(Span::styled(format!("{} ", g.arrow), theme.dim()));
                }
                bases.push(Span::styled(base.label.clone(), theme.accent()));
                bases.push(Span::styled(format!(" {}", base.note()), theme.dim()));
            }
        }
        None => bases.push(Span::styled("probing bases…", theme.dim())),
    }
    let right = match app.summary.as_ref().map(|s| s.attention) {
        Some(n) if n > 0 => vec![Span::styled(
            format!(
                "{} {n} need{} attention ",
                g.warn,
                if n == 1 { "s" } else { "" }
            ),
            theme.warn(),
        )],
        _ if !app.session.is_empty() => vec![
            Span::styled("this session: ", theme.dim()),
            Span::styled(app.session.join(&format!("  {}  ", g.sep)), theme.dim()),
            Span::raw(" "),
        ],
        _ => Vec::new(),
    };
    lines.push(split_line(bases, right, width, g.ellipsis));

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
        let placeholder = "/ to search";
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
                "{} of {} projects   {}   ? for help",
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
    use crate::tui::app::modal::Modal;

    let theme = &app.theme;
    let mut spans = vec![Span::raw(" ")];
    let pairs = match app.modals.top() {
        Some(Modal::Palette(_)) => vec![
            ("↑↓".to_string(), "move"),
            ("Enter".to_string(), "run"),
            ("Esc".to_string(), "close"),
            ("#".to_string(), "projects only"),
        ],
        // The menu's own keys, from the registry: the verbs' letters work
        // here exactly as they do on the list.
        Some(Modal::Actions(_)) => command::hints(
            crate::tui::command::Context::Actions,
            app,
            area.width.saturating_sub(2) as usize,
        ),

        Some(Modal::TextPrompt(_)) => {
            vec![
                ("Enter".to_string(), "confirm"),
                ("Esc".to_string(), "cancel"),
            ]
        }
        Some(Modal::Confirm(_)) => vec![
            ("y".to_string(), "yes"),
            ("n".to_string(), "no"),
            ("Esc".to_string(), "cancel"),
        ],
        Some(Modal::MultiPick(_)) => vec![
            ("Space".to_string(), "toggle"),
            ("Enter".to_string(), "confirm"),
            ("Esc".to_string(), "cancel"),
        ],
        Some(Modal::Pick(_)) => vec![
            ("↑↓".to_string(), "move"),
            ("Enter".to_string(), "run"),
            ("Esc".to_string(), "close"),
        ],
        Some(Modal::Help { .. }) | Some(Modal::Message { .. }) => {
            vec![("Esc".to_string(), "close"), ("↑↓".to_string(), "scroll")]
        }
        // A flow, the studio and the builder draw their own key line inside
        // their frame, beside what the keys act on; repeating it down here
        // would say it twice.
        Some(Modal::Flow(_))
        | Some(Modal::Studio(_))
        | Some(Modal::Builder(_))
        | Some(Modal::Settings(_)) => Vec::new(),
        Some(Modal::Onboarding(_)) => vec![
            ("Enter".to_string(), "create it"),
            ("Esc".to_string(), "skip for now"),
        ],
        None => match app.context() {
            crate::tui::command::Context::SearchEdit => vec![
                ("Enter".to_string(), "keep"),
                ("Esc".to_string(), "clear / leave"),
                ("↑↓".to_string(), "move"),
            ],
            other => command::hints(other, app, area.width.saturating_sub(2) as usize),
        },
    };
    for (key, title) in pairs {
        spans.push(Span::styled(key, theme.key()));
        spans.push(Span::styled(format!(" {title}  "), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
