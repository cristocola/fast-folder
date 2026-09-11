//! What sits on top of the dashboard: the palette, help, a picker, a message.
//!
//! A stack, so a picker opened from the palette returns to the palette. Esc
//! pops one. What a picker's answer *means* is data (`Then`), not a closure,
//! so `update` stays inspectable.

use std::path::PathBuf;

use ratatui::crossterm::event::KeyCode;

use super::App;
use crate::tui::app::actions::{ActionsState, Confirm, MultiPick, NoteState, TextPrompt, TextThen};
use crate::tui::app::jobs;
use crate::tui::app::library::Sort;
use crate::tui::app::palette::PaletteState;
use crate::tui::app::pane;
use crate::tui::app::settings::{Onboarding, SettingsState};
use crate::tui::app::studio::{Builder, Open};
use crate::tui::app::wizard::Flow;
use crate::tui::command::{self, Context, Key};
use crate::tui::effect::{Action, Effect};
use crate::tui::fuzzy::Fuzzy;
use crate::tui::layout;
use crate::tui::validators;
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLevel {
    Info,
    Warn,
    Error,
}

/// What choosing a `Pick` item does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Then {
    SortPick,
    TemplateFilter,
    /// The picked value is a tag to add, or `NEW_TAG` to type one.
    AddTag,
    /// The picked value is a base path to move into.
    MoveToBase,
    /// The picked value is a base path to restrict the list to.
    BaseFilter,
    /// The picked value is a tag; it goes into the search bar as `tag:x`,
    /// because that is what a tag filter *is* here — the grammar already had
    /// it, and this is a way to find it without typing it.
    TagFilter,
    /// The picked value answers the named field of the open flow's form —
    /// what Space on a choice opens, so a twenty-template list is one fuzzy
    /// search rather than twenty presses of `→`.
    FormField(String),
    /// The picked value is the new value of the named variable of the
    /// selected project — a `select` variable edited from the detail pane,
    /// which offers its options and nothing else.
    PaneVariable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickItem {
    pub label: String,
    pub detail: String,
    /// What the item stands for (a sort name, a template slug).
    pub value: String,
}

/// A fuzzy-filtered list to choose one entry from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickState {
    pub title: String,
    pub items: Vec<PickItem>,
    pub query: LineEdit,
    /// Item indices in ranked order, with the characters the query hit.
    pub ranked: Vec<(usize, Vec<u32>)>,
    pub selected: Option<usize>,
    pub offset: usize,
    pub then: Then,
}

impl PickState {
    pub fn new(title: impl Into<String>, items: Vec<PickItem>, then: Then) -> Self {
        let ranked = (0..items.len()).map(|i| (i, Vec::new())).collect();
        Self {
            title: title.into(),
            selected: (!items.is_empty()).then_some(0),
            items,
            query: LineEdit::new(),
            ranked,
            offset: 0,
            then,
        }
    }

    pub fn rank(&mut self, fuzzy: &mut Fuzzy) {
        let candidates: Vec<(usize, String)> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| (i, format!("{} {}", item.label, item.detail)))
            .collect();
        self.ranked = fuzzy
            .rank(self.query.text(), candidates)
            .into_iter()
            .map(|(i, hit)| (i, hit.indices))
            .collect();
        self.selected = (!self.ranked.is_empty()).then_some(0);
        self.offset = 0;
    }

    pub fn chosen(&self) -> Option<&PickItem> {
        self.selected
            .and_then(|at| self.ranked.get(at))
            .and_then(|(index, _)| self.items.get(*index))
    }

    pub fn step(&mut self, delta: isize) {
        self.selected = nav::step(self.selected, self.ranked.len(), delta);
    }

    pub fn clamp_viewport(&mut self, rows: usize) {
        self.offset = nav::viewport_offset(self.offset, self.selected, self.ranked.len(), rows);
    }
}

/// The template guide: which page, and how far down it.
///
/// The one surface in the app that teaches rather than does. Its words live in
/// [`crate::tui::guide`]; this is only where the reader is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideState {
    pub page: usize,
    pub scroll: usize,
}

impl GuideState {
    pub fn at(page: usize) -> Self {
        Self {
            page: page.min(crate::tui::guide::PAGES.len().saturating_sub(1)),
            scroll: 0,
        }
    }

    /// Turn `delta` pages, stopping at either end rather than wrapping — a
    /// document has a beginning and an end, and wrapping past the last page
    /// back to the first reads as having lost your place.
    pub fn turn(&mut self, delta: isize) -> bool {
        let last = crate::tui::guide::PAGES.len().saturating_sub(1);
        let next = (self.page as isize + delta).clamp(0, last as isize) as usize;
        let moved = next != self.page;
        if moved {
            self.page = next;
            self.scroll = 0;
        }
        moved
    }

