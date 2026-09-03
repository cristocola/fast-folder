//! The template studio and the builder inside it.
//!
//! Both are one dialog with several faces, drawn at one size so entering a
//! section and coming back does not move the box under the reader — the same
//! bargain `view::modals::render_flow` makes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::tui::app::App;
use crate::tui::app::settings::{Editing, SettingsState};
use crate::tui::app::studio::{Builder, FileEdit, FileList, Open, Row, Section, Studio, VarList};
use crate::tui::layout::centered;
use crate::tui::theme::Theme;
use crate::tui::view::{fit, pad};
use crate::tui::widgets::form::Form;

/// How wide the labels down the left of every list here are.
const LABEL: usize = 18;

/// A box sized to what it holds — never taller than most of the screen, never
/// so short that the footer and the key line crowd the content.
fn sized(area: Rect, body: u16) -> Rect {
    let full = centered(area, 84, 96);
    let height = (body + 4).clamp(8.min(full.height), full.height);
    Rect::new(
        full.x,
        full.y + (full.height - height) / 2,
        full.width,
        height,
    )
}

/// How many rows the builder's open face wants.
fn body_height(builder: &Builder) -> u16 {
    match &builder.open {
        None => Row::ALL.len() as u16,
        Some(Open::Metadata(form)) | Some(Open::Id(form)) => form.rows() as u16,
        Some(Open::Variables(list)) => match &list.editing {
            Some((_, form)) => form.rows() as u16,
            None => builder.template.variables.len().max(1) as u16,
        },
        // A document, with room to grow into: never so tight that adding a
        // line moves the box, never a cavern around three folders.
        Some(Open::Structure(area)) => (area.lines().len() + 3).clamp(8, 24) as u16,
        Some(Open::Files(list)) => match &list.editing {
            Some(_) => 14,
            None => builder.template.files.len().max(1) as u16,
        },
    }
}

fn block<'a>(app: &App, title: String) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, app.theme.accent()))
        .border_style(app.theme.border(true))
}

/// The dialog's chrome: a bordered box with a footer line and a key line, and
/// the body between them. Returns the body and the two lines.
fn frame_parts(
    app: &App,
    title: String,
    frame: &mut Frame,
    area: Rect,
) -> Option<(Rect, Rect, Rect)> {
    frame.render_widget(Clear, area);
    let outer = block(app, title);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if inner.height < 4 {
        return None;
    }
    let body = Rect::new(inner.x, inner.y, inner.width, inner.height - 2);
    let footer = Rect::new(inner.x, inner.y + body.height, inner.width, 1);
    let keys = Rect::new(inner.x, inner.y + body.height + 1, inner.width, 1);
    Some((body, footer, keys))
}

fn footer_line(frame: &mut Frame, area: Rect, text: &str, style: ratatui::style::Style) {
    frame.render_widget(Paragraph::new(Span::styled(text.to_string(), style)), area);
}

fn key_line<'a>(theme: &Theme, pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = Vec::new();
    for (key, what) in pairs {
        spans.push(Span::styled(format!(" {key} "), theme.key()));
        spans.push(Span::styled(format!("{what}  "), theme.dim()));
    }
    Line::from(spans)
}

// ---------------------------------------------------------------------------
// The studio
// ---------------------------------------------------------------------------

