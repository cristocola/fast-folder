//! The agent verbs: `x` opens a window running the agent at the selected
//! project, `X` steps the app aside for it, and neither is offered without an
//! `agent` configured.

use crate::harness::*;
use fastf::tui::command::{Availability, CommandId, find};
use fastf::tui::effect::{SpawnKind, Suspended};
use fastf::tui::msg::Resumed;

fn with_agent(app: &mut App, line: Option<&str>) {
    app.summary.as_mut().unwrap().prefs.agent = line.map(str::to_string);
}

/// Without an agent both verbs are dimmed with the command line's own
/// sentence, and press nothing; with one, `x` asks for a window and `X` for
/// the terminal.
#[test]
fn the_agent_verbs_need_an_agent_and_then_spawn_or_suspend() {
    let mut app = fixture(6, 120, 40);
    app.has_display = true;
    with_agent(&mut app, None);
    let dimmed = (find(CommandId::Agent).available)(&app);
    assert!(
        matches!(dimmed, Availability::Disabled(reason) if reason.contains("no agent configured")),
        "{dimmed:?}"
    );
    assert!(press(&mut app, Key::ch('x')).is_empty());
    assert!(press(&mut app, Key::ch('X')).is_empty());

    with_agent(&mut app, Some("claude"));
    let project = app.library.selected().unwrap().clone();
    let effects = press(&mut app, Key::ch('x'));
    assert!(
        matches!(&effects[..], [Effect::Spawn(SpawnKind::Agent(p))] if p.path == project.path),
        "{effects:?}"
    );
    let effects = press(&mut app, Key::ch('X'));
    assert!(
        matches!(&effects[..], [Effect::Suspend(Suspended::Agent(p))] if p.path == project.path),
        "{effects:?}"
    );

    // Back from the agent: the status says how it went and the pane reads
    // the project's file again.
    let effects = update(
        &mut app,
        Msg::Resumed(Resumed::Agent {
            project: Box::new(project.clone()),
            ok: true,
        }),
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDetail(path) if *path == project.path)),
        "{effects:?}"
    );
    assert!(
        app.status.text.contains("finished"),
        "{:?}",
        app.status.text
    );

    // No display: the window verb says so and the in-place one still works.
    app.has_display = false;
    let dimmed = (find(CommandId::Agent).available)(&app);
    assert!(
        matches!(dimmed, Availability::Disabled(reason) if reason.contains("no display")),
        "{dimmed:?}"
    );
    assert!(matches!(
        (find(CommandId::AgentHere).available)(&app),
        Availability::Enabled
    ));
}
