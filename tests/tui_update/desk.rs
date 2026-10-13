//! The desk tab: the library grouped by who each project's next open todo
//! is assigned to.

use crate::harness::*;
use fastf::core::desk::{Assignee, Order};
use fastf::tui::app::Screen;
use fastf::tui::app::desk::DeskRow;

fn headings(app: &App) -> Vec<Option<Assignee>> {
    app.desk
        .rows
        .iter()
        .filter_map(|row| match row {
            DeskRow::Heading { group, .. } => Some(group.clone()),
            DeskRow::Card { .. } => None,
        })
        .collect()
}

/// `w` opens the desk on the library's rows: the groups in the desk's
/// order (no owner first when no hat is set, the owners by how many projects
/// each has, no open todos) and the projects not read yet under `reading`.
/// The cursor is on a card, never a heading, and the library's selection is
/// that card's project.
#[test]
fn w_opens_the_desk_grouped_by_assignee() {
    let app = desk_fixture(12, 120, 40);
    assert_eq!(app.screen, Screen::Desk);
    assert_eq!(
        headings(&app),
        [
            Some(Assignee::NoOwner),
            Some(Assignee::Owner("claude".into())),
            Some(Assignee::Owner("sales".into())),
            Some(Assignee::NoOpenTodo),
            None,
        ]
    );
    assert_eq!((app.desk.placed, app.desk.unread), (11, 1));
    let on = app.desk.selected_path().expect("a card under the cursor");
    assert_eq!(
        app.library.selected().map(|p| p.path.as_path()),
        Some(on),
        "the library's selection is the desk's row"
    );
}

/// Opening the desk asks for every record the pane's cache lacks, one
/// request per base, and once for who is working where; a second discovery
/// of the same rows asks for none of them again.
#[test]
fn the_desk_asks_once_for_every_record_it_lacks() {
    let mut app = fixture(6, 120, 40);
    let effects = press(&mut app, Key::ch('w'));
    let asked: Vec<&PathBuf> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::LoadDetails(paths) => Some(paths),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(asked.len(), 6, "every project once: {effects:?}");
    assert!(
        effects.iter().any(|e| matches!(e, Effect::LoadWorking(_))),
        "and who is working where: {effects:?}"
    );
    let again = update(
        &mut app,
        Msg::Discovered {
            generation: 2,
            projects: sample_projects(6),
        },
    );
    assert!(
        !again.iter().any(|e| matches!(e, Effect::LoadDetails(_))),
        "the reads are on their way; nothing is asked twice: {again:?}"
    );
    // The records land: the cards are placed, and nothing is pending.
    let _ = update(&mut app, Msg::Details(desk_records(&sample_projects(6))));
    assert_eq!(app.desk.placed, 5);
    assert_eq!(app.desk.pending.len(), 1, "the one record not delivered");
}

/// The cursor walks cards, headings skipped, and the pane follows it as it
/// follows the library's cursor; `<` and `>` in the pane walk the desk's
/// rows too.
#[test]
fn the_cursor_walks_the_cards_and_the_pane_follows() {
    let mut app = desk_fixture(12, 120, 40);
    let first = app.desk.selected_path().unwrap().to_path_buf();
    let effects = press(&mut app, Key::ch('j'));
    let second = app.desk.selected_path().unwrap().to_path_buf();
    assert_ne!(first, second);
    assert_eq!(app.library.selected().unwrap().path, second);
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadDetail(path) | Effect::RefreshDetail { path, .. } if *path == second
        )),
        "the pane reads the project under the desk's cursor: {effects:?}"
    );
    // Over a heading: `j` lands on the next group's first card.
    press(&mut app, Key::ch('G'));
    let last = app.desk.selected.unwrap();
    assert!(matches!(app.desk.rows[last], DeskRow::Card { .. }));
    press(&mut app, Key::ch('g'));
    assert_eq!(
        app.desk.selected,
        Some(1),
        "the first card, under the first heading"
    );
    // From the pane, `>` is the next card.
    press(&mut app, Key::plain(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail);
    press(&mut app, Key::ch('>'));
    assert_eq!(app.desk.selected_path().unwrap(), second.as_path());
}