pub fn render_studio(
    app: &App,
    studio: &Studio,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let body = studio.cards.len().max(studio.lines.len()).max(4) as u16;
    let area = sized(area, body);
    let (body, footer, keys) = frame_parts(app, " templates ".to_string(), frame, area)?;

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(body);

    let items: Vec<ListItem> = studio
        .cards
        .iter()
        .map(|card| {
            let count = app.templates.count(&card.slug);
            ListItem::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(pad(&card.slug, 20), theme.text()),
                Span::styled(format!("{count:>4}"), theme.dim()),
            ]))
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " no templates yet — n makes one, g reads one out of a folder",
                theme.dim(),
            )),
            panes[0],
        );
    } else {
        let list = List::new(items).highlight_style(theme.selection);
        let mut state = ListState::default()
            .with_offset(studio.offset)
            .with_selected(Some(studio.selected));
        frame.render_stateful_widget(list, panes[0], &mut state);
    }

    let detail: Vec<Line> = studio
        .lines
        .iter()
        .map(|line| Line::from(Span::styled(format!(" {line}"), theme.text())))
        .collect();
    let detail = if detail.is_empty() {
        vec![Line::from(Span::styled(" reading…", theme.dim()))]
    } else {
        detail
    };
    let max_scroll = detail.len().saturating_sub(panes[1].height as usize);
    frame.render_widget(
        Paragraph::new(detail).scroll((studio.scroll.min(max_scroll) as u16, 0)),
        panes[1],
    );

    let note = match studio.cards.len() {
        0 => String::new(),
        n => format!(" {n} template{}", if n == 1 { "" } else { "s" }),
    };
    footer_line(frame, footer, &note, theme.dim());
    frame.render_widget(
        Paragraph::new(key_line(
            theme,
            &[
                ("↑↓", "choose"),
                ("Enter", "edit"),
                ("n", "new"),
                ("g", "from a folder"),
                ("D", "delete"),
                ("Esc", "close"),
            ],
        )),
        keys,
    );
    let _ = g;
    None
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

pub fn render_builder(
    app: &App,
    builder: &Builder,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let area = sized(area, body_height(builder));
    let title = match &builder.open {
        None => format!(" {} ", builder.title()),
        Some(open) => format!(
            " {} {} {} ",
            builder.title(),
            theme.glyphs.sep,
            section_of(open)
        ),
    };
    let (body, footer, keys) = frame_parts(app, title, frame, area)?;

    if builder.pending {
        footer_line(frame, footer, " reading the template…", theme.dim());
        return None;
    }

    let (caret, hint, key_pairs) = match &builder.open {
        None => (
            render_sections(app, builder, frame, body),
            builder.error.clone(),
            vec![
                ("↑↓", "choose"),
                ("Enter", "open / save"),
                ("Esc", "discard"),
            ],
        ),
        Some(Open::Metadata(form)) | Some(Open::Id(form)) => (
            render_form(app, form, frame, body),
            form.error()
                .map(str::to_string)
                .or_else(|| form.focused().map(|field| field.hint.clone())),
            vec![("Tab", "next field"), ("Enter", "keep"), ("Esc", "back")],
        ),
        Some(Open::Variables(list)) => render_variables(app, builder, list, frame, body),
        Some(Open::Structure(area_state)) => {
            let caret = render_structure(app, builder, area_state, frame, body);
            (
                caret,
                Some("one folder path per line — use / to nest on every platform".to_string()),
                vec![
                    ("Ctrl-S", "keep"),
                    ("Enter", "new line"),
                    ("Ctrl-K", "drop the line"),
                    ("Esc", "back"),
                ],
            )
        }
        Some(Open::Files(list)) => render_files(app, builder, list, frame, body),
    };

    let style = if builder.error.is_some() {
        theme.warn()
    } else {
        theme.dim()
    };
    let text = hint.unwrap_or_default();
    footer_line(
        frame,
        footer,
        &format!(
            " {}",
            fit(&text, footer.width as usize, theme.glyphs.ellipsis)
        ),
        style,
    );
    frame.render_widget(Paragraph::new(key_line(theme, &key_pairs)), keys);
    caret
}

fn section_of(open: &Open) -> &'static str {
    match open {
        Open::Metadata(_) => Section::Metadata.label(),
        Open::Id(_) => Section::Id.label(),
        Open::Variables(_) => Section::Variables.label(),
        Open::Structure(_) => Section::Structure.label(),
        Open::Files(_) => Section::Files.label(),
    }
}

