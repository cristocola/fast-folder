//! Motion, which only ever appears where a still frame could not answer a
//! question: what changed, where the focus went, where a row went, what is
//! working, what is going away — and which fades rather than flashes.

use crate::harness::*;
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
        mid != app.theme.pulse && mid != app.theme.ground && mid != ratatui::style::Color::Reset,
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
    let title =
        |buf: &ratatui::buffer::Buffer, rect: ratatui::layout::Rect| buf[(rect.x + 2, rect.y)].fg;
    let edge =
        |buf: &ratatui::buffer::Buffer, rect: ratatui::layout::Rect| buf[(rect.x, rect.y + 1)].fg;
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
