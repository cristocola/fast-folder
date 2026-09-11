//! The single-project actions as native dialogs: the action-menu modal,
//! the text prompt, the yes/no confirm, the multi-pick for tags, and the pure
//! lookups that feed them. The verbs themselves are declared once in
//! `command.rs`; this module holds the modal state a verb opens and the lists
//! its pickers show, so `update` stays a function of data and not of closures.

use std::path::PathBuf;

use ratatui::crossterm::event::KeyCode;

use super::{App, Focus};
use crate::core::assets::Progress;
use crate::tui::app::jobs;
use crate::tui::app::modal::{MessageLevel, Modal, PickItem, PickState, Then};
use crate::tui::app::pane;
use crate::tui::app::settings;
use crate::tui::command::{self, Availability, CommandId, Context, Key};
use crate::tui::effect::{Action, Effect, ViewKind};
use crate::tui::validators;
use crate::tui::widgets::input::LineEdit;

/// The picker's "type your own" entry.
pub const NEW_TAG: &str = "New tag…";

/// The action menu: the selected project's verbs, chosen from the registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionsState {
    pub selected: usize,
    pub offset: usize,
}

impl ActionsState {
    pub fn step(&mut self, len: usize, delta: isize) {
        self.selected =
            crate::tui::widgets::nav::step(Some(self.selected), len, delta).unwrap_or(0);
    }

    pub fn clamp_viewport(&mut self, len: usize, rows: usize) {
        self.offset =
            crate::tui::widgets::nav::viewport_offset(self.offset, Some(self.selected), len, rows);
    }
}

/// What a text prompt's answer does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextThen {
    /// Rename the project the prompt named, carried by path.
    ///
    /// **The dialog carries its target rather than re-reading the selection.**
    /// The prompt text is built once from the row under the cursor, and the
    /// action used to be built from whatever was selected when Enter landed —
    /// so a discovery arriving under an open dialog, which moves the cursor
    /// when the named row is no longer in the snapshot, could point a
    /// destructive verb at a different project from the one on screen.
    Rename(std::path::PathBuf),
    AddTag,
    /// A todo for the selected project, from the pane's add row.
    AddTodo,
    /// Type the word `delete` to confirm; nothing else deletes. The prompt
    /// names the folder — or the folders, over marks — so what is being
    /// confirmed is on screen, and the word is the same every time.
    ///
    /// Carries the single project's path for the same reason `Rename` does;
    /// a batch delete goes by the marks, which are kept by path already.
    Delete(std::path::PathBuf),
    /// Raise the global ID counter to the number typed.
    RaiseCounter,
    /// The folder to copy into. Refused by the engine rather than here, so the
    /// command line and the app say the same words about the same rule.
    CopyTo,
}

/// The quick note: a few lines typed where you are. Enter saves, Alt-Enter
/// (or a pasted newline) breaks a line — so a pasted paragraph is one note,
/// and never a run of keystrokes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteState {
    pub area: crate::tui::widgets::text_area::TextArea,
    /// How many projects the note goes to: the marks, or the one selected.
    pub count: usize,
}

impl NoteState {
    pub fn new(count: usize) -> Self {
        Self {
            area: crate::tui::widgets::text_area::TextArea::new(),
            count,
        }
    }
}

/// A single-line prompt drawn over the dashboard. Esc cancels; Enter submits
/// the text and `update` interprets `then`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPrompt {
    pub title: String,
    pub input: LineEdit,
    /// A validation message shown under the line; the text stays for editing.
    pub error: Option<String>,
    pub then: TextThen,
}

impl TextPrompt {
    pub fn new(title: impl Into<String>, then: TextThen) -> Self {
        Self {
            title: title.into(),
            input: LineEdit::new(),
            error: None,
            then,
        }
    }
}

