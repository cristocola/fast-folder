//! Tenax, with no network: his box, the brief, the chat, and the changes he
//! proposes, driven as messages in and effects out.
//!
//! What these hold, in the person's terms: he is not there when he is off or
//! has no key; the brief is asked once, and only with a key saved in fastf;
//! nothing he proposes is written without a yes given with the question in
//! view; every call he makes is answered, however the question went away; and
//! what he says never reaches the status line or the log.

use crate::harness::*;
use fastf::core::tenax::KeySource;
use fastf::core::tenax::api::Call;
use fastf::core::tenax::turn::{Ending, Turn};
use fastf::tui::app::data::TenaxPrefs;
use fastf::tui::app::tenax::{ARM_MS, Brief, BriefRequest, ChatRequest};
use fastf::tui::command::{Availability, CommandId, find};

fn with_tenax(app: &mut App, key: KeySource) {
    app.tenax.prefs = TenaxPrefs {
        enabled: true,
        key,
        model: "claude-sonnet-5-5".to_string(),
    };
}

fn saved() -> KeySource {
    KeySource::Stored {
        last4: "abcd".to_string(),
    }
}

fn brief_of(effects: &[Effect]) -> Option<&BriefRequest> {
    effects.iter().find_map(|effect| match effect {
        Effect::TenaxBrief(request) => Some(request.as_ref()),
        _ => None,
    })
}

fn chat_of(effects: &[Effect]) -> Option<&ChatRequest> {
    effects.iter().find_map(|effect| match effect {
        Effect::TenaxChat(request) => Some(request.as_ref()),
        _ => None,
    })
}

/// Everything a request's user turn says, as one string.
fn said(request: &ChatRequest) -> String {
    serde_json::to_string(&request.user).unwrap()
}

fn done(text: &str, history_len: usize) -> Turn {
    Turn {
        history: vec![serde_json::json!({ "role": "user", "content": "x" }); history_len],
        text: text.to_string(),
        ending: Ending::Done,
        carry: Vec::new(),
        pending: Vec::new(),
        order: Vec::new(),
        calls_left: 7,
    }
}

/// A reply that calls `name` once, with `input`.
fn calls(name: &str, input: serde_json::Value) -> Turn {
    Turn {
        history: vec![
            serde_json::json!({ "role": "user", "content": "x" }),
            serde_json::json!({ "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": name, "input": input }] }),
        ],
        text: String::new(),
        ending: Ending::NeedsApp,
        carry: Vec::new(),
        pending: vec![Call {
            id: "toolu_1".to_string(),
            name: name.to_string(),
            input,
        }],
        order: vec!["toolu_1".to_string()],
        calls_left: 7,
    }
}

/// Open the chat and send `text`; the request it starts.
fn send(app: &mut App, text: &str) -> u64 {
    if !matches!(app.modals.top(), Some(Modal::Tenax(_))) {
        press(app, Key::ch('.'));
    }
    type_text(app, text);
    let effects = press(app, Key::plain(KeyCode::Enter));
    chat_of(&effects)
        .unwrap_or_else(|| panic!("a message starts a request: {effects:?}"))
        .request
}

fn turned(app: &mut App, request: u64, turn: Turn) -> Vec<Effect> {
    update(
        app,
        Msg::TenaxTurned {
            request,
            result: Box::new(Ok(turn)),
        },
    )
}

/// The selected project's id, for a call about it.
fn selected_id(app: &App) -> String {
    app.library.selected().unwrap().id.clone()
}

/// Off, he is nowhere: `.` does nothing, no box, no request, no wake.
#[test]
fn off_he_offers_nothing_and_asks_nothing() {
    let mut app = fixture(6, 120, 40);
    assert!(!app.tenax_on());
    assert!(matches!(
        (find(CommandId::Tenax).available)(&app),
        Availability::Hidden
    ));
    assert!(press(&mut app, Key::ch('.')).is_empty());
    assert!(app.modals.is_empty());
    assert!(app.regions().tenax.is_none());
    app.summary_seen = [1, 1, 1];
    let effects = update(&mut app, Msg::Tick);
    assert!(brief_of(&effects).is_none() && chat_of(&effects).is_none());
}

/// On without a key, `.` says what is missing and opens nothing.
#[test]
fn without_a_key_the_key_says_what_is_missing() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, KeySource::None);
    let availability = (find(CommandId::Tenax).available)(&app);
    assert!(
        matches!(availability, Availability::Disabled(why) if why.contains("API key")),
        "{availability:?}"
    );
    press(&mut app, Key::ch('.'));
    assert!(app.modals.is_empty());
    assert!(app.status.text.contains("API key"), "{:?}", app.status.text);
}

