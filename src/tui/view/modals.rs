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
use crate::tui::app::wizard::{Flow, Preview, Step};
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
        Modal::Note(note) => Some(render_note(app, note, frame, area)),
        Modal::Confirm(confirm) => render_confirm(app, confirm, frame, area),
        Modal::MultiPick(pick) => render_multi_pick(app, pick, frame, area),
        Modal::Flow(flow) => render_flow(app, flow, frame, area),
        Modal::Builder(builder) => {
            crate::tui::view::builder::render_builder(app, builder, frame, area)
        }
        Modal::Settings(state) => {
            crate::tui::view::builder::render_settings(app, state, frame, area)
        }
        Modal::Onboarding(state) => {
            crate::tui::view::builder::render_onboarding(app, state, frame, area)
        }
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

/// A progress bar, drawn from the theme's own two glyphs.
///
/// `width` cells, `done` of `total` filled, with the percentage after it. A
/// `total` of zero draws an empty track rather than a full one — nothing
/// measured is not everything done, and a move that has not scanned its
/// manifest yet would otherwise open at 100 %.
fn bar<'a>(app: &App, width: usize, done: u64, total: u64) -> Line<'a> {
    let g = app.theme.glyphs;
    let width = width.max(4);
    let filled = if total == 0 {
        0
    } else {
        // Saturating, because `done` is read from a live mutex and a manifest
        // can be re-totalled between two frames.
        ((done.min(total) as u128 * width as u128) / total as u128) as usize
    };
    let percent = if total == 0 {
        0
    } else {
        (done.min(total) as u128 * 100 / total as u128) as u64
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(g.bar_full.repeat(filled), app.theme.accent()),
        Span::styled(g.bar_empty.repeat(width - filled), app.theme.dim()),
        Span::styled(format!(" {percent:>3}%"), app.theme.dim()),
    ])
}

