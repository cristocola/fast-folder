//! The project table and its detail pane — beside it, under it, or in its
//! place (`layout::place`).

use ratatui::Frame;
use ratatui::layout::{Constraint, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, ScrollbarState, Table, TableState};
use unicode_width::UnicodeWidthStr;

use crate::core::library;
use crate::tui::app::pane::{
    EditTarget, Fact, NOTE_INDENT, PHASE_INDENT, PaneEdit, PaneRow, TODO_INDENT, notes_label,
    todos_label,
};
use crate::tui::app::{App, Focus};
use crate::tui::layout;
use crate::tui::rows::{SIZE_CELL, date_cell, size_label};
use crate::tui::view::{fit, highlighted};
use crate::util::size_scan::SizeCell;

/// The widest a template column gets before it is clamped.
const TEMPLATE_MAX: usize = 16;
const BASE_MAX: usize = 14;
/// The date column: `YYYY-MM-DD`.
const DATE_CELL: usize = 10;
/// The widest a tags column gets before it is clamped: three short tags and
/// their `+n`.
const TAGS_MAX: usize = 24;
/// The least a note editor in the pane is drawn at: three lines of text and
/// its key line.
const NOTE_EDITOR_ROWS: u16 = 4;

/// Which optional columns a table shows.
///
/// **The folder name is never cut.** It is the one column that tells projects
/// apart, and a row is eaten from the right — so the optional columns are added
/// only while the widest name still fits whole, in the order a person misses
/// them: the size, the base, the date, the template, the tags. The size comes
/// first because it is the one thing the row knows that the name does not.
///
/// **The base comes second, but only when there is more than one.** With one
/// base the column repeats the same word on every row and the date is worth
/// more; with two it is the only thing on the row that says which drive a
/// project is on, and after `copy-to` two rows can carry the same id and differ
/// in nothing else. Every bundled naming pattern already carries the date
/// inside the folder name, so the date is what gives way.
///
/// **Election stops at the first column that does not fit**, rather than
/// skipping it and trying the next. A narrower later column squeezing in past a
/// wider earlier one produced a 60-column table with a BASE column and no SIZE,
/// which reads as a bug rather than as a priority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Columns {
    pub created: bool,
    pub size: bool,
    pub base: bool,
    pub template: bool,
    pub tags: bool,
}

/// The display width of the tag cell `table` draws: at most three tags, spaced,
/// then `+n` for the rest. Measured rather than guessed, because it is what
/// decides the column's width.
fn tag_cell_width(tags: &[String]) -> usize {
    if tags.is_empty() {
        return 0;
    }
    let shown: usize = tags.iter().take(3).map(|t| t.width()).sum();
    let gaps = tags.len().min(3).saturating_sub(1);
    let extra = match tags.len().saturating_sub(3) {
        0 => 0,
        n => 1 + format!("+{n}").width(),
    };
    shown + gaps + extra
}

/// Whether every row after `todo` up to `row` is a continuation of it — so
/// `row` is part of the todo that starts at `todo`.
fn continues_todo(rows: &[PaneRow], todo: usize, row: usize) -> bool {
    rows.get(todo + 1..=row).is_some_and(|between| {
        between
            .iter()
            .all(|r| matches!(r, PaneRow::TodoLine { .. }))
    })
}

/// One header fact as the pane draws it (`pane::Fact`).
fn fact_spans(
    fact: &Fact,
    app: &App,
    project: &crate::core::library::Project,
    theme: &crate::tui::theme::Theme,
) -> Vec<Span<'static>> {
    match fact {
        Fact::Template => vec![Span::styled(project.template.clone(), theme.dim())],
        Fact::Base => vec![Span::styled(
            library::base_label(&project.base),
            Style::default().fg(theme.accent),
        )],
        Fact::Created => vec![
            Span::styled("created ", theme.dim()),
            Span::styled(date_cell(&project.created).to_string(), theme.text()),
        ],
        Fact::Size => vec![Span::styled(
            match app.size_cell(&project.path) {
                SizeCell::Pending => theme.glyphs.pending.to_string(),
                SizeCell::Known(size) => size_label(size),
            },
            theme.text(),
        )],
        Fact::Notes(notes) => vec![Span::styled(notes_label(*notes), theme.dim())],
        Fact::Todos { done, total } => {
            vec![Span::styled(todos_label(*done, *total), theme.dim())]
        }
    }
}

