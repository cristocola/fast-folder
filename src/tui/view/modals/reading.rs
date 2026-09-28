//! The dialogs that are read: help, the guide, a message, the activity.

use super::*;

pub(super) fn render_help(
    app: &App,
    ctx: command::Context,
    scroll: usize,
    frame: &mut Frame,
    area: Rect,
) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = crate::tui::layout::help_box(area);
    super::clear(frame, area, &app.theme);
    let block = frame_block(app, format!(" help {} {} ", g.sep, ctx.label()), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The columns are measured from the commands, so a long title cannot
    // run into its description, and a description that does not fit its
    // line continues under itself rather than being cut.
    let width = inner.width as usize;
    let (keys_w, title_w, _) = command::help_columns(ctx, width, &g);
    let mut lines: Vec<Line> = Vec::new();
    for line in command::help_lines(ctx, width, &g) {
        lines.push(match line {
            command::HelpLine::Heading(label) => {
                Line::from(Span::styled(format!(" {label}"), theme.accent()))
            }
            command::HelpLine::Command {
                keys,
                title,
                description,
            } => Line::from(vec![
                Span::styled(format!("   {} ", pad(&keys, keys_w)), theme.key()),
                Span::styled(format!("{} ", pad(title, title_w)), theme.text()),
                Span::styled(description, theme.dim()),
            ]),
            command::HelpLine::Continuation { indent, text } => Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(text, theme.dim()),
            ]),

            command::HelpLine::Blank => Line::from(""),
        });
    }
    lines.push(Line::from(Span::styled(
        format!(
            " The command palette ({}) lists every command with its key; type to filter.",
            crate::tui::command::key_of(crate::tui::command::CommandId::Palette)
        ),
        theme.dim(),
    )));
    lines.push(Line::from(Span::styled(
        " Search: a word matches inside a name, id, template or tag (a typo is forgiven; a number means an id, and a/b is literal);",
        theme.dim(),
    )));
    lines.push(Line::from(Span::styled(
        " tag:x  template=y  created>date match exactly, and combine with the words.",
        theme.dim(),
    )));
    lines.push(Line::from(""));
    let mut footer = format!(" fastf {}", env!("CARGO_PKG_VERSION"));
    if let Some(dir) = &app.data_dir {
        footer.push_str(&format!("  {}  data in {dir}", g.sep));
    }
    footer.push_str(&format!(
        "  {}  docs: github.com/cristocola/fast-folder",
        g.sep
    ));
    lines.push(Line::from(Span::styled(footer, theme.dim())));
    let max_scroll = lines.len().saturating_sub(inner.height as usize);

    let paragraph = Paragraph::new(lines).scroll((scroll.min(max_scroll) as u16, 0));
    frame.render_widget(paragraph, inner);
}

// ---------------------------------------------------------------------------
// The guide
// ---------------------------------------------------------------------------

/// The columns a guide page's text is wrapped at: the box, less its border and
/// a column of padding on each side.
fn guide_text_width(box_: Rect) -> usize {
    box_.width.saturating_sub(4) as usize
}

/// How many rows a page takes once wrapped, at the width the view draws it.
///
/// `update` clamps the scroll with exactly this — the same function and the
/// same `Rect` — because a ceiling derived from anything else leaves the end
/// of a long body unreachable.
pub(crate) fn guide_rows(page: usize, box_: Rect) -> usize {
    crate::tui::guide::note_rows(&crate::tui::guide::page_notes(page), guide_text_width(box_))
}

/// The template guide: one page of prose at a time, in the same frame every
/// other dialog wears.
pub(super) fn render_guide(
    app: &App,
    state: &crate::tui::app::modal::GuideState,
    frame: &mut Frame,
    area: Rect,
) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = crate::tui::layout::guide_box(area);
    let total = crate::tui::guide::PAGES.len();
    let title = format!(
        " template guide {} {} ",
        g.sep,
        crate::tui::guide::page_title(state.page)
    );
    let Some((body, footer, keys)) =
        crate::tui::view::builder::frame_parts(app, title, frame, area)
    else {
        return;
    };

    // A column of padding inside the border, so prose does not sit against the
    // frame the way a table's measured cells are meant to.
    let text = Rect::new(
        body.x + 1,
        body.y,
        body.width.saturating_sub(2),
        body.height,
    );
    let lines = crate::tui::view::note_lines(
        &crate::tui::guide::page_notes(state.page),
        theme,
        guide_text_width(area),
    );
    let max_scroll = lines.len().saturating_sub(body.height as usize);
    frame.render_widget(
        Paragraph::new(lines).scroll((state.scroll.min(max_scroll) as u16, 0)),
        text,
    );

    crate::tui::view::builder::footer_line(
        frame,
        footer,
        &format!(" page {} of {total}", state.page + 1),
        theme.dim(),
    );

    // Every key on this line is a declared command — the guide has a
    // `Context` of its own for its pages — so every label is read rather
    // than written. The way out comes first, because `key_line` drops from
    // the end.
    let last = state.is_last();
    let pairs: Vec<(String, String)> = vec![
        (command::key_of(CommandId::Close), "close".to_string()),
        (
            command::key_of_in(CommandId::GuideNext, &app.theme.glyphs),
            if last { "close" } else { "next page" }.to_string(),
        ),
        (
            command::key_of_in(CommandId::GuidePrevious, &app.theme.glyphs),
            "back".to_string(),
        ),
    ]
    .into_iter()
    .chain(
        command::movement_pair(Context::Guide, &app.theme.glyphs)
            .map(|(keys, what)| (keys, what.to_string())),
    )
    .collect();
    frame.render_widget(
        Paragraph::new(crate::tui::view::builder::key_line(
            theme,
            &pairs,
            keys.width as usize,
        )),
        keys,
    );
}

