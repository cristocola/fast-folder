//! The command surface: The ID counter, and what a base carries about it.
//!
//! Driven as a **real process** — see `common::mod`'s preamble for why.

mod common;

use common::{Sandbox, ids_in};
use std::fs;

/// `fastf id set` used to write one file that `Counters::floor` then ignored,
/// print "Global ID counter set to 0", and hand the next project ID0005.
/// The counter only moves up, so the honest answer to a lower value is a refusal.
#[test]
fn id_set_below_the_floor_is_refused() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["new", "race", "--name=One", "--yes", "--no-preview"]);
    sb.ok(&["new", "race", "--name=Two", "--yes", "--no-preview"]);

    let err = sb.fails(&["id", "set", "1"]);
    assert!(
        err.contains("cannot go below 2"),
        "the refusal must name the floor: {err}"
    );

    // And the floor is untouched — the next project follows the highest ID.
    sb.ok(&["new", "race", "--name=Three", "--yes", "--no-preview"]);
    let ids = ids_in(&sb.base);
    assert!(
        ids.contains(&"R0003".to_string()),
        "expected R0003 after the refusal, got {ids:?}"
    );
}

/// Deleting every project must not let the counter fall back and reissue IDs.
/// `fastf id reset` used to report success and change nothing at all.
#[test]
fn id_reset_is_gone_and_says_why() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.ok(&["new", "race", "--name=One", "--yes", "--no-preview"]);

    let err = sb.fails(&["id", "reset"]);
    assert!(
        err.contains("sync"),
        "the removal must point at the replacement: {err}"
    );
}

/// The headline of the counter design: three bases holding different
/// highest IDs must all converge on the largest one.
#[test]
fn id_sync_propagates_the_highest_id_to_every_base() {
    let sb = Sandbox::new();
    let bases = sb.with_bases(&["dir2", "dir3"]);
    sb.plant_project(&sb.base, "a", "ID0004");
    sb.plant_project(&bases[0], "b", "ID0082");
    sb.plant_project(&bases[1], "c", "ID0017");

    sb.ok(&["id", "sync"]);

    for base in [&sb.base, &bases[0], &bases[1]] {
        assert_eq!(
            sb.base_counter(base),
            82,
            "every base must record the global maximum, {} did not",
            base.display()
        );
    }
}

/// A base whose counter file outranks its own projects is authoritative — that
/// is what carries the number across a machine that cannot see the other bases.
///
/// Not a regression the floor could have caught (it already consulted base counters); this
/// pins the rule down so a future simplification of `floor` cannot drop it.
#[test]
fn a_base_counter_above_its_projects_is_authoritative() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.plant_project(&sb.base, "small", "ID0004");
    sb.ok(&["id", "set", "500"]);

    sb.ok(&["new", "race", "--name=Next", "--yes", "--no-preview"]);
    let ids = ids_in(&sb.base);
    assert!(
        ids.contains(&"R0501".to_string()),
        "the counter file must win over the projects: {ids:?}"
    );
}

/// Propagating the counter writes into every base, which bumps each base's
/// mtime — the same signal the index cache reads as "a project appeared".
/// Without re-stamping the cache, every create would force a full rescan of
/// every base, defeating the cache entirely.
///
/// Guards the cost of propagation rather than an old bug: propagation never
/// wrote other bases at all, so it passed this vacuously.
#[test]
fn propagating_the_counter_does_not_invalidate_other_bases_caches() {
    let sb = Sandbox::new();
    sb.write_template("race");
    let bases = sb.with_bases(&["dir2"]);
    sb.plant_project(&bases[0], "other", "ID0002");
    // Populate every cache.
    sb.ok(&["recent", "--plain"]);

    let cache = bases[0].join(".fastf-index.json");
    assert!(cache.is_file(), "the other base should have a cache");

    sb.ok(&["new", "race", "--name=Bump", "--yes", "--no-preview"]);

    let base_m = fs::metadata(&bases[0]).unwrap().modified().unwrap();
    let cache_m = fs::metadata(&cache).unwrap().modified().unwrap();
    assert!(
        cache_m >= base_m,
        "the other base's cache went stale after an unrelated create"
    );
}

