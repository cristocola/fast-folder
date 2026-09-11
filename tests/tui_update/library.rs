//! The dashboard: opening, discovery, the table, search and sort, the palette,
//! patches and the detail pane.

use crate::harness::*;

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
