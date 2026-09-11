//! What every module of this suite shares: the imports, and the helpers that
//! press keys, read the list back and find the action among the effects.

pub use std::path::{Path, PathBuf};

pub use fastf::core::library::Project;
pub use fastf::tui::app::modal::Modal;
pub use fastf::tui::app::{App, Focus, update};
pub use fastf::tui::command::Key;
pub use fastf::tui::effect::{Action, Effect, Exit, ListChange, SpawnKind, Suspended};
pub use fastf::tui::entry::{Entry, Preset};
pub use fastf::tui::msg::Msg;
pub use fastf::tui::testing::{
    empty_fixture, fixture, sample_projects, sample_summary, sample_summary_moveable,
};
pub use fastf::tui::theme::Theme;
pub use ratatui::crossterm::event::KeyCode;

pub fn press(app: &mut App, key: Key) -> Vec<Effect> {
    update(app, Msg::Key(key))
}

pub fn type_text(app: &mut App, text: &str) -> Vec<Effect> {
    let mut effects = Vec::new();
    for c in text.chars() {
        effects.extend(press(app, Key::ch(c)));
    }
    effects
}

pub fn names(app: &App) -> Vec<String> {
    (0..app.library.len())
        .filter_map(|row| app.library.row(row).map(|p| p.name.clone()))
        .collect()
}

pub fn selected_name(app: &App) -> String {
    app.library
        .selected()
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

/// The one `Effect::Run` among the effects. Exactly one action may be started
/// at a time, but a batch item's outcome also carries the list maintenance its
/// row change asked for (`ForgetSizes`, `RequestSizes`, a detail read), so the
/// run is looked for rather than required to stand alone.
pub fn action_of(effects: &[Effect]) -> &Action {
    let mut runs = effects.iter().filter_map(|effect| match effect {
        Effect::Run(_, action) => Some(action.as_ref()),
        _ => None,
    });
    let action = runs.next().unwrap_or_else(|| {
        panic!("expected an action, got {effects:?}");
    });
    assert!(runs.next().is_none(), "one action at a time: {effects:?}");
    action
}

/// The id the one `Effect::Run` among the effects carries.
pub fn run_id(effects: &[Effect]) -> fastf::tui::effect::ActionId {
    let mut runs = effects.iter().filter_map(|effect| match effect {
        Effect::Run(id, _) => Some(*id),
        _ => None,
    });
    let id = runs
        .next()
        .unwrap_or_else(|| panic!("expected a run, got {effects:?}"));
    assert!(runs.next().is_none(), "one action at a time: {effects:?}");
    id
}

pub fn item_done(id: fastf::tui::effect::ActionId, change: ListChange) -> Msg {
    Msg::ActionDone {
        id,
        outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
            change, "done",
        ))),
    }
}