/// The move job's progress, drawn over the dashboard while one runs. It is not
/// a modal on the stack — it shares the lifetime of `App::busy` and disappears
/// when the move answers. A batch job draws its own modal (`render_job`),
/// which shows the move detail for its moving items, so this stays out of the
/// way while `App::job` is up.
pub fn render_move_progress(app: &App, frame: &mut Frame, area: Rect) {
    if app.job.is_some() {
        return;
    }
    let Some(progress) = &app.move_progress else {
        return;
    };
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = 54.min(area.width);
    let track = (width as usize).saturating_sub(9);
    let lines = vec![
        Line::from(Span::styled(
            format!(" {} ", progress.phase.as_str()),
            theme.accent(),
        )),
        bar(app, track, progress.copied_bytes, progress.total_bytes),
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
                fit(&progress.current_file, width as usize - 3, g.ellipsis)
            ),
            theme.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(" Ctrl-C cancels", theme.dim())),
    ];
    let area = centered_fixed(area, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    let block = frame_block(app, " moving ".to_string(), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A batch job's progress, drawn over the dashboard while one runs. Like
/// `render_move_progress`, it is not a modal on the stack — it lives exactly
/// as long as `App::job`. The modal names the item being acted on and counts
/// the failures so far; a moving item's byte progress is folded in beneath.
pub fn render_job(app: &App, frame: &mut Frame, area: Rect) {
    let Some(job) = &app.job else {
        return;
    };
    let theme = &app.theme;
    let g = theme.glyphs;
    // Wide enough for the cancel line, which names what happens to the rows
    // that have not run — a cut sentence there is the one worth reading whole.
    let width = 68.min(area.width);
    let track = (width as usize).saturating_sub(9);

    let mut lines = vec![
        Line::from(Span::styled(
            format!(" {} ", job.progress_line()),
            theme.accent(),
        )),
        // The items, always — a batch of deletes or tags has no bytes to
        // report and its bar is the only thing that moves.
        bar(app, track, job.finished() as u64, job.total() as u64),
        Line::from(Span::styled(
            format!(
                " {}",
                match &job.inflight {
                    Some(project) => fit(&project.name, width as usize - 3, g.ellipsis),
                    None => "finishing…".to_string(),
                }
            ),
            theme.text(),
        )),
    ];
    if let Some(progress) = &app.move_progress {
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", progress.phase.as_str()), theme.accent()),
            Span::styled(
                format!(
                    "{} of {}",
                    crate::util::human_bytes::human_bytes(progress.copied_bytes),
                    crate::util::human_bytes::human_bytes(progress.total_bytes)
                ),
                theme.dim(),
            ),
        ]));
        lines.push(bar(app, track, progress.copied_bytes, progress.total_bytes));
    }
    if !job.failed.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(" {} failed so far", job.failed.len()),
            theme.warn(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Esc or Ctrl-C cancels — the rest stay marked",
        theme.dim(),
    )));

    // **Sized to what it holds**, like every other dialog here: a height
    // guessed at the widest case left blank rows under a two-line batch and
    // cut the cancel line off the tall one.
    let area = centered_fixed(area, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    let block = frame_block(app, format!(" {} ", job.kind.verb()), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
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

fn render_pick(app: &App, pick: &PickState, frame: &mut Frame, area: Rect) -> Position {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = crate::tui::layout::pick_box(area, pick.ranked.len());

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
    let area = crate::tui::layout::actions_box(area, entries.len());
    frame.render_widget(Clear, area);
    let project = app.library.selected();
    // Over marks the verbs act on every one of them, and the title says so.
    let marked = app.library.marks.len();
    let title = match project {
        _ if marked > 0 => format!(" {marked} marked {} actions ", g.sep),
        Some(p) => format!(" {} {} actions ", p.id, g.sep),
        None => " actions ".to_string(),
    };
    let block = frame_block(app, title, true);
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
        .map(|(id, _)| command::find(*id).title.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 36)
        + 1;

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
            left.push(Span::styled(pad(&key, key_w), theme.key()));
            left.push(Span::styled(pad(command.title, title_w), title_style));
            let room = width.saturating_sub(1 + key_w + title_w + 2);
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
        TextThen::Delete => "delete",
        TextThen::RaiseCounter => "ID counter",
        TextThen::CopyTo => "copy to",
    };
    // The box grows with its question: a confirmation over six marked
    // folders names all six.
    let width: u16 = 62;
    let prompt_rows = wrapped_rows(&prompt.title, usize::from(width) - 3).clamp(1, 6) as u16;
    let area = centered_fixed(area, width, prompt_rows + 6);
    frame.render_widget(Clear, area);
    let block = frame_block(app, format!(" {} ", verb), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The prompt itself, wrapped, above the input line — inset a column, so
    // every wrapped line sits where the first one does.
    let prompt_area = inset(Rect::new(inner.x, inner.y, inner.width, prompt_rows));
    frame.render_widget(
        Paragraph::new(Span::styled(prompt.title.clone(), theme.dim())).wrap(Wrap { trim: false }),
        prompt_area,
    );

    let input_area = Rect::new(inner.x, inner.y + prompt_rows + 1, inner.width, 1);
    let caret = prompt
        .input
        .render_line(input_area, frame.buffer_mut(), Span::raw(" "), theme.text())
        .unwrap_or(Position::new(inner.x, input_area.y));
    if let Some(error) = &prompt.error {
        let error_area = Rect::new(inner.x, input_area.y + 1, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {error}"), theme.warn())),
            error_area,
        );
    }
    caret
}

/// One column of padding on the left: wrapped text drawn here keeps every
/// line where the first one starts, which a leading space cannot do.
fn inset(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(1),
        area.height,
    )
}

/// How many rows `text` takes when word-wrapped at `width` columns — the
/// greedy wrap `Paragraph::wrap` does, a word at a time and a long word
/// broken where it must be — so a box can be sized to its question.
fn wrapped_rows(text: &str, width: usize) -> usize {
    let width = width.max(1);
    text.lines()
        .map(|line| {
            let mut rows = 1usize;
            let mut used = 0usize;
            for word in line.split(' ') {
                let w = word.width();
                if used == 0 {
                    used = w;
                } else if used + 1 + w <= width {
                    used += 1 + w;
                } else {
                    rows += 1;
                    used = w;
                }
                while used > width {
                    rows += 1;
                    used -= width;
                }
            }
            rows
        })
        .sum()
}

/// The quick note: a small text area over the dashboard. Enter saves,
/// Alt-Enter breaks a line, and a pasted paragraph lands whole.
fn render_note(
    app: &App,
    note: &crate::tui::app::actions::NoteState,
    frame: &mut Frame,
    area: Rect,
) -> Position {
    let theme = &app.theme;
    let g = theme.glyphs;
    let rows = (note.area.lines().len() as u16).clamp(3, 8);
    let area = centered_fixed(area, 62, rows + 5);
    frame.render_widget(Clear, area);
    let title = if note.count > 1 {
        format!(" note {} {} projects ", g.sep, note.count)
    } else {
        " note ".to_string()
    };
    let block = frame_block(app, title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", crate::tui::validators::note_prompt(note.count)),
            theme.dim(),
        )),
        prompt_area,
    );
    let text_area = Rect::new(
        inner.x + 1,
        inner.y + 1,
        inner.width.saturating_sub(1),
        rows,
    );
    let caret = note
        .area
        .render(text_area, frame.buffer_mut(), theme.text())
        .unwrap_or(Position::new(text_area.x, text_area.y));
    let keys_area = Rect::new(inner.x, inner.y + 1 + rows, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Enter ", theme.key()),
            Span::styled("save   ", theme.dim()),
            Span::styled("Alt-Enter ", theme.key()),
            Span::styled("new line   ", theme.dim()),
            Span::styled("Esc ", theme.key()),
            Span::styled("cancel", theme.dim()),
        ])),
        keys_area,
    );
    caret
}

