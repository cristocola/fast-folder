//! Which bases the list shows: the bases panel (`b`), a base's menu, and the
//! view — every base, or only the active ones — that the next run starts on.

use crate::harness::*;
use fastf::tui::app::bases::BaseMenu;
use fastf::tui::app::data::{BaseInfo, SummaryPart};
use fastf::tui::app::library::BasesView;
use fastf::tui::session::Session;
use fastf::tui::testing::render_to_string;
use fastf::util::paths::Probe;

const HOME: &str = "/mnt/projects/home";
const ARCHIVE: &str = "/mnt/projects/archive";

/// `n` projects that live in `base`, numbered from `first`.
fn rows_in(base: &str, n: usize, first: u64) -> Vec<Project> {
    sample_projects(n)
        .into_iter()
        .enumerate()
        .map(|(i, mut project)| {
            let number = first + i as u64;
            project.id = format!("ID{number:04}");
            project.id_number = Some(number);
            project.name = format!(
                "{}_{}",
                base.trim_start_matches("/mnt/projects/"),
                project.id
            );
            project.base = PathBuf::from(base);
            project.path = project.base.join(&project.name);
            project
        })
        .collect()
}

fn summary_of_two_bases() -> Box<SummaryPart> {
    let base = |path: &str, label: &str, default| BaseInfo {
        path: PathBuf::from(path),
        configured: PathBuf::from(path),
        label: label.to_string(),
        probe: Probe::Mounted,
        indexed: Some(if default { 3 } else { 2 }),
        is_default: default,
    };
    Box::new(SummaryPart::Bases {
        bases: vec![base(HOME, "home", true), base(ARCHIVE, "archive", false)],
        projects: 5,
        max_id: Some("ID0011".to_string()),
        newest: None,
    })
}

/// The app after its first discovery of two bases — three projects at home,
/// two in the archive — with the summary in, as `session` left it.
fn two_bases_as(session: &Session) -> App {
    let mut app = empty_fixture(110, 30);
    app.apply_session(session);
    let _ = app.start();
    update(
        &mut app,
        Msg::DiscoveryPlanned {
            generation: 1,
            bases: vec![PathBuf::from(HOME), PathBuf::from(ARCHIVE)],
        },
    );
    for (base, rows) in [
        (HOME, rows_in(HOME, 3, 1)),
        (ARCHIVE, rows_in(ARCHIVE, 2, 10)),
    ] {
        update(
            &mut app,
            Msg::DiscoveredBase {
                generation: 1,
                base: PathBuf::from(base),
                projects: rows,
            },
        );
    }
    update(
        &mut app,
        Msg::DiscoverySettled {
            generation: 1,
            silent: Vec::new(),
        },
    );
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 1,
            part: summary_of_two_bases(),
        },
    );
    app
}

fn two_bases() -> App {
    two_bases_as(&Session::default())
}

fn saves(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::SaveSession))
}

/// The archive unticked from the panel, the panel closed again.
fn archive_unticked() -> App {
    let mut app = two_bases();
    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::plain(KeyCode::Esc));
    app
}

/// **Unticking a base takes its rows out at once, and is remembered at
/// once.** Every base was shown, where unticking would change nothing on
/// screen, so the view becomes the active bases; the panel stays up, so the
/// change is seen where it was made.
#[test]
fn unticking_a_base_takes_its_rows_out_and_saves_the_choice() {
    let mut app = two_bases();
    assert_eq!(app.library.len(), 5);
    press(&mut app, Key::ch('b'));
    assert!(matches!(app.modals.top(), Some(Modal::Bases(_))));
    press(&mut app, Key::plain(KeyCode::Down));
    let effects = press(&mut app, Key::ch(' '));
    assert!(saves(&effects), "remembered as it is made: {effects:?}");
    assert!(
        matches!(app.modals.top(), Some(Modal::Bases(_))),
        "the panel stays"
    );
    assert_eq!(app.library.view, BasesView::Active);
    assert_eq!(app.library.len(), 3, "{:?}", names(&app));
    assert!(names(&app).iter().all(|name| name.starts_with("home_")));

    let session = Session::capture(&app, &Session::default());
    assert_eq!(session.bases_view(), BasesView::Active);
    assert_eq!(session.inactive_bases, vec![ARCHIVE.to_string()]);

    // Ticked again, nothing is left out: the two views are one list, and
    // the one shown says so.
    let effects = press(&mut app, Key::ch(' '));
    assert!(saves(&effects));
    assert_eq!(app.library.view, BasesView::Every);
    assert_eq!(app.library.len(), 5);
}

