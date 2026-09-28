//! The single-project actions as native dialogs: the action-menu modal,
//! the text prompt, the yes/no confirm, the multi-pick for tags, and the pure
//! lookups that feed them. The verbs themselves are declared once in
//! `tui::command`; this module holds the modal state a verb opens and the lists
//! its pickers show, so `update` stays a function of data and not of closures.

use std::path::PathBuf;

use ratatui::crossterm::event::KeyCode;

use super::{App, Focus};
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

/// The projects a dialog was opened about: the marks in view, or the
/// selection, by path.
///
/// **A dialog carries its targets rather than re-reading the selection**: a
/// discovery arriving under an open dialog takes a marked row out of the list,
/// or moves the cursor when the named row is no longer in the snapshot, and a
/// verb built from the selection at Enter would land on a project other than
/// the ones on screen. `App::targets_now` takes them as the dialog opens and
/// `App::still_here` resolves them at its answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Targets {
    pub paths: Vec<PathBuf>,
    /// Asked over marks, so the verb runs as a batch over what is left of
    /// them — one project included.
    pub batch: bool,
}

impl Targets {
    pub fn one(path: PathBuf) -> Self {
        Self {
            paths: vec![path],
            batch: false,
        }
    }
}

/// What a text prompt's answer does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextThen {
    /// Rename the project the prompt named, carried by path ([`Targets`]
    /// says why).
    Rename(std::path::PathBuf),
    AddTag(Targets),
    /// A todo for the project at `project`, where the pane cannot show its
    /// list: written at `place`, under a phase when one was named first.
    AddTodo {
        place: crate::core::body::TodoPlace,
        project: PathBuf,
    },
    /// A new phase's name for the project at this path, where the pane cannot
    /// show the list; its first todo is asked for next, and the two are
    /// written together.
    AddPhase(PathBuf),
    /// Type the word `delete` to confirm; nothing else deletes. The prompt
    /// names the folder — or the folders, over marks — so what is being
    /// confirmed is on screen, and the word is the same every time.
    Delete(Targets),
    /// Raise the global ID counter to the number typed.
    RaiseCounter,
    /// The folder to copy into. Refused by the engine rather than here, so the
    /// command line and the app say the same words about the same rule.
    CopyTo(Targets),
    /// Type the word `discard` to settle an unfinished item by removing it
    /// (`core::attention::Action::Discard`); carries the item's path.
    DiscardAttention(std::path::PathBuf),
}

/// The quick note: a few lines typed where you are. Enter saves, Alt-Enter
/// (or a pasted newline) breaks a line — so a pasted paragraph is one note,
/// and never a run of keystrokes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteState {
    pub area: crate::tui::widgets::text_area::TextArea,
    /// The projects the note goes to: the marks, or the one selected.
    pub targets: Targets,
}

impl NoteState {
    pub fn new(targets: Targets) -> Self {
        Self {
            area: crate::tui::widgets::text_area::TextArea::new(),
            targets,
        }
    }

