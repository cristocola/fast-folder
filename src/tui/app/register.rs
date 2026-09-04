//! Register's half of the flow machinery: its fields, and the request its
//! answers make.
//!
//! Register asks more than the other two flows and asks it conditionally —
//! bulk registration never renames, never applies and never takes a date other
//! than each folder's own, which `RegisterFlags::validate` refuses on the
//! command line. On one screen that is not a branch through a sequence of
//! prompts but a scope field whose value hides the three questions it makes
//! meaningless.

use std::path::PathBuf;

use crate::tui::app::wizard::{FIELD_TEMPLATE, Flow, NO_TEMPLATE};
use crate::tui::widgets::form::{Field, Form};

/// One folder, or every unregistered child of one.
pub const FIELD_SCOPE: &str = "scope";
/// The folder to register, or the base whose children to register.
pub const FIELD_PATH: &str = "path";
/// Rename the folder to the naming pattern.
pub const FIELD_RENAME: &str = "rename";
/// Which date the record claims.
pub const FIELD_CREATED: &str = "created";
/// The date typed when `Created` is "a date I type" — what `--created` is on
/// the command line.
pub const FIELD_CREATED_DATE: &str = "created_date";
/// Fill in the template's missing folders and files.
pub const FIELD_APPLY: &str = "apply";

pub const SCOPE_ONE: &str = "one folder";
pub const SCOPE_RECURSIVE: &str = "every unregistered folder in a base";

pub const CREATED_OWN: &str = "the folder's own date";
pub const CREATED_TODAY: &str = "today";
pub const CREATED_TYPED: &str = "a date I type";

/// The register form. `templates` already carries [`NO_TEMPLATE`] first.
pub fn register_form(templates: &[String]) -> Form {
    Form::new(vec![
        Field::choice(
            FIELD_SCOPE,
            "Register",
            "← → to change: one folder, or every unregistered child of a base",
            vec![SCOPE_ONE.to_string(), SCOPE_RECURSIVE.to_string()],
            0,
        ),
        Field::text(
            FIELD_PATH,
            "Folder",
            "the folder to adopt — it must already exist",
            String::new(),
        ),
        Field::choice(
            FIELD_TEMPLATE,
            "Template",
            "optional: a template enables tags and variable capture",
            templates.to_vec(),
            0,
        ),
        Field::toggle(
            FIELD_RENAME,
            "Standardize name",
            "rename the folder to the naming pattern — the preview shows the new name first",
            false,
        ),
        Field::choice(
            FIELD_CREATED,
            "Created",
            "a folder that predates fastf should usually keep its own date",
            vec![
                CREATED_OWN.to_string(),
                CREATED_TODAY.to_string(),
                CREATED_TYPED.to_string(),
            ],
            0,
        ),
        Field::text(
            FIELD_CREATED_DATE,
            "Date",
            "YYYY-MM-DD — the date the record will claim",
            String::new(),
        )
        .hidden(true),
        Field::toggle(
            FIELD_APPLY,
            "Fill in the template",
            "create the template's missing folders and files — never overwrites",
            false,
        )
        .hidden(true),
    ])
}

/// Whether the form is asking about a whole base.
pub fn is_recursive(form: &Form) -> bool {
    form.value(FIELD_SCOPE) == SCOPE_RECURSIVE
}

/// Show only the questions the current answers leave meaningful: bulk
/// registration never renames and never applies, and "fill in the template"
/// says nothing without a template.
pub fn sync_visibility(flow: &mut Flow) {
    let recursive = is_recursive(&flow.form);
    let has_template = flow.form.value(FIELD_TEMPLATE) != NO_TEMPLATE;
    let typed_date = flow.form.value(FIELD_CREATED) == CREATED_TYPED;
    flow.form.set_hidden(FIELD_RENAME, recursive);
    flow.form
        .set_hidden(FIELD_APPLY, recursive || !has_template);
    flow.form
        .set_hidden(FIELD_CREATED_DATE, recursive || !typed_date);
    if let Some(field) = flow.form.field_mut(FIELD_PATH) {
        field.label = if recursive { "Base" } else { "Folder" }.to_string();
        field.hint = if recursive {
            "the base folder whose direct children to register"
        } else {
            "the folder to adopt — it must already exist"
        }
        .to_string();
    }
}

/// What the form asks for, as the request a worker previews and commits.
pub fn request(flow: &Flow) -> Request {
    Request {
        path: PathBuf::from(flow.form.value(FIELD_PATH).trim()),
        template_slug: flow.template_slug(),
        vars: flow.variables(),
        apply_structure: flow.form.is_on(FIELD_APPLY),
        rename: flow.form.is_on(FIELD_RENAME),
        use_today: flow.form.value(FIELD_CREATED) == CREATED_TODAY,
        created_override: (flow.form.value(FIELD_CREATED) == CREATED_TYPED)
            .then(|| flow.form.value(FIELD_CREATED_DATE).trim().to_string())
            .filter(|date| !date.is_empty()),
        recursive: is_recursive(&flow.form),
    }
}

/// Everything a register — of one folder or of a whole base — needs. One type
/// for both, because the scope is an answer like any other and the engine
/// underneath takes the same values either way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub path: PathBuf,
    pub template_slug: Option<String>,
    pub vars: std::collections::HashMap<String, String>,
    pub apply_structure: bool,
    pub rename: bool,
    pub use_today: bool,
    /// A date typed for the record, as `--created` gives one.
    pub created_override: Option<String>,
    pub recursive: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::wizard::{Flow, FlowKind};

    fn flow() -> Flow {
        Flow::new(
            FlowKind::Register,
            register_form(&[NO_TEMPLATE.to_string(), "general".to_string()]),
        )
    }

    #[test]
    fn a_recursive_scope_hides_what_bulk_registration_never_does() {
        let mut flow = flow();
        flow.form.field_mut(FIELD_TEMPLATE).unwrap().step(1);
        sync_visibility(&mut flow);
        assert!(!flow.form.field(FIELD_RENAME).unwrap().hidden);
        assert!(!flow.form.field(FIELD_APPLY).unwrap().hidden);

        flow.form.field_mut(FIELD_SCOPE).unwrap().step(1);
        sync_visibility(&mut flow);
        assert!(flow.form.field(FIELD_RENAME).unwrap().hidden);
        assert!(flow.form.field(FIELD_APPLY).unwrap().hidden);
        assert_eq!(flow.form.field(FIELD_PATH).unwrap().label, "Base");
        assert!(request(&flow).recursive);
    }

    #[test]
    fn fill_in_the_template_needs_a_template() {
        let mut flow = flow();
        sync_visibility(&mut flow);
        assert!(
            flow.form.field(FIELD_APPLY).unwrap().hidden,
            "there is no template to fill in from"
        );
        assert_eq!(request(&flow).template_slug, None);
    }

    #[test]
    fn the_answers_become_the_request() {
        let mut flow = flow();
        flow.form.field_mut(FIELD_PATH).unwrap().set_text("/tmp/x");
        flow.form.field_mut(FIELD_TEMPLATE).unwrap().step(1);
        flow.form.field_mut(FIELD_RENAME).unwrap().step(1);
        flow.form.field_mut(FIELD_CREATED).unwrap().step(1);
        let request = request(&flow);
        assert_eq!(request.path, PathBuf::from("/tmp/x"));
        assert_eq!(request.template_slug.as_deref(), Some("general"));
        assert!(request.rename && request.use_today && !request.recursive);
    }
}
