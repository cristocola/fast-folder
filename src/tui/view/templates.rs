//! The templates tab: every template on disk, what each one is, and how many
//! projects were made from it.
//!
//! It replaces two things at once — an 84 %-wide studio modal over the library,
//! and a three-row strip along the bottom of the library that showed the same
//! counts and could be filtered by pressing Enter on a card and nothing else.
//! Neither was a place you could work, and between them they cost the table
//! three rows on every screen.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::{App, TemplatesState};
use crate::tui::view::{fit, pad};

/// The widest a slug column gets; a longer one is cut with an ellipsis.
const SLUG_MAX: usize = 24;

/// The list of templates, with the selected one's details beside it.
pub fn screen(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let studio = &app.studio;
    let focused = app.modals.is_empty();

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);

    // --- the list ---------------------------------------------------------
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " templates ",
            if focused { theme.accent() } else { theme.dim() },
        ))
        .border_style(theme.border(focused));
    let inner = block.inner(panes[0]);
    frame.render_widget(block, panes[0]);

    let rows = studio.rows(app.search.input.text());
    if rows.is_empty() {
        let sentence = if studio.cards.is_empty() {
            "no templates yet — n makes one, g reads one out of a folder"
        } else {
            "nothing matches"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                fit(sentence, inner.width as usize, g.ellipsis),
                theme.dim(),
            )),
            inner,
        );
    } else {
        // Measured, not fixed: a slug can never run into its count.
        let slug_w = rows
            .iter()
            .filter_map(|&i| studio.cards.get(i))
            .map(|card| TemplatesState::display_name(card).width())
            .max()
            .unwrap_or(8)
            .min(SLUG_MAX);
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

    // --- the detail -------------------------------------------------------
    let title = match studio.selected_card() {
        Some(card) => format!(" {} ", TemplatesState::display_name(card)),
        None => " template ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, theme.dim()))
        .border_style(theme.border(false));
    let inner = block.inner(panes[1]);
    frame.render_widget(block, panes[1]);

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
                        if uses == 1 { "" } else { "s" },
                        if uses == 1 { "s" } else { "" }
                    ),
                    theme.text(),
                )),
            ]
        }
        Some(_) if studio.lines.is_empty() => {
            vec![Line::from(Span::styled(" reading…", theme.dim()))]
        }
        Some(_) => studio
            .lines
            .iter()
            .map(|line| Line::from(Span::styled(format!(" {line}"), theme.text())))
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