/// The home list: the five sections with what each holds, then Save and
/// Discard. The list *is* the summary the old builder printed after each step.
fn render_sections(
    app: &App,
    builder: &Builder,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let width = area.width as usize;
    let items: Vec<ListItem> = Row::ALL
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let cursor = if index == builder.selected {
                g.cursor
            } else {
                " "
            };
            let (label, value, style) = match row {
                Row::Section(section) => (section.label(), builder.summary(*section), theme.dim()),
                Row::Save => (
                    "Save",
                    match builder.error.as_deref() {
                        Some(_) => "refused — see below".to_string(),
                        None => "write the template".to_string(),
                    },
                    if builder.error.is_some() {
                        theme.warn()
                    } else {
                        theme.good()
                    },
                ),
                Row::Discard => ("Discard", "leave without writing".to_string(), theme.dim()),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{cursor} "), theme.accent()),
                Span::styled(pad(label, 12), theme.text()),
                Span::styled(fit(&value, width.saturating_sub(15), g.ellipsis), style),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection);
    let mut state = ListState::default().with_selected(Some(builder.selected));
    frame.render_stateful_widget(list, area, &mut state);
    None
}

fn render_form(app: &App, form: &Form, frame: &mut Frame, area: Rect) -> Option<Position> {
    form.render(area, frame.buffer_mut(), &app.theme, LABEL)
}

type Face<'a> = (Option<Position>, Option<String>, Vec<(&'a str, &'a str)>);

fn render_variables<'a>(
    app: &App,
    builder: &Builder,
    list: &VarList,
    frame: &mut Frame,
    area: Rect,
) -> Face<'a> {
    let theme = &app.theme;
    if let Some((_, form)) = &list.editing {
        return (
            render_form(app, form, frame, area),
            form.error()
                .map(str::to_string)
                .or_else(|| form.focused().map(|field| field.hint.clone())),
            vec![("Tab", "next field"), ("Enter", "keep"), ("Esc", "back")],
        );
    }
    let items: Vec<ListItem> = builder
        .template
        .variables
        .iter()
        .map(|v| {
            let kind = match v.var_type {
                crate::core::template::VarType::Text => "text",
                crate::core::template::VarType::Select => "select",
            };
            ListItem::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(pad(&v.slug, LABEL), theme.text()),
                Span::styled(pad(kind, 8), theme.dim()),
                Span::styled(
                    if v.required { "required" } else { "" }.to_string(),
                    theme.dim(),
                ),
            ]))
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" no variables yet — a adds one", theme.dim())),
            area,
        );
    } else {
        let widget = List::new(items).highlight_style(theme.selection);
        let mut state = ListState::default().with_selected(Some(list.selected));
        frame.render_stateful_widget(widget, area, &mut state);
    }
    (
        None,
        Some("a variable's slug is its token: {artist}".to_string()),
        vec![
            ("a", "add"),
            ("Enter", "edit"),
            ("d", "remove"),
            ("K J", "reorder"),
            ("Esc", "back"),
        ],
    )
}

/// The folder paths on the left, the tree they make on the right, redrawn as
/// it is typed — which is what "a live tree" means and what a list of paths
/// alone never showed.
fn render_structure(
    app: &App,
    builder: &Builder,
    area_state: &crate::tui::widgets::text_area::TextArea,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let tree = crate::tui::app::studio::parse_paths_to_tree(&area_state.entries());
    let ascii = theme.glyphs.rule == "-";
    let lines: Vec<Line> = crate::tui::widgets::tree::lines(&tree, ascii)
        .into_iter()
        .map(|line| Line::from(Span::styled(format!(" {line}"), theme.dim())))
        .collect();
    let lines = if lines.is_empty() {
        vec![Line::from(Span::styled(" (no folders yet)", theme.dim()))]
    } else {
        lines
    };
    frame.render_widget(Paragraph::new(lines), panes[1]);

    // `render` keeps the editor's own scroll, so a long list does not jump.
    let mut editable = area_state.clone();
    let caret = editable.render(panes[0], frame.buffer_mut(), theme.text());
    let _ = builder;
    caret
}