/// What a yes/no confirm answers. A bare `y`/`n` answers without Enter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmThen {
    /// Unregister the project the question named, carried by path — see
    /// [`TextThen::Rename`].
    Unregister(std::path::PathBuf),
    /// Delete the named template and everything bundled with it.
    DeleteTemplate(String),
    /// Leave the builder, throwing away a template that has been worked on.
    ///
    /// `then_quit` is set when the gesture was a quit rather than a close, so
    /// answering the question does what was asked instead of stopping one
    /// level short.
    DiscardTemplate {
        then_quit: Option<crate::tui::effect::Exit>,
    },
    /// Delete every marked project (the marks are the batch).
    DeleteBatch,
    /// Unregister every marked project (the marks are the batch).
    UnregisterBatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub prompt: String,
    pub then: ConfirmThen,
}

/// What a multi-pick answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiThen {
    RemoveTags,
}

/// A list where Space toggles and Enter confirms the picked set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiPick {
    pub title: String,
    pub items: Vec<String>,
    pub picked: Vec<bool>,
    pub selected: usize,
    pub then: MultiThen,
}

impl MultiPick {
    pub fn new(title: impl Into<String>, items: Vec<String>, then: MultiThen) -> Self {
        Self {
            title: title.into(),
            picked: vec![false; items.len()],
            items,
            selected: 0,
            then,
        }
    }

    pub fn chosen(&self) -> Vec<String> {
        self.items
            .iter()
            .zip(&self.picked)
            .filter(|(_, picked)| **picked)
            .map(|(item, _)| item.clone())
            .collect()
    }
}

/// The commands the action menu lists: every project verb that fires in the
/// `Actions` context and is not hidden, with its availability, in registry
/// order. Hidden means the verb makes no sense here (Move with no other
/// mounted base), so it is not listed at all. The menu's own navigation —
/// Enter, the arrows, Esc — fires there too and is not a row.
pub fn action_entries(app: &App) -> Vec<(CommandId, Availability)> {
    crate::tui::command::COMMANDS
        .iter()
        .filter(|command| {
            command.contexts.contains(&Context::Actions)
                && command.category == crate::tui::command::Category::Project
        })
        .map(|command| (command.id, (command.available)(app)))
        .filter(|(_, availability)| *availability != Availability::Hidden)
        .collect()
}

/// The mounted bases the selected project could move to, in summary order.
pub fn move_targets(app: &App) -> Vec<PathBuf> {
    let Some(project) = app.library.selected() else {
        return Vec::new();
    };
    app.summary
        .as_ref()
        .map(|summary| {
            summary
                .bases
                .iter()
                .filter(|base| base.probe.usable() && base.path != project.base)
                .map(|base| base.path.clone())
                .collect()
        })
        .unwrap_or_default()
}

