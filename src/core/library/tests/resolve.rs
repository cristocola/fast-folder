//! A query to one project: the tiers, and what is said when there are several.

use super::*;

#[test]
fn resolve_by_id_prefix_and_name() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(
        base,
        "music_video_alpha",
        "ID0042",
        "mv",
        "2026-01-01T00:00:00Z",
    );
    write_project(
        base,
        "research_beta",
        "ID0100",
        "rn",
        "2026-02-01T00:00:00Z",
    );
    let cfg = cfg_for(base, &[]);

    // Exact id.
    assert_eq!(resolve(&cfg, "ID0042").unwrap().name, "music_video_alpha");
    // Id prefix (unique).
    assert_eq!(resolve(&cfg, "ID004").unwrap().id, "ID0042");
    // Name substring (case-insensitive).
    assert_eq!(resolve(&cfg, "BETA").unwrap().id, "ID0100");
    // The name as it was typed, before a transform put `_` in the folder.
    assert_eq!(resolve(&cfg, "Music Video").unwrap().id, "ID0042");
    assert_eq!(resolve(&cfg, "video-alpha").unwrap().id, "ID0042");
    // No match.
    assert!(resolve(&cfg, "nope").is_err());
}

/// `resolve` has three distinct outcomes and each must stay distinguishable:
/// nothing matched, exactly one, or an ambiguous set.
#[test]
fn resolve_distinguishes_no_match_exact_and_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "alpha_one", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_project(base, "alpha_two", "ID0012", "gen", "2026-01-02T00:00:00Z");
    let cfg = cfg_for(base, &[]);

    // Exactly one → the project.
    assert_eq!(resolve(&cfg, "ID0001").unwrap().name, "alpha_one");
    assert_eq!(resolve(&cfg, "alpha_two").unwrap().id, "ID0012");

    // Nothing matched → a "no project matches" error, not an ambiguity one.
    let err = resolve(&cfg, "nothing_like_this").unwrap_err().to_string();
    assert!(
        err.contains("no project matches"),
        "expected a not-found error, got: {err}"
    );

    // Several matched → an ambiguity error listing the candidates.
    let err = resolve(&cfg, "alpha").unwrap_err().to_string();
    assert!(
        err.contains("ambiguous") && err.contains("ID0001") && err.contains("ID0012"),
        "expected an ambiguity error naming the candidates, got: {err}"
    );

    // An exact ID wins over a prefix that would also match it.
    assert_eq!(resolve(&cfg, "ID0001").unwrap().id, "ID0001");
}

#[test]
fn resolve_ambiguous_errors_with_candidates() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "shared_one", "ID0011", "gen", "2026-01-01T00:00:00Z");
    write_project(base, "shared_two", "ID0012", "gen", "2026-02-01T00:00:00Z");
    let cfg = cfg_for(base, &[]);

    // "shared" matches both by name substring.
    let err = resolve(&cfg, "shared").unwrap_err().to_string();
    assert!(err.contains("ambiguous"), "err: {err}");
    assert!(err.contains("ID0011") && err.contains("ID0012"));
}

/// `fastf open 37` must find ID0037. Before the numeric tier, "4" fell through
/// to the ID-prefix search and matched every ID starting with a 4 — so in a
/// library holding ID0004 alongside ID0040..ID0049 the obvious query was
/// ambiguous.
#[test]
fn an_all_digit_query_matches_the_id_number_exactly() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "the_one", "ID0004", "gen", "2026-01-01T00:00:00Z");
    for n in 40..50 {
        write_project(
            base,
            &format!("decoy_{n}"),
            &format!("ID00{n}"),
            "gen",
            "2026-01-02T00:00:00Z",
        );
    }
    let cfg = cfg_for(base, &[]);

    assert_eq!(resolve(&cfg, "4").unwrap().id, "ID0004");
    // Leading zeros are the same number.
    assert_eq!(resolve(&cfg, "0004").unwrap().id, "ID0004");
    // A number nothing holds does not silently widen into a prefix match; it
    // falls through to the ordinary name search, which here finds `decoy_47`.
    assert_eq!(resolve(&cfg, "7").unwrap().id, "ID0047");
}

/// Exact ID → numeric → ID prefix. Exact stays first because a template may
/// declare a digits-only ID prefix, which makes an all-digits query a legal
/// complete ID; the prefix tier stays last because it is the broadest.
#[test]
fn the_numeric_tier_sits_between_exact_id_and_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    // Literally `4` — a digits-only ID prefix with no padding.
    write_project(base, "bare_four", "4", "gen", "2026-01-01T00:00:00Z");
    write_project(base, "padded_four", "ID0004", "gen", "2026-01-02T00:00:00Z");
    // Starts with "4", but its number is 4200.
    write_project(base, "far_away", "4200", "gen", "2026-01-03T00:00:00Z");
    let cfg = cfg_for(base, &[]);

    // Exact ID beats the numeric tier.
    assert_eq!(resolve(&cfg, "4").unwrap().name, "bare_four");

    // With no exact holder, the numeric tier answers and the prefix tier —
    // which would also have matched 4200 — never runs.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "padded_four", "ID0004", "gen", "2026-01-02T00:00:00Z");
    write_project(base, "far_away", "4200", "gen", "2026-01-03T00:00:00Z");
    let cfg = cfg_for(base, &[]);
    assert_eq!(resolve(&cfg, "4").unwrap().id, "ID0004");
}

/// A digit run too long for `u64` is not a number; it must fall through to the
/// ordinary tiers rather than panicking or matching nothing forever.
#[test]
fn a_numeric_query_too_large_for_u64_falls_through() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(
        base,
        "big_99999999999999999999999",
        "ID0001",
        "gen",
        "2026-01-01T00:00:00Z",
    );
    let cfg = cfg_for(base, &[]);

    // Falls past the numeric tier into the name substring search.
    assert_eq!(
        resolve(&cfg, "99999999999999999999999").unwrap().id,
        "ID0001"
    );
}

/// The structured resolver hands the candidates back as data, which is what
/// lets a caller offer them to a picker instead of an error string.
#[test]
fn resolve_matches_reports_many_without_erroring() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "shared_one", "ID0011", "gen", "2026-01-01T00:00:00Z");
    write_project(base, "shared_two", "ID0012", "gen", "2026-02-01T00:00:00Z");
    write_project(
        base,
        "shared_three",
        "ID0013",
        "gen",
        "2026-03-01T00:00:00Z",
    );
    let cfg = cfg_for(base, &[]);

    match resolve_matches(&cfg, "shared") {
        Resolution::Many(candidates) => {
            assert_eq!(candidates.len(), 3);
            let mut ids: Vec<&str> = candidates.iter().map(|p| p.id.as_str()).collect();
            ids.sort_unstable();
            assert_eq!(ids, ["ID0011", "ID0012", "ID0013"]);
        }
        other => panic!("expected Many, got {other:?}"),
    }

    assert!(matches!(
        resolve_matches(&cfg, "ID0011"),
        Resolution::One(_)
    ));
    assert!(matches!(
        resolve_matches(&cfg, "nothing_like_this"),
        Resolution::NoMatch
    ));

    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        resolve_matches(&cfg_for(empty.path(), &[]), "anything"),
        Resolution::NoProjects
    ));
}
