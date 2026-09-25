//! Messages: the sentences fastf said to a person, kept.
//!
//! What the app's status line said, what the command line's outcome said, and
//! what a job reports when it ends — each a sentence, most with what to do
//! next. `<data dir>/messages.log` holds them from every session, one JSON
//! record per line, so `L` in the app and `fastf messages` show yesterday's as
//! well as today's. The log ([`crate::util::log`]) is the other half: every
//! message is written there too, among everything else.
//!
//! Appended in one write per record, as the log is, and rotated past
//! [`ROTATE_BYTES`] into `messages.log.1`. The reader keeps what it can read:
//! a record from a newer fastf with fields this one does not know reads, and a
//! line that is not a record is skipped.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Past this the file is rotated, once.
pub const ROTATE_BYTES: u64 = 512 * 1024;

/// How a message reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Info,
    /// Something worked.
    Good,
    Warn,
    Error,
}

/// One message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Message {
    /// UTC, ISO-8601, seconds.
    pub at: String,
    pub level: Level,
    /// Who said it: `app`, `cli`, or a job's id.
    pub source: String,
    pub text: String,
}

impl Message {
    /// A message said now.
    pub fn now(level: Level, source: &str, text: impl Into<String>) -> Self {
        Self {
            at: crate::util::time::now_iso8601(),
            level,
            source: source.to_string(),
            text: text.into(),
        }
    }
}

/// The messages file, when there is a data directory.
pub fn path() -> Option<PathBuf> {
    crate::util::log::dir().and_then(|logs| logs.parent().map(|dir| dir.join("messages.log")))
}

/// Keep `message`, and write it to the log as well. Best effort.
pub fn append(message: &Message) {
    let level = match message.level {
        Level::Info | Level::Good => crate::util::log::Level::Info,
        Level::Warn => crate::util::log::Level::Warn,
        Level::Error => crate::util::log::Level::Error,
    };
    crate::util::log::write(level, &format!("[{}] {}", message.source, message.text));
    let (Some(path), Ok(mut line)) = (path(), serde_json::to_string(message)) else {
        return;
    };
    line.push('\n');
    crate::util::log::append(&path, &line, ROTATE_BYTES);
}

/// The last `count` messages, oldest first, from the rotated file too when
/// the live one holds fewer.
pub fn last(count: usize) -> Vec<Message> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let mut messages = read(&PathBuf::from(format!("{}.1", path.display())));
    messages.extend(read(&path));
    let skip = messages.len().saturating_sub(count);
    messages.split_off(skip)
}

fn read(path: &std::path::Path) -> Vec<Message> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse(&text)
}

/// Every record `text` holds, skipping any line that is not one — a line torn
/// by a crash mid-write among them.
pub fn parse(text: &str) -> Vec<Message> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Message>(line).ok())
        .filter(|message| !message.text.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_keeps_what_it_can_read() {
        let text = concat!(
            r#"{"at":"2026-09-25T14:03:11Z","level":"good","source":"cli","text":"moved"}"#,
            "\n",
            "not a record\n",
            r#"{"at":"x","level":"warn","source":"app","text":"careful","colour":"red"}"#,
            "\n",
            r#"{"at":"x","level":"good","source":"cli","te"#,
        );
        let messages = parse(text);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].level, Level::Good);
        assert_eq!(messages[1].text, "careful");
    }

    #[test]
    fn a_message_round_trips() {
        let message = Message::now(Level::Warn, "app", "the original is still there");
        let line = serde_json::to_string(&message).unwrap();
        assert_eq!(parse(&line), vec![message]);
    }
}