/// His box takes the top of the pane's column with a saved key, and with only
/// the environment's key once a conversation has started — never before.
#[test]
fn the_box_needs_a_saved_key_or_a_conversation() {
    let mut app = fixture(6, 120, 40);
    let whole = app.regions().detail.unwrap();
    with_tenax(&mut app, KeySource::Env);
    assert!(
        app.regions().tenax.is_none(),
        "an environment key alone shows no box"
    );
    send(&mut app, "hello");
    assert!(app.regions().tenax.is_some(), "a conversation shows it");

    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    let regions = app.regions();
    let (tenax, pane) = (regions.tenax.unwrap(), regions.detail.unwrap());
    assert_eq!(tenax.y, whole.y);
    assert_eq!(pane.y, tenax.y + tenax.height);
    assert_eq!(pane.height + tenax.height, whole.height);
}

/// The brief is asked once a run, with every record read first — and only
/// with a key saved in fastf: an environment key never sends a desk unasked.
#[test]
fn the_brief_is_asked_once_and_only_with_a_saved_key() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, KeySource::Env);
    app.summary_seen = [1, 1, 1];
    assert!(brief_of(&update(&mut app, Msg::Tick)).is_none());

    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    app.summary_seen = [1, 1, 1];
    let effects = update(&mut app, Msg::Tick);
    assert!(brief_of(&effects).is_none(), "records first");
    assert!(
        effects.iter().any(|e| matches!(e, Effect::LoadDetails(_))),
        "{effects:?}"
    );
    let effects = update(&mut app, Msg::Details(desk_records(&sample_projects(6))));
    // The last project is left unread by the fixture's records, so it is
    // still pending: the brief waits for it.
    assert!(brief_of(&effects).is_none());
    let last = sample_projects(6).last().unwrap().path.clone();
    let effects = update(
        &mut app,
        Msg::Details(vec![(
            last,
            fastf::tui::app::data::ProjectDetail::default(),
        )]),
    );
    let brief = brief_of(&effects).expect("the brief, once every record is in");
    assert!(brief.digest.contains("<context>"));
    assert!(
        brief.digest.contains("Projects in view: 6"),
        "{}",
        brief.digest
    );
    assert!(
        brief_of(&update(&mut app, Msg::Tick)).is_none(),
        "once a run"
    );
}

/// A brief of the same desk, younger than twelve hours, is not asked again.
#[test]
fn an_unchanged_desk_costs_no_request() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    app.summary_seen = [1, 1, 1];
    let mut records = desk_records(&sample_projects(6));
    records.push((
        sample_projects(6).last().unwrap().path.clone(),
        Default::default(),
    ));
    let _ = update(&mut app, Msg::Details(records));
    let (_, print) = app.tenax_digest(12);
    app.tenax.brief = Some(Brief {
        say: "Same as before.".to_string(),
        start: None,
        at: "2026-01-20T08:00:00Z".to_string(),
        fingerprint: print,
    });
    let effects = update(&mut app, Msg::Tick);
    assert!(brief_of(&effects).is_none(), "{effects:?}");
}

/// A brief that lands is shown and kept for the next run; an answer to an
/// older request is not.
#[test]
fn a_brief_lands_in_the_box_and_is_kept() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    let effects = update(&mut app, Msg::Key(Key::ch('.')));
    assert!(effects.is_empty());
    press(&mut app, Key::plain(KeyCode::Esc));
    let effects = app_run(&mut app, CommandId::TenaxBrief);
    let request = brief_of(&effects).unwrap().request;
    let brief = Brief {
        say: "Morning.".to_string(),
        start: None,
        at: "2026-01-20T10:00:00Z".to_string(),
        fingerprint: "f".to_string(),
    };
    let stale = update(
        &mut app,
        Msg::TenaxBriefed {
            request: request + 7,
            result: Ok(Box::new(brief.clone())),
        },
    );
    assert!(stale.is_empty() && app.tenax.brief.is_none());
    let effects = update(
        &mut app,
        Msg::TenaxBriefed {
            request,
            result: Ok(Box::new(brief)),
        },
    );
    assert_eq!(app.tenax.brief.as_ref().unwrap().say, "Morning.");
    assert!(
        effects.iter().any(|e| matches!(e, Effect::SaveTenax(_))),
        "{effects:?}"
    );
}

