//! The bases panel: one row per base — ticked when active, the default
//! marked — under a line saying which view is on, with what the selected
//! base's state means on the footer and the keys from the registry.

use super::*;
use crate::tui::app::bases::BasesPanel;
use crate::tui::app::data::BaseInfo;
use crate::tui::app::library::BasesView;
use crate::tui::view::builder::{footer_line, frame_parts, key_line};

/// The cells a row spends before its label: a space, the cursor and a space,
/// the tick box and a space; the default's marker comes after them.
const ROW_LEAD: usize = 1 + 1 + 1 + 3 + 1;

pub(super) fn render_bases(app: &App, panel: &BasesPanel, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let g = theme.glyphs;
    let bases: &[BaseInfo] = app.known_bases().unwrap_or(&[]);
    let notes: Vec<String> = bases.iter().map(|base| base_note_text(app, base)).collect();
    let paths: Vec<String> = bases
        .iter()
        .map(|base| crate::util::paths::display_path(&base.path))
        .collect();

    // **Columns measured from their content**: the label, the figure, the
    // path, each as wide as its widest, so a long label never runs into a
    // number and the box is no wider than its rows.
    let arrow_w = g.arrow.width() + 1;
    let label_w = bases.iter().map(|b| b.label.width()).max().unwrap_or(0);
    let note_w = notes.iter().map(|n| n.width()).max().unwrap_or(0);
    let path_w = paths.iter().map(|p| p.width()).max().unwrap_or(0);
    let row_w = ROW_LEAD + arrow_w + label_w + 2 + note_w + 3 + path_w + 1;
    // The footer's sentences are measured too, so whichever row the cursor
    // is on, its sentence is read whole on a window with the room — and the
    // box does not change width as the cursor moves.
    let sentence_w = sentences(app)
        .iter()
        .map(|sentence| sentence.width() + 2)
        .max()
        .unwrap_or(0);
    let wanted = row_w.max(sentence_w) + 2;
    let box_ =
        crate::tui::layout::bases_box(area, bases.len(), wanted.min(u16::MAX as usize) as u16);
    let Some((body, footer, keys)) = frame_parts(app, " bases ".to_string(), frame, box_) else {
        return;
    };
    let width = body.width as usize;

    frame.render_widget(
        Paragraph::new(Line::from(fit_spans(showing(app), width, g.ellipsis))),
        Rect { height: 1, ..body },
    );

    // The line that says which view is on and a blank under it; a blank
    // between the last base and the footer.
    let list = Rect {
        y: body.y + 2.min(body.height),
        height: body.height.saturating_sub(3),
        ..body
    };
    let path_room = width.saturating_sub(ROW_LEAD + arrow_w + label_w + 2 + note_w + 3);
    let items: Vec<ListItem> = bases
        .iter()
        .zip(notes.iter().zip(&paths))
        .enumerate()
        .map(|(index, (base, (note, path)))| {
            let active = app.base_is_active(base);
            // A base the list leaves out is dim as a whole: the row says it
            // is not on screen before the box does.
            let out = !active && app.library.view == BasesView::Active;
            let text = if out { theme.dim() } else { theme.text() };
            let cursor = if index == panel.selected {
                g.cursor
            } else {
                " "
            };
            let tick = if active { "[x]" } else { "[ ]" };
            let marker = if base.is_default { g.arrow } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {cursor} "), theme.accent()),
                Span::styled(format!("{tick} "), text),
                Span::styled(pad(marker, arrow_w), theme.dim()),
                Span::styled(pad(&base.label, label_w), text),
                Span::raw("  "),
                Span::styled(
                    format!("{}{note}", " ".repeat(note_w.saturating_sub(note.width()))),
                    theme.dim(),
                ),
                Span::raw("   "),
                Span::styled(fit(path, path_room, g.ellipsis), theme.dim()),
            ]))
        })
        .collect();
    let mut state = ListState::default()
        .with_offset(panel.offset)
        .with_selected(Some(panel.selected));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme.selection),
        list,
        &mut state,
    );

    let sentence = match bases.get(panel.selected) {
        Some(base) => crate::tui::guide::base_note(
            base.is_default,
            app.base_is_active(base),
            app.library.view == BasesView::Every,
        ),
        None => "reading the bases…".to_string(),
    };
    footer_line(
        frame,
        footer,
        &fit(&format!(" {sentence}"), footer.width as usize, g.ellipsis),
        theme.dim(),
    );
    let pairs = key_pairs(app, keys.width as usize);
    frame.render_widget(
        Paragraph::new(key_line(theme, &pairs, keys.width as usize)),
        keys,
    );
}

