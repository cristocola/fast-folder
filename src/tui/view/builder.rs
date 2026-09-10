//! The template builder, the settings screen and the first-run question.
//!
//! The builder is one dialog with several faces, drawn at one size so entering
//! a section and coming back does not move the box under the reader — the same
//! bargain `view::modals::render_flow` makes. (The studio it used to open from
//! is a tab now: `view::templates`.)

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::tui::app::App;
use crate::tui::app::settings::{Editing, SettingsState};
use crate::tui::app::studio::{Builder, FileEdit, FileList, Open, Row, Section, VarList};
use crate::tui::command::{self, Context};
use crate::tui::theme::Theme;
use crate::tui::view::{fit, pad};
use crate::tui::widgets::form::Form;

/// How wide the labels down the left of every list here are.
const LABEL: usize = 18;

/// A box sized to what it holds — `layout::sized_dialog`, which the app
/// reads too, so a list's viewport is clamped to the rows that are drawn.
fn sized(area: Rect, body: u16) -> Rect {
    crate::tui::layout::sized_dialog(area, body)
}

/// How many rows the builder's open face wants.
///
/// With the panel open this is also how much room the *explanation* gets, and
/// a seven-row list would otherwise cut every paragraph beside it to three
/// lines. The list keeps its own height; the dialog takes the taller of the
/// two, so opening a part and coming back never moves the box.
fn body_height(explaining: bool, builder: &Builder) -> u16 {
    let wanted = face_height(builder);
    if explaining {
        wanted.max(PANEL_ROWS)
    } else {
        wanted
    }
}

/// The rows a panel needs before it is worth the width it costs.
const PANEL_ROWS: u16 = 16;

fn face_height(builder: &Builder) -> u16 {
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
pub(crate) fn frame_parts(
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

pub(crate) fn footer_line(frame: &mut Frame, area: Rect, text: &str, style: ratatui::style::Style) {
    frame.render_widget(Paragraph::new(Span::styled(text.to_string(), style)), area);
}

/// A dialog's own key line, cut at a **whole pair**.
///
/// `registry_keys` below has always fitted its list to the width it is drawn
/// in; this one took every pair it was handed and let the terminal cut the
/// last one wherever it landed — so a narrow settings dialog advertised
/// `Esc leave i`, which is a key line saying something that is not a key. Half
/// an entry is worse than none: the entries are ordered, so the ones that fit
/// are the ones that matter most.
pub(crate) fn key_line<K: AsRef<str>, V: AsRef<str>>(
    theme: &Theme,
    pairs: &[(K, V)],
    width: usize,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (key, what) in pairs {
        let key = format!(" {} ", key.as_ref());
        let what = format!("{}  ", what.as_ref());
        let cost = unicode_width::UnicodeWidthStr::width(key.as_str())
            + unicode_width::UnicodeWidthStr::width(what.as_str());
        if used + cost > width {
            break;
        }
        used += cost;
        spans.push(Span::styled(key, theme.key()));
        spans.push(Span::styled(what, theme.dim()));
    }
    Line::from(spans)
}

/// Owned pairs, for the key lines a widget writes itself — a form, a text
/// area — whose keys the widget consumes and the registry does not see.
pub(crate) fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(key, what)| ((*key).to_string(), (*what).to_string()))
        .collect()
}