fn render_confirm(app: &App, confirm: &Confirm, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    // Sized to its question, which names every folder it is about.
    let width: u16 = 64;
    let rows = wrapped_rows(&confirm.prompt, usize::from(width) - 3).clamp(1, 8) as u16;
    let area = centered_fixed(area, width, rows + 5);
    frame.render_widget(Clear, area);
    let block = frame_block(app, " confirm ".to_string(), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt_area = inset(Rect::new(inner.x, inner.y, inner.width, rows));
    frame.render_widget(
        Paragraph::new(Span::styled(confirm.prompt.clone(), theme.text()))
            .wrap(Wrap { trim: false }),
        prompt_area,
    );
    let keys_area = Rect::new(inner.x, inner.y + rows + 1, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" y ", theme.key()),
            Span::styled("yes   ", theme.dim()),
            Span::styled("n ", theme.key()),
            Span::styled("no   ", theme.dim()),
            Span::styled("Esc ", theme.key()),
            Span::styled("cancel", theme.dim()),
        ])),
        keys_area,
    );
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

// ---------------------------------------------------------------------------
// The flows: create, apply, register
// ---------------------------------------------------------------------------

/// A flow is one dialog with two faces: the questions, and what answering them
/// would do. Both are drawn in the same frame at the same size, so committing
/// and going back do not move the box under the reader.
fn render_flow(app: &App, flow: &Flow, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    // Sized to what it holds, so the footer sits under the last answer rather
    // than at the bottom of a mostly-empty box. The preview takes the room it
    // needs and scrolls past that.
    let width = (area.width * 76 / 100).clamp(46.min(area.width), 96);
    let body = match flow.step {
        Step::Form => flow.form.rows() as u16,
        Step::Preview => preview_height(flow),
    };
    let height = (body + 4).clamp(8.min(area.height), area.height * 88 / 100);
    let area = centered_fixed(area, width, height);
    frame.render_widget(Clear, area);
    let title = match flow.step {
        Step::Form => format!(" {} ", flow.kind.title()),
        Step::Preview => format!(" {} {} preview ", flow.kind.title(), theme.glyphs.sep),
    };
    let block = frame_block(app, title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 4 {
        return None;
    }

    // Two rows are reserved at the bottom: the footer's hint or refusal, and
    // the keys. Every state has both, so nothing below the fold is a surprise.
    let body = Rect::new(inner.x, inner.y, inner.width, inner.height - 2);
    let footer = Rect::new(inner.x, inner.y + body.height, inner.width, 1);
    let keys = Rect::new(inner.x, inner.y + body.height + 1, inner.width, 1);

    let caret = match flow.step {
        Step::Form => render_flow_form(app, flow, frame, body),
        Step::Preview => {
            render_flow_preview(app, flow, frame, body);
            None
        }
    };

    let (footer_text, footer_style) = match (flow.form.error(), flow.pending) {
        (Some(error), _) => (format!(" {} {error}", theme.glyphs.warn), theme.warn()),
        (None, true) => (" working…".to_string(), theme.dim()),
        (None, false) => match flow.step {
            Step::Form => (
                format!(
                    " {}",
                    flow.form
                        .focused()
                        .map(|field| field.hint.clone())
                        .unwrap_or_default()
                ),
                theme.dim(),
            ),
            Step::Preview => (String::new(), theme.dim()),
        },
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            fit(&footer_text, inner.width as usize, theme.glyphs.ellipsis),
            footer_style,
        )),
        footer,
    );

    let key_line = match flow.step {
        Step::Form => vec![
            Span::styled(" Tab ", theme.key()),
            Span::styled("next field   ", theme.dim()),
            Span::styled("Enter ", theme.key()),
            Span::styled("preview   ", theme.dim()),
            Span::styled("Esc ", theme.key()),
            Span::styled("cancel", theme.dim()),
        ],
        Step::Preview => vec![
            Span::styled(" Enter ", theme.key()),
            Span::styled(
                format!("{}   ", flow.kind.commit().trim_start_matches("Enter ")),
                theme.dim(),
            ),
            Span::styled("Esc ", theme.key()),
            Span::styled("back to the answers   ", theme.dim()),
            Span::styled("↑ ↓ ", theme.key()),
            Span::styled("scroll", theme.dim()),
        ],
    };
    frame.render_widget(Paragraph::new(Line::from(key_line)), keys);
    caret
}