/// **The default base stays active**: new projects land there, and a list
/// that left it out would hide every project made from now on. Space on it
/// says why and changes nothing.
#[test]
fn the_default_base_cannot_be_unticked() {
    let mut app = two_bases();
    press(&mut app, Key::ch('b'));
    let effects = press(&mut app, Key::ch(' '));
    assert!(!saves(&effects));
    assert!(app.library.inactive.is_empty());
    assert!(
        app.status.text.contains("default base stays active"),
        "{}",
        app.status.text
    );
    // And a session that marked it — written before it became the default
    // — is set right when the bases are known.
    let app = two_bases_as(&Session {
        bases_view: Some("active".to_string()),
        inactive_bases: vec![HOME.to_string()],
        ..Session::default()
    });
    assert!(app.library.inactive.is_empty());
    assert_eq!(app.library.len(), 5);
}

/// **`B` switches the view from the list**, and is refused with the reason
/// while every base is active — the active view would be the same list under
/// another name.
#[test]
fn b_switches_between_every_base_and_the_active_ones() {
    let mut app = two_bases();
    let effects = press(&mut app, Key::ch('B'));
    assert!(!saves(&effects));
    assert!(
        app.status.text.contains("every base is active"),
        "{}",
        app.status.text
    );

    let mut app = archive_unticked();
    assert_eq!(app.library.len(), 3);
    let effects = press(&mut app, Key::ch('B'));
    assert!(saves(&effects));
    assert_eq!(app.library.view, BasesView::Every);
    assert_eq!(app.library.len(), 5, "the archive is back, still unticked");
    assert!(!app.library.inactive.is_empty());
    let effects = press(&mut app, Key::ch('B'));
    assert!(saves(&effects));
    assert_eq!(app.library.len(), 3);
}

/// **Showing one base alone beats the view**: an inactive base can be looked
/// into without being made active. It is a filter, so Esc takes it off — and
/// leaves the view, a remembered choice, as it was; so does `F`.
#[test]
fn an_inactive_base_can_be_shown_alone_and_esc_goes_back() {
    let mut app = archive_unticked();
    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::ch('f'));
    assert!(app.modals.is_empty(), "what was asked for is the list");
    assert_eq!(app.library.base_filter, Some(PathBuf::from(ARCHIVE)));
    assert_eq!(app.library.len(), 2);
    assert!(names(&app).iter().all(|name| name.starts_with("archive_")));

    press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.library.base_filter, None);
    assert_eq!(
        app.library.view,
        BasesView::Active,
        "Esc never changes the view"
    );
    assert_eq!(app.library.len(), 3);

    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::ch('f'));
    press(&mut app, Key::ch('F'));
    assert_eq!(app.library.base_filter, None);
    assert_eq!(app.library.view, BasesView::Active, "nor does F");
    assert_eq!(app.library.len(), 3);
}

/// **Enter opens the base's menu, every verb with its key**, about the base
/// it was opened on; a verb chosen there closes the menu and leaves the panel
/// up, showing the change.
#[test]
fn enter_opens_the_base_menu_and_a_verb_there_acts_on_its_base() {
    let mut app = two_bases();
    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::plain(KeyCode::Enter));
    let Some(Modal::BaseMenu(BaseMenu { base, .. })) = app.modals.top() else {
        panic!("the base's menu: {:?}", app.modals.top());
    };
    assert_eq!(base, &PathBuf::from(ARCHIVE));
    let rows: Vec<&str> = fastf::tui::app::bases::base_menu_entries(&app)
        .iter()
        .map(|(id, _)| fastf::tui::app::bases::base_verb_title(&app, *id))
        .collect();
    assert_eq!(
        rows,
        [
            "Mark inactive",
            "Show only this base",
            "Show active bases only",
            "Add or remove bases"
        ]
    );
    let screen = render_to_string(&app, 110, 30);
    assert!(screen.contains("archive · actions"), "{screen}");
    assert!(screen.contains("Space"), "each row with its key:\n{screen}");

    // The first row, run with Enter.
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(saves(&effects));
    assert!(matches!(app.modals.top(), Some(Modal::Bases(_))));
    assert!(app.library.inactive.contains(&PathBuf::from(ARCHIVE)));

    // A verb's own key in the menu: its base, not the panel's cursor.
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::plain(KeyCode::Up));
    press(&mut app, Key::ch('f'));
    assert_eq!(app.library.base_filter, Some(PathBuf::from(ARCHIVE)));
}

