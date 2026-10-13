//! The desk tab's list: one line per project under the group its next open
//! todo puts it in, drawn in the library table's place with the library's
//! pane beside it.
//!
//! A heading is a row (`yours · 3`, with a rule to the border) and a card
//! is the cursor cell, the id, the name whole, the next todo (its phase
//! before it), how long the project has been quiet, and `active` when a
//! program is working in it. The id and the name are measured over the
//! library in view as the table measures them, so a card never moves when a
//! group is shown alone; the next todo takes what is left and is cut from
//! the right. In a window narrower than the list asked for
//! (`App::desk_needs`) the name gives way instead, so the next todo keeps
//! its minimum (`name_and_todo`). Projects not read yet sit dim under
//! `reading`, with no claim made about them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, ScrollbarState};
use unicode_width::UnicodeWidthStr;

use crate::core::desk::Assignee;
use crate::tui::app::desk::{ACTIVE_CELL, AGE_CELL, DeskRow, NEXT_TODO_MIN, row_fixed};
use crate::tui::app::{App, Focus};
use crate::tui::view::{fit, fit_spans};

/// How many days a todo assigned to an owner may wait with nothing written
/// in the folder and no program working in it before the row shows the
/// warning glyph.
const STUCK_AFTER_DAYS: u64 = 3;

pub fn table(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let focused = app.focus == Focus::Projects && app.modals.is_empty() && !app.search.editing;

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " desk ",
            crate::tui::view::projects::title_style(app, focused),
        ))
        .border_style(crate::tui::view::projects::border_style(app, focused));
    let full_inner = block.inner(area);
    frame.render_widget(block, area);
    if full_inner.height < 1 || full_inner.width < 4 {
        return;
    }
    // One column of gutter, as the table keeps: a right-aligned age against
    // the border reads as cut off, and the scrollbar lands on it.
    let inner = Rect::new(
        full_inner.x,
        full_inner.y,
        full_inner.width - 1,
        full_inner.height,
    );
    let width = inner.width as usize;

    let desk = &app.desk;
    if desk.rows.is_empty() {
        let sentence = if !app.library.loaded {
            format!("reading the library {}", g.spin(app.elapsed_ms))
        } else if app.library.is_empty() && app.library.in_view == 0 {
            format!(
                "No projects yet: {} creates one.",
                crate::tui::command::key_of(crate::tui::command::CommandId::NewProject)
            )
        } else if desk.filter.is_some() {
            format!(
                "Nothing in this group: {} shows every group again.",
                crate::tui::command::key_of(crate::tui::command::CommandId::ClearFilters)
            )
        } else {
            "No project matches the search.".to_string()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(fit(&sentence, width, g.ellipsis), theme.dim())),
            Rect::new(inner.x + 1, inner.y, width.saturating_sub(1) as u16, 1),
        );
        return;
    }

    let (id_w, name_w) = app.library.widths;
    // The name and the next todo share what the rest of the row leaves.
    let (name_w, todo_w) = name_and_todo(width.saturating_sub(row_fixed(id_w)), name_w);
    let hat = app.hat();
    let now = (app.now)();

    let rows_visible = inner.height as usize;
    let end = (desk.offset + rows_visible).min(desk.rows.len());
    for (line_at, index) in (desk.offset..end).enumerate() {
        let y = inner.y + line_at as u16;
        let bar = Rect::new(inner.x, y, inner.width, 1);
        let row = &desk.rows[index];
        let line = match row {
            DeskRow::Heading { group, count } => {
                heading_line(app, group.as_ref(), *count, hat.as_deref(), width)
            }
            DeskRow::Card { card, read } => {
                card_line(app, card, *read, id_w, name_w, todo_w, Some(now.as_str()))
            }
        };
        // A row a verb has just changed wears the wash, under the cursor,
        // as the table's rows do.
        let wash = row.path().and_then(|path| {
            app.pulses
                .style_for(path, app.elapsed_ms, theme, app.motion)
        });
        let lit = focused && desk.selected == Some(index);
        let mut line = line;
        if let Some(wash) = wash {
            line = line.style(wash);
            frame.buffer_mut().set_style(bar, wash);
        }
        if lit {
            line = line.patch_style(theme.selection);
            frame.buffer_mut().set_style(bar, theme.selection);
        }
        frame.render_widget(Paragraph::new(line), bar);
    }

    if desk.rows.len() > rows_visible {
        let mut scroll = ScrollbarState::new(desk.rows.len().saturating_sub(rows_visible))
            .position(desk.offset)
            .viewport_content_length(rows_visible);
        frame.render_stateful_widget(
            crate::tui::view::scrollbar(&g),
            Rect::new(area.x + area.width - 1, inner.y, 1, inner.height),
            &mut scroll,
        );
    }
}

