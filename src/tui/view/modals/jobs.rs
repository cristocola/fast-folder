//! A job on screen: the dialog a move, copy, delete or reconcile is followed
//! in, and the batch runner's.

use super::*;

/// A job's progress — a move, a copy out of the library, a delete, a
/// reconcile — drawn over the dashboard. It is not a modal on the stack: it is
/// drawn while a job is followed and not hidden (`App::shown_progress`), and
/// Esc hides it while the job goes on. An in-app batch draws its own
/// (`render_job`), so this stays out of the way while `App::job` is up.
///
/// **One row per step**, when the job knows its steps: the finished ones
/// ticked with what they counted, the current one with its count, its bar and
/// the entry it is at, the rest dim — so removing the old copy after a move,
/// minutes on a cloud mount, is a step that moves, never a full bar that sits
/// still. A window too short for every row shows the current step alone.
pub fn render_job_progress(app: &App, frame: &mut Frame, area: Rect) {
    if app.job.is_some() {
        return;
    }
    let Some(progress) = app.shown_progress() else {
        return;
    };
    let progress = &progress;
    let theme = &app.theme;
    let width = 62.min(area.width);
    let inner_width = (width as usize).saturating_sub(2);
    let room = area.height.saturating_sub(2) as usize;

    let mut lines = progress_lines(app, progress, inner_width, true);
    if lines.len() > room {
        lines = progress_lines(app, progress, inner_width, false);
    }
    lines.push(Line::from(""));
    // The way out, read from the registry: Esc hides the dialog and the job
    // goes on; Ctrl-C cancels, until the job is past its point of no return.
    let hide = command::key_of_in(CommandId::Back, &theme.glyphs);
    let cancel = command::key_of_in(CommandId::Interrupt, &theme.glyphs);
    lines.push(Line::from(Span::styled(
        fit(
            &if progress.committed {
                format!(" {hide} hides · it finishes by itself, even if fastf closes")
            } else {
                format!(" {hide} hides · {cancel} cancels · it goes on if fastf closes")
            },
            inner_width,
            theme.glyphs.ellipsis,
        ),
        theme.dim(),
    )));
    // Shed from the bottom of the body, never the cancel line.
    while lines.len() > room.max(1) && lines.len() > 2 {
        lines.remove(lines.len() - 3);
    }

    let title = app
        .shown_title()
        .trim_end_matches('…')
        .trim_end_matches("...")
        .trim();
    let area = centered_fixed(area, width, lines.len() as u16 + 2);
    super::clear(frame, area, &app.theme);
    let block = frame_block(app, format!(" {title} "), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A job's progress as rows `width` wide: every planned step when `all`, else
/// only the current one — each with the item it is about, when there are
/// several.
fn progress_lines<'a>(
    app: &App,
    progress: &crate::core::assets::Progress,
    width: usize,
    all: bool,
) -> Vec<Line<'a>> {
    use crate::core::assets::{JobPhase, count_text};
    let theme = &app.theme;
    let g = theme.glyphs;
    // The bar, two in, its percentage, and a column of margin: `bar` draws
    // one cell before the track and five after it.
    let track = width.saturating_sub(9);
    let mut lines = Vec::new();

    let item = progress.item_text();
    if !item.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!(" {item}  "), theme.dim()),
            Span::styled(
                fit(
                    &progress.item_label,
                    width.saturating_sub(item.width() + 3),
                    g.ellipsis,
                ),
                theme.text(),
            ),
        ]));
    }

    let current = progress.phase;
    let planned = all && progress.steps.contains(&current);
    let finished = |phase: JobPhase| progress.finished.iter().find(|step| step.phase == phase);
    let label_of = |phase: JobPhase| {
        if finished(phase).is_some() && phase != current {
            phase.past()
        } else {
            phase.as_str()
        }
    };
    let steps: Vec<JobPhase> = if planned {
        progress.steps.clone()
    } else {
        vec![current]
    };
    let column = steps
        .iter()
        .map(|phase| label_of(*phase).width())
        .max()
        .unwrap_or(0)
        .min(width.saturating_sub(4));

    for phase in steps {
        let label = pad(&fit(label_of(phase), column, g.ellipsis), column);
        if phase == current {
            let count = progress.count_text();
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", g.cursor), theme.accent()),
                Span::styled(label, theme.text()),
                Span::styled(
                    fit(
                        &format!("  {count}"),
                        width.saturating_sub(column + 3),
                        g.ellipsis,
                    ),
                    theme.text(),
                ),
            ]));
            if phase == JobPhase::Copying && progress.total_bytes > 0 {
                // The bar measures bytes, since one large file is most of a
                // copy's time, and says them after it; the row above counts
                // files.
                let bytes = format!(
                    "  {} of {}",
                    crate::util::human_bytes::human_bytes(progress.copied_bytes),
                    crate::util::human_bytes::human_bytes(progress.total_bytes)
                );
                let mut line = indented(bar(
                    app,
                    track.saturating_sub(bytes.width()),
                    progress.copied_bytes,
                    progress.total_bytes,
                ));
                line.spans.push(Span::styled(bytes, theme.dim()));
                lines.push(line);
            } else if progress.has_total() {
                lines.push(indented(bar(
                    app,
                    track,
                    progress.step_done as u64,
                    progress.step_total as u64,
                )));
            }
            if let Some(stall) = progress.stall_text() {
                lines.push(Line::from(Span::styled(
                    format!("   {}", fit(&stall, width.saturating_sub(4), g.ellipsis)),
                    theme.warn(),
                )));
            } else if !progress.current_file.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!(
                        "   {}",
                        fit(&progress.current_file, width.saturating_sub(4), g.ellipsis)
                    ),
                    theme.dim(),
                )));
            }
        } else if let Some(step) = finished(phase) {
            let count = count_text(phase, step.count, 0);
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", g.check), theme.good()),
                Span::styled(label, theme.dim()),
                Span::styled(
                    fit(
                        &format!("  {count}"),
                        width.saturating_sub(column + 3),
                        g.ellipsis,
                    ),
                    theme.dim(),
                ),
            ]));
        } else {
            lines.push(Line::from(Span::styled(format!("   {label}"), theme.dim())));
        }
    }
    lines
}

/// A bar under a step's label, two columns in.
fn indented(line: Line<'_>) -> Line<'_> {
    let mut spans = vec![Span::raw("  ")];
    spans.extend(line.spans);
    Line::from(spans)
}

/// An in-app batch's progress (tag, note, unregister), drawn over the
/// dashboard while one runs. Like `render_job_progress`, it is not a modal on
/// the stack — it lives exactly as long as `App::job`. The modal names the
/// item being acted on and counts the failures so far.
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
        // The items — a batch of tags or notes has no bytes to report, and
        // its bar is the only thing that moves.
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
    // guessed at the widest case leaves blank rows under a two-line batch and
    // cuts the cancel line off a tall one.
    let area = centered_fixed(area, width, lines.len() as u16 + 2);
    super::clear(frame, area, &app.theme);
    let block = frame_block(app, format!(" {} ", job.kind.verb()), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}
