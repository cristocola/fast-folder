//! The flows: create, apply, register.

use crate::harness::*;
use fastf::tui::app::data::{TemplateInfo, VarInfo};
use fastf::tui::app::wizard::{FIELD_TARGET, FIELD_TEMPLATE, FlowKind, Step};
use fastf::tui::effect::Request;

fn template(slug: &str, vars: &[(&str, bool)]) -> TemplateInfo {
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

/// Answer the template read the flow asked for.
fn land_template(app: &mut App, slug: &str, vars: &[(&str, bool)]) {
    let _ = update(
        app,
        Msg::TemplateLoaded {
            slug: slug.to_string(),
            result: Ok(Box::new(template(slug, vars))),
        },
    );
}

fn flow_kind(app: &App) -> FlowKind {
    match app.modals.top() {
        Some(Modal::Flow(flow)) => flow.kind,
        other => panic!("expected a flow, got {other:?}"),
    }
}

fn flow_step(app: &App) -> Step {
    match app.modals.top() {
        Some(Modal::Flow(flow)) => flow.step,
        other => panic!("expected a flow, got {other:?}"),
    }
}

fn form_error(app: &App) -> Option<String> {
    match app.modals.top() {
        Some(Modal::Flow(flow)) => flow.form.error().map(str::to_string),
        other => panic!("expected a flow, got {other:?}"),
    }
}

#[test]
fn n_opens_the_wizard_on_the_first_template_and_reads_it() {
    let mut app = fixture(6, 120, 40);
    let effects = press(&mut app, Key::ch('n'));
    assert_eq!(flow_kind(&app), FlowKind::Create);
    assert!(
        matches!(&effects[..], [Effect::LoadTemplate { slug }] if slug == "general"),
        "the wizard reads the template it opened on: {effects:?}"
    );
    land_template(&mut app, "general", &[("name", true)]);
    match app.modals.top() {
        Some(Modal::Flow(flow)) => {
            assert!(!flow.pending);
            assert!(
                flow.form.field("var:name").is_some(),
                "its variable is asked for"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_required_variable_is_refused_before_any_preview_is_asked_for() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('n'));
    land_template(&mut app, "general", &[("name", true)]);
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        effects.is_empty(),
        "nothing was asked of a worker: {effects:?}"
    );
    assert_eq!(form_error(&app).as_deref(), Some("name is required"));
    assert_eq!(flow_step(&app), Step::Form);
}

#[test]
fn the_answers_become_a_preview_request_and_then_a_create() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('n'));
    land_template(&mut app, "general", &[("name", true)]);
    press(&mut app, Key::plain(KeyCode::Tab));
    type_text(&mut app, "Lullaby");

    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let request = match &effects[..] {
        [Effect::Preview(request)] => (**request).clone(),
        other => panic!("expected a preview request, got {other:?}"),
    };
    match &request {
        Request::Create(create) => {
            assert_eq!(create.template_slug, "general");
            assert_eq!(create.vars.get("name").map(String::as_str), Some("Lullaby"));
            assert_eq!(
                create.base_dir_override, None,
                "the default base is not an override"
            );
        }
        other => panic!("expected a create, got {other:?}"),
    }

    // The preview lands; Enter on it commits the very same request.
    let _ = update(&mut app, Msg::Previewed(Box::new(sample_create_preview())));
    assert_eq!(flow_step(&app), Step::Preview);
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    match action_of(&effects) {
        Action::Create(create) => assert_eq!(create.template_slug, "general"),
        other => panic!("expected a create action, got {other:?}"),
    }
    assert!(app.modals.is_empty(), "the flow closed when it committed");
}

#[test]
fn confirm_create_false_commits_without_showing_the_plan() {
    let mut app = fixture(6, 120, 40);
    let mut summary = sample_summary(6);
    summary.prefs.confirm_create = false;
    app.summary = Some(summary);
    press(&mut app, Key::ch('n'));
    land_template(&mut app, "general", &[]);
    press(&mut app, Key::plain(KeyCode::Enter));

    let effects = update(&mut app, Msg::Previewed(Box::new(sample_create_preview())));
    assert!(
        matches!(action_of(&effects), Action::Create(_)),
        "the plan was still built, and then committed unasked: {effects:?}"
    );
    assert!(app.modals.is_empty());
}

#[test]
fn esc_at_the_answers_cancels_and_esc_at_the_preview_goes_back_to_them() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('n'));
    land_template(&mut app, "general", &[]);
    press(&mut app, Key::plain(KeyCode::Enter));
    let _ = update(&mut app, Msg::Previewed(Box::new(sample_create_preview())));
    assert_eq!(flow_step(&app), Step::Preview);

    press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(flow_step(&app), Step::Form, "one step back, nothing lost");

    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.modals.is_empty());
    assert_eq!(app.status.text, "Cancelled — nothing was created.");
}

