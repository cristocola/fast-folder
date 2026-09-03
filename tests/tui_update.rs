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
use fastf::tui::msg::{Msg, Resumed};
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

    let effects = update(
        &mut app,
        Msg::Resumed(Resumed::Legacy {
            change: ListChange::Patched {
                project: Box::new(patched),
                stale: vec![path.clone()],
            },
            quit: false,
        }),
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![path.clone()])));
    assert!(effects.contains(&Effect::LoadSummary));
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
    let effects = update(
        &mut app,
        Msg::Resumed(Resumed::Legacy {
            change: ListChange::Removed {
                path: doomed.clone(),
            },
            quit: false,
        }),
    );
    assert!(effects.contains(&Effect::ForgetSizes(vec![doomed.clone()])));
    assert_eq!(app.library.len(), 2);
    assert_eq!(app.library.selected, Some(1));
    assert!(!names(&app).iter().any(|n| doomed.ends_with(n)));
}

#[test]
fn quitting_from_the_action_menu_quits_the_app() {
    let mut app = fixture(3, 80, 24);
    let effects = update(
        &mut app,
        Msg::Resumed(Resumed::Legacy {
            change: ListChange::None,
            quit: true,
        }),
    );
    assert_eq!(effects, vec![Effect::Quit(Exit::Normal)]);
}

#[test]
fn a_patch_during_a_discovery_in_flight_asks_once_more() {
    let mut app = fixture(3, 80, 24);
    let effects = press(&mut app, Key::plain(KeyCode::F(5)));
    assert!(effects.contains(&Effect::Discover { generation: 1 }));
    assert!(effects.contains(&Effect::LoadSummary));

    let patched = app.library.selected().unwrap().clone();
    update(
        &mut app,
        Msg::Resumed(Resumed::Legacy {
            change: ListChange::Patched {
                project: Box::new(patched),
                stale: Vec::new(),
            },
            quit: false,
        }),
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
            outcome: Ok(Box::new(fastf::tui::effect::ActionOutcome {
                change: ListChange::Reload,
                message: "✓  Reindexed 3 projects across 1 base.".to_string(),
                warning: None,
                session: None,
            })),
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
            outcome: Ok(Box::new(ActionOutcome {
                change: ListChange::Patched {
                    project: Box::new(patched),
                    stale: vec![selected_path.clone()],
                },
                message: "Added 1 tag".to_string(),
                warning: None,
                session: None,
            })),
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
            outcome: Ok(Box::new(ActionOutcome {
                change: ListChange::Removed {
                    path: doomed.clone(),
                },
                message: "Deleted".to_string(),
                warning: None,
                session: None,
            })),
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
