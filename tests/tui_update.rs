//! The guided app's state machine, driven without a terminal.
//!
//! `App` plus one `Msg` in, the `Effect`s out: these tests feed keys and worker
//! answers to an app built from fixtures and assert on what it asks the runtime
//! to do. Nothing here touches a disk or a screen.

use std::path::{Path, PathBuf};

use fastf::core::library::Project;
use fastf::tui::app::modal::Modal;
use fastf::tui::app::{App, Focus, update};
use fastf::tui::command::Key;
use fastf::tui::effect::{Action, Effect, Exit, ListChange, SpawnKind, Suspended};
use fastf::tui::entry::{Entry, Preset};
use fastf::tui::msg::Msg;
use fastf::tui::testing::{
    empty_fixture, fixture, sample_projects, sample_summary, sample_summary_moveable,
};
use fastf::tui::theme::Theme;
use ratatui::crossterm::event::KeyCode;

fn press(app: &mut App, key: Key) -> Vec<Effect> {
    update(app, Msg::Key(key))
}

fn type_text(app: &mut App, text: &str) -> Vec<Effect> {
    let mut effects = Vec::new();
    for c in text.chars() {
        effects.extend(press(app, Key::ch(c)));
    }
    effects
}

fn names(app: &App) -> Vec<String> {
    (0..app.library.len())
        .filter_map(|row| app.library.row(row).map(|p| p.name.clone()))
        .collect()
}

fn selected_name(app: &App) -> String {
    app.library
        .selected()
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

#[test]
fn opening_asks_for_the_summary_and_one_discovery() {
    let mut app = empty_fixture(80, 24);
    assert_eq!(
        app.start(),
        vec![Effect::LoadSummary, Effect::Discover { generation: 1 }]
    );
    assert!(!app.library.loaded);
}

/// A destructive verb runs on the project its dialog named, or on nothing.
///
/// The prompt text was built once from the row under the cursor and the action
/// was built again at submit time from whatever was selected *then*. A
/// discovery landing under an open dialog re-filters the list, and
/// `clamped_selection` moves the cursor when the named row is no longer in the
/// snapshot — so a delete or an unregister could point at a different project
/// from the one the question named. The dialog carries its target by path now,
/// and a target that is gone is a refusal rather than a neighbour.
#[test]
fn a_delete_whose_project_left_the_library_does_not_delete_a_neighbour() {
    let mut app = fixture(4, 100, 30);
    let named = app
        .library
        .selected()
        .expect("a row under the cursor")
        .clone();

    press(&mut app, Key::ch('D'));
    assert!(
        matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
        "the delete prompt is up"
    );

    // The library moves on underneath: the named project is no longer in the
    // snapshot, which is what a discovery landing under an open dialog does
    // when the row it named has been deleted or moved elsewhere.
    app.library
        .snapshot
        .retain(|project| project.path != named.path);
    let query = app.search.query.clone();
    app.library.recompute(&query, &mut app.fuzzy);
    assert!(
        app.library.selected().is_some_and(|p| p.path != named.path),
        "the cursor has moved to a different project, which is the hazard"
    );

    // Typing the word and pressing Enter must not delete whatever the cursor
    // has landed on instead.
    for c in "delete".chars() {
        press(&mut app, Key::ch(c));
    }
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Run(_, action) if matches!(**action, Action::Delete(_)))),
        "nothing may be deleted once the named project is gone: {effects:?}"
    );
}

#[test]
fn a_stale_discovery_is_dropped_and_the_current_one_installs() {
    let mut app = empty_fixture(80, 24);
    let _ = app.start();
    let stale = update(
        &mut app,
        Msg::Discovered {
            generation: 7,
            projects: sample_projects(3),
        },
    );
    assert!(stale.is_empty());
    assert!(
        !app.library.loaded,
        "an answer to a request never sent is ignored"
    );

    let effects = update(
        &mut app,
        Msg::Discovered {
            generation: 1,
            projects: sample_projects(3),
        },
    );
    assert!(app.library.loaded);
    assert_eq!(app.library.len(), 3);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::RequestSizes(paths) if paths.len() == 3)),
        "the visible rows are measured once the list is known: {effects:?}"
    );
}

#[test]
fn recent_installs_the_rows_without_a_discovery() {
    let mut app = App::new(
        Entry::Recent {
            preset: Preset {
                template: Some("general".to_string()),
                ..Default::default()
            },
            initial: sample_projects(6),
        },
        Theme::mono(),
        (80, 24),
    );
    let effects = app.start();
    assert!(app.library.loaded);
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Discover { .. })),
        "the rows were handed in already: {effects:?}"
    );
    assert!(effects.contains(&Effect::LoadSummary));
    assert!(
        names(&app)
            .iter()
            .all(|n| n.contains("Client_Onboarding") || n.contains("Spring")),
        "the preset filters the rows: {:?}",
        names(&app)
    );
}

/// **A list stops at its ends.** It wrapped — one `j` too many at the bottom
/// of a long table and the cursor was back at the top with nothing to say
/// why, which reads as the cursor escaping rather than as a feature. Every
/// list shares `nav::step`, so this holds for all of them.
#[test]
fn arrows_and_page_keys_stop_at_the_ends() {
    let mut app = fixture(12, 80, 24);
    assert_eq!(app.library.selected, Some(0));
    press(&mut app, Key::plain(KeyCode::Up));
    assert_eq!(app.library.selected, Some(0), "up from the top stays put");
    press(&mut app, Key::ch('G'));
    press(&mut app, Key::ch('j'));
    assert_eq!(
        app.library.selected,
        Some(11),
        "down from the bottom stays put"
    );
    press(&mut app, Key::ch('g'));
    press(&mut app, Key::plain(KeyCode::PageDown));
    assert_eq!(
        app.library.selected,
        Some(11),
        "a page down clamps at the end"
    );
    press(&mut app, Key::plain(KeyCode::PageUp));
    assert_eq!(app.library.selected, Some(0));
    press(&mut app, Key::ch('G'));
    assert_eq!(app.library.selected, Some(11));
    press(&mut app, Key::ch('g'));
    assert_eq!(app.library.selected, Some(0));
}

#[test]
fn the_selected_row_is_measured_first() {
    let mut app = fixture(12, 80, 24);
    let effects = press(&mut app, Key::ch('j'));
    let wanted = effects
        .iter()
        .find_map(|e| match e {
            Effect::RequestSizes(paths) => Some(paths.clone()),
            _ => None,
        })
        .expect("moving the selection re-prioritises the scanner");
    assert_eq!(wanted[0], app.library.selected().unwrap().path);
    assert_eq!(wanted.len(), 12, "every visible row is wanted");
}

#[test]
fn sizes_landing_leave_the_selection_alone() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('j'));
    press(&mut app, Key::ch('j'));
    let before = selected_name(&app);
    let path = app.library.row(0).unwrap().path.clone();
    let effects = update(&mut app, Msg::Sizes(vec![(path.clone(), Some(4096))]));
    assert!(effects.is_empty());
    assert_eq!(selected_name(&app), before);
    assert_eq!(app.library.sizes.get(&path), Some(&Some(4096)));
}

#[test]
fn the_search_bar_matches_inside_a_name_and_esc_clears_then_leaves() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('/'));
    assert!(app.search.editing);
    type_text(&mut app, "lulaby");
    assert!(!app.library.is_empty());
    assert!(
        names(&app).iter().all(|n| n.contains("Lullaby_Remix")),
        "a word with a dropped letter still finds the name, and nothing else: {:?}",
        names(&app)
    );
    assert!(
        app.library
            .match_info(0)
            .is_some_and(|i| !i.name_hits.is_empty()),
        "the hit characters are known, for highlighting"
    );

    // Letters picked from across the name — and across the id, the template
    // and the tags — are not a match. This is what "too fuzzy" looked like.
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "lulrmx");
    assert!(app.library.is_empty(), "{:?}", names(&app));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "cdraft");
    assert!(
        app.library.is_empty(),
        "a word cannot match half in one field and half in another: {:?}",
        names(&app)
    );
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "lulla");
    assert!(
        names(&app).iter().all(|n| n.contains("Lullaby_Remix")),
        "a substring finds the name: {:?}",
        names(&app)
    );

    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.search.editing, "the first Esc clears the query");
    assert!(app.search.input.is_empty());
    assert_eq!(app.library.len(), 12);
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(!app.search.editing, "the second Esc leaves the bar");
}

#[test]
fn a_structured_predicate_filters_from_the_row_and_a_variable_asks_for_metadata() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('/'));
    let effects = type_text(&mut app, "tag:draft");
    assert!(
        app.library.len() == 8 && !effects.iter().any(|e| matches!(e, Effect::LoadMeta(_))),
        "tags live on the row, so no file is read: {} rows, {effects:?}",
        app.library.len()
    );

    press(&mut app, Key::ctrl('u'));
    let effects = type_text(&mut app, "artist=Aria*");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadMeta(paths) if paths.len() == 12)),
        "a template variable needs the metadata of every row: {effects:?}"
    );
    assert!(
        app.library.is_empty(),
        "nothing matches until the metadata answers"
    );
}

#[test]
fn sort_cycles_and_relevance_follows_a_fuzzy_query() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('s'));
    assert!(app.status.text.contains("oldest"));
    assert!(names(&app)[0].contains("2026-08-17"));
    press(&mut app, Key::ch('s'));
    assert!(app.status.text.contains("name"));

    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "shoot");
    assert_eq!(
        app.library.effective_sort(&app.search.query).label(),
        "relevance",
        "a bare word sorts by how well it matched"
    );
}

#[test]
fn f_filters_by_the_selected_template_and_big_f_clears() {
    let mut app = fixture(12, 80, 24);
    let slug = app.library.selected().unwrap().template.clone();
    press(&mut app, Key::ch('f'));
    assert_eq!(app.library.template_filter.as_deref(), Some(slug.as_str()));
    assert!((0..app.library.len()).all(|r| app.library.row(r).unwrap().template == slug));
    press(&mut app, Key::ch('F'));
    assert!(app.library.template_filter.is_none());
    assert_eq!(app.library.len(), 12);
}

#[test]
fn esc_clears_the_query_and_the_filter_before_it_quits() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "lulla");
    press(&mut app, Key::plain(KeyCode::Enter));
    press(&mut app, Key::ch('f'));
    let no_quit = |effects: Vec<Effect>| !effects.iter().any(|e| matches!(e, Effect::Quit(_)));
    assert!(no_quit(press(&mut app, Key::plain(KeyCode::Esc))));
    assert!(app.search.input.is_empty(), "the query goes first");
    assert!(no_quit(press(&mut app, Key::plain(KeyCode::Esc))));
    assert!(app.library.template_filter.is_none(), "then the filter");
    assert_eq!(
        press(&mut app, Key::plain(KeyCode::Esc)),
        vec![Effect::Quit(Exit::Normal)],
        "then, with nothing left to clear, Esc quits"
    );
}

#[test]
fn too_small_swallows_everything_but_q() {
    let mut app = fixture(3, 40, 10);
    assert!(press(&mut app, Key::ch('j')).is_empty());
    assert!(press(&mut app, Key::ch('n')).is_empty());
    assert_eq!(
        press(&mut app, Key::ch('q')),
        vec![Effect::Quit(Exit::Normal)]
    );
}

#[test]
fn ctrl_c_closes_a_dialog_first_and_then_interrupts() {
    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('?'));
    assert!(press(&mut app, Key::ctrl('c')).is_empty());
    assert!(app.modals.is_empty());
    assert_eq!(
        press(&mut app, Key::ctrl('c')),
        vec![Effect::Quit(Exit::Interrupted)]
    );
}

#[test]
fn the_palette_ranks_the_folder_command_first_for_open() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('c'));
    type_text(&mut app, "open");
    let titles: Vec<String> = match app.modals.top() {
        Some(fastf::tui::app::modal::Modal::Palette(p)) => {
            p.entries.iter().map(|e| e.title.clone()).collect()
        }
        _ => panic!("the palette should be open"),
    };
    assert_eq!(titles[0], "Open project folder", "{titles:?}");
    assert_eq!(titles[1], "Open terminal here", "{titles:?}");
    // Enter runs it exactly as the key would.
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(app.modals.is_empty());
    assert!(matches!(
        effects.first(),
        Some(Effect::Spawn(SpawnKind::Reveal(_)))
    ));
}

#[test]
fn the_palette_jumps_to_a_project() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('c'));
    type_text(&mut app, "#test run");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        selected_name(&app).contains("Test_Run"),
        "{}",
        selected_name(&app)
    );
    assert_eq!(app.focus, Focus::Projects);
}

#[test]
fn enter_opens_the_native_action_menu() {
    let mut app = fixture(12, 80, 24);
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(effects.is_empty());
    assert!(matches!(app.modals.top(), Some(Modal::Actions(_))));
    // Esc closes it back to the list.
    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(effects.is_empty());
    assert!(app.modals.is_empty());
}

#[test]
fn copy_and_show_path_use_the_selected_project() {
    let mut app = fixture(12, 80, 24);
    let path = app.library.selected().unwrap().path.clone();
    let effects = press(&mut app, Key::ch('y'));
    assert!(
        matches!(&effects[..], [Effect::Spawn(SpawnKind::Clipboard(text))] if text.contains("Lullaby"))
    );
    press(&mut app, Key::ch('p'));
    assert!(app.status.text.contains(&path.display().to_string()));

    // A clipboard with no tool falls back to showing the path in a dialog.
    update(
        &mut app,
        Msg::Spawned {
            what: SpawnKind::Clipboard(path.display().to_string()),
            outcome: Err("no clipboard tool found".to_string()),
        },
    );
    assert!(matches!(
        app.modals.top(),
        Some(fastf::tui::app::modal::Modal::Message { .. })
    ));
}

#[test]
fn a_patched_row_keeps_its_place_and_forgets_its_size() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('j'));
    let mut patched = app.library.selected().unwrap().clone();
    patched.tags.push("urgent".to_string());
    let path = patched.path.clone();
    app.library.sizes.insert(path.clone(), Some(10));

    let id = run_id(&press(&mut app, Key::ch('R')));
    let effects = update(
        &mut app,
        item_done(
            id,
            ListChange::Patched {
                project: Box::new(patched),
                was: path.clone(),
                stale: vec![path.clone()],
            },
        ),
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![path.clone()])));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Discover { .. })),
        "{effects:?}"
    );
    assert_eq!(app.library.selected, Some(1));
    assert!(
        app.library
            .selected()
            .unwrap()
            .tags
            .contains(&"urgent".to_string())
    );
    assert!(
        !app.library.sizes.contains_key(&path),
        "the size is pending again"
    );
    assert!(app.library.known_tags.contains(&"urgent".to_string()));
}

#[test]
fn a_removed_row_leaves_and_the_selection_clamps() {
    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('G'));
    let doomed = app.library.selected().unwrap().path.clone();
    let id = run_id(&press(&mut app, Key::ch('R')));
    let effects = update(
        &mut app,
        item_done(
            id,
            ListChange::Removed {
                path: doomed.clone(),
            },
        ),
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![doomed.clone()])));
    assert_eq!(app.library.len(), 2);
    assert_eq!(app.library.selected, Some(1));
    assert!(!names(&app).iter().any(|n| doomed.ends_with(n)));
}

#[test]
fn a_patch_during_a_discovery_in_flight_asks_once_more() {
    let mut app = fixture(3, 80, 24);
    let effects = press(&mut app, Key::plain(KeyCode::F(5)));
    assert!(effects.contains(&Effect::Discover { generation: 1 }));
    assert!(effects.contains(&Effect::LoadSummary));

    let patched = app.library.selected().unwrap().clone();
    let patched_path = patched.path.clone();
    let id = run_id(&press(&mut app, Key::ch('R')));
    update(
        &mut app,
        item_done(
            id,
            ListChange::Patched {
                project: Box::new(patched),
                was: patched_path,
                stale: Vec::new(),
            },
        ),
    );
    assert!(app.library.dirty);

    let effects = update(
        &mut app,
        Msg::Discovered {
            generation: 1,
            projects: sample_projects(3),
        },
    );
    assert!(
        effects.contains(&Effect::Discover { generation: 2 }),
        "the answer may predate the patch, so ask again: {effects:?}"
    );
    assert!(!app.library.dirty);
}

#[test]
fn reindex_runs_once_and_is_refused_while_busy() {
    let mut app = fixture(3, 80, 24);
    let effects = press(&mut app, Key::ch('R'));
    let id = match effects.as_slice() {
        [Effect::Run(id, action)] if **action == Action::Reindex => *id,
        other => panic!("expected one reindex, got {other:?}"),
    };
    assert!(app.busy.is_some());
    assert!(press(&mut app, Key::ch('R')).is_empty());
    assert!(app.status.text.contains("working"));

    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                ListChange::Reload,
                "✓  Reindexed 3 projects across 1 base.",
            ))),
        },
    );
    assert!(app.busy.is_none());
    assert!(effects.iter().any(|e| matches!(e, Effect::Discover { .. })));
    assert!(app.status.text.contains("Reindexed"));
}

#[test]
fn a_failed_discovery_opens_a_dialog_and_leaves_the_app_usable() {
    let mut app = empty_fixture(80, 24);
    let _ = app.start();
    update(
        &mut app,
        Msg::DiscoverFailed {
            generation: 1,
            error: "parsing config.toml: bad".to_string(),
        },
    );
    assert!(app.library.loaded);
    assert!(matches!(
        app.modals.top(),
        Some(fastf::tui::app::modal::Modal::Message { .. })
    ));
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.modals.is_empty());
    assert_eq!(
        press(&mut app, Key::ch('q')),
        vec![Effect::Quit(Exit::Normal)]
    );
}

#[test]
fn the_status_toast_expires_on_its_own() {
    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('p'));
    assert!(!app.status.text.is_empty());
    assert!(app.needs_tick(), "a toast keeps the clock running");
    app.elapsed_ms = 5_999;
    update(&mut app, Msg::Tick);
    assert!(!app.status.text.is_empty(), "not a moment early");
    app.elapsed_ms = 6_000;
    update(&mut app, Msg::Tick);
    assert!(app.status.text.is_empty());
}

/// The templates tab carries what the strip used to: every template, the
/// orphan slugs after them, and the counts. `f` filters the library by the
/// selected one **and goes back to it** — the strip set the filter and left
/// you looking at the strip, which is the one place the answer is not.
#[test]
fn the_templates_tab_filters_the_library_and_returns_to_it() {
    use fastf::tui::app::Screen;

    let mut app = fixture(12, 120, 40);
    assert_eq!(app.templates.cards.len(), 3);
    assert_eq!(app.studio.cards.len(), 3, "the tab has the same list");

    press(&mut app, Key::ch('T'));
    assert_eq!(app.screen, Screen::Templates);
    let slug = app.studio.selected_slug().unwrap();

    press(&mut app, Key::ch('f'));
    assert_eq!(app.library.template_filter.as_deref(), Some(slug.as_str()));
    assert_eq!(app.screen, Screen::Library, "and back to the projects");

    press(&mut app, Key::ch('T'));
    press(&mut app, Key::ch('f'));
    assert!(
        app.library.template_filter.is_none(),
        "the same template again clears it"
    );
}

/// Tab and Shift-Tab move between the table and the pane; the templates tab is
/// a tab, not a third pane in that ring.
#[test]
fn the_focus_ring_is_the_table_and_the_pane() {
    let mut app = fixture(12, 120, 40);
    assert_eq!(app.focus, Focus::Projects);
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Detail);
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Projects);
}

