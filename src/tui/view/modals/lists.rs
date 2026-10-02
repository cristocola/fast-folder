//! The dialogs that are a list to choose from: the palette, a picker, the
//! action menu, a list to tick.

use super::*;

pub(super) fn render_palette(
    app: &App,
    palette: &PaletteState,
    frame: &mut Frame,
    area: Rect,
) -> Position {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = centered(area, 70, 70);
    super::clear(frame, area, &app.theme);
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

    // What to *type*, not a key — which is why it lives on the palette's own
    // blank row rather than on the hint bar, where every pair is read from the
    // registry and `#` is nothing the registry knows. Only while the query is
    // empty: once there is one, the list below is the answer.
    if palette.input.text().is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" # or @ for projects only", theme.dim())),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }

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
            ListItem::new(split_line(left, right, width, g.ellipsis))
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

pub(super) fn render_pick(app: &App, pick: &PickState, frame: &mut Frame, area: Rect) -> Position {
    let theme = &app.theme;
    let g = theme.glyphs;
    let lines = pick.detail_lines(crate::tui::layout::wide_pick_text_width(area));
    let (area, shown_detail) = if pick.wide {
        crate::tui::layout::wide_pick_box(area, pick.ranked.len(), lines)
    } else {
        (crate::tui::layout::pick_box(area, pick.ranked.len()), 0)
    };

    super::clear(frame, area, &app.theme);
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

    // A wide picker keeps its last rows for the selected row's reason, under
    // a rule.
    let detail_rows = if pick.wide { shown_detail + 1 } else { 0 };
    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2 + detail_rows),
    );
    if pick.wide {
        let rule_y = list_area.y + list_area.height;
        frame.render_widget(
            Paragraph::new(Span::styled(
                g.rule.repeat(inner.width as usize),
                theme.dim(),
            )),
            Rect::new(inner.x, rule_y, inner.width, 1),
        );
        let detail = pick.selected_detail().unwrap_or_default();
        frame.render_widget(
            Paragraph::new(Span::styled(detail.to_string(), theme.text()))
                .wrap(Wrap { trim: true }),
            Rect::new(
                inner.x + 1,
                rule_y + 1,
                inner.width.saturating_sub(2),
                shown_detail,
            ),
        );
    }
    let items: Vec<ListItem> = pick
        .ranked
        .iter()
        .filter_map(|(index, hits)| pick.items.get(*index).map(|item| (item, hits)))
        .map(|(item, hits)| {
            let hits: Vec<usize> = hits.iter().map(|&h| h as usize).collect();
            let mut spans = vec![Span::raw(" ")];
            spans.extend(highlighted(&item.label, &hits, theme.text(), theme.hit()));
            if !item.detail.is_empty() && !pick.wide {
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

pub(super) fn render_actions(
    app: &App,
    actions: &ActionsState,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let g = app.theme.glyphs;
    let entries = crate::tui::app::actions::action_entries(app);
    // Over marks the verbs act on every one of them, and the title says so.
    let marked = app.library.marks.len();
    let title = match app.library.selected() {
        _ if marked > 0 => format!(" {marked} marked {} actions ", g.sep),
        Some(p) => format!(" {} {} actions ", p.id, g.sep),
        None => " actions ".to_string(),
    };
    let menu = VerbMenu {
        title,
        entries: &entries,
        selected: actions.selected,
        offset: actions.offset,
    };
    render_verb_menu(app, menu, |id| command::find(id).title, frame, area);
    None
}

/// A base's menu: the shape of the project action menu, about one base.
pub(super) fn render_base_menu(
    app: &App,
    menu: &crate::tui::app::bases::BaseMenu,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let g = app.theme.glyphs;
    let entries = crate::tui::app::bases::base_menu_entries(app);
    let label = app
        .base_in_hand()
        .map(|base| base.label.clone())
        .unwrap_or_default();
    let menu = VerbMenu {
        title: format!(" {label} {} actions ", g.sep),
        entries: &entries,
        selected: menu.selected,
        offset: 0,
    };
    render_verb_menu(
        app,
        menu,
        |id| crate::tui::app::bases::base_verb_title(app, id),
        frame,
        area,
    );
    None
}

/// A menu of verbs: its title, its rows and its cursor.
struct VerbMenu<'a> {
    title: String,
    entries: &'a [(CommandId, Availability)],
    selected: usize,
    offset: usize,
}

/// **A menu of verbs, each with its key**: the key, the title and what it
/// does, a verb that cannot run dimmed with its reason. `title_of` is what a
/// row is called — the registry's title, or one that names the change a
/// toggle would make.
fn render_verb_menu(
    app: &App,
    menu: VerbMenu<'_>,
    title_of: impl Fn(CommandId) -> &'static str,
    frame: &mut Frame,
    area: Rect,
) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let entries = menu.entries;
    let area = crate::tui::layout::actions_box(area, entries.len());
    super::clear(frame, area, &app.theme);
    let block = frame_block(app, menu.title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width as usize;
    // The key and title columns are as wide as the widest of each, and one
    // space wider than that, so a title never runs into its description.
    let key_w = entries
        .iter()
        .map(|(id, _)| {
            command::find(*id)
                .keys
                .first()
                .map(|k| k.label().chars().count())
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
        .max(6)
        + 1;
    let title_w = entries
        .iter()
        .map(|(id, _)| title_of(*id).chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 36)
        + 1;

    let items: Vec<ListItem> = entries
        .iter()
        .map(|(id, availability)| {
            let command = command::find(*id);
            // A verb with no key of its own shows `Enter`, which is what runs
            // the row under the cursor and is therefore the only way to reach
            // it. An empty column under a menu whose own description promises
            // "the verb under the cursor — or press its own key" reads as a
            // row that cannot be run at all.
            let key = command
                .keys
                .first()
                .map(|k| k.label())
                .unwrap_or_else(|| command::key_of(CommandId::ActionsRun));
            let (title_style, detail) = match availability {
                Availability::Enabled => (theme.text(), command.description),
                Availability::Disabled(reason) => (theme.dim(), *reason),
                // The entries leave `Hidden` out before this is reached, so a
                // row that says nothing cannot get here; the arm is only for
                // the match to be exhaustive.
                Availability::Hidden => (theme.dim(), ""),
            };
            let title = title_of(*id);
            let mut left = vec![Span::raw(" ")];
            left.push(Span::styled(pad(&key, key_w), theme.key()));
            // A description squeezed to a letter or two says nothing and
            // costs the title its room: on a narrow window the menu is keys
            // and titles, and a disabled verb says why when it is pressed.
            let room = width.saturating_sub(1 + key_w + title_w + 2);
            if room >= DESCRIPTION_MIN {
                left.push(Span::styled(pad(title, title_w), title_style));
                left.push(Span::styled(fit(detail, room, g.ellipsis), theme.dim()));
            } else {
                let title_room = width.saturating_sub(1 + key_w);
                left.push(Span::styled(
                    fit(title, title_room, g.ellipsis),
                    title_style,
                ));
            }
            ListItem::new(Line::from(left))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    let mut state = ListState::default()
        .with_offset(menu.offset)
        .with_selected(Some(menu.selected));
    frame.render_stateful_widget(list, inner, &mut state);
}

/// The least a description column in the action menu is worth: below it the
/// menu drops the column rather than draw a letter and an ellipsis.
const DESCRIPTION_MIN: usize = 12;

pub(super) fn render_multi_pick(
    app: &App,
    pick: &MultiPick,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let height = (pick.items.len() as u16 + 4).clamp(5, 16);
    let area = centered_fixed(area, 44, height);
    super::clear(frame, area, &app.theme);
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
