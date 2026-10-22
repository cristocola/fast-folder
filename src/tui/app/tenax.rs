//! Tenax in the app: his box, his chat, the brief, and the changes he proposes.
//!
//! The protocol — the request, the loop over his read tools, the key — is
//! `core::tenax`; this is what the app holds and decides, with no I/O, as
//! everything under `app/` is. Every request is an `Effect` the runtime
//! carries out on a worker, and every answer a `Msg` that names its request.
//!
//! **Every call he makes is answered once.** A reply's calls are answered on
//! the worker (reads), here at once (`select_project`, `filter_library`), or
//! by the person (writes, through a `Confirm`); all of their results go back
//! in one user message, and a question that went away unanswered — a confirm
//! dismissed any way at all, a new chat, Tenax turned off — is answered
//! "declined" by `tenax_settle`, which runs after every message. A call left
//! without a result is a 400 on the next request.
//!
//! **What he says stays in his box and his chat.** It never reaches the
//! status line, the session's log or `messages.log`: a status line about him
//! says only that he answered.

use std::collections::VecDeque;
use std::path::PathBuf;

use serde_json::Value;

use super::*;
use crate::core::library::Project;
use crate::core::tenax::api::Call;
use crate::core::tenax::tools::{self as tenax_tools, Kind as ToolKind};
use crate::core::tenax::turn::{self as tenax_turn, Ending, Failed, Outcome, Turn};
use crate::core::tenax::{ApiError, Ask};
use crate::tui::sprite::{self, Animation};
use crate::tui::widgets::input::LineEdit;

/// Requests a person's message may cost, read tools included.
pub const CALLS_PER_MESSAGE: u32 = 8;
/// A brief older than this is asked for again even when the desk is the same.
const BRIEF_STALE_HOURS: i64 = 12;
/// How long a proposal's confirmation ignores a yes: a key pressed as it
/// appeared was meant for what was there before.
pub const ARM_MS: u64 = 500;
/// The most cards a group lists in a digest; the rest are counted.
const CARDS_PER_GROUP: usize = 12;

/// The last brief: what he said, where he said to start, when, and the desk
/// it was about.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Brief {
    pub say: String,
    #[serde(default)]
    pub start: Option<Start>,
    /// When it was written, as a timestamp.
    pub at: String,
    /// The desk it was about (`digest`'s fingerprint).
    pub fingerprint: String,
}

/// Where the brief said to start.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Start {
    pub project: String,
    pub folder: String,
    pub todo: String,
}

/// One entry of the chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    You(String),
    Tenax(String),
    /// A change he proposed, and what came of it.
    Done(String),
    Declined(String),
    /// What went wrong, in one line.
    Trouble(String),
}

/// What he is doing, for the box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    #[default]
    Idle,
    /// The last request failed, with this sentence.
    Trouble(String),
}

/// A change he proposed, put to the person. Typed fields only:
/// `ConfirmThen` is `Eq`, and the conversation stays in `TenaxState`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub call_id: String,
    pub project: Box<Project>,
    pub change: Change,
    /// Before this moment (`elapsed_ms`) a yes is ignored.
    pub armed_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    AddTodo {
        text: String,
        phase: Option<String>,
    },
    Toggle {
        ordinal: usize,
        was: String,
    },
    Owner {
        ordinal: usize,
        was: String,
        owner: Option<String>,
    },
    Note(String),
    Tag(String),
    Describe(String),
}

impl Proposal {
    /// The question, naming the folder it is about.
    pub fn question(&self) -> String {
        let folder = &self.project.name;
        match &self.change {
            Change::AddTodo {
                text,
                phase: Some(phase),
            } => {
                format!("Tenax would add the todo \"{text}\" under {phase} in {folder}. Add it?")
            }
            Change::AddTodo { text, phase: None } => {
                format!("Tenax would add the todo \"{text}\" to {folder}. Add it?")
            }
            Change::Toggle { was, .. } => {
                format!(
                    "Tenax would tick \"{was}\" in {folder} (or open it again if it is done). Do it?"
                )
            }
            Change::Owner {
                was,
                owner: Some(owner),
                ..
            } => {
                format!("Tenax would give \"{was}\" in {folder} to @{owner}. Do it?")
            }
            Change::Owner {
                was, owner: None, ..
            } => {
                format!("Tenax would give \"{was}\" in {folder} to nobody. Do it?")
            }
            Change::Note(text) => {
                format!("Tenax would add a note to {folder}: \"{text}\". Add it?")
            }
            Change::Tag(tag) => format!("Tenax would tag {folder} {tag}. Tag it?"),
            Change::Describe(text) if text.is_empty() => {
                format!("Tenax would clear the description of {folder}. Clear it?")
            }
            Change::Describe(text) => {
                format!("Tenax would describe {folder} as \"{text}\". Do it?")
            }
        }
    }

    /// What the transcript says once it is done.
    fn done(&self) -> String {
        let id = &self.project.id;
        match &self.change {
            Change::AddTodo { text, .. } => format!("added \"{text}\" to {id}"),
            Change::Toggle { was, .. } => format!("ticked \"{was}\" in {id}"),
            Change::Owner { was, owner, .. } => match owner {
                Some(owner) => format!("gave \"{was}\" in {id} to @{owner}"),
                None => format!("gave \"{was}\" in {id} to nobody"),
            },
            Change::Note(_) => format!("added a note to {id}"),
            Change::Tag(tag) => format!("tagged {id} {tag}"),
            Change::Describe(_) => format!("described {id}"),
        }
    }

    fn busy(&self) -> &'static str {
        "writing…"
    }

    fn action(&self) -> Action {
        let project = self.project.clone();
        match self.change.clone() {
            Change::AddTodo { text, phase } => Action::AddTodos {
                project,
                texts: vec![text],
                place: match phase {
                    Some(phase) => crate::core::body::TodoPlace::Phase(phase),
                    None => crate::core::body::TodoPlace::End,
                },
            },
            Change::Toggle { ordinal, was } => Action::ToggleTodo {
                project,
                ordinal,
                was,
            },
            Change::Owner {
                ordinal,
                was,
                owner,
            } => Action::SetTodoOwner {
                project,
                ordinal,
                was,
                owner,
            },
            Change::Note(text) => Action::AppendNote { project, text },
            Change::Tag(tag) => Action::AddTag { project, tag },
            Change::Describe(text) => Action::SetDescription { project, text },
        }
    }
}