/// The key line a list draws from the registry: the arrows first, then every
/// hinted command that fires in `ctx`, as many as fit in `width`. This is the
/// same list the help overlay and the palette read, so a key the line shows
/// is a key the list answers.
fn registry_keys(app: &App, ctx: Context, width: usize) -> Vec<(String, String)> {
    let mut out = vec![("↑↓".to_string(), "choose".to_string())];
    out.extend(
        command::hints(ctx, app, width.saturating_sub(12))
            .into_iter()
            .map(|(key, what)| (key, what.to_string())),
    );
    out
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
    // Whether the panel is coming has to be settled before the box is sized,
    // because a panel wants more rows than a seven-row list — and asking after
    // the fact grew the dialog on every window too narrow to draw one.
    let explaining = app.explain_open
        && crate::tui::layout::panel_fits_width(sized(area, 0).width.saturating_sub(2));
    let area = sized(area, body_height(explaining, builder));
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

    if builder.pending.is_some() {
        footer_line(frame, footer, " reading the template…", theme.dim());
        return None;
    }
    if builder.saving {
        // The list stays drawn under it: a refusal lands here and the answers
        // have to still be on screen when it does.
        render_sections(app, builder, frame, body);
        footer_line(frame, footer, " saving…", theme.dim());
        return None;
    }

    // The explanation, beside what it explains. In a window too narrow for
    // both, the list keeps the whole body and the footer carries the one-line
    // hint it always did — which is the 80x24 path, and the reason the footer
    // is still built below whether or not the panel is drawn.
    let (body, explaining) = match explaining
        .then(|| crate::tui::layout::builder_panel(body))
        .flatten()
    {
        Some((list, panel)) => {
            render_panel(app, builder, frame, panel);
            (list, true)
        }
        None => (body, false),
    };

    let width = keys.width as usize;
    let (caret, hint, key_pairs) = match &builder.open {
        None => (
            render_sections(app, builder, frame, body),
            // A refusal first; then whatever is wrong with the pattern; then
            // what the highlighted row is for. The line was empty until a save
            // was refused, which is a whole interface's worth of unused space
            // over a list of five nouns.
            // A refusal, then a warning, then what the highlighted row is
            // for — and the last of those only when the panel is not already
            // saying it two columns across.
            //
            // **The warning is always here and never there.** The panel
            // explains; this line refuses and warns. Splitting them that way
            // gives each one place — the rule the whole app is built on — and
            // it is also the only one that cannot lose: the footer is a fixed
            // row of the box, so a warning can never be pushed off the end of
            // it the way it can off the bottom of a panel.
            builder
                .error
                .clone()
                .or_else(|| row_note(builder))
                .or_else(|| (!explaining).then(|| builder.row().hint().to_string())),
            // Esc asks before it discards now, so the registry's own word for
            // the key — "close" — is the true one and the rewrite is gone.
            registry_keys(app, Context::Builder, width),
        ),
        Some(Open::Metadata(form)) | Some(Open::Id(form)) => (
            render_form(app, form, frame, body),
            form.error()
                .map(str::to_string)
                .or_else(|| field_hint(form, explaining)),
            pairs(&[("Tab", "next field"), ("Enter", "keep"), ("Esc", "back")]),
        ),
        Some(Open::Variables(list)) => {
            render_variables(app, builder, list, frame, body, width, explaining)
        }
        Some(Open::Structure(area_state)) => {
            let caret = render_structure(app, area_state, frame, body);
            (
                caret,
                (!explaining).then(|| {
                    "one folder path per line — use / to nest on every platform".to_string()
                }),
                // Same order as the base list's, and for the same reason.
                pairs(&[
                    ("Ctrl-S", "keep"),
                    ("Enter", "new line"),
                    ("Esc", "back"),
                    ("Ctrl-K", "drop the line"),
                ]),
            )
        }
        Some(Open::Files(list)) => render_files(app, builder, list, frame, body, width, explaining),
    };

    let warned = builder.error.is_some() || (builder.open.is_none() && row_note(builder).is_some());
    let style = if warned { theme.warn() } else { theme.dim() };
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
    frame.render_widget(
        Paragraph::new(key_line(theme, &key_pairs, keys.width as usize)),
        keys,
    );
    caret
}

/// The panel: what the highlighted row or field is, and — on the section list,
/// where no editor is showing it — what this template would produce.
///
/// It exists because the whole teaching budget of this editor used to be one
/// footer line cut with an ellipsis, over a list of five nouns in the
/// manifest's own vocabulary. Its words come from `guide`, which is the one
/// place any of them are written.
fn render_panel(app: &App, builder: &Builder, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(theme.border(false));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // A column of padding off the rule, so the prose is not against it.
    let text = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let notes = panel_notes(builder, theme.glyphs.is_ascii());
    let lines = crate::tui::view::note_lines(&notes, theme, text.width as usize);
    frame.render_widget(Paragraph::new(lines), text);
}

/// Which explanation belongs beside the face that is open.
fn panel_notes(builder: &Builder, ascii: bool) -> Vec<crate::tui::guide::Note> {
    use crate::tui::guide;

    let field_or_section = |section: Section, form: &Form| {
        form.focused()
            .and_then(|field| guide::panel_for_field(section, &field.key))
            .unwrap_or_else(|| guide::explain_section(section))
    };
    match &builder.open {
        None => guide::panel_for_row(builder.row(), &builder.template, ascii),
        Some(Open::Metadata(form)) => field_or_section(Section::Metadata, form),
        Some(Open::Id(form)) => field_or_section(Section::Id, form),
        Some(Open::Variables(list)) => match &list.editing {
            Some((_, form)) => field_or_section(Section::Variables, form),
            None => guide::explain_section(Section::Variables),
        },
        Some(Open::Structure(_)) => guide::explain_section(Section::Structure),
        Some(Open::Files(_)) => guide::explain_section(Section::Files),
    }
}

