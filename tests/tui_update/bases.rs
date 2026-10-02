//! Bases that answer at their own pace: a discovery a base at a time, and the
//! summary a part at a time. A base on a mount that stopped answering holds up
//! nothing but itself.

use crate::harness::*;
use fastf::tui::app::data::{BaseInfo, SummaryPart};
use fastf::tui::testing::render_to_string;
use fastf::util::paths::Probe;

const FAST: &str = "/mnt/fast";
const SLOW: &str = "/mnt/slow";

/// `n` projects that live in `base`, numbered from `first`.
fn rows_in(base: &str, n: usize, first: u64) -> Vec<Project> {
    sample_projects(n)
        .into_iter()
        .enumerate()
        .map(|(i, mut project)| {
            let number = first + i as u64;
            project.id = format!("ID{number:04}");
            project.id_number = Some(number);
            project.name = format!("{}_{}", base.trim_start_matches("/mnt/"), project.id);
            project.base = PathBuf::from(base);
            project.path = project.base.join(&project.name);
            project
        })
        .collect()
}

/// An app that asked for a discovery of the two bases.
fn asking_two_bases() -> App {
    let mut app = empty_fixture(100, 30);
    let _ = app.start();
    update(
        &mut app,
        Msg::DiscoveryPlanned {
            generation: 1,
            bases: vec![PathBuf::from(FAST), PathBuf::from(SLOW)],
        },
    );
    app
}

fn base_rows(app: &mut App, generation: u64, base: &str, rows: Vec<Project>) -> Vec<Effect> {
    update(
        app,
        Msg::DiscoveredBase {
            generation,
            base: PathBuf::from(base),
            projects: rows,
        },
    )
}

fn two_bases_answered() -> Box<SummaryPart> {
    let base = |path: &str, label: &str, default| BaseInfo {
        path: PathBuf::from(path),
        configured: PathBuf::from(path),
        label: label.to_string(),
        probe: Probe::Mounted,
        indexed: Some(3),
        is_default: default,
    };
    Box::new(SummaryPart::Bases {
        bases: vec![base(FAST, "fast", true), base(SLOW, "slow", false)],
        projects: 6,
        max_id: Some("ID0012".to_string()),
        newest: None,
    })
}

/// **A slow base holds up nothing but itself, as state.** One base answers and
/// its rows are on screen, measured, while the other is still being asked; the
/// discovery settles at its deadline and names the silent base; its rows
/// arrive when it answers.
#[test]
fn a_base_that_answers_is_on_screen_while_another_is_still_asked() {
    let mut app = asking_two_bases();
    let effects = base_rows(&mut app, 1, FAST, rows_in(FAST, 3, 1));
    assert_eq!(app.library.len(), 3, "the fast base's rows are in");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::RequestSizes(paths) if paths.len() == 3)),
        "and measured at once: {effects:?}"
    );
    assert!(!app.library.loaded, "the discovery is still reading");

    update(
        &mut app,
        Msg::DiscoverySettled {
            generation: 1,
            silent: vec![PathBuf::from(SLOW)],
        },
    );
    assert!(app.library.loaded);
    assert!(app.library.inflight.is_none());
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 1,
            part: two_bases_answered(),
        },
    );
    let screen = render_to_string(&app, 100, 30);
    assert!(
        screen.contains("slow unresponsive"),
        "the header names the silent base:\n{screen}"
    );
    assert!(screen.contains("fast 3"), "{screen}");

    // The mount comes back: its worker answers, late, for the discovery
    // that asked it.
    base_rows(&mut app, 1, SLOW, rows_in(SLOW, 2, 10));
    assert_eq!(app.library.len(), 5);
    assert!(app.library.silent.is_empty());
    let screen = render_to_string(&app, 100, 30);
    assert!(screen.contains("slow 3"), "{screen}");
}

/// A base's index answers first, the base after; the second replaces the
/// first, and the cursor stays on its project.
#[test]
fn the_verified_rows_replace_the_index_rows_and_the_cursor_stays() {
    let mut app = asking_two_bases();
    let cached = rows_in(FAST, 3, 1);
    base_rows(&mut app, 1, FAST, cached.clone());
    press(&mut app, Key::plain(KeyCode::Down));
    let chosen = app.library.selected().unwrap().path.clone();

    // The base holds one project fewer than its index said, and it was not
    // the chosen one.
    let verified: Vec<Project> = cached
        .into_iter()
        .filter(|project| project.path == chosen || project.id == "ID0001")
        .collect();
    base_rows(&mut app, 1, FAST, verified);
    assert_eq!(app.library.len(), 2);
    assert_eq!(app.library.selected().unwrap().path, chosen);
}

/// An older discovery's rows for a base never replace a newer one's, and a
/// base the configuration no longer names is dropped.
#[test]
fn an_older_answer_for_a_base_never_replaces_a_newer_one() {
    let mut app = asking_two_bases();
    base_rows(&mut app, 1, FAST, rows_in(FAST, 3, 1));
    base_rows(&mut app, 1, SLOW, rows_in(SLOW, 2, 10));
    update(
        &mut app,
        Msg::DiscoverySettled {
            generation: 1,
            silent: Vec::new(),
        },
    );

    // F5: a new discovery, which no longer lists the slow base.
    let effects = press(&mut app, Key::plain(KeyCode::F(5)));
    let generation = effects
        .iter()
        .find_map(|e| match e {
            Effect::Discover { generation } => Some(*generation),
            _ => None,
        })
        .expect("F5 discovers again");
    update(
        &mut app,
        Msg::DiscoveryPlanned {
            generation,
            bases: vec![PathBuf::from(FAST)],
        },
    );
    assert_eq!(app.library.len(), 3, "the unconfigured base's rows go");
    base_rows(&mut app, generation, FAST, rows_in(FAST, 4, 1));
    assert_eq!(app.library.len(), 4);

    // The first discovery's worker, late.
    assert!(base_rows(&mut app, 1, FAST, rows_in(FAST, 1, 1)).is_empty());
    assert_eq!(app.library.len(), 4, "the newer rows stay");
    assert!(base_rows(&mut app, 1, SLOW, rows_in(SLOW, 2, 10)).is_empty());
    assert_eq!(
        app.library.len(),
        4,
        "a base no longer configured stays out"
    );
}

