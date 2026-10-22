//! The Messages API request Tenax sends, and how its answer is read.
//!
//! **The prefix never moves.** The API rejects a replayed thinking block once
//! anything before it — the tools, the system prompt, an earlier message — has
//! changed, and a cached prefix is only as long as the bytes that match. So the
//! tools and the system prompt are fixed for the life of the binary, the
//! history is only ever appended to, and everything that changes per request
//! (the date, the hat, the desk) rides in a text block of the user turn.
//!
//! **Every block the model wrote goes back as it came**, as a `Value`:
//! thinking blocks whose text is empty (only their signature is returned), a
//! server fallback's block, and types this build has never heard of.

use std::time::Duration;

use serde_json::{Value, json};

use super::key::Secret;
use crate::util::http::Reply;

/// Where the request goes.
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// The API version every request names.
pub const API_VERSION: &str = "2026-10-21";
/// The model Tenax runs on unless `tenax-model` names another.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
/// How hard the model thinks. The user asked for high.
pub const EFFORT: &str = "high";
/// Room for the thinking and the reply. A non-streaming request this size
/// answers well inside [`TIMEOUT`].
pub const MAX_TOKENS: u32 = 16_000;
/// The whole exchange, thinking included, at most.
pub const TIMEOUT: Duration = Duration::from_secs(600);
/// A refusal on the default model is retried on another server-side, by the
/// refusal's category, when this beta is named with `fallbacks: "default"`.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// The URL a request goes to. A debug build reads `FASTF_TENAX_URL`, so the
/// suites can answer from a server of their own; a release build cannot be
/// pointed anywhere else.
pub fn endpoint() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("FASTF_TENAX_URL")
        && !url.is_empty()
    {
        return url;
    }
    API_URL.to_string()
}

/// One request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request<'a> {
    pub model: &'a str,
    /// The system prompt's blocks; the last carries the cache breakpoint.
    pub system: &'a [&'a str],
    /// The tool definitions, or none.
    pub tools: Option<&'a Value>,
    pub messages: &'a [Value],
    /// A JSON schema the answer must follow, for a reply that is data.
    pub format: Option<&'a Value>,
}

impl Request<'_> {
    /// The JSON body. No sampling parameters (the API refuses them), no
    /// `tool_choice` (`auto` is the default and the only one these models take).
    pub fn body(&self) -> Value {
        let last = self.system.len().saturating_sub(1);
        let system: Vec<Value> = self
            .system
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let mut block = json!({ "type": "text", "text": text });
                if i == last {
                    block["cache_control"] = json!({ "type": "ephemeral" });
                }
                block
            })
            .collect();
        let mut output = json!({ "effort": EFFORT });
        if let Some(schema) = self.format {
            output["format"] = json!({ "type": "json_schema", "schema": schema });
        }
        let mut body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": self.messages,
            "output_config": output,
            // The growing history is cached too, not only the system prompt:
            // a turn of eight calls re-sends it eight times.
            "cache_control": { "type": "ephemeral" },
        });
        if let Some(tools) = self.tools {
            body["tools"] = tools.clone();
        }
        if self.falls_back() {
            body["fallbacks"] = json!("default");
        }
        body
    }

    /// The headers besides the key.
    pub fn headers(&self) -> Vec<(&'static str, &'static str)> {
        let mut headers = vec![("anthropic-version", API_VERSION)];
        if self.falls_back() {
            headers.push(("anthropic-beta", FALLBACK_BETA));
        }
        headers
    }

    /// The fallback is named for the model it is known to work with. Another
    /// model may refuse the parameter outright.
    fn falls_back(&self) -> bool {
        self.model == DEFAULT_MODEL
    }
}

/// Why the model stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    EndTurn,
    ToolUse,
    /// Out of room: the reply, or a tool call, is cut off.
    MaxTokens,
    /// A safety classifier declined, and so did any fallback.
    Refusal,
    Other(String),
}

/// What a request cost, for the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// A reply the model gave.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// Every block, untouched — the assistant turn as it goes back.
    pub content: Vec<Value>,
    pub stop: Stop,
    pub usage: Usage,
}

impl Answer {
    /// The text blocks, joined: what is said to the person.
    pub fn text(&self) -> String {
        text_of(&self.content)
    }

    /// The tool calls, in the order the model made them.
    pub fn calls(&self) -> Vec<Call> {
        self.content
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .map(|block| Call {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                input: block["input"].clone(),
            })
            .collect()
    }
}