/// What the highlighted row has to say beyond its own hint: for Metadata,
/// what the naming pattern would actually do. Nothing for the others yet.
fn row_note(builder: &Builder) -> Option<String> {
    match builder.row() {
        Row::Section(Section::Metadata) => {
            crate::tui::app::studio::pattern_warning(&builder.template)
        }
        _ => None,
    }
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
                // A pattern that ignores the template's own variables is
                // marked where it can be seen without walking the list — the
                // mistake costs a whole first template, and every project made
                // from one gets the same folder name.
                Row::Section(Section::Metadata)
                    if crate::tui::app::studio::pattern_warning(&builder.template).is_some() =>
                {
                    (
                        Section::Metadata.label(),
                        format!("{}   {}", builder.summary(Section::Metadata), g.warn),
                        theme.warn(),
                    )
                }
                Row::Section(section) => (section.label(), builder.summary(*section), theme.dim()),
                // The row counts what is still worth a look, so the coach
                // works with the panel closed too. It is **advice and never a
                // refusal** — `Template::validate` and `operations` keep all of
                // the authority, and every one of these saves perfectly well.
                Row::Save => {
                    let gaps = crate::tui::guide::gaps(&builder.template).len();
                    match (builder.error.as_deref(), gaps) {
                        (Some(_), _) => ("Save", "refused — see below".to_string(), theme.warn()),
                        (None, 0) => ("Save", "write the template".to_string(), theme.good()),
                        // Just the count: the row's own footer hint and the
                        // panel both already say what Save does, and a summary
                        // cut at the column edge says neither.
                        (None, n) => ("Save", format!("{n} to look at first"), theme.dim()),
                    }
                }
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

/// The focused field's one-line hint, unless the panel is already explaining
/// that field at length beside it.
fn field_hint(form: &Form, explaining: bool) -> Option<String> {
    (!explaining)
        .then(|| form.focused().map(|field| field.hint.clone()))
        .flatten()
}

fn render_form(app: &App, form: &Form, frame: &mut Frame, area: Rect) -> Option<Position> {
    form.render(area, frame.buffer_mut(), &app.theme, LABEL)
}

type Face = (Option<Position>, Option<String>, Vec<(String, String)>);

fn render_variables(
    app: &App,
    builder: &Builder,
    list: &VarList,
    frame: &mut Frame,
    area: Rect,
    width: usize,
    explaining: bool,
) -> Face {
    let theme = &app.theme;
    if let Some((_, form)) = &list.editing {
        return (
            render_form(app, form, frame, area),
            form.error()
                .map(str::to_string)
                .or_else(|| field_hint(form, explaining)),
            pairs(&[("Tab", "next field"), ("Enter", "keep"), ("Esc", "back")]),
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
        (!explaining).then(|| "a variable's slug is its token: {artist}".to_string()),
        registry_keys(app, Context::Builder, width),
    )
}

/// The folder paths on the left, the tree they make on the right, redrawn as
/// it is typed — which is what "a live tree" means and what a list of paths
/// alone never showed.
fn render_structure(
    app: &App,
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
    let ascii = theme.glyphs.is_ascii();
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
    area_state.render(panes[0], frame.buffer_mut(), theme.text())
}

fn render_files(
    app: &App,
    builder: &Builder,
    list: &FileList,
    frame: &mut Frame,
    area: Rect,
    width: usize,
    explaining: bool,
) -> Face {
    let theme = &app.theme;
    if let Some(edit) = &list.editing {
        return render_file_edit(app, builder, edit, frame, area, explaining);
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
        (!explaining).then(|| "a file's text is interpolated at create time".to_string()),
        registry_keys(app, Context::Builder, width),
    )
}

fn render_file_edit(
    app: &App,
    builder: &Builder,
    edit: &FileEdit,
    frame: &mut Frame,
    area: Rect,
    explaining: bool,
) -> Face {
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

    let caret_body = edit
        .body
        .render(body_area, frame.buffer_mut(), theme.text());

    (
        if edit.in_body { caret_body } else { caret_path },
        edit.error.clone().or_else(|| {
            (!explaining).then(|| {
                "Tab moves between the path and the text; an empty text is a marker file".into()
            })
        }),
        pairs(&[("Ctrl-S", "keep"), ("Tab", "path / text"), ("Esc", "back")]),
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
    if state.rows.is_empty() {
        footer_line(frame, footer, " reading the settings…", theme.dim());
        frame.render_widget(
            Paragraph::new(key_line(
                theme,
                &pairs(&[("Esc", "close")]),
                keys.width as usize,
            )),
            keys,
        );
        return None;
    }

    let width = body.width as usize;
    // **Measured from the rows, not fixed at 26.** `pad` does not truncate, so
    // a label longer than the constant pushed its value right and past the box
    // edge — and today's longest, "Ask to open after creating", is exactly 26,
    // which is zero headroom for the next setting anybody adds. The rule
    // everywhere else in this app is that a column is as wide as its content:
    // a title can never run into its description.
    let label_width = state
        .rows
        .iter()
        .filter(|row| row.selectable())
        .map(|row| unicode_width::UnicodeWidthStr::width(row.label))
        .max()
        .unwrap_or(26)
        .min(width.saturating_sub(12));

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
                Span::styled(pad(row.label, label_width), theme.text()),
                Span::raw("  "),
                Span::styled(
                    // The cursor, two spaces, the label, two more: what is left
                    // is the value's.
                    fit(
                        &row.value,
                        width.saturating_sub(label_width + 6),
                        g.ellipsis,
                    ),
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
        render_setting_editor(app, editing, frame, body, row, label_width)
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
    let key_pairs: Vec<(String, String)> = match &state.editing {
        // `Ctrl-K` too: the same widget answers it, the Structure editor's
        // key line names it, and dropping a line is exactly what somebody
        // opens this box to do.
        //
        // The way out comes **before** it, because `key_line` drops whole
        // pairs from the end when the box is narrow — and the one entry that
        // must survive that is the one that says how to leave.
        Some(Editing::Bases { .. }) => pairs(&[
            ("Ctrl-S", "keep"),
            ("Enter", "new line"),
            ("Esc", "leave it unchanged"),
            ("Ctrl-K", "drop the line"),
        ]),
        Some(Editing::Value { .. }) => pairs(&[("Enter", "keep"), ("Esc", "leave it unchanged")]),
        None => registry_keys(app, Context::Settings, keys.width as usize),
    };
    frame.render_widget(
        Paragraph::new(key_line(theme, &key_pairs, keys.width as usize)),
        keys,
    );
    caret
}

fn render_setting_editor(
    app: &App,
    editing: &Editing,
    frame: &mut Frame,
    body: Rect,
    row: u16,
    label_width: usize,
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
                // The same width the list uses, so the editor opens exactly
                // over the row it belongs to.
                Span::styled(format!("   {}  ", pad(label, label_width)), theme.accent()),
                theme.text(),
            )
        }
        Editing::Bases { area, .. } => {
            // A list needs room, so it opens *over* its row in a frame of its
            // own — an editor with no edges looks like the screen went wrong.
            // `box_at_row` slides it up when the row is near the bottom; this
            // was a `clamp(4, body.height - row)`, and `Ord::clamp` panics when
            // the room is smaller than the minimum, which every window between
            // 16 and 23 rows tall made it.
            let box_area =
                crate::tui::layout::box_at_row(body, row, area.lines().len() as u16 + 2, 4);
            // The whole band, not just the box: half a label showing past the
            // edge of an editor reads as a drawing fault.
            frame.render_widget(Clear, box_area);
            let outer = block(app, " one base per line ".to_string());
            let inner = outer.inner(box_area);
            frame.render_widget(outer, box_area);
            area.render(inner, frame.buffer_mut(), theme.text())
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
    let area = crate::tui::layout::centered_fixed(area, 68.min(area.width), 10);
    let (body, footer, keys) = frame_parts(app, " welcome ".to_string(), frame, area)?;

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(" {}", crate::tui::validators::ONBOARDING_PROMPT),
                theme.text(),
            )),
            Line::from(Span::styled(
                " This folder is your first base — where new projects are created.",
                theme.dim(),
            )),
            Line::from(Span::styled(
                " Add more later (a second drive, a network share) under Settings.",
                theme.dim(),
            )),
            // The one thing a first run cannot discover for itself: that
            // templates are what shape a project, and that there is a guide to
            // them. Both keys read from the registry, never spelled here.
            Line::from(Span::styled(
                format!(
                    " Templates shape every project: {} opens them, {} explains them.",
                    crate::tui::command::key_of(command::CommandId::Templates),
                    crate::tui::command::key_of(command::CommandId::Guide),
                ),
                theme.dim(),
            )),
        ]),
        Rect::new(body.x, body.y, body.width, 4),
    );
    let line = Rect::new(body.x, body.y + 5, body.width, 1);
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
            keys.width as usize,
        )),
        keys,
    );
    caret
}
