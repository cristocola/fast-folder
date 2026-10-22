//! One turn of a conversation: the person's message in, the model asked as
//! many times as its read tools need, out when it has said its piece or wants
//! something only the app can give.
//!
//! **Every `tool_use` gets exactly one `tool_result`, all of them in the next
//! user message.** A call the model made and nobody answered is a 400 on the
//! next request, so whatever stops a turn — a write the app must put to the
//! person, a reply cut off mid-call, the call budget spent, a refusal — hands
//! back the results already decided ([`Turn::carry`]) for the caller to put
//! first in the next user message, beside its own answers.
//!
//! **No pause is taken here.** `core` may not sleep (`tests/layering.rs`); the
//! one wait before asking again is the caller's `wait`.

use std::time::Duration;

use serde_json::{Value, json};

use super::api::{ApiError, Call, Request, Stop, Transport};
use super::key::Secret;
use super::tools::{self, Kind};

/// What a read tool answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub content: String,
    pub is_error: bool,
}

impl Outcome {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

/// Runs the read tools, on whatever thread the turn runs on.
pub trait Executor {
    /// `name` is the tool's canonical name.
    fn read(&mut self, name: &str, input: &Value) -> Outcome;
}

/// The `tool_result` block answering call `id`.
pub fn tool_result(id: &str, outcome: &Outcome) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": outcome.content,
        "is_error": outcome.is_error,
    })
}

/// A user message of `blocks`.
pub fn user_message(blocks: Vec<Value>) -> Value {
    json!({ "role": "user", "content": blocks })
}

/// A text block.
pub fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

/// Results first, in the order the model made the calls, then anything else:
/// the order a person reading the request would expect.
pub fn ordered(order: &[String], mut blocks: Vec<Value>) -> Vec<Value> {
    let rank = |block: &Value| {
        block["tool_use_id"]
            .as_str()
            .and_then(|id| order.iter().position(|known| known == id))
            .unwrap_or(usize::MAX)
    };
    blocks.sort_by_key(|block| (block["type"] != "tool_result", rank(block)));
    blocks
}

/// How a turn ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The model said its piece.
    Done,
    /// The model made calls only the app can answer: `Turn::pending`.
    NeedsApp,
    /// A safety classifier declined. The message that drew it is not kept.
    Refused,
    /// Out of room mid-reply.
    Cut,
    /// The call budget ran out before the model was done.
    OutOfCalls,
}

/// A turn that ended in an answer of some kind.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    /// The conversation so far, to send again next time.
    pub history: Vec<Value>,
    /// What the model said this turn, every text block of every reply.
    pub text: String,
    pub ending: Ending,
    /// `tool_result` blocks already decided, for the next user message.
    pub carry: Vec<Value>,
    /// The calls the app must answer (with `Ending::NeedsApp`).
    pub pending: Vec<Call>,
    /// The ids of the last reply's calls, in the order the model made them.
    pub order: Vec<String>,
    pub calls_left: u32,
}

/// A turn that ended without an answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Failed {
    /// The conversation as it was before the request that failed.
    pub history: Vec<Value>,
    /// The blocks of the user message that was never answered, to send again.
    pub carry: Vec<Value>,
    pub text: String,
    pub error: ApiError,
    pub calls_left: u32,
}

/// What every request in a turn shares.
pub struct Setup<'a> {
    pub transport: &'a dyn Transport,
    pub key: &'a Secret,
    pub model: &'a str,
    pub system: &'a [&'a str],
    pub tools: Option<&'a Value>,
    pub format: Option<&'a Value>,
    /// The pause before asking again after a busy or failed answer.
    pub wait: &'a dyn Fn(Duration),
}