/// The chat dialog: its line and where its transcript is scrolled to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chat {
    pub input: LineEdit,
    pub scroll: usize,
    /// Follow the newest line while the transcript grows, until scrolled up.
    pub follow: bool,
}

impl Chat {
    pub fn new() -> Self {
        Self {
            input: LineEdit::default(),
            scroll: 0,
            follow: true,
        }
    }
}

/// One project as the read tools see it — what the worker answers from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRef {
    pub id: String,
    pub folder: String,
    pub base: String,
    pub path: PathBuf,
    pub template: String,
    pub tags: Vec<String>,
    pub description: String,
}

/// What a request's read tools answer from, built here with no I/O.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub projects: Vec<ProjectRef>,
    /// The desk as `desk` returns it.
    pub desk: String,
    /// What needs attention, as `attention` returns it.
    pub attention: String,
}

/// One turn of the chat, for the worker.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatRequest {
    pub request: u64,
    pub model: String,
    pub history: Vec<Value>,
    pub user: Vec<Value>,
    pub calls_left: u32,
    pub snapshot: Snapshot,
}

/// The brief, for the worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BriefRequest {
    pub request: u64,
    pub model: String,
    pub ask: Ask,
    pub digest: String,
    pub fingerprint: String,
    pub at: String,
}

/// Everything the app holds about Tenax.
#[derive(Debug, Default)]
pub struct TenaxState {
    pub prefs: crate::tui::app::data::TenaxPrefs,
    pub brief: Option<Brief>,
    /// The automatic brief was asked for this run (it is asked once).
    pub briefed: bool,
    /// The records were asked for, for the brief.
    pub reading: bool,
    pub brief_request: Option<u64>,
    pub history: Vec<Value>,
    pub transcript: Vec<Said>,
    pub chat_request: Option<u64>,
    pub calls_left: u32,
    /// Results the API is owed, to lead the next user message.
    pub carry: Vec<Value>,
    /// What was typed while a request was in flight.
    pub queued: Vec<String>,
    /// The last reply's calls still to be answered here, in order.
    pub pending: VecDeque<Call>,
    /// The last reply's call ids, in the order it made them.
    pub order: Vec<String>,
    /// Results decided for the last reply's calls.
    pub answered: Vec<Value>,
    /// A proposal on screen as a `Confirm`, by its call id.
    pub asking: Option<String>,
    /// A proposal waiting for the chat to be open and its line empty.
    pub held: Option<Proposal>,
    /// The action carrying out a proposal, and the proposal.
    pub running: Option<(ActionId, Proposal)>,
    /// A conversation was started this run: his box shows even with only the
    /// environment's key.
    pub talked: bool,
    pub animation: Option<(Animation, u64)>,
    pub mood: Mood,
    next_request: u64,
    /// Feeds the animation's seed, so two answers in one millisecond differ.
    spoke: u64,
}

impl TenaxState {
    fn next(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    /// Whether a request of his is on its way.
    pub fn thinking(&self) -> bool {
        self.chat_request.is_some() || self.brief_request.is_some()
    }

    /// The newest thing he said in the chat, if he said anything this run.
    pub fn last_said(&self) -> Option<&str> {
        self.transcript.iter().rev().find_map(|said| match said {
            Said::Tenax(text) => Some(text.as_str()),
            _ => None,
        })
    }
}

/// The sentence a failed request leaves in his box and the chat.
pub fn trouble(error: &ApiError, model: &str) -> String {
    let settings = command::key_of(CommandId::Settings);
    match error {
        ApiError::NoKey => format!("I need an API key. Settings ({settings}), Tenax, API key."),
        ApiError::Key => format!("That key was turned down. Check it in settings ({settings})."),
        ApiError::Billing => "The account says no: billing. Not my department.".to_string(),
        ApiError::Permission(_) => "This key isn't allowed to do that.".to_string(),
        ApiError::NoModel(_) => {
            format!("I can't find {model}. Check tenax-model in settings ({settings}).")
        }
        ApiError::Refused(why) => format!("The API refused the request: {why}"),
        ApiError::TooLarge => format!(
            "That was too much for one request. {} starts a new chat.",
            command::key_of(CommandId::TenaxNewChat)
        ),
        ApiError::Busy { .. } | ApiError::Overloaded | ApiError::Server(_) => {
            "Anthropic's busy. Ask me again in a minute.".to_string()
        }
        ApiError::Network(_) => "Can't reach Anthropic from here.".to_string(),
        ApiError::Garbled(_) => "I got an answer I couldn't read.".to_string(),
    }
}

/// Text from the model, made safe to draw: no control characters (an ESC
/// would be a terminal escape), and under the ASCII alphabet no typographic
/// punctuation a legacy console cannot show.
pub fn sanitize(text: &str, ascii: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push('\n'),
            '\t' => out.push(' '),
            c if c.is_control() => {}
            c if ascii && !c.is_ascii() => out.push_str(match c {
                '\u{2018}' | '\u{2019}' => "'",
                '\u{201C}' | '\u{201D}' => "\"",
                '\u{2013}' | '\u{2014}' => "-",
                '\u{2026}' => "...",
                '\u{2022}' | '\u{25B8}' => "*",
                '\u{00A0}' => " ",
                _ => "?",
            }),
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// A stable hash of `text` (FNV-1a), for the brief's cache: the same desk
/// reads the same on every toolchain, which `DefaultHasher` does not promise.
pub fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The brief's answer, as the API is asked to shape it.
pub fn brief_schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::json!({
            "type": "object",
            "properties": {
                "say": { "type": "string", "description": "One or two sentences, at most 140 characters, in your voice." },
                "start_project": { "type": "string", "description": "The id of the project to start with, or an empty string." },
                "start_folder": { "type": "string", "description": "That project's folder name, or an empty string." },
                "start_todo": { "type": "string", "description": "The todo to start with, or an empty string." },
            },
            "required": ["say", "start_project", "start_folder", "start_todo"],
            "additionalProperties": false,
        })
    })
}

/// What the brief asks, after the digest.
pub const BRIEF_ASK: &str = "Write the brief for the box beside the project list. say: one or two sentences, \
at most 140 characters, in your voice: what is on the desk and where to start. start_project, start_folder and \
start_todo name the one project and todo to start with (prefer the person's own, then something stuck, then \
what has no owner), or are empty strings when there is nothing to start.";

impl App {
    /// Whether Tenax is switched on at all.
    pub fn tenax_on(&self) -> bool {
        self.tenax.prefs.enabled
    }