fn render_files<'a>(
    app: &App,
    builder: &Builder,
    list: &FileList,
    frame: &mut Frame,
    area: Rect,
) -> Face<'a> {
    let theme = &app.theme;
    if let Some(edit) = &list.editing {
        return render_file_edit(app, builder, edit, frame, area);
    }
    let items: Vec<ListItem> = builder
        .template
        .files
        .iter()
        .map(|file| {
            let bytes = file.template.len().max(file.content.len());
            ListItem::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(pad(&file.path, 32), theme.text()),
                Span::styled(
                    if bytes == 0 {
                        "empty".to_string()
                    } else {
                        format!("{bytes} bytes")
                    },
                    theme.dim(),
                ),
            ]))
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " no files yet — a adds one (PROJECT_INFO.md is written for every project already)",
                theme.dim(),
            )),
            area,
        );
    } else {
        let widget = List::new(items).highlight_style(theme.selection);
        let mut state = ListState::default().with_selected(Some(list.selected));
        frame.render_stateful_widget(widget, area, &mut state);
    }
    (
        None,
        Some("a file's text is interpolated at create time".to_string()),
        vec![
            ("a", "add"),
            ("Enter", "edit"),
            ("d", "remove"),
            ("Esc", "back"),
        ],
    )
}

fn render_file_edit<'a>(
    app: &App,
    builder: &Builder,
    edit: &FileEdit,
    frame: &mut Frame,
    area: Rect,
) -> Face<'a> {
    let theme = &app.theme;
    let path_area = Rect::new(area.x, area.y, area.width, 1);
    let tokens_area = Rect::new(area.x, area.y + 1, area.width, 1);
    let body_area = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );

    let caret_path = edit.path.render_line(
        path_area,
        frame.buffer_mut(),
        Span::styled(" path  ", theme.accent()),
        theme.text(),
    );

    // Which tokens the body actually uses — the check that catches
    // `{clientname}` typed for a variable called `client_name`, before saving.
    let used = crate::tui::app::studio::tokens_used(&edit.body.text(), &builder.template);
    let available = crate::tui::app::studio::tokens(&builder.template);
    let tokens = if used.is_empty() {
        format!(" tokens  {}", available.join(" "))
    } else {
        format!(" will substitute  {}", used.join(" "))
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            fit(&tokens, area.width as usize, theme.glyphs.ellipsis),
            if used.is_empty() {
                theme.dim()
            } else {
                theme.good()
            },
        )),
        tokens_area,
    );

    let mut body = edit.body.clone();
    let caret_body = body.render(body_area, frame.buffer_mut(), theme.text());

    (
        if edit.in_body { caret_body } else { caret_path },
        edit.error.clone().or_else(|| {
            Some("Tab moves between the path and the text; an empty text is a marker file".into())
        }),
        vec![("Ctrl-S", "keep"), ("Tab", "path / text"), ("Esc", "back")],
    )
}

// ---------------------------------------------------------------------------
// Settings, and the first-run question
// ---------------------------------------------------------------------------

/// Every setting on one screen, grouped by heading, with what it is set to
/// beside it. The menu this replaces was seven submenus deep, so seeing what
/// fastf was configured to do meant walking the whole tree and remembering.
pub fn render_settings(
    app: &App,
    state: &SettingsState,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let g = theme.glyphs;
    let area = sized(area, 22);
    let (body, footer, keys) = frame_parts(app, " settings ".to_string(), frame, area)?;

    let width = body.width as usize;
    let items: Vec<ListItem> = state
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            if !row.selectable() {
                return ListItem::new(Line::from(Span::styled(
                    format!(" {}", row.label),
                    theme.accent(),
                )));
            }
            let cursor = if index == state.selected {
                g.cursor
            } else {
                " "
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{cursor}  "), theme.accent()),
                Span::styled(pad(row.label, 26), theme.text()),
                Span::raw("  "),
                Span::styled(
                    fit(&row.value, width.saturating_sub(32), g.ellipsis),
                    theme.dim(),
                ),
            ]))
        })
        .collect();
    let mut list_state = ListState::default()
        .with_offset(state.offset)
        .with_selected(Some(state.selected));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme.selection),
        body,
        &mut list_state,
    );

    // The editor draws over the row it belongs to, so the value being changed
    // stays where the eye already is.
    let caret = state.editing.as_ref().and_then(|editing| {
        let row = state.selected.checked_sub(state.offset)? as u16;
        render_setting_editor(app, editing, frame, body, row)
    });

    let (text, style) = match (state.error(), state.pending) {
        (Some(error), _) => (format!(" {} {error}", g.warn), theme.warn()),
        (None, true) => (" working…".to_string(), theme.dim()),
        (None, false) => (
            format!(" {}", state.row().map(|row| row.hint).unwrap_or_default()),
            theme.dim(),
        ),
    };
    footer_line(
        frame,
        footer,
        &fit(&text, footer.width as usize, g.ellipsis),
        style,
    );
    let pairs: Vec<(&str, &str)> = match &state.editing {
        Some(Editing::Bases { .. }) => vec![
            ("Ctrl-S", "keep"),
            ("Enter", "new line"),
            ("Esc", "leave it unchanged"),
        ],
        Some(Editing::Value { .. }) => {
            vec![("Enter", "keep"), ("Esc", "leave it unchanged")]
        }
        None => vec![
            ("↑↓", "choose"),
            ("Enter", "change / run"),
            ("Esc", "close"),
        ],
    };
    frame.render_widget(Paragraph::new(key_line(theme, &pairs)), keys);
    caret
}