#[test]
fn the_detail_pane_is_read_once_per_project_and_only_when_visible() {
    let mut app = fixture(12, 120, 40);
    let effects = press(&mut app, Key::ch('j'));
    let wanted: Vec<PathBuf> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::LoadDetail(path) => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(wanted, vec![app.library.selected().unwrap().path.clone()]);
    update(
        &mut app,
        Msg::Detail {
            path: wanted[0].clone(),
            detail: Box::default(),
        },
    );
    press(&mut app, Key::ch('k'));
    let effects = press(&mut app, Key::ch('j'));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::LoadDetail(_))),
        "a cached detail is not read again: {effects:?}"
    );
    // It is *checked* against the disk — a stat, and a read only if the
    // file changed — carrying the stamp the cache holds, so a line added to
    // the file in another window shows on the next visit.
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::RefreshDetail { path, stamp: None } if *path == wanted[0]
        )),
        "a cached detail is checked: {effects:?}"
    );
    // And on F5, which is the key a person presses after editing the file.
    let effects = press(&mut app, Key::plain(KeyCode::F(5)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::RefreshDetail { path, .. } if *path == wanted[0])),
        "{effects:?}"
    );

    let mut narrow = fixture(12, 80, 24);
    let effects = press(&mut narrow, Key::ch('j'));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDetail(_) | Effect::RefreshDetail { .. })),
        "no pane on screen, no read and no check: {effects:?}"
    );
}

/// **The file is the truth, and the pane just read it.** A detail whose
/// metadata disagrees with the row — tags edited in another window, an
/// index that went stale — patches the row and has the base's index entry
/// written to match; a detail that agrees changes nothing.
#[test]
fn a_detail_that_disagrees_with_its_row_patches_the_row_and_the_index() {
    use fastf::core::project_info::Metadata;
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch('j'));
    let project = app.library.selected().unwrap().clone();
    let meta = |tags: Vec<&str>| Metadata {
        id: project.id.clone(),
        id_number: project.id_number,
        template: project.template.clone(),
        template_name: project.template_name.clone(),
        created: project.created.clone(),
        folder: project.name.clone(),
        path: String::new(),
        variables: Default::default(),
        tags: tags.into_iter().map(str::to_string).collect(),
        auto_tags: Vec::new(),
        provisioning: false,
    };
    let detail = fastf::tui::app::data::ProjectDetail {
        meta: Some(meta(project.tags.iter().map(String::as_str).collect())),
        ..Default::default()
    };
    let effects = update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::new(detail),
        },
    );
    assert!(
        effects.is_empty(),
        "the file agrees with the row: {effects:?}"
    );

    let detail = fastf::tui::app::data::ProjectDetail {
        meta: Some(meta(vec!["edited-outside"])),
        ..Default::default()
    };
    let effects = update(
        &mut app,
        Msg::Detail {
            path: project.path.clone(),
            detail: Box::new(detail),
        },
    );
    assert_eq!(
        app.library.selected().unwrap().tags,
        vec!["edited-outside".to_string()],
        "the row took the file's tags"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::RefreshCache(p) if *p == project.path)),
        "{effects:?}"
    );
}

/// The mouse setting, flipped from the palette: written through `config
/// set`, so the word on disk is the word every surface reads, and switched
/// the moment the write is on its way.
#[test]
fn the_mouse_is_a_setting_flipped_live_from_the_palette() {
    use fastf::tui::command::CommandId;
    let mut app = fixture(12, 120, 40);
    assert!(!app.mouse, "off by default: text selects as in any program");
    let effects = app.run(CommandId::ToggleMouse);
    assert!(app.mouse);
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Mouse(true))),
        "{effects:?}"
    );
    let sent = effects.iter().find_map(|e| match e {
        Effect::Run(_, action) => Some(action.as_ref()),
        _ => None,
    });
    assert_eq!(
        sent,
        Some(&Action::SetConfig {
            key: "mouse",
            value: "on".to_string(),
        })
    );
    // Busy: refused, and the terminal is left as it is.
    let effects = app.run(CommandId::ToggleMouse);
    assert!(app.mouse && effects.is_empty(), "{effects:?}");
}

#[test]
fn a_summary_that_names_a_template_no_project_uses_still_gets_a_card() {
    let mut app = empty_fixture(120, 40);
    let _ = app.start();
    let mut projects: Vec<Project> = sample_projects(2);
    projects[0].template = "orphan".to_string();
    update(&mut app, Msg::Summary(Box::new(sample_summary(2))));
    update(
        &mut app,
        Msg::Discovered {
            generation: 1,
            projects,
        },
    );
    let slugs: Vec<&str> = app
        .templates
        .cards
        .iter()
        .map(|c| c.slug.as_str())
        .collect();
    assert!(slugs.contains(&"orphan"), "{slugs:?}");
    assert_eq!(app.templates.count("orphan"), 1);
}

// --- single-project actions ----------------------------------------------

/// The one `Effect::Run` among the effects. Exactly one action may be started
/// at a time, but a batch item's outcome also carries the list maintenance its
/// row change asked for (`ForgetSizes`, `RequestSizes`, a detail read), so the
/// run is looked for rather than required to stand alone.
fn action_of(effects: &[Effect]) -> &Action {
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

#[test]
fn rederive_rename_and_move_each_run_their_action() {
    use fastf::tui::command::CommandId;

    // Re-derive tags: no prompt, straight to the worker.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    let effects = app.run(CommandId::ReautoTags);
    assert!(matches!(
        action_of(&effects),
        Action::ReautoTags(p) if **p == selected
    ));

    // Rename: a text prompt pre-filled with the current name.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('r'));
    assert!(matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
    press(&mut app, Key::ctrl('u'));
    type_text(&mut app, "New_Name");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::Rename { project, name } if **project == selected && name == "New_Name"
    ));
}

#[test]
fn add_and_remove_tags_run_their_actions() {
    // Add: the library already knows `client/Acme`, which the selected project
    // lacks, so `A` offers it in a picker.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('A'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::AddTag { project, tag } if **project == selected && tag == "client/Acme"
    ));

    // Remove: a multi-pick of the project's own tags, Space toggles.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ctrl('t'));
    assert!(matches!(app.modals.top(), Some(Modal::MultiPick(_))));
    press(&mut app, Key::ch(' '));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::RemoveTags { project, tags } if **project == selected && tags == &vec!["draft".to_string()]
    ));
}

#[test]
fn move_picks_a_target_and_runs_a_move_action() {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ch('m'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::Move { project, target } if **project == selected
            && target == Path::new("/media/usb/archive")
    ));
    assert!(app.move_progress.is_some(), "the progress modal is up");
}

#[test]
fn notes_run_their_actions_and_the_editor_suspends() {
    // The quick note types inline and appends.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    press(&mut app, Key::ctrl('n'));
    assert!(matches!(app.modals.top(), Some(Modal::Note(_))));
    type_text(&mut app, "mixing started");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::AppendNote { project, text } if **project == selected && text == "mixing started"
    ));

    // `N` opens $EDITOR, which runs while the screen is suspended.
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();
    let effects = press(&mut app, Key::ch('N'));
    assert!(matches!(
        effects.as_slice(),
        [Effect::Suspend(Suspended::Note(project))] if **project == selected
    ));
}

#[test]
fn an_action_done_patch_forgets_the_stale_sizes() {
    use fastf::tui::effect::ActionOutcome;

    let mut app = fixture(12, 80, 24);
    let selected_path = app.library.selected().unwrap().path.clone();
    app.library.sizes.insert(selected_path.clone(), Some(10));

    // Start an add-tag action to get a busy id.
    press(&mut app, Key::ch('A'));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id = match effects.as_slice() {
        [Effect::Run(id, _)] => *id,
        other => panic!("{other:?}"),
    };
    assert!(app.busy.is_some());

    let mut patched = app.library.selected().unwrap().clone();
    patched.tags.push("client/Acme".to_string());
    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(ActionOutcome::new(
                ListChange::Patched {
                    project: Box::new(patched),
                    was: selected_path.clone(),
                    stale: vec![selected_path.clone()],
                },
                "Added 1 tag",
            ))),
        },
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![selected_path])));
    assert!(app.busy.is_none());
    assert!(app.move_progress.is_none());
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Discover { .. })),
        "a tag patch must not rescan: {effects:?}"
    );
}

#[test]
fn an_action_done_removal_clamps_the_selection() {
    use fastf::tui::effect::ActionOutcome;

    let mut app = fixture(3, 80, 24);
    press(&mut app, Key::ch('G'));
    let doomed = app.library.selected().unwrap().path.clone();
    let name = app.library.selected().unwrap().name.clone();

    // Delete: the word confirms, and the prompt names the folder.
    press(&mut app, Key::ch('D'));
    assert!(
        matches!(app.modals.top(), Some(Modal::TextPrompt(prompt)) if prompt.title.contains(&name))
    );
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id = match effects.as_slice() {
        [Effect::Run(id, _)] => *id,
        other => panic!("{other:?}"),
    };

    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Ok(Box::new(ActionOutcome::new(
                ListChange::Removed {
                    path: doomed.clone(),
                },
                "Deleted",
            ))),
        },
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![doomed])));
    assert_eq!(app.library.len(), 2);
    assert_eq!(app.library.selected, Some(1));
}

#[test]
fn delete_asks_for_the_word_and_a_mismatch_deletes_nothing() {
    let mut app = fixture(12, 80, 24);
    let name = app.library.selected().unwrap().name.clone();
    press(&mut app, Key::ch('D'));
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("delete asks in a text prompt");
    };
    assert!(
        prompt.title.contains(&name) && prompt.title.contains("Type delete"),
        "the question names the folder and the word: {}",
        prompt.title
    );
    type_text(&mut app, "delet");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(effects.is_empty(), "nothing runs: {effects:?}");
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("a mismatch keeps the prompt up, text and all");
    };
    assert_eq!(prompt.input.text(), "delet");
    assert!(
        prompt
            .error
            .as_deref()
            .is_some_and(|e| e.contains("type delete to confirm — nothing deleted")),
        "{:?}",
        prompt.error
    );
    type_text(&mut app, "e");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::Delete(_)),
        "the word, any case, deletes: {effects:?}"
    );
    assert!(app.modals.is_empty());
}

#[test]
fn y_and_n_answer_a_confirm_without_enter() {
    let mut app = fixture(12, 80, 24);
    let selected = app.library.selected().unwrap().clone();

    press(&mut app, Key::ch('u'));
    assert!(matches!(app.modals.top(), Some(Modal::Confirm(_))));
    // `n` answers without Enter and runs nothing.
    let effects = press(&mut app, Key::ch('n'));
    assert!(effects.is_empty());
    assert!(app.modals.is_empty());

    // `y` runs the unregister action.
    press(&mut app, Key::ch('u'));
    let effects = press(&mut app, Key::ch('y'));
    assert!(matches!(
        action_of(&effects),
        Action::Unregister(project) if **project == selected
    ));
}

#[test]
fn quit_keys_cancel_a_running_move() {
    use fastf::core::assets::Progress;

    let mut app = fixture(12, 80, 24);
    app.move_progress = Some(Progress::new(&[]));
    // Every quit gesture cancels the job instead of abandoning it mid-write.
    assert_eq!(press(&mut app, Key::ctrl('c')), vec![Effect::CancelMove]);
    assert_eq!(
        press(&mut app, Key::plain(KeyCode::Esc)),
        vec![Effect::CancelMove]
    );
    assert_eq!(press(&mut app, Key::ch('q')), vec![Effect::CancelMove]);
}

// ---------------------------------------------------------------------------
// Marks: what a batch verb will act on
// ---------------------------------------------------------------------------

#[test]
fn space_toggles_the_selected_row_and_steps_on() {
    let mut app = fixture(12, 80, 24);
    let first = app.library.selected().unwrap().path.clone();

    press(&mut app, Key::ch(' '));
    assert!(app.library.marks.contains(&first), "Space marks the row");
    assert_eq!(app.library.selected_index(), Some(1), "and moves on");

    let second = app.library.selected().unwrap().path.clone();
    press(&mut app, Key::ch(' '));
    assert!(app.library.marks.contains(&second));

    // Back to the first row: Space unmarks it and steps on again.
    press(&mut app, Key::plain(KeyCode::Home));
    press(&mut app, Key::ch(' '));
    assert!(!app.library.marks.contains(&first));
    assert_eq!(app.library.selected_index(), Some(1));
}

#[test]
fn mark_all_marks_what_the_current_view_shows() {
    let mut app = fixture(12, 80, 24);
    // Narrow the view to the two Lullaby rows.
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "lullaby");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(app.library.len(), 2, "the query narrows the view");
    let visible: Vec<PathBuf> = (0..app.library.len())
        .filter_map(|row| app.library.row(row).map(|p| p.path.clone()))
        .collect();
    let hidden = app
        .library
        .snapshot
        .iter()
        .find(|p| !visible.contains(&p.path))
        .unwrap()
        .path
        .clone();

    press(&mut app, Key::ch('*'));
    assert_eq!(
        app.library.marks.len(),
        2,
        "only what the view shows is marked"
    );
    assert!(visible.iter().all(|p| app.library.marks.contains(p)));
    assert!(!app.library.marks.contains(&hidden));
    assert!(
        app.status.text.contains("2 marked"),
        "{:?}",
        app.status.text
    );
}

#[test]
fn minus_clears_the_marks_and_says_so() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    assert_eq!(app.library.marks.len(), 2);

    press(&mut app, Key::ch('-'));
    assert!(app.library.marks.is_empty());
    assert!(
        app.status.text.contains("2 marks cleared"),
        "{:?}",
        app.status.text
    );
}

#[test]
fn esc_clears_marks_before_it_quits() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' '));
    assert_eq!(app.library.marks.len(), 1);

    assert!(press(&mut app, Key::plain(KeyCode::Esc)).is_empty());
    assert!(
        app.library.marks.is_empty(),
        "the first Esc clears the marks"
    );
    assert!(app.status.text.contains("marks cleared"));
    assert_eq!(
        press(&mut app, Key::plain(KeyCode::Esc)),
        vec![Effect::Quit(Exit::Normal)],
        "the second Esc quits"
    );
}

#[test]
fn targets_are_the_marks_in_display_order() {
    let mut app = fixture(12, 80, 24);
    // Marks are a set; the order a job runs in comes from the view.
    let last = app.library.row(app.library.len() - 1).unwrap().path.clone();
    let first = app.library.row(0).unwrap().path.clone();
    app.library.marks.insert(last.clone());
    app.library.marks.insert(first.clone());

    let targets = app.library.targets();
    let paths: Vec<PathBuf> = targets.iter().map(|p| p.path.clone()).collect();
    assert_eq!(paths, vec![first, last], "marks run in display order");
}

#[test]
fn removing_a_row_drops_its_mark() {
    let mut app = fixture(12, 80, 24);
    let doomed = app.library.selected().unwrap().path.clone();
    press(&mut app, Key::ch(' '));
    assert!(app.library.marks.contains(&doomed));

    // What the engine does when a delete completes: the row goes, and with it
    // its mark — a deleted project cannot stay a batch target.
    app.library.remove(&doomed);
    assert!(!app.library.marks.contains(&doomed));
}

// ---------------------------------------------------------------------------
// Batch jobs over the marks
// ---------------------------------------------------------------------------

/// The id the one `Effect::Run` among the effects carries.
fn run_id(effects: &[Effect]) -> fastf::tui::effect::ActionId {
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

fn item_done(id: fastf::tui::effect::ActionId, change: ListChange) -> Msg {
    Msg::ActionDone {
        id,
        outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
            change, "done",
        ))),
    }
}

/// A success is marked with the theme's tick, not with a literal one.
///
/// Twelve of `runtime::run_action`'s messages carried a hardcoded `✓`, which
/// `Glyphs::ascii` maps to `+` — so on a legacy Windows console, or anywhere
/// with `FASTF_ASCII=1`, they drew a replacement box beside the app's own
/// correctly-themed messages. `run_action` runs on a worker with no theme to
/// ask; `App::good` is the one place all of them pass through.
#[test]
fn a_success_message_wears_the_theme_glyph_not_a_hardcoded_one() {
    use fastf::tui::theme::{Glyphs, Theme};

    let mut app = fixture(4, 100, 30);
    app.theme = Theme::mono().with_glyphs(Glyphs::ascii());

    let effects = press(&mut app, Key::ch('A'));
    let effects = if effects.iter().any(|e| matches!(e, Effect::Run(_, _))) {
        effects
    } else {
        // `A` opens a tag picker first; answer it.
        press(&mut app, Key::plain(KeyCode::Enter))
    };
    let id = run_id(&effects);
    let _ = update(&mut app, item_done(id, ListChange::SummaryOnly));

    let shown = &app.status.text;
    assert!(
        !shown.contains(Glyphs::unicode().check),
        "the ASCII theme must not draw a unicode tick: {shown:?}"
    );
    assert!(
        shown.starts_with(Glyphs::ascii().check),
        "and it must draw its own: {shown:?}"
    );
}

#[test]
fn delete_over_marks_confirms_once_then_runs_each_item() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    let first = app.library.row(0).unwrap().path.clone();
    let second = app.library.row(1).unwrap().path.clone();

    // `D` over marks asks for the same word, naming every folder.
    press(&mut app, Key::ch('D'));
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("delete asks in a text prompt");
    };
    assert!(
        prompt.title.contains("these 2 projects"),
        "the question counts the marks: {}",
        prompt.title
    );
    type_text(&mut app, "DELETE");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id1 = run_id(&effects);
    assert!(matches!(action_of(&effects), Action::Delete(project) if project.path == first));
    assert!(app.job.is_some(), "a job is running");
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);

    // The first item lands: its row goes, and the next item starts.
    let effects = update(
        &mut app,
        item_done(
            id1,
            ListChange::Removed {
                path: first.clone(),
            },
        ),
    );
    let id2 = run_id(&effects);
    assert!(
        !app.library.marks.contains(&first),
        "a deleted row loses its mark"
    );
    assert!(matches!(action_of(&effects), Action::Delete(project) if project.path == second));

    // The second lands: the job finishes clean — no report, just the status.
    let effects = update(
        &mut app,
        item_done(
            id2,
            ListChange::Removed {
                path: second.clone(),
            },
        ),
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Run(..))),
        "the job is over, nothing else may start: {effects:?}"
    );
    assert!(app.job.is_none());
    assert!(app.modals.is_empty(), "a clean job needs no report modal");
    assert!(
        app.status.text.contains("2 deleted"),
        "{:?}",
        app.status.text
    );
    assert!(app.library.marks.is_empty());
}

#[test]
fn a_failed_item_keeps_its_mark_and_opens_a_report() {
    let mut app = fixture(12, 80, 24);
    let doomed = app.library.selected().unwrap().path.clone();
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch('D'));
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id = run_id(&effects);

    let effects = update(
        &mut app,
        Msg::ActionDone {
            id,
            outcome: Err("injected fault at 'delete:mid-copy'".to_string()),
        },
    );
    assert!(effects.is_empty());
    assert!(app.job.is_none(), "the job is over");
    assert!(
        app.library.marks.contains(&doomed),
        "the failed row stays marked for a retry"
    );
    assert!(
        app.status.text.contains("1 failed"),
        "{:?}",
        app.status.text
    );
    match app.modals.top() {
        Some(Modal::Message { title, lines, .. }) => {
            assert_eq!(title, "delete report");
            assert!(lines.iter().any(|l| l.contains("mid-copy")), "{lines:?}");
        }
        other => panic!("expected the failure report, got {other:?}"),
    }
}

