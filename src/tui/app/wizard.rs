//! The flows that build something: create a project, apply a template to a
//! folder, register a folder fastf did not create.
//!
//! One shape serves all three, because all three *are* one shape: answer a few
//! questions, look at what that would do, say yes. The questions are a
//! [`Form`]; the look is a [`Preview`] a worker computed from the very
//! functions the commit will use (`project::plan_report`, `project::apply_plan`,
//! `cli::register::preview_rename`), so the screen cannot promise one thing and
//! do another. That was a real defect twice over — a rename prompt offering
//! `ID0001` while the commit wrote `ID0011`, and a preview header saying
//! nothing would be created immediately before creating it.
//!
//! `register.rs` builds the register flow's fields and reads its answers back;
//! this module holds the state the three share and the create and apply halves.

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::crossterm::event::KeyCode;

use super::App;
use crate::core::project::DryRunReport;
use crate::tui::app::data::{self, Prefs, TemplateInfo};
use crate::tui::app::modal::{Modal, PickItem, PickState, Then};
use crate::tui::app::register;
use crate::tui::command::{self, Key};
use crate::tui::effect::{Action, ApplyRequest, CreateRequest, Effect, Request};
use crate::tui::widgets::form::{Field, Form, FormEvent};

/// The field every flow that names a template uses.
pub const FIELD_TEMPLATE: &str = "template";
/// The base a new project is created in; present only with a choice to make.
pub const FIELD_BASE: &str = "base";
/// The folder `apply` fills in.
pub const FIELD_TARGET: &str = "target";
/// A variable field's key is this plus the variable's slug.
pub const VAR_PREFIX: &str = "var:";
/// The folder a template is generated from.
pub const FIELD_SOURCE: &str = "source";
/// What to call the generated template.
pub const FIELD_SLUG: &str = "slug";
pub const FIELD_FORCE: &str = "force";
pub const FIELD_BUNDLE: &str = "bundle";

/// The template choice's "no template at all" entry — register's own answer,
/// which writes a minimal record with the `(registered)` slug.
pub const NO_TEMPLATE: &str = "(none)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowKind {
    Create,
    Apply,
    Register,
    /// Generate a template from a folder that already has the shape wanted.
    FromFolder,
}

impl FlowKind {
    pub fn title(self) -> &'static str {
        match self {
            FlowKind::Create => "new project",
            FlowKind::Apply => "apply a template",
            FlowKind::Register => "register a folder",
            FlowKind::FromFolder => "template from a folder",
        }
    }

    /// What a cancel says. The words are the old flows' own, so a cancelled
    /// run reads the same wherever it happened.
    pub fn cancelled(self) -> &'static str {
        match self {
            FlowKind::Create => "Cancelled — nothing was created.",
            FlowKind::Apply => "Cancelled — nothing was applied.",
            FlowKind::Register => "Cancelled — nothing was registered.",
            FlowKind::FromFolder => "Cancelled — no template was generated.",
        }
    }

    /// The verb on the preview, where Enter commits.
    pub fn commit(self) -> &'static str {
        match self {
            FlowKind::Create => "Enter creates it",
            FlowKind::Apply => "Enter applies it",
            FlowKind::Register => "Enter registers it",
            FlowKind::FromFolder => "Enter generates it",
        }
    }
}

/// Which half of the flow is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The questions.
    Form,
    /// What answering them would do.
    Preview,
}

/// What one `apply` would create and what it would leave alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyPreview {
    pub target: PathBuf,
    /// `(would create, path as shown)`, in plan order.
    pub rows: Vec<(bool, String)>,
    pub creates: usize,
    pub skips: usize,
}

/// What registering one folder would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterPreview {
    pub path: PathBuf,
    /// The template's display name, or `(registered)`.
    pub template: String,
    pub id: String,
    /// Where the ID came from: an `ID####` token in the folder name, or the
    /// counter. Worth saying, because recovering one is the whole reason
    /// register looks at a folder name at all.
    pub id_note: &'static str,
    pub created: String,
    /// `(current name, name after the rename)` when one would happen.
    pub rename: Option<(String, String)>,
    /// The folder already holds a `PROJECT_INFO.md`, so this re-registers it.
    pub pinfo_exists: bool,
    pub apply_structure: bool,
}