/// Every sentence the footer can show, for the box's width.
fn sentences(app: &App) -> [String; 4] {
    let every = app.library.view == BasesView::Every;
    [
        crate::tui::guide::base_note(true, true, every),
        crate::tui::guide::base_note(false, true, every),
        crate::tui::guide::base_note(false, false, true),
        crate::tui::guide::base_note(false, false, false),
    ]
}

/// **The panel's keys, the way out second.** A key line is cut at whole pairs
/// from its end, and the registry's order would put Esc after every verb: here
/// Space leads, then Esc, then Enter — whose menu lists every verb with its
/// key — then the rest. The arrows go first only where the three after them
/// still fit, since a list with a cursor already says how to move. Labels and
/// words are the registry's.
fn key_pairs(app: &App, width: usize) -> Vec<(String, String)> {
    const ORDER: [CommandId; 6] = [
        CommandId::BaseToggleActive,
        CommandId::Close,
        CommandId::BasesOpenMenu,
        CommandId::BasesView,
        CommandId::BaseShowOnly,
        CommandId::Help,
    ];
    let g = &app.theme.glyphs;
    let verbs: Vec<(String, String)> = ORDER
        .iter()
        .filter_map(|&id| {
            let found = command::find(id);
            if (found.available)(app) == Availability::Hidden {
                return None;
            }
            let key = command::keys_in(Context::Bases, found).first().copied()?;
            Some((
                key.label_in(g),
                command::hint_title(id, found.title, app).to_string(),
            ))
        })
        .collect();
    // What `key_line` spends on a pair: a space each side of the key, two
    // after the words.
    let cost = |(key, what): &(String, String)| key.width() + what.width() + 4;
    let mut pairs: Vec<(String, String)> = Vec::new();
    if let Some(movement) =
        command::movement_pair(Context::Bases, g).map(|(keys, what)| (keys, what.to_string()))
        && cost(&movement) + verbs.iter().take(3).map(cost).sum::<usize>() <= width
    {
        pairs.push(movement);
    }
    pairs.extend(verbs);
    pairs
}

/// The line over the rows: which view is on, and what it leaves out.
fn showing(app: &App) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let alone = app
        .known_bases()
        .and_then(|bases| bases.iter().find(|base| app.shown_alone(base)));
    let mut spans = vec![Span::styled(" showing ", theme.dim())];
    match (alone, app.library.view) {
        (Some(base), _) => spans.push(Span::styled(format!("only {}", base.label), theme.accent())),
        (None, BasesView::Every) => spans.push(Span::styled("every base", theme.accent())),
        (None, BasesView::Active) => {
            spans.push(Span::styled("the active bases", theme.accent()));
            let out = app
                .known_bases()
                .map(|bases| bases.iter().filter(|b| !app.base_is_active(b)).count())
                .unwrap_or(0);
            if out > 0 {
                spans.push(Span::styled(
                    format!("   {out} inactive left out"),
                    theme.dim(),
                ));
            }
        }
    }
    spans
}

/// The figure beside a base: its projects, or why there are none to count —
/// `unresponsive` for a base the last discovery did not hear from, whatever
/// its probe said a moment before.
pub(crate) fn base_note_text(app: &App, base: &BaseInfo) -> String {
    if app.library.silent.contains(&base.configured) {
        return crate::util::paths::Probe::Unresponsive
            .note()
            .trim()
            .trim_matches(['(', ')'])
            .to_string();
    }
    base.note()
}
