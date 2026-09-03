//! What is drawn over the dashboard: the palette, help, a picker, a message.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::App;
use crate::tui::app::actions::{ActionsState, Confirm, MultiPick, TextPrompt};
use crate::tui::app::modal::{MessageLevel, Modal, PickState};
use crate::tui::app::palette::PaletteState;
use crate::tui::command::{self, Availability};
use crate::tui::layout::{centered, centered_fixed};
use crate::tui::view::{fit, highlighted, pad, split_line};

/// Draw the top modal, if any. Returns where its caret is.
pub fn render(app: &App, frame: &mut Frame, area: Rect) -> Option<Position> {
    match app.modals.top()? {
        Modal::Palette(palette) => Some(render_palette(app, palette, frame, area)),
        Modal::Help { ctx, scroll } => {
            render_help(app, *ctx, *scroll, frame, area);
            None
        }
        Modal::Pick(pick) => Some(render_pick(app, pick, frame, area)),
        Modal::Actions(actions) => render_actions(app, actions, frame, area),
        Modal::TextPrompt(prompt) => Some(render_text_prompt(app, prompt, frame, area)),
        Modal::Confirm(confirm) => render_confirm(app, confirm, frame, area),
        Modal::MultiPick(pick) => render_multi_pick(app, pick, frame, area),
        Modal::Message {
            title,
            lines,
            level,
            scroll,
        } => {
            render_message(app, title, lines, *level, *scroll, frame, area);
            None
        }
    }
}