/// "Add or remove bases" opens the settings on the row that holds the list,
/// over the panel, so leaving them comes back to it.
#[test]
fn the_menu_opens_the_list_of_bases_in_the_settings() {
    let mut app = two_bases();
    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::plain(KeyCode::End));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(effects.iter().any(|e| matches!(e, Effect::LoadSettings)));
    assert!(matches!(app.modals.top(), Some(Modal::Settings(_))));
    update(&mut app, Msg::SettingsLoaded(Box::default()));
    let Some(Modal::Settings(state)) = app.modals.top() else {
        panic!("the settings");
    };
    assert_eq!(state.row().and_then(|row| row.kind.key()), Some("bases"));
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(matches!(app.modals.top(), Some(Modal::Bases(_))));
}

/// **Nothing is left out in silence**: a search the active view leaves empty
/// says how many matches the inactive bases hold, and the key that shows
/// them.
#[test]
fn an_empty_list_names_what_the_view_left_out() {
    let mut app = archive_unticked();
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "archive_");
    assert!(app.library.is_empty());
    assert_eq!(app.library.beyond_view, 2);
    // The status line speaks once the message about the untick has gone.
    app.status = Default::default();
    let screen = render_to_string(&app, 110, 30);
    assert!(
        screen.contains("nothing in the active bases — B shows every base"),
        "the table says it:\n{screen}"
    );
    assert!(
        screen.contains("no matches in the active bases — 2 in inactive ones, B shows every base"),
        "and the status line counts them:\n{screen}"
    );
    assert!(
        screen.contains("0/3"),
        "counted against what is in view:\n{screen}"
    );
}

/// **A project the view leaves out is still found by name**: the palette
/// lists it, and going to it shows its base alone rather than nothing.
#[test]
fn going_to_an_inactive_project_shows_its_base_alone() {
    let mut app = archive_unticked();
    press(&mut app, Key::ch('c'));
    type_text(&mut app, "#ID0010");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(app.library.base_filter, Some(PathBuf::from(ARCHIVE)));
    assert_eq!(
        app.library.selected().map(|p| p.id.as_str()),
        Some("ID0010")
    );
    assert!(
        app.status.text.contains("inactive base"),
        "{}",
        app.status.text
    );
    assert_eq!(
        app.library.view,
        BasesView::Active,
        "the view is not changed"
    );
}

/// A project made in an inactive base is selected when it lands, its base
/// shown alone, instead of being made and never seen.
#[test]
fn a_project_made_in_an_inactive_base_is_shown_when_it_lands() {
    let mut app = archive_unticked();
    let made = PathBuf::from(ARCHIVE).join("archive_ID0012");
    // What a create's outcome asks for, and the discovery after it.
    app.select_when_found = Some(made.clone());
    press(&mut app, Key::plain(KeyCode::F(5)));
    let generation = app.library.inflight.expect("a discovery");
    let mut rows = rows_in(ARCHIVE, 2, 10);
    let mut new = rows[0].clone();
    new.id = "ID0012".to_string();
    new.name = "archive_ID0012".to_string();
    new.path = made.clone();
    rows.insert(0, new);
    update(
        &mut app,
        Msg::DiscoveryPlanned {
            generation,
            bases: vec![PathBuf::from(HOME), PathBuf::from(ARCHIVE)],
        },
    );
    update(
        &mut app,
        Msg::DiscoveredBase {
            generation,
            base: PathBuf::from(ARCHIVE),
            projects: rows,
        },
    );
    assert_eq!(app.library.selected().map(|p| p.path.clone()), Some(made));
    assert_eq!(app.library.base_filter, Some(PathBuf::from(ARCHIVE)));
    assert!(app.select_when_found.is_none());
}

/// **The view is in place before the first frame**: a run that starts on the
/// active bases never draws an inactive base's rows, even before the summary
/// has said which bases there are.
#[test]
fn a_remembered_view_hides_the_rows_before_the_summary_lands() {
    let session = Session {
        bases_view: Some("active".to_string()),
        inactive_bases: vec![ARCHIVE.to_string()],
        ..Session::default()
    };
    let mut app = empty_fixture(110, 30);
    app.apply_session(&session);
    let _ = app.start();
    update(
        &mut app,
        Msg::DiscoveryPlanned {
            generation: 1,
            bases: vec![PathBuf::from(HOME), PathBuf::from(ARCHIVE)],
        },
    );
    update(
        &mut app,
        Msg::DiscoveredBase {
            generation: 1,
            base: PathBuf::from(ARCHIVE),
            projects: rows_in(ARCHIVE, 2, 10),
        },
    );
    assert!(app.library.is_empty(), "{:?}", names(&app));
    assert_eq!(app.library.beyond_view, 2);
}

