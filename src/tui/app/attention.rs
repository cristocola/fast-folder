//! `!`: what fastf left unfinished, and what settles it.
//!
//! The header counts two things: what **needs you** — a warning, with the key
//! — and what fastf is **finishing** by itself, dimmed, since nobody has to
//! act on it. The page lists every item with its reason; Enter on one that
//! needs you offers what settles it, and discarding asks for the word.
//!
//! **fastf finishes its own leftovers.** When the header's summary shows
//! something it can finish and nothing else is running, the app starts a
//! reconcile of its own — quietly: no dialog, and a word on the status line
//! only when it found something that needs you. It looks again after every
//! job, and every five minutes while something waits for a base.

use std::path::PathBuf;

use super::App;
use crate::core::attention::{Action as Resolution, State};
use crate::tui::app::actions::{TextPrompt, TextThen};
use crate::tui::app::modal::{Modal, PickItem, PickState, Then};
use crate::tui::effect::{Action, Effect};

/// The word that confirms a discard, typed, like `delete`.
pub const DISCARD_WORD: &str = "discard";
/// What a prompt says when the word did not match.
pub const DISCARD_MISMATCH: &str = "type the word discard to confirm — nothing was discarded";
/// The page's first row: finish now what fastf can.
pub const ATTENTION_FINISH: &str = "\u{0}finish";

/// How long after one automatic reconcile the app starts another, at the
/// soonest.
pub const AUTO_EVERY_MS: u64 = 60_000;
/// How often the app looks again while something waits for a base.
pub const WAITING_EVERY_MS: u64 = 5 * 60_000;