    /// Whether his box is wanted above the pane: on, on a tab that lists
    /// projects, and with a key saved — or a conversation started this run.
    /// `layout::regions` decides whether there is room.
    pub fn tenax_wanted(&self) -> bool {
        self.tenax_on()
            && self.screen.lists_projects()
            && (self.tenax.prefs.key.stored() || (self.tenax.talked && self.tenax.prefs.key.any()))
    }

    /// Whether the chat is open, anywhere in the stack.
    pub fn tenax_chat_open(&self) -> bool {
        self.modals
            .iter()
            .any(|modal| matches!(modal, Modal::Tenax(_)))
    }

    /// The wake his face and his spinner ask for, if any.
    pub fn tenax_tick(&self) -> Option<u64> {
        if !self.tenax_on() {
            return None;
        }
        if self.tenax.thinking() {
            return Some(SLOW_FRAME_MS);
        }
        let moving = self.motion.is_on() && self.theme.kind != crate::tui::theme::ThemeKind::Mono;
        match self.tenax.animation {
            Some((animation, started))
                if moving && sprite::playing(animation, started, self.elapsed_ms) =>
            {
                Some(SLOW_FRAME_MS)
            }
            _ => None,
        }
    }

    /// The frame his face shows now.
    pub fn tenax_frame(&self) -> sprite::Frame {
        let ascii = self.theme.glyphs.is_ascii();
        let moving = self.motion.is_on() && self.theme.kind != crate::tui::theme::ThemeKind::Mono;
        if !moving {
            return sprite::rest(ascii);
        }
        if self.tenax.thinking() {
            return sprite::frame(Animation::Think, 0, self.elapsed_ms, ascii);
        }
        match self.tenax.animation {
            Some((animation, started)) => sprite::frame(animation, started, self.elapsed_ms, ascii),
            None => sprite::rest(ascii),
        }
    }

    /// He spoke: one animation, chosen by what the projects say.
    fn tenax_speaks(&mut self) {
        self.tenax.spoke += 1;
        let seed = self.elapsed_ms ^ self.tenax.spoke.rotate_left(32);
        let animation = sprite::choose(&self.tenax_facts(), seed);
        self.tenax.animation = Some((animation, self.elapsed_ms));
    }

    /// The facts his animation is chosen by, from what the app holds.
    pub fn tenax_facts(&self) -> sprite::Facts {
        let now = (self.now)();
        let today = now.get(..10).unwrap_or_default().to_string();
        let mut facts = sprite::Facts {
            all_quiet: true,
            ..sprite::Facts::default()
        };
        let mut seen = 0;
        for project in self.tenax_projects() {
            let Some(detail) = self.details.get(&project.path) else {
                continue;
            };
            seen += 1;
            let working = self
                .working
                .get(&project.path)
                .is_some_and(|w| !w.is_empty());
            if working {
                facts.working += 1;
            }
            let quiet_since = crate::tui::app::desk::quiet_since(detail.touched.as_deref(), None);
            let quiet =
                |days| crate::core::desk::quiet_for_days(quiet_since.as_deref(), &now, days);
            if !quiet(7) {
                facts.all_quiet = false;
            }
            for todo in &detail.todos {
                if todo.done {
                    if todo
                        .done_at
                        .as_deref()
                        .is_some_and(|at| at.starts_with(&today))
                    {
                        facts.done_today += 1;
                    }
                } else {
                    facts.open_todos += 1;
                }
            }
            let card =
                crate::core::desk::Card::of((*project).clone(), &detail.todos, Vec::new(), None);
            if matches!(card.assignee(), crate::core::desk::Assignee::Owner(_))
                && !working
                && quiet(crate::core::desk::STUCK_AFTER_DAYS)
            {
                facts.stuck += 1;
            }
        }
        if seen == 0 {
            facts.all_quiet = false;
        }
        facts.needs_you = self
            .summary
            .as_ref()
            .map(|s| s.attention.needs_you())
            .unwrap_or(0);
        facts
    }

    /// The projects Tenax reads: the library in the bases' view, whatever the
    /// search bar holds, so typing a query never changes what he knows.
    fn tenax_projects(&self) -> impl Iterator<Item = &Project> {
        self.library
            .snapshot
            .iter()
            .filter(|project| !self.library.out_of_view(project))
    }

    /// The desk as words, and its fingerprint: the same rows the desk tab
    /// groups, each card a line. The fingerprint leaves out what moves with
    /// the clock — an age — so an unchanged desk is the same desk tomorrow.
    pub fn tenax_digest(&self, cards_per_group: usize) -> (String, String) {
        let now = (self.now)();
        let hat = self.hat();
        let mut cards = Vec::new();
        let mut unread = 0;
        for project in self.tenax_projects() {
            let active = self.working.get(&project.path).cloned().unwrap_or_default();
            match self.details.get(&project.path) {
                Some(detail) => {
                    let newest = self
                        .library
                        .sizes
                        .get(&project.path)
                        .and_then(|facts| facts.as_ref())
                        .and_then(|facts| facts.newest_stamp());
                    let quiet =
                        crate::tui::app::desk::quiet_since(detail.touched.as_deref(), newest);
                    cards.push(crate::core::desk::Card::of(
                        project.clone(),
                        &detail.todos,
                        active,
                        quiet,
                    ));
                }
                None => unread += 1,
            }
        }
        let total = cards.len() + unread;
        let groups =
            crate::core::desk::group(cards, hat.as_deref(), crate::core::desk::Order::Quiet);
        let mut text = String::new();
        let mut stable = String::new();
        text.push_str(&format!("Projects in view: {total}"));
        if unread > 0 {
            text.push_str(&format!(" ({unread} not read yet)"));
        }
        text.push('\n');
        for group in &groups {
            let label = group.assignee.label(hat.as_deref());
            text.push_str(&format!("\n{label} ({}):\n", group.cards.len()));
            stable.push_str(&format!("[{label}]"));
            if group.assignee == crate::core::desk::Assignee::NoOpenTodo {
                // Nothing to do there; the names are enough, and not all of them.
                let names: Vec<String> = group
                    .cards
                    .iter()
                    .take(cards_per_group)
                    .map(|card| format!("{} {}", card.project.id, card.project.name))
                    .collect();
                text.push_str(&format!("- {}", names.join("; ")));
                if group.cards.len() > cards_per_group {
                    text.push_str(&format!(
                        "; and {} more",
                        group.cards.len() - cards_per_group
                    ));
                }
                text.push('\n');
                stable.push_str(&group.cards.len().to_string());
                continue;
            }
            for card in group.cards.iter().take(cards_per_group) {
                let next = card.next_todo_line(" / ");
                let age = card
                    .quiet_since
                    .as_deref()
                    .map(|since| crate::core::desk::age_label(since, &now))
                    .unwrap_or_else(|| "unknown".to_string());
                let stuck = matches!(card.assignee(), crate::core::desk::Assignee::Owner(_))
                    && card.active.is_empty()
                    && crate::core::desk::quiet_for_days(
                        card.quiet_since.as_deref(),
                        &now,
                        crate::core::desk::STUCK_AFTER_DAYS,
                    );
                let mut line = format!(
                    "- {} {} | next: {} | {} open, {} done | quiet {}",
                    card.project.id, card.project.name, next, card.open, card.done, age
                );
                if !card.active.is_empty() {
                    line.push_str(&format!(" | being worked on ({})", card.active.join(", ")));
                }
                if stuck {
                    line.push_str(" | stuck");
                }
                text.push_str(&line);
                text.push('\n');
                stable.push_str(&format!(
                    "{}|{}|{}|{}|{stuck};",
                    card.project.id, next, card.open, card.done
                ));
            }
            if group.cards.len() > cards_per_group {
                text.push_str(&format!(
                    "- and {} more\n",
                    group.cards.len() - cards_per_group
                ));
            }
        }
        let attention = self.tenax_attention();
        if !attention.is_empty() {
            text.push_str(&format!("\nUnfinished work:\n{attention}\n"));
            stable.push_str(&attention);
        }
        let print = fingerprint(&format!("{}|{stable}", hat.unwrap_or_default()));
        (text, print)
    }

