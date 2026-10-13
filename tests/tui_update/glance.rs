//! The pane's glance: what last moved, the repository's head, who is working.

use crate::harness::*;
use fastf::tui::app::pane::PaneRow;

/// The pane asks who is working in the selected project once, when it first
/// shows it; the answer and the size walk's newest file each become a row
/// as they land, and the cursor keeps its row.
#[test]
fn the_glance_fills_in_as_the_answers_land() {
    use std::time::{Duration, UNIX_EPOCH};
    let mut app = fixture(6, 120, 40);
    let project = app.library.selected().unwrap().clone();
    let effects = press(&mut app, Key::ch('j'));
    let second = app.library.selected().unwrap().clone();
    let asked_once = effects
        .iter()
        .filter(|e| matches!(e, Effect::LoadWorking(paths) if paths == &vec![second.path.clone()]))
        .count();
    assert_eq!(
        asked_once,
        usize::from(fastf::core::holders::WORKING_KNOWN),
        "asked where the platform can answer, and once: {effects:?}"
    );
    press(&mut app, Key::ch('k'));
    assert_eq!(app.library.selected().unwrap().path, project.path);

    let glance_rows = |app: &App| -> Vec<(&'static str, String)> {
        app.pane_rows()
            .iter()
            .filter_map(|row| match row {
                PaneRow::Glance { key, value } => Some((*key, value.clone())),
                _ => None,
            })
            .collect()
    };
    let _ = update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::default(),
        },
    );
    assert!(glance_rows(&app).is_empty(), "nothing known yet");

    let _ = update(
        &mut app,
        Msg::Sizes(vec![(
            project.path.clone(),
            Some(fastf::util::tree_size::TreeFacts {
                bytes: 10,
                newest: Some((
                    UNIX_EPOCH + Duration::from_secs(1_767_951_665),
                    PathBuf::from("notes/brief.md"),
                )),
            }),
        )]),
    );
    assert_eq!(
        glance_rows(&app),
        [("last moved", "notes/brief.md  2026-01-09 09:41".to_string())]
    );
    assert!(app.pane_pulses.is_empty(), "a first fill is not a change");
    // A different newest file replacing it is a change, and its row pulses.
    let _ = update(
        &mut app,
        Msg::Sizes(vec![(
            project.path.clone(),
            Some(fastf::util::tree_size::TreeFacts {
                bytes: 12,
                newest: Some((
                    UNIX_EPOCH + Duration::from_secs(1_767_951_665 + 3600),
                    PathBuf::from("notes/brief.md"),
                )),
            }),
        )]),
    );
    assert!(!app.pane_pulses.is_empty(), "what moved is lit");
    assert!(
        glance_rows(&app)[0]
            .1
            .starts_with("notes/brief.md  2026-01-09 10:41")
    );
    let _ = update(
        &mut app,
        Msg::Working {
            asked: vec![project.path.clone()],
            active: [(project.path.clone(), vec!["claude".to_string()])]
                .into_iter()
                .collect(),
        },
    );
    if fastf::core::holders::WORKING_KNOWN {
        assert_eq!(
            glance_rows(&app).last().cloned(),
            Some(("working", "claude".to_string()))
        );
        // Asked again for the same project, nothing more is asked: the
        // runtime keeps it fresh on its own beat.
        let effects = press(&mut app, Key::ch('j'));
        let back = press(&mut app, Key::ch('k'));
        assert!(
            !back.iter().any(
                |e| matches!(e, Effect::LoadWorking(paths) if paths == &vec![project.path.clone()])
            ),
            "{effects:?} {back:?}"
        );
    }
}