fn render_setting_editor(
    app: &App,
    editing: &Editing,
    frame: &mut Frame,
    body: Rect,
    row: u16,
) -> Option<Position> {
    let theme = &app.theme;
    if row >= body.height {
        return None;
    }
    match editing {
        Editing::Value { label, input, .. } => {
            let line = Rect::new(body.x, body.y + row, body.width, 1);
            frame.render_widget(Clear, line);
            input.render_line(
                line,
                frame.buffer_mut(),
                Span::styled(format!("   {}  ", pad(label, 26)), theme.accent()),
                theme.text(),
            )
        }
        Editing::Bases { area, .. } => {
            // A list needs room, so it opens *over* its row in a frame of its
            // own — an editor with no edges looks like the screen went wrong.
            let height = (area.lines().len() as u16 + 2).clamp(4, body.height.saturating_sub(row));
            let box_area = Rect::new(body.x, body.y + row, body.width, height);
            // The whole band, not just the box: half a label showing past the
            // edge of an editor reads as a drawing fault.
            frame.render_widget(Clear, box_area);
            let outer = block(app, " one base per line ".to_string());
            let inner = outer.inner(box_area);
            frame.render_widget(outer, box_area);
            let mut editable = area.clone();
            editable.render(inner, frame.buffer_mut(), theme.text())
        }
    }
}

/// The first-run question, over an empty dashboard.
pub fn render_onboarding(
    app: &App,
    state: &crate::tui::app::settings::Onboarding,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let area = crate::tui::layout::centered_fixed(area, 68.min(area.width), 9);
    let (body, footer, keys) = frame_parts(app, " welcome ".to_string(), frame, area)?;

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(" {}", crate::tui::validators::ONBOARDING_PROMPT),
                theme.text(),
            )),
            Line::from(Span::styled(
                " Every new project is created inside this folder. You can add more",
                theme.dim(),
            )),
            Line::from(Span::styled(
                " later — a second drive, a network share — under Settings.",
                theme.dim(),
            )),
        ]),
        Rect::new(body.x, body.y, body.width, 3),
    );
    let line = Rect::new(body.x, body.y + 4, body.width, 1);
    let caret = state
        .input
        .render_line(line, frame.buffer_mut(), Span::raw(" "), theme.text());

    let (text, style) = match (&state.error, state.pending) {
        (Some(error), _) => (format!(" {} {error}", theme.glyphs.warn), theme.warn()),
        (None, true) => (" creating it…".to_string(), theme.dim()),
        (None, false) => (
            " an empty answer skips — the question comes back next time".to_string(),
            theme.dim(),
        ),
    };
    footer_line(
        frame,
        footer,
        &fit(&text, footer.width as usize, theme.glyphs.ellipsis),
        style,
    );
    frame.render_widget(
        Paragraph::new(key_line(
            theme,
            &[("Enter", "create it"), ("Esc", "skip for now")],
        )),
        keys,
    );
    caret
}
