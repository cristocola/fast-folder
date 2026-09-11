//! The template studio and the builder.

use crate::harness::*;
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
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadTemplateView { slug } if slug == "client-project")),
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
