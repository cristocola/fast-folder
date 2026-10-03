//! The bands around the table: the header, the search bar, the status line
//! and the hint bar.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::app::data::BaseInfo;
use crate::tui::app::{App, Screen, StatusLevel};
use crate::tui::command;
use crate::tui::view::{first_that_fits, fit, fit_spans, plural, split_line};

pub fn header(app: &App, frame: &mut Frame, area: Rect) {
    let width = area.width as usize;
    let gap = "   ";

    let known = app
        .summary
        .as_ref()
        .and_then(crate::tui::app::data::Summary::bases_known);
    let mut lines = vec![tabs_line(app, known, width, gap)];
    lines.push(bases_line(app, known, width, gap));

    frame.render_widget(Paragraph::new(lines), area);
}

/// Line 1: the product's name, then the tabs. **Not the project count** —
/// the search bar states it, live, beside the sort and the marks, which is
/// where a reader looking for "how many am I seeing" already is. Three sites
/// saying the same pair of numbers in three formats read as three different
/// facts.
fn tabs_line(
    app: &App,
    known: Option<&[BaseInfo]>,
    width: usize,
    gap: &'static str,
) -> Line<'static> {
    let theme = &app.theme;
    let g = theme.glyphs;
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
    let mut with_bases = left.clone();
    if let Some(bases) = known {
        with_bases.push(Span::styled(
            format!("{gap}{}", plural(bases.len(), "base", "bases")),
            theme.text(),
        ));
    }

    let highest = match app.summary.as_ref().and_then(|s| s.max_id.as_ref()) {
        Some(id) => vec![
            Span::styled("highest ", theme.dim()),
            Span::styled(id.clone(), theme.text()),
            Span::raw(" "),
        ],
        None => Vec::new(),
    };
    // A narrow window gives up the highest ID first, then the base count —
    // never the tabs, which say where you are and where `T` goes.
    first_that_fits(
        vec![
            (with_bases.clone(), highest),
            (with_bases, Vec::new()),
            (left, Vec::new()),
        ],
        width,
        g.ellipsis,
    )
}

/// Line 2: the bases, and on the right whatever needs attention — else
/// what this session did.
fn bases_line(
    app: &App,
    known: Option<&[BaseInfo]>,
    width: usize,
    gap: &'static str,
) -> Line<'static> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let mut bases = base_spans(app, known, gap);
    // A running job is the first thing on the row: what it is and how far,
    // with the spinner. It goes on whether or not its dialog is up.
    if let Some(job) = app.background.live().next() {
        let more = app.background.live().count().saturating_sub(1);
        let mut chip = vec![
            Span::styled(format!("{} ", g.spin(app.elapsed_ms)), theme.accent()),
            Span::styled(crate::tui::app::background::chip_title(job), theme.text()),
            Span::styled(
                format!(" {} {}", g.sep, crate::tui::app::background::step_of(job)),
                theme.dim(),
            ),
        ];
        if more > 0 {
            chip.push(Span::styled(format!(" {} {more} more", g.sep), theme.dim()));
        }
        chip.push(Span::raw("   "));
        chip.extend(bases);
        bases = chip;
    }
    attention_or_session(app, bases, width)
}