#[test]
fn esc_cancels_a_job_and_the_rest_stay_marked() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    let first = app.library.row(0).unwrap().path.clone();
    let second = app.library.row(1).unwrap().path.clone();
    press(&mut app, Key::ch('D'));
    type_text(&mut app, "delete");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let id1 = run_id(&effects);
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);

    // Esc while an item runs asks the job to stop after it — no quitting, no
    // CancelMove (nothing is in flight to cancel for a delete).
    assert!(press(&mut app, Key::plain(KeyCode::Esc)).is_empty());
    assert!(app.job.as_ref().unwrap().cancelled);

    // The running item still lands: its row goes, its mark with it. Nothing
    // new starts, and the unrun row keeps its mark.
    let effects = update(
        &mut app,
        item_done(
            id1,
            ListChange::Removed {
                path: first.clone(),
            },
        ),
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Run(..))),
        "no further item may start: {effects:?}"
    );
    assert!(app.job.is_none(), "the cancelled job is over");
    assert!(!app.library.marks.contains(&first));
    assert!(
        app.library.marks.contains(&second),
        "the unrun row stays marked"
    );
    assert!(app.library.snapshot.iter().any(|p| p.path == second));
    assert!(
        app.status.text.contains("1 deleted — cancelled"),
        "{:?}",
        app.status.text
    );
    match app.modals.top() {
        Some(Modal::Message { title, lines, .. }) => {
            assert_eq!(title, "delete report");
            assert!(
                lines.iter().any(|l| l.contains("1 project is left marked")),
                "{lines:?}"
            );
        }
        other => panic!("expected the cancel report, got {other:?}"),
    }
}

#[test]
fn move_over_marks_runs_a_move_job() {
    let mut app = fixture(12, 80, 24);
    app.summary = Some(sample_summary_moveable(12));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let first = app.library.row(0).unwrap().clone();

    press(&mut app, Key::ch('m'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(matches!(
        action_of(&effects),
        Action::Move { project, .. } if **project == first
    ));
    assert!(app.job.is_some(), "the marks started a move job");
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);
    assert!(app.move_progress.is_some(), "the progress modal is up");
}

#[test]
fn unregister_over_marks_confirms_the_count_then_runs() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch('u'));
    let prompt = match app.modals.top() {
        Some(Modal::Confirm(confirm)) => confirm.prompt.clone(),
        other => panic!("expected a batch confirm, got {other:?}"),
    };
    assert!(prompt.contains("2 projects"), "{prompt}");
    let effects = press(&mut app, Key::ch('y'));
    assert!(matches!(action_of(&effects), Action::Unregister(_)));
    assert!(app.job.is_some());
    assert_eq!(app.job.as_ref().unwrap().pending.len(), 1);
}

// ---------------------------------------------------------------------------
// The flows: create, apply, register
// ---------------------------------------------------------------------------

mod flows {
    use super::*;
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
}

// ---------------------------------------------------------------------------
// The template studio and the builder
// ---------------------------------------------------------------------------

mod studio {
    use super::*;
    use fastf::core::template::Template;
    use fastf::tui::app::studio::{Open, Row, Section};
    use fastf::tui::effect::Request;

    fn builder(app: &App) -> &fastf::tui::app::studio::Builder {
        match app.modals.top() {
            Some(Modal::Builder(builder)) => builder,
            other => panic!("expected the builder, got {other:?}"),
        }
    }

    /// `T` → `n`: a new template, on the section list.
    fn open_new(app: &mut App) {
        press(app, Key::ch('T'));
        press(app, Key::ch('n'));
    }

    #[test]
    fn the_templates_tab_lists_them_and_reads_the_selected_one() {
        use fastf::tui::app::Screen;

        let mut app = fixture(6, 120, 40);
        let effects = press(&mut app, Key::ch('T'));
        assert_eq!(app.screen, Screen::Templates);
        assert_eq!(app.studio.cards.len(), 3);
        assert!(
            effects.iter().any(
                |e| matches!(e, Effect::LoadTemplateView { slug } if slug == "client-project")
            ),
            "the tab is alphabetical, real templates first: {effects:?}"
        );
        let effects = press(&mut app, Key::plain(KeyCode::Down));
        assert!(
            matches!(&effects[..], [Effect::LoadTemplateView { slug }] if slug == "general"),
            "moving reads the next one: {effects:?}"
        );
        // `T` again is the way back, and Esc is the other one.
        press(&mut app, Key::ch('T'));
        assert_eq!(app.screen, Screen::Library);
    }

    /// The tab's own search box: a plain substring over the slugs and names,
    /// with the cursor kept on a row the query still keeps.
    /// `fastf template new` and `template edit <slug>` open the app on the
    /// templates tab, so Esc out of the builder leaves you among the templates
    /// rather than in a library nobody asked for.
    #[test]
    fn template_new_from_the_command_line_opens_on_the_tab() {
        use fastf::tui::app::Screen;
        use fastf::tui::entry::StudioEntry;

        let mut app = fixture(6, 120, 40);
        app.studio_entry = Some(StudioEntry::New);
        let _ = app.start();
        assert_eq!(app.screen, Screen::Templates);
        assert!(matches!(app.modals.top(), Some(Modal::Builder(_))));

        let _ = press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.modals.is_empty(), "Esc discards the new template");
        assert_eq!(app.screen, Screen::Templates, "and lands on the tab");
    }

    #[test]
    fn the_templates_tab_filters_its_own_list() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        assert_eq!(app.studio.rows("").len(), 3);

        press(&mut app, Key::ch('/'));
        type_text(&mut app, "music");
        let rows = app.studio.rows(app.search.input.text());
        assert_eq!(rows.len(), 1, "one template matches");
        assert_eq!(
            app.studio.selected_slug().as_deref(),
            Some("music-video"),
            "the cursor lands on a row the query keeps"
        );
    }

    #[test]
    fn a_late_read_for_a_row_that_moved_on_is_dropped() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        let _ = update(
            &mut app,
            Msg::TemplateViewLoaded {
                slug: "music-video".to_string(),
                lines: vec!["stale".to_string()],
            },
        );
        assert!(app.studio.lines.is_empty(), "a stale read is dropped");
        let _ = update(
            &mut app,
            Msg::TemplateViewLoaded {
                slug: "client-project".to_string(),
                lines: vec!["Client project".to_string()],
            },
        );
        assert_eq!(app.studio.lines, vec!["Client project".to_string()]);
    }

    #[test]
    fn a_section_opens_commits_and_closes_without_writing_anything() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        assert!(builder(&app).open.is_none(), "the section list first");

        press(&mut app, Key::plain(KeyCode::Enter)); // → Metadata
        assert!(matches!(builder(&app).open, Some(Open::Metadata(_))));
        type_text(&mut app, "Music video");
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(effects.is_empty(), "a section writes nothing: {effects:?}");
        assert!(builder(&app).open.is_none(), "back on the section list");
        assert_eq!(builder(&app).template.name, "Music video");
        assert_eq!(
            builder(&app).template.slug,
            "music-video",
            "the slug follows the name until one is typed"
        );
    }

    #[test]
    fn save_refuses_an_invalid_template_and_says_so() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        // Straight to Save with nothing filled in.
        for _ in 0..5 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        assert_eq!(builder(&app).row(), Row::Save);
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(effects.is_empty(), "nothing was written: {effects:?}");
        let error = builder(&app).error.clone().expect("a refusal");
        assert!(error.starts_with("Cannot save:"), "{error}");
        assert!(!app.modals.is_empty(), "the builder is still open");
    }

    #[test]
    fn a_valid_template_is_handed_to_the_runtime_to_write() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter)); // → Metadata
        type_text(&mut app, "Demo");
        press(&mut app, Key::plain(KeyCode::Enter));
        for _ in 0..5 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        match action_of(&effects) {
            Action::SaveTemplate {
                template,
                original_slug,
            } => {
                assert_eq!(template.slug, "demo");
                assert_eq!(*original_slug, None, "a new template renames nothing");
            }
            other => panic!("expected a save, got {other:?}"),
        }
        // The builder stays up until the write has actually landed — a
        // refusal from under the lock has to have something to land on. It
        // closes when the outcome says the template is on disk.
        assert!(builder(&app).saving, "the save is in flight");
        let id = run_id(&effects);
        let _ = update(&mut app, item_done(id, ListChange::SummaryOnly));
        assert!(
            app.modals.is_empty(),
            "the builder closed onto the tab it came from"
        );
        assert_eq!(app.screen, fastf::tui::app::Screen::Templates);
    }

    #[test]
    fn editing_reads_the_template_and_remembers_what_it_was_called() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        // Down once: not the first row, so the read is plainly the selected
        // one and not whatever happens to sort first.
        press(&mut app, Key::plain(KeyCode::Down));
        let effects = press(&mut app, Key::ch('e'));
        assert!(
            matches!(&effects[..], [Effect::LoadTemplateSource { slug }] if slug == "general"),
            "{effects:?}"
        );
        assert_eq!(
            builder(&app).pending.as_deref(),
            Some("general"),
            "the screen says which template it is reading"
        );

        let template = Template {
            name: "General".to_string(),
            slug: "general".to_string(),
            ..Template::default()
        };
        let _ = update(
            &mut app,
            Msg::TemplateSourceLoaded {
                slug: "general".to_string(),
                result: Ok(Box::new(template)),
            },
        );
        assert!(builder(&app).pending.is_none());
        assert_eq!(builder(&app).original_slug.as_deref(), Some("general"));
    }

    /// A read that answers for a template the builder has moved off is dropped.
    ///
    /// `TemplateSourceLoaded` replaced whatever builder was on top with
    /// whatever landed, checking nothing. Enter on one template, Esc while it
    /// is still reading, Enter on another: on a slow disk or a network share
    /// the first read arrives and silently becomes the second's contents, and
    /// the second's own read then wipes anything typed meanwhile.
    /// `on_template_loaded` and `TemplateViewLoaded` both guard this; the
    /// builder's own read was the one that did not.
    #[test]
    fn a_template_read_for_a_builder_that_moved_on_is_dropped() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::plain(KeyCode::Enter));
        // Esc out while it is still reading, then open a different one.
        press(&mut app, Key::plain(KeyCode::Esc));
        press(&mut app, Key::plain(KeyCode::Down));
        press(&mut app, Key::plain(KeyCode::Enter));
        let awaited = builder(&app)
            .pending
            .clone()
            .expect("the second builder is reading");

        // The *first* read lands late.
        let stale = Template {
            name: "Stale".to_string(),
            slug: "not-the-one".to_string(),
            ..Template::default()
        };
        assert_ne!(awaited, "not-the-one");
        let _ = update(
            &mut app,
            Msg::TemplateSourceLoaded {
                slug: "not-the-one".to_string(),
                result: Ok(Box::new(stale)),
            },
        );
        assert_eq!(
            builder(&app).pending.as_deref(),
            Some(awaited.as_str()),
            "an answer to a question nobody asked changes nothing"
        );
        assert_ne!(builder(&app).template.name, "Stale");

        // And the one it is waiting for still lands.
        let wanted = Template {
            name: "Wanted".to_string(),
            slug: awaited.clone(),
            ..Template::default()
        };
        let _ = update(
            &mut app,
            Msg::TemplateSourceLoaded {
                slug: awaited.clone(),
                result: Ok(Box::new(wanted)),
            },
        );
        assert!(builder(&app).pending.is_none());
        assert_eq!(builder(&app).template.name, "Wanted");
    }

    /// Quitting from the palette asks the same question Esc asks.
    ///
    /// `CommandId::Quit` ran `Effect::Quit` on the spot, and it is reachable
    /// from `c` → "quit" → Enter and from the too-small-window guard as well as
    /// from `q` — so a template worked on for ten minutes went with one
    /// keystroke while Esc on the same screen asked first. Every quit goes
    /// through `App::quit` now, and answering the question still quits.
    #[test]
    fn quitting_over_a_worked_on_template_asks_first() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "Music video");
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(builder(&app).is_dirty(), "the fixture has to be dirty");

        let effects = app.run(fastf::tui::command::CommandId::Quit);
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Quit(_))),
            "a worked-on template is not thrown away without a question: {effects:?}"
        );
        assert!(
            matches!(app.modals.top(), Some(Modal::Confirm(_))),
            "and the question is the one Esc asks"
        );

        // Answering it does what was asked: the template goes *and* so do we.
        let effects = press(&mut app, Key::ch('y'));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Quit(Exit::Normal))),
            "answering the question must still quit: {effects:?}"
        );
    }

    #[test]
    fn the_variables_section_adds_edits_reorders_and_removes() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Down)); // ID
        press(&mut app, Key::plain(KeyCode::Down)); // Variables
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(matches!(builder(&app).open, Some(Open::Variables(_))));

        for slug in ["artist", "title"] {
            press(&mut app, Key::ch('a'));
            type_text(&mut app, slug);
            press(&mut app, Key::plain(KeyCode::Enter));
        }
        let slugs: Vec<String> = builder(&app)
            .template
            .variables
            .iter()
            .map(|v| v.slug.clone())
            .collect();
        assert_eq!(slugs, vec!["artist".to_string(), "title".to_string()]);

        // `K` moves the selected row up — what the sort prompt used to be.
        press(&mut app, Key::ch('K'));
        let slugs: Vec<String> = builder(&app)
            .template
            .variables
            .iter()
            .map(|v| v.slug.clone())
            .collect();
        assert_eq!(slugs, vec!["title".to_string(), "artist".to_string()]);

        press(&mut app, Key::ch('d'));
        assert_eq!(builder(&app).template.variables.len(), 1);
        assert_eq!(
            builder(&app).summary(Section::Variables, app.theme.glyphs),
            "1  (artist)"
        );
    }

    #[test]
    fn the_structure_section_keeps_a_tree_and_enter_is_a_newline() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        for _ in 0..3 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "01_Assets");
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(builder(&app).open, Some(Open::Structure(_))),
            "Enter is a newline in a document, not a submit"
        );
        type_text(&mut app, "01_Assets/raw");
        press(&mut app, Key::ctrl('s'));
        assert!(builder(&app).open.is_none());
        assert_eq!(
            builder(&app).summary(Section::Structure, app.theme.glyphs),
            "2 folders"
        );
        assert_eq!(builder(&app).template.structure.len(), 1, "raw nests");
    }

    #[test]
    fn a_reserved_filename_is_refused_where_it_was_typed() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        for _ in 0..4 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        press(&mut app, Key::plain(KeyCode::Enter)); // → Files
        press(&mut app, Key::ch('a'));
        type_text(&mut app, "PROJECT_INFO.md");
        press(&mut app, Key::ctrl('s'));
        match &builder(&app).open {
            Some(Open::Files(list)) => {
                let edit = list.editing.as_ref().expect("still open");
                assert!(edit.error.as_deref().unwrap_or("").contains("reserved"));
                assert_eq!(edit.path.text(), "PROJECT_INFO.md", "the text stays");
            }
            other => panic!("{other:?}"),
        }
        assert!(builder(&app).template.files.is_empty());
    }

    /// Save handed the write to a worker and popped the builder in the same
    /// breath, so a refusal from under the data lock — an occupied slug, a
    /// lock held by another terminal, a full disk — arrived with nothing left
    /// to land on. The template and every answer in it were gone, and all that
    /// remained was one red line on the status bar.
    #[test]
    fn a_refused_save_keeps_the_builder_and_everything_in_it() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter)); // → Metadata
        // A slug the fixture's templates do not already answer to, so the
        // save that is refused here is refused by the worker and not by the
        // occupied-slug check.
        type_text(&mut app, "Reel edit");
        press(&mut app, Key::plain(KeyCode::Enter));
        for _ in 0..5 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        let effects = press(&mut app, Key::plain(KeyCode::Enter)); // Save
        let id = run_id(&effects);
        assert!(builder(&app).saving, "the builder says it is saving");

        let _ = update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Err("the data directory is locked by another fastf".to_string()),
            },
        );

        let builder = builder(&app);
        assert!(!builder.saving, "the save is over");
        assert_eq!(
            builder.template.name, "Reel edit",
            "the work is still there"
        );
        let error = builder.error.clone().expect("the refusal is on the list");
        assert!(error.contains("locked"), "it names the cause: {error}");
    }

    /// Esc and `q` are ignored while a save is in flight — it is about to
    /// land and its refusal needs the list to land on. **Ctrl-C is not**, and
    /// must not be: `DataLock::acquire` waits up to thirty seconds when
    /// another fastf holds it, so a save can sit there for half a minute, and
    /// routing the interrupt key into the same guard left no way out of it at
    /// all.
    #[test]
    fn a_save_in_flight_ignores_esc_but_never_traps_the_user() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "Reel edit");
        press(&mut app, Key::plain(KeyCode::Enter));
        for _ in 0..5 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        press(&mut app, Key::plain(KeyCode::Enter)); // Save
        assert!(builder(&app).saving);

        // Esc waits for the outcome rather than dropping the work.
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(
            matches!(app.modals.top(), Some(Modal::Builder(_))),
            "Esc during a save waits for it"
        );
        assert!(builder(&app).saving, "and does not cancel it");

        // Ctrl-C is the way out, as it is everywhere else in the app.
        press(&mut app, Key::ctrl('c'));
        assert!(
            app.modals.is_empty(),
            "Ctrl-C must not be swallowed while a save waits on the data lock"
        );
    }

    /// The other half: a save that lands closes the builder, once.
    #[test]
    fn a_save_that_lands_closes_the_builder() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "Demo");
        press(&mut app, Key::plain(KeyCode::Enter));
        for _ in 0..5 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        let id = run_id(&effects);
        let _ = update(&mut app, item_done(id, ListChange::SummaryOnly));
        assert!(app.modals.is_empty(), "the builder closed onto the tab");
    }

    /// Esc, `q` and Ctrl-C all popped the builder outright and said so
    /// afterwards on the status line, by which time every answer was gone.
    #[test]
    fn leaving_a_worked_on_template_asks_first() {
        for key in [Key::plain(KeyCode::Esc), Key::ch('q'), Key::ctrl('c')] {
            let mut app = fixture(6, 120, 40);
            open_new(&mut app);
            press(&mut app, Key::plain(KeyCode::Enter));
            type_text(&mut app, "Music video");
            press(&mut app, Key::plain(KeyCode::Enter));
            assert!(builder(&app).is_dirty());

            press(&mut app, key);
            match app.modals.top() {
                Some(Modal::Confirm(confirm)) => assert!(
                    confirm.prompt.contains("without saving"),
                    "{}",
                    confirm.prompt
                ),
                other => panic!("{key:?} must ask before discarding, got {other:?}"),
            }
            // No is no: the template is still there, with its answer.
            press(&mut app, Key::ch('n'));
            assert_eq!(builder(&app).template.name, "Music video");

            press(&mut app, key);
            press(&mut app, Key::ch('y'));
            assert!(app.modals.is_empty(), "yes leaves");
        }
    }

    /// A question nobody needs teaches people to answer it without reading, so
    /// a builder nothing was typed into closes on the first key.
    #[test]
    fn an_untouched_builder_closes_without_a_question() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        assert!(!builder(&app).is_dirty());
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.modals.is_empty(), "nothing was typed, nothing is asked");
    }

    /// Esc inside a section is still one rung of the ladder, not a discard.
    #[test]
    fn esc_inside_a_section_goes_back_to_the_list() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "Music video");
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(builder(&app).open.is_none(), "back on the section list");
        assert!(!app.modals.is_empty(), "and still in the builder");
    }

    /// `suggest_slug` rewrites any slug nobody has typed in, and a form built
    /// from an existing template has touched nothing — so correcting a typo in
    /// the *title* of `music-video` silently retyped the slug, and Save
    /// renamed the template's directory on disk to match.
    #[test]
    fn editing_a_template_does_not_let_the_name_rewrite_the_slug() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('e'));
        let template = Template {
            name: "Client project".to_string(),
            slug: "client-project".to_string(),
            naming_pattern: "{date}_{id}".to_string(),
            ..Template::default()
        };
        let _ = update(
            &mut app,
            Msg::TemplateSourceLoaded {
                slug: "client-project".to_string(),
                result: Ok(Box::new(template)),
            },
        );

        press(&mut app, Key::plain(KeyCode::Enter)); // → Metadata
        type_text(&mut app, " renamed");
        press(&mut app, Key::plain(KeyCode::Enter));

        assert_eq!(builder(&app).template.name, "Client project renamed");
        assert_eq!(
            builder(&app).template.slug,
            "client-project",
            "the slug is the directory on disk and does not follow the title"
        );
    }

    /// A new template typed onto an occupied slug overwrote the template that
    /// was there. The refusal is asked of the cards already in memory, so it
    /// lands on the list rather than after a worker round trip.
    #[test]
    fn a_new_template_may_not_take_a_slug_already_on_disk() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter)); // → Metadata
        type_text(&mut app, "General");
        press(&mut app, Key::plain(KeyCode::Enter));
        assert_eq!(builder(&app).template.slug, "general", "the slug follows");

        for _ in 0..5 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        let effects = press(&mut app, Key::plain(KeyCode::Enter)); // Save
        assert!(effects.is_empty(), "nothing was written: {effects:?}");
        let error = builder(&app).error.clone().expect("a refusal");
        assert!(
            error.contains("already exists"),
            "it names the collision: {error}"
        );
        assert!(!app.modals.is_empty(), "the work is still on screen");
    }

    /// `s` saves from anywhere on the section list — the section list is the
    /// one face of the builder with nothing to type into.
    #[test]
    fn s_saves_from_the_section_list() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "Demo");
        press(&mut app, Key::plain(KeyCode::Enter));

        // The cursor is still on Metadata, nowhere near the Save row.
        assert_eq!(builder(&app).row(), Row::Section(Section::Metadata));
        let effects = press(&mut app, Key::ch('s'));
        match action_of(&effects) {
            Action::SaveTemplate { template, .. } => assert_eq!(template.slug, "demo"),
            other => panic!("expected a save, got {other:?}"),
        }
    }

    /// Inside a section a letter is text, so `s` types rather than saving.
    #[test]
    fn s_inside_a_section_is_just_a_letter() {
        let mut app = fixture(6, 120, 40);
        open_new(&mut app);
        press(&mut app, Key::plain(KeyCode::Enter)); // → Metadata
        let effects = press(&mut app, Key::ch('s'));
        assert!(effects.is_empty(), "no save was started: {effects:?}");
        assert!(matches!(builder(&app).open, Some(Open::Metadata(_))));
    }

    #[test]
    fn deleting_a_template_asks_and_then_runs() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('D'));
        match app.modals.top() {
            Some(Modal::Confirm(confirm)) => assert!(
                confirm.prompt.contains("client-project"),
                "{}",
                confirm.prompt
            ),
            other => panic!("expected a confirm, got {other:?}"),
        }
        let effects = press(&mut app, Key::ch('n'));
        assert!(effects.is_empty(), "no is no: {effects:?}");

        press(&mut app, Key::ch('D'));
        let effects = press(&mut app, Key::ch('y'));
        assert!(
            matches!(action_of(&effects), Action::DeleteTemplate(slug) if slug == "client-project")
        );
    }

    #[test]
    fn from_folder_previews_before_it_writes() {
        use fastf::tui::app::wizard::{FIELD_SLUG, FIELD_SOURCE, FlowKind};

        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('I'));
        match app.modals.top() {
            Some(Modal::Flow(flow)) => assert_eq!(flow.kind, FlowKind::FromFolder),
            other => panic!("expected the from-folder flow, got {other:?}"),
        }
        type_text(&mut app, "/mnt/projects/Source");
        press(&mut app, Key::plain(KeyCode::Tab));
        type_text(&mut app, "from-a-folder");
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        match &effects[..] {
            [Effect::Preview(request)] => match &**request {
                Request::FromFolder(from) => {
                    assert_eq!(from.slug, "from-a-folder");
                    assert!(!from.bundle_assets, "assets are opt-in");
                }
                other => panic!("{other:?}"),
            },
            other => panic!("expected a preview, got {other:?}"),
        }
        // Its refusal lands on the field that caused it, like every other flow.
        let _ = update(
            &mut app,
            Msg::PreviewFailed {
                field: Some(FIELD_SOURCE.to_string()),
                error: "no such folder: /mnt/projects/Source".to_string(),
            },
        );
        match app.modals.top() {
            Some(Modal::Flow(flow)) => {
                assert_eq!(flow.form.focused().unwrap().key, FIELD_SOURCE);
                assert_eq!(flow.form.value(FIELD_SLUG), "from-a-folder");
            }
            other => panic!("{other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Settings, the counter, maintenance and the first run
// ---------------------------------------------------------------------------

mod settings {
    use super::*;
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
}

// ---------------------------------------------------------------------------
// The mouse
// ---------------------------------------------------------------------------

mod mouse {
    use super::*;
    use fastf::tui::app::Focus;
    use fastf::tui::msg::{Mouse, MouseKind};

    fn click(app: &mut App, column: u16, row: u16) -> Vec<Effect> {
        update(
            app,
            Msg::Mouse(Mouse {
                kind: MouseKind::Click,
                column,
                row,
            }),
        )
    }

    fn wheel(app: &mut App, down: bool) -> Vec<Effect> {
        update(
            app,
            Msg::Mouse(Mouse {
                kind: if down {
                    MouseKind::ScrollDown
                } else {
                    MouseKind::ScrollUp
                },
                column: 1,
                row: 8,
            }),
        )
    }

    #[test]
    fn a_click_in_the_table_selects_the_row_under_it() {
        let mut app = fixture(12, 120, 40);
        let table = app.regions().table;
        // Two rows in: the border and the header sit above the first project.
        click(&mut app, table.x + 4, table.y + 2 + 3);
        assert_eq!(app.library.selected, Some(3));
        assert_eq!(app.focus, Focus::Projects);
    }

    #[test]
    fn a_click_past_the_last_row_changes_nothing() {
        let mut app = fixture(3, 120, 40);
        let table = app.regions().table;
        click(&mut app, table.x + 4, table.y + 2 + 9);
        assert_eq!(
            app.library.selected,
            Some(0),
            "nothing to select down there"
        );
    }

    #[test]
    fn a_click_moves_focus_to_the_pane_it_landed_in() {
        let mut app = fixture(12, 120, 40);
        let regions = app.regions();
        let detail = regions.detail.expect("120 columns has the pane");
        click(&mut app, detail.x + 2, detail.y + 2);
        assert_eq!(app.focus, Focus::Detail);

        click(&mut app, regions.search.x + 2, regions.search.y);
        assert!(app.search.editing, "the bar is where you type");
    }

    /// The wheel is `↑`/`↓`, three at a time, wherever the keys already go — so
    /// it needs no geometry and cannot drift from the layout.
    #[test]
    fn the_wheel_moves_whatever_the_arrows_would() {
        let mut app = fixture(12, 120, 40);
        wheel(&mut app, true);
        assert_eq!(app.library.selected, Some(3));
        wheel(&mut app, false);
        assert_eq!(app.library.selected, Some(0));

        // In the detail pane it moves the pane's cursor, because that is what
        // ↓ does there — over the rows Enter can act on, and no further than
        // the last of them.
        let path = app.library.selected().unwrap().path.clone();
        app.details
            .insert(path, fastf::tui::app::data::ProjectDetail::default());
        press(&mut app, Key::plain(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Detail);
        let rows = app.pane_rows();
        wheel(&mut app, true);
        assert!(app.pane_cursor > 0, "the wheel moved the pane's cursor");
        assert!(rows[app.pane_cursor].selectable());
        assert_eq!(app.library.selected, Some(0), "the list did not move");
        press(&mut app, Key::plain(KeyCode::End));
        let at_end = app.pane_cursor;
        assert_eq!(
            rows[at_end],
            fastf::tui::app::pane::PaneRow::AddTodo,
            "End is the last row Enter can act on"
        );
        wheel(&mut app, true);
        assert_eq!(app.pane_cursor, at_end, "nothing past the end");
    }

    #[test]
    fn a_click_in_the_palette_runs_the_entry_under_it() {
        let mut app = fixture(12, 120, 40);
        press(&mut app, Key::ch('c'));
        type_text(&mut app, "open");
        let box_area = fastf::tui::layout::centered(app.area(), 70, 70);
        // The border, the query line, then a blank one: the first entry is
        // three rows down.
        let effects = click(&mut app, box_area.x + 4, box_area.y + 3);
        assert!(
            matches!(&effects[..], [Effect::Spawn(SpawnKind::Reveal(_))]),
            "the first `open` entry is Open project folder: {effects:?}"
        );
        assert!(app.modals.is_empty(), "and the palette closed");
    }

    #[test]
    fn a_click_outside_anything_is_ignored() {
        let mut app = fixture(12, 120, 40);
        let before = app.library.selected;
        // The header, which nothing answers for.
        let effects = click(&mut app, 2, 0);
        assert!(effects.is_empty(), "{effects:?}");
        assert_eq!(app.library.selected, before);
    }
}

// --- the session: what a run leaves for the next one ------------------------

#[test]
fn a_remembered_sort_pane_and_row_come_back_on_the_first_discovery() {
    use fastf::tui::app::library::{Order, Sort};
    use fastf::tui::session::Session;

    let mut app = empty_fixture(120, 40);
    app.apply_session(&Session {
        sort: Some("name".to_string()),
        detail_open: Some(false),
        selected: Some("ID0245".to_string()),
        ..Session::default()
    });
    assert!(!app.detail_open, "the pane comes back the way it was left");
    let _ = app.start();
    let _ = update(
        &mut app,
        Msg::Discovered {
            generation: 1,
            projects: sample_projects(6),
        },
    );
    assert_eq!(
        app.library.effective_sort(&app.search.query),
        Sort::new(Order::Name),
        "the sort order survives the restart"
    );
    assert_eq!(
        app.library.selected().map(|p| p.id.as_str()),
        Some("ID0245"),
        "the cursor is back on the row it was on"
    );
    let captured = Session::capture(&app, &Session::default());
    assert_eq!(captured.sort.as_deref(), Some("name"));
    assert_eq!(captured.selected.as_deref(), Some("ID0245"));
    assert_eq!(captured.detail_open, Some(false));

    // A reload is not a restart: the cursor stays wherever it has since gone.
    let _ = press(&mut app, Key::plain(KeyCode::Down));
    let moved = app.library.selected().map(|p| p.id.clone());
    let _ = press(&mut app, Key::plain(KeyCode::F(5)));
    let _ = update(
        &mut app,
        Msg::Discovered {
            generation: 2,
            projects: sample_projects(6),
        },
    );
    assert_eq!(app.library.selected().map(|p| p.id.clone()), moved);
}

#[test]
fn recent_and_search_keep_their_own_order_and_rows() {
    use fastf::tui::session::Session;

    let mut app = App::new(
        Entry::Recent {
            preset: Preset::default(),
            initial: sample_projects(4),
        },
        Theme::mono(),
        (120, 40),
    );
    let remembered = Session {
        sort: Some("name".to_string()),
        detail_open: Some(false),
        selected: Some("ID0246".to_string()),
        ..Session::default()
    };
    app.apply_session(&remembered);
    assert!(!app.detail_open, "the pane's state is everyone's");
    assert_eq!(app.library.explicit_sort, None, "recent keeps newest first");
    assert_eq!(
        app.library.selected().map(|p| p.id.as_str()),
        Some("ID0248"),
        "recent starts on its own first row"
    );
    // And it leaves the remembered sort and row for `fastf` untouched.
    let captured = Session::capture(&app, &remembered);
    assert_eq!(captured.sort, remembered.sort);
    assert_eq!(captured.selected, remembered.selected);
}

// --- one registry: every dialog's keys are declared, and help is everywhere ----

#[test]
fn a_verb_key_in_the_action_menu_runs_it_and_closes_the_menu() {
    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('a'));
    assert!(matches!(app.modals.top(), Some(Modal::Actions(_))));
    let effects = press(&mut app, Key::ch('o'));
    assert!(
        matches!(effects.first(), Some(Effect::Spawn(SpawnKind::Reveal(_)))),
        "o in the menu reveals the folder: {effects:?}"
    );
    assert!(app.modals.is_empty(), "the menu closes once its verb ran");

    let _ = press(&mut app, Key::ch('a'));
    let _ = press(&mut app, Key::ch('D'));
    assert!(
        matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
        "D in the menu opens the delete confirmation"
    );
}

#[test]
fn help_opens_over_any_dialog_for_that_dialogs_context() {
    use fastf::tui::command::Context;

    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('a'));
    let _ = press(&mut app, Key::ch('?'));
    assert!(
        matches!(
            app.modals.top(),
            Some(Modal::Help {
                ctx: Context::Actions,
                ..
            })
        ),
        "? over the action menu is the action menu's help"
    );
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        matches!(app.modals.top(), Some(Modal::Actions(_))),
        "closing the help lands back on the menu"
    );

    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    let _ = press(&mut app, Key::ch(','));
    let _ = update(&mut app, Msg::SettingsLoaded(Box::default()));
    assert!(matches!(app.modals.top(), Some(Modal::Settings(_))));
    let _ = press(&mut app, Key::plain(KeyCode::F(1)));
    assert!(matches!(
        app.modals.top(),
        Some(Modal::Help {
            ctx: Context::Settings,
            ..
        })
    ));
    let _ = press(&mut app, Key::ch('q'));
    let effects = press(&mut app, Key::ch('q'));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Quit(_))),
        "q closes the settings, it does not quit: {effects:?}"
    );
    assert!(app.modals.is_empty());
}

