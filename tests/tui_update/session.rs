//! The session: what a run leaves for the next one.

use crate::harness::*;

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