    /// How many projects the note goes to.
    pub fn count(&self) -> usize {
        self.targets.paths.len()
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
    /// Unregister the projects the question named.
    Unregister(Targets),
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub prompt: String,
    pub then: ConfirmThen,
}

/// What a multi-pick answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MultiThen {
    RemoveTags(Targets),
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
            TextThen::Rename(path) => self.rename_to(path, text),
            TextThen::AddPhase(path) => self.add_phase_named(path, text),
            TextThen::AddTodo { place, project } => self.add_typed_todo(place, project, text),
            TextThen::AddTag(targets) => self.add_typed_tag(targets, text),
            TextThen::CopyTo(targets) => self.copy_to_typed(targets, text),
            TextThen::RaiseCounter => self.raise_counter_to(text),
            TextThen::DiscardAttention(path) => self.discard_when_typed(path, text),
            TextThen::Delete(targets) => self.delete_when_typed(targets, text),
        }
    }

    fn rename_to(&mut self, path: PathBuf, text: String) -> Vec<Effect> {
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

    fn add_phase_named(&mut self, path: PathBuf, text: String) -> Vec<Effect> {
        if text.trim().is_empty() {
            self.modals.pop();
            return Vec::new();
        }
        let Some(name) = crate::core::body::phase_label(&text) else {
            if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                prompt.error = Some(validators::PHASE_NAMELESS.to_string());
            }
            return Vec::new();
        };
        self.modals.pop();
        if !self.is_selected(&path) {
            return self.gone_from_the_library();
        }
        self.start_adding(crate::core::body::TodoPlace::Phase(name))
    }

    fn add_typed_todo(
        &mut self,
        place: crate::core::body::TodoPlace,
        project: PathBuf,
        text: String,
    ) -> Vec<Effect> {
        self.modals.pop();
        if text.trim().is_empty() {
            return Vec::new();
        }
        let Some(project) = self.project_at(&project) else {
            return self.gone_from_the_library();
        };
        let next = self
            .details
            .get(&project.path)
            .map(|detail| detail.todos.len())
            .unwrap_or(0);
        let effects = self.run_action(
            "writing…",
            Action::AddTodos {
                project: Box::new(project),
                texts: vec![text],
                place,
            },
        );
        if !effects.is_empty() {
            self.pane_pending = Some(pane::PaneTarget::Todo(next));
        }
        effects
    }

    fn add_typed_tag(&mut self, targets: Targets, text: String) -> Vec<Effect> {
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
        self.add_tag(tag, &targets)
    }

    fn copy_to_typed(&mut self, targets: Targets, text: String) -> Vec<Effect> {
        let typed = text.trim().to_string();
        if typed.is_empty() {
            self.modals.pop();
            return Vec::new();
        }
        // Expanded here, refused in the engine: `~/backups` has to mean
        // the same thing it means in `config set bases`, and the rule
        // about bases is stated once, in `copy_engine`.
        let destination = match crate::core::config::expand_base_path(&typed) {
            Ok(path) => path,
            Err(error) => {
                self.refuse_under_the_line(format!("{error:#}"));
                return Vec::new();
            }
        };
        self.modals.pop();
        let projects = self.still_here(&targets);
        if projects.is_empty() {
            return self.gone_from_the_library();
        }
        self.start_background(
            crate::core::jobs::JobKind::Copy,
            projects,
            Some(destination),
        )
    }

    fn raise_counter_to(&mut self, text: String) -> Vec<Effect> {
        match text.trim().parse::<u64>() {
            Ok(value) => {
                self.modals.pop();
                self.run_action(
                    settings::Job::RaiseCounter.busy(),
                    Action::RaiseCounter(value),
                )
            }
            Err(_) => {
                self.refuse_under_the_line(format!("expected a number, got '{}'", text.trim()));
                Vec::new()
            }
        }
    }

    /// A refusal of what was typed: under the prompt's line, with the text
    /// still there to correct.
    fn refuse_under_the_line(&mut self, error: String) {
        if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
            prompt.error = Some(error);
        }
    }

    fn discard_when_typed(&mut self, path: PathBuf, text: String) -> Vec<Effect> {
        if !text
            .trim()
            .eq_ignore_ascii_case(super::attention::DISCARD_WORD)
        {
            if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                prompt.error = Some(super::attention::DISCARD_MISMATCH.to_string());
            }
            return Vec::new();
        }
        self.modals.pop();
        self.resolve_attention(path, crate::core::attention::Action::Discard)
    }

    fn delete_when_typed(&mut self, targets: Targets, text: String) -> Vec<Effect> {
        if !text.trim().eq_ignore_ascii_case(validators::DELETE_WORD) {
            // The text stays: one Backspace fixes a typo.
            if let Some(Modal::TextPrompt(prompt)) = self.modals.top_mut() {
                prompt.error = Some(validators::DELETE_MISMATCH.to_string());
            }
            return Vec::new();
        }
        self.modals.pop();
        // What is left of what the question named, and nothing else:
        // the word was typed about those folders.
        let projects = self.still_here(&targets);
        if projects.is_empty() {
            return self.gone_from_the_library();
        }
        self.start_background(crate::core::jobs::JobKind::Delete, projects, None)
    }

    /// One tag, on the projects its question was about.
    pub(super) fn add_tag(&mut self, tag: String, targets: &Targets) -> Vec<Effect> {
        let mut projects = self.still_here(targets);
        if projects.is_empty() {
            return self.gone_from_the_library();
        }
        if targets.batch {
            return self.start_job_over(jobs::JobKind::AddTag(tag), projects);
        }
        let project = projects.remove(0);
        self.run_action(
            "tagging…",
            Action::AddTag {
                project: Box::new(project),
                tag,
            },
        )
    }

    /// One note, on the projects its question was about.
    pub(super) fn add_note(&mut self, text: String, targets: &Targets) -> Vec<Effect> {
        let mut projects = self.still_here(targets);
        if projects.is_empty() {
            return self.gone_from_the_library();
        }
        if targets.batch {
            return self.start_job_over(jobs::JobKind::Note(text), projects);
        }
        let project = projects.remove(0);
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
        self.add_note(text, &note.targets)
    }

    pub(super) fn on_confirm_key(&mut self, key: Key) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if !key.ctrl => self.answer_confirm(true),
            KeyCode::Char('n') | KeyCode::Char('N') if !key.ctrl => {
                self.modals.pop();
                Vec::new()
            }
            // Enter, the commonest reflex on a two-button dialog, answers `y`:
            // the key line's first entry and the default every `confirm` in
            // this app offers. Nothing binds it in `Context::Modal`, so without
            // this arm it falls through to the registry and does nothing.
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
            ConfirmThen::Unregister(targets) => {
                let mut projects = self.still_here(&targets);
                if projects.is_empty() {
                    return self.gone_from_the_library();
                }
                if targets.batch {
                    return self.start_job_over(jobs::JobKind::Unregister, projects);
                }
                let project = projects.remove(0);
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
            Some(Modal::MultiPick(pick)) => (pick.chosen(), pick.then.clone()),
            _ => return Vec::new(),
        };
        self.modals.pop();
        match then {
            MultiThen::RemoveTags(targets) => {
                if chosen.is_empty() {
                    return Vec::new();
                }
                let mut projects = self.still_here(&targets);
                if projects.is_empty() {
                    return self.gone_from_the_library();
                }
                if targets.batch {
                    return self.start_job_over(jobs::JobKind::RemoveTags(chosen), projects);
                }
                let project = projects.remove(0);
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
        let asked = self.targets_now();
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
                TextThen::AddTag(asked),
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
        self.modals.push(Modal::Pick(PickState::new(
            title,
            items,
            Then::AddTag(asked),
        )));
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
        let asked = self.targets_now();
        self.modals.push(Modal::Pick(PickState::new(
            "Move to which base?",
            items,
            Then::MoveToBase(asked),
        )));
        Vec::new()
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
        let request = self.open_reading(&title);
        vec![Effect::LoadView {
            request,
            title,
            path,
            kind,
        }]
    }

    /// Put up the dialog a worker's read will fill, and number the read.
    ///
    /// **The answer names its question by that number, not by the title**:
    /// two projects can carry one id (`copy-to` keeps it), so two dialogs can
    /// carry one title, and a slow read of the first would land as the
    /// second's contents.
    pub(super) fn open_reading(&mut self, title: &str) -> u64 {
        self.modals
            .push(Modal::message(title, "reading…", MessageLevel::Info));
        self.view_request += 1;
        self.view_request
    }
}