#[test]
fn the_templates_tab_and_the_builder_answer_their_declared_keys() {
    use fastf::tui::app::Screen;

    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('T'));
    assert_eq!(app.screen, Screen::Templates);
    let _ = press(&mut app, Key::ch('D'));
    assert!(
        matches!(app.modals.top(), Some(Modal::Confirm(_))),
        "D asks before deleting a template"
    );
    let _ = press(&mut app, Key::ch('x'));
    assert!(
        matches!(app.modals.top(), Some(Modal::Confirm(_))),
        "a confirmation takes only y, n, Esc and the global keys"
    );
    let _ = press(&mut app, Key::ch('n'));
    assert!(
        app.modals.is_empty(),
        "n answers no and closes the confirmation"
    );
    let _ = press(&mut app, Key::ch('n'));
    assert!(
        matches!(app.modals.top(), Some(Modal::Builder(_))),
        "n opens the builder on a new template"
    );
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        app.modals.is_empty(),
        "Esc on an untouched section list returns to the tab without asking"
    );
    assert_eq!(app.screen, Screen::Templates);
    assert!(app.status.text.contains("Closed"));
    // Esc again is one more level out: the tab you came from.
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Library);
}

#[test]
fn a_pager_stops_at_its_last_line() {
    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('?'));
    let _ = press(&mut app, Key::plain(KeyCode::End));
    let Some(Modal::Help { scroll, .. }) = app.modals.top() else {
        panic!("expected the help");
    };
    let at_end = *scroll;
    assert!(at_end > 0, "End scrolls a long help");
    let _ = press(&mut app, Key::plain(KeyCode::PageDown));
    let Some(Modal::Help { scroll, .. }) = app.modals.top() else {
        panic!("expected the help");
    };
    assert_eq!(*scroll, at_end, "nothing past the last line");
    let _ = press(&mut app, Key::plain(KeyCode::Home));
    let Some(Modal::Help { scroll, .. }) = app.modals.top() else {
        panic!("expected the help");
    };
    assert_eq!(*scroll, 0);
}

// --- the defects the consolidation pass found ---------------------------------

#[test]
fn a_recent_preset_is_a_filter_esc_takes_off() {
    let mut app = App::new(
        Entry::Recent {
            preset: Preset {
                template: Some("general".to_string()),
                ..Default::default()
            },
            initial: sample_projects(6),
        },
        Theme::mono(),
        (120, 40),
    );
    assert_eq!(app.library.len(), 2, "the preset narrows the rows");
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.library.preset.is_none());
    assert_eq!(app.library.len(), 6, "Esc shows every project again");
    let effects = press(&mut app, Key::plain(KeyCode::Esc));
    assert!(
        matches!(effects.first(), Some(Effect::Quit(Exit::Normal))),
        "with nothing left to clear, Esc quits: {effects:?}"
    );
}

#[test]
fn a_dialog_that_reads_from_a_worker_goes_up_at_once() {
    let mut app = fixture(3, 120, 40);
    let effects = press(&mut app, Key::ch('M'));
    assert!(matches!(effects.first(), Some(Effect::LoadView { .. })));
    let Some(Modal::Message { title, lines, .. }) = app.modals.top() else {
        panic!("the metadata dialog goes up before the read lands");
    };
    assert!(title.ends_with("metadata"));
    assert_eq!(lines, &vec!["reading…".to_string()]);
    let title = title.clone();
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title: title.clone(),
            lines: vec!["id             ID0248".to_string()],
        },
    );
    let Some(Modal::Message { lines, .. }) = app.modals.top() else {
        panic!("still the same dialog");
    };
    assert_eq!(lines[0], "id             ID0248");
    let _ = press(&mut app, Key::plain(KeyCode::Esc));
    let _ = update(
        &mut app,
        Msg::ViewLoaded {
            title,
            lines: vec!["late".to_string()],
        },
    );
    assert!(
        app.modals.is_empty(),
        "a read that lands after Esc is dropped"
    );

    let effects = press(&mut app, Key::ch(','));
    assert!(matches!(effects.first(), Some(Effect::LoadSettings)));
    assert!(
        matches!(app.modals.top(), Some(Modal::Settings(state)) if state.rows.is_empty() && state.pending),
        "the settings screen goes up empty and pending"
    );
}

#[test]
fn a_malformed_query_is_named_while_it_is_typed() {
    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('/'));
    let _ = type_text(&mut app, "created>notadate");
    assert!(
        app.status.text.contains("needs a date like 2026-01-01"),
        "{:?}",
        app.status
    );
    let _ = press(&mut app, Key::ctrl('u'));
    assert!(
        app.status.text.is_empty(),
        "a good query clears the warning"
    );
}

#[test]
fn move_with_one_base_says_why() {
    let mut app = fixture(3, 120, 40);
    let effects = press(&mut app, Key::ch('m'));
    assert!(effects.is_empty());
    assert!(
        app.status.text.contains("base"),
        "m explains itself: {:?}",
        app.status
    );
    assert!(app.modals.is_empty());
}

#[test]
fn the_too_small_guard_keeps_the_quit_gestures_honest() {
    let mut app = fixture(3, 40, 10);
    let effects = press(&mut app, Key::ch('?'));
    assert!(effects.is_empty() && app.modals.is_empty());
    let effects = press(&mut app, Key::ctrl('c'));
    assert!(
        matches!(effects.first(), Some(Effect::Quit(Exit::Interrupted))),
        "Ctrl-C is still an interrupt: {effects:?}"
    );
    let effects = press(&mut app, Key::ch('q'));
    assert!(matches!(effects.first(), Some(Effect::Quit(Exit::Normal))));
}