/// What generating a template from a folder would pick up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromFolderPreview {
    pub slug: String,
    pub structure: Vec<crate::core::template::FolderNode>,
    /// The text files reproduced as editable entries.
    pub files: Vec<String>,
    /// `(path, bytes)` for what would be copied byte for byte.
    pub assets: Vec<(String, u64)>,
    pub folders: usize,
    /// Binary or oversized files left out because bundling was not asked for.
    pub skipped: usize,
    pub bundle_bytes: u64,
    pub bundle: bool,
}

/// What registering every unregistered child of a base would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecursivePreview {
    pub base: PathBuf,
    /// `(folder name, what happens to its ID)`.
    pub rows: Vec<(String, String)>,
}

/// The answer a worker computed for the flow that is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Create(Box<DryRunReport>),
    Apply(ApplyPreview),
    Register(Box<RegisterPreview>),
    Recursive(RecursivePreview),
    FromFolder(Box<FromFolderPreview>),
}

/// One flow: the questions, the answer to them, and which is on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub kind: FlowKind,
    /// The template whose variables the form is asking for, when there is one.
    pub template: Option<TemplateInfo>,
    pub form: Form,
    pub step: Step,
    pub preview: Option<Preview>,
    pub scroll: usize,
    /// A worker is reading a template or building a preview. The screen says
    /// so and Enter is refused, so a slow disk cannot be answered twice.
    pub pending: bool,
    /// Commit as soon as the preview is built rather than showing it —
    /// `confirm_create = false`, which is a standing answer to the question
    /// the preview asks. The plan is still built the same way, so every
    /// refusal still lands on the field that caused it.
    pub auto_commit: bool,
}

impl Flow {
    pub fn new(kind: FlowKind, form: Form) -> Self {
        Self {
            kind,
            template: None,
            form,
            step: Step::Form,
            preview: None,
            scroll: 0,
            pending: false,
            auto_commit: false,
        }
    }

    /// The slug the form names, or `None` for register's `(none)`.
    pub fn template_slug(&self) -> Option<String> {
        match self.form.value(FIELD_TEMPLATE) {
            slug if slug.is_empty() || slug == NO_TEMPLATE => None,
            slug => Some(slug),
        }
    }

    /// Every variable the form collected, by slug.
    pub fn variables(&self) -> HashMap<String, String> {
        self.form
            .fields
            .iter()
            .filter_map(|field| {
                field
                    .key
                    .strip_prefix(VAR_PREFIX)
                    .map(|slug| (slug.to_string(), field.value()))
            })
            .collect()
    }

    /// The first required variable left empty, as a `(field key, message)` to
    /// refuse with. Checked here rather than at the commit because `update`
    /// can answer it without a disk, and an answer that arrives before the
    /// preview keeps the other answers on screen.
    pub fn missing_required(&self) -> Option<(String, String)> {
        let template = self.template.as_ref()?;
        template.variables.iter().find_map(|var| {
            let key = format!("{VAR_PREFIX}{}", var.slug);
            let empty = self.form.value(&key).trim().is_empty();
            (var.required && empty).then(|| (key, format!("{} is required", var.label)))
        })
    }

    /// Replace the variable fields with `template`'s, keeping any answer whose
    /// variable the new template also has — changing template mid-form is a
    /// correction, not a reason to retype a name that still applies.
    pub fn set_template(&mut self, template: Option<TemplateInfo>) {
        let held: HashMap<String, String> = self.variables();
        self.form
            .fields
            .retain(|field| !field.key.starts_with(VAR_PREFIX));
        if let Some(info) = &template {
            for var in &info.variables {
                let mut field = variable_field(var);
                if let Some(value) = held.get(&var.slug).filter(|value| !value.is_empty()) {
                    field.set_text(value.clone());
                    field.select(value);
                }
                self.form.fields.push(field);
            }
        }
        self.template = template;
        if self.form.focused().is_none() {
            self.form.selected = 0;
        }
    }
}

