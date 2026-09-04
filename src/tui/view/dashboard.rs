//! The bands around the table: the header, the search bar, the status line
//! and the hint bar.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::{App, Screen, StatusLevel};
use crate::tui::command;
use crate::tui::view::{fit, plural, split_line};

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub fn header(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = area.width as usize;
    let gap = "   ";

    // Line 1: the product's name, then the tabs. **Not the project count** —
    // the search bar states it, live, beside the sort and the marks, which is
    // where a reader looking for "how many am I seeing" already is. Three sites
    // saying the same pair of numbers in three formats read as three different
    // facts.
    let mut left = vec![
        Span::styled(
            " fast-folder",
            theme.accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw(gap),
    ];
    for (i, screen) in Screen::ALL.iter().enumerate() {
        if i > 0 {
            left.push(Span::styled(" │ ", theme.dim()));
        }
        let here = *screen == app.screen;
        left.push(Span::styled(
            screen.label(),
            if here {
                theme
                    .accent()
                    .add_modifier(ratatui::style::Modifier::UNDERLINED)
            } else {
                theme.dim()
            },
        ));
    }
    if let Some(summary) = &app.summary {
        left.push(Span::styled(
            format!("{gap}{}", plural(summary.bases.len(), "base", "bases")),
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
        None => match &app.summary_error {
            Some(error) => {
                bases.push(Span::styled(
                    format!("{} the bases could not be read: {error}", g.warn),
                    theme.warn(),
                ));
                bases.push(Span::styled("   F5 retries", theme.dim()));
            }
            None => bases.push(Span::styled("probing bases…", theme.dim())),
        },
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

    // **The one place the count is stated.** Live, next to the sort it is
    // ordered by, the filters that produced it and the marks a verb would act
    // on — everything a reader asking "how many am I seeing" wants at once.
    let mut right = vec![Span::styled(
        format!("{}/{}", app.library.len(), app.library.snapshot.len()),
        theme.text(),
    )];
    // The first frame's counts come from the index; the spinner rides with the
    // number it qualifies rather than sitting in a header that no longer has
    // one.
    if !app.library.loaded {
        right.push(Span::styled(
            format!(" (from index) {}", SPINNER[(app.ticks % 4) as usize]),
            theme.dim(),
        ));
    }
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
    if let Some(base) = &app.library.base_filter {
        right.push(Span::styled(
            format!(" {} base={}", g.sep, crate::core::library::base_label(base)),
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
    // The counts win the row over the query text, but never past the row's
    // end: a long template filter on a narrow terminal is cut, not drawn
    // outside the frame.
    let right_width: usize = right.iter().map(|s| s.width()).sum::<usize>().min(width);

    let mut prefix = format!(" {} ", g.search);
    if let Some(preset) = &app.library.preset {
        // `fastf recent --tag draft`: the chip is a filter, and Esc takes it
        // off like any other.
        prefix.push_str(&format!("[{} {} Esc clears] ", preset.label(), g.sep));
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
        area.x + width.saturating_sub(right_width) as u16,
        area.y,
        right_width as u16,
        1,
    );
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(ratatui::layout::Alignment::Right),
        right_area,
    );
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
    } else if app.unseen_warnings > 0 {
        Line::from(Span::styled(
            format!(
                " {} {} warning{} arrived while a dialog was open   {}   L messages",
                g.warn,
                app.unseen_warnings,
                if app.unseen_warnings == 1 { "" } else { "s" },
                g.sep
            ),
            theme.warn(),
        ))
    } else if !app.library.loaded {
        Line::from(Span::styled(" reading the library…", theme.dim()))
    } else if let Some(error) = &app.library.error {
        Line::from(Span::styled(format!(" {error}"), theme.bad()))
    } else {
        let idle = if app.library.is_empty() && app.library.snapshot.is_empty() {
            "no projects yet — press n to create one, or e to register a folder".to_string()
        } else if app.library.is_empty() {
            // Name the thing that is hiding the rows, not every thing that
            // could.
            match (
                app.library.template_filter.is_some(),
                app.library.preset.is_some(),
                app.search.input.is_empty(),
            ) {
                (true, _, _) => {
                    "no matches — loosen the query, or press F to clear the template filter"
                        .to_string()
                }
                (false, true, _) => {
                    "no matches — Esc clears the filter this app was opened with".to_string()
                }
                (false, false, false) => "no matches — loosen the query".to_string(),
                (false, false, true) => "nothing to show".to_string(),
            }
        } else {
            // The counts are in the search bar and the keys are in the hint
            // bar, both of them from the one place each fact lives. What is
            // left for this line is the state neither of those can show: what
            // a batch verb would act on right now.
            match app.library.marks.len() {
                0 => String::new(),
                n => format!(
                    "{n} marked {} a verb acts on {} instead of the row under the cursor",
                    g.sep,
                    if n == 1 { "it" } else { "them" }
                ),
            }
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
        Some(Modal::Note(_)) => vec![
            ("Enter".to_string(), "save"),
            ("Alt-Enter".to_string(), "new line"),
            ("Esc".to_string(), "cancel"),
        ],
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
        Some(Modal::Flow(_)) | Some(Modal::Builder(_)) | Some(Modal::Settings(_)) => Vec::new(),
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
