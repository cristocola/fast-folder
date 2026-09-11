//! The template guide, and the panel that explains the editor.

use crate::harness::*;
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