/// The field one variable is answered in: a list for a `select`, a line for
/// anything else, pre-filled with the template's default — which is what
/// a prompt's `[default]` meant, made editable instead of invisible.
pub fn variable_field(var: &crate::tui::app::data::VarInfo) -> Field {
    let hint = if var.required {
        format!("{} — required", var.slug)
    } else {
        format!("{} — optional, may be left empty", var.slug)
    };
    let key = format!("{VAR_PREFIX}{}", var.slug);
    if var.options.is_empty() {
        Field::text(&key, &var.label, &hint, var.default.clone())
    } else {
        let at = var
            .options
            .iter()
            .position(|option| option == &var.default)
            .unwrap_or(0);
        Field::choice(&key, &var.label, &hint, var.options.clone(), at)
    }
}

/// The create form: which template, which base, and the template's variables.
///
/// The template is a field rather than a picker that runs first, so changing
/// your mind about it costs one keystroke instead of the whole flow — and the
/// base is only asked about when there is more than one to choose from, which
/// is what `pick_base_interactively` decided by returning early.
pub fn create_form(templates: &[String], template_at: usize, bases: &[String]) -> Form {
    let mut fields = vec![Field::choice(
        FIELD_TEMPLATE,
        "Template",
        "← → to change, Space for the list",
        templates.to_vec(),
        template_at,
    )];
    fields.push(
        Field::choice(
            FIELD_BASE,
            "Base",
            "which library base the project folder is created in",
            bases.to_vec(),
            0,
        )
        .hidden(bases.len() < 2),
    );
    Form::new(fields)
}

/// The from-folder form: which folder to read, what to call the template, and
/// the two decisions that change what lands in it.
pub fn from_folder_form() -> Form {
    Form::new(vec![
        Field::text(
            FIELD_SOURCE,
            "Source folder",
            "an existing folder whose shape becomes the template",
            String::new(),
        ),
        Field::text(
            FIELD_SLUG,
            "Slug",
            "what the new template is called on the command line",
            String::new(),
        ),
        Field::toggle(
            FIELD_FORCE,
            "Overwrite",
            "replace a template that already answers to this slug",
            false,
        ),
        Field::toggle(
            FIELD_BUNDLE,
            "Bundle assets",
            "copy binary and oversized files in byte for byte — the preview totals them",
            false,
        ),
    ])
}

/// The apply form: which template, which folder, and the variables its files
/// interpolate. The target comes second and is checked before anything that
/// depends on it — `apply` used to reject it after every variable was answered.
pub fn apply_form(templates: &[String], template_at: usize) -> Form {
    Form::new(vec![
        Field::choice(
            FIELD_TEMPLATE,
            "Template",
            "← → to change, Space for the list",
            templates.to_vec(),
            template_at,
        ),
        Field::text(
            FIELD_TARGET,
            "Target folder",
            "an existing folder — the template fills in what it lacks, and never overwrites",
            String::new(),
        ),
    ])
}

