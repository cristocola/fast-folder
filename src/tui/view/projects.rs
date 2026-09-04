//! The project table and the detail pane beside it.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState, Wrap,
};
use unicode_width::UnicodeWidthStr;

use crate::core::library;
use crate::tui::app::{App, Focus};
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
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            if focused { theme.accent() } else { theme.dim() },
        ))
        .border_style(theme.border(focused));
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
                Cell::from(Line::from(highlighted(
                    &p.name,
                    info.map(|i| i.name_hits.as_slice()).unwrap_or(&[]),
                    theme.text(),
                    theme.hit(),
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
            Row::new(cells)
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
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        let bar_area = Rect::new(
            area.x + area.width - 1,
            inner.y + 1,
            1,
            inner.height.saturating_sub(1),
        );
        frame.render_stateful_widget(scrollbar, bar_area, &mut scroll);
    }
}

pub fn detail(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let focused = app.focus == Focus::Detail && app.modals.is_empty() && !app.search.editing;

    let Some(project) = app.library.selected() else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" detail ", theme.dim()))
            .border_style(theme.border(focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Span::styled("nothing selected", theme.dim())),
            inner,
        );
        return;
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {} ", project.id),
            if focused { theme.accent() } else { theme.dim() },
        ))
        .border_style(theme.border(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width as usize;

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

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(project.name.clone(), theme.bold())),
        Line::from(vec![
            Span::styled(project.template.clone(), theme.dim()),
            Span::styled(format!(" {} ", g.sep), theme.dim()),
            Span::styled(
                library::base_label(&project.base),
                Style::default().fg(theme.accent),
            ),
            Span::styled(format!(" {} created ", g.sep), theme.dim()),
            Span::styled(date_cell(&project.created).to_string(), theme.text()),
        ]),
    ];

    let detail = app.details.get(&project.path);
    let size = match app.size_cell(&project.path) {
        SizeCell::Pending => g.pending.to_string(),
        SizeCell::Known(size) => size_label(size),
    };
    let journal = detail.map(|d| d.journal_count).unwrap_or(0);
    lines.push(Line::from(vec![
        Span::styled(size, theme.text()),
        Span::styled(
            format!(
                "   {}   {journal} journal entr{}",
                g.sep,
                if journal == 1 { "y" } else { "ies" }
            ),
            theme.dim(),
        ),
    ]));

    if !project.tags.is_empty() {
        let mut spans = vec![Span::styled("tags  ", theme.dim())];
        for tag in &project.tags {
            spans.push(Span::styled(
                format!("{} {tag}  ", g.dot),
                Style::default().fg(theme.tag_color(tag)),
            ));
        }
        lines.push(Line::from(spans));
    }

    match detail {
        None => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("reading…", theme.dim())));
        }
        Some(detail) => {
            if let Some(error) = &detail.error {
                lines.push(Line::from(Span::styled(
                    format!("warning: {error}"),
                    theme.warn(),
                )));
            }
            if let Some(meta) = &detail.meta
                && !meta.variables.is_empty()
            {
                lines.push(rule("variables"));
                let key_w = meta
                    .variables
                    .keys()
                    .map(|k| k.width())
                    .max()
                    .unwrap_or(0)
                    .min(18);
                for (key, value) in meta.variables.iter().take(8) {
                    let shown = if value.is_empty() {
                        "(empty)"
                    } else {
                        value.as_str()
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{:<key_w$} ", fit(key, key_w, g.ellipsis)),
                            theme.dim(),
                        ),
                        Span::styled(
                            fit(shown, width.saturating_sub(key_w + 1), g.ellipsis),
                            theme.text(),
                        ),
                    ]));
                }
            }
            if !detail.listing.is_empty() {
                lines.push(rule("inside"));
                for entry in detail.listing.iter().take(8) {
                    let shown = if entry.is_dir {
                        format!("{}/", entry.name)
                    } else {
                        entry.name.clone()
                    };
                    lines.push(Line::from(vec![
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
                    ]));
                }
                if detail.listing.len() > 8 {
                    lines.push(Line::from(Span::styled(
                        format!("  {} {} more", g.ellipsis, detail.listing.len() - 8),
                        theme.dim(),
                    )));
                }
            }
            if !detail.notes.is_empty() {
                lines.push(rule("notes"));
                for note in &detail.notes {
                    lines.push(Line::from(Span::styled(note.clone(), theme.text())));
                }
            }
            if !detail.journal.is_empty() {
                lines.push(rule("journal"));
                for (date, message) in &detail.journal {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{date} "), theme.dim()),
                        Span::styled(message.clone(), theme.text()),
                    ]));
                }
            }
        }
    }

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll as u16, 0));
    frame.render_widget(paragraph, inner);
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
