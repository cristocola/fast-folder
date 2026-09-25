//! One clock.
//!
//! `now_iso8601` lived in `core::library`, which is why `project_info` and
//! `provisioning` — neither of which has anything to do with the project library
//! — both imported it. A timestamp is not a library concern.

/// Current UTC timestamp, ISO-8601 with seconds precision.
///
/// Seconds, not milliseconds, because this string is compared lexicographically
/// (`created > "2026-01-01"` in a search, journal entries sorted as text) and a
/// fixed-width representation is what makes that correct.
pub fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Current UTC timestamp with milliseconds, for a log line: fixed width, so a
/// log still sorts as text, and fine enough to order the steps of one move.
pub fn now_iso8601_millis() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// An ISO-8601 stamp as a person reads it here: local time, to the second,
/// `2026-09-25 16:03:11`. Anything that does not parse is shown as it is.
pub fn local_readable(stamp: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(stamp) {
        Ok(at) => at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => stamp.to_string(),
    }
}

/// The local wall-clock time as `HH:MM:SS` — what a message log stamps a
/// line with, for a person reading it back a minute later.
pub fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::now_iso8601;

    #[test]
    fn the_stamp_is_fixed_width_utc_and_sorts_as_text() {
        let now = now_iso8601();
        assert_eq!(now.len(), 20, "fixed width: {now}");
        assert!(now.ends_with('Z'), "UTC: {now}");
        assert_eq!(&now[4..5], "-");
        assert_eq!(&now[10..11], "T");
        // Lexicographic order is chronological order, which is what the search
        // predicates and the journal both rely on.
        assert!("2026-01-01T00:00:00Z" < now.as_str());
    }
}