/// How many lines the preview wants, so a short one gets a short box.
fn preview_height(flow: &Flow) -> u16 {
    match &flow.preview {
        Some(Preview::Create(report)) => {
            (report.structure.len() + report.files.len() + report.values.len() + 10) as u16
        }
        Some(Preview::Apply(apply)) => (apply.rows.len() + 5) as u16,
        Some(Preview::Register(register)) => 6 + u16::from(register.pinfo_exists) * 2,
        Some(Preview::Recursive(recursive)) => (recursive.rows.len() + 5) as u16,
        Some(Preview::FromFolder(scan)) => {
            (scan.structure.len() + scan.files.len() + scan.assets.len() + 8) as u16
        }
        None => 3,
    }
}

fn render_flow_form(app: &App, flow: &Flow, frame: &mut Frame, area: Rect) -> Option<Position> {
    let label_width = flow
        .form
        .visible()
        .map(|(_, field)| field.label.width())
        .max()
        .unwrap_or(8)
        .min(24);
    flow.form
        .render(area, frame.buffer_mut(), &app.theme, label_width)
}

fn render_flow_preview(app: &App, flow: &Flow, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let lines = match &flow.preview {
        Some(preview) => preview_lines(app, preview),
        None => vec![Line::from(Span::styled(" nothing to show", theme.dim()))],
    };
    let max_scroll = lines.len().saturating_sub(area.height as usize);
    frame.render_widget(
        Paragraph::new(lines).scroll((flow.scroll.min(max_scroll) as u16, 0)),
        area,
    );
}