    pub fn is_last(&self) -> bool {
        self.page + 1 >= crate::tui::guide::PAGES.len()
    }
}

#[derive(Debug)]
pub enum Modal {
    Palette(PaletteState),
    Help {
        ctx: Context,
        scroll: usize,
    },
    Pick(PickState),
    /// The selected project's action menu.
    Actions(ActionsState),
    /// A single-line prompt (rename, add a tag, the delete confirmation).
    TextPrompt(TextPrompt),
    /// The quick journal note: a few lines, Enter saves.
    Note(NoteState),

    /// A yes/no question; a bare `y`/`n` answers.
    Confirm(Confirm),
    /// A list where Space toggles and Enter confirms the picked set.
    MultiPick(MultiPick),
    /// A flow that builds something: create, apply, register, from-folder.
    Flow(Box<Flow>),
    /// Every template, with the selected one's details.
    /// A template being written.
    Builder(Box<Builder>),
    /// Every setting, the ID counter and maintenance.
    Settings(Box<SettingsState>),
    /// First run: where should projects live?
    Onboarding(Onboarding),
    /// How templates work, and a walkthrough that builds one.
    Guide(Box<GuideState>),
    Message {
        title: String,
        lines: Vec<String>,
        level: MessageLevel,
        scroll: usize,
    },
}

impl Modal {
    pub fn message(title: impl Into<String>, body: impl Into<String>, level: MessageLevel) -> Self {
        Modal::Message {
            title: title.into(),
            lines: body.into().lines().map(str::to_string).collect(),
            level,
            scroll: 0,
        }
    }

    /// Which key context this modal answers in. A screen with a text field
    /// or a form open is `Modal` — the widget has the keys — and its own
    /// context only while a list has them.
    pub fn context(&self) -> Context {
        use crate::tui::app::studio::Open;
        match self {
            Modal::Palette(_) => Context::Palette,
            Modal::Actions(_) => Context::Actions,
            Modal::Builder(builder) if builder.pending.is_none() => match &builder.open {
                None => Context::Builder,
                Some(Open::Variables(list)) if list.editing.is_none() => Context::Builder,
                Some(Open::Files(list)) if list.editing.is_none() => Context::Builder,
                Some(_) => Context::Modal,
            },
            Modal::Settings(state) if state.editing.is_none() => Context::Settings,
            Modal::Guide(_) => Context::Guide,
            Modal::TextPrompt(_) | Modal::Note(_) | Modal::Onboarding(_) => Context::Prompt,
            Modal::Pick(_) | Modal::MultiPick(_) => Context::Pick,
            _ => Context::Modal,
        }
    }
}

#[derive(Debug, Default)]
pub struct ModalStack(Vec<Modal>);

impl ModalStack {
    pub fn push(&mut self, modal: Modal) {
        self.0.push(modal);
    }

    pub fn pop(&mut self) -> Option<Modal> {
        self.0.pop()
    }

    pub fn top(&self) -> Option<&Modal> {
        self.0.last()
    }

    pub fn top_mut(&mut self) -> Option<&mut Modal> {
        self.0.last_mut()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl App {
    /// Everything printable is the query, exactly as in the palette; the rows
    /// and the way out are commands.
    pub(super) fn on_pick_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Pick, key, self)
        {
            return self.run(id);
        }
        if let Some(Modal::Pick(pick)) = self.modals.top_mut()
            && pick.query.apply(&key)
        {
            pick.rank(&mut self.fuzzy);
        }
        Vec::new()
    }

    /// Move a picker's cursor, whichever kind it is.
    pub(super) fn step_pick(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        match self.modals.top_mut() {
            Some(Modal::Pick(pick)) => {
                pick.step(delta);
                pick.clamp_viewport(layout::list_rows(
                    layout::pick_box(area, pick.ranked.len()),
                    2,
                ));
            }
            // A multi-pick has no viewport of its own to keep — the box is
            // sized to its rows — so it is `step_top_modal`'s arm verbatim.
            Some(Modal::MultiPick(_)) => return self.step_top_modal(delta),
            _ => {}
        }
        Vec::new()
    }

