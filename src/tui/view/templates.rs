//! The templates tab: every template on disk, what each one is, and how many
//! projects were made from it.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::{App, Focus, TemplatesState};
use crate::tui::layout;
use crate::tui::view::{fit, pad};

/// The widest a slug column gets; a longer one is cut with an ellipsis.
pub const SLUG_MAX: usize = 24;

/// The list of templates, with the selected one's details beside it.
pub fn screen(app: &App, frame: &mut Frame, area: Rect) {
    // Two panes, one focus: the list has it unless `→` or Tab put it in the
    // pane, and neither has it under a dialog.
    let focused = app.modals.is_empty() && app.focus == Focus::Projects;
    let pane_focused = app.modals.is_empty() && app.focus == Focus::Detail;

    let (list_area, pane_area, placement) = layout::templates_panes(area, app.templates_needs());
    let panes = [list_area, pane_area];
    // Where the pane takes the list's place, one of the two is drawn: the
    // pane while it has the focus, the list otherwise.
    // Which of the two is drawn follows the focus alone, not whether a dialog
    // is up: help opened over the template must not swap the list in behind it.
    let over = placement == layout::Placement::Over;
    let in_pane = app.focus == Focus::Detail;
    let draw_list = !over || !in_pane;
    let draw_pane = !over || in_pane;

    // --- the list ---------------------------------------------------------
    if draw_list {
        template_list(app, frame, panes[0], focused);
    }
    if !draw_pane {
        return;
    }

    // --- the detail -------------------------------------------------------
    template_detail(app, frame, panes[1], pane_focused);
}

fn template_list(app: &App, frame: &mut Frame, area: Rect, focused: bool) {
    let studio = &app.studio;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " templates ",
            crate::tui::view::projects::title_style(app, focused),
        ))
        .border_style(crate::tui::view::projects::border_style(app, focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = studio.rows(app.search.input.text());
    if rows.is_empty() {
        empty_list(app, frame, inner);
    } else {
        list_rows(app, frame, inner, rows);
    }
}

/// The registry's sentence, plus the two other ways in that only this
/// tab offers — one of which is the only one that explains anything.
fn empty_list(app: &App, frame: &mut Frame, inner: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let studio = &app.studio;
    let lines: Vec<String> = if studio.cards.is_empty() {
        vec![
            format!(
                "{}, or {} reads one out of a folder",
                crate::tui::command::NO_TEMPLATES,
                crate::tui::command::key_of(crate::tui::command::CommandId::StudioFromFolder)
            ),
            format!(
                "{} explains what a template is and walks through building one",
                crate::tui::command::key_of(crate::tui::command::CommandId::Guide)
            ),
        ]
    } else {
        vec!["nothing matches".to_string()]
    };
    frame.render_widget(
        Paragraph::new(
            lines
                .iter()
                .map(|line| {
                    Line::from(Span::styled(
                        fit(line, inner.width as usize, g.ellipsis),
                        theme.dim(),
                    ))
                })
                .collect::<Vec<_>>(),
        ),
        inner,
    );
}

fn list_rows(app: &App, frame: &mut Frame, inner: Rect, rows: Vec<usize>) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let studio = &app.studio;
    // Measured, not fixed: a slug can never run into its count.
    let slug_w = rows
        .iter()
        .filter_map(|&i| studio.cards.get(i))
        .map(|card| TemplatesState::display_name(card).width())
        .max()
        .unwrap_or(8)
        .min(SLUG_MAX)
        // A list narrower than its names cuts them with the ellipsis,
        // and the count keeps its column.
        .min((inner.width as usize).saturating_sub(2 + 5));
    let items: Vec<ListItem> = rows
        .iter()
        .filter_map(|&i| studio.cards.get(i))
        .map(|card| {
            let count = app.templates.count(&card.slug);
            let filtered = app.library.template_filter.as_deref() == Some(card.slug.as_str());
            // A slug no template on disk answers to recedes: it is a
            // project's memory of a template, not a template.
            let style = if !card.on_disk {
                theme.dim()
            } else {
                theme.text()
            };
            ListItem::new(Line::from(vec![
                Span::styled(if filtered { g.cursor } else { " " }, theme.accent_alt()),
                Span::raw(" "),
                Span::styled(
                    pad(
                        &fit(TemplatesState::display_name(card), slug_w, g.ellipsis),
                        slug_w,
                    ),
                    style,
                ),
                Span::styled(format!("{count:>5}"), theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    let mut state = ListState::default()
        .with_offset(studio.offset)
        .with_selected(studio.row_of(studio.selected, &rows));
    frame.render_stateful_widget(list, inner, &mut state);
}

fn template_detail(app: &App, frame: &mut Frame, area: Rect, pane_focused: bool) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let studio = &app.studio;
    let title = match studio.selected_card() {
        Some(card) => format!(" {} ", TemplatesState::display_name(card)),
        None => " template ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            crate::tui::view::projects::title_style(app, pane_focused),
        ))
        .border_style(crate::tui::view::projects::border_style(app, pane_focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = match studio.selected_card() {
        Some(card) if !card.on_disk => {
            let uses = app.templates.count(&card.slug);
            vec![
                Line::from(Span::styled(
                    format!(
                        " no template on disk answers to '{}'",
                        TemplatesState::display_name(card)
                    ),
                    theme.dim(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        " {uses} project{} name{} it",
                        crate::util::plural::s(uses),
                        crate::util::plural::of(uses, "s", "")
                    ),
                    theme.text(),
                )),
            ]
        }
        Some(_) if studio.lines.is_empty() => {
            vec![Line::from(Span::styled(" reading…", theme.dim()))]
        }
        // Cut to the pane with the app's own ellipsis, like every other cell,
        // so a cut line says it was cut. A line never wraps, or the tree
        // beside it would stop lining up; the pane scrolls, so the tail of a
        // long one is reachable.
        Some(_) => studio
            .lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    format!(
                        " {}",
                        fit(line, inner.width.saturating_sub(1) as usize, g.ellipsis)
                    ),
                    theme.text(),
                ))
            })
            .collect(),
        None => vec![Line::from(Span::styled(" nothing selected", theme.dim()))],
    };
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    frame.render_widget(
        Paragraph::new(lines).scroll((studio.scroll.min(max_scroll) as u16, 0)),
        inner,
    );
}

/// The templates tab's own search bar: the same field the library uses, over
/// the slugs and names rather than the projects, with the counts on the right.
pub fn bar(app: &App, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = area.width as usize;
    let studio = &app.studio;

    let shown = studio.rows(app.search.input.text()).len();
    let mut right = vec![Span::styled(
        format!("{shown}/{}", studio.cards.len()),
        theme.text(),
    )];
    if let Some(slug) = &app.library.template_filter {
        right.push(Span::styled(
            format!(" {} filtering {slug}", g.sep),
            theme.accent_alt(),
        ));
    }
    right.push(Span::raw(" "));
    let right_width: usize = right.iter().map(|s| s.width()).sum::<usize>().min(width);

    let prefix = format!(" {} ", g.search);
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

    let caret = if app.search.editing || !app.search.input.text().is_empty() {
        app.search
            .input
            .render_line(text_area, frame.buffer_mut(), prefix_span, theme.text())
    } else {
        let line = Line::from(vec![
            prefix_span,
            Span::styled(
                fit(
                    "/ to search the templates",
                    text_room.saturating_sub(prefix.width()),
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