/// Run a palette-only command, as the palette would.
fn app_run(app: &mut App, id: CommandId) -> Vec<Effect> {
    press(app, Key::ch(':'));
    type_text(app, find(id).title);
    press(app, Key::plain(KeyCode::Enter))
}

/// A message carries the context and the words; a second one typed while
/// the first is out waits, and goes when the first answers.
#[test]
fn a_message_goes_with_its_context_and_one_at_a_time() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    press(&mut app, Key::ch('.'));
    type_text(&mut app, "where do I start");
    let effects = press(&mut app, Key::plain(KeyCode::Enter));
    let request = chat_of(&effects).unwrap();
    assert!(said(request).contains("<context>"));
    assert!(said(request).contains("where do I start"));
    assert_eq!(request.calls_left, 8);
    let first = request.request;

    type_text(&mut app, "and then");
    assert!(
        chat_of(&press(&mut app, Key::plain(KeyCode::Enter))).is_none(),
        "one at a time"
    );
    let effects = turned(&mut app, first, done("Lullaby.", 2));
    let next = chat_of(&effects).expect("the waiting message goes");
    assert!(said(next).contains("and then"));
    assert_eq!(next.history.len(), 2);
}

/// A proposed todo is written only after a yes — and not a yes pressed in the
/// moment the question appeared. Its result goes back to him.
#[test]
fn a_proposal_is_written_only_after_a_yes() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    let request = send(&mut app, "add a todo");
    let id = selected_id(&app);
    let folder = app.library.selected().unwrap().name.clone();
    let effects = turned(
        &mut app,
        request,
        calls(
            "add_todo",
            serde_json::json!({ "project": id, "folder": null, "text": "export the cover", "phase": null }),
        ),
    );
    assert!(
        chat_of(&effects).is_none(),
        "nothing goes back until he is answered"
    );
    let Some(Modal::Confirm(confirm)) = app.modals.top() else {
        panic!("a proposal is a question: {:?}", app.modals.top());
    };
    assert!(confirm.prompt.contains(&folder), "{}", confirm.prompt);
    assert!(confirm.prompt.contains("export the cover"));

    assert!(
        press(&mut app, Key::ch('y')).is_empty(),
        "a yes in the first moment is ignored"
    );
    app.elapsed_ms += ARM_MS;
    let effects = press(&mut app, Key::ch('y'));
    let Action::AddTodos { texts, project, .. } = action_of(&effects) else {
        panic!("{effects:?}");
    };
    assert_eq!(texts, &vec!["export the cover".to_string()]);
    assert_eq!(project.id, id);

    let effects = update(&mut app, item_done(run_id(&effects), ListChange::None));
    let back = chat_of(&effects).expect("the result goes back");
    let back = said(back);
    assert!(back.contains("toolu_1") && back.contains("done"), "{back}");
}

/// However the question goes away — `n`, Esc, Ctrl-C — he is told it was
/// declined, and nothing is written.
#[test]
fn every_way_out_of_a_proposal_answers_it() {
    for key in [Key::ch('n'), Key::plain(KeyCode::Esc), Key::ctrl('c')] {
        let mut app = fixture(6, 120, 40);
        with_tenax(&mut app, saved());
        let request = send(&mut app, "tag it");
        let id = selected_id(&app);
        turned(
            &mut app,
            request,
            calls(
                "add_tag",
                serde_json::json!({ "project": id, "folder": null, "tag": "client" }),
            ),
        );
        assert!(matches!(app.modals.top(), Some(Modal::Confirm(_))));
        let effects = press(&mut app, key);
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Run(..))),
            "{key:?} wrote something: {effects:?}"
        );
        let back = chat_of(&effects).unwrap_or_else(|| panic!("{key:?} left the call unanswered"));
        assert!(said(back).contains("declined"), "{key:?}: {}", said(back));
    }
}

/// A proposal that lands while a message is half typed waits for the line to
/// be empty, so no Enter meant for the line answers it.
#[test]
fn a_proposal_waits_while_the_line_has_text() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    let request = send(&mut app, "note this");
    type_text(&mut app, "hal");
    let id = selected_id(&app);
    turned(
        &mut app,
        request,
        calls(
            "add_note",
            serde_json::json!({ "project": id, "folder": null, "text": "signed off" }),
        ),
    );
    assert!(
        matches!(app.modals.top(), Some(Modal::Tenax(_))),
        "held, not asked"
    );
    for _ in 0..3 {
        press(&mut app, Key::plain(KeyCode::Backspace));
    }
    assert!(
        matches!(app.modals.top(), Some(Modal::Confirm(_))),
        "asked once the line is empty"
    );
}

