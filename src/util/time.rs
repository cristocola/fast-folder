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