/// Run one turn: `user` (content blocks) after `history`, with at most
/// `calls_left` requests.
pub fn turn(
    setup: &Setup,
    history: Vec<Value>,
    user: Vec<Value>,
    executor: &mut dyn Executor,
    mut calls_left: u32,
) -> Result<Turn, Failed> {
    let mut messages = history;
    messages.push(user_message(user));
    let mut said: Vec<String> = Vec::new();
    loop {
        if calls_left == 0 {
            let carry = take_last_user(&mut messages);
            return Ok(Turn {
                history: messages,
                text: said.join("\n\n"),
                ending: Ending::OutOfCalls,
                carry,
                pending: Vec::new(),
                order: Vec::new(),
                calls_left,
            });
        }
        let request = Request {
            model: setup.model,
            system: setup.system,
            tools: setup.tools,
            messages: &messages,
            format: setup.format,
        };
        calls_left -= 1;
        let mut answer = setup.transport.send(&request, setup.key);
        if let Err(err) = &answer
            && err.retryable()
            && calls_left > 0
        {
            (setup.wait)(err.wait());
            calls_left -= 1;
            answer = setup.transport.send(&request, setup.key);
        }
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                let carry = take_last_user(&mut messages);
                return Err(Failed {
                    history: messages,
                    carry,
                    text: said.join("\n\n"),
                    error,
                    calls_left,
                });
            }
        };

        if answer.stop == Stop::Refusal {
            // The message that drew it goes, or it draws the same refusal on
            // every turn after; the results it carried still answer their calls.
            let carry: Vec<Value> = take_last_user(&mut messages)
                .into_iter()
                .filter(|block| block["type"] == "tool_result")
                .collect();
            return Ok(Turn {
                history: messages,
                text: said.join("\n\n"),
                ending: Ending::Refused,
                carry,
                pending: Vec::new(),
                order: Vec::new(),
                calls_left,
            });
        }

        let text = answer.text();
        if !text.is_empty() {
            said.push(text);
        }
        let calls = answer.calls();
        let order: Vec<String> = calls.iter().map(|call| call.id.clone()).collect();
        if !answer.content.is_empty() {
            messages.push(json!({ "role": "assistant", "content": answer.content }));
        }

        match answer.stop {
            Stop::ToolUse => {
                let mut done = Vec::new();
                let mut pending = Vec::new();
                for call in calls {
                    match tools::kind_of(&call.name) {
                        Some((name, Kind::Read)) => {
                            let outcome = executor.read(name, &call.input);
                            done.push(tool_result(&call.id, &outcome));
                        }
                        Some((name, Kind::App | Kind::Write)) => pending.push(Call {
                            name: name.to_string(),
                            ..call
                        }),
                        None => done.push(tool_result(
                            &call.id,
                            &Outcome::error(format!("there is no tool called {}", call.name)),
                        )),
                    }
                }
                if pending.is_empty() {
                    messages.push(user_message(done));
                    continue;
                }
                return Ok(Turn {
                    history: messages,
                    text: said.join("\n\n"),
                    ending: Ending::NeedsApp,
                    carry: done,
                    pending,
                    order,
                    calls_left,
                });
            }
            stop => {
                // A reply that stopped for any other reason may still hold a
                // call — one cut off mid-input — and every call is answered.
                let carry = calls
                    .iter()
                    .map(|call| {
                        tool_result(
                            &call.id,
                            &Outcome::error(
                                "the reply was cut off before this call was finished; nothing ran",
                            ),
                        )
                    })
                    .collect();
                let ending = if stop == Stop::MaxTokens {
                    Ending::Cut
                } else {
                    Ending::Done
                };
                return Ok(Turn {
                    history: messages,
                    text: said.join("\n\n"),
                    ending,
                    carry,
                    pending: Vec::new(),
                    order,
                    calls_left,
                });
            }
        }
    }
}