    /// What needs attention, one line an item; empty when nothing does.
    fn tenax_attention(&self) -> String {
        let Some(summary) = &self.summary else {
            return String::new();
        };
        summary
            .attention
            .items
            .iter()
            .map(|item| {
                let who = match item.state {
                    crate::core::attention::State::NeedsYou => "needs the person",
                    crate::core::attention::State::Waiting => "waiting",
                    crate::core::attention::State::Auto => "fastf will finish it",
                };
                format!("- {} ({who}): {}", item.what, item.reason)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The context block that opens each of the person's messages: what is
    /// true now and moves, which is why it is never in the system prompt.
    fn tenax_context(&self) -> String {
        let now = (self.now)();
        let hat = match self.hat() {
            Some(hat) => format!(
                "The person's hat is \"{hat}\": their own todos are the ones owned by @{hat}."
            ),
            None => "No hat is set: the person's own todos are the ones with no owner.".to_string(),
        };
        let selected = match self.library.selected() {
            Some(project) => format!("{} {}", project.id, project.name),
            None => "nothing".to_string(),
        };
        let tab = self.screen.label();
        format!(
            "<context>\nNow: {now}\n{hat}\nSelected in the app: {selected} (on the {tab} tab).\nProjects in view: {}.\n</context>",
            self.tenax_projects().count()
        )
    }

    fn tenax_snapshot(&self) -> Snapshot {
        let projects = self
            .tenax_projects()
            .map(|project| ProjectRef {
                id: project.id.clone(),
                folder: project.name.clone(),
                base: crate::core::library::base_label(&project.base),
                path: project.path.clone(),
                template: project.template.clone(),
                tags: project.tags.clone(),
                description: project.description.clone(),
            })
            .collect();
        let (desk, _) = self.tenax_digest(40);
        let attention = self.tenax_attention();
        Snapshot {
            projects,
            desk,
            attention: if attention.is_empty() {
                "Nothing is unfinished.".to_string()
            } else {
                attention
            },
        }
    }

    // --- the chat ---------------------------------------------------------

    /// `.`: the chat, with a held proposal put to the person at once.
    pub(super) fn open_tenax(&mut self) -> Vec<Effect> {
        if !self.tenax_chat_open() {
            self.modals.push(Modal::Tenax(Box::new(Chat::new())));
        }
        self.tenax_ask_held();
        Vec::new()
    }

    /// Enter in the chat.
    pub(super) fn tenax_send(&mut self) -> Vec<Effect> {
        let Some(Modal::Tenax(chat)) = self.modals.top_mut() else {
            return Vec::new();
        };
        let text = chat.input.text().trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        chat.input.clear();
        chat.follow = true;
        self.tenax.transcript.push(Said::You(text.clone()));
        self.tenax.talked = true;
        self.tenax.mood = Mood::Idle;
        if self.tenax.chat_request.is_some() || self.tenax_answering() {
            self.tenax.queued.push(text);
            return Vec::new();
        }
        let mut blocks = std::mem::take(&mut self.tenax.carry);
        blocks.push(tenax_turn::text_block(&format!(
            "{}\n\n{text}",
            self.tenax_context()
        )));
        self.tenax.calls_left = CALLS_PER_MESSAGE;
        self.tenax_request(blocks)
    }

    /// Whether the last reply's calls are still being answered here.
    fn tenax_answering(&self) -> bool {
        !self.tenax.order.is_empty()
    }

    fn tenax_request(&mut self, user: Vec<Value>) -> Vec<Effect> {
        let request = self.tenax.next();
        self.tenax.chat_request = Some(request);
        vec![Effect::TenaxChat(Box::new(ChatRequest {
            request,
            model: self.tenax.prefs.model.clone(),
            history: self.tenax.history.clone(),
            user,
            calls_left: self.tenax.calls_left,
            snapshot: self.tenax_snapshot(),
        }))]
    }

    /// Ctrl-n: a new conversation. Whatever was in flight is let go, and every
    /// question still open is withdrawn.
    pub(super) fn tenax_new_chat(&mut self) -> Vec<Effect> {
        self.tenax_forget_conversation();
        if let Some(Modal::Tenax(chat)) = self.modals.top_mut() {
            **chat = Chat::new();
        }
        Vec::new()
    }

    fn tenax_forget_conversation(&mut self) {
        let asking = self.tenax.asking.take();
        if let Some(id) = asking {
            self.drop_tenax_confirm(&id);
        }
        let state = &mut self.tenax;
        state.history.clear();
        state.transcript.clear();
        state.chat_request = None;
        state.carry.clear();
        state.queued.clear();
        state.pending.clear();
        state.order.clear();
        state.answered.clear();
        state.held = None;
        state.mood = Mood::Idle;
    }

    /// Take a proposal's confirmation off the stack, wherever it is.
    fn drop_tenax_confirm(&mut self, id: &str) {
        let mut kept = ModalStack::default();
        while let Some(modal) = self.modals.pop() {
            match &modal {
                Modal::Confirm(Confirm {
                    then: ConfirmThen::Tenax(proposal),
                    ..
                }) if proposal.call_id == id => {}
                _ => kept.push(modal),
            }
        }
        while let Some(modal) = kept.pop() {
            self.modals.push(modal);
        }
    }

    /// A turn came back.
    pub(super) fn on_tenax_turned(
        &mut self,
        request: u64,
        result: Result<Turn, Failed>,
    ) -> Vec<Effect> {
        if self.tenax.chat_request != Some(request) {
            return Vec::new();
        }
        self.tenax.chat_request = None;
        let ascii = self.theme.glyphs.is_ascii();
        let model = self.tenax.prefs.model.clone();
        match result {
            Ok(turn) => {
                self.tenax.history = turn.history;
                self.tenax.calls_left = turn.calls_left;
                let text = sanitize(&turn.text, ascii);
                if !text.is_empty() {
                    self.tenax.transcript.push(Said::Tenax(text));
                }
                match turn.ending {
                    Ending::NeedsApp => {
                        self.tenax.answered = turn.carry;
                        self.tenax.order = turn.order;
                        self.tenax.pending = turn.pending.into();
                    }
                    ending => {
                        self.tenax.carry.extend(turn.carry);
                        match ending {
                            Ending::Refused => self
                                .tenax
                                .transcript
                                .push(Said::Trouble("I'll pass on that one.".to_string())),
                            Ending::Cut => self.tenax.transcript.push(Said::Trouble(
                                "That ran long and got cut off. Ask me to go on.".to_string(),
                            )),
                            Ending::OutOfCalls => self.tenax.transcript.push(Said::Trouble(
                                "I'll stop there before this gets expensive. Ask me to go on."
                                    .to_string(),
                            )),
                            Ending::Done | Ending::NeedsApp => {}
                        }
                    }
                }
                self.tenax_speaks();
            }
            Err(failed) => {
                self.tenax.history = failed.history;
                self.tenax.calls_left = failed.calls_left;
                self.tenax.carry.extend(failed.carry);
                let sentence = trouble(&failed.error, &model);
                self.tenax.transcript.push(Said::Trouble(sentence.clone()));
                self.tenax.mood = Mood::Trouble(sentence);
            }
        }
        self.tenax_said_while_away();
        self.tenax_process()
    }

    /// When his box is not on screen and the chat is closed, the status line
    /// says he answered — and nothing of what he said.
    fn tenax_said_while_away(&mut self) {
        let box_shown = self.regions().tenax.is_some();
        if !self.tenax_chat_open() && !box_shown {
            self.info(format!(
                "Tenax answered — {} to read",
                command::key_of(CommandId::Tenax)
            ));
        }
    }

    /// Answer the last reply's calls, in order, until one needs the person;
    /// then, with every call answered, send the results back.
    fn tenax_process(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        loop {
            if self.tenax.running.is_some()
                || self.tenax.asking.is_some()
                || self.tenax.held.is_some()
            {
                return effects;
            }
            let Some(call) = self.tenax.pending.pop_front() else {
                effects.extend(self.tenax_continue());
                return effects;
            };
            match tenax_tools::kind_of(&call.name).map(|(_, kind)| kind) {
                Some(ToolKind::App) => {
                    let (outcome, more) = self.tenax_app_call(&call);
                    effects.extend(more);
                    self.tenax
                        .answered
                        .push(tenax_turn::tool_result(&call.id, &outcome));
                }
                Some(ToolKind::Write) => match self.tenax_proposal(&call) {
                    Ok(proposal) => {
                        self.tenax.held = Some(proposal);
                        self.tenax_ask_held();
                    }
                    Err(why) => self
                        .tenax
                        .answered
                        .push(tenax_turn::tool_result(&call.id, &Outcome::error(why))),
                },
                // The worker answers reads and unknown names; one here is a
                // call nobody can answer, and it is answered as such.
                _ => self.tenax.answered.push(tenax_turn::tool_result(
                    &call.id,
                    &Outcome::error("that call could not be run here"),
                )),
            }
        }
    }

    /// Every call answered: the results go back, with anything typed
    /// meanwhile after them. Nothing waiting and something typed: that goes.
    fn tenax_continue(&mut self) -> Vec<Effect> {
        let queued = std::mem::take(&mut self.tenax.queued);
        let text = (!queued.is_empty()).then(|| queued.join("\n\n"));
        if self.tenax_answering() {
            let order = std::mem::take(&mut self.tenax.order);
            let mut blocks = std::mem::take(&mut self.tenax.answered);
            blocks.append(&mut self.tenax.carry);
            if let Some(text) = text {
                blocks.push(tenax_turn::text_block(&format!(
                    "{}\n\n{text}",
                    self.tenax_context()
                )));
                self.tenax.calls_left = CALLS_PER_MESSAGE;
            }
            if !self.tenax_on() {
                // Turned off with calls answered: kept for a later message.
                self.tenax.carry = blocks;
                return Vec::new();
            }
            return self.tenax_request(tenax_turn::ordered(&order, blocks));
        }
        match text {
            Some(text) if self.tenax_on() => {
                let mut blocks = std::mem::take(&mut self.tenax.carry);
                blocks.push(tenax_turn::text_block(&format!(
                    "{}\n\n{text}",
                    self.tenax_context()
                )));
                self.tenax.calls_left = CALLS_PER_MESSAGE;
                self.tenax_request(blocks)
            }
            _ => Vec::new(),
        }
    }

    /// Put a held proposal to the person, when the chat is open and nothing
    /// is half typed on its line.
    fn tenax_ask_held(&mut self) {
        let typing = match self.modals.top() {
            Some(Modal::Tenax(chat)) => !chat.input.is_empty(),
            _ => true,
        };
        if typing || self.tenax.asking.is_some() {
            return;
        }
        let Some(mut proposal) = self.tenax.held.take() else {
            return;
        };
        proposal.armed_at = self.elapsed_ms + ARM_MS;
        self.tenax.asking = Some(proposal.call_id.clone());
        self.modals.push(Modal::Confirm(Confirm {
            prompt: proposal.question(),
            then: ConfirmThen::Tenax(Box::new(proposal)),
        }));
    }

    /// The person answered a proposal.
    pub(super) fn answer_tenax(&mut self, proposal: Proposal, yes: bool) -> Vec<Effect> {
        self.tenax.asking = None;
        if !yes {
            self.tenax_declined(&proposal);
            return self.tenax_process();
        }
        let project = match self
            .still_here(&crate::tui::app::actions::Targets {
                paths: vec![proposal.project.path.clone()],
                batch: false,
            })
            .into_iter()
            .next()
        {
            Some(project) => project,
            None => {
                self.tenax_answer(
                    &proposal.call_id,
                    Outcome::error("the project is no longer in the library; nothing was written"),
                );
                return self.tenax_process();
            }
        };
        if let Availability::Disabled(why) = crate::tui::command::not_busy(self) {
            self.tenax_answer(
                &proposal.call_id,
                Outcome::error(format!(
                    "fastf could not write now ({why}); nothing was written"
                )),
            );
            return self.tenax_process();
        }
        let proposal = Proposal {
            project: Box::new(project),
            ..proposal
        };
        let effects = self.run_action(proposal.busy(), proposal.action());
        match effects.iter().find_map(|effect| match effect {
            Effect::Run(id, _) => Some(*id),
            _ => None,
        }) {
            Some(id) => {
                self.tenax.running = Some((id, proposal));
                effects
            }
            None => {
                self.tenax_answer(
                    &proposal.call_id,
                    Outcome::error("fastf was busy with another change; nothing was written"),
                );
                let mut effects = effects;
                effects.extend(self.tenax_process());
                effects
            }
        }
    }

    fn tenax_answer(&mut self, call_id: &str, outcome: Outcome) {
        self.tenax
            .answered
            .push(tenax_turn::tool_result(call_id, &outcome));
    }

    fn tenax_declined(&mut self, proposal: &Proposal) {
        self.tenax_answer(
            &proposal.call_id,
            Outcome::ok("The person declined; nothing was written."),
        );
        self.tenax.transcript.push(Said::Declined(proposal.done()));
    }

    /// Whether `id` is the action carrying out one of his proposals.
    pub(super) fn tenax_runs(&self, id: ActionId) -> bool {
        self.tenax
            .running
            .as_ref()
            .is_some_and(|(running, _)| *running == id)
    }

    /// One of his proposals was written, or refused. Its result goes to him,
    /// and the change lands in the app as the same action from a key would.
    pub(super) fn on_tenax_action_done(
        &mut self,
        outcome: Result<Box<ActionOutcome>, String>,
    ) -> Vec<Effect> {
        let Some((_, proposal)) = self.tenax.running.take() else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        match outcome {
            Ok(outcome) => {
                let mut said = outcome.message.clone();
                if let Some(ordinal) = outcome.todo_ordinal {
                    said.push_str(&format!(" It is todo index {ordinal}."));
                }
                self.tenax_answer(&proposal.call_id, Outcome::ok(said));
                self.tenax.transcript.push(Said::Done(proposal.done()));
                effects.extend(self.on_action_succeeded(outcome));
            }
            Err(error) => {
                self.tenax_answer(
                    &proposal.call_id,
                    Outcome::error(format!("not written: {error}")),
                );
                self.tenax.transcript.push(Said::Trouble(format!(
                    "{}: not written ({error})",
                    proposal.done()
                )));
                self.error(format!("error: {error}"));
            }
        }
        effects.extend(self.tenax_process());
        effects
    }

    /// A call that moves the app's own view.
    fn tenax_app_call(&mut self, call: &Call) -> (Outcome, Vec<Effect>) {
        if !self.tenax_chat_open() {
            return (
                Outcome::error("not applied — the chat was closed"),
                Vec::new(),
            );
        }
        let mut effects = Vec::new();
        if self.screen == Screen::Templates {
            effects.extend(self.show_screen(Screen::Library));
        }
        match call.name.as_str() {
            "select_project" => {
                let at =
                    match tenax_tools::input::<tenax_tools::ProjectRef>(&call.name, &call.input) {
                        Ok(at) => at,
                        Err(why) => return (Outcome::error(why), effects),
                    };
                let project = match self.tenax_resolve(&at.project, at.folder.as_deref()) {
                    Ok(project) => project,
                    Err(why) => return (Outcome::error(why), effects),
                };
                if self.library.select_path(&project.path)
                    || self.show_its_base_alone(&project.path)
                {
                    if self.screen == Screen::Desk {
                        self.rebuild_desk();
                    }
                    effects.extend(self.after_selection_change());
                    (
                        Outcome::ok(format!("{} {} is selected", project.id, project.name)),
                        effects,
                    )
                } else {
                    (
                        Outcome::error(format!(
                            "{} is filtered out of the list; clear the search to show it",
                            project.id
                        )),
                        effects,
                    )
                }
            }
            "filter_library" => {
                let filter =
                    match tenax_tools::input::<tenax_tools::Filter>(&call.name, &call.input) {
                        Ok(filter) => filter,
                        Err(why) => return (Outcome::error(why), effects),
                    };
                self.search.input.set_text(filter.query.trim());
                effects.extend(self.after_query_change());
                (
                    Outcome::ok(format!("the list shows {} projects", self.library.len())),
                    effects,
                )
            }
            other => (
                Outcome::error(format!("{other} is not a call the app makes")),
                effects,
            ),
        }
    }

    /// The project an id (and, where two share it, a folder) names.
    fn tenax_resolve(&self, id: &str, folder: Option<&str>) -> Result<Project, String> {
        let id = id.trim();
        let matches: Vec<&Project> = self
            .library
            .snapshot
            .iter()
            .filter(|project| project.id.eq_ignore_ascii_case(id))
            .filter(|project| folder.is_none_or(|folder| project.name == folder.trim()))
            .collect();
        match matches.as_slice() {
            [] => Err(format!(
                "no project has the id {id}; find_projects lists them"
            )),
            [one] => Ok((*one).clone()),
            many => Err(format!(
                "{} projects have the id {id} (folders: {}); name the folder",
                many.len(),
                many.iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// A write call as a proposal, or the sentence that refuses it.
    fn tenax_proposal(&self, call: &Call) -> Result<Proposal, String> {
        use tenax_tools::input;
        let one_line = |text: &str, what: &str| -> Result<String, String> {
            let text = text.trim();
            if text.is_empty() {
                Err(format!("the {what} is empty"))
            } else if text.contains(['\n', '\r']) {
                Err(format!("a {what} is one line"))
            } else {
                Ok(text.to_string())
            }
        };
        let (at, folder, change) = match call.name.as_str() {
            "add_todo" => {
                let i: tenax_tools::AddTodo = input(&call.name, &call.input)?;
                let text = one_line(&i.text, "todo")?;
                let phase = i
                    .phase
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty());
                (i.project, i.folder, Change::AddTodo { text, phase })
            }
            "toggle_todo" => {
                let i: tenax_tools::TodoRef = input(&call.name, &call.input)?;
                (
                    i.project,
                    i.folder,
                    Change::Toggle {
                        ordinal: i.index,
                        was: i.text,
                    },
                )
            }
            "set_todo_owner" => {
                let i: tenax_tools::SetOwner = input(&call.name, &call.input)?;
                let owner = match i.owner.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
                    Some(owner) => Some(crate::tui::validators::owner(owner)?),
                    None => None,
                };
                (
                    i.project,
                    i.folder,
                    Change::Owner {
                        ordinal: i.index,
                        was: i.text,
                        owner,
                    },
                )
            }
            "add_note" => {
                let i: tenax_tools::Text = input(&call.name, &call.input)?;
                let text = i.text.trim().to_string();
                if text.is_empty() {
                    return Err("the note is empty".to_string());
                }
                (i.project, i.folder, Change::Note(text))
            }
            "add_tag" => {
                let i: tenax_tools::Tag = input(&call.name, &call.input)?;
                let tag = crate::tui::validators::tag(&i.tag)?;
                (i.project, i.folder, Change::Tag(tag))
            }
            "set_description" => {
                let i: tenax_tools::Text = input(&call.name, &call.input)?;
                let text = crate::tui::validators::description(&i.text)?;
                (i.project, i.folder, Change::Describe(text))
            }
            other => return Err(format!("{other} is not a change Tenax can propose")),
        };
        let project = self.tenax_resolve(&at, folder.as_deref())?;
        Ok(Proposal {
            call_id: call.id.clone(),
            project: Box::new(project),
            change,
            armed_at: 0,
        })
    }

    // --- the brief --------------------------------------------------------

    /// After every message: a question that went away unanswered is
    /// declined; and, once a run has what a brief needs, the one automatic
    /// brief is asked for.
    pub(super) fn tenax_settle(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(id) = self.tenax.asking.clone() {
            let shown = self.modals.iter().any(|modal| {
                matches!(modal, Modal::Confirm(Confirm { then: ConfirmThen::Tenax(p), .. }) if p.call_id == id)
            });
            if !shown {
                self.tenax.asking = None;
                self.tenax_answer(
                    &id,
                    Outcome::ok("The person declined; nothing was written."),
                );
                self.tenax
                    .transcript
                    .push(Said::Declined("a proposed change".to_string()));
                effects.extend(self.tenax_process());
            }
        }
        if !self.tenax_on() {
            if self.tenax.thinking() || !self.tenax.pending.is_empty() || self.tenax.held.is_some()
            {
                self.tenax_turned_off();
            }
            return effects;
        }
        // A proposal held while the line had text goes up once it is empty.
        if self.tenax.held.is_some() && self.tenax_chat_open() {
            self.tenax_ask_held();
        }
        effects.extend(self.tenax_auto_brief());
        effects
    }

    /// Off: nothing in flight is waited for, and every open call is answered
    /// "declined", kept for a later conversation.
    fn tenax_turned_off(&mut self) {
        self.tenax.chat_request = None;
        self.tenax.brief_request = None;
        if let Some(proposal) = self.tenax.held.take() {
            self.tenax_answer(
                &proposal.call_id,
                Outcome::ok("Tenax was turned off; nothing was written."),
            );
        }
        while let Some(call) = self.tenax.pending.pop_front() {
            self.tenax_answer(
                &call.id,
                Outcome::ok("Tenax was turned off; nothing was run."),
            );
        }
        if self.tenax_answering() {
            self.tenax.order.clear();
            let answered = std::mem::take(&mut self.tenax.answered);
            self.tenax.carry.extend(answered);
        }
    }

    /// The one automatic brief: plain `fastf`, a saved key, once per run, with
    /// every record read and the summary in.
    fn tenax_auto_brief(&mut self) -> Vec<Effect> {
        if self.tenax.briefed || !self.is_menu || !self.tenax.prefs.key.stored() {
            return Vec::new();
        }
        let settled = self.library.loaded && self.library.inflight.is_none();
        if !settled || self.summary_seen.contains(&0) {
            return Vec::new();
        }
        if !self.tenax.reading {
            self.tenax.reading = true;
            let reads = self.tenax_read_effects();
            if !reads.is_empty() {
                return reads;
            }
        }
        let silent = &self.library.silent;
        let waiting = self
            .desk
            .pending
            .iter()
            .any(|path| !silent.iter().any(|base| path.starts_with(base)));
        if waiting {
            return Vec::new();
        }
        self.tenax.briefed = true;
        let (digest, print) = self.tenax_digest(CARDS_PER_GROUP);
        let now = (self.now)();
        if let Some(brief) = &self.tenax.brief
            && brief.fingerprint == print
            && hours_between(&brief.at, &now).is_some_and(|hours| hours < BRIEF_STALE_HOURS)
        {
            return Vec::new();
        }
        self.tenax_brief_effect(Ask::Automatic, digest, print)
    }

    /// The palette's fresh brief: asked now, whatever the cache says, with
    /// the environment's key if that is the one there is — asking is consent.
    pub(super) fn tenax_fresh_brief(&mut self) -> Vec<Effect> {
        let mut effects = self.tenax_read_effects();
        let (digest, print) = self.tenax_digest(CARDS_PER_GROUP);
        effects.extend(self.tenax_brief_effect(Ask::Interactive, digest, print));
        self.tenax.talked = true;
        effects
    }

    fn tenax_brief_effect(&mut self, ask: Ask, digest: String, print: String) -> Vec<Effect> {
        let request = self.tenax.next();
        self.tenax.brief_request = Some(request);
        let now = (self.now)();
        let digest = format!("{}\n\n{digest}", self.tenax_context());
        vec![Effect::TenaxBrief(Box::new(BriefRequest {
            request,
            model: self.tenax.prefs.model.clone(),
            ask,
            digest,
            fingerprint: print,
            at: now,
        }))]
    }

    /// Every project's record the brief needs and the cache lacks, read the
    /// way the desk reads them — on a worker per base, in chunks.
    fn tenax_read_effects(&mut self) -> Vec<Effect> {
        let mut by_base: std::collections::BTreeMap<PathBuf, Vec<PathBuf>> = Default::default();
        let wanted: Vec<(PathBuf, PathBuf)> = self
            .tenax_projects()
            .filter(|project| !self.details.contains_key(&project.path))
            .filter(|project| !self.desk.pending.contains(&project.path))
            .map(|project| (project.base.clone(), project.path.clone()))
            .collect();
        for (base, path) in wanted {
            by_base.entry(base).or_default().push(path);
        }
        let mut effects = Vec::new();
        for paths in by_base.into_values() {
            self.desk.pending.extend(paths.iter().cloned());
            effects.push(Effect::LoadDetails(paths));
        }
        effects
    }

    /// The brief came back.
    pub(super) fn on_tenax_briefed(
        &mut self,
        request: u64,
        result: Result<Brief, ApiError>,
    ) -> Vec<Effect> {
        if self.tenax.brief_request != Some(request) {
            return Vec::new();
        }
        self.tenax.brief_request = None;
        match result {
            Ok(brief) => {
                let ascii = self.theme.glyphs.is_ascii();
                let brief = Brief {
                    say: sanitize(&brief.say, ascii),
                    start: brief.start.map(|start| Start {
                        project: sanitize(&start.project, ascii),
                        folder: sanitize(&start.folder, ascii),
                        todo: sanitize(&start.todo, ascii),
                    }),
                    ..brief
                };
                self.tenax.mood = Mood::Idle;
                self.tenax.brief = Some(brief.clone());
                self.tenax_speaks();
                self.tenax_said_while_away();
                vec![Effect::SaveTenax(Box::new(brief))]
            }
            Err(error) => {
                self.tenax.mood = Mood::Trouble(trouble(&error, &self.tenax.prefs.model));
                Vec::new()
            }
        }
    }

    /// The palette's "go to where Tenax said to start".
    pub(super) fn tenax_go_to_start(&mut self) -> Vec<Effect> {
        let Some(start) = self
            .tenax
            .brief
            .as_ref()
            .and_then(|brief| brief.start.clone())
        else {
            return Vec::new();
        };
        let folder = (!start.folder.is_empty()).then_some(start.folder.as_str());
        let project = match self.tenax_resolve(&start.project, folder) {
            Ok(project) => project,
            Err(_) => {
                self.warn(format!("{} is not in the library now", start.project));
                return Vec::new();
            }
        };
        let mut effects = Vec::new();
        if self.screen == Screen::Templates {
            effects.extend(self.show_screen(Screen::Library));
        }
        if self.library.select_path(&project.path) || self.show_its_base_alone(&project.path) {
            if self.screen == Screen::Desk {
                self.rebuild_desk();
            }
            self.pulse_selected();
            effects.extend(self.after_selection_change());
        } else {
            self.warn(format!(
                "{} is filtered out of the list — {} clears the filters",
                project.id,
                command::key_of(CommandId::ClearFilters)
            ));
        }
        effects
    }

    /// The palette's switch: Tenax on or off, as the settings row writes it.
    pub(super) fn tenax_toggle(&mut self) -> Vec<Effect> {
        let value = if self.tenax_on() { "false" } else { "true" };
        self.run_action(
            "saving…",
            Action::SetConfig {
                key: "tenax",
                value: value.to_string(),
            },
        )
    }

    // --- the chat's keys --------------------------------------------------

    /// The chat's keys: the line first for anything it takes, then the
    /// registry (send, close, scroll, a new chat), then the line again.
    pub(super) fn on_chat_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_some() || command::field_claims(&key) {
            if let Some(Modal::Tenax(chat)) = self.modals.top_mut() {
                chat.input.apply(&key);
            }
            return Vec::new();
        }
        match command::lookup(Context::Chat, key, self) {
            Some(id) => self.run(id),
            None => {
                if let Some(Modal::Tenax(chat)) = self.modals.top_mut() {
                    chat.input.apply(&key);
                }
                Vec::new()
            }
        }
    }

    /// The transcript's rows as the chat draws them, at its current size.
    pub fn tenax_chat_rows(&self) -> usize {
        let dialog = layout::tenax_chat(self.area());
        let width = crate::tui::view::modals::tenax::transcript_rect(dialog).width as usize;
        crate::tui::view::modals::tenax::transcript_lines(self, width).len()
    }

    /// How many transcript rows the chat shows at once.
    pub fn tenax_chat_window(&self) -> usize {
        crate::tui::view::modals::tenax::transcript_rect(layout::tenax_chat(self.area())).height
            as usize
    }

    /// Scroll the transcript by `delta` rows (`isize::MIN`/`MAX` for its ends).
    pub(super) fn tenax_scroll(&mut self, delta: isize) {
        let max = self
            .tenax_chat_rows()
            .saturating_sub(self.tenax_chat_window());
        if let Some(Modal::Tenax(chat)) = self.modals.top_mut() {
            let from = if chat.follow {
                max
            } else {
                chat.scroll.min(max)
            };
            chat.scroll = (from as isize).saturating_add(delta).clamp(0, max as isize) as usize;
            chat.follow = chat.scroll >= max;
        }
    }
}

/// Hours from one timestamp to another, when both read.
fn hours_between(from: &str, to: &str) -> Option<i64> {
    let from = chrono::DateTime::parse_from_rfc3339(from).ok()?;
    let to = chrono::DateTime::parse_from_rfc3339(to).ok()?;
    Some((to - from).num_hours())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_text_draws_no_escape_and_ascii_draws_ascii() {
        assert_eq!(sanitize("hi\u{1b}[2Jthere", false), "hi[2Jthere");
        assert_eq!(sanitize("“quoted” — done…", true), "\"quoted\" - done...");
        assert_eq!(sanitize("“quoted”", false), "“quoted”");
    }

    #[test]
    fn a_fingerprint_is_stable_and_tells_desks_apart() {
        assert_eq!(fingerprint("desk"), fingerprint("desk"));
        assert_ne!(fingerprint("desk"), fingerprint("desk "));
        assert_eq!(fingerprint(""), "cbf29ce484222325");
    }

    #[test]
    fn the_brief_schema_is_one_strict_output_accepts() {
        let schema = brief_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"].as_array().unwrap().len(), 4);
    }
}