/// A reload while one base is still silent keeps every other base's rows on
/// screen: nothing blinks empty.
#[test]
fn a_reload_keeps_the_rows_until_each_base_answers_again() {
    let mut app = asking_two_bases();
    base_rows(&mut app, 1, FAST, rows_in(FAST, 3, 1));
    update(
        &mut app,
        Msg::DiscoverySettled {
            generation: 1,
            silent: vec![PathBuf::from(SLOW)],
        },
    );
    let effects = press(&mut app, Key::plain(KeyCode::F(5)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Discover { generation: 2 })),
        "{effects:?}"
    );
    update(
        &mut app,
        Msg::DiscoveryPlanned {
            generation: 2,
            bases: vec![PathBuf::from(FAST), PathBuf::from(SLOW)],
        },
    );
    assert_eq!(app.library.len(), 3, "still on screen while it is asked");
    assert!(
        app.library.silent.contains(&PathBuf::from(SLOW)),
        "still silent until it answers"
    );
}

/// A tag added while a base is still silent is not undone when that base
/// answers: the union is rebuilt from rows that carry the patch.
#[test]
fn a_patch_survives_another_base_answering() {
    let mut app = asking_two_bases();
    base_rows(&mut app, 1, FAST, rows_in(FAST, 3, 1));
    let mut tagged = app.library.snapshot[0].clone();
    tagged.tags.push("urgent".to_string());
    let was = tagged.path.clone();
    assert!(app.library.patch(&was, tagged));

    base_rows(&mut app, 1, SLOW, rows_in(SLOW, 2, 10));
    let row = app
        .library
        .snapshot
        .iter()
        .find(|project| project.path == was)
        .unwrap();
    assert!(row.tags.contains(&"urgent".to_string()), "{:?}", row.tags);
}

/// The templates never wait on a base: they land first, while the header
/// still says it is probing, and nothing offers a base yet.
#[test]
fn the_templates_land_before_the_bases_have_answered() {
    let mut app = empty_fixture(100, 30);
    let _ = app.start();
    let summary = sample_summary(3);
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 1,
            part: Box::new(SummaryPart::Local {
                templates: summary.templates.clone(),
                prefs: summary.prefs.clone(),
            }),
        },
    );
    let summary_now = app.summary.as_ref().unwrap();
    assert_eq!(summary_now.templates.len(), summary.templates.len());
    assert!(summary_now.bases_known().is_none());
    let screen = render_to_string(&app, 100, 30);
    assert!(screen.contains("probing bases"), "{screen}");

    update(
        &mut app,
        Msg::SummaryPart {
            generation: 1,
            part: two_bases_answered(),
        },
    );
    let screen = render_to_string(&app, 100, 30);
    assert!(!screen.contains("probing bases"), "{screen}");
    assert!(screen.contains("2 bases"), "{screen}");
}

/// A slow read answering after a newer one puts nothing old back.
#[test]
fn an_older_summary_part_never_replaces_a_newer_one() {
    let mut app = empty_fixture(100, 30);
    let _ = app.start();
    let local = |names: &[&str]| {
        let mut templates = sample_summary(1).templates;
        templates.retain(|card| names.contains(&card.slug.as_str()));
        Box::new(SummaryPart::Local {
            templates,
            prefs: sample_summary(1).prefs,
        })
    };
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 2,
            part: local(&["general", "music-video"]),
        },
    );
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 1,
            part: local(&["general"]),
        },
    );
    assert_eq!(app.summary.as_ref().unwrap().templates.len(), 2);
    // Another part of the older read is still news.
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 1,
            part: two_bases_answered(),
        },
    );
    assert!(app.summary.as_ref().unwrap().bases_known().is_some());
}

/// A silent base is named among the bases and not counted as work to
/// finish; a move that waits for its base is.
#[test]
fn a_silent_base_is_named_not_counted() {
    use fastf::core::attention::{A_BASE, Attention, Item, State};
    let waiting = |what: &str, path: &str| Item {
        state: State::Waiting,
        what: what.to_string(),
        path: PathBuf::from(path),
        project: None,
        reason: "waits for its base".to_string(),
        actions: Vec::new(),
    };
    let mut app = fixture(3, 100, 30);
    let mut summary = sample_summary(3);
    summary.attention = Attention {
        items: vec![waiting(A_BASE, "/media/usb/archive")],
    };
    update(&mut app, Msg::Summary(Box::new(summary.clone())));
    let screen = render_to_string(&app, 100, 30);
    assert!(
        !screen.contains("finishing") && !screen.contains("waiting"),
        "nothing to finish while a base is silent:\n{screen}"
    );

    summary.attention.items.push(waiting(
        "a move",
        "/mnt/projects/.fastf-transactions/18d8e2f16082c791-e6a94-0",
    ));
    update(&mut app, Msg::Summary(Box::new(summary)));
    let screen = render_to_string(&app, 100, 30);
    assert!(screen.contains("1 waiting"), "{screen}");
}