/// The text blocks of an assistant turn's content, joined by blank lines.
pub fn text_of(content: &[Value]) -> String {
    content
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// A request that did not end in an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// No key to send: none saved, and none in the environment for a person
    /// who asked.
    NoKey,
    /// 400: the API refused the request; its own words.
    Refused(String),
    /// 401: the key.
    Key,
    /// 402: billing.
    Billing,
    /// 403: this key may not do this.
    Permission(String),
    /// 404: the model is unknown, or not this account's.
    NoModel(String),
    /// 413: too large.
    TooLarge,
    /// 429: too many requests; `retry-after` when the server said.
    Busy { retry_after: Option<u64> },
    /// 529: the API is overloaded.
    Overloaded,
    /// Any other 5xx.
    Server(u16),
    /// No answer at all: no connection, TLS, a timeout.
    Network(String),
    /// An answer that is not the API's shape.
    Garbled(String),
}

impl ApiError {
    /// Worth asking once more after a pause.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            ApiError::Busy { .. } | ApiError::Overloaded | ApiError::Server(_)
        )
    }

    /// How long to wait before asking again: what the server said, else two
    /// seconds, never more than ten.
    pub fn wait(&self) -> Duration {
        let secs = match self {
            ApiError::Busy {
                retry_after: Some(secs),
            } => *secs,
            _ => 2,
        };
        Duration::from_secs(secs.min(10))
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::NoKey => f.write_str("no API key"),
            ApiError::Refused(why) => write!(f, "the API refused the request: {why}"),
            ApiError::Key => f.write_str("the API key was refused (401)"),
            ApiError::Billing => f.write_str("the account has a billing problem (402)"),
            ApiError::Permission(why) => write!(f, "not allowed with this key (403): {why}"),
            ApiError::NoModel(why) => write!(f, "model not found (404): {why}"),
            ApiError::TooLarge => f.write_str("the request was too large (413)"),
            ApiError::Busy { .. } => f.write_str("too many requests (429)"),
            ApiError::Overloaded => f.write_str("the API is overloaded (529)"),
            ApiError::Server(status) => write!(f, "the API failed ({status})"),
            ApiError::Network(why) => write!(f, "no answer: {why}"),
            ApiError::Garbled(why) => write!(f, "an answer that could not be read: {why}"),
        }
    }
}

/// Read a reply: an answer, or what went wrong.
pub fn read(reply: &Reply) -> Result<Answer, ApiError> {
    let parsed: Option<Value> = serde_json::from_str(&reply.body).ok();
    let message = || {
        parsed
            .as_ref()
            .and_then(|v| v["error"]["message"].as_str())
            .unwrap_or("no message")
            .to_string()
    };
    match reply.status {
        200 => {}
        400 => return Err(ApiError::Refused(message())),
        401 => return Err(ApiError::Key),
        402 => return Err(ApiError::Billing),
        403 => return Err(ApiError::Permission(message())),
        404 => return Err(ApiError::NoModel(message())),
        413 => return Err(ApiError::TooLarge),
        429 => {
            return Err(ApiError::Busy {
                retry_after: reply.retry_after,
            });
        }
        529 => return Err(ApiError::Overloaded),
        status if status >= 500 => return Err(ApiError::Server(status)),
        status => return Err(ApiError::Garbled(format!("status {status}: {}", message()))),
    }
    let Some(value) = parsed else {
        return Err(ApiError::Garbled("not JSON".into()));
    };
    let stop = match value["stop_reason"].as_str() {
        Some("end_turn") | Some("stop_sequence") => Stop::EndTurn,
        Some("tool_use") => Stop::ToolUse,
        Some("max_tokens") => Stop::MaxTokens,
        Some("refusal") => Stop::Refusal,
        Some(other) => Stop::Other(other.to_string()),
        None => return Err(ApiError::Garbled("no stop_reason".into())),
    };
    let content = value["content"].as_array().cloned().unwrap_or_default();
    let number = |key: &str| value["usage"][key].as_u64().unwrap_or(0);
    Ok(Answer {
        content,
        stop,
        usage: Usage {
            input: number("input_tokens"),
            output: number("output_tokens"),
            cache_read: number("cache_read_input_tokens"),
            cache_write: number("cache_creation_input_tokens"),
        },
    })
}

/// Something that carries a request to the model and brings back its answer:
/// the network in the app, a script of replies in a test.
pub trait Transport {
    fn send(&self, request: &Request, key: &Secret) -> Result<Answer, ApiError>;
}

/// The real one: HTTPS to [`endpoint`].
pub struct Https;