/// The move job's progress, drawn over the dashboard while one runs. It is not
/// a modal on the stack — it shares the lifetime of `App::busy` and disappears
/// when the move answers.
pub fn render_move_progress(app: &App, frame: &mut Frame, area: Rect) {
    let Some(progress) = &app.move_progress else {
        return;
    };
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = centered_fixed(area, 52, 9);
    frame.render_widget(Clear, area);
    let block = frame_block(app, " moving ".to_string(), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(
            format!(" {} ", progress.phase.as_str()),
            theme.accent(),
        )),
        Line::from(vec![
            Span::styled(
                format!(" {} of {} files", progress.done_files, progress.total_files),
                theme.text(),
            ),
            Span::styled(
                format!(
                    "   {} of {}",
                    crate::util::human_bytes::human_bytes(progress.copied_bytes),
                    crate::util::human_bytes::human_bytes(progress.total_bytes)
                ),
                theme.dim(),
            ),
        ]),
        Line::from(Span::styled(
            format!(
                " {}",
                fit(&progress.current_file, inner.width as usize, g.ellipsis)
            ),
            theme.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(" Ctrl-C cancels", theme.dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn frame_block<'a>(app: &App, title: String, focused: bool) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, app.theme.accent()))
        .border_style(app.theme.border(focused))
}

fn render_palette(app: &App, palette: &PaletteState, frame: &mut Frame, area: Rect) -> Position {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = centered(area, 70, 70);
    frame.render_widget(Clear, area);
    let block = frame_block(app, " commands ".to_string(), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let input_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let caret = palette
        .input
        .render_line(
            input_area,
            frame.buffer_mut(),
            Span::styled(format!(" {} ", g.search), theme.accent()),
            theme.text(),
        )
        .unwrap_or(Position::new(inner.x, inner.y));

    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let width = list_area.width as usize;
    let items: Vec<ListItem> = palette
        .entries
        .iter()
        .map(|entry| {
            let hits: Vec<usize> = entry.hits.iter().map(|&h| h as usize).collect();
            let title_style = if entry.enabled {
                theme.bold()
            } else {
                theme.dim()
            };
            let mut left = vec![Span::raw(" ")];
            left.extend(highlighted(&entry.title, &hits, title_style, theme.hit()));
            let detail = match entry.reason {
                Some(reason) => format!("  {} {reason}", g.sep),
                None if entry.detail.is_empty() => String::new(),
                None => format!("  {} {}", g.sep, entry.detail),
            };
            let left_width: usize = left.iter().map(|s| s.width()).sum();
            let room = width.saturating_sub(left_width + entry.key.width() + 3);
            left.push(Span::styled(fit(&detail, room, g.ellipsis), theme.dim()));
            let right = vec![Span::styled(entry.key.clone(), theme.key()), Span::raw(" ")];
            ListItem::new(split_line(left, right, width))
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" nothing matches", theme.dim())),
            list_area,
        );
        return caret;
    }
    let list = List::new(items).highlight_style(theme.selection);
    let mut state = ListState::default()
        .with_offset(palette.offset)
        .with_selected(palette.selected);
    frame.render_stateful_widget(list, list_area, &mut state);
    caret
}

fn render_pick(app: &App, pick: &PickState, frame: &mut Frame, area: Rect) -> Position {
    let theme = &app.theme;
    let g = theme.glyphs;
    let height = (pick.ranked.len() as u16 + 4).clamp(6, 16);
    let area = centered_fixed(area, 50, height);
    frame.render_widget(Clear, area);
    let block = frame_block(app, format!(" {} ", pick.title), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let input_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let caret = pick
        .query
        .render_line(
            input_area,
            frame.buffer_mut(),
            Span::styled(format!(" {} ", g.search), theme.accent()),
            theme.text(),
        )
        .unwrap_or(Position::new(inner.x, inner.y));

    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let items: Vec<ListItem> = pick
        .ranked
        .iter()
        .filter_map(|(index, hits)| pick.items.get(*index).map(|item| (item, hits)))
        .map(|(item, hits)| {
            let hits: Vec<usize> = hits.iter().map(|&h| h as usize).collect();
            let mut spans = vec![Span::raw(" ")];
            spans.extend(highlighted(&item.label, &hits, theme.text(), theme.hit()));
            if !item.detail.is_empty() {
                spans.push(Span::styled(
                    format!("  {} {}", g.sep, item.detail),
                    theme.dim(),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    let mut state = ListState::default()
        .with_offset(pick.offset)
        .with_selected(pick.selected);
    frame.render_stateful_widget(list, list_area, &mut state);
    caret
}

fn render_actions(
    app: &App,
    actions: &ActionsState,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let entries = crate::tui::app::actions::action_entries(app);
    let height = (entries.len() as u16 + 4).clamp(8, 30);
    let area = centered_fixed(area, 64, height);
    frame.render_widget(Clear, area);
    let project = app.library.selected();
    let title = project
        .map(|p| format!(" {} · actions ", p.id))
        .unwrap_or_else(|| " actions ".to_string());
    let block = frame_block(app, title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width as usize;

    let items: Vec<ListItem> = entries
        .iter()
        .map(|(id, availability)| {
            let command = command::find(*id);
            let key = command.keys.first().map(|k| k.label()).unwrap_or_default();
            let (title_style, detail) = match availability {
                Availability::Enabled => (theme.text(), command.description),
                Availability::Disabled(reason) => (theme.dim(), *reason),
                Availability::Hidden => (theme.dim(), ""),
            };
            let mut left = vec![Span::raw(" ")];
            left.push(Span::styled(pad(&key, 7), theme.key()));
            left.push(Span::styled(pad(command.title, 24), title_style));
            let room = width.saturating_sub(1 + 8 + 24 + 2);
            left.push(Span::styled(fit(detail, room, g.ellipsis), theme.dim()));
            ListItem::new(Line::from(left))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    let mut state = ListState::default()
        .with_offset(actions.offset)
        .with_selected(Some(actions.selected));
    frame.render_stateful_widget(list, inner, &mut state);
    None
}

fn render_text_prompt(app: &App, prompt: &TextPrompt, frame: &mut Frame, area: Rect) -> Position {
    use crate::tui::app::actions::TextThen;

    let theme = &app.theme;
    let verb = match prompt.then {
        TextThen::Rename => "rename",
        TextThen::AddTag => "add a tag",
        TextThen::Note => "note",
        TextThen::Delete { .. } => "delete",
    };
    let area = centered_fixed(area, 62, 8);
    frame.render_widget(Clear, area);
    let block = frame_block(app, format!(" {} ", verb), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The prompt itself, wrapped, above the input line.
    let prompt_area = Rect::new(inner.x, inner.y, inner.width, 2);
    frame.render_widget(
        Paragraph::new(Span::styled(prompt.title.clone(), theme.dim())).wrap(Wrap { trim: false }),
        prompt_area,
    );

    let input_area = Rect::new(inner.x, inner.y + 2, inner.width, 1);
    let caret = prompt
        .input
        .render_line(input_area, frame.buffer_mut(), Span::raw(" "), theme.text())
        .unwrap_or(Position::new(inner.x, inner.y + 2));
    if let Some(error) = &prompt.error {
        let error_area = Rect::new(inner.x, inner.y + 3, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {error}"), theme.warn())),
            error_area,
        );
    }
    caret
}

fn render_confirm(app: &App, confirm: &Confirm, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    let area = centered_fixed(area, 64, 8);
    frame.render_widget(Clear, area);
    let block = frame_block(app, " confirm ".to_string(), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::styled(format!(" {}", confirm.prompt), theme.text())),
        Line::from(""),
        Line::from(vec![
            Span::styled("y ", theme.key()),
            Span::styled("yes   ", theme.dim()),
            Span::styled("n ", theme.key()),
            Span::styled("no   ", theme.dim()),
            Span::styled("Esc ", theme.key()),
            Span::styled("cancel", theme.dim()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    None
}

fn render_multi_pick(
    app: &App,
    pick: &MultiPick,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let height = (pick.items.len() as u16 + 4).clamp(5, 16);
    let area = centered_fixed(area, 44, height);
    frame.render_widget(Clear, area);
    let block = frame_block(app, format!(" {} ", pick.title), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = pick
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mark = if pick.picked[i] { g.mark } else { " " };
            let style = if i == pick.selected {
                theme.text()
            } else {
                theme.dim()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{mark} "), theme.accent()),
                Span::styled(item.clone(), style),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    let mut state = ListState::default().with_selected(Some(pick.selected));
    frame.render_stateful_widget(list, inner, &mut state);
    None
}

fn render_help(app: &App, ctx: command::Context, scroll: usize, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = centered(area, 84, 84);
    frame.render_widget(Clear, area);
    let block = frame_block(app, format!(" help {} {} ", g.sep, ctx.label()), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for (category, commands) in command::help_sections(ctx) {
        lines.push(Line::from(Span::styled(
            format!(" {}", category.label()),
            theme.accent(),
        )));
        for c in commands {
            let keys = c
                .keys
                .iter()
                .map(|k| k.label())
                .collect::<Vec<_>>()
                .join(" / ");
            lines.push(Line::from(vec![
                Span::styled(format!("   {} ", pad(&keys, 14)), theme.key()),
                Span::styled(pad(c.title, 30), theme.text()),
                Span::styled(
                    fit(
                        c.description,
                        (inner.width as usize).saturating_sub(49),
                        g.ellipsis,
                    ),
                    theme.dim(),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        " The command palette (c) lists every command with its key; type to filter.",
        theme.dim(),
    )));
    lines.push(Line::from(Span::styled(
        " Search: a word matches inside a name, id, template or tag (a typo is forgiven); tag:x  template=y  created>date match exactly.",
        theme.dim(),
    )));
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    let paragraph = Paragraph::new(lines).scroll((scroll.min(max_scroll) as u16, 0));
    frame.render_widget(paragraph, inner);
}

fn render_message(
    app: &App,
    title: &str,
    lines: &[String],
    level: MessageLevel,
    scroll: usize,
    frame: &mut Frame,
    area: Rect,
) {
    let theme = &app.theme;
    let area = centered(area, 70, 50);
    frame.render_widget(Clear, area);
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
        .map(|line| {
            Line::from(Span::styled(
                format!(" {line}"),
                Style::default().fg(theme.text),
            ))
        })
        .collect();
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    frame.render_widget(paragraph, inner);
}
