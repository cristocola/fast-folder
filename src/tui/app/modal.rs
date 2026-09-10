//! What sits on top of the dashboard: the palette, help, a picker, a message.
//!
//! A stack, so a picker opened from the palette returns to the palette. Esc
//! pops one. What a picker's answer *means* is data (`Then`), not a closure,
//! so `update` stays inspectable.

use crate::tui::app::actions::{ActionsState, Confirm, MultiPick, NoteState, TextPrompt};
use crate::tui::app::palette::PaletteState;
use crate::tui::app::settings::{Onboarding, SettingsState};
use crate::tui::app::studio::Builder;
use crate::tui::app::wizard::Flow;
use crate::tui::command::Context;
use crate::tui::fuzzy::Fuzzy;
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
        self.selected = nav::wrap_step(self.selected, self.ranked.len(), delta);
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