/// The list's peek at a hidden pane: ` 2 notes · 2/4 todos done `, the parts
/// there are, and `None` when there are none.
fn peek((notes, done, todos): (usize, usize, usize), sep: &str) -> Option<String> {
    let mut parts = Vec::new();
    if notes > 0 {
        parts.push(notes_label(notes));
    }
    if todos > 0 {
        parts.push(todos_label(done, todos));
    }
    (!parts.is_empty()).then(|| format!(" {} ", parts.join(&format!(" {sep} "))))
}

/// `room` is what is left after the cursor cell, the id, their spacing and the
/// right gutter. `many_bases` promotes the base column above the date.
pub fn choose_columns(
    room: usize,
    name_w: usize,
    base_w: usize,
    template_w: usize,
    tags_w: usize,
    many_bases: bool,
) -> Columns {
    let mut columns = Columns::default();
    let Some(mut left) = room.checked_sub(name_w) else {
        return columns;
    };
    let mut done = false;
    let mut take = |width: usize, flag: &mut bool| {
        if done {
            return;
        }
        if left > width {
            left -= width + 1;
            *flag = true;
        } else {
            done = true;
        }
    };
    take(SIZE_CELL, &mut columns.size);
    if many_bases {
        take(base_w, &mut columns.base);
        take(DATE_CELL, &mut columns.created);
    } else {
        take(DATE_CELL, &mut columns.created);
        take(base_w, &mut columns.base);
    }
    take(template_w, &mut columns.template);
    // A measured width, and zero when no row on screen carries a tag — a TAGS
    // header over a column of nothing is a column spent on nothing.
    if tags_w > 0 {
        take(tags_w, &mut columns.tags);
    }
    columns
}

