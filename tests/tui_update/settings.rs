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

/// Motion goes off and comes back on. The row cycled from a value it never
/// read back, so every press wrote `off` and the second one did nothing.
#[test]
fn motion_turns_off_and_the_next_press_turns_it_back_on() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Motion");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::SetConfig { key, value }
            if *key == "motion" && value == "off"),
        "{effects:?}"
    );
    let id = run_id(&effects);
    // What the write stored comes back with the re-read.
    let _ = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(
                fastf::tui::effect::ActionOutcome::new(ListChange::SummaryOnly, "Set motion = off")
                    .settings(),
            )),
        },
    );
    let off = Settings {
        motion: "off".to_string(),
        ..sample()
    };
    let _ = update(&mut app, Msg::SettingsLoaded(Box::new(off)));
    assert_eq!(state(&app).row().unwrap().label, "Motion");
    assert_eq!(state(&app).row().unwrap().value, "off");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::SetConfig { key, value }
            if *key == "motion" && value == "on"),
        "{effects:?}"
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
    assert_eq!(state(&app).row().unwrap().label, "Reconcile");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        job_started(&effects),
        Some((fastf::core::jobs::JobKind::Reconcile, _))
    ));
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

/// **`!` lists what is unfinished**, what needs you first, each with its
/// reason; Enter on an item that needs you offers what settles it, and
/// discarding asks for the word before anything is sent.
#[test]
fn bang_lists_the_unfinished_work_and_settles_what_needs_you() {
    use fastf::core::attention::Action as Resolution;
    use fastf::tui::app::modal::Then;
    let mut app = fixture(6, 120, 40);
    assert!(press(&mut app, Key::ch('!')).is_empty());
    let Some(Modal::Pick(page)) = app.modals.top() else {
        panic!("expected the page, got {:?}", app.modals.top());
    };
    assert!(matches!(page.then, Then::Attention));
    assert_eq!(page.title, "Unfinished — 1 needs you");
    assert!(
        page.items[0].label.starts_with("needs you"),
        "{:?}",
        page.items
    );
    assert_eq!(
        page.items.len(),
        1,
        "nothing for fastf to finish: no row for it"
    );

    press(&mut app, Key::plain(KeyCode::Enter));
    let Some(Modal::Pick(actions)) = app.modals.top() else {
        panic!("expected its actions, got {:?}", app.modals.top());
    };
    assert!(matches!(actions.then, Then::AttentionAction(_)));
    let words: Vec<&str> = actions
        .items
        .iter()
        .map(|item| item.value.as_str())
        .collect();
    assert_eq!(words, ["put-back", "discard"]);

    press(&mut app, Key::plain(KeyCode::Down));
    let asked = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(asked.is_empty(), "the word first: {asked:?}");
    type_text(&mut app, "discrad");
    assert!(
        press(&mut app, Key::plain(KeyCode::Enter)).is_empty(),
        "a typo sends nothing"
    );
    for _ in 0.."discrad".len() {
        press(&mut app, Key::plain(KeyCode::Backspace));
    }
    type_text(&mut app, "discard");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::ResolveAttention {
            action: Resolution::Discard,
            ..
        }
    ));
}

/// **The app finishes its own leftovers**, quietly: a summary that shows
/// something fastf can finish, with nothing running, starts a reconcile of
/// the app's own — once, not again within the minute, and never beside a
/// running job — and no dialog comes up for it.
#[test]
fn a_summary_with_leftovers_starts_a_quiet_reconcile() {
    use fastf::core::attention::{Attention, Item, State};
    let mut app = fixture(6, 120, 40);
    let mut summary = sample_summary(6);
    summary.attention = Attention {
        items: vec![Item {
            state: State::Auto,
            what: "a move".to_string(),
            path: PathBuf::from("/srv/projects/.fastf-transactions/1-1-0"),
            project: None,
            reason: "an interrupted move; fastf finishes it".to_string(),
            actions: Vec::new(),
        }],
    };
    let effects = update(&mut app, Msg::Summary(Box::new(summary.clone())));
    assert!(effects.contains(&Effect::StartAutoReconcile), "{effects:?}");
    assert!(app.shown_progress().is_none(), "no dialog for it");
    let again = update(&mut app, Msg::Summary(Box::new(summary.clone())));
    assert!(
        !again.contains(&Effect::StartAutoReconcile),
        "not twice in a minute"
    );
    app.elapsed_ms += 61_000;
    let later = update(&mut app, Msg::Summary(Box::new(summary)));
    assert!(later.contains(&Effect::StartAutoReconcile), "{later:?}");
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

/// **F2 edits here too**: a value opens on its line, as Enter opens it; on a
/// yes/no there is nothing to type, so F2 is not bound there and Enter keeps
/// flipping it.
#[test]
fn f2_opens_a_value_and_leaves_a_toggle_to_enter() {
    let mut app = fixture(6, 120, 40);
    open(&mut app);
    go_to(&mut app, "Date format");
    press(&mut app, Key::plain(KeyCode::F(2)));
    assert!(
        matches!(state(&app).editing, Some(Editing::Value { .. })),
        "F2 opens the value"
    );
    press(&mut app, Key::plain(KeyCode::Esc));

    go_to(&mut app, "Confirm before creating");
    let effects = press(&mut app, Key::plain(KeyCode::F(2)));
    assert!(
        effects.is_empty(),
        "nothing to type on a yes/no: {effects:?}"
    );
    assert!(state(&app).editing.is_none());
}