/// A counter write that fails must say so.
///
/// The data-directory counter is the one that spans every base this machine has
/// written to, so it is what stops an unplugged drive restarting numbering. Its
/// two per-base siblings warn when they cannot be written; this one dropped the
/// error on the floor (`let _ = local.save()`), so the protection could be gone
/// with nothing on screen to say it.
///
/// A read-only data directory is what makes only the *write* fail: the config
/// still loads, the lock file already exists, and the atomic write cannot claim
/// its temp sibling. Unix-only because that is where the permission bit is a
/// one-liner; the code path it exercises is platform-independent.
#[cfg(unix)]
#[test]
fn a_failed_counter_write_warns_instead_of_going_quiet() {
    use std::os::unix::fs::PermissionsExt;

    let sb = Sandbox::new();
    sb.write_template("race");

    let set_mode = |mode: u32| {
        fs::set_permissions(&sb.install, fs::Permissions::from_mode(mode)).unwrap();
    };
    set_mode(0o555);
    let out = sb.run(&["new", "race", "--name=One", "--yes", "--no-preview"]);
    // Restore before any assertion, so a failure still leaves a removable tempdir.
    set_mode(0o755);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the project must still be created: {out:?}"
    );
    assert!(
        stderr.contains("could not record the ID counter"),
        "a dropped counter write must be reported: {stderr}"
    );
    assert_eq!(ids_in(&sb.base), ["R0001".to_string()]);
}

/// Creating one project loads each thing a small, bounded number of times.
///
/// A design guard rather than a regression test: the two template parses are
/// deliberate (the preview outside the data lock, then the authority inside it,
/// which is what stops two racing creates minting the same ID), and so are the
/// base scans — `library::max_id` must stay read-only, so it never leaves a
/// cache behind for the next call. What this pins is that none of those numbers
/// grows: a third parse or a fourth scan means something started reloading.
#[cfg(debug_assertions)]
#[test]
fn a_create_does_not_reload_the_same_things_over_and_over() {
    let sb = Sandbox::new();
    sb.write_template("race");

    let trace = sb.traced(&["new", "race", "--name=Solo", "--yes"]);

    assert!(
        trace.count("template_load") <= 2,
        "the template should be parsed at most twice (preview, then under the \
         lock), traced {}",
        trace.summary()
    );
    assert!(
        trace.count("scan_base") <= 3,
        "a create should not rescan every base repeatedly, traced {}",
        trace.summary()
    );
}

/// `template list` prints names and descriptions. It has no reason to read a
/// single template file, and it used to read all of them.
#[cfg(debug_assertions)]
#[test]
fn listing_templates_reads_no_template_file_contents() {
    let sb = Sandbox::new();
    sb.write_template("race");
    sb.write_template("other");

    let trace = sb.traced(&["template", "list"]);

    assert!(
        trace.count("template_load") >= 2,
        "the manifests must still be parsed, traced {}",
        trace.summary()
    );
    assert_eq!(
        trace.count("template_file_scan"),
        0,
        "listing must not read template file contents, traced {}",
        trace.summary()
    );
}

/// `fastf id set` accepted any value above the floor, `u64::MAX` included — and
/// then the next create computed `value + 1` and overflowed: a panic in a debug
/// build, a silent wrap to zero in a release one. Both ends are now bounded.
#[test]
fn the_counter_has_a_maximum_and_stops_cleanly_at_it() {
    const MAX: u64 = 999_999_999_999;

    let sb = Sandbox::new();
    sb.write_template("race");

    let err = sb.fails(&["id", "set", &(MAX + 1).to_string()]);
    assert!(
        err.contains(&MAX.to_string()),
        "the refusal must name the maximum: {err}"
    );
    let err = sb.fails(&["id", "set", &u64::MAX.to_string()]);
    assert!(
        err.contains(&MAX.to_string()),
        "the refusal must name the maximum: {err}"
    );

    // The maximum itself is a legal setting.
    sb.ok(&["id", "set", &MAX.to_string()]);

    // And the create that would have to mint MAX + 1 fails, saying so, without
    // leaving a folder behind.
    let before = fs::read_dir(&sb.base).unwrap().count();
    let err = sb.fails(&["new", "race", "--name=Overflow", "--yes", "--no-preview"]);
    assert!(
        err.contains("maximum") && err.contains(&MAX.to_string()),
        "the create must name the maximum it hit: {err}"
    );
    assert_eq!(
        fs::read_dir(&sb.base).unwrap().count(),
        before,
        "no folder may be created once the counter is exhausted"
    );
}