/// The name's column and the next todo's, out of the `room` the two share:
/// the name whole while the next todo keeps its minimum. A window narrower
/// than that takes the difference from the name, or a long folder name
/// pushes the todo, the age and the warning past the border and the row no
/// longer says what is next.
fn name_and_todo(room: usize, name_w: usize) -> (usize, usize) {
    let name_w = name_w.min(room.saturating_sub(NEXT_TODO_MIN));
    (name_w, room - name_w)
}

/// `yours · 3 ────`: the group's name in the text colour, bold, its count
/// dim, and a rule to the border. The heading over the unread rows is
/// `reading`, dim throughout.
fn heading_line(
    app: &App,
    group: Option<&Assignee>,
    count: usize,
    hat: Option<&str>,
    width: usize,
) -> Line<'static> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let (label, style) = match group {
        Some(group) => (group.label(hat), theme.text().add_modifier(Modifier::BOLD)),
        None => (format!("reading {}", g.spin(app.elapsed_ms)), theme.dim()),
    };
    let head = format!("  {label} {} {count} ", g.sep);
    let rule = g.rule.repeat(width.saturating_sub(head.width()));
    Line::from(vec![
        Span::raw("  "),
        Span::styled(label, style),
        Span::styled(format!(" {} {count} ", g.sep), theme.dim()),
        Span::styled(rule, theme.dim()),
    ])
}

/// One project: `▸ ID0248  Lullaby_Remix  Main Edit · cut the first minute
/// 2 h  active`.
fn card_line(
    app: &App,
    card: &crate::core::desk::Card,
    read: bool,
    id_w: usize,
    name_w: usize,
    todo_w: usize,
    now: Option<&str>,
) -> Line<'static> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let selected = app
        .library
        .selected()
        .is_some_and(|p| p.path == card.project.path);
    let cursor = if selected { g.cursor } else { " " };
    let dim_all = !read;
    let text = if dim_all { theme.dim() } else { theme.text() };
    let id_style = if dim_all { theme.dim() } else { theme.accent() };

    let next = if !read {
        String::new()
    } else {
        card.next_todo_line(&format!(" {} ", g.sep))
    };
    let next_style = if card.next_todo.is_none() || dim_all {
        theme.dim()
    } else {
        theme.text()
    };
    let age = match (&card.quiet_since, now) {
        (Some(since), Some(now)) if read => crate::core::desk::age_label(since, now),
        (Some(since), None) if read => since.get(..10).unwrap_or(since).to_string(),
        _ => String::new(),
    };
    // `active` when a program is working in the folder; the warning when
    // the next todo is assigned to an owner, nothing has been written in the
    // folder for days, and no program is working in it.
    let (active, active_style) = if !card.active.is_empty() {
        ("active".to_string(), theme.accent_alt())
    } else if read
        && matches!(card.assignee(), Assignee::Owner(_))
        && now.is_some_and(|now| {
            crate::core::desk::quiet_for_days(card.quiet_since.as_deref(), now, STUCK_AFTER_DAYS)
        })
    {
        (g.warn.to_string(), theme.warn())
    } else {
        (String::new(), theme.dim())
    };

    let mut spans = vec![
        Span::styled(format!("{cursor} "), theme.accent()),
        Span::styled(format!("{:<id_w$} ", card.project.id), id_style),
        Span::styled(
            format!("{:<name_w$} ", fit(&card.project.name, name_w, g.ellipsis)),
            text,
        ),
    ];
    spans.extend(fit_spans(
        vec![Span::styled(format!("{next:<todo_w$}"), next_style)],
        todo_w,
        g.ellipsis,
    ));
    spans.push(Span::styled(format!(" {age:>AGE_CELL$} "), theme.dim()));
    spans.push(Span::styled(
        format!("{active:<ACTIVE_CELL$}"),
        active_style,
    ));
    Line::from(spans)
}

/// The bar's report for the desk: how many projects are under a group out
/// of the library in view, `reading n` while records are still landing,
/// the group shown alone, the order, and the library's own filters. The
/// order goes first when the bar is narrow, as the sort does on the library.
pub fn report(app: &App, width: usize) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let desk = &app.desk;
    let mut parts: Vec<(u8, Span)> = vec![(
        0,
        Span::styled(
            format!("{}/{}", desk.placed, app.library.in_view),
            theme.text(),
        ),
    )];
    if desk.unread > 0 {
        parts.push((
            3,
            Span::styled(
                format!(" reading {} {}", desk.unread, g.spin(app.elapsed_ms)),
                theme.dim(),
            ),
        ));
    }
    parts.push((
        4,
        Span::styled(format!(" {} {}", g.sep, desk.order.label()), theme.dim()),
    ));
    if let Some(group) = &desk.filter {
        parts.push((
            2,
            Span::styled(
                format!(" {} {}", g.sep, group.label(app.hat().as_deref())),
                theme.accent_alt(),
            ),
        ));
    }
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
    parts.push((0, Span::raw(" ")));
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
