//! A flow (new, apply, register, from-folder): its form, and the preview of
//! what it would do.

use super::*;
use crate::core::project::DryRunReport;
use crate::tui::app::wizard::{ApplyPreview, FromFolderPreview, RecursivePreview, RegisterPreview};

// ---------------------------------------------------------------------------
// The flows: create, apply, register
// ---------------------------------------------------------------------------

/// A flow is one dialog with two faces: the questions, and what answering them
/// would do. Both are drawn in the same frame at the same size, so committing
/// and going back do not move the box under the reader.
///
/// The dialog's own rectangle, sized to what it holds.
///
/// Split out so `update` can ask the same question `view` answers: the
/// preview's scroll is clamped from this, and a scroll clamped only at draw
/// time runs past the end of the preview and the dialog reads as frozen.
fn flow_rect(area: Rect, flow: &Flow) -> Rect {
    // Sized to what it holds, so the footer sits under the last answer rather
    // than at the bottom of a mostly-empty box. The preview takes the room it
    // needs and scrolls past that.
    let width = crate::tui::layout::fit_between(
        crate::tui::layout::percent_of(area.width, 76),
        46.min(area.width),
        96,
    );
    let body = match flow.step {
        Step::Form => flow.form.rows() as u16,
        Step::Preview => preview_height(flow),
    };
    let height = crate::tui::layout::fit_between(
        body + 4,
        8,
        crate::tui::layout::percent_of(area.height, 88),
    );
    centered_fixed(area, width, height)
}

/// The rows the body gets: the dialog less its border and its two bottom rows.
fn flow_body_rows(area: Rect, flow: &Flow) -> usize {
    // `Block::inner` on an all-borders block is two rows and two columns.
    let inner = flow_rect(area, flow).height.saturating_sub(2);
    if inner < 4 {
        return 0;
    }
    (inner - 2) as usize
}

/// How far the preview can be scrolled before its last line is on screen.
///
/// `update` reads this so the cursor can never leave the drawn window, which is
/// what `layout.rs` exists for everywhere else in the app.
pub(crate) fn preview_max_scroll(app: &App, flow: &Flow) -> usize {
    let lines = match &flow.preview {
        Some(preview) => preview_lines(app, preview).len(),
        None => 1,
    };
    lines.saturating_sub(flow_body_rows(app.area(), flow))
}

pub(super) fn render_flow(
    app: &App,
    flow: &Flow,
    frame: &mut Frame,
    area: Rect,
) -> Option<Position> {
    let theme = &app.theme;
    let area = flow_rect(area, flow);
    super::clear(frame, area, &app.theme);
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

    // Cut at whole pairs, the way on and the way out before the extras.
    let pairs: Vec<(String, String)> = match flow.step {
        Step::Form => crate::tui::view::builder::pairs(&[
            ("Enter", "preview"),
            ("Esc", "cancel"),
            ("Tab", "next field"),
        ]),
        Step::Preview => vec![
            (
                "Enter".to_string(),
                flow.kind.commit().trim_start_matches("Enter ").to_string(),
            ),
            ("Esc".to_string(), "back to the answers".to_string()),
            (scroll_keys(&theme.glyphs), "scroll".to_string()),
        ],
    };
    frame.render_widget(
        Paragraph::new(crate::tui::view::builder::key_line(
            theme,
            &pairs,
            inner.width as usize,
        )),
        keys,
    );
    caret
}

/// The arrows, as the preview's key line prints them — read, never written.
fn scroll_keys(g: &crate::tui::theme::Glyphs) -> String {
    command::movement_pair(Context::Modal, g)
        .map(|(keys, _)| keys)
        .unwrap_or_default()
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
    let mut lines: Vec<Line> = Vec::new();
    match preview {
        Preview::Create(report) => create_preview_lines(theme, report, &mut lines),
        Preview::Apply(apply) => apply_preview_lines(theme, apply, &mut lines),
        Preview::Register(register) => register_preview_lines(theme, register, &mut lines),
        Preview::FromFolder(scan) => from_folder_preview_lines(theme, scan, &mut lines),
        Preview::Recursive(recursive) => recursive_preview_lines(theme, recursive, &mut lines),
    }
    lines
}

fn create_preview_lines<'a>(
    theme: &crate::tui::theme::Theme,
    report: &'a DryRunReport,
    lines: &mut Vec<Line<'a>>,
) {
    let g = theme.glyphs;
    lines.push(Line::from(vec![
        Span::styled(" ", theme.dim()),
        Span::styled(report.folder_name.clone(), theme.bold()),
    ]));
    for line in crate::tui::widgets::tree::lines(&report.structure, g.is_ascii()) {
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
        // The same marker the command line prints: this file's
        // `{braces}` are what lands, not a substitution that failed.
        let mut path = vec![Span::styled(format!(" {}", preview.path), theme.accent())];
        if preview.verbatim {
            path.push(Span::styled("  (verbatim)", theme.dim()));
        }
        lines.push(Line::from(path));
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

fn apply_preview_lines<'a>(
    theme: &crate::tui::theme::Theme,
    apply: &'a ApplyPreview,
    lines: &mut Vec<Line<'a>>,
) {
    let g = theme.glyphs;
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

fn register_preview_lines<'a>(
    theme: &crate::tui::theme::Theme,
    register: &'a RegisterPreview,
    lines: &mut Vec<Line<'a>>,
) {
    let g = theme.glyphs;
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

fn from_folder_preview_lines<'a>(
    theme: &crate::tui::theme::Theme,
    scan: &'a FromFolderPreview,
    lines: &mut Vec<Line<'a>>,
) {
    let g = theme.glyphs;
    lines.push(Line::from(vec![
        Span::styled(format!(" {} ", g.arrow), theme.accent()),
        Span::styled(scan.slug.clone(), theme.bold()),
    ]));
    if !scan.structure.is_empty() {
        lines.push(Line::from(""));
        for line in crate::tui::widgets::tree::lines(&scan.structure, g.is_ascii()) {
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
                crate::util::plural::s(scan.folders),
                scan.files.len(),
                crate::util::plural::s(scan.files.len())
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

fn recursive_preview_lines<'a>(
    theme: &crate::tui::theme::Theme,
    recursive: &'a RecursivePreview,
    lines: &mut Vec<Line<'a>>,
) {
    let g = theme.glyphs;
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
        return;
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
            crate::util::plural::s(recursive.rows.len())
        ),
        theme.good(),
    )));
}

fn field_line<'a>(theme: &crate::tui::theme::Theme, key: &'a str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("   {key:<14} "), theme.dim()),
        Span::styled(value, theme.text()),
    ])
}