fn preview_lines<'a>(app: &App, preview: &'a Preview) -> Vec<Line<'a>> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let mut lines: Vec<Line> = Vec::new();
    match preview {
        Preview::Create(report) => {
            lines.push(Line::from(vec![
                Span::styled(" ", theme.dim()),
                Span::styled(report.folder_name.clone(), theme.bold()),
            ]));
            for line in crate::tui::widgets::tree::lines(&report.structure, g.rule == "-") {
                lines.push(Line::from(Span::styled(format!(" {line}"), theme.dim())));
            }
            if !report.files.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" Files", theme.accent())));
                for file in &report.files {
                    lines.push(Line::from(Span::styled(
                        format!("   {} {file}", g.sep),
                        theme.dim(),
                    )));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(" Resolved", theme.accent())));
            for value in &report.values {
                lines.push(field_line(
                    theme,
                    &value.slug,
                    if value.value.is_empty() {
                        "(empty)"
                    } else {
                        &value.value
                    },
                ));
            }
            let (from, to) = report.counter;
            lines.push(Line::from(vec![
                Span::styled(format!("   {:<14} ", "{id}"), theme.dim()),
                Span::styled(report.id.clone(), theme.text()),
                Span::styled(format!("   counter {from} {} {to}", g.arrow), theme.dim()),
            ]));
            lines.push(field_line(theme, "{date}", &report.date));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", g.arrow), theme.accent()),
                Span::styled(
                    crate::util::paths::display_path(&report.root_path),
                    theme.text(),
                ),
            ]));
            for preview in &report.previews {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(" {}", preview.path),
                    theme.accent(),
                )));
                for line in &preview.lines {
                    lines.push(Line::from(Span::styled(format!("   {line}"), theme.dim())));
                }
                if preview.hidden > 0 {
                    lines.push(Line::from(Span::styled(
                        format!("   {} {} more lines", g.ellipsis, preview.hidden),
                        theme.dim(),
                    )));
                }
            }
        }
        Preview::Apply(apply) => {
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", g.arrow), theme.accent()),
                Span::styled(
                    crate::util::paths::display_path(&apply.target),
                    theme.text(),
                ),
            ]));
            lines.push(Line::from(""));
            for (create, path) in &apply.rows {
                let (tag, style) = if *create {
                    ("create", theme.good())
                } else {
                    ("skip  ", theme.dim())
                };
                lines.push(Line::from(vec![
                    Span::styled(format!(" {tag} "), style),
                    Span::styled(
                        path.clone(),
                        if *create { theme.text() } else { theme.dim() },
                    ),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(format!(" {} to create", apply.creates), theme.good()),
                Span::styled(format!("   {} already there", apply.skips), theme.dim()),
            ]));
        }
        Preview::Register(register) => {
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", g.arrow), theme.accent()),
                Span::styled(
                    crate::util::paths::display_path(&register.path),
                    theme.text(),
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(field_line(theme, "template", &register.template));
            lines.push(Line::from(vec![
                Span::styled(format!("   {:<14} ", "id"), theme.dim()),
                Span::styled(register.id.clone(), theme.text()),
                Span::styled(format!("   {}", register.id_note), theme.dim()),
            ]));
            lines.push(field_line(theme, "created", &register.created));
            match &register.rename {
                Some((from, to)) => lines.push(Line::from(vec![
                    Span::styled(format!("   {:<14} ", "rename"), theme.dim()),
                    Span::styled(from.clone(), theme.dim()),
                    Span::styled(format!(" {} ", g.arrow), theme.accent()),
                    Span::styled(to.clone(), theme.text()),
                ])),
                None => lines.push(field_line(theme, "rename", "no")),
            }
            if register.apply_structure {
                lines.push(field_line(
                    theme,
                    "fill in",
                    "the template's missing folders",
                ));
            }
            if register.pinfo_exists {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(
                        " {} PROJECT_INFO.md already exists — it will be overwritten",
                        g.warn
                    ),
                    theme.warn(),
                )));
            }
        }
        Preview::FromFolder(scan) => {
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", g.arrow), theme.accent()),
                Span::styled(scan.slug.clone(), theme.bold()),
            ]));
            if !scan.structure.is_empty() {
                lines.push(Line::from(""));
                for line in crate::tui::widgets::tree::lines(&scan.structure, g.rule == "-") {
                    lines.push(Line::from(Span::styled(format!(" {line}"), theme.dim())));
                }
            }
            if !scan.files.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(" Files", theme.accent())));
                for file in &scan.files {
                    lines.push(Line::from(Span::styled(
                        format!("   {} {file}", g.sep),
                        theme.dim(),
                    )));
                }
            }
            if !scan.assets.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " Bundled byte for byte",
                    theme.accent(),
                )));
                for (path, size) in &scan.assets {
                    lines.push(Line::from(vec![
                        Span::styled(format!("   {} {path}", g.sep), theme.dim()),
                        Span::styled(
                            format!("   {}", crate::util::human_bytes::human_bytes(*size)),
                            theme.dim(),
                        ),
                    ]));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        " {} folder{}, {} text file{}",
                        scan.folders,
                        if scan.folders == 1 { "" } else { "s" },
                        scan.files.len(),
                        if scan.files.len() == 1 { "" } else { "s" }
                    ),
                    theme.good(),
                ),
                Span::styled(
                    if scan.bundle {
                        format!(
                            "   {} bundled ({})",
                            scan.assets.len(),
                            crate::util::human_bytes::human_bytes(scan.bundle_bytes)
                        )
                    } else if scan.skipped > 0 {
                        format!("   {} skipped — turn on Bundle assets", scan.skipped)
                    } else {
                        String::new()
                    },
                    if scan.bundle {
                        theme.dim()
                    } else {
                        theme.warn()
                    },
                ),
            ]));
        }
        Preview::Recursive(recursive) => {
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", g.arrow), theme.accent()),
                Span::styled(
                    crate::util::paths::display_path(&recursive.base),
                    theme.text(),
                ),
            ]));
            lines.push(Line::from(""));
            if recursive.rows.is_empty() {
                lines.push(Line::from(Span::styled(
                    " every direct child already has a PROJECT_INFO.md — nothing to register",
                    theme.dim(),
                )));
                return lines;
            }
            for (name, note) in &recursive.rows {
                lines.push(Line::from(vec![
                    Span::styled(" + ", theme.good()),
                    Span::styled(name.clone(), theme.text()),
                    Span::styled(format!("   {note}"), theme.dim()),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    " {} folder{} would be registered",
                    recursive.rows.len(),
                    if recursive.rows.len() == 1 { "" } else { "s" }
                ),
                theme.good(),
            )));
        }
    }
    lines
}

fn field_line<'a>(theme: &crate::tui::theme::Theme, key: &'a str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("   {key:<14} "), theme.dim()),
        Span::styled(value, theme.text()),
    ])
}

fn render_help(app: &App, ctx: command::Context, scroll: usize, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = crate::tui::layout::help_box(area);
    frame.render_widget(Clear, area);
    let block = frame_block(app, format!(" help {} {} ", g.sep, ctx.label()), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The columns are measured from the commands, so a long title cannot
    // run into its description, and a description that does not fit its
    // line continues under itself rather than being cut.
    let width = inner.width as usize;
    let (keys_w, title_w, _) = command::help_columns(ctx, width);
    let mut lines: Vec<Line> = Vec::new();
    for line in command::help_lines(ctx, width) {
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
        " The command palette (c) lists every command with its key; type to filter.",
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
    let area = crate::tui::layout::message_box(area);
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
        .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(theme.text))))
        .collect();
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll.min(max_scroll) as u16, 0));
    frame.render_widget(paragraph, inset(inner));
}
