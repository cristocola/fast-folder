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

use crate::core::project::DryRunReport;
use crate::tui::app::data::TemplateInfo;
use crate::tui::widgets::form::{Field, Form};

/// The field every flow that names a template uses.
pub const FIELD_TEMPLATE: &str = "template";
/// The base a new project is created in; present only with a choice to make.
pub const FIELD_BASE: &str = "base";
/// The folder `apply` fills in.
pub const FIELD_TARGET: &str = "target";
/// A variable field's key is this plus the variable's slug.
pub const VAR_PREFIX: &str = "var:";

/// The template choice's "no template at all" entry — register's own answer,
/// which writes a minimal record with the `(registered)` slug.
pub const NO_TEMPLATE: &str = "(none)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowKind {
    Create,
    Apply,
    Register,
}

impl FlowKind {
    pub fn title(self) -> &'static str {
        match self {
            FlowKind::Create => "new project",
            FlowKind::Apply => "apply a template",
            FlowKind::Register => "register a folder",
        }
    }

    /// What a cancel says. The words are the dialoguer flows' own, so a
    /// cancelled run reads the same wherever it happened.
    pub fn cancelled(self) -> &'static str {
        match self {
            FlowKind::Create => "Cancelled — nothing was created.",
            FlowKind::Apply => "Cancelled — nothing was applied.",
            FlowKind::Register => "Cancelled — nothing was registered.",
        }
    }

    /// The verb on the preview, where Enter commits.
    pub fn commit(self) -> &'static str {
        match self {
            FlowKind::Create => "Enter creates it",
            FlowKind::Apply => "Enter applies it",
            FlowKind::Register => "Enter registers it",
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
/// dialoguer's `[default]` meant, made editable instead of invisible.
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
