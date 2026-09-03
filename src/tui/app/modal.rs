//! What sits on top of the dashboard: the palette, help, a picker, a message.
//!
//! A stack, so a picker opened from the palette returns to the palette. Esc
//! pops one. What a picker's answer *means* is data (`Then`), not a closure,
//! so `update` stays inspectable.

use crate::tui::app::actions::{ActionsState, Confirm, MultiPick, TextPrompt};
use crate::tui::app::palette::PaletteState;
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
    /// A single-line prompt (rename, add a tag, a quick note, delete confirm).
    TextPrompt(TextPrompt),
    /// A yes/no question; a bare `y`/`n` answers.
    Confirm(Confirm),
    /// A list where Space toggles and Enter confirms the picked set.
    MultiPick(MultiPick),
    /// A flow that builds something: create, apply, register.
    Flow(Box<Flow>),
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

    /// Which key context this modal answers in.
    pub fn context(&self) -> Context {
        match self {
            Modal::Palette(_) => Context::Palette,
            Modal::Actions(_) => Context::Actions,
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