/// **The bases, and which of them are active.** Showing every base, an
/// inactive one is named dim; showing the active ones, the inactive are one
/// quiet count at the end of the row — the first thing a narrow window cuts.
fn base_spans(app: &App, known: Option<&[BaseInfo]>, gap: &'static str) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let mut bases = vec![Span::raw(" ")];
    match known {
        Some(known) => {
            let active_only = app.library.view == crate::tui::app::library::BasesView::Active;
            let mut left_out = 0;
            let mut first = true;
            for base in known {
                let active = app.base_is_active(base);
                if active_only && !active {
                    left_out += 1;
                    continue;
                }
                if !first {
                    bases.push(Span::raw(gap));
                }
                first = false;
                if base.is_default {
                    bases.push(Span::styled(format!("{} ", g.arrow), theme.dim()));
                }
                let label = if active { theme.accent() } else { theme.dim() };
                bases.push(Span::styled(base.label.clone(), label));
                let note = crate::tui::view::modals::base_note_text(app, base);
                bases.push(Span::styled(format!(" {note}"), theme.dim()));
            }
            if left_out > 0 {
                bases.push(Span::styled(
                    format!("{gap}{left_out} inactive"),
                    theme.dim(),
                ));
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
    bases
}

/// Something needing attention wins the row over the bases; what this
/// session did is the first thing a narrow window gives up.
fn attention_or_session(app: &App, bases: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let attention = app.summary.as_ref().map(|summary| {
        let attention = &summary.attention;
        (
            attention.needs_you(),
            attention.auto(),
            attention.waiting_work(),
        )
    });
    let key = crate::tui::command::key_of(crate::tui::command::CommandId::Attention);
    match attention {
        // Only what needs a person is a warning; what fastf is finishing by
        // itself is said quietly, so nobody is sent to act on it.
        Some((needs_you, _, _)) if needs_you > 0 => split_line(
            bases,
            vec![Span::styled(
                format!(
                    "{} {needs_you} need{} you  {key} ",
                    g.warn,
                    crate::util::plural::of(needs_you, "s", "")
                ),
                theme.warn(),
            )],
            width,
            g.ellipsis,
        ),
        Some((_, finishing, _)) if finishing > 0 => split_line(
            bases,
            vec![Span::styled(format!("finishing {finishing} "), theme.dim())],
            width,
            g.ellipsis,
        ),
        // Work that waits for a base to answer; a silent base itself is
        // named among the bases, not counted again here.
        Some((_, _, waiting)) if waiting > 0 => split_line(
            bases,
            vec![Span::styled(format!("{waiting} waiting "), theme.dim())],
            width,
            g.ellipsis,
        ),
        _ if !app.session.is_empty() => first_that_fits(
            vec![
                (
                    bases.clone(),
                    vec![
                        Span::styled("this session: ", theme.dim()),
                        Span::styled(app.session.join(&format!("  {}  ", g.sep)), theme.dim()),
                        Span::raw(" "),
                    ],
                ),
                (bases, Vec::new()),
            ],
            width,
            g.ellipsis,
        ),
        _ => Line::from(fit_spans(bases, width, g.ellipsis)),
    }
}

/// The search bar. Returns where the caret is when the bar is being edited.
pub fn search_bar(app: &App, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = area.width as usize;

    let right = list_report(app, width);
    let right_width: usize = right.iter().map(|s| s.width()).sum::<usize>().min(width);

    let mut prefix = format!(" {} ", g.search);
    if let Some(preset) = app.library.preset_filter() {
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

/// **The one place the count is stated.** Live, next to the sort it is
/// ordered by, the filters that produced it and the marks a verb would act
/// on — everything a reader asking "how many am I seeing" wants at once.
///
/// Each part carries its priority: a narrow window gives up the sort
/// first, then the "(from index)" words, then the filters, and never the
/// count or the marks — the one fact a person looks here for, and the one
/// that says a verb will act on more than the row under the cursor.
fn list_report(app: &App, width: usize) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let g = theme.glyphs;
    // Counted against the projects in view: a base the view leaves out is
    // not part of what this list could show.
    let mut parts: Vec<(u8, Span)> = vec![(
        0,
        Span::styled(
            format!("{}/{}", app.library.len(), app.library.in_view),
            theme.text(),
        ),
    )];
    // The first frame's counts come from the index; the spinner rides with the
    // number it qualifies.
    if !app.library.loaded {
        parts.push((
            3,
            Span::styled(
                format!(" (from index) {}", theme.glyphs.spin(app.elapsed_ms)),
                theme.dim(),
            ),
        ));
    }
    parts.push((
        4,
        Span::styled(
            format!(
                " {} {}",
                g.sep,
                app.library.effective_sort(&app.search.query).label()
            ),
            theme.dim(),
        ),
    ));
    if let Some(slug) = &app.library.template_filter {
        parts.push((
            2,
            Span::styled(format!(" {} template={slug}", g.sep), theme.accent_alt()),
        ));
    }
    if let Some(base) = &app.library.base_filter {
        parts.push((
            2,
            Span::styled(
                format!(" {} base={}", g.sep, crate::core::library::base_label(base)),
                theme.accent_alt(),
            ),
        ));
    }
    if app.library.view_narrows() {
        parts.push((
            2,
            Span::styled(format!(" {} active bases", g.sep), theme.accent_alt()),
        ));
    }
    if !app.library.marks.is_empty() {
        parts.push((
            1,
            Span::styled(
                format!(" {} {} {}", g.sep, app.library.marks.len(), g.mark),
                theme.warn(),
            ),
        ));
    }
    parts.push((0, Span::raw(" ")));
    // The query keeps room for the search glyph and a few letters.
    let most = width.saturating_sub(10);
    for dropped in [4, 3, 2] {
        let wide: usize = parts.iter().map(|(_, s)| s.width()).sum();
        if wide <= most {
            break;
        }
        parts.retain(|(priority, _)| *priority != dropped);
    }
    fit_spans(
        parts.into_iter().map(|(_, s)| s).collect(),
        width,
        g.ellipsis,
    )
}

pub fn status(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let line = if let Some(what) = app.busy {
        Line::from(vec![
            Span::styled(
                format!(" {} ", theme.glyphs.spin(app.elapsed_ms)),
                theme.accent(),
            ),
            Span::styled(what, theme.text()),
        ])
    } else if !app.status.text.is_empty() {
        // On its way out it dims, so it reads as expiring rather than as a
        // line that was there one frame and gone the next.
        let style = match crate::tui::motion::expiring_style(
            app.status.expires_at,
            app.elapsed_ms,
            theme,
            app.motion,
        ) {
            Some(fading) => fading,
            None => match app.status.level {
                StatusLevel::Info => theme.text(),
                StatusLevel::Good => theme.good(),
                StatusLevel::Warn => theme.warn(),
                StatusLevel::Error => theme.bad(),
            },
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
                crate::util::plural::s(app.unseen_warnings),
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
        } else if app.library.is_empty() && app.library.beyond_view > 0 {
            // The view is what hides them: say how many, and the key.
            format!(
                "{} in the active bases — {} in inactive ones, {} shows every base",
                if app.search.input.is_empty() {
                    "no projects"
                } else {
                    "no matches"
                },
                app.library.beyond_view,
                crate::tui::command::key_of(crate::tui::command::CommandId::ListBasesView)
            )
        } else if app.library.is_empty() {
            // Name the thing that is hiding the rows, not every thing that
            // could.
            match (
                app.library.template_filter.is_some(),
                app.library.preset_filter().is_some(),
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
                    crate::util::plural::of(n, "it", "them")
                ),
            }
        };
        Line::from(Span::styled(format!(" {idle}"), theme.dim()))
    };
    // Every sentence here fits the row, cut with the ellipsis rather than by
    // the edge of the window.
    let line = Line::from(fit_spans(line.spans, area.width as usize, g.ellipsis));
    // A message that just arrived wears the wash under the whole line, so
    // the eye is drawn to where what just happened is said; it lets go as a
    // row's does.
    let paragraph = Paragraph::new(line);
    let paragraph = match crate::tui::motion::arriving_style(
        app.status.shown_at,
        app.elapsed_ms,
        theme,
        app.motion,
    ) {
        Some(wash) if !app.status.text.is_empty() && app.busy.is_none() => paragraph.style(wash),
        _ => paragraph,
    };
    frame.render_widget(paragraph, area);
}

pub fn hints(app: &App, frame: &mut Frame, area: Rect) {
    use crate::tui::app::modal::Modal;

    let theme = &app.theme;
    let mut spans = vec![Span::raw(" ")];
    let width = area.width.saturating_sub(2) as usize;
    // **Every pair on this bar is read, not written**: a key spelled here is
    // a key that drifts.
    let pairs = match app.modals.top() {
        // A flow, the builder, the settings, the guide, a note, a confirmation
        // and the welcome dialog each draw their own key line inside their
        // frame, beside what the keys act on; repeating it down here would say
        // it twice.
        Some(Modal::Note(_))
        | Some(Modal::Confirm(_))
        | Some(Modal::Flow(_))
        | Some(Modal::Builder(_))
        | Some(Modal::Settings(_))
        | Some(Modal::Bases(_))
        | Some(Modal::Guide(_))
        | Some(Modal::Onboarding(_)) => Vec::new(),
        _ => {
            let ctx = app.context();
            let mut pairs: Vec<(String, &'static str)> =
                command::movement_pair(ctx, &app.theme.glyphs)
                    .into_iter()
                    .filter(|_| ctx.hints_movement())
                    .collect();
            // Only what the movement pair actually costs comes off the width
            // the rest is measured against: a flat allowance drops a verb from
            // every bar that shows no arrows at all.
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