impl App {
    /// `!`: the page. With nothing unfinished it says so instead.
    pub(super) fn open_attention(&mut self) -> Vec<Effect> {
        let Some(summary) = &self.summary else {
            self.info("still reading the bases…");
            return Vec::new();
        };
        let attention = &summary.attention;
        if attention.items.is_empty() {
            self.good("Nothing unfinished.");
            return Vec::new();
        }
        // What needs you first; then, when fastf has anything to finish, the
        // row that finishes it now; then what it is finishing.
        let row = |item: &crate::core::attention::Item| {
            let who = match item.state {
                State::NeedsYou => "needs you",
                State::Auto => "fastf finishes",
                State::Waiting => "waiting",
            };
            let about = match &item.project {
                Some(project) => format!("{} of {project}", item.what),
                None => format!(
                    "{} {}",
                    item.what,
                    item.path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| crate::util::paths::display_path(&item.path))
                ),
            };
            PickItem {
                label: format!("{who} · {about}"),
                detail: capitalised(&item.reason),
                value: item.path.display().to_string(),
            }
        };
        let mut items: Vec<PickItem> = attention
            .items
            .iter()
            .filter(|item| item.state == State::NeedsYou)
            .map(row)
            .collect();
        // A silent base alone is nothing a reconcile can finish.
        let finishing = attention.auto() + attention.waiting_work();
        if finishing > 0 {
            items.push(PickItem {
                label: "Finish now what fastf can".to_string(),
                detail: format!(
                    "Run a reconcile now rather than wait for the app to: {} for fastf to \
                     finish, {} waiting for a base to answer. What needs you stays on this \
                     list until you choose.",
                    attention.auto(),
                    attention.waiting_work()
                ),
                value: ATTENTION_FINISH.to_string(),
            });
        }
        items.extend(
            attention
                .items
                .iter()
                .filter(|item| item.state != State::NeedsYou)
                .map(row),
        );
        let title = match attention.needs_you() {
            0 => "Unfinished — fastf finishes it".to_string(),
            1 => "Unfinished — 1 needs you".to_string(),
            n => format!("Unfinished — {n} need you"),
        };
        self.modals.push(Modal::Pick(
            PickState::new(title, items, Then::Attention).wide(),
        ));
        Vec::new()
    }

    /// A row of the page, picked: what needs you offers what settles it;
    /// anything else is finished now.
    pub(super) fn on_attention_pick(&mut self, value: String) -> Vec<Effect> {
        let item = self.summary.as_ref().and_then(|summary| {
            summary
                .attention
                .items
                .iter()
                .find(|item| item.path.display().to_string() == value)
                .cloned()
        });
        match item {
            Some(item) if item.state == State::NeedsYou && !item.actions.is_empty() => {
                let items = item
                    .actions
                    .iter()
                    .map(|action| PickItem {
                        label: capitalised(action.label()),
                        detail: capitalised(&item.reason),
                        value: action.word().to_string(),
                    })
                    .collect();
                self.modals.push(Modal::Pick(
                    PickState::new(
                        format!(
                            "{} — what to do",
                            item.project.clone().unwrap_or_else(|| item.what.clone())
                        ),
                        items,
                        Then::AttentionAction(item.path.clone()),
                    )
                    .wide(),
                ));
                Vec::new()
            }
            _ => self.run_job(super::settings::Job::Reconcile),
        }
    }

    /// An action picked for the item at `path`.
    pub(super) fn on_attention_action(&mut self, path: PathBuf, word: String) -> Vec<Effect> {
        let Some(action) = Resolution::from_word(&word) else {
            return Vec::new();
        };
        match action {
            Resolution::Finish => self.run_job(super::settings::Job::Reconcile),
            Resolution::Discard => {
                // Named as the page named it, with where it is after.
                let named = self
                    .summary
                    .as_ref()
                    .and_then(|summary| {
                        summary
                            .attention
                            .items
                            .iter()
                            .find(|item| item.path == path)
                    })
                    .map(|item| match &item.project {
                        Some(project) => format!("{} of {project}", item.what),
                        None => item.what.clone(),
                    })
                    .unwrap_or_else(|| "it".to_string());
                self.modals.push(Modal::TextPrompt(TextPrompt::new(
                    format!(
                        "Discard {named} for good? Everything in it goes, at {}. Type \
                         {DISCARD_WORD} to confirm.",
                        crate::util::paths::display_path(&path)
                    ),
                    TextThen::DiscardAttention(path),
                )));
                Vec::new()
            }
            _ => self.resolve_attention(path, action),
        }
    }

    /// Settle the item at `path` as chosen, on a worker, under the lock.
    pub(super) fn resolve_attention(&mut self, path: PathBuf, action: Resolution) -> Vec<Effect> {
        self.run_action("settling…", Action::ResolveAttention { path, action })
    }

    /// The summary landed: start a reconcile of the app's own when fastf has
    /// something to finish, nothing else runs, and the last one was a while
    /// ago.
    pub(super) fn maybe_finish_leftovers(&mut self) -> Vec<Effect> {
        self.summary_at = Some(self.elapsed_ms);
        let Some(summary) = &self.summary else {
            return Vec::new();
        };
        let idle = self.background.live().next().is_none()
            && self.background.starting.is_none()
            && self.background.auto.is_none();
        let due = self
            .background
            .auto_at
            .is_none_or(|at| self.elapsed_ms.saturating_sub(at) >= AUTO_EVERY_MS);
        if summary.attention.auto() > 0 && idle && due {
            self.background.auto_at = Some(self.elapsed_ms);
            return vec![Effect::StartAutoReconcile];
        }
        Vec::new()
    }

    /// A tick: look again while something waits for a base.
    pub(super) fn look_again_while_waiting(&mut self) -> Vec<Effect> {
        let waiting = self
            .summary
            .as_ref()
            .is_some_and(|summary| summary.attention.waiting() > 0);
        let due = self
            .summary_at
            .is_some_and(|at| self.elapsed_ms.saturating_sub(at) >= WAITING_EVERY_MS);
        if waiting && due {
            self.summary_at = Some(self.elapsed_ms);
            return vec![Effect::LoadSummary];
        }
        Vec::new()
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
