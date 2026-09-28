//! `update`'s side of the template builder: opening it, the keys each of its
//! lists and forms takes, and the save.

use super::App;
use super::studio::*;
use crate::tui::app::modal::Modal;
use crate::tui::command::Key;
use crate::tui::effect::{Action, Effect};
use crate::tui::widgets::form::FormEvent;
use ratatui::crossterm::event::KeyCode;

impl App {
    /// Enter on the builder: open the highlighted section (or save, or
    /// discard) from the section list; edit the highlighted entry on the
    /// variables and files lists.
    pub(super) fn builder_open(&mut self) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match &mut builder.open {
            None => match builder.row() {
                Row::Section(section) => {
                    builder.open_section(section);
                    Vec::new()
                }
                Row::Save => self.save_template(),
                // Discard asks the same question Esc does, through the same
                // ladder — the row is the spelled-out spelling of the key.
                Row::Discard => self.close_top(),
            },
            Some(Open::Variables(list)) => {
                if let Some(variable) = builder.template.variables.get(list.selected) {
                    list.editing = Some((list.selected, variable_form(Some(variable))));
                }
                Vec::new()
            }
            Some(Open::Files(list)) => {
                if let Some(file) = builder.template.files.get(list.selected) {
                    let body = if file.template.is_empty() {
                        file.content.clone()
                    } else {
                        file.template.clone()
                    };
                    list.editing = Some(FileEdit {
                        index: list.selected,
                        path: crate::tui::widgets::input::LineEdit::with_text(file.path.clone()),
                        body: crate::tui::widgets::text_area::TextArea::with_text(&body),
                        in_body: false,
                        error: None,
                    });
                }
                Vec::new()
            }
            Some(_) => Vec::new(),
        }
    }

    /// `a` on the variables or files list: a new entry at the end.
    pub(super) fn builder_add(&mut self) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match &mut builder.open {
            Some(Open::Variables(list)) => {
                let count = builder.template.variables.len();
                list.editing = Some((count, variable_form(None)));
            }
            Some(Open::Files(list)) => {
                list.editing = Some(FileEdit {
                    index: builder.template.files.len(),
                    path: crate::tui::widgets::input::LineEdit::new(),
                    body: crate::tui::widgets::text_area::TextArea::new(),
                    in_body: false,
                    error: None,
                });
            }
            _ => {}
        }
        Vec::new()
    }

    /// `d` on the variables or files list: the highlighted entry goes.
    pub(super) fn builder_remove(&mut self) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match &mut builder.open {
            Some(Open::Variables(list)) => {
                let count = builder.template.variables.len();
                if list.selected < count {
                    builder.template.variables.remove(list.selected);
                    list.selected = list.selected.min(count.saturating_sub(2));
                    builder.error = None;
                }
            }
            Some(Open::Files(list)) => {
                let count = builder.template.files.len();
                if list.selected < count {
                    builder.template.files.remove(list.selected);
                    list.selected = list.selected.min(count.saturating_sub(2));
                    builder.error = None;
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// `K`/`J` on the variables list: reorder in place. Moving a row is one
    /// keystroke and shows the result immediately.
    pub(super) fn builder_move(&mut self, up: bool) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let count = builder.template.variables.len();
        if let Some(Open::Variables(list)) = &mut builder.open {
            let at = list.selected;
            let other = if up {
                at.checked_sub(1)
            } else {
                (at + 1 < count).then_some(at + 1)
            };
            if let Some(other) = other {
                builder.template.variables.swap(at, other);
                list.selected = other;
            }
        }
        Vec::new()
    }

    /// New (`slug` is `None`) or edit: the builder over a scratch template.
    pub(super) fn open_builder(&mut self, slug: Option<String>) -> Vec<Effect> {
        let effects = self.open_builder_inner(slug);
        self.offer_guide_once();
        effects
    }

    fn open_builder_inner(&mut self, slug: Option<String>) -> Vec<Effect> {
        match slug {
            Some(slug) => {
                let mut builder = Builder::new(None);
                builder.pending = Some(slug.clone());
                self.modals.push(Modal::Builder(Box::new(builder)));
                vec![Effect::LoadTemplateSource { slug }]
            }
            None => {
                self.modals
                    .push(Modal::Builder(Box::new(Builder::new(None))));
                Vec::new()
            }
        }
    }

    pub(super) fn on_builder_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if builder.pending.is_some() {
            if key.code == KeyCode::Esc {
                self.modals.pop();
            }
            return Vec::new();
        }
        // A save is in flight: it is one worker call away from an answer, and
        // every key until then would act on a template that may be about to
        // close. Esc included — cancelling a write already sent is not a
        // thing this can promise.
        if builder.saving {
            return Vec::new();
        }
        match &mut builder.open {
            None => self.on_builder_list_key(key),
            Some(Open::Metadata(_)) | Some(Open::Id(_)) => self.on_builder_form_key(key),
            Some(Open::Variables(_)) => self.on_variables_key(key),
            Some(Open::Structure(_)) => self.on_structure_key(key),
            Some(Open::Files(_)) => self.on_files_key(key),
        }
    }

    /// The section list: enter a section, save, or discard — every key of it
    /// declared under `Context::Builder`.
    fn on_builder_list_key(&mut self, key: Key) -> Vec<Effect> {
        self.lookup_and_run(key)
    }

    /// Save, or say what refused it.
    ///
    /// **The builder stays up until the write has actually landed**, so a
    /// refusal from under the data lock — an occupied slug, a lock held by
    /// another terminal, a full disk — lands on it with everything typed still
    /// there. `Modal::Builder` is popped in `on_action_done`, on the success
    /// path only.
    pub(super) fn save_template(&mut self) -> Vec<Effect> {
        // Read what the save needs first, so the borrow ends before the app
        // itself is needed.
        let (template, original_slug) = {
            let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
                return Vec::new();
            };
            if builder.saving {
                return Vec::new();
            }
            if let Err(error) = builder.template.validate() {
                builder.error = Some(format!("Cannot save: {error:#}"));
                return Vec::new();
            }
            (builder.template.clone(), builder.original_slug.clone())
        };

        // The slug must be free unless this very template was loaded from it.
        // `operations::save_template` enforces that under the lock and is the
        // authority; this is the same question asked of the cards already in
        // memory, so the answer lands on the list instead of arriving from a
        // worker. `update` reads no disk to do it.
        let occupied = self.studio.cards.iter().any(|card| {
            card.on_disk
                && card.slug == template.slug
                && original_slug.as_deref() != Some(card.slug.as_str())
        });
        if occupied {
            if let Some(Modal::Builder(builder)) = self.modals.top_mut() {
                builder.error = Some(format!(
                    "Cannot save: a template called '{}' already exists — \
                     give this one another slug, or edit that one instead",
                    template.slug
                ));
            }
            return Vec::new();
        }

        let effects = self.run_action(
            "saving the template…",
            Action::SaveTemplate {
                template: Box::new(template),
                original_slug,
            },
        );
        // `run_action` refuses while another one is running; only a save that
        // actually started may put the builder into its saving state.
        if !effects.is_empty()
            && let Some(Modal::Builder(builder)) = self.modals.top_mut()
        {
            builder.saving = true;
            builder.error = None;
        }
        effects
    }

    /// The metadata and ID sections: a form, checked here because every rule
    /// they enforce is a rule about the text and not about a disk.
    fn on_builder_form_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let is_metadata = matches!(builder.open, Some(Open::Metadata(_)));
        // Read off the template before the form is borrowed from the same
        // builder: the naming-pattern advice is a question about the
        // variables, and the form is a field of the thing that holds them.
        let declared: Vec<String> = builder
            .template
            .variables
            .iter()
            .map(|v| v.slug.clone())
            .collect();
        let (Some(Open::Metadata(form)) | Some(Open::Id(form))) = &mut builder.open else {
            return Vec::new();
        };
        match form.apply(&key) {
            FormEvent::Cancel => builder.open = None,
            FormEvent::Submit => {
                let refusal = if is_metadata {
                    check_metadata(form)
                } else {
                    check_id(form)
                };
                if let Some((field, message)) = refusal {
                    form.fail(Some(field), message);
                    return Vec::new();
                }
                let form = form.clone();
                if is_metadata {
                    builder.commit_metadata(&form);
                } else {
                    builder.commit_id(&form);
                }
                builder.open = None;
                builder.error = None;
            }
            // The slug follows the name until one is typed, and the naming
            // pattern says on this keystroke what it would do with the
            // variables this template declares.
            FormEvent::Changed if is_metadata => {
                let declared: Vec<&str> = declared.iter().map(String::as_str).collect();
                sync_metadata_form(form, &declared);
            }
            _ => {}
        }
        Vec::new()
    }

    fn on_variables_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let count = builder.template.variables.len();
        let Some(Open::Variables(list)) = &mut builder.open else {
            return Vec::new();
        };
        // Editing one variable: the form owns every key until it answers.
        if let Some((index, form)) = &mut list.editing {
            match form.apply(&key) {
                FormEvent::Cancel => list.editing = None,
                FormEvent::Changed => sync_variable_form(form, self.theme.glyphs),
                FormEvent::Submit => match variable_from(form) {
                    Ok(variable) => {
                        let index = *index;
                        list.editing = None;
                        if index < count {
                            builder.template.variables[index] = variable;
                        } else {
                            builder.template.variables.push(variable);
                            list.selected = count;
                        }
                        builder.error = None;
                    }
                    Err((field, message)) => form.fail(Some(field), message),
                },
                _ => {}
            }
            return Vec::new();
        }
        // The list itself: every key it answers is in the registry.
        self.lookup_and_run(key)
    }

    /// The structure section: one folder path per line, the tree drawn beside
    /// it as it is typed. Enter is a newline here, so Ctrl-S commits.
    fn on_structure_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let Some(Open::Structure(area)) = &mut builder.open else {
            return Vec::new();
        };
        match (key.code, key.ctrl) {
            (KeyCode::Esc, false) => builder.open = None,
            (KeyCode::Char('s'), true) => {
                let area = area.clone();
                builder.commit_structure(&area);
                builder.open = None;
                builder.error = None;
            }
            _ => {
                area.apply(&key);
            }
        }
        Vec::new()
    }

    fn on_files_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let count = builder.template.files.len();
        let Some(Open::Files(list)) = &mut builder.open else {
            return Vec::new();
        };
        if let Some(edit) = &mut list.editing {
            match (key.code, key.ctrl) {
                (KeyCode::Esc, false) => list.editing = None,
                (KeyCode::Tab, false) | (KeyCode::BackTab, false) => edit.in_body = !edit.in_body,
                (KeyCode::Char('s'), true) => match file_from(edit) {
                    Ok(entry) => {
                        let index = edit.index;
                        list.editing = None;
                        if index < count {
                            builder.template.files[index] = entry;
                        } else {
                            builder.template.files.push(entry);
                            list.selected = count;
                        }
                        builder.error = None;
                    }
                    Err(message) => edit.error = Some(message),
                },
                _ => {
                    if edit.in_body {
                        edit.body.apply(&key);
                    } else {
                        edit.path.apply(&key);
                    }
                    edit.error = None;
                }
            }
            return Vec::new();
        }
        // The list itself: every key it answers is in the registry.
        self.lookup_and_run(key)
    }
}