// --- every verb over the marks ------------------------------------------------

#[test]
fn a_tag_over_marks_is_asked_once_and_runs_as_a_job_in_display_order() {
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    press(&mut app, Key::ch(' ')); // mark row 2
    let rows: Vec<PathBuf> = (0..3)
        .map(|row| app.library.row(row).unwrap().path.clone())
        .collect();

    press(&mut app, Key::ch('A'));
    let Some(Modal::Pick(pick)) = app.modals.top() else {
        panic!("A opens the tag picker");
    };
    assert!(pick.title.contains("3 projects"), "{}", pick.title);
    // Type a new tag rather than pick one.
    let new_tag = pick
        .items
        .iter()
        .position(|item| item.value == fastf::tui::app::actions::NEW_TAG)
        .expect("the picker offers a new tag");
    for _ in 0..new_tag {
        press(&mut app, Key::plain(KeyCode::Down));
    }
    press(&mut app, Key::plain(KeyCode::Enter));
    type_text(&mut app, "reviewed");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));

    let job = app.job.as_ref().expect("a tag job is running");
    assert!(matches!(&job.kind, fastf::tui::app::jobs::JobKind::AddTag(tag) if tag == "reviewed"));
    assert_eq!(job.pending.len(), 2, "one in flight, two to go");
    let id1 = run_id(&effects);
    assert!(
        matches!(action_of(&effects), Action::AddTag { project, tag } if project.path == rows[0] && tag == "reviewed")
    );

    // The first item lands patched; the next starts; the mark stays until the
    // row changes say it is done.
    let mut patched = app.library.row(0).unwrap().clone();
    patched.tags.push("reviewed".to_string());
    let effects = update(
        &mut app,
        item_done(
            id1,
            ListChange::Patched {
                project: Box::new(patched),
                was: rows[0].clone(),
                stale: vec![rows[0].clone()],
            },
        ),
    );
    assert!(
        matches!(action_of(&effects), Action::AddTag { project, .. } if project.path == rows[1])
    );
    assert!(
        app.library
            .row(0)
            .unwrap()
            .tags
            .contains(&"reviewed".to_string()),
        "the row shows the tag as soon as its item lands"
    );
}

/// **The batch's effects are the app's too.** They were dropped, so a
/// `Reload` — which `discover` arms by setting `inflight` *before* returning
/// the effect that answers it — left the app waiting on a generation nothing
/// would send, and every later patch only set `dirty`. A batch re-derive of
/// tags rewrote every file, showed nothing, and froze the list for the rest of
/// the session.
#[test]
fn a_batch_item_returns_the_effects_its_change_asked_for() {
    use fastf::tui::command::CommandId;

    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    let effects = app.run(CommandId::ReautoTags);
    let id1 = run_id(&effects);

    let effects = update(&mut app, item_done(id1, ListChange::Reload));
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Discover { .. })),
        "a reload must reach the runtime: {effects:?}"
    );
    assert!(
        effects.iter().any(|e| matches!(e, Effect::LoadSummary)),
        "and so must the summary it asked for: {effects:?}"
    );

    // The discovery it armed is the one in flight, so its answer installs.
    let generation = app.library.inflight.expect("a discovery is in flight");
    update(
        &mut app,
        Msg::Discovered {
            generation,
            projects: sample_projects(12),
        },
    );
    assert!(
        app.library.inflight.is_none(),
        "the list is not left waiting on a generation nothing will send"
    );
}

/// Marks are kept by path and survive a filter change; `targets()` intersects
/// them with the rows on screen. When the two disagreed, `batching()` said yes
/// and every batch verb hit an early return with no picker, no dialog and no
/// message — which is what "batch tagging does nothing" was.
#[test]
fn a_verb_aimed_at_marks_a_filter_hides_says_so() {
    use fastf::tui::command::CommandId;

    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    assert_eq!(app.library.marks.len(), 2);

    // A query that keeps nothing the marks are on.
    press(&mut app, Key::ch('/'));
    type_text(&mut app, "zzzznothing");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(app.library.is_empty(), "the filter hides every marked row");

    let effects = app.run(CommandId::AddTag);
    assert!(app.modals.is_empty(), "no picker over nothing");
    assert!(effects.is_empty());
    assert!(
        app.status.text.contains("every marked row is hidden"),
        "the refusal has to be said out loud: {:?}",
        app.status.text
    );
    assert_eq!(app.library.marks.len(), 2, "the marks are not touched");
}

#[test]
fn a_quick_note_takes_several_lines_and_goes_to_every_mark() {
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ctrl('n'));
    assert!(matches!(app.modals.top(), Some(Modal::Note(note)) if note.count == 1));
    type_text(&mut app, "first cut");
    let mut alt_enter = Key::plain(KeyCode::Enter);
    alt_enter.alt = true;
    press(&mut app, alt_enter);
    type_text(&mut app, "due Friday");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(action_of(&effects), Action::AppendNote { text, .. } if text == "first cut\ndue Friday"),
        "Enter saves, Alt-Enter broke the line: {effects:?}"
    );
    assert!(app.modals.is_empty());

    // Over marks the note is asked once and becomes a job.
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ctrl('n'));
    assert!(matches!(app.modals.top(), Some(Modal::Note(note)) if note.count == 2));
    let _ = update(&mut app, Msg::Paste("line one\nline two\n".to_string()));
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        matches!(&app.job.as_ref().unwrap().kind, fastf::tui::app::jobs::JobKind::Note(text) if text == "line one\nline two"),
        "a pasted paragraph is one note"
    );
    assert!(matches!(action_of(&effects), Action::AppendNote { .. }));
}

#[test]
fn a_paste_never_becomes_keystrokes() {
    let mut app = fixture(12, 120, 40);
    let before = app.library.selected().unwrap().path.clone();
    // Nothing takes typing: the paste is ignored and said so.
    let effects = update(&mut app, Msg::Paste("D\ndelete\n".to_string()));
    assert!(effects.is_empty() && app.modals.is_empty());
    assert_eq!(app.library.selected().unwrap().path, before);
    assert!(
        app.status.text.contains("pasted text ignored"),
        "{}",
        app.status.text
    );

    // A single-line field keeps the first line and says so.
    press(&mut app, Key::ch('r'));
    press(&mut app, Key::ctrl('u')); // the prompt opens on the old name
    let _ = update(&mut app, Msg::Paste("New_Name\nq\nD".to_string()));
    let Some(Modal::TextPrompt(prompt)) = app.modals.top() else {
        panic!("the rename prompt is still up");
    };
    assert_eq!(prompt.input.text(), "New_Name");
    assert!(
        app.status.text.contains("kept the first"),
        "{}",
        app.status.text
    );
}

#[test]
fn rename_does_not_batch_and_says_so() {
    let mut app = fixture(12, 120, 40);
    press(&mut app, Key::ch(' '));
    let effects = press(&mut app, Key::ch('r'));
    assert!(effects.is_empty() && app.modals.is_empty());
    assert!(
        app.status.text.contains("one folder at a time"),
        "{}",
        app.status.text
    );
    press(&mut app, Key::ch('-'));
    press(&mut app, Key::ch('r'));
    assert!(matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
}

// --- the message log ----------------------------------------------------------

#[test]
fn every_status_line_is_logged_and_l_reads_them_back_newest_first() {
    use fastf::tui::app::{LOG_CAP, StatusLevel};

    let mut app = fixture(3, 120, 40);
    for i in 0..(LOG_CAP + 50) {
        let _ = update(
            &mut app,
            Msg::Diag(fastf::util::diag::Level::Note, format!("note {i}")),
        );
    }
    assert_eq!(app.log.len(), LOG_CAP, "the log is bounded");
    assert!(
        app.log.front().unwrap().text.ends_with("note 50"),
        "the oldest lines go first: {:?}",
        app.log.front()
    );
    assert_eq!(app.unseen_warnings, 0, "a note on the dashboard was seen");

    // A warning that lands under a dialog is counted until the log is read.
    let _ = press(&mut app, Key::ch('a'));
    let _ = update(
        &mut app,
        Msg::Diag(fastf::util::diag::Level::Warn, "disk is full".to_string()),
    );
    assert_eq!(app.unseen_warnings, 1);
    assert!(matches!(app.log.back(), Some(entry) if entry.level == StatusLevel::Warn));
    let _ = press(&mut app, Key::plain(KeyCode::Esc));

    let _ = press(&mut app, Key::ch('L'));
    let Some(Modal::Message { title, lines, .. }) = app.modals.top() else {
        panic!("L opens the messages");
    };
    assert_eq!(title, "messages");
    assert!(
        lines[0].starts_with("10:00:00") && lines[0].contains("disk is full"),
        "newest first, stamped: {:?}",
        lines.first()
    );
    assert_eq!(app.unseen_warnings, 0, "reading the log clears the count");
}

#[test]
fn register_can_take_a_typed_date_like_the_command_line() {
    use fastf::tui::app::register::{CREATED_TYPED, FIELD_CREATED, FIELD_CREATED_DATE};

    let mut app = fixture(3, 120, 40);
    let _ = press(&mut app, Key::ch('e'));
    let Some(Modal::Flow(flow)) = app.modals.top() else {
        panic!("e opens the register form");
    };
    assert!(
        flow.form.field(FIELD_CREATED_DATE).unwrap().hidden,
        "the date field waits for its choice"
    );
    // Walk to the Created choice and pick "a date I type".
    while app.modals.top().is_some_and(|m| matches!(m, Modal::Flow(flow) if flow.form.focused().map(|f| f.key.as_str()) != Some(FIELD_CREATED))) {
        let _ = press(&mut app, Key::plain(KeyCode::Tab));
    }
    let _ = press(&mut app, Key::plain(KeyCode::Right));
    let _ = press(&mut app, Key::plain(KeyCode::Right));
    let Some(Modal::Flow(flow)) = app.modals.top() else {
        panic!("still the form");
    };
    assert_eq!(flow.form.value(FIELD_CREATED), CREATED_TYPED);
    assert!(!flow.form.field(FIELD_CREATED_DATE).unwrap().hidden);
    let _ = press(&mut app, Key::plain(KeyCode::Tab));
    type_text(&mut app, "2024-05-06");
    let Some(Modal::Flow(flow)) = app.modals.top() else {
        panic!("still the form");
    };
    let request = fastf::tui::app::register::request(flow);
    assert_eq!(request.created_override.as_deref(), Some("2024-05-06"));
    assert!(!request.use_today);
}

// --- the terminal in every situation ----------------------------------------

#[test]
fn a_window_verb_says_no_display_where_there_is_none() {
    let mut app = fixture(3, 120, 40);
    app.has_display = false;
    let effects = press(&mut app, Key::ch('o'));
    assert!(effects.is_empty(), "nothing is spawned: {effects:?}");
    assert!(
        app.status.text.contains("no display"),
        "o says why: {}",
        app.status.text
    );
    let effects = press(&mut app, Key::ch('t'));
    assert!(effects.is_empty());
    // Copying the path still works — it is what the reason points at.
    let effects = press(&mut app, Key::ch('y'));
    assert!(matches!(
        effects.first(),
        Some(Effect::Spawn(SpawnKind::Clipboard(_)))
    ));
}

#[cfg(unix)]
#[test]
fn ctrl_z_suspends_to_the_shell_and_the_size_is_reread_on_return() {
    let mut app = fixture(3, 120, 40);
    let effects = press(&mut app, Key::ctrl('z'));
    assert_eq!(effects, vec![Effect::Suspend(Suspended::Shell)]);
    let _ = update(&mut app, Msg::Resumed(fastf::tui::msg::Resumed::Shell));
    let _ = update(&mut app, Msg::Resize(100, 30));
    assert_eq!(app.size, (100, 30));
}

// ---------------------------------------------------------------------------
// The template guide, and the panel that explains the editor
// ---------------------------------------------------------------------------

mod guide {
    use super::*;
    use fastf::tui::app::studio::Section;
    use fastf::tui::guide;
    use fastf::tui::testing::guide_fixture;

    fn page(app: &App) -> Option<usize> {
        match app.modals.top() {
            Some(Modal::Guide(state)) => Some(state.page),
            _ => None,
        }
    }

    /// The offer, the first time templates come up at all — and it goes **on
    /// top of** what asked for it, so Esc leaves you where you were going.
    #[test]
    fn the_guide_offers_itself_once_and_then_never_again() {
        let mut app = guide_fixture(3, 120, 40);
        let _ = press(&mut app, Key::ch('T'));
        assert_eq!(page(&app), Some(0), "it opens at the beginning");
        let _ = press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.modals.is_empty(), "Esc leaves the tab underneath");
        assert_eq!(app.screen, fastf::tui::app::Screen::Templates);

        // Back to the library and in again: it has been read.
        let _ = press(&mut app, Key::ch('T'));
        let _ = press(&mut app, Key::ch('T'));
        assert!(app.modals.is_empty(), "asked once, not once per visit");
    }

    /// **One flag for both doors.** Two would show it twice in one afternoon
    /// to the person who looked at the tab and then pressed new — which is
    /// exactly the reader it is trying not to annoy.
    #[test]
    fn the_tab_and_the_editor_share_one_offer() {
        let mut app = guide_fixture(3, 120, 40);
        let _ = press(&mut app, Key::ch('T'));
        assert!(matches!(app.modals.top(), Some(Modal::Guide(_))));
        let _ = press(&mut app, Key::plain(KeyCode::Esc));

        let _ = press(&mut app, Key::ch('n'));
        assert!(
            matches!(app.modals.top(), Some(Modal::Builder(_))),
            "the editor opens with no second welcome"
        );
    }

    /// Opened from the editor it lands on the page for the row under the
    /// cursor. A guide that always opens at page one is a guide nobody opens
    /// twice.
    #[test]
    fn the_guide_opens_on_the_page_for_what_is_highlighted() {
        let mut app = fixture(3, 120, 40);
        let _ = press(&mut app, Key::ch('T'));
        let _ = press(&mut app, Key::ch('n'));
        let _ = press(&mut app, Key::plain(KeyCode::Down)); // → ID
        let _ = press(&mut app, Key::plain(KeyCode::Down)); // → Variables
        let _ = press(&mut app, Key::ch('H'));
        assert_eq!(page(&app), Some(guide::page_for(Section::Variables)));
        let _ = press(&mut app, Key::plain(KeyCode::Esc));
        assert!(
            matches!(app.modals.top(), Some(Modal::Builder(_))),
            "and Esc goes back to the template, not out of it"
        );
    }

    /// A document has a beginning and an end: turning past either stays put
    /// rather than wrapping, which reads as having lost your place. Forward
    /// off the end is the one direction that leaves.
    #[test]
    fn the_pages_stop_at_both_ends_and_enter_walks_out_of_the_last() {
        let mut app = fixture(3, 120, 40);
        let _ = press(&mut app, Key::ch('T'));
        let _ = press(&mut app, Key::ch('H'));
        let _ = press(&mut app, Key::plain(KeyCode::Left));
        assert_eq!(
            page(&app),
            Some(0),
            "the first page does not wrap to the last"
        );

        for _ in 0..guide::PAGES.len() - 1 {
            let _ = press(&mut app, Key::plain(KeyCode::Right));
        }
        assert_eq!(page(&app), Some(guide::PAGES.len() - 1));
        // Forward off the last page lets the reader go, whichever key they
        // have been pressing — `→` is Enter's twin here as everywhere else,
        // and the alternative is pressing a key against the end of a document.
        let _ = press(&mut app, Key::plain(KeyCode::Right));
        assert!(
            app.modals.is_empty(),
            "a reader who keeps pressing the same key is let go at the end"
        );
    }

    /// Enter is the other half of the same command, and leaves the same way.
    #[test]
    fn enter_walks_forward_and_out_of_the_last_page() {
        let mut app = fixture(3, 120, 40);
        let _ = press(&mut app, Key::ch('T'));
        let _ = press(&mut app, Key::ch('H'));
        for _ in 0..guide::PAGES.len() - 1 {
            let _ = press(&mut app, Key::plain(KeyCode::Enter));
        }
        assert_eq!(page(&app), Some(guide::PAGES.len() - 1));
        let _ = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(app.modals.is_empty());
    }

    /// Scrolling stops where the page does, at the width the view draws it —
    /// the ceiling `update` and `view` have to agree about.
    #[test]
    fn the_scroll_stops_at_the_end_of_the_page() {
        let mut app = fixture(3, 120, 40);
        let _ = press(&mut app, Key::ch('T'));
        let _ = press(&mut app, Key::ch('H'));
        for _ in 0..200 {
            let _ = press(&mut app, Key::plain(KeyCode::Down));
        }
        let scrolled = match app.modals.top() {
            Some(Modal::Guide(state)) => state.scroll,
            _ => panic!("the guide should still be open"),
        };
        let room = fastf::tui::layout::guide_box(app.area())
            .height
            .saturating_sub(4) as usize;
        let rows = guide::note_rows(
            &guide::page_notes(0),
            fastf::tui::layout::guide_box(app.area())
                .width
                .saturating_sub(4) as usize,
        );
        assert_eq!(scrolled, rows.saturating_sub(room));
        // And back up lands exactly at the top rather than under it.
        for _ in 0..200 {
            let _ = press(&mut app, Key::plain(KeyCode::Up));
        }
        assert!(matches!(app.modals.top(), Some(Modal::Guide(s)) if s.scroll == 0));
    }

    /// The panel is on until it is turned off, and the choice is remembered.
    #[test]
    fn the_explanation_panel_toggles_and_is_remembered() {
        use fastf::tui::session::Session;

        let mut app = fixture(3, 120, 40);
        assert!(app.explain_open, "on by default");
        let _ = press(&mut app, Key::ch('T'));
        let _ = press(&mut app, Key::ch('n'));
        let _ = press(&mut app, Key::ch('i'));
        assert!(!app.explain_open);
        assert!(
            matches!(app.modals.top(), Some(Modal::Builder(_))),
            "i is not a way out"
        );

        let kept = Session::capture(&app, &Session::default());
        assert_eq!(kept.explain_open, Some(false));
        assert_eq!(kept.guide_seen, Some(true), "the fixture has read it");

        let mut next = fixture(3, 120, 40);
        next.apply_session(&kept);
        assert!(!next.explain_open, "the next run opens the way it was left");
    }

    /// The coach is **advice and never a refusal**: it names what is missing
    /// and the template still saves.
    #[test]
    fn the_coach_names_what_is_missing_without_refusing_anything() {
        let mut app = fixture(3, 120, 40);
        let _ = press(&mut app, Key::ch('T'));
        let _ = press(&mut app, Key::ch('n'));
        let Some(Modal::Builder(builder)) = app.modals.top() else {
            panic!("the editor should be open");
        };
        let gaps = guide::gaps(&builder.template);
        assert!(!gaps.is_empty(), "a blank template has plenty to say");
        assert_eq!(guide::next_step(&builder.template).as_ref(), gaps.first());
    }
}

/// The movement grammar and the horizontal axis, which arrived together at
/// v3.5.0: one set of movement keys in every list, and `→`/`←` as Enter's and
/// Esc's twins wherever there is something to go into.
mod movement {
    use super::*;
    use fastf::tui::app::Screen;