/// `f` shows the cursor's group alone and `F` (or Esc) shows every group
/// again; `s` cycles the order within a group; the search bar narrows the
/// desk as it narrows the library.
#[test]
fn the_filters_narrow_the_desk_and_esc_takes_them_off_in_order() {
    let mut app = desk_fixture(12, 120, 40);
    // The cursor opens on the first card with no owner.
    press(&mut app, Key::ch('f'));
    assert_eq!(app.desk.filter, Some(Assignee::NoOwner));
    assert_eq!(headings(&app), [Some(Assignee::NoOwner)]);
    assert!(
        app.desk.selected_card().is_some(),
        "the cursor stays on a card"
    );

    press(&mut app, Key::ch('s'));
    assert_eq!(app.desk.order, Order::Recent);
    press(&mut app, Key::ch('S'));
    assert!(matches!(app.modals.top(), Some(Modal::Pick(_))));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::plain(KeyCode::Down));
    press(&mut app, Key::plain(KeyCode::Enter));
    assert_eq!(app.desk.order, Order::Name);

    press(&mut app, Key::ch('/'));
    type_text(&mut app, "Lullaby");
    press(&mut app, Key::plain(KeyCode::Enter));
    assert!(
        app.desk
            .rows
            .iter()
            .filter_map(DeskRow::card)
            .all(|c| c.project.name.contains("Lullaby")),
        "the query narrows the desk's rows"
    );
    // Esc's order: the query, then the group filter, then the library.
    press(&mut app, Key::plain(KeyCode::Esc));
    assert!(app.search.input.is_empty());
    assert_eq!(app.screen, Screen::Desk);
    press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.desk.filter, None);
    assert_eq!(app.screen, Screen::Desk);
    press(&mut app, Key::plain(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Library);

    // `F` is the same clearing, from the desk.
    press(&mut app, Key::ch('w'));
    press(&mut app, Key::ch('f'));
    press(&mut app, Key::ch('F'));
    assert_eq!(app.desk.filter, None);
}

/// `z` lands the cursor on a card, and the row lights.
#[test]
fn z_picks_a_card() {
    let mut app = desk_fixture(12, 120, 40);
    app.elapsed_ms = 4_321;
    press(&mut app, Key::ch('z'));
    let picked = app.desk.selected_path().unwrap().to_path_buf();
    assert!(app.desk.selected_card().is_some());
    assert!(!app.pulses.is_empty(), "the pick is lit");
    assert_eq!(app.library.selected().unwrap().path, picked);
}

/// A record landing after the cursor moved keeps the cursor on its card,
/// by path; a todo ticked in the desk's pane re-reads the card rather than
/// dropping it.
#[test]
fn a_record_landing_keeps_the_cursor_and_a_tick_rereads_the_card() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch('w'));
    let projects = sample_projects(6);
    let mut records = desk_records(&projects);
    let late = records.pop().unwrap();
    let _ = update(&mut app, Msg::Details(records));
    press(&mut app, Key::ch('j'));
    press(&mut app, Key::ch('j'));
    let on = app.desk.selected_path().unwrap().to_path_buf();
    let _ = update(&mut app, Msg::Details(vec![late]));
    assert_eq!(app.desk.selected_path().unwrap(), on.as_path());

    // The pane is off: a todo verb's `DetailOnly` is a re-read for the desk.
    app.detail_open = false;
    let effects = app.apply_change(ListChange::DetailOnly { path: on.clone() });
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDetails(paths) if paths == &vec![on.clone()])),
        "{effects:?}"
    );
    assert!(
        app.details.contains_key(&on),
        "the card stays until the read lands"
    );
}

/// A verb on the desk acts on the row under the cursor alone: the marks are
/// the library's, and the desk shows none of them.
#[test]
fn a_verb_on_the_desk_acts_on_the_cursor_alone() {
    let mut app = fixture(6, 120, 40);
    press(&mut app, Key::ch(' '));
    press(&mut app, Key::ch(' '));
    assert_eq!(app.library.marks.len(), 2);
    let _ = update(&mut app, Msg::Details(desk_records(&sample_projects(6))));
    press(&mut app, Key::ch('w'));
    press(&mut app, Key::ch('u'));
    let Some(Modal::Confirm(confirm)) = app.modals.top() else {
        panic!("u asks before unregistering");
    };
    let on = app.library.selected().unwrap().name.clone();
    assert!(confirm.prompt.contains(&on), "{}", confirm.prompt);
    assert!(
        !confirm.prompt.contains("2 "),
        "one project, not the two marks: {}",
        confirm.prompt
    );
}