/// A base dropped from the configuration is forgotten: the summary that no
/// longer names it takes its mark away, and it would come back active.
#[test]
fn a_base_no_longer_configured_is_forgotten() {
    let app = two_bases_as(&Session {
        bases_view: Some("active".to_string()),
        inactive_bases: vec![ARCHIVE.to_string(), "/mnt/projects/gone".to_string()],
        ..Session::default()
    });
    assert_eq!(
        app.library.inactive.iter().collect::<Vec<_>>(),
        [&PathBuf::from(ARCHIVE)]
    );
    assert_eq!(
        Session::capture(&app, &Session::default()).inactive_bases,
        vec![ARCHIVE.to_string()]
    );
}

/// **The header says which bases are active**: showing every base, an
/// inactive one is still named; showing the active ones, the rest are one
/// count.
#[test]
fn the_header_says_which_bases_are_active() {
    let mut app = archive_unticked();
    let screen = render_to_string(&app, 110, 30);
    let header: Vec<&str> = screen.lines().take(2).collect();
    assert!(header[1].contains("home 3"), "{header:?}");
    assert!(header[1].contains("1 inactive"), "{header:?}");
    assert!(!header[1].contains("archive"), "{header:?}");
    assert!(
        screen.contains("active bases"),
        "the search bar says the view:\n{screen}"
    );
    press(&mut app, Key::ch('B'));
    let screen = render_to_string(&app, 110, 30);
    let header: Vec<&str> = screen.lines().take(2).collect();
    assert!(header[1].contains("archive 2"), "{header:?}");
    assert!(!header[1].contains("inactive"), "{header:?}");
}

/// A base removed while the panel is open — the summary read again after the
/// settings changed the list — leaves the cursor on a base that is there.
#[test]
fn the_panel_cursor_stays_on_a_base_that_is_still_there() {
    let mut app = two_bases();
    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Down));
    assert_eq!(
        app.base_in_hand().map(|b| b.label.as_str()),
        Some("archive")
    );
    let only_home = BaseInfo {
        path: PathBuf::from(HOME),
        configured: PathBuf::from(HOME),
        label: "home".to_string(),
        probe: Probe::Mounted,
        indexed: Some(3),
        is_default: true,
    };
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 2,
            part: Box::new(SummaryPart::Bases {
                bases: vec![only_home],
                projects: 3,
                max_id: Some("ID0003".to_string()),
                newest: None,
            }),
        },
    );
    assert_eq!(app.base_in_hand().map(|b| b.label.as_str()), Some("home"));
}

fn summary_of_home_only() -> Msg {
    Msg::SummaryPart {
        generation: 2,
        part: Box::new(SummaryPart::Bases {
            bases: vec![BaseInfo {
                path: PathBuf::from(HOME),
                configured: PathBuf::from(HOME),
                label: "home".to_string(),
                probe: Probe::Mounted,
                indexed: Some(3),
                is_default: true,
            }],
            projects: 3,
            max_id: Some("ID0003".to_string()),
            newest: None,
        }),
    }
}

/// The base removed from the settings the panel opened, which lie over it:
/// the panel underneath is set right too, so it answers its keys when the
/// settings close.
#[test]
fn a_base_removed_from_the_settings_over_the_panel_leaves_it_working() {
    let mut app = two_bases();
    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::plain(KeyCode::End));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(app.modals.top(), Some(Modal::Settings(_))));
    update(&mut app, summary_of_home_only());
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(matches!(app.modals.top(), Some(Modal::Bases(_))));
    assert_eq!(app.base_in_hand().map(|b| b.label.as_str()), Some("home"));
}

/// **Nothing marked, nothing narrowed**: the base that was inactive is gone
/// from the configuration, and the view goes back to every base rather than
/// calling the whole library "the active bases".
#[test]
fn with_no_base_left_inactive_the_view_is_every_base() {
    let mut app = archive_unticked();
    assert_eq!(app.library.view, BasesView::Active);
    update(&mut app, summary_of_home_only());
    assert!(app.library.inactive.is_empty());
    assert_eq!(app.library.view, BasesView::Every);
    assert_eq!(
        Session::capture(&app, &Session::default()).bases_view(),
        BasesView::Every
    );
}