pub fn table(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let focused = app.focus == Focus::Projects && app.modals.is_empty() && !app.search.editing;

    let title = " projects ".to_string();
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, title_style(app, focused)))
        .border_style(border_style(app, focused));
    // Where the pane is out of sight, the list's border says what it would
    // show for the row under the cursor — the one reason to go and look.
    // Nothing for a project with nothing in it.
    if app.pane_behind_list()
        && let Some(peek) = app.pane_counts().and_then(|counts| peek(counts, g.sep))
    {
        block = block.title_bottom(Line::from(Span::styled(peek, theme.dim())).right_aligned());
    }
    let full_inner = block.inner(area);
    frame.render_widget(block, area);
    if full_inner.height < 2 || full_inner.width < 4 {
        return;
    }
    // **One column of gutter, always.** The last cell is right-aligned, so
    // without it a size sits against the border glyph and reads as cut off —
    // and the scrollbar, which is drawn over the border column, lands directly
    // on the digits. It is reserved whether or not a scrollbar is showing:
    // taking it back when the list gets short would reflow every width as rows
    // arrive, which is the one thing the measured columns exist to prevent.
    let inner = Rect::new(
        full_inner.x,
        full_inner.y,
        full_inner.width - 1,
        full_inner.height,
    );

    // Widths measured from every row shown, never from the sizes, so a landing
    // snapshot cannot reflow the table.
    let mut id_w = 4usize;
    let mut name_w = 8usize;
    let mut base_w = 4usize;
    let mut template_w = 8usize;
    let mut tags_w = 0usize;
    // Whether the base column is worth promoting is a question about the rows
    // on screen, not about the configuration: two bases with one unmounted
    // shows one base's projects, and a column repeating one word earns nothing.
    let mut first_base: Option<&std::path::Path> = None;
    let mut many_bases = false;
    for row in 0..app.library.len() {
        if let Some(p) = app.library.row(row) {
            id_w = id_w.max(p.id.width());
            name_w = name_w.max(p.name.width());
            base_w = base_w.max(library::base_label(&p.base).width());
            template_w = template_w.max(p.template.width());
            tags_w = tags_w.max(tag_cell_width(&p.tags));
            match first_base {
                None => first_base = Some(p.base.as_path()),
                Some(seen) if seen != p.base.as_path() => many_bases = true,
                Some(_) => {}
            }
        }
    }
    let base_w = base_w.min(BASE_MAX);
    let template_w = template_w.min(TEMPLATE_MAX);
    let tags_w = tags_w.min(TAGS_MAX);
    // The cursor cell and the id, each followed by a space.
    let room = (inner.width as usize).saturating_sub(2 + id_w + 1);
    let columns = choose_columns(room, name_w, base_w, template_w, tags_w, many_bases);
    // What the name has once the elected columns are placed. A name wider than
    // that — a window narrower than the name itself — is cut with the
    // ellipsis, never by the table's border in the middle of a letter.
    let name_col = room.saturating_sub(
        [
            (columns.size, SIZE_CELL),
            (columns.created, DATE_CELL),
            (columns.base, base_w),
            (columns.template, template_w),
            (columns.tags, tags_w),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, w)| w + 1)
        .sum::<usize>(),
    );

    let mut header = vec![Cell::from(""), Cell::from("ID"), Cell::from("PROJECT")];
    let mut constraints = vec![
        Constraint::Length(1),
        Constraint::Length(id_w as u16),
        Constraint::Fill(2),
    ];
    if columns.size {
        // Right-aligned over right-aligned figures: a left-aligned SIZE header
        // sat seven columns away from every number under it.
        header.push(Cell::from(format!("{:>width$}", "SIZE", width = SIZE_CELL)));
        constraints.push(Constraint::Length(SIZE_CELL as u16));
    }
    if columns.created {
        header.push(Cell::from("CREATED"));
        constraints.push(Constraint::Length(DATE_CELL as u16));
    }
    if columns.base {
        header.push(Cell::from("BASE"));
        constraints.push(Constraint::Length(base_w as u16));
    }
    if columns.template {
        header.push(Cell::from("TEMPLATE"));
        constraints.push(Constraint::Length(template_w as u16));
    }
    if columns.tags {
        header.push(Cell::from("TAGS"));
        // Measured, like every other column, rather than whatever `Fill` leaves
        // over: sharing the slack with the name meant one column of gutter cut
        // the first tag's last letter, and a tag cut mid-word says the wrong
        // tag. The name keeps all the slack, which is the column that needs it.
        constraints.push(Constraint::Length(tags_w as u16));
    }

    let rows_visible = inner.height.saturating_sub(1) as usize;
    // An empty table says so inside the box, where the eye is, not only on
    // the status line.
    if app.library.loaded && app.library.is_empty() && inner.height > 3 {
        let sentence = if app.library.snapshot.is_empty() {
            "nothing here yet — n creates a project, e registers a folder"
        } else {
            "nothing matches"
        };
        // Centred in the box, so the gutter the table reserves does not push
        // the one sentence in an empty list half a column off.
        let line = Rect::new(full_inner.x, full_inner.y + 2, full_inner.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                fit(sentence, full_inner.width as usize, g.ellipsis),
                theme.dim(),
            ))
            .alignment(ratatui::layout::Alignment::Center),
            line,
        );
    }
    let offset = app.library.offset;

    let end = (offset + rows_visible).min(app.library.len());
    let rows: Vec<Row> = (offset..end)
        .filter_map(|row| app.library.row(row).map(|p| (row, p)))
        .map(|(row, p)| {
            let info = app.library.match_info(row);
            let marked = app.library.marks.contains(&p.path);
            let selected = app.library.selected == Some(row);
            // One cell says both things: the cursor, or a mark. A marked row
            // that is also selected shows the mark — the highlight says the rest.
            let (glyph, style) = if marked {
                (g.mark, theme.warn())
            } else if selected {
                (g.cursor, theme.accent())
            } else {
                (" ", theme.dim())
            };
            let mut cells = vec![
                Cell::from(Span::styled(glyph, style)),
                Cell::from(Line::from(highlighted(
                    &p.id,
                    info.map(|i| i.id_hits.as_slice()).unwrap_or(&[]),
                    theme.accent(),
                    theme.hit(),
                ))),
                Cell::from(Line::from(crate::tui::view::fit_spans(
                    highlighted(
                        &p.name,
                        info.map(|i| i.name_hits.as_slice()).unwrap_or(&[]),
                        theme.text(),
                        theme.hit(),
                    ),
                    name_col,
                    g.ellipsis,
                ))),
            ];
            if columns.size {
                let (label, style) = match app.size_cell(&p.path) {
                    SizeCell::Pending => (g.pending.to_string(), theme.dim()),
                    SizeCell::Known(size) => (size_label(size), theme.text()),
                };
                cells.push(Cell::from(Line::from(Span::styled(
                    format!("{label:>width$}", width = SIZE_CELL),
                    style,
                ))));
            }
            if columns.created {
                cells.push(Cell::from(Span::styled(
                    date_cell(&p.created).to_string(),
                    theme.dim(),
                )));
            }
            if columns.base {
                cells.push(Cell::from(Span::styled(
                    fit(&library::base_label(&p.base), base_w, g.ellipsis),
                    Style::default().fg(theme.accent),
                )));
            }
            if columns.template {
                cells.push(Cell::from(Span::styled(
                    fit(&p.template, template_w, g.ellipsis),
                    theme.dim(),
                )));
            }
            if columns.tags {
                let mut spans = Vec::new();
                for (i, tag) in p.tags.iter().take(3).enumerate() {
                    if i > 0 {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(Span::styled(
                        tag.clone(),
                        Style::default().fg(theme.tag_color(tag)),
                    ));
                }
                if p.tags.len() > 3 {
                    spans.push(Span::styled(format!(" +{}", p.tags.len() - 3), theme.dim()));
                }
                cells.push(Cell::from(Line::from(spans)));
            }
            // A row a verb has just changed wears the second accent for
            // half a second. It is a `Row` style rather than a per-cell one so
            // the whole row lights, and it sits **under** the selection
            // highlight, which ratatui draws over it — the cursor still says
            // where you are while the pulse says what moved.
            match app
                .pulses
                .style_for(&p.path, app.elapsed_ms, theme, app.motion)
            {
                Some(style) => Row::new(cells).style(style),
                None => Row::new(cells),
            }
        })
        .collect();

    let table = Table::new(rows, constraints)
        .header(Row::new(header).style(theme.dim().add_modifier(ratatui::style::Modifier::BOLD)))
        .column_spacing(1)
        .row_highlight_style(theme.selection)
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Never);
    let mut state =
        TableState::default().with_selected(app.library.selected.map(|s| s.saturating_sub(offset)));
    frame.render_stateful_widget(table, inner, &mut state);

    if app.library.len() > rows_visible {
        let mut scroll = ScrollbarState::new(app.library.len().saturating_sub(rows_visible))
            .position(offset)
            .viewport_content_length(rows_visible);
        let scrollbar = crate::tui::view::scrollbar(&g);
        let bar_area = Rect::new(
            area.x + area.width - 1,
            inner.y + 1,
            1,
            inner.height.saturating_sub(1),
        );
        frame.render_stateful_widget(scrollbar, bar_area, &mut scroll);
    }
}