/// Take the last message off when it is the person's, and hand back its
/// blocks: what was sent and not answered goes again, never twice.
fn take_last_user(messages: &mut Vec<Value>) -> Vec<Value> {
    if messages.last().is_some_and(|m| m["role"] == "user") {
        let last = messages.pop().unwrap_or_default();
        return last["content"].as_array().cloned().unwrap_or_default();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::super::api::{Answer, Usage};
    use super::*;
    use std::cell::RefCell;

    /// A transport that answers from a script and remembers what it was sent.
    struct Script {
        replies: RefCell<Vec<Result<Answer, ApiError>>>,
        sent: RefCell<Vec<Value>>,
    }

    impl Script {
        fn new(replies: Vec<Result<Answer, ApiError>>) -> Self {
            Self {
                replies: RefCell::new(replies.into_iter().rev().collect()),
                sent: RefCell::new(Vec::new()),
            }
        }
    }

    impl Transport for Script {
        fn send(&self, request: &Request, _key: &Secret) -> Result<Answer, ApiError> {
            self.sent.borrow_mut().push(request.body());
            self.replies
                .borrow_mut()
                .pop()
                .expect("the script ran out of replies")
        }
    }

    struct Reads(Vec<String>);

    impl Executor for Reads {
        fn read(&mut self, name: &str, _input: &Value) -> Outcome {
            self.0.push(name.to_string());
            Outcome::ok(format!("{name} says hi"))
        }
    }

    fn answer(stop: Stop, content: Value) -> Result<Answer, ApiError> {
        Ok(Answer {
            content: content.as_array().cloned().unwrap(),
            stop,
            usage: Usage::default(),
        })
    }

    fn call(id: &str, name: &str) -> Value {
        json!({ "type": "tool_use", "id": id, "name": name, "input": {} })
    }

    fn run(script: &Script, calls: u32) -> (Result<Turn, Failed>, Vec<String>, u32) {
        let key = Secret::new("test-key").unwrap();
        let waited = RefCell::new(0u32);
        let wait = |_: Duration| *waited.borrow_mut() += 1;
        let setup = Setup {
            transport: script,
            key: &key,
            model: "m",
            system: &["s"],
            tools: Some(tools::definitions()),
            format: None,
            wait: &wait,
        };
        let mut reads = Reads(Vec::new());
        let result = turn(
            &setup,
            Vec::new(),
            vec![text_block("hello")],
            &mut reads,
            calls,
        );
        let waited = *waited.borrow();
        (result, reads.0, waited)
    }

    #[test]
    fn a_read_is_answered_on_the_spot_and_the_model_goes_on() {
        let script = Script::new(vec![
            answer(
                Stop::ToolUse,
                json!([{ "type": "text", "text": "Let me look." }, call("t1", "desk")]),
            ),
            answer(
                Stop::EndTurn,
                json!([{ "type": "text", "text": "Start with lullaby." }]),
            ),
        ]);
        let (result, reads, _) = run(&script, 8);
        let turn = result.unwrap();
        assert_eq!(turn.ending, Ending::Done);
        assert_eq!(turn.text, "Let me look.\n\nStart with lullaby.");
        assert_eq!(reads, vec!["desk"]);
        assert_eq!(turn.calls_left, 6);
        // The second request carried the first reply and the read's result.
        let second = &script.sent.borrow()[1]["messages"];
        assert_eq!(second[1]["role"], "assistant");
        assert_eq!(second[2]["content"][0]["tool_use_id"], "t1");
        assert_eq!(second[2]["content"][0]["content"], "desk says hi");
    }

    /// A reply with a read and a write: the read is answered, the write is
    /// handed to the app, and both results are left for one message.
    #[test]
    fn a_write_stops_the_turn_with_the_reads_already_answered() {
        let script = Script::new(vec![answer(
            Stop::ToolUse,
            json!([call("t1", "desk"), call("t2", "ADD_TODO")]),
        )]);
        let (result, reads, _) = run(&script, 8);
        let turn = result.unwrap();
        assert_eq!(turn.ending, Ending::NeedsApp);
        assert_eq!(reads, vec!["desk"]);
        assert_eq!(turn.pending.len(), 1);
        assert_eq!(turn.pending[0].name, "add_todo");
        assert_eq!(turn.carry.len(), 1);
        assert_eq!(turn.order, vec!["t1", "t2"]);
        // The history ends with the reply that made the calls.
        assert_eq!(turn.history.last().unwrap()["role"], "assistant");
        let write = tool_result("t2", &Outcome::ok("added"));
        let blocks = ordered(
            &turn.order,
            vec![text_block("next"), write, turn.carry[0].clone()],
        );
        assert_eq!(blocks[0]["tool_use_id"], "t1");
        assert_eq!(blocks[1]["tool_use_id"], "t2");
        assert_eq!(blocks[2]["type"], "text");
    }

    #[test]
    fn a_busy_answer_is_asked_once_more_after_the_wait() {
        let script = Script::new(vec![
            Err(ApiError::Overloaded),
            answer(Stop::EndTurn, json!([{ "type": "text", "text": "Here." }])),
        ]);
        let (result, _, waited) = run(&script, 8);
        assert_eq!(result.unwrap().text, "Here.");
        assert_eq!(waited, 1);
    }

    /// A refused key is not asked again, and the person's message comes back
    /// to be sent once the key is fixed.
    #[test]
    fn a_refused_key_is_not_asked_again() {
        let script = Script::new(vec![Err(ApiError::Key)]);
        let (result, _, waited) = run(&script, 8);
        let failed = result.unwrap_err();
        assert_eq!(failed.error, ApiError::Key);
        assert_eq!(waited, 0);
        assert!(failed.history.is_empty());
        assert_eq!(failed.carry, vec![text_block("hello")]);
    }

    /// Reads that never end stop at the budget, with the last results kept so
    /// the calls they answer are answered next time.
    #[test]
    fn the_budget_stops_a_model_that_keeps_reading() {
        let script = Script::new(vec![
            answer(Stop::ToolUse, json!([call("t1", "desk")])),
            answer(Stop::ToolUse, json!([call("t2", "desk")])),
        ]);
        let (result, _, _) = run(&script, 2);
        let turn = result.unwrap();
        assert_eq!(turn.ending, Ending::OutOfCalls);
        assert_eq!(turn.carry[0]["tool_use_id"], "t2");
        assert_eq!(turn.history.last().unwrap()["role"], "assistant");
    }

    #[test]
    fn a_reply_cut_off_mid_call_answers_the_call() {
        let script = Script::new(vec![answer(
            Stop::MaxTokens,
            json!([{ "type": "text", "text": "I'll add" }, call("t1", "add_todo")]),
        )]);
        let (result, _, _) = run(&script, 8);
        let turn = result.unwrap();
        assert_eq!(turn.ending, Ending::Cut);
        assert!(turn.pending.is_empty());
        assert_eq!(turn.carry[0]["tool_use_id"], "t1");
        assert_eq!(turn.carry[0]["is_error"], true);
    }

    #[test]
    fn a_refusal_drops_the_message_that_drew_it() {
        let script = Script::new(vec![answer(Stop::Refusal, json!([]))]);
        let (result, _, _) = run(&script, 8);
        let turn = result.unwrap();
        assert_eq!(turn.ending, Ending::Refused);
        assert!(turn.history.is_empty());
        assert!(turn.carry.is_empty());
    }

    #[test]
    fn an_unknown_tool_is_answered_with_an_error() {
        let script = Script::new(vec![
            answer(Stop::ToolUse, json!([call("t1", "rm_rf")])),
            answer(Stop::EndTurn, json!([{ "type": "text", "text": "Fine." }])),
        ]);
        let (result, reads, _) = run(&script, 8);
        assert_eq!(result.unwrap().text, "Fine.");
        assert!(reads.is_empty());
        let second = &script.sent.borrow()[1]["messages"];
        assert_eq!(second[2]["content"][0]["is_error"], true);
    }
}
