//! The bands around the table: the header, the search bar, the status line
//! and the hint bar.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::{App, Screen, StatusLevel};
use crate::tui::command;
use crate::tui::view::{fit, plural, split_line};

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
                bases.push(Span::styled(
                    format!(
                        "   {} retries",
                        crate::tui::command::key_of(crate::tui::command::CommandId::Reload)
                    ),
                    theme.dim(),
                ));
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
            format!(" (from index) {}", theme.glyphs.spin(app.ticks)),
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
                    text_room
                        .saturating_sub(unicode_width::UnicodeWidthStr::width(prefix.as_str())),
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
                format!(" {} ", theme.glyphs.spin(app.ticks)),
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
                " {} {} warning{} arrived while a dialog was open   {}   {} messages",
                g.warn,
                app.unseen_warnings,
                if app.unseen_warnings == 1 { "" } else { "s" },
                g.sep,
                crate::tui::command::key_of(crate::tui::command::CommandId::ShowLog)
            ),
            theme.warn(),
        ))
    } else if !app.library.loaded {
        Line::from(Span::styled(" reading the library…", theme.dim()))
    } else if let Some(error) = &app.library.error {
        Line::from(Span::styled(format!(" {error}"), theme.bad()))
    } else {
        let idle = if app.library.is_empty() && app.library.snapshot.is_empty() {
            format!(
                "no projects yet — press {} to create one, or {} to register a folder",
                crate::tui::command::key_of(crate::tui::command::CommandId::NewProject),
                crate::tui::command::key_of(crate::tui::command::CommandId::Register)
            )
        } else if app.library.is_empty() {
            // Name the thing that is hiding the rows, not every thing that
            // could.
            match (
                app.library.template_filter.is_some(),
                app.library.preset.is_some(),
                app.search.input.is_empty(),
            ) {
                (true, _, _) => format!(
                    "no matches — loosen the query, or press {} to clear the template filter",
                    crate::tui::command::key_of(crate::tui::command::CommandId::ClearFilters)
                ),
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
    let width = area.width.saturating_sub(2) as usize;
    // **Every pair on this bar is read, not written.** It used to hand-write
    // six of them — the palette's, the prompt's, the multi-pick's, the
    // picker's, the pager's and the search bar's — which is why four of those
    // dialogs had a `Context` with no commands in it: nothing needed them,
    // because the bar already knew. A key spelled here is a key that drifts.
    let pairs = match app.modals.top() {
        // A flow, the studio, the builder, the guide, a note, a confirmation
        // and the welcome dialog each draw their own key line inside their
        // frame, beside what the keys act on; repeating it down here would say
        // it twice.
        Some(Modal::Note(_))
        | Some(Modal::Confirm(_))
        | Some(Modal::Flow(_))
        | Some(Modal::Builder(_))
        | Some(Modal::Settings(_))
        | Some(Modal::Guide(_))
        | Some(Modal::Onboarding(_)) => Vec::new(),
        _ => {
            let ctx = app.context();
            let mut pairs: Vec<(String, &'static str)> = command::movement_pair(ctx)
                .into_iter()
                .filter(|_| ctx.hints_movement())
                .collect();
            // Only what the movement pair actually costs comes off the width
            // the rest is measured against — a flat allowance dropped a verb
            // from every bar that never showed the arrows at all.
            let spent: usize = pairs
                .iter()
                .map(|(key, what)| key.chars().count() + 1 + what.chars().count() + 2)
                .sum();
            pairs.extend(command::hints(ctx, app, width.saturating_sub(spent)));
            pairs
        }
    };
    for (key, title) in pairs {
        spans.push(Span::styled(key, theme.key()));
        spans.push(Span::styled(format!(" {title}  "), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