    /// Take the row under a single picker's cursor and do what it was opened
    /// for.
    pub(super) fn choose_pick(&mut self) -> Vec<Effect> {
        let Some(Modal::Pick(pick)) = self.modals.pop() else {
            return Vec::new();
        };
        let Some(item) = pick.chosen().cloned() else {
            return Vec::new();
        };
        match pick.then {
            Then::SortPick => {
                self.library.explicit_sort = Sort::from_label(&item.value);
                self.reordered()
            }
            Then::TemplateFilter => self.set_template_filter(Some(item.value.clone())),
            // A tag filter is `tag:x` in the bar and nothing else — one
            // mechanism, so clearing it is the same Esc rung as clearing any
            // other query and the bar goes on reporting what is filtering the
            // list.
            Then::TagFilter => {
                self.search.input.set_text(format!("tag:{}", item.value));
                self.search.sync();
                let effects = self.after_query_change();
                self.pulse_selected();
                effects
            }
            Then::BaseFilter => {
                let base = (!item.value.is_empty()).then(|| PathBuf::from(&item.value));
                self.set_base_filter(base)
            }
            Then::AddTag => {
                if item.value == crate::tui::app::actions::NEW_TAG {
                    self.modals.push(Modal::TextPrompt(TextPrompt::new(
                        validators::ADD_TAG_PROMPT,
                        TextThen::AddTag,
                    )));
                    return Vec::new();
                }
                self.add_tag(item.value.clone())
            }
            Then::MoveToBase => {
                let target = PathBuf::from(item.value.clone());
                // `batching()`, not `!marks.is_empty()`: marks are kept
                // by path and survive a filter change, so a marked row
                // can be off screen while the verb is aimed at it. Every
                // other verb asks this question the same way — the raw
                // mark set is what "batch tagging does nothing" was, and
                // this was the last caller still asking it.
                if self.batching() {
                    self.start_job(jobs::JobKind::Move, Some(target))
                } else {
                    self.run_move(target)
                }
            }
            Then::PaneVariable(slug) => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                // The picker was the edit; there is no line to keep open, so
                // the cursor's row is remembered for the pulse by the answer.
                self.pane_edit = Some(pane::PaneEdit::Line {
                    row: self.pane_cursor,
                    target: pane::EditTarget::Variable(slug.clone()),
                    input: crate::tui::widgets::input::LineEdit::with_text(item.value.clone()),
                    error: None,
                    pending: false,
                });
                self.send_pane_edit(Action::SetVariable {
                    project: Box::new(project),
                    slug,
                    value: item.value.clone(),
                })
            }
            Then::FormField(key) => {
                let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
                    return Vec::new();
                };
                let Some(field) = flow.form.field_mut(&key) else {
                    return Vec::new();
                };
                if !field.select(&item.value) {
                    return Vec::new();
                }
                flow.form.selected = flow
                    .form
                    .fields
                    .iter()
                    .position(|field| field.key == key)
                    .unwrap_or(flow.form.selected);
                self.on_form_changed()
            }
        }
    }

    /// Help and a message: a pager. Enter and `?` close it too — `?` because
    /// the key that opened it should put it away, and it must not open a
    /// second help over the first.
    pub(super) fn on_scroll_modal_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter | KeyCode::Char('?') if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
            KeyCode::Char(' ') if !key.ctrl => self.scroll_top_modal(self.page_rows() as isize),
            _ => self.lookup_and_run(key),
        }
    }

    /// The guide's own keys.
    ///
    /// A reader owns its own left and right, which is why the guide has a
    /// `Context` to itself: `←` turns a page here and backs out of a dialog
    /// everywhere else, and both can be declared. Space is the pager's own key
    /// and stays here — `PageDown` cannot carry it, because Space is the mark
    /// on the list `PageDown` also answers on.
    pub(super) fn on_guide_key(&mut self, key: Key) -> Vec<Effect> {
        if key == Key::ch(' ') {
            return self.scroll_top_modal(self.page_rows() as isize);
        }
        self.lookup_and_run(key)
    }

    /// Turn a page of the guide. Forward from the last page leaves it: a
    /// reader who keeps pressing the same key reaches the end and is let go,
    /// rather than pressing it against the last page.
    pub(super) fn turn_guide(&mut self, delta: isize) -> Vec<Effect> {
        let last = matches!(self.modals.top(), Some(Modal::Guide(state)) if state.is_last());
        if delta > 0 && last {
            self.modals.pop();
            return Vec::new();
        }
        if let Some(Modal::Guide(state)) = self.modals.top_mut() {
            state.turn(delta);
        }
        Vec::new()
    }

    /// Scroll whatever dialog is on top by `delta` rows, clamped to its
    /// content. `isize::MIN` and `isize::MAX` are the ends.
    fn scroll_top_modal(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        let Some(top) = self.modals.top_mut() else {
            return Vec::new();
        };
        let (scroll, lines, rows) = match top {
            Modal::Help { ctx, scroll } => {
                let inner = layout::help_box(area);
                (
                    scroll,
                    command::help_line_count(*ctx, inner.width.saturating_sub(2) as usize),
                    inner.height.saturating_sub(2) as usize,
                )
            }

            Modal::Guide(state) => {
                let box_ = layout::guide_box(area);
                (
                    &mut state.scroll,
                    crate::tui::view::modals::guide_rows(state.page, box_),
                    // The border, the footer and the key line: what
                    // `view::builder::frame_parts` takes off the top and the
                    // bottom before the body is drawn.
                    box_.height.saturating_sub(4) as usize,
                )
            }
            Modal::Message { lines, scroll, .. } => (
                scroll,
                // Wrapped rows, not entries — the paragraph wraps, and
                // `Modal::Help` above has always counted them the same way.
                crate::tui::view::modals::message_rows(
                    lines,
                    crate::tui::view::modals::message_text_width(area),
                ),
                layout::message_box(area).height.saturating_sub(2) as usize,
            ),
            _ => return Vec::new(),
        };
        let max = lines.saturating_sub(rows);
        *scroll = scroll.saturating_add_signed(delta).min(max);
        Vec::new()
    }

    /// The arrows on whatever dialog is on top.
    pub(super) fn step_top_modal(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        let actions_len = crate::tui::app::actions::action_entries(self).len();
        match self.modals.top_mut() {
            Some(Modal::Actions(actions)) => {
                actions.step(actions_len, delta);
                actions.clamp_viewport(
                    actions_len,
                    layout::list_rows(layout::actions_box(area, actions_len), 0),
                );
                Vec::new()
            }
            Some(Modal::MultiPick(pick)) => {
                pick.selected =
                    crate::tui::widgets::nav::step(Some(pick.selected), pick.items.len(), delta)
                        .unwrap_or(0);
                Vec::new()
            }
            Some(Modal::Builder(builder)) => {
                match &mut builder.open {
                    None => builder.step(delta),
                    Some(Open::Variables(list)) => {
                        let count = builder.template.variables.len();
                        list.selected =
                            crate::tui::widgets::nav::step(Some(list.selected), count, delta)
                                .unwrap_or(0);
                    }
                    Some(Open::Files(list)) => {
                        let count = builder.template.files.len();
                        list.selected =
                            crate::tui::widgets::nav::step(Some(list.selected), count, delta)
                                .unwrap_or(0);
                    }
                    Some(_) => {}
                }
                Vec::new()
            }
            Some(Modal::Settings(state)) => {
                state.step(delta);
                state.clamp_viewport(layout::settings_rows(area));
                Vec::new()
            }
            Some(Modal::Help { .. }) | Some(Modal::Message { .. }) => self.scroll_top_modal(delta),
            _ => Vec::new(),
        }
    }

    /// A page, or the ends (`isize::MIN`/`isize::MAX`), on whatever dialog
    /// is on top.
    pub(super) fn page_top_modal(&mut self, delta: isize) -> Vec<Effect> {
        let area = self.area();
        let actions_len = crate::tui::app::actions::action_entries(self).len();
        let jump = |selected: usize, len: usize| -> usize {
            crate::tui::widgets::nav::step(Some(selected), len, delta).unwrap_or(0)
        };
        match self.modals.top_mut() {
            Some(Modal::Actions(actions)) => {
                actions.selected = jump(actions.selected, actions_len);
                actions.clamp_viewport(
                    actions_len,
                    layout::list_rows(layout::actions_box(area, actions_len), 0),
                );
                Vec::new()
            }
            Some(Modal::MultiPick(pick)) => {
                pick.selected = jump(pick.selected, pick.items.len());
                Vec::new()
            }
            Some(Modal::Settings(state)) => {
                state.jump(delta);
                state.clamp_viewport(layout::settings_rows(area));
                Vec::new()
            }
            Some(Modal::Help { .. }) | Some(Modal::Message { .. }) | Some(Modal::Guide(_)) => {
                self.scroll_top_modal(delta)
            }
            _ => Vec::new(),
        }
    }

    /// Put the guide up, at `page`, and remember that it has been seen.
    ///
    /// Every door goes through here — the key, the palette and both automatic
    /// offers — so the flag is set wherever the guide is actually read, and a
    /// person who found it themselves is never offered it afterwards.
    pub(super) fn open_guide(&mut self, page: usize) -> Vec<Effect> {
        self.guide_seen = true;
        self.modals
            .push(Modal::Guide(Box::new(GuideState::at(page))));
        Vec::new()
    }

    /// The guide, offered once, the first time templates come up at all.
    ///
    /// It goes **on top of** whatever asked for it rather than instead of it,
    /// so Esc leaves the reader exactly where they were going — on the tab, or
    /// in the editor with an untouched template under the dialog.
    pub(super) fn offer_guide_once(&mut self) {
        if !self.guide_seen {
            self.open_guide(0);
        }
    }
}