pub(super) fn render_message(
    app: &App,
    title: &str,
    lines: &[String],
    level: MessageLevel,
    scroll: usize,
    frame: &mut Frame,
    area: Rect,
) {
    let theme = &app.theme;
    let area = crate::tui::layout::message_box(area);
    super::clear(frame, area, &app.theme);
    let style = match level {
        MessageLevel::Info => theme.accent(),
        MessageLevel::Warn => theme.warn(),
        MessageLevel::Error => theme.bad(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(format!(" {title} "), style))
        .border_style(style);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text: Vec<Line> = lines
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(theme.text))))
        .collect();
    let target = inset(inner);
    let max_scroll =
        message_rows(lines, target.width as usize).saturating_sub(inner.height as usize);
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll.min(max_scroll) as u16, 0));
    frame.render_widget(paragraph, target);
}

/// The activity screen (`L`): the message box, a row naming the pages with
/// the one on show in the accent and the key that turns them, a blank row, and
/// the page — the same wrapped paragraph a message is.
pub(super) fn render_activity(
    app: &App,
    activity: &crate::tui::app::modal::Activity,
    frame: &mut Frame,
    area: Rect,
) {
    use crate::tui::app::modal::ActivityPage;
    let theme = &app.theme;
    let box_ = crate::tui::layout::message_box(area);
    super::clear(frame, box_, &app.theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" activity ", theme.accent()))
        .border_style(theme.accent());
    let inner = block.inner(box_);
    frame.render_widget(block, box_);
    let target = inset(inner);

    // The page on show wears the cursor as well as the accent, so a theme
    // with no colour still says which it is.
    let mut tabs = Vec::new();
    for page in ActivityPage::ALL {
        if page == activity.page {
            tabs.push(Span::styled(
                format!("{} {}", theme.glyphs.cursor, page.title()),
                theme.accent(),
            ));
        } else {
            tabs.push(Span::styled(format!("  {}", page.title()), theme.dim()));
        }
        tabs.push(Span::raw("   "));
    }
    let key = command::key_of_in(CommandId::FocusNext, &theme.glyphs);
    tabs.push(Span::styled(
        format!("{key} turns the page · newest first"),
        theme.dim(),
    ));
    let header = Rect {
        height: 1.min(target.height),
        ..target
    };
    frame.render_widget(
        Paragraph::new(Line::from(fit_spans(
            tabs,
            target.width as usize,
            theme.glyphs.ellipsis,
        ))),
        header,
    );

    let body = Rect {
        y: target.y + 2.min(target.height),
        height: target.height.saturating_sub(2),
        ..target
    };
    let lines = activity.lines();
    let on_jobs = activity.page == ActivityPage::Jobs && !activity.job_ids.is_empty();
    let text: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if !on_jobs {
                return Line::from(Span::styled(line.clone(), Style::default().fg(theme.text)));
            }
            // The job Enter opens: the cursor and the selection style, which
            // mono draws reversed.
            // One line a job, cut to fit: a list, not a paragraph.
            let line = fit(
                line,
                (body.width as usize).saturating_sub(2),
                theme.glyphs.ellipsis,
            );
            if index == activity.job_cursor {
                Line::from(Span::styled(
                    format!("{} {line}", theme.glyphs.cursor),
                    theme.selection,
                ))
            } else {
                Line::from(Span::styled(
                    format!("  {line}"),
                    Style::default().fg(theme.text),
                ))
            }
        })
        .collect();
    let max_scroll = message_rows(lines, body.width as usize).saturating_sub(body.height as usize);
    let scroll = activity.scroll[activity.page as usize].min(max_scroll);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        body,
    );
}

/// How many rows of a page the activity screen shows: the message box's,
/// less its border and the row of page names with the blank one under it.
pub(crate) fn activity_body_rows(area: Rect) -> usize {
    crate::tui::layout::message_box(area)
        .height
        .saturating_sub(4) as usize
}

/// How many rows a message takes once it is wrapped, which is not how many
/// entries it has.
///
/// The journal and metadata views are drawn with `Wrap`, so a scroll limit
/// counted in `lines.len()` counts a wrapped note once where it draws twice,
/// and the end of a long journal is unreachable. This is the sum
/// `command::help_line_count` makes for the help.
pub(crate) fn message_rows(lines: &[String], width: usize) -> usize {
    lines.iter().map(|line| wrapped_rows(line, width)).sum()
}

/// The columns a message's text is wrapped at, from the same geometry
/// `render_message` draws with.
pub(crate) fn message_text_width(area: Rect) -> usize {
    // The block's border takes two columns, and `inset` one more.
    crate::tui::layout::message_box(area)
        .width
        .saturating_sub(3) as usize
}
