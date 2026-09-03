//! The one place `core` and `util` are allowed to say something.
//!
//! Almost everything below `cli` returns its result and lets a surface render
//! it. The exception is the best-effort failure: a cache that could not be
//! rewritten, a counter file that could not be saved, a `PROJECT_INFO.md` whose
//! `path:` line is now stale after a move. None of those may fail the operation
//! — the folders are the truth and the operation succeeded — but going silent
//! about them is how a library quietly stops being self-describing.
//!
//! Routing them all through here means there is exactly one sink to change when
//! a surface needs something other than stderr, and one thing for
//! `tests/layering.rs` to allow. A non-terminal surface renders warnings from
//! returned values; what reaches this function is what nothing above could have
//! reported.

use std::sync::Mutex;

/// Which kind of message reached the sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Warn,
    Note,
}

/// A surface that owns the screen. While one is installed, `warn` and `note`
/// hand it the message instead of writing to stderr — the guided app draws on
/// the alternate screen, where a stray `eprintln!` from a worker thread would
/// land in the middle of a frame and be scrolled away on exit.
type Sink = Box<dyn Fn(Level, &str) + Send + Sync>;

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// Route every later `warn` and `note` through `sink`. The guided app installs
/// one for as long as it owns the terminal and clears it on the way out.
pub fn set_sink(sink: Sink) {
    if let Ok(mut slot) = SINK.lock() {
        *slot = Some(sink);
    }
}

/// Back to stderr.
pub fn clear_sink() {
    if let Ok(mut slot) = SINK.lock() {
        *slot = None;
    }
}

/// `true` when a sink took the message.
fn delivered(level: Level, message: &str) -> bool {
    match SINK.lock() {
        Ok(slot) => match slot.as_ref() {
            Some(sink) => {
                sink(level, message);
                true
            }
            None => false,
        },
        // A poisoned lock means a sink panicked; stderr is the honest fallback.
        Err(_) => false,
    }
}

/// Report a best-effort failure that must not change what the operation did.
///
/// Write the message as a sentence about the file or the operation, without a
/// `warning:` prefix — this adds it, so every one of them looks the same.
pub fn warn(message: impl std::fmt::Display) {
    let message = message.to_string();
    if delivered(Level::Warn, &message) {
        return;
    }
    // Deliberately unstyled: this is the sink core writes through, and core does
    // not know whether anything is reading a terminal. The surfaces colour their
    // own output.
    eprintln!("warning: {message}");
}

/// Report something that happened which the caller could not have known about
/// and did not ask for — a partial project rolled back, a step skipped.
///
/// Distinct from [`warn`] because a rollback that *worked* is not a warning; it
/// is the one piece of code that knows a folder was removed saying so.
pub fn note(message: impl std::fmt::Display) {
    let message = message.to_string();
    if delivered(Level::Note, &message) {
        return;
    }
    eprintln!("note: {message}");
}

/// The last thing a process says before it stops.
///
/// Not for ordinary errors — those are `Result`s that reach `main`. This is for
/// the two paths that cannot return one: an armed failpoint calling `abort`, and
/// a data directory that cannot be resolved at all.
pub fn fatal(message: impl std::fmt::Display) {
    eprintln!("fastf: {message}");
}

#[cfg(test)]
mod tests {
    use super::{Level, clear_sink, set_sink, warn};
    use std::sync::{Arc, Mutex};

    /// `warn` writes to stderr, which a unit test cannot capture without
    /// redirecting the process. What is worth pinning is the shape of the
    /// message, which is why the prefix lives here rather than at 20 call sites.
    #[test]
    fn the_prefix_is_added_once_and_here() {
        let source = include_str!("diag.rs");
        assert_eq!(
            source.matches("\"warning: {message}\"").count(),
            1,
            "there is one warning prefix, and it is in this function"
        );
    }

    /// The sink is process-global, so this test owns it for its duration; the
    /// other test in this module never installs one.
    #[test]
    fn an_installed_sink_takes_the_message_without_its_prefix() {
        let seen: Arc<Mutex<Vec<(Level, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        set_sink(Box::new(move |level, message| {
            sink_seen.lock().unwrap().push((level, message.to_string()));
        }));
        warn("the cache could not be written");
        clear_sink();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, Level::Warn);
        assert_eq!(seen[0].1, "the cache could not be written");
    }
}
