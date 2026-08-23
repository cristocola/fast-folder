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
//! `tests/layering.rs` to allow. `fastf ui` renders warnings its own way from
//! returned values; what reaches this function is what nothing above could have
//! reported.

/// Report a best-effort failure that must not change what the operation did.
///
/// Write the message as a sentence about the file or the operation, without a
/// `warning:` prefix — this adds it, so every one of them looks the same.
pub fn warn(message: impl std::fmt::Display) {
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
}
