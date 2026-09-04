//! The session ring: the last few things this process did, for the header.
//!
//! In memory and per process: it is a reminder, not a log. `PROJECT_INFO.md`'s
//! journal is where anything durable belongs.

use std::sync::Mutex;

/// How many of this session's actions the header remembers.
const RING: usize = 3;

static RECENT_ACTIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Record one action, e.g. `created ID0248`, `tagged ID0100 urgent`,
/// `moved ID0017 → archive`.
pub fn record(action: impl Into<String>) {
    let Ok(mut ring) = RECENT_ACTIONS.lock() else {
        return;
    };
    ring.push(action.into());
    let overflow = ring.len().saturating_sub(RING);
    if overflow > 0 {
        ring.drain(..overflow);
    }
}

/// What `record` has collected, oldest first.
pub fn recent_actions() -> Vec<String> {
    RECENT_ACTIONS
        .lock()
        .map(|ring| ring.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{RECENT_ACTIONS, RING, recent_actions, record};

    #[test]
    fn the_session_ring_keeps_the_last_few_actions() {
        RECENT_ACTIONS.lock().unwrap().clear();
        for n in 1..=6 {
            record(format!("created ID000{n}"));
        }
        let actions = recent_actions();
        assert_eq!(actions.len(), RING);
        assert_eq!(actions.first().unwrap(), "created ID0004");
        assert_eq!(actions.last().unwrap(), "created ID0006");
        RECENT_ACTIONS.lock().unwrap().clear();
    }
}