#[test]
fn a_worker_refusal_lands_on_the_field_that_caused_it() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('E'));
    land_template(&mut app, "general", &[]);
    press(&mut app, Key::plain(KeyCode::Tab));
    type_text(&mut app, "/nope");
    press(&mut app, Key::plain(KeyCode::Enter));

    let _ = update(
        &mut app,
        Msg::PreviewFailed {
            field: Some(FIELD_TARGET.to_string()),
            error: "no such folder: /nope".to_string(),
        },
    );
    assert_eq!(flow_step(&app), Step::Form);
    assert_eq!(form_error(&app).as_deref(), Some("no such folder: /nope"));
    match app.modals.top() {
        Some(Modal::Flow(flow)) => {
            assert_eq!(flow.form.value(FIELD_TARGET), "/nope", "the text stays");
            assert_eq!(flow.form.focused().unwrap().key, FIELD_TARGET);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn changing_the_template_reads_the_new_one_and_keeps_the_shared_answers() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('n'));
    land_template(&mut app, "general", &[("name", true)]);
    press(&mut app, Key::plain(KeyCode::Tab));
    type_text(&mut app, "Lullaby");
    // Back to the template field, and on to the next template.
    press(&mut app, Key::plain(KeyCode::BackTab));
    let effects = press(&mut app, Key::plain(KeyCode::Right));
    assert!(
        matches!(&effects[..], [Effect::LoadTemplate { slug }] if slug == "music-video"),
        "{effects:?}"
    );
    land_template(&mut app, "music-video", &[("name", true), ("artist", true)]);
    match app.modals.top() {
        Some(Modal::Flow(flow)) => {
            assert_eq!(flow.form.value("var:name"), "Lullaby");
            assert_eq!(flow.form.value("var:artist"), "");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn space_on_a_choice_opens_a_picker_that_answers_the_field() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('n'));
    land_template(&mut app, "general", &[]);
    press(&mut app, Key::ch(' '));
    match app.modals.top() {
        Some(Modal::Pick(pick)) => assert_eq!(pick.items.len(), 3, "every template"),
        other => panic!("expected a picker over the options, got {other:?}"),
    }
    type_text(&mut app, "music");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(&effects[..], [Effect::LoadTemplate { slug }] if slug == "music-video"),
        "{effects:?}"
    );
    match app.modals.top() {
        Some(Modal::Flow(flow)) => assert_eq!(flow.form.value(FIELD_TEMPLATE), "music-video"),
        other => panic!("expected the flow back, got {other:?}"),
    }
}

#[test]
fn register_hides_what_bulk_registration_never_does() {
    use fastf::tui::app::register::{FIELD_APPLY, FIELD_RENAME, FIELD_SCOPE};

    let mut app = fixture(6, 120, 40);
    let effects = press(&mut app, Key::ch('e'));
    assert!(
        effects.is_empty(),
        "no template is read until one is chosen"
    );
    assert_eq!(flow_kind(&app), FlowKind::Register);
    press(&mut app, Key::plain(KeyCode::Right)); // scope → recursive
    match app.modals.top() {
        Some(Modal::Flow(flow)) => {
            assert_eq!(
                flow.form.value(FIELD_SCOPE),
                "every unregistered folder in a base"
            );
            assert!(flow.form.field(FIELD_RENAME).unwrap().hidden);
            assert!(flow.form.field(FIELD_APPLY).unwrap().hidden);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_created_project_is_selected_once_discovery_has_seen_it() {
    let mut app = fixture(6, 120, 40);
    let fresh = sample_projects(7).pop().unwrap();
    let path = fresh.path.clone();
    // Any action will do: what is under test is what its outcome asks for.
    let id = run_id(&press(&mut app, Key::ch('R')));
    // What a finished create reports: reload the list, and put the cursor
    // on the row that will appear in it.
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(
                fastf::tui::effect::ActionOutcome::new(ListChange::Reload, "created")
                    .select(path.clone()),
            )),
        },
    );
    assert_eq!(app.select_when_found.as_ref(), Some(&path));
    let generation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Discover { generation } => Some(*generation),
            _ => None,
        })
        .expect("a reload discovers");

    let mut projects = sample_projects(6);
    projects.push(fresh);
    let _ = update(
        &mut app,
        Msg::Discovered {
            generation,
            projects,
        },
    );
    assert_eq!(
        app.library.selected().map(|p| p.path.clone()),
        Some(path),
        "the cursor lands on what was just made"
    );
    assert!(app.select_when_found.is_none());
}

fn sample_create_preview() -> fastf::tui::app::wizard::Preview {
    use fastf::core::project::DryRunReport;
    fastf::tui::app::wizard::Preview::Create(Box::new(DryRunReport {
        folder_name: "2026-09-03_Lullaby_ID0249".to_string(),
        root_path: PathBuf::from("/mnt/projects/2026-09-03_Lullaby_ID0249"),
        structure: vec![fastf::core::template::FolderNode {
            name: "00_Inbox".to_string(),
            children: Vec::new(),
        }],
        files: vec!["BRIEF.md".to_string()],
        values: vec![fastf::core::project::ResolvedValue {
            slug: "name".to_string(),
            value: "Lullaby".to_string(),
            transform: None,
        }],
        id: "ID0249".to_string(),
        counter: (248, 249),
        date: "2026-09-03".to_string(),
        date_parts: ("2026".to_string(), "09".to_string(), "03".to_string()),
        previews: Vec::new(),
    }))
}
