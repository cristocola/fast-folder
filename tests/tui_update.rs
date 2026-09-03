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
        matches!(effects.first(), Some(Effect::RequestSizes(paths)) if paths.len() == 3),
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

#[test]
fn arrows_wrap_and_page_keys_clamp() {
    let mut app = fixture(12, 80, 24);
    assert_eq!(app.library.selected, Some(0));
    press(&mut app, Key::plain(KeyCode::Up));
    assert_eq!(app.library.selected, Some(11), "up from the top wraps");
    press(&mut app, Key::ch('j'));
    assert_eq!(app.library.selected, Some(0), "down from the bottom wraps");
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
    let id = run_id(&press(&mut app, Key::ch('R')));
    update(
        &mut app,
        item_done(
            id,
            ListChange::Patched {
                project: Box::new(patched),
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
    for _ in 0..31 {
        update(&mut app, Msg::Tick);
    }
    assert!(app.status.text.is_empty());
}

#[test]
fn the_summary_fills_the_template_strip_and_a_strip_filter_toggles() {
    let mut app = fixture(12, 120, 40);
    assert_eq!(app.templates.cards.len(), 3);
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Detail);
    press(&mut app, Key::plain(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Templates);
    let slug = app.templates.selected_card().unwrap().slug.clone();
    press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(app.library.template_filter.as_deref(), Some(slug.as_str()));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        app.library.template_filter.is_none(),
        "the same card again clears it"
    );
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

    let mut narrow = fixture(12, 80, 24);
    let effects = press(&mut narrow, Key::ch('j'));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::LoadDetail(_))),
        "no pane on screen, no read: {effects:?}"
    );
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

// --- Phase 1: single-project actions -------------------------------------

/// The single `Effect::Run` an action key produces.
fn action_of(effects: &[Effect]) -> &Action {
    match effects {
        [Effect::Run(_, action)] => action,
        other => panic!("expected one action, got {other:?}"),
    }
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
    assert!(matches!(app.modals.top(), Some(Modal::TextPrompt(_))));
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

    // Delete with the exact name to confirm.
    press(&mut app, Key::ch('D'));
    type_text(&mut app, &name);
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
fn a_typed_confirm_mismatch_deletes_nothing() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch('D'));
    type_text(&mut app, "not the name");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    assert!(effects.is_empty(), "nothing runs: {effects:?}");
    assert!(app.modals.is_empty());
    assert!(
        app.status
            .text
            .contains("name did not match — nothing deleted"),
        "{}",
        app.status.text
    );
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
// Marks: what a batch verb will act on (Phase 2)
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
// Batch jobs over the marks (Phase 2)
// ---------------------------------------------------------------------------

/// The id an `Effect::Run` carries, when the effects are exactly one run.
fn run_id(effects: &[Effect]) -> fastf::tui::effect::ActionId {
    match effects {
        [Effect::Run(id, _)] => *id,
        other => panic!("expected one run, got {other:?}"),
    }
}

fn item_done(id: fastf::tui::effect::ActionId, change: ListChange) -> Msg {
    Msg::ActionDone {
        id,
        outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome::new(
            change, "done",
        ))),
    }
}

#[test]
fn delete_over_marks_confirms_once_then_runs_each_item() {
    let mut app = fixture(12, 80, 24);
    press(&mut app, Key::ch(' ')); // mark row 0
    press(&mut app, Key::ch(' ')); // mark row 1
    let first = app.library.row(0).unwrap().path.clone();
    let second = app.library.row(1).unwrap().path.clone();

    // `D` over marks is a yes/no confirm, not the typed-name guard.
    press(&mut app, Key::ch('D'));
    assert!(matches!(app.modals.top(), Some(Modal::Confirm(_))));
    let effects = press(&mut app, Key::ch('y'));
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
    assert!(effects.is_empty());
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
    let effects = press(&mut app, Key::ch('y'));
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
    let effects = press(&mut app, Key::ch('y'));
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
    assert!(effects.is_empty(), "no further item may start: {effects:?}");
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
// The flows: create, apply, register (Phase 3)
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
// The template studio and the builder (Phase 4)
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
    fn the_studio_lists_the_templates_and_reads_the_selected_one() {
        let mut app = fixture(6, 120, 40);
        let effects = press(&mut app, Key::ch('T'));
        match app.modals.top() {
            Some(Modal::Studio(studio)) => assert_eq!(studio.cards.len(), 3),
            other => panic!("expected the studio, got {other:?}"),
        }
        assert!(
            matches!(&effects[..], [Effect::LoadTemplateView { slug }] if slug == "general"),
            "{effects:?}"
        );
        let effects = press(&mut app, Key::plain(KeyCode::Down));
        assert!(
            matches!(&effects[..], [Effect::LoadTemplateView { slug }] if slug == "music-video"),
            "moving reads the next one: {effects:?}"
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
        match app.modals.top() {
            Some(Modal::Studio(studio)) => assert!(studio.lines.is_empty()),
            other => panic!("{other:?}"),
        }
        let _ = update(
            &mut app,
            Msg::TemplateViewLoaded {
                slug: "general".to_string(),
                lines: vec!["General".to_string()],
            },
        );
        match app.modals.top() {
            Some(Modal::Studio(studio)) => assert_eq!(studio.lines, vec!["General".to_string()]),
            other => panic!("{other:?}"),
        }
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
        assert!(
            matches!(app.modals.top(), Some(Modal::Studio(_))),
            "the builder closed onto the studio it came from"
        );
    }

    #[test]
    fn editing_reads_the_template_and_remembers_what_it_was_called() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        let effects = press(&mut app, Key::ch('e'));
        assert!(
            matches!(&effects[..], [Effect::LoadTemplateSource { slug }] if slug == "general"),
            "{effects:?}"
        );
        assert!(builder(&app).pending, "the screen says it is reading");

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
        assert!(!builder(&app).pending);
        assert_eq!(builder(&app).original_slug.as_deref(), Some("general"));
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
        assert_eq!(builder(&app).summary(Section::Variables), "1  (artist)");
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
        assert_eq!(builder(&app).summary(Section::Structure), "2 folders");
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

    #[test]
    fn deleting_a_template_asks_and_then_runs() {
        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('D'));
        match app.modals.top() {
            Some(Modal::Confirm(confirm)) => {
                assert!(confirm.prompt.contains("general"), "{}", confirm.prompt)
            }
            other => panic!("expected a confirm, got {other:?}"),
        }
        let effects = press(&mut app, Key::ch('n'));
        assert!(effects.is_empty(), "no is no: {effects:?}");

        press(&mut app, Key::ch('D'));
        let effects = press(&mut app, Key::ch('y'));
        assert!(matches!(action_of(&effects), Action::DeleteTemplate(slug) if slug == "general"));
    }

    #[test]
    fn from_folder_previews_before_it_writes() {
        use fastf::tui::app::wizard::{FIELD_SLUG, FIELD_SOURCE, FlowKind};

        let mut app = fixture(6, 120, 40);
        press(&mut app, Key::ch('T'));
        press(&mut app, Key::ch('g'));
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
// Settings, the counter, maintenance and the first run (Phase 5)
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
// The mouse (Phase 7)
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

        let strip = regions.strip.expect("40 rows has the strip");
        click(&mut app, strip.x + 2, strip.y + 1);
        assert_eq!(app.focus, Focus::Templates);

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

        // In the detail pane it scrolls the pane, because that is what ↓ does
        // there.
        press(&mut app, Key::plain(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Detail);
        wheel(&mut app, true);
        assert_eq!(app.detail_scroll, 3);
        assert_eq!(app.library.selected, Some(0), "the list did not move");
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