impl Transport for Https {
    fn send(&self, request: &Request, key: &Secret) -> Result<Answer, ApiError> {
        let body = request.body().to_string();
        let mut headers: Vec<(&str, &str)> = request.headers();
        headers.push(("x-api-key", key.expose()));
        let started = std::time::Instant::now();
        let result = match crate::util::http::post_json(&endpoint(), &headers, &body, TIMEOUT) {
            Ok(reply) => read(&reply),
            Err(err) => Err(ApiError::Network(format!("{err:#}"))),
        };
        let ms = started.elapsed().as_millis();
        // Facts only: never a word of the conversation, never the key.
        match &result {
            Ok(answer) => crate::util::log::info(format!(
                "tenax: {} answered in {ms} ms ({:?}); tokens in {} out {} cache read {} written {}",
                request.model,
                answer.stop,
                answer.usage.input,
                answer.usage.output,
                answer.usage.cache_read,
                answer.usage.cache_write
            )),
            Err(err) => {
                crate::util::log::warn(format!("tenax: {} after {ms} ms: {err}", request.model))
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            retry_after: None,
            body: body.to_string(),
        }
    }

    /// The request names the model, high effort and the cache breakpoints, and
    /// nothing the API refuses: no sampling, no forced tool choice, no thinking
    /// budget.
    #[test]
    fn the_request_asks_for_high_effort_and_nothing_refused() {
        let tools = json!([{ "name": "desk" }]);
        let messages = vec![json!({ "role": "user", "content": "hi" })];
        let request = Request {
            model: DEFAULT_MODEL,
            system: &["who you are", "the keys"],
            tools: Some(&tools),
            messages: &messages,
            format: None,
        };
        let body = request.body();
        assert_eq!(body["model"], "claude-sonnet-5-5");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["max_tokens"], 16_000);
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["cache_control"]["type"], "ephemeral");
        assert!(body["system"][0].get("cache_control").is_none());
        assert_eq!(body["system"][1]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"], tools);
        for refused in ["temperature", "top_p", "top_k", "tool_choice", "thinking"] {
            assert!(body.get(refused).is_none(), "{refused} must not be sent");
        }
        assert!(
            request
                .headers()
                .contains(&("anthropic-beta", FALLBACK_BETA))
        );
        assert!(
            request
                .headers()
                .contains(&("anthropic-version", API_VERSION))
        );
    }

    /// Another model gets no fallback: it may refuse the parameter.
    #[test]
    fn only_the_default_model_names_the_fallback() {
        let messages = vec![];
        let request = Request {
            model: "claude-opus-5-5",
            system: &["x"],
            tools: None,
            messages: &messages,
            format: Some(&json!({ "type": "object" })),
        };
        let body = request.body();
        assert!(body.get("fallbacks").is_none());
        assert!(body.get("tools").is_none());
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert!(
            !request
                .headers()
                .iter()
                .any(|(name, _)| *name == "anthropic-beta")
        );
    }

    /// A thinking block with empty text, a fallback block and a type nobody
    /// knows all come back exactly as the API sent them.
    #[test]
    fn every_block_goes_back_as_it_came() {
        let body = r#"{"content":[
            {"type":"thinking","thinking":"","signature":"c2lnbmF0dXJl"},
            {"type":"fallback","from":{"model":"a"},"to":{"model":"b"}},
            {"type":"text","text":"Morning."},
            {"type":"something_new","x":[1,2.5]},
            {"type":"tool_use","id":"toolu_1","name":"desk","input":{}}
        ],"stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":7}}"#;
        let answer = read(&reply(200, body)).unwrap();
        let original: Value = serde_json::from_str(body).unwrap();
        assert_eq!(Value::Array(answer.content.clone()), original["content"]);
        assert_eq!(answer.stop, Stop::ToolUse);
        assert_eq!(answer.text(), "Morning.");
        assert_eq!(answer.calls()[0].name, "desk");
        assert_eq!(answer.usage.cache_read, 7);
    }

    #[test]
    fn a_refusal_is_read_before_the_content() {
        let answer = read(&reply(200, r#"{"content":[],"stop_reason":"refusal"}"#)).unwrap();
        assert_eq!(answer.stop, Stop::Refusal);
        assert_eq!(answer.text(), "");
    }

    #[test]
    fn every_status_has_its_class() {
        let err =
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"model: bad"}}"#;
        assert_eq!(
            read(&reply(400, err)),
            Err(ApiError::Refused("model: bad".into()))
        );
        assert_eq!(read(&reply(401, err)), Err(ApiError::Key));
        assert_eq!(read(&reply(402, err)), Err(ApiError::Billing));
        assert!(matches!(
            read(&reply(403, err)),
            Err(ApiError::Permission(_))
        ));
        assert!(matches!(read(&reply(404, err)), Err(ApiError::NoModel(_))));
        assert_eq!(read(&reply(413, "")), Err(ApiError::TooLarge));
        assert_eq!(read(&reply(529, "")), Err(ApiError::Overloaded));
        assert_eq!(read(&reply(503, "")), Err(ApiError::Server(503)));
        let busy = Reply {
            status: 429,
            retry_after: Some(30),
            body: String::new(),
        };
        let busy = read(&busy).unwrap_err();
        assert!(busy.retryable());
        assert_eq!(busy.wait(), Duration::from_secs(10));
        assert!(!ApiError::Key.retryable());
        assert!(matches!(
            read(&reply(200, "not json")),
            Err(ApiError::Garbled(_))
        ));
    }
}