    /// Half a page is half of what a page key moves, and it stops at the ends
    /// rather than wrapping — the same bargain the page keys make.
    #[test]
    fn ctrl_d_and_ctrl_u_move_half_a_page_and_clamp() {
        let mut app = fixture(40, 80, 24);
        let page = app.rows_on_screen();
        assert!(page >= 4, "the fixture needs a page worth of rows");
        press(&mut app, Key::ctrl('d'));
        assert_eq!(app.library.selected, Some(page / 2));
        press(&mut app, Key::ctrl('u'));
        assert_eq!(app.library.selected, Some(0));
        press(&mut app, Key::ctrl('u'));
        assert_eq!(
            app.library.selected,
            Some(0),
            "a half page clamps at the top"
        );
        for _ in 0..40 {
            press(&mut app, Key::ctrl('d'));
        }
        assert_eq!(app.library.selected, Some(app.library.len() - 1));
    }

    /// The action menu is a list of eighteen verbs that cannot be searched.
    /// Before this it was the one list that could only be walked a row at a
    /// time: neither the page keys nor the jumps reached it.
    #[test]
    fn the_action_menu_pages_and_jumps() {
        let mut app = fixture(6, 80, 24);
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(app.modals.top(), Some(Modal::Actions(_))),
            "the action menu should be open"
        );
        let rows = fastf::tui::app::actions::action_entries(&app).len();
        assert!(rows > 3, "the menu has more rows than a step");
        let at = |app: &App| match app.modals.top() {
            Some(Modal::Actions(state)) => state.selected,
            _ => panic!("the action menu closed"),
        };
        press(&mut app, Key::ch('G'));
        assert_eq!(at(&app), rows - 1, "G is the last row here too");
        press(&mut app, Key::ch('g'));
        assert_eq!(at(&app), 0);
        press(&mut app, Key::plain(KeyCode::PageDown));
        assert!(at(&app) > 0, "PgDn moves in the action menu");
        press(&mut app, Key::plain(KeyCode::Home));
        assert_eq!(at(&app), 0);
    }

    /// `g` and `G` used to mean "template from a folder" and "the guide" on the
    /// templates tab, so the one list of arbitrary length had no jump keys at
    /// all. The two verbs moved to `I` and `H`.
    #[test]
    fn the_templates_tab_jumps_to_its_ends() {
        let mut app = fixture(3, 100, 30);
        press(&mut app, Key::ch('T'));
        assert_eq!(app.screen, Screen::Templates);
        press(&mut app, Key::ch('G'));
        let last = app.studio.selected;
        press(&mut app, Key::ch('g'));
        assert_eq!(app.studio.selected, 0);
        assert!(last > 0, "the fixture has more than one template");
        assert!(app.modals.is_empty(), "neither key opened a dialog");
    }

    /// The templates tab's page keys route through the same step as its
    /// arrows, so this is where a page used to come round to the top.
    #[test]
    fn the_templates_tab_pages_without_wrapping() {
        let mut app = fixture(3, 100, 30);
        press(&mut app, Key::ch('T'));
        assert_eq!(app.screen, Screen::Templates);
        press(&mut app, Key::plain(KeyCode::PageDown));
        let last = app.studio.selected;
        assert!(last > 0, "a page moved the cursor");
        press(&mut app, Key::plain(KeyCode::PageDown));
        assert_eq!(
            app.studio.selected, last,
            "and a second page stays at the end"
        );
        press(&mut app, Key::plain(KeyCode::PageUp));
        press(&mut app, Key::plain(KeyCode::PageUp));
        assert_eq!(app.studio.selected, 0, "the top is the top");
    }

    /// **The horizontal axis is focus.** `→` puts the cursor in the pane
    /// beside the list, `←` puts it back — and neither runs anything. `→`
    /// used to open the action menu, which is what Enter is for; an arrow
    /// that executes a verb is an arrow you cannot lean on.
    #[test]
    fn the_right_arrow_focuses_the_pane_and_the_left_arrow_the_list() {
        let mut app = fixture(3, 120, 40);
        assert!(app.detail_visible());
        press(&mut app, Key::plain(KeyCode::Right));
        assert_eq!(app.focus, Focus::Detail, "→ moves into the pane");
        assert!(app.modals.is_empty(), "and opens nothing");
        press(&mut app, Key::plain(KeyCode::Left));
        assert_eq!(app.focus, Focus::Projects, "← comes back to the list");
        press(&mut app, Key::ch('l'));
        assert_eq!(app.focus, Focus::Detail);
        press(&mut app, Key::ch('h'));
        assert_eq!(app.focus, Focus::Projects);
        assert!(app.modals.is_empty());
    }

    /// **The horizontal axis never quits, and never runs.** On the list `←`
    /// has nothing to its left and is not bound; without a pane — the window
    /// is under a hundred columns — `→` has nothing to its right either.
    #[test]
    fn the_arrows_are_unbound_where_there_is_nowhere_to_go() {
        let mut app = fixture(3, 80, 24);
        assert!(
            !app.detail_visible(),
            "the fixture is too narrow for a pane"
        );
        for key in [
            Key::plain(KeyCode::Left),
            Key::ch('h'),
            Key::plain(KeyCode::Right),
            Key::ch('l'),
        ] {
            assert!(
                press(&mut app, key).is_empty(),
                "{} did something",
                key.label()
            );
            assert!(app.modals.is_empty());
            assert_eq!(app.focus, Focus::Projects);
        }
    }

    /// The templates tab has a pane too, and it is always drawn — so `→`
    /// reaches it at any width, and `←` from it is the card list, never the
    /// library: leaving a tab is Esc's ladder and `T`, not an arrow.
    #[test]
    fn the_left_arrow_leaves_the_pane_but_never_the_tab() {
        let mut app = fixture(3, 80, 24);
        press(&mut app, Key::ch('T'));
        assert_eq!(app.screen, Screen::Templates);
        press(&mut app, Key::plain(KeyCode::Right));
        assert_eq!(app.focus, Focus::Detail, "→ reaches the template pane");
        press(&mut app, Key::ch('h'));
        assert_eq!(app.focus, Focus::Projects);
        assert_eq!(app.screen, Screen::Templates, "← is not the way off a tab");
        assert!(press(&mut app, Key::ch('h')).is_empty());
        assert_eq!(
            app.screen,
            Screen::Templates,
            "and a second ← is not either"
        );
        press(&mut app, Key::plain(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Library, "Esc is");
    }

    /// Tab reaches the template pane on a window too narrow for the library's
    /// pane. It measured the library's geometry before, so on an 80-column
    /// window the ring had one member and the template pane's tail — a
    /// `template show` taller than the box — was unreachable.
    #[test]
    fn tab_reaches_the_template_pane_on_a_narrow_window() {
        let mut app = fixture(3, 80, 24);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::plain(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Detail);
        app.studio.lines = (0..60).map(|n| format!("line {n}")).collect();
        press(&mut app, Key::ch('j'));
        assert_eq!(app.studio.scroll, 1, "and the arrows scroll the pane");
        press(&mut app, Key::ch('G'));
        assert!(app.studio.scroll > 1, "G reaches the end of the pane");
        press(&mut app, Key::plain(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Projects, "Tab comes round");
    }

    /// Ctrl-C is a declared command now, so it is in the help — and it still
    /// answers before anything else, from under a dialog that takes every key.
    #[test]
    fn ctrl_c_is_a_command_and_still_answers_first() {
        use fastf::tui::command::{CommandId, Context, find};
        assert!(
            find(CommandId::Interrupt)
                .contexts
                .contains(&Context::Global)
        );
        let mut app = fixture(3, 80, 24);
        press(&mut app, Key::ch('?'));
        assert!(
            press(&mut app, Key::ctrl('c')).is_empty(),
            "it closed the help"
        );
        assert!(app.modals.is_empty());
        assert_eq!(
            press(&mut app, Key::ctrl('c')),
            vec![Effect::Quit(Exit::Interrupted)]
        );
    }

    /// Everything printable in the search bar is the query — `c` types a `c`
    /// rather than opening the palette — and everything else is offered to the
    /// registry, which is what makes `Context::SearchEdit`'s help true.
    #[test]
    fn the_search_bar_types_letters_and_lets_chords_through() {
        let mut app = fixture(3, 80, 24);
        press(&mut app, Key::ch('/'));
        type_text(&mut app, "cq?");
        assert_eq!(app.search.input.text(), "cq?");
        assert!(app.modals.is_empty(), "not one of those opened a dialog");

        press(&mut app, Key::ctrl('p'));
        assert!(
            matches!(app.modals.top(), Some(Modal::Palette(_))),
            "a chord still reaches the registry from the bar"
        );
        press(&mut app, Key::plain(KeyCode::Esc));

        // Esc's ladder: the first clears the query, the second leaves the bar.
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.search.input.is_empty());
        assert!(app.search.editing, "still in the bar, ready to retype");
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(!app.search.editing);
    }

    /// The palette's own keys are declared too, so `Ctrl-p` means the previous
    /// entry there and the opener is bound in every context but this one.
    #[test]
    fn the_palette_moves_with_its_own_keys() {
        let mut app = fixture(6, 100, 30);
        press(&mut app, Key::ch('c'));
        let at = |app: &App| match app.modals.top() {
            Some(Modal::Palette(state)) => state.selected,
            _ => panic!("the palette closed"),
        };
        assert_eq!(at(&app), Some(0));
        press(&mut app, Key::ctrl('n'));
        assert_eq!(at(&app), Some(1));
        press(&mut app, Key::ctrl('p'));
        assert_eq!(at(&app), Some(0), "Ctrl-p is back up, not a second palette");
        type_text(&mut app, "q");
        assert!(
            matches!(app.modals.top(), Some(Modal::Palette(_))),
            "`q` is a letter of the query, not the quit key"
        );
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.modals.is_empty());
    }
}

/// The field-first rule, and the dialogs that could not describe themselves
/// until v3.5.0 declared their keys.
mod key_lines {
    use super::*;

    /// `Ctrl-u` is `HalfUp` on a list and kill-to-start inside the bar. The
    /// field has first refusal, so the same physical key one keystroke apart
    /// keeps meaning what the surface it is on says it means.
    #[test]
    fn the_field_takes_its_chords_before_the_registry_does() {
        let mut app = fixture(40, 80, 24);
        press(&mut app, Key::ctrl('d'));
        assert!(app.library.selected.unwrap() > 0, "Ctrl-d paged the list");

        press(&mut app, Key::ch('/'));
        type_text(&mut app, "lull");
        press(&mut app, Key::ctrl('u'));
        assert!(app.search.input.is_empty(), "Ctrl-u killed the line");
        assert!(app.search.editing, "and did not page anything");
    }

    /// The arrows in the bar are `Down` and `Up` themselves, moving the list
    /// under the query rather than a second pair written out in the handler.
    #[test]
    fn the_arrows_move_the_library_under_the_query() {
        let mut app = fixture(6, 80, 24);
        press(&mut app, Key::ch('/'));
        let first = selected_name(&app);
        press(&mut app, Key::plain(KeyCode::Down));
        assert_ne!(selected_name(&app), first);
        assert!(app.search.editing, "still typing");
        // …and `j` is a letter of the query, not a step.
        let at = app.library.selected;
        type_text(&mut app, "j");
        assert_eq!(app.library.selected, at);
        assert_eq!(app.search.input.text(), "j");
    }

    /// Space ticks a row in a multi-pick and types a space in a picker with a
    /// query — one command, `Hidden` where it does not apply.
    #[test]
    fn space_ticks_a_multi_pick_and_types_in_a_picker() {
        let mut app = fixture(3, 100, 30);
        press(&mut app, Key::ch('A')); // add a tag → a picker with a query
        assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
        press(&mut app, Key::ch(' '));
        match app.modals.top() {
            Some(Modal::Pick(pick)) => assert_eq!(pick.query.text(), " "),
            other => panic!("the picker closed: {other:?}"),
        }
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.modals.is_empty());
    }

    /// Enter and Esc on a one-line prompt are commands now, which is what lets
    /// `Context::Prompt` have a help at all.
    #[test]
    fn a_prompt_confirms_and_cancels_through_the_registry() {
        use fastf::tui::command::{Context, keys_in};

        let mut app = fixture(3, 100, 30);
        press(&mut app, Key::ch('r')); // rename → a text prompt
        assert!(matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
        assert_eq!(app.context(), Context::Prompt);

        // `?` is a letter here, so the bar must not offer it.
        let help = fastf::tui::command::find(fastf::tui::command::CommandId::Help);
        let offered = keys_in(Context::Prompt, help);
        assert!(offered.iter().all(|k| k.typed().is_none()));
        assert!(offered.contains(&Key::plain(KeyCode::F(1))));

        type_text(&mut app, "?");
        assert!(
            matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
            "`?` typed a question mark rather than opening the help"
        );
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.modals.is_empty());
    }
}

/// The four things v3.5.0 added that the app could not do before.
mod more_options {
    use super::*;
    use fastf::tui::app::library::{Order, Sort};
    use fastf::tui::app::modal::Then;

    /// Space marks a row and steps on; `v` reaches from there to the cursor.
    /// Twenty rows are three keystrokes instead of twenty.
    #[test]
    fn v_marks_from_the_last_mark_to_the_cursor() {
        let mut app = fixture(8, 80, 24);
        press(&mut app, Key::ch(' ')); // mark row 0, cursor → 1
        assert_eq!(app.library.marks.len(), 1);
        for _ in 0..3 {
            press(&mut app, Key::plain(KeyCode::Down));
        }
        assert_eq!(app.library.selected, Some(4));
        press(&mut app, Key::ch('v'));
        assert_eq!(
            app.library.marks.len(),
            5,
            "rows 0 through 4, inclusive at both ends"
        );
        for row in 0..5 {
            let path = app.library.row(row).unwrap().path.clone();
            assert!(app.library.marks.contains(&path), "row {row} is marked");
        }
    }

    /// It reaches backwards too, and needs an anchor before it will do
    /// anything at all.
    #[test]
    fn v_refuses_without_an_anchor_and_reaches_both_ways() {
        let mut app = fixture(8, 80, 24);
        assert!(!app.library.has_anchor());
        press(&mut app, Key::ch('v'));
        assert!(app.library.marks.is_empty(), "nothing to reach from");

        press(&mut app, Key::ch('G')); // last row
        press(&mut app, Key::ch(' ')); // mark it (and wrap to the first)
        press(&mut app, Key::ch('g')); // back to the top
        press(&mut app, Key::ch('v'));
        assert_eq!(app.library.marks.len(), app.library.len());
    }

    /// Every one-way order runs both ways, and the tie-break does not turn
    /// round with it.
    #[test]
    fn a_sort_runs_both_ways() {
        let mut app = fixture(6, 80, 24);
        // `s` walks the cycle from newest: oldest, then name.
        for _ in 0..2 {
            press(&mut app, Key::ch('s'));
        }
        assert_eq!(
            app.library.effective_sort(&app.search.query),
            Sort::new(Order::Name)
        );
        let forwards = names(&app);
        app.library.explicit_sort = Some(Sort {
            order: Order::Name,
            reversed: true,
        });
        app.library.recompute(&app.search.query, &mut app.fuzzy);
        let mut expected = forwards.clone();
        expected.reverse();
        assert_eq!(names(&app), expected);

        // The picker offers both, and the label it writes is the label the
        // session file reads back.
        press(&mut app, Key::ch('S'));
        let labels: Vec<String> = match app.modals.top() {
            Some(Modal::Pick(pick)) => pick.items.iter().map(|i| i.label.clone()).collect(),
            other => panic!("the sort picker should be open, not {other:?}"),
        };
        assert!(labels.contains(&"size".to_string()));
        assert!(labels.contains(&"size reversed".to_string()));
        assert!(
            !labels.contains(&"newest reversed".to_string()),
            "newest and oldest are already the two directions of one order"
        );
    }

    /// The tag filter writes the search grammar's own clause, so clearing it
    /// is the same Esc rung as clearing any other query.
    #[test]
    fn filter_by_tag_writes_the_query_the_grammar_already_had() {
        use fastf::tui::command::CommandId;

        let mut app = fixture(9, 100, 30);
        assert!(!app.library.known_tags.is_empty(), "the fixture has tags");
        let _ = app.run(CommandId::FilterTag);
        let tag = match app.modals.top() {
            Some(Modal::Pick(pick)) => {
                assert_eq!(pick.then, Then::TagFilter);
                pick.items[0].value.clone()
            }
            other => panic!("the tag picker should be open, not {other:?}"),
        };
        press(&mut app, Key::plain(KeyCode::Enter));
        assert_eq!(app.search.input.text(), format!("tag:{tag}"));
        assert!(app.modals.is_empty());
    }

    /// `/` narrows the settings list and the title says what to; Esc gives the
    /// whole screen back, because a filter left behind is a screen missing
    /// rows for a reason nobody can see.
    #[test]
    fn slash_filters_the_settings_and_esc_gives_them_back() {
        let mut app = fixture(3, 100, 30);
        press(&mut app, Key::ch(','));
        let _ = update(&mut app, Msg::SettingsLoaded(Box::default()));
        let all = match app.modals.top() {
            Some(Modal::Settings(state)) => state.rows.len(),
            other => panic!("the settings should be open, not {other:?}"),
        };
        press(&mut app, Key::ch('/'));
        type_text(&mut app, "base");
        match app.modals.top() {
            Some(Modal::Settings(state)) => {
                assert!(state.rows.len() < all, "the list narrowed");
                assert!(
                    state.rows.iter().any(|row| row.label == "Base directory"),
                    "and kept what was asked for"
                );
                assert!(
                    state.rows.iter().any(|row| !row.selectable()),
                    "a kept row keeps the heading it is under"
                );
            }
            other => panic!("the settings closed: {other:?}"),
        }
        press(&mut app, Key::plain(KeyCode::Esc));
        match app.modals.top() {
            Some(Modal::Settings(state)) => {
                assert_eq!(state.rows.len(), all, "Esc gave the screen back");
                assert!(state.filter.text().is_empty());
            }
            other => panic!("Esc closed the settings instead: {other:?}"),
        }
    }
}

/// The detail pane's own cursor: it rests only on rows Enter can act on, it
/// stops at the ends, and the pane scrolls to keep it in view.
mod pane_cursor {
    use super::*;
    use fastf::tui::app::data::ProjectDetail;
    use fastf::tui::app::pane::PaneRow;

    fn with_detail(app: &mut App, detail: ProjectDetail) {
        let path = app.library.selected().unwrap().path.clone();
        update(
            app,
            Msg::Detail {
                path,
                detail: Box::new(detail),
            },
        );
    }

