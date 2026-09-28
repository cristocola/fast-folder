//! Whether a command can run now: each `Availability` is a function of the
//! app.

use super::*;

pub(super) fn always(_: &App) -> Availability {
    Availability::Enabled
}

/// `←` is bound only while the pane has the focus: on the list there is
/// nothing to its left, and a key that does nothing should not be in the help
/// saying it does. **The axis never quits** — leaving a tab is Esc's ladder.
pub(super) fn pane_has_focus(app: &App) -> Availability {
    if app.focus == Focus::Detail {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// `→` is bound only while there is a pane to go to and the cursor is not
/// already in it. The library's pane is always one key away while it is
/// switched on — beside the list, under it, or in its place — and with `i`
/// it is off, and then the key is unbound rather than a no-op on the bar.
pub(super) fn pane_can_take_focus(app: &App) -> Availability {
    if app.focus == Focus::Projects && app.pane_present() {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// Alt-Enter breaks a line, and only a quick note has lines to break: in a
/// one-line prompt or the first-run question the key is not bound at all.
pub(super) fn in_a_note(app: &App) -> Availability {
    if matches!(
        app.modals.top(),
        Some(crate::tui::app::modal::Modal::Note(_))
    ) {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// Space ticks a row, and only a picker that takes several has rows to tick.
pub(super) fn in_a_multi_pick(app: &App) -> Availability {
    if matches!(
        app.modals.top(),
        Some(crate::tui::app::modal::Modal::MultiPick(_))
    ) {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// Job control is a unix thing; on Windows the key is not bound at all.
pub(super) fn unix_only(_: &App) -> Availability {
    if exists_here(CommandId::Suspend) {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// Whether this platform has the command at all, whatever the app's state —
/// what the help overlay, which has no app to ask, must know. Job control is
/// the one command a platform lacks.
pub fn exists_here(id: CommandId) -> bool {
    cfg!(unix) || id != CommandId::Suspend
}

/// A verb that starts a window — the file manager, a terminal — needs a
/// desktop to start it on. Over ssh or on a console there is none, and the
/// key says so instead of pretending.
pub(super) fn needs_selection_and_display(app: &App) -> Availability {
    if !app.has_display {
        return Availability::Disabled("no display — needs a desktop session; y copies the path");
    }
    needs_selection(app)
}

pub(super) fn needs_selection(app: &App) -> Availability {
    if app.library.selected().is_some() {
        Availability::Enabled
    } else if app.library.loaded {
        Availability::Disabled("no project selected")
    } else {
        Availability::Disabled("still loading the library")
    }
}

/// **What every batching verb is available on.** Marks are kept by path and
/// survive a filter change, so a marked row can be off screen while the verb is
/// aimed at it — `targets()` intersects the two and comes back empty. Every one
/// of these verbs then hit an early return with no picker, no dialog and no
/// message, which is what "batch tagging does nothing" was. It is deliberately
/// not part of `needs_selection`: `o`, `t` and `y` act on the row under the
/// cursor and are none of a hidden mark's business.
pub(super) fn batch_target(app: &App) -> Availability {
    if !app.library.marks.is_empty() && app.library.targets().is_empty() {
        return Availability::Disabled(
            "every marked row is hidden — F clears the filters, - clears the marks",
        );
    }
    selection_and_not_busy(app)
}

pub(super) fn not_busy(app: &App) -> Availability {
    if app.busy.is_some() || app.job.is_some() {
        Availability::Disabled("working…")
    } else if app.background.starting.is_some() {
        Availability::Disabled("starting a job…")
    } else if app.background.lock_holder().is_some() {
        // A job holds the library while it copies; every change would only
        // wait for it. Browsing and reading go on.
        Availability::Disabled("a job holds the library until it has copied — L shows it")
    } else {
        Availability::Enabled
    }
}

/// Reconcile, unless one is running already: a second would only find the
/// first's work claimed and do nothing.
pub(super) fn no_reconcile_running(app: &App) -> Availability {
    let running = app
        .background
        .live()
        .any(|job| job.kind() == Some(crate::core::jobs::JobKind::Reconcile));
    if running {
        Availability::Disabled("a reconcile is running — L shows it")
    } else {
        not_busy(app)
    }
}

/// Enter in the pane: only over a row it can act on, and not while a write
/// is in flight.
pub(super) fn pane_row_and_not_busy(app: &App) -> Availability {
    match selection_and_not_busy(app) {
        Availability::Enabled => {
            if app
                .pane_rows()
                .get(app.pane_cursor)
                .is_some_and(|row| row.selectable())
            {
                Availability::Enabled
            } else {
                Availability::Hidden
            }
        }
        other => other,
    }
}

/// F2 opens text in place: on a row that holds some — the name, a tag, a
/// variable, a note, a todo. Hidden on a rule, a heading, an add row, the
/// folder listing: there is nothing there to type over.
pub(super) fn pane_text_row(app: &App) -> Availability {
    use crate::tui::app::pane::PaneRow;
    match selection_and_not_busy(app) {
        Availability::Enabled => match app.pane_rows().get(app.pane_cursor) {
            // The name is the rename, with the rename's own rule about marks.
            Some(PaneRow::Name(_)) => single_and_not_busy(app),
            Some(
                PaneRow::Tag(_)
                | PaneRow::Variable { .. }
                | PaneRow::Note { .. }
                | PaneRow::Todo { .. },
            ) => Availability::Enabled,
            _ => Availability::Hidden,
        },
        other => other,
    }
}

/// The pane's line editor is open and not yet sent: Enter keeps. Hidden in
/// the note editor, where Enter is a new line and `Ctrl-S` is the keep.
pub(super) fn pane_line_editing(app: &App) -> Availability {
    match &app.pane_edit {
        Some(edit) if edit.is_note() => Availability::Hidden,
        Some(edit) if edit.pending() => Availability::Disabled("writing…"),
        Some(_) => Availability::Enabled,
        None => Availability::Hidden,
    }
}

/// The pane's note editor is open and not yet sent: `Ctrl-S` saves.
pub(super) fn pane_note_editing(app: &App) -> Availability {
    match &app.pane_edit {
        Some(edit) if !edit.is_note() => Availability::Hidden,
        Some(edit) if edit.pending() => Availability::Disabled("writing…"),
        Some(_) => Availability::Enabled,
        None => Availability::Hidden,
    }
}

pub(super) fn selection_and_not_busy(app: &App) -> Availability {
    match not_busy(app) {
        Availability::Enabled => needs_selection(app),
        other => other,
    }
}

/// A verb that cannot batch: rename, where every row would need its own name.
pub(super) fn single_and_not_busy(app: &App) -> Availability {
    if !app.library.marks.is_empty() {
        return Availability::Disabled("one folder at a time — clear the marks (-) to rename");
    }
    selection_and_not_busy(app)
}

/// A todo goes to one list: marks would make "which one" a guess.
pub(super) fn one_project(app: &App) -> Availability {
    if !app.library.marks.is_empty() {
        return Availability::Disabled("one project at a time — clear the marks (-) first");
    }
    selection_and_not_busy(app)
}

pub(super) fn has_search(app: &App) -> Availability {
    if app.search.input.is_empty() {
        Availability::Hidden
    } else {
        Availability::Enabled
    }
}

pub(super) fn has_row_filter(app: &App) -> Availability {
    if app.library.template_filter.is_some() || app.library.base_filter.is_some() {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// A base filter is worth offering only where there is more than one base to
/// choose between — with one, every row answers it already.
pub(super) fn many_bases(app: &App) -> Availability {
    match app
        .summary
        .as_ref()
        .and_then(crate::tui::app::data::Summary::bases_known)
    {
        Some(bases) if bases.len() > 1 => Availability::Enabled,
        Some(_) => Availability::Hidden,
        // The summary is still being read; the key is bound, and pressing it
        // before the bases are known says so rather than doing nothing.
        None => Availability::Disabled("still reading the bases"),
    }
}

pub(super) fn has_any_rows(app: &App) -> Availability {
    if app.library.is_empty() {
        Availability::Disabled("no projects")
    } else {
        Availability::Enabled
    }
}

/// `v` reaches from the last row Space touched to the cursor, so it needs one
/// — and needs it to still be on the list the filter is showing.
/// A tag filter needs a tag to filter by.
pub(super) fn has_any_tags(app: &App) -> Availability {
    if app.library.known_tags.is_empty() {
        Availability::Hidden
    } else {
        Availability::Enabled
    }
}

pub(super) fn has_anchor(app: &App) -> Availability {
    if app.library.has_anchor() {
        Availability::Enabled
    } else {
        Availability::Disabled("mark a row with Space first, then move and press this")
    }
}

pub(super) fn has_marks(app: &App) -> Availability {
    if app.library.marks.is_empty() {
        Availability::Hidden
    } else {
        Availability::Enabled
    }
}

/// The templates tab's verbs need a template selected.
pub(super) fn has_studio_selection(app: &App) -> Availability {
    if app.studio.selected_slug().is_some() {
        Availability::Enabled
    } else {
        Availability::Disabled(NO_TEMPLATES)
    }
}

/// `a` and `d` belong to the builder's variables and files lists; `K`/`J`
/// reorder the variables only. On the section list they are not bound.
pub(super) fn builder_list_open(app: &App) -> Availability {
    use crate::tui::app::studio::Open;
    match app.modals.top() {
        Some(crate::tui::app::modal::Modal::Builder(builder))
            if matches!(
                builder.open,
                Some(Open::Variables(_)) | Some(Open::Files(_))
            ) =>
        {
            Availability::Enabled
        }
        _ => Availability::Hidden,
    }
}

/// `s` saves, and belongs to the section list alone.
///
/// Every other face of the builder is somewhere a letter is text: a form
/// field, a path line, a folder list, a file's contents. The section list is
/// the one place with nothing to type into, which is what lets a bare letter
/// mean a verb there — the same bargain `a`, `d`, `K` and `J` already make on
/// the lists inside it.
pub(super) fn builder_list_closed(app: &App) -> Availability {
    match app.modals.top() {
        Some(crate::tui::app::modal::Modal::Builder(builder))
            if builder.open.is_none() && builder.pending.is_none() && !builder.saving =>
        {
            Availability::Enabled
        }
        _ => Availability::Hidden,
    }
}

/// F2 in the builder opens what holds text: a part of the template on the
/// section list (never Save or Discard, which are verbs), or the highlighted
/// variable or file on its list. Hidden inside a form or an editor, where the
/// key is the field's to ignore.
pub(super) fn builder_text_row(app: &App) -> Availability {
    use crate::tui::app::studio::{Open, Row};
    let Some(crate::tui::app::modal::Modal::Builder(builder)) = app.modals.top() else {
        return Availability::Hidden;
    };
    if builder.pending.is_some() || builder.saving {
        return Availability::Hidden;
    }
    let text = match &builder.open {
        None => matches!(builder.row(), Row::Section(_)),
        Some(Open::Variables(list)) => {
            list.editing.is_none() && !builder.template.variables.is_empty()
        }
        Some(Open::Files(list)) => list.editing.is_none() && !builder.template.files.is_empty(),
        Some(_) => false,
    };
    if text {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// F2 in the settings opens a value that is text — the rows Enter would open
/// on their line. A yes/no, a choice and a maintenance verb have nothing to
/// type, and are Enter's.
pub(super) fn settings_text_row(app: &App) -> Availability {
    use crate::tui::app::settings::Kind;
    match app.modals.top() {
        Some(crate::tui::app::modal::Modal::Settings(state))
            if state.editing.is_none()
                && !state.pending
                && state
                    .row()
                    .is_some_and(|row| matches!(row.kind, Kind::Text(_) | Kind::Bases)) =>
        {
            Availability::Enabled
        }
        _ => Availability::Hidden,
    }
}

pub(super) fn builder_variables_open(app: &App) -> Availability {
    use crate::tui::app::studio::Open;
    match app.modals.top() {
        Some(crate::tui::app::modal::Modal::Builder(builder))
            if matches!(builder.open, Some(Open::Variables(_))) =>
        {
            Availability::Enabled
        }
        _ => Availability::Hidden,
    }
}

/// Move needs a mounted base to move to that is not the one the project is
/// in. With one base it is listed dimmed with the reason rather than hidden:
/// a person with one drive should still learn that a second one is a key
/// away, and pressing `m` should say why nothing happened.
pub(super) fn can_move(app: &App) -> Availability {
    if let Availability::Disabled(reason) = batch_target(app)
        && !app.library.marks.is_empty()
    {
        return Availability::Disabled(reason);
    }
    let Some(project) = app.library.selected() else {
        return Availability::Hidden;
    };
    let Some(bases) = app
        .summary
        .as_ref()
        .and_then(crate::tui::app::data::Summary::bases_known)
    else {
        return Availability::Disabled("still probing the bases");
    };
    if bases
        .iter()
        .any(|base| base.probe.usable() && base.path != project.base)
    {
        not_busy(app)
    } else if bases.len() > 1 {
        Availability::Disabled("no other base is mounted right now")
    } else {
        Availability::Disabled("only one base is configured — add another under Settings")
    }
}
