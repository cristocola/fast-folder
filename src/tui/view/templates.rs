//! The template strip under the table: one card per template, the busiest
//! first, and what the selected one is.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::app::{App, Focus};
use crate::tui::view::fit;

pub fn strip(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let focused = app.focus == Focus::Templates && app.modals.is_empty() && !app.search.editing;

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " templates ",
            if focused { theme.accent() } else { theme.dim() },
        ))
        .border_style(theme.border(focused));
    let inner = block.inner(panes[0]);
    frame.render_widget(block, panes[0]);

    let mut spans = vec![Span::raw(" ")];
    let mut used = 1usize;
    let room = inner.width as usize;
    for (i, card) in app.templates.cards.iter().enumerate() {
        let active = app.library.template_filter.as_deref() == Some(card.slug.as_str());
        let selected = focused && i == app.templates.selected;
        let text = format!("{} {}", card.slug, app.templates.count(&card.slug));
        let cell = format!(" {text} ");
        if used + cell.len() > room && i > 0 {
            spans.push(Span::styled(g.ellipsis, theme.dim()));
            break;
        }
        used += cell.len();
        // The filter that is on is underlined; the card the cursor is on is
        // highlighted like a row.
        let style = if selected {
            theme.selection
        } else if active {
            theme
                .accent()
                .add_modifier(ratatui::style::Modifier::UNDERLINED)
        } else {
            theme.text()
        };
        spans.push(Span::styled(cell, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" template ", theme.dim()))
        .border_style(theme.border(false));
    let inner = block.inner(panes[1]);
    frame.render_widget(block, panes[1]);
    let line = match app.templates.selected_card() {
        Some(card) => {
            let text = format!(
                " {} {} {} vars {} {}",
                card.name,
                g.sep,
                card.variables,
                g.sep,
                if card.naming_pattern.is_empty() {
                    "(no template on disk)".to_string()
                } else {
                    card.naming_pattern.clone()
                }
            );
            Line::from(Span::styled(
                fit(&text, inner.width as usize, g.ellipsis),
                theme.text(),
            ))
        }
        None => Line::from(Span::styled(" no templates yet", theme.dim())),
    };
    frame.render_widget(Paragraph::new(line), inner);
}
