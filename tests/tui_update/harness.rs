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
    desk_fixture, desk_records, empty_fixture, fixture, sample_projects, sample_summary,
    sample_summary_moveable,
};
pub use fastf::tui::theme::Theme;
pub use ratatui::crossterm::event::KeyCode;

/// What a size scan hands the app for a tree of `bytes`, with no newest
/// file — the fixtures' shape, where only the number matters.
pub fn facts(bytes: u64) -> fastf::util::tree_size::TreeFacts {
    fastf::util::tree_size::TreeFacts {
        bytes,
        newest: None,
    }
}

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

/// The job the effects start, if one does: its kind and items.
pub fn job_started(
    effects: &[Effect],
) -> Option<(fastf::core::jobs::JobKind, &[fastf::core::jobs::JobItem])> {
    effects.iter().find_map(|effect| match effect {
        Effect::StartJob { kind, items, .. } => Some((*kind, items.as_slice())),
        _ => None,
    })
}

/// Whether the job the effects start goes past what uses its folders.
pub fn job_forced(effects: &[Effect]) -> Option<bool> {
    effects.iter().find_map(|effect| match effect {
        Effect::StartJob { force, .. } => Some(*force),
        _ => None,
    })
}

/// The question about what uses some folders the effects ask, if they do:
/// its number and the paths.
pub fn in_use_asked(effects: &[Effect]) -> Option<(u64, Vec<PathBuf>)> {
    effects.iter().find_map(|effect| match effect {
        Effect::CheckInUse { request, folders } => Some((
            *request,
            folders.iter().map(|(path, _)| path.clone()).collect(),
        )),
        _ => None,
    })
}

/// Answer the question the effects ask with `uses` for the folders `busy`
/// names, and nothing for the rest: the effects of the answer.
pub fn answer_in_use(
    app: &mut App,
    effects: &[Effect],
    busy: &[(&Path, Vec<fastf::core::holders::Use>)],
) -> Vec<Effect> {
    let (request, paths) = in_use_asked(effects).expect("what uses the folders is asked");
    let found = paths
        .into_iter()
        .map(|path| {
            let uses = busy
                .iter()
                .find(|(busy, _)| *busy == path)
                .map(|(_, uses)| uses.clone())
                .unwrap_or_default();
            let holders = fastf::core::holders::Holders {
                root: path.clone(),
                label: "ID".to_string(),
                uses,
            };
            (path, holders)
        })
        .collect();
    update(app, Msg::InUse { request, found })
}

/// Answer the question the effects ask: nothing uses any of the folders.
pub fn nothing_uses_them(app: &mut App, effects: &[Effect]) -> Vec<Effect> {
    answer_in_use(app, effects, &[])
}

/// A program working in a folder, as the look reports one.
pub fn sitter(pid: u32) -> fastf::core::holders::Use {
    fastf::core::holders::Use {
        by: fastf::core::holders::By::Process {
            pid,
            program: "node".to_string(),
        },
        how: fastf::core::holders::How::WorkingIn {
            at: PathBuf::from("src"),
            shell: false,
        },
    }
}