impl App {
    /// The templates on disk, by slug. Deliberately not `templates.cards`,
    /// which also carries a bare card for every slug the projects mention that
    /// no template answers to — `(registered)` is a slug, not a template.
    pub(super) fn template_slugs(&self) -> Vec<String> {
        self.summary
            .as_ref()
            .map(|summary| {
                summary
                    .templates
                    .iter()
                    .map(|card| card.slug.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn prefs(&self) -> Prefs {
        self.summary
            .as_ref()
            .map(|summary| summary.prefs.clone())
            .unwrap_or_default()
    }

    /// The bases a new project could go in, the configured default first —
    /// which is what makes a plain Enter mean exactly what it always meant.
    fn base_options(&self) -> Vec<String> {
        let Some(summary) = &self.summary else {
            return Vec::new();
        };
        let mut bases: Vec<&data::BaseInfo> = summary
            .bases
            .iter()
            .filter(|base| base.probe.usable())
            .collect();
        bases.sort_by_key(|base| !base.is_default);
        bases
            .iter()
            .map(|base| crate::util::paths::display_path(&base.path))
            .collect()
    }

    /// `n`: the new-project wizard.
    pub(super) fn open_create(&mut self) -> Vec<Effect> {
        let slugs = self.template_slugs();
        if slugs.is_empty() {
            self.warn(command::NO_TEMPLATES);
            return Vec::new();
        }
        let default = self.prefs().default_template;
        let at = slugs.iter().position(|slug| *slug == default).unwrap_or(0);
        let mut flow = Flow::new(
            FlowKind::Create,
            create_form(&slugs, at, &self.base_options()),
        );
        flow.auto_commit = !self.prefs().confirm_create;
        flow.pending = true;
        let slug = slugs[at].clone();
        self.modals.push(Modal::Flow(Box::new(flow)));
        vec![Effect::LoadTemplate { slug }]
    }

    /// The apply flow: a template over a folder that already exists.
    pub(super) fn open_apply(&mut self) -> Vec<Effect> {
        let slugs = self.template_slugs();
        if slugs.is_empty() {
            self.warn(command::NO_TEMPLATES);
            return Vec::new();
        }
        let default = self.prefs().default_template;
        let at = slugs.iter().position(|slug| *slug == default).unwrap_or(0);
        let mut flow = Flow::new(FlowKind::Apply, apply_form(&slugs, at));
        flow.pending = true;
        let slug = slugs[at].clone();
        self.modals.push(Modal::Flow(Box::new(flow)));
        vec![Effect::LoadTemplate { slug }]
    }

    pub(super) fn on_flow_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top() else {
            return Vec::new();
        };
        if flow.step == Step::Preview {
            return self.on_preview_key(key);
        }
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let event = flow.form.apply(&key);
        let rows = flow.form.rows();
        flow.form.clamp_viewport(rows.min(12));
        match event {
            FormEvent::Cancel => {
                let kind = flow.kind;
                self.modals.pop();
                self.info(kind.cancelled());
                Vec::new()
            }
            FormEvent::Submit => self.submit_flow(),
            FormEvent::Pick => self.open_field_picker(),
            FormEvent::Changed => self.on_form_changed(),
            // **A form does not fall through to the registry, and the preview
            // does.** The difference is that a form is a place you type into:
            // on a choice field every letter is `Ignored`, so handing those to
            // `lookup_and_run` would make `q` — `Close` in every context — throw
            // away a form somebody had filled in, with no question and no undo.
            // Esc is the form's own cancel and its key line says so. The
            // preview has nothing to type into, which is why `?` and `q` work
            // there.
            FormEvent::Moved | FormEvent::Ignored => Vec::new(),
        }
    }

    /// Keys on the preview half: Esc goes back to the answers (the app's Esc
    /// ladder — one step at a time, nothing typed is lost), Enter commits.
    fn on_preview_key(&mut self, key: Key) -> Vec<Effect> {
        let delta: isize = match key.code {
            KeyCode::Esc => {
                if let Some(Modal::Flow(flow)) = self.modals.top_mut() {
                    flow.step = Step::Form;
                    flow.scroll = 0;
                }
                return Vec::new();
            }
            KeyCode::Enter => return self.commit_flow(),
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::PageDown | KeyCode::Char(' ') => 10,
            KeyCode::PageUp => -10,
            KeyCode::Home => isize::MIN / 2,
            KeyCode::End => isize::MAX / 2,
            // Anything the preview does not consume is whatever the registry
            // binds where the keys are: `?` for the help, `q` to close. There
            // is nothing to type into on this step, so swallowing them said
            // nothing and did nothing.
            _ => return self.lookup_and_run(key),
        };
        // Clamped **here**, against the geometry `view` draws with. Only the
        // view clamped before, so ten PgDns over a short preview drove `scroll`
        // to 100 with nothing moving on screen, and the next ten PgUps did
        // nothing either — a dialog that reads as frozen. `layout.rs`'s whole
        // job is that a cursor cannot leave the drawn window.
        let max = match self.modals.top() {
            Some(Modal::Flow(flow)) => {
                crate::tui::view::modals::preview_max_scroll(self, flow) as isize
            }
            _ => 0,
        };
        if let Some(Modal::Flow(flow)) = self.modals.top_mut() {
            flow.scroll = (flow.scroll as isize + delta).clamp(0, max) as usize;
        }
        Vec::new()
    }

    /// A value changed: the template field decides which variables are asked
    /// for, and register's scope decides which questions apply at all.
    pub(super) fn on_form_changed(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if flow.kind == FlowKind::Register {
            register::sync_visibility(flow);
        }
        let focused = flow.form.focused().map(|field| field.key.clone());
        if focused.as_deref() != Some(FIELD_TEMPLATE) {
            return Vec::new();
        }
        self.load_flow_template()
    }

    /// Read the template the form now names, and rebuild its variable fields.
    fn load_flow_template(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        match flow.template_slug() {
            Some(slug) => {
                flow.pending = true;
                vec![Effect::LoadTemplate { slug }]
            }
            None => {
                flow.pending = false;
                flow.set_template(None);
                Vec::new()
            }
        }
    }

    pub(super) fn on_template_loaded(
        &mut self,
        slug: &str,
        result: Result<Box<data::TemplateInfo>, String>,
    ) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        // A slower read for a template the form has already moved off is an
        // answer to a question nobody is asking any more.
        if flow.template_slug().as_deref() != Some(slug) {
            return Vec::new();
        }
        flow.pending = false;
        match result {
            Ok(info) => {
                flow.set_template(Some(*info));
                if flow.kind == FlowKind::Register {
                    register::sync_visibility(flow);
                }
            }
            Err(error) => {
                flow.set_template(None);
                flow.form.fail(Some(FIELD_TEMPLATE), error);
            }
        }
        Vec::new()
    }

    pub(super) fn on_previewed(&mut self, preview: Preview) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        flow.pending = false;
        flow.preview = Some(preview);
        // `confirm_create = false` is a standing answer to the question the
        // preview asks, so it is not asked: the plan was still built, by the
        // same code path, and every refusal it can produce still lands on the
        // field that caused it.
        if flow.auto_commit {
            return self.commit_flow();
        }
        flow.step = Step::Preview;
        flow.scroll = 0;
        Vec::new()
    }

    /// Space on a choice: the same options as a fuzzy-filtered picker.
    fn open_field_picker(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top() else {
            return Vec::new();
        };
        let Some(field) = flow.form.focused() else {
            return Vec::new();
        };
        let crate::tui::widgets::form::FieldKind::Choice { options, .. } = &field.kind else {
            return Vec::new();
        };
        let describe = field.key == FIELD_TEMPLATE;
        let cards = self.summary.as_ref().map(|s| s.templates.clone());
        let items: Vec<PickItem> = options
            .iter()
            .map(|option| PickItem {
                label: option.clone(),
                detail: if describe {
                    cards
                        .as_ref()
                        .and_then(|cards| cards.iter().find(|card| &card.slug == option))
                        .map(|card| card.description.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                },
                value: option.clone(),
            })
            .collect();
        let title = field.label.clone();
        let key = field.key.clone();
        self.modals.push(Modal::Pick(PickState::new(
            title,
            items,
            Then::FormField(key),
        )));
        Vec::new()
    }

    /// Enter on the form: check what `update` can check, then ask a worker for
    /// the preview — which is where a path that does not exist is refused,
    /// because looking is I/O and `update` does none.
    fn submit_flow(&mut self) -> Vec<Effect> {
        let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
            return Vec::new();
        };
        if flow.pending {
            return Vec::new();
        }
        if let Some((key, message)) = flow.missing_required() {
            flow.form.fail(Some(&key), message);
            return Vec::new();
        }
        let Some(request) = self.flow_request() else {
            return Vec::new();
        };
        if let Some(Modal::Flow(flow)) = self.modals.top_mut() {
            flow.pending = true;
            flow.form.clear_errors();
        }
        vec![Effect::Preview(Box::new(request))]
    }

    /// The open flow's answers, as the request both the preview and the commit
    /// are built from.
    fn flow_request(&self) -> Option<Request> {
        let Some(Modal::Flow(flow)) = self.modals.top() else {
            return None;
        };
        match flow.kind {
            FlowKind::Create => Some(Request::Create(CreateRequest {
                template_slug: flow.template_slug()?,
                vars: flow.variables(),
                base_dir_override: self.chosen_base(flow),
            })),
            FlowKind::Apply => Some(Request::Apply(ApplyRequest {
                template_slug: flow.template_slug()?,
                target: PathBuf::from(flow.form.value(FIELD_TARGET).trim()),
                vars: flow.variables(),
            })),
            FlowKind::Register => Some(Request::Register(register::request(flow))),
            FlowKind::FromFolder => {
                Some(Request::FromFolder(crate::tui::effect::FromFolderRequest {
                    source: PathBuf::from(flow.form.value(FIELD_SOURCE).trim()),
                    slug: flow.form.value(FIELD_SLUG).trim().to_string(),
                    force: flow.form.is_on(FIELD_FORCE),
                    bundle_assets: flow.form.is_on(FIELD_BUNDLE),
                }))
            }
        }
    }

    /// The base the create form names, or `None` for the configured default —
    /// the same distinction `pick_base_interactively` drew by returning early
    /// when there was only one base to offer.
    fn chosen_base(&self, flow: &Flow) -> Option<String> {
        let chosen = flow.form.value(FIELD_BASE);
        if chosen.is_empty() {
            return None;
        }
        let default = self.base_options().into_iter().next();
        (Some(&chosen) != default.as_ref()).then_some(chosen)
    }

    /// Enter on the preview: run it.
    fn commit_flow(&mut self) -> Vec<Effect> {
        let Some(request) = self.flow_request() else {
            return Vec::new();
        };
        self.modals.pop();
        match request {
            Request::Create(request) => {
                self.run_action("creating…", Action::Create(Box::new(request)))
            }
            Request::Apply(request) => {
                self.run_action("applying…", Action::Apply(Box::new(request)))
            }
            Request::Register(request) => {
                self.run_action("registering…", Action::Register(Box::new(request)))
            }
            Request::FromFolder(request) => self.run_action(
                "generating the template…",
                Action::TemplateFromFolder(Box::new(request)),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::data::VarInfo;

    fn info(slug: &str, vars: &[(&str, bool)]) -> TemplateInfo {
        TemplateInfo {
            slug: slug.to_string(),
            name: slug.to_string(),
            naming_pattern: "{date}_{name}_{id}".to_string(),
            variables: vars
                .iter()
                .map(|(name, required)| VarInfo {
                    slug: (*name).to_string(),
                    label: (*name).to_string(),
                    required: *required,
                    options: Vec::new(),
                    default: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn changing_template_keeps_the_answers_the_new_one_still_asks_for() {
        let mut flow = Flow::new(
            FlowKind::Create,
            create_form(
                &["music-video".into(), "general".into()],
                0,
                &["base".into()],
            ),
        );
        flow.set_template(Some(info(
            "music-video",
            &[("artist", true), ("title", true)],
        )));
        flow.form.field_mut("var:artist").unwrap().set_text("Aria");
        flow.form
            .field_mut("var:title")
            .unwrap()
            .set_text("Lullaby");

        flow.set_template(Some(info("general", &[("artist", true)])));
        assert_eq!(flow.form.value("var:artist"), "Aria");
        assert!(
            flow.form.field("var:title").is_none(),
            "a variable the new template does not have is gone"
        );
    }

    #[test]
    fn a_required_variable_left_empty_is_named_before_any_preview() {
        let mut flow = Flow::new(FlowKind::Create, create_form(&["t".into()], 0, &[]));
        flow.set_template(Some(info("t", &[("artist", true), ("note", false)])));
        let (key, message) = flow.missing_required().expect("artist is required");
        assert_eq!(key, "var:artist");
        assert!(message.contains("required"), "{message}");
        flow.form.field_mut("var:artist").unwrap().set_text("A");
        assert!(flow.missing_required().is_none());
    }

    #[test]
    fn the_template_field_answers_none_as_no_template() {
        let mut flow = Flow::new(
            FlowKind::Register,
            Form::new(vec![Field::choice(
                FIELD_TEMPLATE,
                "Template",
                "",
                vec![NO_TEMPLATE.to_string(), "general".to_string()],
                0,
            )]),
        );
        assert_eq!(flow.template_slug(), None);
        flow.form.field_mut(FIELD_TEMPLATE).unwrap().step(1);
        assert_eq!(flow.template_slug().as_deref(), Some("general"));
    }
}