    #[test]
    fn the_cursor_walks_selectable_rows_and_stops_at_the_ends() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('j'));
        // Row 1 of the fixture carries two tags.
        assert_eq!(app.library.selected().unwrap().tags.len(), 2);
        with_detail(&mut app, ProjectDetail::default());
        press(&mut app, Key::plain(KeyCode::Right));
        assert_eq!(app.focus, Focus::Detail);
        assert_eq!(app.pane_cursor, 0, "the cursor starts on the name");

        let rows = app.pane_rows();
        press(&mut app, Key::ch('j'));
        assert!(
            matches!(rows[app.pane_cursor], PaneRow::Tag(_)),
            "down from the name is the first tag, over the facts: {:?}",
            rows[app.pane_cursor]
        );
        press(&mut app, Key::ch('G'));
        assert_eq!(rows[app.pane_cursor], PaneRow::AddTodo);
        press(&mut app, Key::ch('j'));
        assert_eq!(
            rows[app.pane_cursor],
            PaneRow::AddTodo,
            "the last row is the last row"
        );
        press(&mut app, Key::ch('g'));
        assert_eq!(app.pane_cursor, 0);
        press(&mut app, Key::ch('k'));
        assert_eq!(app.pane_cursor, 0, "and the first is the first");
        assert!(
            rows.iter().all(|row| !matches!(row, PaneRow::Reading)),
            "a read detail leaves no reading row"
        );
    }

    #[test]
    fn the_pane_scrolls_to_keep_its_cursor_in_view_and_a_new_row_resets_it() {
        let mut app = fixture(6, 120, 24);
        // Five notes of six lines each: thirty rows, over a pane of twenty.
        let detail = ProjectDetail {
            notes: (0..5)
                .map(|n| fastf::core::body::Note {
                    timestamp: Some(format!("2026-01-0{}T00:00:00Z", n + 1)),
                    text: (0..6)
                        .map(|l| format!("note {n} line {l}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                })
                .collect(),
            ..Default::default()
        };
        with_detail(&mut app, detail);
        press(&mut app, Key::plain(KeyCode::Right));
        assert_eq!(app.detail_scroll, 0);
        press(&mut app, Key::ch('G'));
        let rows = app.pane_rows();
        assert_eq!(rows[app.pane_cursor], PaneRow::AddTodo);
        assert!(
            app.detail_scroll > 0,
            "the todo rows sit under thirty rows of notes, so the pane scrolled"
        );
        assert!(
            app.pane_cursor >= app.detail_scroll,
            "and the cursor is inside the window it scrolled to"
        );
        press(&mut app, Key::plain(KeyCode::Left));
        press(&mut app, Key::ch('j'));
        assert_eq!(
            app.pane_cursor, 0,
            "another project, the cursor starts again"
        );
        assert_eq!(app.detail_scroll, 0);
    }

    #[test]
    fn the_cursor_is_drawn_only_while_the_pane_has_the_focus() {
        let mut app = fixture(6, 120, 40);
        with_detail(&mut app, ProjectDetail::default());
        press(&mut app, Key::plain(KeyCode::Right));
        // The cursor is a style, not a glyph — the pane's text does not move
        // when the focus arrives — so it is the buffer that shows it: mono
        // draws the selection reversed.
        let lit = fastf::tui::testing::render_to_buffer(&app, 120, 40);
        let pane = app.regions().detail.expect("a pane at 120 columns");
        let name_row = &lit[(pane.x + 1, pane.y + 1)];
        assert!(
            name_row
                .modifier
                .contains(ratatui::style::Modifier::REVERSED),
            "the name row wears the selection while the pane has the focus"
        );
        press(&mut app, Key::plain(KeyCode::Left));
        let dark = fastf::tui::testing::render_to_buffer(&app, 120, 40);
        assert!(
            !dark[(pane.x + 1, pane.y + 1)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED),
            "and not while the list has it"
        );
    }
}

/// The detail pane as an editor you enter on purpose: nothing changes until
/// Enter on a row, Esc leaves the row as it was, and what can be typed is
/// what the file can hold.
mod pane_editor {
    use super::*;
    use fastf::core::project_info::Metadata;
    use fastf::core::template::{Transform, VarType, Variable};
    use fastf::tui::app::data::ProjectDetail;
    use fastf::tui::app::pane::{PaneEdit, PaneRow};
    use fastf::tui::command::Context;
    use fastf::tui::effect::Action;
    use std::collections::BTreeMap;

    fn variable(slug: &str, var_type: VarType, options: &[&str]) -> Variable {
        Variable {
            slug: slug.to_string(),
            label: slug.to_uppercase(),
            var_type,
            required: false,
            options: options.iter().map(|o| o.to_string()).collect(),
            default: String::new(),
            transform: Transform::None,
        }
    }

    /// A pane with a text variable, a select, notes, and the fixture's tags,
    /// focused and ready.
    fn editing_fixture() -> App {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('j'));
        let project = app.library.selected().unwrap().clone();
        assert_eq!(project.tags, vec!["client/Acme", "draft"]);
        let meta = Metadata {
            id: project.id.clone(),
            id_number: project.id_number,
            template: project.template.clone(),
            template_name: project.template_name.clone(),
            created: project.created.clone(),
            folder: project.name.clone(),
            path: String::new(),
            variables: BTreeMap::from([
                ("artist".to_string(), "Ariana".to_string()),
                ("tier".to_string(), "Indie".to_string()),
            ]),
            tags: project.tags.clone(),
            auto_tags: Vec::new(),
            provisioning: false,
        };
        let detail = ProjectDetail {
            meta: Some(meta),
            variables: vec![
                variable("artist", VarType::Text, &[]),
                variable("tier", VarType::Select, &["Indie", "Major"]),
            ],
            notes: vec![
                fastf::core::body::Note {
                    timestamp: None,
                    text: "first cut Friday".to_string(),
                },
                fastf::core::body::Note {
                    timestamp: Some("2026-08-28T10:00:00Z".to_string()),
                    text: "began the edit\nrough cut by Friday".to_string(),
                },
            ],
            todos: vec![
                fastf::core::body::Todo {
                    done: true,
                    text: "ingested the videos".to_string(),
                },
                fastf::core::body::Todo {
                    done: false,
                    text: "delivered the video".to_string(),
                },
            ],
            ..Default::default()
        };
        update(
            &mut app,
            Msg::Detail {
                path: project.path.clone(),
                detail: Box::new(detail),
            },
        );
        press(&mut app, Key::plain(KeyCode::Right));
        assert_eq!(app.focus, Focus::Detail);
        app
    }

    fn go_to(app: &mut App, wanted: impl Fn(&PaneRow) -> bool) {
        let rows = app.pane_rows();
        let at = rows
            .iter()
            .position(wanted)
            .expect("the row is in the pane");
        press(app, Key::ch('g'));
        while app.pane_cursor < at {
            let before = app.pane_cursor;
            press(app, Key::ch('j'));
            assert!(app.pane_cursor > before, "the cursor cannot reach row {at}");
        }
        assert_eq!(app.pane_cursor, at);
    }

    fn sent(effects: &[Effect]) -> Option<&Action> {
        effects.iter().find_map(|e| match e {
            Effect::Run(_, action) => Some(action.as_ref()),
            _ => None,
        })
    }

    #[test]
    fn nothing_edits_before_enter_and_esc_leaves_the_row_as_it_was() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
        );
        for key in [Key::ch('x'), Key::ch('a'), Key::plain(KeyCode::Backspace)] {
            let effects = press(&mut app, key);
            assert!(
                !effects.iter().any(|e| matches!(e, Effect::Run(..))),
                "{} ran something without Enter: {effects:?}",
                key.label()
            );
        }
        assert!(
            app.pane_edit.is_none(),
            "typing on a row does not open an edit"
        );
        // `a` opened the action menu — that key still works in the pane.
        assert!(matches!(app.modals.top(), Some(Modal::Actions(_))));
        press(&mut app, Key::plain(KeyCode::Esc));

        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(&app.pane_edit, Some(PaneEdit::Line { .. })),
            "Enter opens the line editor: {:?}",
            app.pane_edit
        );
        assert_eq!(app.context(), Context::PaneEdit);
        type_text(&mut app, " Grande");
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.pane_edit.is_none(), "Esc closes the edit");
        let rows = app.pane_rows();
        assert!(
            matches!(&rows[app.pane_cursor], PaneRow::Variable { value, .. } if value == "Ariana"),
            "and the row is as it was"
        );
    }

    #[test]
    fn enter_on_a_text_variable_edits_in_place_and_enter_again_sends_it() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
        );
        let row = app.pane_cursor;
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "_Grande");
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        let project = app.library.selected().unwrap().clone();
        assert_eq!(
            sent(&effects),
            Some(&Action::SetVariable {
                project: Box::new(project.clone()),
                slug: "artist".to_string(),
                value: "Ariana_Grande".to_string(),
            })
        );
        assert!(
            app.pane_edit.as_ref().is_some_and(|edit| edit.pending()),
            "the edit stays open, pending, until the worker answers"
        );
        // Typing while pending changes nothing.
        type_text(&mut app, "zzz");
        assert!(
            matches!(&app.pane_edit, Some(PaneEdit::Line { input, .. }) if input.text() == "Ariana_Grande")
        );

        // The worker answers: the row is patched, the edit closes, the cursor
        // stays on the row, and the row pulses.
        app.theme = fastf::tui::theme::Theme::rich();
        app.motion = fastf::tui::motion::Motion::On;
        app.elapsed_ms = 2_000;
        let mut patched = project.clone();
        patched.tags.push("tier/Indie".to_string());
        let id = app.busy_id.expect("an action in flight");
        update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                    fastf::tui::effect::ListChange::Patched {
                        project: Box::new(patched),
                        was: project.path.clone(),
                        stale: vec![project.path.clone()],
                    },
                    "Set artist",
                ))),
            },
        );
        assert!(app.pane_edit.is_none(), "an Ok closes the edit");
        // The patch dropped the cached detail, so the variables are gone
        // until the re-read lands — and a tag row arrived above them. The
        // cursor follows the variable, not its old index.
        let rows = app.pane_rows();
        assert!(
            rows.iter().all(|r| !matches!(r, PaneRow::Variable { .. })),
            "the detail is being re-read"
        );
        let detail = ProjectDetail {
            meta: Some(Metadata {
                id: project.id.clone(),
                id_number: project.id_number,
                template: project.template.clone(),
                template_name: project.template_name.clone(),
                created: project.created.clone(),
                folder: project.name.clone(),
                path: String::new(),
                variables: BTreeMap::from([
                    ("artist".to_string(), "Ariana_Grande".to_string()),
                    ("tier".to_string(), "Indie".to_string()),
                ]),
                tags: Vec::new(),
                auto_tags: Vec::new(),
                provisioning: false,
            }),
            variables: vec![
                variable("artist", VarType::Text, &[]),
                variable("tier", VarType::Select, &["Indie", "Major"]),
            ],
            ..Default::default()
        };
        update(
            &mut app,
            Msg::Detail {
                path: project.path.clone(),
                detail: Box::new(detail),
            },
        );
        let rows = app.pane_rows();
        assert!(
            matches!(&rows[app.pane_cursor], PaneRow::Variable { slug, value, .. } if slug == "artist" && value == "Ariana_Grande"),
            "the cursor is on the variable that changed, wherever it is now: {:?}",
            rows[app.pane_cursor]
        );
        assert_ne!(
            app.pane_cursor, row,
            "which is one row down, under the new tag"
        );
        assert!(
            app.pane_pulses
                .style_for(&app.pane_cursor, 2_000, &app.theme, app.motion)
                .is_some(),
            "and that row pulses"
        );
        assert_eq!(
            app.tick_interval(),
            Some(std::time::Duration::from_millis(
                fastf::tui::motion::FRAME_MS
            ))
        );
    }

    #[test]
    fn a_refusal_lands_on_the_open_edit_with_the_text_still_there() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
        );
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "!");
        press(&mut app, Key::plain(KeyCode::Enter));
        let id = app.busy_id.expect("an action in flight");
        update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Err("artist is required".to_string()),
            },
        );
        match &app.pane_edit {
            Some(PaneEdit::Line {
                input,
                error,
                pending,
                ..
            }) => {
                assert_eq!(
                    input.text(),
                    "Ariana!",
                    "the text is still there to correct"
                );
                assert_eq!(error.as_deref(), Some("artist is required"));
                assert!(!pending, "and it can be sent again");
            }
            other => panic!("the edit closed on a refusal: {other:?}"),
        }
        assert!(
            app.modals.is_empty(),
            "no dialog for a refusal under the field"
        );
        // Typing clears the message.
        press(&mut app, Key::plain(KeyCode::Backspace));
        assert!(app.pane_edit.as_ref().unwrap().error().is_none());
    }

    #[test]
    fn a_tag_is_edited_in_place_and_emptied_it_is_removed() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Tag(tag) if tag == "draft"),
        );
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(&app.pane_edit, Some(PaneEdit::Line { input, .. }) if input.text() == "draft")
        );
        // Unchanged is a cancel, not a write.
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(sent(&effects).is_none());
        assert!(app.pane_edit.is_none());

        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "-v2");
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        let project = app.library.selected().unwrap().clone();
        assert_eq!(
            sent(&effects),
            Some(&Action::ReplaceTag {
                project: Box::new(project.clone()),
                from: "draft".to_string(),
                to: Some("draft-v2".to_string()),
            })
        );
        let id = app.busy_id.unwrap();
        update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Err("no".to_string()),
            },
        );
        press(&mut app, Key::plain(KeyCode::Esc));

        press(&mut app, Key::plain(KeyCode::Enter));
        for _ in 0..5 {
            press(&mut app, Key::plain(KeyCode::Backspace));
        }
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        assert_eq!(
            sent(&effects),
            Some(&Action::ReplaceTag {
                project: Box::new(project),
                from: "draft".to_string(),
                to: None,
            }),
            "an emptied tag is removed"
        );
    }

    #[test]
    fn a_tag_that_is_not_a_tag_is_refused_under_the_line() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Tag(tag) if tag == "draft"),
        );
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, " a poem");
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(sent(&effects).is_none(), "nothing was sent");
        match &app.pane_edit {
            Some(PaneEdit::Line { error, .. }) => {
                assert!(
                    error.as_deref().is_some_and(|e| e.contains("one word")),
                    "{error:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn enter_on_add_tag_the_name_and_the_journal_open_the_flows_that_exist() {
        let mut app = editing_fixture();
        go_to(&mut app, |row| matches!(row, PaneRow::AddTag));
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(
                app.modals.top(),
                Some(Modal::Pick(_)) | Some(Modal::TextPrompt(_))
            ),
            "add a tag is the tag flow: {:?}",
            app.modals.top().map(|m| m.context())
        );
        press(&mut app, Key::plain(KeyCode::Esc));

        go_to(&mut app, |row| matches!(row, PaneRow::Name));
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
            "the name is the rename prompt"
        );
        press(&mut app, Key::plain(KeyCode::Esc));

        go_to(&mut app, |row| matches!(row, PaneRow::AddNote));
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(app.modals.top(), Some(Modal::Note(_))),
            "add a note is the quick note"
        );
        press(&mut app, Key::plain(KeyCode::Esc));

        go_to(&mut app, |row| matches!(row, PaneRow::AddTodo));
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            matches!(app.modals.top(), Some(Modal::TextPrompt(_))),
            "add a todo asks for its text"
        );
        type_text(&mut app, "invoice");
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        let project = app.library.selected().unwrap().clone();
        assert_eq!(
            sent(&effects),
            Some(&Action::AddTodo {
                project: Box::new(project.clone()),
                text: "invoice".to_string(),
            })
        );
        assert!(app.modals.is_empty());
        let id = app.busy_id.unwrap();
        update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                    fastf::tui::effect::ListChange::DetailOnly {
                        path: project.path.clone(),
                    },
                    "Todo added.",
                ))),
            },
        );
    }

    /// Enter on a todo writes the toggle at once — there is nothing to type
    /// — with no edit open, and the answer lands on that row: the cursor
    /// stays there and the row pulses, as an edit's does. Nothing on the
    /// list lights up, because nothing on the list changed.
    #[test]
    fn enter_on_a_todo_toggles_it_and_the_answer_lands_on_its_row() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Todo { text, .. } if text == "delivered the video"),
        );
        let at = app.pane_cursor;
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        let project = app.library.selected().unwrap().clone();
        assert_eq!(
            sent(&effects),
            Some(&Action::ToggleTodo {
                project: Box::new(project.clone()),
                ordinal: 1,
                was: "delivered the video".to_string(),
            })
        );
        assert!(app.pane_edit.is_none(), "a toggle opens nothing");
        // Nothing else starts while the write is on its way.
        let again = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(sent(&again).is_none(), "{again:?}");

        let id = app.busy_id.unwrap();
        let effects = update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
                    fastf::tui::effect::ListChange::DetailOnly {
                        path: project.path.clone(),
                    },
                    "Done.",
                ))),
            },
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadDetail(p) if *p == project.path)),
            "the detail is read again: {effects:?}"
        );
        assert!(
            app.pulses.is_empty(),
            "no row of the list changed, so none lights up"
        );
        assert_eq!(app.pane_cursor, at, "the cursor stays on the todo");
        assert!(!app.pane_pulses.is_empty(), "and the todo's row pulses");
        // The pane keeps what it shows until the re-read lands — no
        // `reading…` frame between the keypress and the answer — and the
        // re-read lands with the todo flipped, the cursor still on it.
        assert!(
            app.details.contains_key(&project.path),
            "the detail on screen stays until the fresh one arrives"
        );
        let fresh = fastf::tui::app::data::ProjectDetail {
            todos: vec![
                fastf::core::body::Todo {
                    done: true,
                    text: "ingested the videos".to_string(),
                },
                fastf::core::body::Todo {
                    done: true,
                    text: "delivered the video".to_string(),
                },
            ],
            ..Default::default()
        };
        update(
            &mut app,
            Msg::Detail {
                path: project.path.clone(),
                detail: Box::new(fresh),
            },
        );
        assert!(matches!(
            app.pane_rows()[app.pane_cursor],
            PaneRow::Todo {
                done: true,
                ordinal: 1,
                ..
            }
        ));
    }

    /// `… n earlier` shows every note; Enter on it is `J`.
    #[test]
    fn enter_on_earlier_notes_shows_them_all() {
        let mut app = editing_fixture();
        let project = app.library.selected().unwrap().clone();
        let mut detail = app.details.get(&project.path).cloned().unwrap();
        detail.notes = (0..7)
            .map(|n| fastf::core::body::Note {
                timestamp: Some(format!("2026-01-0{}T00:00:00Z", n + 1)),
                text: format!("note {n}"),
            })
            .collect();
        update(
            &mut app,
            Msg::Detail {
                path: project.path.clone(),
                detail: Box::new(detail),
            },
        );
        go_to(&mut app, |row| matches!(row, PaneRow::EarlierNotes(2)));
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::LoadView {
                    kind: fastf::tui::effect::ViewKind::Journal,
                    ..
                }
            )),
            "{effects:?}"
        );
    }

    #[test]
    fn a_select_variable_offers_only_its_options_and_the_pick_is_the_edit() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "tier"),
        );
        press(&mut app, Key::plain(KeyCode::Enter));
        let labels: Vec<String> = match app.modals.top() {
            Some(Modal::Pick(pick)) => pick.items.iter().map(|i| i.label.clone()).collect(),
            other => panic!("a select opens a picker: {other:?}"),
        };
        assert_eq!(labels, vec!["Indie", "Major"]);
        press(&mut app, Key::plain(KeyCode::Down));
        let effects = press(&mut app, Key::plain(KeyCode::Enter));
        let project = app.library.selected().unwrap().clone();
        assert_eq!(
            sent(&effects),
            Some(&Action::SetVariable {
                project: Box::new(project),
                slug: "tier".to_string(),
                value: "Major".to_string(),
            })
        );
        assert!(app.modals.is_empty());
    }

    #[test]
    fn a_note_is_a_text_area_saved_with_ctrl_s_and_enter_is_a_new_line() {
        let mut app = editing_fixture();
        go_to(&mut app, |row| {
            matches!(row, PaneRow::Note { ordinal: 0, .. })
        });
        press(&mut app, Key::plain(KeyCode::Enter));
        assert!(matches!(
            &app.pane_edit,
            Some(PaneEdit::Note { ordinal: 0, .. })
        ));
        press(&mut app, Key::plain(KeyCode::End));
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "then colour");
        assert!(
            app.pane_edit.is_some(),
            "Enter in a note is a new line, not a send"
        );
        let effects = press(&mut app, Key::ctrl('s'));
        let project = app.library.selected().unwrap().clone();
        assert_eq!(
            sent(&effects),
            Some(&Action::ReplaceNote {
                project: Box::new(project.clone()),
                ordinal: 0,
                was: "first cut Friday".to_string(),
                text: "first cut Friday\nthen colour".to_string(),
            }),
            "Ctrl-S saves the note, naming the text it read"
        );
        let id = app.busy_id.unwrap();
        update(
            &mut app,
            Msg::ActionDone {
                id,
                outcome: Err("the note changed meanwhile — reload and edit it again".to_string()),
            },
        );
        assert!(
            matches!(&app.pane_edit, Some(PaneEdit::Note { error: Some(e), .. }) if e.contains("changed meanwhile")),
            "a refusal lands on the note editor"
        );

        // A dated note edits the same way, and its other lines are there.
        press(&mut app, Key::plain(KeyCode::Esc));
        go_to(&mut app, |row| {
            matches!(row, PaneRow::Note { ordinal: 1, .. })
        });
        press(&mut app, Key::plain(KeyCode::Enter));
        match &app.pane_edit {
            Some(PaneEdit::Note { area, was, .. }) => {
                assert_eq!(area.text(), "began the edit\nrough cut by Friday");
                assert_eq!(was, "began the edit\nrough cut by Friday");
            }
            other => panic!("{other:?}"),
        }
        // Unchanged, Ctrl-S is a cancel.
        let effects = press(&mut app, Key::ctrl('s'));
        assert!(sent(&effects).is_none() && app.pane_edit.is_none());
    }

    #[test]
    fn leaving_the_pane_or_the_row_drops_an_open_edit_untouched() {
        let mut app = editing_fixture();
        go_to(
            &mut app,
            |row| matches!(row, PaneRow::Variable { slug, .. } if slug == "artist"),
        );
        press(&mut app, Key::plain(KeyCode::Enter));
        type_text(&mut app, "x");
        // `←` is the registry's while a line is being edited? No — the field
        // has the arrows as its caret, so it is Esc, then ←.
        press(&mut app, Key::plain(KeyCode::Left));
        assert!(app.pane_edit.is_some(), "← moves the caret, not the focus");
        press(&mut app, Key::plain(KeyCode::Esc));
        assert!(app.pane_edit.is_none());
        press(&mut app, Key::plain(KeyCode::Enter));
        press(&mut app, Key::plain(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Projects);
        assert!(
            app.pane_edit.is_none(),
            "leaving the pane leaves the row as it was"
        );
    }

    /// The bar says what the pane is for once it has the focus, and what an
    /// open edit answers to.
    #[test]
    fn the_hint_bar_reads_the_pane_and_the_edit() {
        let mut app = editing_fixture();
        let hints: Vec<String> = fastf::tui::command::hints(app.context(), &app, 200)
            .into_iter()
            .map(|(key, what)| format!("{key} {what}"))
            .collect();
        assert!(hints.iter().any(|h| h == "Enter edit"), "{hints:?}");
        assert!(hints.iter().any(|h| h == "← list"), "{hints:?}");
        assert!(
            hints.iter().any(|h| h == "a actions"),
            "`a` still works in the pane: {hints:?}"
        );
        // Enter says what it will do to the row under the cursor.
        for (wanted, verb) in [
            (
                Box::new(|row: &PaneRow| matches!(row, PaneRow::Todo { .. }))
                    as Box<dyn Fn(&PaneRow) -> bool>,
                "Enter toggle",
            ),
            (
                Box::new(|row: &PaneRow| matches!(row, PaneRow::AddNote)),
                "Enter add",
            ),
            (
                Box::new(|row: &PaneRow| matches!(row, PaneRow::AddTodo)),
                "Enter add",
            ),
            (
                Box::new(|row: &PaneRow| matches!(row, PaneRow::Note { .. })),
                "Enter edit",
            ),
        ] {
            go_to(&mut app, wanted);
            let hints: Vec<String> = fastf::tui::command::hints(app.context(), &app, 200)
                .into_iter()
                .map(|(key, what)| format!("{key} {what}"))
                .collect();
            assert!(hints.iter().any(|h| h == verb), "{verb}: {hints:?}");
        }
        go_to(&mut app, |row| {
            matches!(row, PaneRow::Note { ordinal: 0, .. })
        });
        press(&mut app, Key::plain(KeyCode::Enter));
        let hints: Vec<String> = fastf::tui::command::hints(app.context(), &app, 200)
            .into_iter()
            .map(|(key, what)| format!("{key} {what}"))
            .collect();
        assert!(hints.iter().any(|h| h == "Ctrl-s save"), "{hints:?}");
        assert!(hints.iter().any(|h| h == "Esc cancel"), "{hints:?}");
        assert!(
            !hints.iter().any(|h| h.starts_with("Enter")),
            "Enter is a new line here: {hints:?}"
        );
    }
}