/// A project asked for that something else leaves out too — here a template
/// filter — is not found, and the list is put back as it was rather than left
/// on a base nobody asked to see.
#[test]
fn a_project_still_left_out_leaves_the_list_as_it_was() {
    let mut app = archive_unticked();
    app.library.template_filter = Some("no-such-template".to_string());
    let wanted = rows_in(ARCHIVE, 1, 10)[0].path.clone();
    app.select_when_found = Some(wanted);
    press(&mut app, Key::plain(KeyCode::F(5)));
    let generation = app.library.inflight.expect("a discovery");
    update(
        &mut app,
        Msg::DiscoveryPlanned {
            generation,
            bases: vec![PathBuf::from(HOME), PathBuf::from(ARCHIVE)],
        },
    );
    update(
        &mut app,
        Msg::DiscoveredBase {
            generation,
            base: PathBuf::from(ARCHIVE),
            projects: rows_in(ARCHIVE, 2, 10),
        },
    );
    assert_eq!(app.library.base_filter, None);
}

/// **`fastf recent` is cut after the view, not before.** It hands the app
/// every match and the limit; the newest projects here are in an inactive
/// base, and the list still shows as many active ones as the limit allows —
/// and Esc does not take a limit nobody typed off.
#[test]
fn fastf_recent_is_cut_to_its_limit_after_the_view() {
    let mut rows = rows_in(ARCHIVE, 2, 10);
    rows.extend(rows_in(HOME, 3, 1));
    let mut app = App::new(
        Entry::Recent {
            preset: Preset {
                default_limit: Some(2),
                ..Default::default()
            },
            initial: rows,
        },
        Theme::mono(),
        (110, 30),
    );
    app.apply_session(&Session {
        bases_view: Some("active".to_string()),
        inactive_bases: vec![ARCHIVE.to_string()],
        ..Session::default()
    });
    let _ = app.start();
    assert_eq!(app.library.len(), 2, "{:?}", names(&app));
    assert!(names(&app).iter().all(|name| name.starts_with("home_")));
    let screen = render_to_string(&app, 110, 30);
    assert!(
        !screen.contains("recent:"),
        "no chip for the default:\n{screen}"
    );
    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Quit(_))),
        "Esc has no filter to take off: {effects:?}"
    );
}

/// **A base reached through a link is matched before any summary**: the
/// session keeps its real path beside its configured one, so rows handed in
/// whole — carrying the real path — are never drawn and taken away.
#[test]
fn a_linked_base_is_matched_by_its_real_path_from_the_first_frame() {
    const REAL: &str = "/mnt/projects/real_archive";
    let session = Session {
        bases_view: Some("active".to_string()),
        inactive_bases: vec![ARCHIVE.to_string(), REAL.to_string()],
        ..Session::default()
    };
    let mut rows = rows_in(REAL, 2, 10);
    rows.extend(rows_in(HOME, 3, 1));
    let mut app = App::new(
        Entry::Search {
            terms: Vec::new(),
            initial: rows,
        },
        Theme::mono(),
        (110, 30),
    );
    app.apply_session(&session);
    let _ = app.start();
    assert_eq!(app.library.len(), 3, "{:?}", names(&app));

    // And the session written from a summary that knows the link keeps both.
    let mut app = two_bases();
    let mut summary = summary_of_two_bases();
    if let SummaryPart::Bases { bases, .. } = summary.as_mut() {
        bases[1].path = PathBuf::from(REAL);
    }
    update(
        &mut app,
        Msg::SummaryPart {
            generation: 2,
            part: summary,
        },
    );
    press(&mut app, Key::ch('b'));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::ch(' '));
    let kept = Session::capture(&app, &Session::default()).inactive_bases;
    assert_eq!(kept, vec![ARCHIVE.to_string(), REAL.to_string()]);
    // Ticked again, both names go.
    press(&mut app, Key::ch(' '));
    assert!(app.library.inactive.is_empty());
}

/// **The way out survives the narrowest window**: at 40 columns the panel's
/// key line keeps Space and Esc, and gives up the arrows first.
#[test]
fn the_panel_key_line_keeps_esc_at_forty_columns() {
    let mut app = two_bases();
    app.size = (40, 12);
    press(&mut app, Key::ch('b'));
    let screen = render_to_string(&app, 40, 12);
    assert!(screen.contains("Esc close"), "{screen}");
    let wide = render_to_string(&app, 110, 30);
    assert!(
        wide.contains("↑↓ choose"),
        "the arrows where there is room:\n{wide}"
    );
}