/// `T` from the desk goes to the templates tab, and `w` from there to the
/// desk; the header's tabs list all three.
#[test]
fn the_tabs_lead_to_one_another() {
    let mut app = desk_fixture(6, 120, 40);
    press(&mut app, Key::ch('T'));
    assert_eq!(app.screen, Screen::Templates);
    press(&mut app, Key::ch('w'));
    assert_eq!(app.screen, Screen::Desk);
    press(&mut app, Key::ch('w'));
    assert_eq!(app.screen, Screen::Library);
    assert_eq!(Screen::ALL.len(), 3);
}

/// The session remembers the desk: the tab, the group shown alone, the
/// order, and when the desk was last open; `fastf desk` opens on it with
/// its flags.
#[test]
fn the_session_and_the_entry_bring_the_desk_back() {
    use fastf::tui::session::Session;
    let mut app = desk_fixture(6, 120, 40);
    press(&mut app, Key::ch('f'));
    press(&mut app, Key::ch('s'));
    let session = Session::capture(&app, &Session::default());
    assert_eq!(session.tab.as_deref(), Some("desk"));
    assert_eq!(session.desk_group.as_deref(), Some("no owner"));
    assert_eq!(session.desk_order.as_deref(), Some("recent"));
    assert!(session.desk_seen_at.is_some(), "the desk was looked at");

    let mut next = empty_fixture(120, 40);
    next.apply_session(&session);
    let _ = next.start();
    let _ = update(
        &mut next,
        Msg::Discovered {
            generation: 1,
            projects: sample_projects(6),
        },
    );
    assert_eq!(next.screen, Screen::Desk);
    assert_eq!(next.desk.filter, Some(Assignee::NoOwner));
    assert_eq!(next.desk.order, Order::Recent);
    assert_eq!(next.desk_seen_at, session.desk_seen_at);

    let mut entered = App::new(
        Entry::Desk {
            group: Some(Assignee::Owner("claude".into())),
            order: Some(Order::Name),
            hat: Some("sales".into()),
            random: false,
        },
        Theme::mono(),
        (120, 40),
    );
    let _ = entered.start();
    assert_eq!(entered.screen, Screen::Desk);
    assert_eq!(entered.desk.filter, Some(Assignee::Owner("claude".into())));
    assert_eq!(entered.hat().as_deref(), Some("sales"));
}

/// `L`'s journal page: every dated note and todo event across the records,
/// newest first, with the rule where the desk was last open; it fills in as
/// records land.
#[test]
fn the_journal_page_lists_what_happened_with_a_rule_where_you_last_looked() {
    use fastf::tui::app::modal::ActivityPage;
    let mut app = fixture(6, 120, 40);
    app.desk_seen_at = Some("2026-01-10T00:00:00Z".to_string());
    let effects = press(&mut app, Key::ch('L'));
    assert!(
        effects.iter().any(|e| matches!(e, Effect::LoadDetails(_))),
        "the page asks for the records it lacks: {effects:?}"
    );
    press(&mut app, Key::plain(KeyCode::Tab));
    let Some(Modal::Activity(activity)) = app.modals.top() else {
        panic!("L opens the activity screen");
    };
    assert_eq!(activity.page, ActivityPage::Journal);
    assert_eq!(activity.journal, ["reading…"]);

    let _ = update(&mut app, Msg::Details(desk_records(&sample_projects(6))));
    let Some(Modal::Activity(activity)) = app.modals.top() else {
        panic!("still open");
    };
    let rows = &activity.journal;
    assert!(
        rows[0].starts_with("2026-01-12 17:30"),
        "newest first: {rows:?}"
    );
    assert!(rows[0].contains("done "), "{rows:?}");
    let rule = rows
        .iter()
        .position(|row| row.contains("the desk was last open"))
        .expect("the rule");
    assert!(
        rows[rule - 1] > rows[rule + 1],
        "older entries read below the rule: {rows:?}"
    );
    assert!(rows[rule + 1].starts_with("2026-01-0"), "{rows:?}");
}