/// Motion, which only ever appears where a still frame could not answer a
/// question: what changed, where the focus went, where a row went, what is
/// working, what is going away — and which fades rather than flashes.
mod motion {
    use super::*;
    use fastf::tui::motion::{self, Motion};
    use fastf::tui::testing::render_to_buffer;
    use fastf::tui::theme::Theme;

    fn row_of(app: &App, name: &str) -> u16 {
        let row = (0..app.library.len())
            .find(|&row| app.library.row(row).is_some_and(|p| p.name == name))
            .expect("the row is on the list");
        // Two for the block's border and the header row above the first.
        app.regions().table.y + 2 + row as u16
    }

    /// **What changed, said on the row it changed.** A batch touches rows the
    /// cursor is nowhere near; without this the frame after is identical to
    /// the frame before except for cells nobody was looking at.
    #[test]
    fn a_changed_row_lights_up_and_lets_go() {
        let mut app = fixture(6, 100, 30);
        app.theme = Theme::rich();
        app.motion = Motion::On;
        let project = app.library.row(2).unwrap().clone();
        let name = project.name.clone();

        app.elapsed_ms = 1_000;
        let _ = app.apply_change(ListChange::Patched {
            project: Box::new(project.clone()),
            was: project.path.clone(),
            stale: Vec::new(),
        });

        let lit = render_to_buffer(&app, 100, 30);
        let at = row_of(&app, &name);
        let column = app.regions().table.x + 3;
        assert_eq!(
            lit[(column, at)].bg,
            app.theme.pulse,
            "the row a verb just changed wears the wash"
        );
        assert_eq!(
            lit[(column, at)].fg,
            app.theme.accent,
            "and its own colours are left alone — the id is still the id"
        );

        // …fades: half way through it is neither the wash nor the ground, on
        // its way from one to the other…
        app.elapsed_ms = 1_000 + motion::PULSE_MS / 2;
        let _ = update(&mut app, Msg::Tick);
        let mid = render_to_buffer(&app, 100, 30)[(column, at)].bg;
        assert!(
            mid != app.theme.pulse
                && mid != app.theme.ground
                && mid != ratatui::style::Color::Reset,
            "a pulse fades rather than flashing: {mid:?}"
        );

        // …and lets go: the whole pulse is over at its duration.
        app.elapsed_ms = 1_000 + motion::PULSE_MS;
        let _ = update(&mut app, Msg::Tick);
        let gone = render_to_buffer(&app, 100, 30);
        assert_ne!(gone[(column, at)].bg, app.theme.pulse);
        assert!(app.pulses.is_empty(), "and nothing is left in flight");
    }

    /// **Find my row.** A sort or a filter keeps the selection by path, so
    /// the row you were on is somewhere else on the screen now — and it
    /// pulses so the eye finds where it went. Rows changing for any other
    /// reason — discovery, sizes, a keystroke in the search — do not.
    #[test]
    fn a_sort_or_a_filter_pulses_the_selected_row_so_the_eye_finds_it() {
        let mut app = fixture(6, 100, 30);
        app.theme = Theme::rich();
        app.elapsed_ms = 1_000;
        press(&mut app, Key::ch('j'));
        press(&mut app, Key::ch('j'));
        let selected = app.library.selected().unwrap().path.clone();
        assert!(app.pulses.is_empty());

        press(&mut app, Key::ch('s'));
        assert_eq!(
            app.library.selected().unwrap().path,
            selected,
            "the selection survives the sort"
        );
        assert!(
            app.pulses
                .style_for(&selected, 1_000, &app.theme, Motion::On)
                .is_some(),
            "and pulses where it landed"
        );

        app.pulses.clear();
        press(&mut app, Key::ch('f'));
        assert!(
            app.pulses
                .style_for(&selected, 1_000, &app.theme, Motion::On)
                .is_some(),
            "a filter reorders too"
        );

        // A change that nobody asked to reorder is not a reorder.
        app.pulses.clear();
        let _ = app.apply_change(ListChange::None);
        assert!(app.pulses.is_empty(), "rows recomputed, nothing reordered");
    }

    /// **A page filling in is not a change, and lighting it up is a flash.**
    /// Every visible row's size lands at once — on the first screenful, and
    /// again on every scroll — so pulsing on arrival washed the whole list at
    /// a stroke, twenty rows together, several times in the first seconds of
    /// a run. It read as a fault, which is how it was reported.
    #[test]
    fn a_page_of_sizes_arriving_for_the_first_time_does_not_pulse() {
        let mut app = fixture(6, 100, 30);
        app.theme = Theme::rich();
        app.elapsed_ms = 500;
        let paths: Vec<_> = (0..app.library.len())
            .map(|row| app.library.row(row).unwrap().path.clone())
            .collect();
        let cells = paths.iter().map(|p| (p.clone(), Some(4096))).collect();
        let _ = update(&mut app, Msg::Sizes(cells));
        assert!(
            app.pulses.is_empty(),
            "the first fill of a page is the page arriving, not a row changing"
        );

        // And the same size again is still not news.
        let _ = update(&mut app, Msg::Sizes(vec![(paths[1].clone(), Some(4096))]));
        assert!(app.pulses.is_empty());
    }

    /// A number replacing a *different* number is a change on that row: the
    /// table is measured from the rows and never from the sizes, so nothing
    /// reflows around it and the figure would otherwise change under your eyes
    /// in silence.
    #[test]
    fn a_size_that_changes_pulses_its_row() {
        let mut app = fixture(6, 100, 30);
        app.theme = Theme::rich();
        app.elapsed_ms = 500;
        let path = app.library.row(1).unwrap().path.clone();
        let _ = update(&mut app, Msg::Sizes(vec![(path.clone(), Some(4096))]));
        assert!(app.pulses.is_empty(), "the first one is an arrival");
        let _ = update(&mut app, Msg::Sizes(vec![(path.clone(), Some(8192))]));
        assert!(
            app.pulses
                .style_for(&path, 500, &app.theme, Motion::On)
                .is_some(),
            "the second one is a change"
        );
    }

    /// A size a verb threw away, coming back, is a change too — and it comes
    /// back looking exactly like a first arrival, because the old number was
    /// discarded with the row's other stale reads. `ListChange::Patched`'s
    /// `stale` set is what tells the two apart.
    #[test]
    fn a_size_rescanned_after_a_verb_pulses_its_row() {
        let mut app = fixture(6, 100, 30);
        app.theme = Theme::rich();
        app.elapsed_ms = 500;
        let project = app.library.row(1).unwrap().clone();
        let path = project.path.clone();
        let _ = update(&mut app, Msg::Sizes(vec![(path.clone(), Some(4096))]));

        let _ = app.apply_change(ListChange::Patched {
            project: Box::new(project.clone()),
            was: path.clone(),
            stale: vec![path.clone()],
        });
        // The verb's own row pulse is not what this test is about.
        app.pulses.clear();

        let _ = update(&mut app, Msg::Sizes(vec![(path.clone(), Some(4096))]));
        assert!(
            app.pulses
                .style_for(&path, 500, &app.theme, Motion::On)
                .is_some(),
            "the rescan that answers a verb is news even at the same number"
        );

        // And it is spent: the next arrival is an arrival again.
        app.pulses.clear();
        let _ = update(&mut app, Msg::Sizes(vec![(path.clone(), Some(4096))]));
        assert!(app.pulses.is_empty());
    }

    /// **Focus that moved is seen where it landed.** A border changing colour
    /// on a line nobody was reading is not seen; the pane the focus arrived
    /// in eases from its resting colour to its focused one, and the pane it
    /// left eases the other way at the same moment — two rest states with a
    /// transition between them, no wash and no snap — and the app asks for
    /// the fast wake only for that moment.
    #[test]
    fn moving_focus_eases_the_borders_and_titles_between_their_rest_states() {
        let mut app = fixture(6, 120, 40);
        app.theme = Theme::rich();
        app.motion = Motion::On;
        for row in 0..app.library.len() {
            let path = app.library.row(row).unwrap().path.clone();
            app.library.sizes.insert(path, Some(1));
        }
        app.status = Default::default();
        assert_eq!(app.tick_interval(), None, "a still app asks for no wake");

        let pane = app.regions().detail.expect("a pane at 120 columns");
        let table = app.regions().table;
        // The title sits on the top border, two cells in; the border's own
        // colour is read one row down, on the left edge.
        let title = |buf: &ratatui::buffer::Buffer, rect: ratatui::layout::Rect| {
            buf[(rect.x + 2, rect.y)].fg
        };
        let edge = |buf: &ratatui::buffer::Buffer, rect: ratatui::layout::Rect| {
            buf[(rect.x, rect.y + 1)].fg
        };
        let before = render_to_buffer(&app, 120, 40);
        assert_eq!(edge(&before, table), app.theme.border_focus);
        assert_eq!(edge(&before, pane), app.theme.border);

        app.elapsed_ms = 1_000;
        press(&mut app, Key::plain(KeyCode::Right));
        assert_eq!(app.focus, Focus::Detail);
        assert_eq!(
            app.tick_interval(),
            Some(std::time::Duration::from_millis(motion::FRAME_MS))
        );
        // The moment of the move: each still the colour it had, and no wash.
        let moment = render_to_buffer(&app, 120, 40);
        assert_eq!(edge(&moment, pane), app.theme.border);
        assert_eq!(edge(&moment, table), app.theme.border_focus);
        assert_eq!(title(&moment, pane), app.theme.dim);
        assert_eq!(
            moment[(pane.x + 2, pane.y)].bg,
            ratatui::style::Color::Reset,
            "no wash under the title: a transition, not a flash"
        );

        // Half way: both borders between their two colours.
        app.elapsed_ms = 1_000 + motion::FOCUS_MS / 2;
        let _ = update(&mut app, Msg::Tick);
        let mid = render_to_buffer(&app, 120, 40);
        for (rect, name) in [(pane, "pane"), (table, "table")] {
            let fg = edge(&mid, rect);
            assert!(
                fg != app.theme.border && fg != app.theme.border_focus,
                "the {name}'s border is on its way: {fg:?}"
            );
        }

        // Done: at rest, on the other side, and the fast wake let go.
        app.elapsed_ms = 1_000 + motion::FOCUS_MS;
        let _ = update(&mut app, Msg::Tick);
        assert_eq!(app.tick_interval(), None, "it lets go with the ease");
        assert!(app.focus_moved_at.is_none());
        let after = render_to_buffer(&app, 120, 40);
        assert_eq!(edge(&after, pane), app.theme.border_focus);
        assert_eq!(edge(&after, table), app.theme.border);
        assert_eq!(title(&after, pane), app.theme.accent);

        // The same focus again is not a move.
        press(&mut app, Key::plain(KeyCode::Right));
        assert!(app.focus_moved_at.is_none());
    }

    /// Off is a hard cut everywhere: the rest state on the first frame.
    #[test]
    fn with_motion_off_the_focus_lands_at_once() {
        let mut app = fixture(6, 120, 40);
        app.theme = Theme::rich();
        app.motion = Motion::Off;
        for row in 0..app.library.len() {
            let path = app.library.row(row).unwrap().path.clone();
            app.library.sizes.insert(path, Some(1));
        }
        app.status = Default::default();
        app.elapsed_ms = 1_000;
        press(&mut app, Key::plain(KeyCode::Right));
        let pane = app.regions().detail.expect("a pane at 120 columns");
        let frame = render_to_buffer(&app, 120, 40);
        assert_eq!(frame[(pane.x, pane.y + 1)].fg, app.theme.border_focus);
        assert_eq!(frame[(pane.x + 2, pane.y)].fg, app.theme.accent);
        assert_eq!(app.tick_interval(), None, "and nothing asks for a wake");
    }

    /// **A message arrives.** The status line is where what just happened is
    /// said, and a line that changes its text in silence is not read; it
    /// wears the wash as it arrives, and lets go — then dims on its way out,
    /// as before.
    #[test]
    fn a_status_line_arrives_with_a_wash_and_settles() {
        let mut app = fixture(6, 100, 30);
        app.theme = Theme::rich();
        app.motion = Motion::On;
        app.elapsed_ms = 1_000;
        press(&mut app, Key::ch('s'));
        assert!(app.status.text.contains("sorted by"), "{:?}", app.status);
        let status = app.regions().status;
        let lit = render_to_buffer(&app, 100, 30);
        assert_eq!(
            lit[(status.x + 1, status.y)].bg,
            app.theme.pulse,
            "the message wears the wash as it arrives"
        );
        assert_eq!(
            lit[(status.x + status.width - 1, status.y)].bg,
            app.theme.pulse,
            "under the whole line, not only the words"
        );
        app.elapsed_ms = 1_000 + motion::ARRIVE_MS;
        let _ = update(&mut app, Msg::Tick);
        let settled = render_to_buffer(&app, 100, 30);
        assert_eq!(
            settled[(status.x + 1, status.y)].bg,
            ratatui::style::Color::Reset
        );
    }

    /// **The app still costs nothing while idle.** A pulse asks for twenty
    /// frames a second while it is in flight and nothing at all once it is
    /// over — a claim `docs/cli.md` makes and this holds it to.
    #[test]
    fn the_faster_wake_ends_with_the_pulse() {
        let mut app = fixture(6, 100, 30);
        // A palette that can show the pulse: mono never moves, and never
        // asks for the wake either.
        app.theme = Theme::rich();
        // Nothing pending: the fixture's sizes are all known.
        for row in 0..app.library.len() {
            let path = app.library.row(row).unwrap().path.clone();
            app.library.sizes.insert(path, Some(1));
        }
        app.status = Default::default();
        assert_eq!(app.tick_interval(), None, "a still app asks for no wake");

        app.elapsed_ms = 100;
        app.pulses
            .start(app.library.row(0).unwrap().path.clone(), 100);
        assert_eq!(
            app.tick_interval(),
            Some(std::time::Duration::from_millis(motion::FRAME_MS))
        );
        app.elapsed_ms = 100 + motion::PULSE_MS;
        let _ = update(&mut app, Msg::Tick);
        assert_eq!(app.tick_interval(), None, "and it stops with the pulse");
    }

    /// Off is a first-class state, and a theme with no colour is always off:
    /// a colour wash on a mono terminal is a flicker rather than a cue.
    #[test]
    fn motion_off_and_mono_draw_the_same_frame_as_before() {
        let mut app = fixture(6, 100, 30);
        app.theme = Theme::rich();
        app.motion = Motion::Off;
        let path = app.library.row(2).unwrap().path.clone();
        app.pulses.start(path.clone(), 0);
        assert!(
            app.pulses
                .style_for(&path, 0, &app.theme, app.motion)
                .is_none()
        );
        app.motion = Motion::On;
        app.theme = Theme::mono();
        assert!(
            app.pulses
                .style_for(&path, 0, &app.theme, app.motion)
                .is_none()
        );
    }
}