impl App {
    /// The action menu: a verb's own key runs it and closes the menu, exactly
    /// as Enter on its row would; anything else — help, the palette, the
    /// arrows — runs over the menu and leaves it open.
    pub(super) fn on_actions_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(id) = command::lookup(self.context(), key, self) else {
            return Vec::new();
        };
        let is_verb = crate::tui::app::actions::action_entries(self)
            .iter()
            .any(|(entry, _)| *entry == id);
        if is_verb {
            self.modals.pop();
        }
        self.run(id)
    }

    pub(super) fn on_text_prompt_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Prompt, key, self)
        {
            return self.run(id);
        }
        let changed = match self.modals.top_mut() {
            Some(Modal::TextPrompt(prompt)) => prompt.input.apply(&key),
            _ => false,
        };
        if changed && let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
            prompt.error = None;
        }
        Vec::new()
    }

    pub(super) fn submit_text_prompt(&mut self) -> Vec<Effect> {
        let (text, then) = match self.modals.top() {
            Some(Modal::TextPrompt(prompt)) => {
                (prompt.input.text().to_string(), prompt.then.clone())
            }
            _ => return Vec::new(),
        };
        match then {
            TextThen::Rename(path) => {
                if let Err(error) = validators::folder_name(&text) {
                    if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                        prompt.error = Some(error);
                    }
                    return Vec::new();
                }
                self.modals.pop();
                let Some(project) = self.project_at(&path) else {
                    return self.gone_from_the_library();
                };
                self.run_action(
                    "renaming…",
                    Action::Rename {
                        project: Box::new(project),
                        name: text,
                    },
                )
            }
            TextThen::AddTodo => {
                self.modals.pop();
                if text.trim().is_empty() {
                    return Vec::new();
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                let next = self
                    .details
                    .get(&project.path)
                    .map(|detail| detail.todos.len())
                    .unwrap_or(0);
                let effects = self.run_action(
                    "writing…",
                    Action::AddTodo {
                        project: Box::new(project),
                        text,
                    },
                );
                if !effects.is_empty() {
                    self.pane_pending = Some(pane::PaneTarget::Todo(next));
                }
                effects
            }
            TextThen::AddTag => {
                if text.trim().is_empty() {
                    self.modals.pop();
                    return Vec::new();
                }
                // Refused under the line, with the rule named, while the
                // text is still there to correct — the same shape a rename
                // takes.
                let tag = match validators::tag(&text) {
                    Ok(tag) => tag,
                    Err(error) => {
                        if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                            prompt.error = Some(error);
                        }
                        return Vec::new();
                    }
                };
                self.modals.pop();
                self.add_tag(tag)
            }
            TextThen::CopyTo => {
                let typed = text.trim().to_string();
                if typed.is_empty() {
                    return Vec::new();
                }
                self.modals.pop();
                // Expanded here, refused in the engine: `~/backups` has to mean
                // the same thing it means in `config set bases`, and the rule
                // about bases is stated once, in `copy_engine`.
                let destination = match crate::core::config::expand_base_path(&typed) {
                    Ok(path) => path,
                    Err(error) => {
                        self.warn(format!("{error:#}"));
                        return Vec::new();
                    }
                };
                if self.batching() {
                    return self.start_job(jobs::JobKind::CopyTo(destination), None);
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                self.run_action(
                    "copying…",
                    Action::CopyTo {
                        project: Box::new(project),
                        destination,
                    },
                )
            }
            TextThen::RaiseCounter => {
                self.modals.pop();
                match text.trim().parse::<u64>() {
                    Ok(value) => self.run_action(
                        settings::Job::RaiseCounter.busy(),
                        Action::RaiseCounter(value),
                    ),
                    Err(_) => {
                        self.warn(format!("expected a number, got '{}'", text.trim()));
                        Vec::new()
                    }
                }
            }
            TextThen::Delete(path) => {
                if !text.trim().eq_ignore_ascii_case(validators::DELETE_WORD) {
                    // The text stays: one Backspace fixes a typo.
                    if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                        prompt.error = Some(validators::DELETE_MISMATCH.to_string());
                    }
                    return Vec::new();
                }
                self.modals.pop();
                if self.batching() {
                    return self.start_job(jobs::JobKind::Delete, None);
                }
                let Some(project) = self.project_at(&path) else {
                    return self.gone_from_the_library();
                };
                self.run_action("deleting…", Action::Delete(Box::new(project)))
            }
        }
    }

    /// One tag, on the selection or on every mark.
    pub(super) fn add_tag(&mut self, tag: String) -> Vec<Effect> {
        if self.batching() {
            return self.start_job(jobs::JobKind::AddTag(tag), None);
        }
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        self.run_action(
            "tagging…",
            Action::AddTag {
                project: Box::new(project),
                tag,
            },
        )
    }

    /// One note, on the selection or on every mark.
    fn add_note(&mut self, text: String) -> Vec<Effect> {
        if self.batching() {
            return self.start_job(jobs::JobKind::Note(text), None);
        }
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        // From the pane, the cursor follows the note to where it lands: the
        // end of the list. From the list, the pane's cursor is not in play.
        let next = self
            .details
            .get(&project.path)
            .map(|detail| detail.notes.len())
            .unwrap_or(0);
        let effects = self.run_action(
            "adding a note…",
            Action::AppendNote {
                project: Box::new(project),
                text,
            },
        );
        if !effects.is_empty() && self.focus == Focus::Detail {
            self.pane_pending = Some(pane::PaneTarget::Note(next));
        }
        effects
    }

    /// The quick note: Enter saves, Alt-Enter breaks a line, Esc cancels;
    /// everything else is the text area's.
    pub(super) fn on_note_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_none()
            && let Some(id) = command::lookup(Context::Prompt, key, self)
        {
            return self.run(id);
        }
        if let Some(Modal::Note(note)) = self.modals.top_mut() {
            note.area.apply(&key);
        }
        Vec::new()
    }

    /// Enter on a quick note: keep it, or say nothing was written.
    pub(super) fn save_note(&mut self) -> Vec<Effect> {
        let Some(Modal::Note(note)) = self.modals.pop() else {
            return Vec::new();
        };
        let text = note.area.text().trim().to_string();
        if text.is_empty() {
            self.info("no note written");
            return Vec::new();
        }
        self.add_note(text)
    }

    pub(super) fn on_confirm_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if !key.ctrl => self.answer_confirm(true),
            KeyCode::Char('n') | KeyCode::Char('N') if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
            // Enter is the commonest reflex there is on a two-button dialog,
            // and nothing bound it in `Context::Modal` — so it fell through to
            // the registry and produced silence. It answers `y`, which is the
            // key line's first entry and the default every `confirm` in this
            // app already offers.
            KeyCode::Enter => self.answer_confirm(true),
            _ => self.lookup_and_run(key),
        }
    }

    fn answer_confirm(&mut self, yes: bool) -> Vec<Effect> {
        let then = match self.modals.top() {
            Some(Modal::Confirm(confirm)) => confirm.then.clone(),
            _ => return Vec::new(),
        };
        self.modals.pop();
        if !yes {
            return Vec::new();
        }
        match then {
            ConfirmThen::Unregister(path) => {
                let Some(project) = self.project_at(&path) else {
                    return self.gone_from_the_library();
                };
                self.run_action("unregistering…", Action::Unregister(Box::new(project)))
            }
            ConfirmThen::DeleteTemplate(slug) => {
                self.run_action("deleting the template…", Action::DeleteTemplate(slug))
            }
            ConfirmThen::DiscardTemplate { then_quit } => {
                // The confirm was popped above; the builder under it goes now.
                self.modals.pop();
                self.info("Discarded — the template was not written.");
                match then_quit {
                    Some(exit) => vec![Effect::Quit(exit)],
                    None => Vec::new(),
                }
            }
            ConfirmThen::DeleteBatch => self.start_job(jobs::JobKind::Delete, None),
            ConfirmThen::UnregisterBatch => self.start_job(jobs::JobKind::Unregister, None),
        }
    }

    /// A multi-pick holds no query, so every key is the registry's — Space
    /// included, which is why `PickToggle` is `Hidden` in the picker that does
    /// have one.
    pub(super) fn on_multi_pick_key(&mut self, key: Key) -> Vec<Effect> {
        self.lookup_and_run(key)
    }

    /// Space on a multi-pick: put the row in the set, or take it out.
    pub(super) fn toggle_multi_pick(&mut self) -> Vec<Effect> {
        if let Some(Modal::MultiPick(pick)) = self.modals.top_mut()
            && let Some(flag) = pick.picked.get_mut(pick.selected)
        {
            *flag = !*flag;
        }
        Vec::new()
    }

    pub(super) fn submit_multi_pick(&mut self) -> Vec<Effect> {
        let (chosen, then) = match self.modals.top() {
            Some(Modal::MultiPick(pick)) => (pick.chosen(), pick.then),
            _ => return Vec::new(),
        };
        self.modals.pop();
        match then {
            MultiThen::RemoveTags => {
                if chosen.is_empty() {
                    return Vec::new();
                }
                if self.batching() {
                    return self.start_job(jobs::JobKind::RemoveTags(chosen), None);
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                self.run_action(
                    "removing tags…",
                    Action::RemoveTags {
                        project: Box::new(project),
                        tags: chosen,
                    },
                )
            }
        }
    }

    /// `A`: pick a tag the library already uses, or type a new one. Over
    /// marks the list is every known tag not already on all of them, and the
    /// answer is asked once.
    pub(super) fn open_add_tag(&mut self) -> Vec<Effect> {
        let targets = self.library.targets();
        if targets.is_empty() {
            return Vec::new();
        }
        let available: Vec<String> = self
            .library
            .known_tags
            .iter()
            .filter(|tag| !targets.iter().all(|project| project.tags.contains(tag)))
            .cloned()
            .collect();
        let title = if targets.len() > 1 {
            format!("Tag to add to {} projects", targets.len())
        } else {
            "Tag to add".to_string()
        };
        if available.is_empty() {
            self.modals.push(Modal::TextPrompt(TextPrompt::new(
                validators::ADD_TAG_PROMPT,
                TextThen::AddTag,
            )));
            return Vec::new();
        }

        let mut items: Vec<PickItem> = available
            .into_iter()
            .map(|tag| PickItem {
                label: tag.clone(),
                detail: String::new(),
                value: tag,
            })
            .collect();
        items.push(PickItem {
            label: crate::tui::app::actions::NEW_TAG.to_string(),
            detail: String::new(),
            value: crate::tui::app::actions::NEW_TAG.to_string(),
        });
        self.modals
            .push(Modal::Pick(PickState::new(title, items, Then::AddTag)));
        Vec::new()
    }

    /// `m`: pick the mounted base to move into, then move.
    pub(super) fn open_move_picker(&mut self) -> Vec<Effect> {
        let targets = crate::tui::app::actions::move_targets(self);
        if targets.is_empty() {
            return Vec::new();
        }
        let items: Vec<PickItem> = targets
            .into_iter()
            .map(|path| PickItem {
                label: crate::core::library::base_label(&path),
                detail: String::new(),
                value: path.display().to_string(),
            })
            .collect();
        self.modals.push(Modal::Pick(PickState::new(
            "Move to which base?",
            items,
            Then::MoveToBase,
        )));
        Vec::new()
    }

    /// A move as a one-item job, with the progress modal up while it runs.
    pub(super) fn run_move(&mut self, target: PathBuf) -> Vec<Effect> {
        let Some(project) = self.library.selected().cloned() else {
            return Vec::new();
        };
        self.move_progress = Some(Progress::new(&[]));
        self.run_action(
            "moving…",
            Action::Move {
                project: Box::new(project),
                target,
            },
        )
    }

    /// `M`/`J`: read the full metadata or journal on a worker, then show it.
    pub(super) fn open_view(&mut self, id: CommandId) -> Vec<Effect> {
        let Some(project) = self.library.selected() else {
            return Vec::new();
        };
        let kind = if id == CommandId::ShowMetadata {
            ViewKind::Metadata
        } else {
            ViewKind::Journal
        };
        let title = format!(
            "{} · {}",
            project.id,
            if kind == ViewKind::Metadata {
                "metadata"
            } else {
                "notes"
            }
        );
        self.load_view(title, project.path.clone(), kind)
    }

    /// A read-only view: the dialog goes up at once saying it is reading, so
    /// the key is seen to have worked on a slow disk, and the worker's answer
    /// fills it in (`Msg::ViewLoaded`) if it is still the one on top.
    pub(super) fn load_view(
        &mut self,
        title: String,
        path: PathBuf,
        kind: ViewKind,
    ) -> Vec<Effect> {
        self.modals.push(Modal::message(
            title.clone(),
            "reading…",
            MessageLevel::Info,
        ));
        vec![Effect::LoadView { title, path, kind }]
    }
}