/// A pane's title: accent while it has the focus, dim otherwise — easing
/// between the two for the moment after the focus moved, so the move is seen
/// where it landed and not only inferred from a border.
pub(crate) fn title_style(app: &App, focused: bool) -> ratatui::style::Style {
    crate::tui::motion::title_style(
        &app.theme,
        focused,
        app.focus_moved_at,
        app.elapsed_ms,
        app.motion,
    )
}

/// A pane's border: the focus colour while it has the focus, the plain one
/// otherwise, easing between the two as the title does.
pub(crate) fn border_style(app: &App, focused: bool) -> ratatui::style::Style {
    crate::tui::motion::border_style(
        &app.theme,
        focused,
        app.focus_moved_at,
        app.elapsed_ms,
        app.motion,
    )
}

/// The detail pane. Returns where the caret is while a row is being edited,
/// so the terminal cursor sits in the field rather than on the search bar.
pub fn detail(app: &App, frame: &mut Frame, area: Rect) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let focused = app.focus == Focus::Detail && app.modals.is_empty() && !app.search.editing;

    let Some(project) = app.library.selected() else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" detail ", title_style(app, focused)))
            .border_style(border_style(app, focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Span::styled("nothing selected", theme.dim())),
            inner,
        );
        return None;
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {} ", project.id),
            title_style(app, focused),
        ))
        .border_style(border_style(app, focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // The text sits a column in from each border (`layout::pane_text`, the
    // rect `pane_rows` wrapped to); a highlight still spans the whole row.
    let text = layout::pane_text(area);
    let width = text.width as usize;

    let rule = |label: &str| -> Line<'static> {
        Line::from(Span::styled(
            format!(
                "{} {label} {}",
                g.rule.repeat(2),
                g.rule.repeat(width.saturating_sub(label.width() + 4))
            ),
            theme.dim(),
        ))
    };

    let rows = app.pane_rows();
    // The row of a todo open in the editor, whose own rows are blank while
    // the editor holds its text.
    let editing_todo = match &app.pane_edit {
        Some(PaneEdit::Line {
            target: EditTarget::Todo { .. },
            row,
            ..
        }) => Some(*row),
        _ => None,
    };
    let key_w = rows
        .iter()
        .filter_map(|row| match row {
            PaneRow::Variable { label, .. } => Some(label.width()),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .min(18);

    // One line per row, and no wrapping: the cursor is an index into the
    // rows, and `detail_scroll` counts rows, so a row that took two lines
    // would put both off by one from there down.
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let mut line = match row {
            PaneRow::Name(first) => Line::from(Span::styled(first.clone(), theme.bold())),
            PaneRow::NameLine(rest) => Line::from(Span::styled(rest.clone(), theme.bold())),
            // Whole facts, as `pane_rows` flowed them: what the project is
            // recedes but for its base, which is the one colour a list of
            // drives needs; what it holds reads in the text colour where it is
            // a figure.
            PaneRow::Facts(facts) => {
                let mut spans = Vec::with_capacity(facts.len() * 2);
                for (at, fact) in facts.iter().enumerate() {
                    if at > 0 {
                        spans.push(Span::styled(format!("   {}   ", g.sep), theme.dim()));
                    }
                    spans.extend(fact_spans(fact, app, project, theme));
                }
                Line::from(spans)
            }
            PaneRow::Rule(section) => rule(section.label()),
            PaneRow::Tag(tag) => Line::from(Span::styled(
                format!("{} {tag}", g.dot),
                Style::default().fg(theme.tag_color(tag)),
            )),
            PaneRow::AddTag => {
                Line::from(Span::styled(format!("{} add a tag", g.sep), theme.dim()))
            }
            PaneRow::Reading => {
                Line::from(Span::styled(format!("reading{}", g.ellipsis), theme.dim()))
            }
            PaneRow::Warning(error) => {
                Line::from(Span::styled(format!("warning: {error}"), theme.warn()))
            }
            PaneRow::Variable { label, value, .. } => {
                let shown = if value.is_empty() {
                    "(empty)"
                } else {
                    value.as_str()
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:<key_w$} ", fit(label, key_w, g.ellipsis)),
                        theme.dim(),
                    ),
                    Span::styled(
                        fit(shown, width.saturating_sub(key_w + 1), g.ellipsis),
                        theme.text(),
                    ),
                ])
            }
            PaneRow::Entry(entry) => {
                let shown = if entry.is_dir {
                    format!("{}/", entry.name)
                } else {
                    entry.name.clone()
                };
                Line::from(vec![
                    Span::styled(
                        format!("{} ", if entry.is_dir { g.folder } else { g.sep }),
                        theme.dim(),
                    ),
                    Span::styled(
                        fit(&shown, width.saturating_sub(2), g.ellipsis),
                        if entry.is_dir {
                            theme.text()
                        } else {
                            theme.dim()
                        },
                    ),
                ])
            }
            PaneRow::More(more) => Line::from(Span::styled(
                format!("  {} {more} more", g.ellipsis),
                theme.dim(),
            )),
            PaneRow::EarlierNotes(earlier) => Line::from(Span::styled(
                format!("  {} {earlier} earlier", g.ellipsis),
                theme.dim(),
            )),
            // A note's day in a column ten wide — blank for the undated
            // note — and its first row as `pane_rows` wrapped it; its other
            // rows under the text, in the text column, so a note reads as one
            // block.
            PaneRow::Note { date, first, .. } => Line::from(vec![
                Span::styled(
                    format!("{:<10} ", date.as_deref().unwrap_or("")),
                    theme.dim(),
                ),
                Span::styled(
                    fit(first, width.saturating_sub(NOTE_INDENT), g.ellipsis),
                    theme.text(),
                ),
            ]),
            PaneRow::NoteLine(line) => Line::from(vec![
                Span::raw(format!("{:<10} ", "")),
                Span::styled(
                    fit(line, width.saturating_sub(NOTE_INDENT), g.ellipsis),
                    theme.text(),
                ),
            ]),
            PaneRow::NoteMore(more) => Line::from(Span::styled(
                format!(
                    "{:<10} {} {more} more line{}",
                    "",
                    g.ellipsis,
                    if *more == 1 { "" } else { "s" }
                ),
                theme.dim(),
            )),
            PaneRow::AddNote => {
                Line::from(Span::styled(format!("{} add a note", g.sep), theme.dim()))
            }
            // A phase is a heading of the list, in the text colour, with
            // how much of it is done on the right; a finished phase recedes
            // with its tasks. Its tasks sit two columns in from it.
            PaneRow::Phase { name, done, total } => {
                let finished = done == total;
                let count = format!("{done}/{total}");
                let room = width.saturating_sub(count.width() + 1);
                let label = fit(name, room, g.ellipsis);
                let gap = width.saturating_sub(label.width() + count.width());
                Line::from(vec![
                    Span::styled(label, if finished { theme.dim() } else { theme.text() }),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(count, theme.dim()),
                ])
            }
            // The markdown's own marker, which every terminal can draw. An
            // open todo reads in the text colour — the accent is for focus,
            // and a list of accented boxes is a list shouting — and a done
            // one recedes with its text.
            PaneRow::Todo {
                done, text, phased, ..
            } => {
                if editing_todo == Some(index) {
                    Line::default()
                } else {
                    let style = if *done { theme.dim() } else { theme.text() };
                    let indent = if *phased { PHASE_INDENT } else { 0 };
                    Line::from(vec![
                        Span::raw(" ".repeat(indent)),
                        Span::styled(if *done { "[x] " } else { "[ ] " }, theme.dim()),
                        Span::styled(
                            fit(text, width.saturating_sub(TODO_INDENT + indent), g.ellipsis),
                            style,
                        ),
                    ])
                }
            }
            // The rest of a long todo, under its text; blank while the todo
            // is open in the editor, which holds the whole of it on one line.
            PaneRow::TodoLine { done, text, phased } => {
                if editing_todo.is_some_and(|row| row < index && continues_todo(&rows, row, index))
                {
                    Line::default()
                } else {
                    let indent = TODO_INDENT + if *phased { PHASE_INDENT } else { 0 };
                    Line::from(vec![
                        Span::raw(" ".repeat(indent)),
                        Span::styled(
                            fit(text, width.saturating_sub(indent), g.ellipsis),
                            if *done { theme.dim() } else { theme.text() },
                        ),
                    ])
                }
            }
            // The add line: its field is drawn over it below.
            PaneRow::Adding => Line::default(),
            PaneRow::AddTodo => {
                Line::from(Span::styled(format!("{} add a todo", g.sep), theme.dim()))
            }
        };
        // A row an edit just landed on wears the wash, under the cursor —
        // the cursor still says where you are while the pulse says what
        // changed, the same order the table keeps.
        let wash = app
            .pane_pulses
            .style_for(&index, app.elapsed_ms, theme, app.motion);
        if let Some(wash) = wash {
            line = line.style(wash);
        }
        // The cursor: the selection's own highlight, and only while the pane
        // has the focus — a lit row in a pane you are not in would say the
        // next key goes there when it does not.
        let lit = focused && index == app.pane_cursor && row.selectable();
        if lit {
            line = line.patch_style(theme.selection);
        }
        lines.push(line);
        // Both reach the borders, across the padding, so the row reads as
        // one bar rather than as a highlighted line of text inside a pane.
        if let Some(y) = index
            .checked_sub(app.detail_scroll)
            .filter(|row| *row < text.height as usize)
        {
            let bar = Rect::new(inner.x, text.y + y as u16, inner.width, 1);
            if let Some(wash) = wash {
                frame.buffer_mut().set_style(bar, wash);
            }
            if lit {
                frame.buffer_mut().set_style(bar, theme.selection);
            }
        }
    }

    let total = lines.len();
    let paragraph = Paragraph::new(lines).scroll((app.detail_scroll as u16, 0));
    frame.render_widget(paragraph, text);

    // More rows than the pane shows: the table's scrollbar, on the border.
    let shown = text.height as usize;
    if total > shown && inner.height > 0 {
        let mut scroll = ScrollbarState::new(total.saturating_sub(shown))
            .position(app.detail_scroll)
            .viewport_content_length(shown);
        frame.render_stateful_widget(
            crate::tui::view::scrollbar(&g),
            Rect::new(area.x + area.width - 1, inner.y, 1, inner.height),
            &mut scroll,
        );
    }

    // An open edit is drawn over its row, in place: the field where the value
    // was, the refusal on the line under it. A note takes the rest of the
    // pane from its own row down, with its own key line at the bottom — a
    // text area is the one widget whose Enter is not the registry's.
    let edit = app.pane_edit.as_ref()?;
    let row_y = (edit.row().checked_sub(app.detail_scroll)? as u16).checked_add(text.y)?;
    if row_y >= text.y + text.height {
        return None;
    }
    match edit {
        PaneEdit::Line {
            input,
            error,
            target,
            ..
        } => {
            let prefix = match target {
                EditTarget::Variable(slug) => {
                    let label = rows
                        .iter()
                        .find_map(|row| match row {
                            PaneRow::Variable { slug: s, label, .. } if s == slug => Some(label),
                            _ => None,
                        })
                        .cloned()
                        .unwrap_or_else(|| slug.clone());
                    format!("{:<key_w$} ", fit(&label, key_w, g.ellipsis))
                }
                EditTarget::Tag(_) => format!("{} ", g.dot),
                // A todo is edited behind its own box, where it sits.
                EditTarget::Todo { .. } => match rows.get(edit.row()) {
                    Some(PaneRow::Todo { done, phased, .. }) => format!(
                        "{}{}",
                        " ".repeat(if *phased { PHASE_INDENT } else { 0 }),
                        if *done { "[x] " } else { "[ ] " }
                    ),
                    _ => "[ ] ".to_string(),
                },
                EditTarget::NewTodo { phase, .. } => format!(
                    "{}[ ] ",
                    " ".repeat(if phase.is_some() { PHASE_INDENT } else { 0 })
                ),
            };
            let line_area = Rect::new(text.x, row_y, text.width, 1);
            frame.render_widget(Paragraph::new(""), line_area);
            let caret = input.render_line(
                line_area,
                frame.buffer_mut(),
                Span::styled(prefix, theme.dim()),
                theme.text().patch(theme.selection),
            );
            if let Some(error) = error
                && row_y + 1 < text.y + text.height
            {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        fit(error, text.width as usize, g.ellipsis),
                        theme.warn(),
                    )),
                    Rect::new(text.x, row_y + 1, text.width, 1),
                );
            }
            caret
        }
        PaneEdit::Note {
            area: editor,
            error,
            ..
        } => {
            // From the note's own row to the key line at the pane's bottom —
            // and never fewer than `NOTE_EDITOR_ROWS`: a note near the bottom
            // slides the editor up over the rows above it, rather than
            // opening as a sliver, or as nothing at all.
            let body = layout::box_at_row(
                text,
                row_y - text.y,
                text.y + text.height - row_y,
                NOTE_EDITOR_ROWS,
            );
            if body.height < 2 {
                return None;
            }
            let text_area = Rect::new(body.x, body.y, body.width, body.height - 1);
            frame.render_widget(ratatui::widgets::Clear, text_area);
            let caret = editor.render(text_area, frame.buffer_mut(), theme.text());
            let keys = Rect::new(body.x, body.y + body.height - 1, body.width, 1);
            frame.render_widget(ratatui::widgets::Clear, keys);
            match error {
                Some(error) => frame.render_widget(
                    Paragraph::new(Span::styled(
                        fit(error, body.width as usize, g.ellipsis),
                        theme.warn(),
                    )),
                    keys,
                ),
                None => {
                    let pairs: Vec<(String, String)> = crate::tui::command::hints(
                        crate::tui::command::Context::PaneEdit,
                        app,
                        body.width as usize,
                    )
                    .into_iter()
                    .map(|(key, what)| (key, what.to_string()))
                    .collect();
                    frame.render_widget(
                        Paragraph::new(crate::tui::view::builder::key_line(
                            theme,
                            &pairs,
                            body.width as usize,
                        )),
                        keys,
                    );
                }
            }
            caret
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Columns, choose_columns, tag_cell_width};

    #[test]
    fn columns_are_added_only_while_the_name_still_fits() {
        // 80 columns: 78 inside the borders, minus the gutter, the cursor cell,
        // a six-char id and their spaces. A forty-char name keeps the size and
        // the date.
        let columns = choose_columns(78 - 1 - 2 - 7, 40, 8, 11, 12, false);
        assert_eq!(
            columns,
            Columns {
                created: true,
                size: true,
                base: false,
                template: false,
                tags: false
            }
        );
        // Beside the detail pane there is room for one more cell: the size,
        // not the date — the name carries the date already.
        let beside_pane = choose_columns(70 - 1 - 2 - 7, 40, 8, 11, 12, false);
        assert!(beside_pane.size && !beside_pane.created, "{beside_pane:?}");
        // No room for anything but the name.
        assert_eq!(choose_columns(20, 40, 8, 11, 12, false), Columns::default());
        // Wide: everything.
        assert!(choose_columns(140, 40, 8, 11, 12, false).tags);
    }

    #[test]
    fn a_second_base_takes_the_date_s_place() {
        // The same room that held the size and the date holds the size and the
        // base instead once the rows come from two bases: the folder names
        // carry the date, and nothing on the row says which drive it is on.
        let one = choose_columns(78 - 1 - 2 - 7, 40, 8, 11, 12, false);
        let two = choose_columns(78 - 1 - 2 - 7, 40, 8, 11, 12, true);
        assert!(one.created && !one.base, "one base: {one:?}");
        assert!(two.base && !two.created, "two bases: {two:?}");
        assert!(one.size && two.size, "the size is first either way");
    }

    #[test]
    fn election_stops_at_the_first_column_that_does_not_fit() {
        // Room for the size and nothing more. The base is narrower than the
        // date, and the greedy version let it slip in behind a date that had
        // just been refused — a table with a BASE column and no SIZE.
        let columns = choose_columns(40 + super::SIZE_CELL + 1 + 6, 40, 4, 11, 12, false);
        assert!(columns.size, "{columns:?}");
        assert!(
            !columns.created && !columns.base && !columns.template && !columns.tags,
            "nothing may be elected past the first refusal: {columns:?}"
        );
    }

    #[test]
    fn a_tags_column_is_measured_and_absent_when_no_row_has_one() {
        // A library with no tags spends no column on a TAGS header.
        let untagged = choose_columns(140, 40, 8, 11, 0, false);
        assert!(!untagged.tags, "{untagged:?}");
        assert!(untagged.template, "the columns before it still fit");

        // Three tags, two gaps.
        assert_eq!(tag_cell_width(&[]), 0);
        assert_eq!(tag_cell_width(&["draft".into()]), 5);
        assert_eq!(tag_cell_width(&["a".into(), "bb".into()]), 4);
        // Past three, the rest become `+n` after a space.
        let many: Vec<String> = ["aa", "bb", "cc", "dd", "ee"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(tag_cell_width(&many), 2 + 1 + 2 + 1 + 2 + 1 + 2);
    }
}