/// A change the file cannot hold goes back in the rule's own words, and is
/// never put to the person.
#[test]
fn a_refused_change_goes_back_in_words() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    let request = send(&mut app, "tag it");
    let id = selected_id(&app);
    let effects = turned(
        &mut app,
        request,
        calls(
            "add_tag",
            serde_json::json!({ "project": id, "folder": null, "tag": "two words" }),
        ),
    );
    assert!(!matches!(app.modals.top(), Some(Modal::Confirm(_))));
    let back = said(chat_of(&effects).expect("the refusal goes straight back"));
    assert!(back.contains("\"is_error\":true"), "{back}");
}

/// A new chat lets go of what was in flight: its answer is not shown.
#[test]
fn a_new_chat_lets_go_of_the_old_one() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    let request = send(&mut app, "hello");
    press(&mut app, Key::ctrl('n'));
    assert!(app.tenax.transcript.is_empty());
    turned(&mut app, request, done("too late", 2));
    assert!(app.tenax.transcript.is_empty());
    assert!(app.tenax.history.is_empty());
}

/// What he says stays in his box and his chat.
#[test]
fn what_he_says_never_reaches_the_status_line_or_the_log() {
    let mut app = fixture(6, 80, 24);
    with_tenax(&mut app, KeySource::Env);
    let request = send(&mut app, "hello");
    press(&mut app, Key::plain(KeyCode::Esc));
    turned(&mut app, request, done("PRIVATE-PHRASE", 2));
    assert!(!app.status.text.contains("PRIVATE-PHRASE"));
    assert!(
        app.status.text.contains("Tenax answered"),
        "{:?}",
        app.status.text
    );
    assert!(
        app.log
            .iter()
            .all(|entry| !entry.text.contains("PRIVATE-PHRASE"))
    );
    assert!(
        app.outbox
            .iter()
            .all(|m| !format!("{m:?}").contains("PRIVATE-PHRASE"))
    );
}

/// Turned off with a call waiting, he is told it was not run, and nothing is
/// asked of the API.
#[test]
fn turning_him_off_answers_what_was_open() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    let request = send(&mut app, "note this");
    type_text(&mut app, "x");
    let id = selected_id(&app);
    turned(
        &mut app,
        request,
        calls(
            "add_note",
            serde_json::json!({ "project": id, "folder": null, "text": "n" }),
        ),
    );
    app.tenax.prefs.enabled = false;
    let effects = update(&mut app, Msg::Tick);
    assert!(chat_of(&effects).is_none());
    assert!(app.tenax.held.is_none());
    assert!(
        serde_json::to_string(&app.tenax.carry)
            .unwrap()
            .contains("toolu_1"),
        "the call is answered, for whenever he is back"
    );
}

/// He moves only while he thinks or has just spoken; at rest, and on a theme
/// with no colour, he asks for no wake — except the spinner while a request
/// is out, which says something is pending whatever the motion setting.
#[test]
fn he_moves_only_when_he_speaks() {
    let mut app = fixture(6, 120, 40);
    with_tenax(&mut app, saved());
    assert_eq!(app.tenax_tick(), None, "at rest");
    let request = send(&mut app, "hello");
    assert!(app.tenax_tick().is_some(), "the spinner while he thinks");
    assert!(app.tick_interval().is_some());
    turned(&mut app, request, done("Hi.", 2));
    assert_eq!(app.tenax_tick(), None, "mono: no animation");

    let mut app = fixture(6, 120, 40);
    app.theme = Theme::doom_one();
    app.motion = fastf::tui::motion::Motion::On;
    with_tenax(&mut app, saved());
    let request = send(&mut app, "hello");
    turned(&mut app, request, done("Hi.", 2));
    assert!(app.tenax_tick().is_some(), "he plays when he speaks");
    app.elapsed_ms += fastf::tui::sprite::PLAY_MS;
    assert_eq!(app.tenax_tick(), None, "and lets go");
    app.motion = fastf::tui::motion::Motion::Off;
    let request = send(&mut app, "again");
    turned(&mut app, request, done("Hi.", 4));
    assert_eq!(app.tenax_tick(), None, "motion off: a still frame");
}
