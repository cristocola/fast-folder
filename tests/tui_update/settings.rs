//! Settings, the counter, maintenance and the first run.

use crate::harness::*;
use fastf::tui::app::data::Settings;
use fastf::tui::app::modal::Modal;
use fastf::tui::app::settings::{Editing, Job, Kind, SettingsState};

fn sample() -> Settings {
    Settings {
        base_dir: "/mnt/projects".to_string(),
        date_format: "%Y-%m-%d".to_string(),
        date_preview: "2026-09-03".to_string(),
        preview_lines: 20,
        confirm_create: true,
        recent_default_limit: 20,
        register_naming_pattern: "{date}_{name}_{id}".to_string(),
        on_name_collision: "suffix".to_string(),
        counter_floor: 248,
        next_id: "ID0249".to_string(),
        data_dir: "/home/user/.config/fastf".to_string(),
        ..Settings::default()
    }
}

fn state(app: &App) -> &SettingsState {
    match app.modals.top() {
        Some(Modal::Settings(state)) => state,
        other => panic!("expected the settings, got {other:?}"),
    }
}

/// `,` asks for the settings, and the screen opens when they land.
fn open(app: &mut App) {
    let effects = press(app, Key::ch(','));
    assert_eq!(effects, vec![Effect::LoadSettings]);
    let _ = update(app, Msg::SettingsLoaded(Box::new(sample())));
}

fn go_to(app: &mut App, label: &str) {
    for _ in 0..40 {
        if state(app).row().unwrap().label == label {
            return;
        }
        press(app, Key::plain(KeyCode::Down));
    }
    panic!("no row called {label}");
}

#[test]
fn the_screen_opens_on_the_settings_that_were_read() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    assert_eq!(state(&app).row().unwrap().label, "Base directory");
    assert_eq!(state(&app).row().unwrap().value, "/mnt/projects");
}

#[test]
fn a_toggle_writes_its_key_with_no_dialog_at_all() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Confirm before creating");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::SetConfig { key, value }
            if *key == "confirm-create" && value == "false")
    );
    assert!(
        state(&app).editing.is_none(),
        "a yes/no is answered where it stands"
    );
}

#[test]
fn a_text_field_opens_edits_and_writes_the_config_key() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Default template");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(state(&app).editing, Some(Editing::Value { .. })));
    type_text(&mut app, "music-video");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::SetConfig { key, value }
            if *key == "default-template" && value == "music-video")
    );
}

#[test]
fn esc_in_a_field_leaves_the_value_alone() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Editor");
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "nvim");
    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(effects.is_empty(), "nothing was written: {effects:?}");
    assert!(state(&app).editing.is_none());
    assert!(!app.modals.is_empty(), "and the screen is still open");
}

#[test]
fn a_refusal_lands_under_the_value_that_earned_it() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Recent limit");
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "0");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id = run_id(&effects);
    let _ = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("recent_limit must be at least 1".to_string()),
        },
    );
    assert_eq!(state(&app).error(), Some("recent_limit must be at least 1"));
    assert!(
        state(&app).editing.is_some(),
        "the field stays open with the text in it"
    );
    assert!(app.status.text.is_empty(), "and it is not a status toast");
}

#[test]
fn the_bases_are_one_text_area_and_enter_is_a_newline() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Bases");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(state(&app).editing, Some(Editing::Bases { .. })));
    type_text(&mut app, "/mnt/one");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(effects.is_empty(), "Enter is a newline in a list");
    type_text(&mut app, "/mnt/two");
    let effects = press(&mut app, Key::ctrl('s'));
    assert!(
        matches!(action_of(&effects), Action::SetConfig { key, value }
            if *key == "bases" && value == "/mnt/one,/mnt/two")
    );
}

#[test]
fn a_write_re_reads_the_settings_rather_than_trusting_what_was_typed() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Confirm before creating");
    let id = run_id(&press(&mut app, Key::plain(KeyCode::Enter)));
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(
                fastf::tui::effect::ActionOutcome::new(
                    ListChange::SummaryOnly,
                    "Set confirm_create = false",
                )
                .settings(),
            )),
        },
    );
    assert!(effects.contains(&Effect::LoadSettings), "{effects:?}");
    assert!(effects.contains(&Effect::LoadSummary), "{effects:?}");
}

#[test]
fn the_maintenance_rows_run_rather_than_set() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Reindex");
    assert_eq!(state(&app).row().unwrap().kind, Kind::Run(Job::Reindex));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id = run_id(&effects);
    assert!(matches!(action_of(&effects), Action::Reindex));
    let _ = update(&mut app, item_done(id, ListChange::None));

    press(&mut app, Key::plain(KeyCode::Down));
    assert_eq!(state(&app).row().unwrap().label, "Check and recover");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(action_of(&effects), Action::Reconcile));
}

#[test]
fn the_counter_is_raised_through_a_prompt_that_names_the_floor() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Counter");
    press(&mut app, Key::plain(KeyCode::Enter));
    match app.modals.top() {
        Some(Modal::TextPrompt(prompt)) => {
            assert!(prompt.title.contains("248"), "{}", prompt.title);
            assert_eq!(prompt.input.text(), "248");
        }
        other => panic!("expected the counter prompt, got {other:?}"),
    }
    type_text(&mut app, "9");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(action_of(&effects), Action::RaiseCounter(2489)));
}

#[test]
fn needs_attention_is_the_recover_command() {
    let mut app = fixture(6, 120, 40);
    let effects = press(&mut app, Key::ch('!'));
    assert!(matches!(action_of(&effects), Action::Reconcile));
}

#[test]
fn the_first_run_asks_once_and_an_empty_answer_skips() {
    let mut app = fixture(0, 120, 40);
    app.request_onboarding("/home/user/Projects".to_string());
    assert!(matches!(app.modals.top(), Some(Modal::Onboarding(_))));

    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::InitBaseDir(path) if path == "/home/user/Projects")
    );
    assert!(
        matches!(app.modals.top(), Some(Modal::Onboarding(_))),
        "the question stays up until the folder exists"
    );

    // A refusal keeps it open with the text and the reason.
    let id = run_id(&effects);
    let _ = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("permission denied".to_string()),
        },
    );
    match app.modals.top() {
        Some(Modal::Onboarding(state)) => {
            assert_eq!(state.error.as_deref(), Some("permission denied"));
            assert_eq!(state.input.text(), "/home/user/Projects");
        }
        other => panic!("{other:?}"),
    }

    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.modals.is_empty());
    assert!(app.status.text.contains("Skipped"), "{}", app.status.text);
}
