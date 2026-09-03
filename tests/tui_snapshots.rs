//! The guided app's frames, rendered through ratatui's test backend and
//! compared against the snapshots under `tests/snapshots/`.
//!
//! Every frame is drawn in the mono theme at a fixed size from a fixture with
//! fixed dates and placeholder paths, so a snapshot depends on the code and
//! nothing else. Colour is not part of a snapshot — the test backend records
//! symbols — which is why the layout is what these guard: what is on screen,
//! where, at 80×24 and at 120×40.
//!
//! When a frame changes on purpose, review the new snapshot with
//! `INSTA_UPDATE=always cargo test --test tui_snapshots` (or `cargo insta
//! review`) and commit it.

use fastf::tui::app::update;
use fastf::tui::command::Key;
use fastf::tui::msg::Msg;
use fastf::tui::testing::{
    empty_fixture, fixture, render_to_string, sample_projects, sample_summary_moveable,
};
use ratatui::crossterm::event::KeyCode;

fn snap(name: &str, frame: String) {
    insta::assert_snapshot!(name, frame);
}

#[test]
fn dashboard_80x24() {
    let app = fixture(12, 80, 24);
    snap("dashboard_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn dashboard_120x40() {
    let mut app = fixture(12, 120, 40);
    let _ = update(
        &mut app,
        Msg::Sizes(vec![(sample_projects(1)[0].path.clone(), Some(3_355_443))]),
    );
    snap("dashboard_120x40", render_to_string(&app, 120, 40));
}

#[test]
fn too_small_40x10() {
    let app = fixture(3, 40, 10);
    snap("too_small_40x10", render_to_string(&app, 40, 10));
}

#[test]
fn loading_before_discovery_answers() {
    let app = empty_fixture(80, 24);
    snap("loading_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn palette_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('c')));
    for c in "open".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    snap("palette_open", render_to_string(&app, 100, 30));
}

#[test]
fn help_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('?')));
    snap("help_open", render_to_string(&app, 100, 30));
}

#[test]
fn search_with_a_fuzzy_term_and_a_tag() {
    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Key(Key::ch('/')));
    for c in "tag:draft lulla".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    snap("search_fuzzy_80x24", render_to_string(&app, 80, 24));
}

#[test]
fn sort_picker_open() {
    let mut app = fixture(12, 80, 24);
    let _ = update(&mut app, Msg::Key(Key::ch('S')));
    snap("sort_picker_80x24", render_to_string(&app, 80, 24));
}

/// The regression the column order exists for: at 80 columns the folder name
/// is still there in full, whatever else had to go.
#[test]
fn narrow_keeps_the_folder_name() {
    let app = fixture(6, 80, 24);
    let frame = render_to_string(&app, 80, 24);
    for project in sample_projects(6) {
        assert!(
            frame.contains(&project.name),
            "the folder name must survive an 80-column window:\n{frame}"
        );
    }
    assert!(!frame.contains("TAGS"), "tags are the first column to go");
    let _ = KeyCode::Null;
}

// --- Phase 1: single-project actions -------------------------------------

#[test]
fn action_menu_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('a')));
    snap("action_menu", render_to_string(&app, 100, 30));
}

#[test]
fn delete_typed_confirm() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    for c in "2026-08-28_Lullaby_Remix_ID0248".chars() {
        let _ = update(&mut app, Msg::Key(Key::ch(c)));
    }
    snap("delete_typed_confirm", render_to_string(&app, 100, 30));
}

#[test]
fn metadata_view_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('M')));
    let lines = vec![
        "id               ID0248".to_string(),
        "template         music-video".to_string(),
        "template_name    Music video".to_string(),
        "created          2026-08-28T10:00:00Z".to_string(),
        "folder           2026-08-28_Lullaby_Remix_ID0248".to_string(),
        "base             /mnt/projects".to_string(),
        "path             2026-08-28_Lullaby_Remix_ID0248".to_string(),
        String::new(),
        "tags:".to_string(),
        "  • draft".to_string(),
    ];
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title: "ID0248 · metadata".to_string(),
            lines,
        },
    );
    snap("metadata_view", render_to_string(&app, 100, 30));
}

#[test]
fn journal_view_open() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('J')));
    let lines = vec![
        "2026-08-28  first cut sent to the label".to_string(),
        "2026-08-29  revision two uploaded".to_string(),
        "2026-08-30  final mix approved".to_string(),
    ];
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title: "ID0248 · journal".to_string(),
            lines,
        },
    );
    snap("journal_view", render_to_string(&app, 100, 30));
}

#[test]
fn move_progress_modal() {
    use fastf::core::assets::{JobPhase, JobStatus, Progress};

    let mut app = fixture(12, 100, 30);
    app.move_progress = Some(Progress {
        total_bytes: 3_500_000,
        copied_bytes: 1_200_000,
        total_files: 34,
        done_files: 12,
        current_file: "03_Assets/raw/footage_A001.mov".to_string(),
        status: JobStatus::Running,
        phase: JobPhase::Copying,
        error: None,
        cleanup_pending: false,
        warning: None,
        last_progress_at: 0,
    });
    app.busy = Some("moving…");
    snap("move_progress", render_to_string(&app, 100, 30));
}

// --- Phase 2: marks and batch jobs ---------------------------------------

use fastf::tui::effect::{ActionOutcome, ListChange};

#[test]
fn batch_delete_confirm() {
    let mut app = fixture(12, 100, 30);
    for _ in 0..3 {
        let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    }
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    snap("batch_delete_confirm", render_to_string(&app, 100, 30));
}

/// A running delete job over three marks, one item in flight.
#[test]
fn job_progress_modal() {
    let mut app = fixture(12, 100, 30);
    for _ in 0..3 {
        let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    }
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    let _ = update(&mut app, Msg::Key(Key::ch('y')));
    let job = app.job.as_ref().expect("the job is running");
    assert_eq!(job.pending.len(), 2);
    assert!(job.inflight.is_some());
    snap("job_progress", render_to_string(&app, 100, 30));
}

/// A moving job with a real byte count under it.
#[test]
fn job_progress_with_bytes_modal() {
    use fastf::core::assets::{JobPhase, JobStatus, Progress};

    let mut app = fixture(12, 100, 30);
    app.summary = Some(sample_summary_moveable(12));
    let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    let _ = update(&mut app, Msg::Key(Key::ch('m')));
    let _ = update(&mut app, Msg::Key(Key::plain(KeyCode::Enter)));
    assert!(app.job.is_some(), "the marks started a move job");
    app.move_progress = Some(Progress {
        total_bytes: 3_500_000,
        copied_bytes: 1_200_000,
        total_files: 34,
        done_files: 12,
        current_file: "03_Assets/raw/footage_A001.mov".to_string(),
        status: JobStatus::Running,
        phase: JobPhase::Copying,
        error: None,
        cleanup_pending: false,
        warning: None,
        last_progress_at: 0,
    });
    snap("job_progress_move", render_to_string(&app, 100, 30));
}

/// The report after a two-item delete job where one item failed: the failed
/// row is named, the clean one is not the story.
#[test]
fn job_report_with_failures() {
    let mut app = fixture(12, 100, 30);
    for _ in 0..2 {
        let _ = update(&mut app, Msg::Key(Key::ch(' ')));
    }
    let _ = update(&mut app, Msg::Key(Key::ch('D')));
    let effects = update(&mut app, Msg::Key(Key::ch('y')));
    let id1 = match effects.as_slice() {
        [fastf::tui::effect::Effect::Run(id, _)] => *id,
        other => panic!("expected one run, got {other:?}"),
    };
    // Item 1 fails; the job moves on to item 2.
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id: id1,
            outcome: Err("injected fault at 'delete:mid-copy'".to_string()),
        },
    );
    let id2 = match effects.as_slice() {
        [fastf::tui::effect::Effect::Run(id, _)] => *id,
        other => panic!("expected one run, got {other:?}"),
    };
    // Item 2 succeeds; the job finishes with a failure report.
    let second_path = app.library.row(1).unwrap().path.clone();
    let _ = update(
        &mut app,
        Msg::ActionDone {
            id: id2,
            outcome: Ok(Box::new(ActionOutcome {
                change: ListChange::Removed {
                    path: second_path.clone(),
                },
                message: "done".to_string(),
                warning: None,
                session: None,
            })),
        },
    );
    assert!(app.job.is_none());
    snap("job_report_with_failures", render_to_string(&app, 100, 30));
    // The failed row keeps its mark for a retry.
    let failed_path = app.library.row(0).unwrap().path.clone();
    assert!(app.library.marks.contains(&failed_path));
}
