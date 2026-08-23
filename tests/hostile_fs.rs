//! The filesystem behaving badly: corrupt state, unreadable files, things
//! vanishing mid-operation.
//!
//! The rule under test is always the same — **degrade, never panic, never lose
//! data**. The folders are the source of truth, so every cache, marker and index
//! is disposable and must be reconstructible from them. A user should be able to
//! delete, truncate or hand-mangle any of fastf's bookkeeping and have the next
//! command quietly put it right.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::{
    config::Config, counter::Counters, library, project, project_info, provisioning, template,
};

mod common;

static SERIAL: Mutex<()> = Mutex::new(());

/// Fresh install dir + base, with HOME redirected — see `common::env`.
fn sandbox<R>(body: impl FnOnce(&Path, &Path) -> R) -> R {
    common::env::with_sandbox(&SERIAL, |sb| body(&sb.install, &sb.base))
}

fn write_hostile_template(install: &Path, slug: &str) {
    let dir = install.join("templates").join(slug);
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(
        dir.join("template.yaml"),
        format!(
            "name: T\nslug: {slug}\nnaming_pattern: \"{{id}}_proj\"\n\
             id:\n  prefix: H\n  digits: 3\n"
        ),
    )
    .unwrap();
    fs::write(dir.join("files/README.md"), "# hello\n").unwrap();
}

fn write_project(base: &Path, folder: &str, frontmatter: &str) -> std::path::PathBuf {
    let dir = base.join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(project_info::pinfo_path(&dir), frontmatter).unwrap();
    dir
}

fn valid_frontmatter(id: &str, folder: &str) -> String {
    format!(
        "---\nid: {id}\ntemplate: t\ntemplate_name: T\n\
         created: 2026-01-01T00:00:00Z\nfolder: {folder}\npath: x\n\
         variables: {{}}\ntags: []\n---\n"
    )
}

fn config_for(base: &Path) -> Config {
    let mut cfg = Config::default();
    cfg.base_dir = base.display().to_string();
    cfg
}

/// A corrupt base cache must be silently rebuilt from the folders.
#[test]
fn corrupt_cache_self_heals() {
    sandbox(|_install, base| {
        write_project(base, "proj_a", &valid_frontmatter("ID0001", "proj_a"));
        let cfg = config_for(base);
        assert_eq!(library::discover(&cfg).len(), 1);

        for garbage in [
            "",                                // empty
            "{",                               // truncated JSON
            "null",                            // valid JSON, wrong shape
            "{\"version\":99,\"entries\":[]}", // future version, empty
            "\u{0}\u{1}\u{2}not json at all",  // binary noise
        ] {
            fs::write(base.join(library::CACHE_FILENAME), garbage).unwrap();
            let found = library::discover(&cfg);
            assert_eq!(
                found.len(),
                1,
                "cache {garbage:?} should have been rebuilt from the folders"
            );
            assert_eq!(found[0].id, "ID0001");
        }
    });
}

/// Projects whose metadata cannot be parsed are skipped, and must not take the
/// rest of the library down with them.
#[test]
fn unparseable_metadata_is_skipped_not_fatal() {
    sandbox(|_install, base| {
        write_project(base, "good", &valid_frontmatter("ID0001", "good"));
        write_project(base, "no_frontmatter", "just some notes, no YAML here\n");
        write_project(base, "truncated", "---\nid: ID0002\ntemplate: t\n");
        write_project(base, "bad_yaml", "---\nid: [unclosed\n---\n");
        write_project(base, "empty", "");

        let cfg = config_for(base);
        let found = library::discover(&cfg);
        assert_eq!(
            found.len(),
            1,
            "only the well-formed project should surface, got {:?}",
            found.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
        assert_eq!(found[0].id, "ID0001");

        // And the counter self-heal must not choke on the broken ones.
        assert_eq!(library::max_id(&cfg), 1);
    });
}

/// `PROJECT_INFO.md` existing as a *directory* is nonsense, but must not panic.
#[test]
fn metadata_as_a_directory_is_not_fatal() {
    sandbox(|_install, base| {
        write_project(base, "good", &valid_frontmatter("ID0001", "good"));
        let weird = base.join("weird");
        fs::create_dir_all(weird.join("PROJECT_INFO.md")).unwrap();

        let cfg = config_for(base);
        let found = library::discover(&cfg);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "good");
        // reconcile walks the same folders and must also survive it.
        let _ = provisioning::reconcile_unlocked(&cfg);
    });
}

/// Corrupt provisioning markers must be reported without being parsed or changed.
#[test]
fn corrupt_markers_are_untouched_and_cannot_touch_data() {
    sandbox(|_install, base| {
        let dir = write_project(base, "proj", &valid_frontmatter("ID0001", "proj"));
        fs::write(dir.join("payload.txt"), "irreplaceable").unwrap();

        // Unparseable create marker.
        fs::write(dir.join(".fastf-provisioning.json"), "{not json").unwrap();
        // Unparseable move marker at the base root.
        fs::write(base.join(".fastf-move-proj.json"), "><").unwrap();

        let create_marker = dir.join(".fastf-provisioning.json");
        let move_marker = base.join(".fastf-move-proj.json");
        let create_before = fs::read(&create_marker).unwrap();
        let move_before = fs::read(&move_marker).unwrap();
        let cfg = config_for(base);
        let report = provisioning::reconcile_unlocked(&cfg);

        assert_eq!(
            fs::read_to_string(dir.join("payload.txt")).unwrap(),
            "irreplaceable",
            "a corrupt marker must never license touching real data"
        );
        assert!(dir.is_dir(), "the project itself must survive");
        assert_eq!(report.obsolete.len(), 2, "report: {report:?}");
        assert_eq!(fs::read(create_marker).unwrap(), create_before);
        assert_eq!(fs::read(move_marker).unwrap(), move_before);
    });
}

/// A move marker with dangling paths is obsolete and cannot authorize cleanup.
#[test]
fn move_marker_with_dangling_paths_is_reported_without_mutation() {
    sandbox(|_install, base| {
        let dir = write_project(base, "proj", &valid_frontmatter("ID0001", "proj"));
        fs::write(dir.join("keep.txt"), "still here").unwrap();

        let missing_src = base.join("vanished_source");
        let missing_temp = base.join(".vanished.fastf-part");
        let missing_final = base.join("never_committed");
        // Planted as bytes on purpose: fastf has no writer for this format any
        // more, and reconcile must report it without parsing a single path.
        fs::write(
            base.join(".fastf-move-vanished.json"),
            format!(
                r#"{{"version":1,"started_at":"2026-01-01T00:00:00Z","src":{src},"temp":{temp},"final_path":{final_path},"phase":"copying","id":"ID9999"}}"#,
                src = serde_json::to_string(&missing_src.display().to_string()).unwrap(),
                temp = serde_json::to_string(&missing_temp.display().to_string()).unwrap(),
                final_path =
                    serde_json::to_string(&missing_final.display().to_string()).unwrap(),
            ),
        )
        .unwrap();

        let cfg = config_for(base);
        let report = provisioning::reconcile_unlocked(&cfg);
        assert_eq!(report.rolled_back, 0, "report: {report:?}");
        assert_eq!(report.obsolete.len(), 1, "report: {report:?}");
        assert!(
            base.join(".fastf-move-vanished.json").is_file(),
            "obsolete marker must remain"
        );
        assert!(
            fs::read_to_string(dir.join("keep.txt")).unwrap() == "still here",
            "an unrelated project must not be touched"
        );
    });
}

/// A missing base directory is an ordinary state (unplugged drive), not an error.
#[test]
fn absent_base_is_treated_as_empty() {
    sandbox(|_install, base| {
        let mut cfg = config_for(base);
        cfg.bases = vec![
            base.parent()
                .unwrap()
                .join("not_mounted")
                .display()
                .to_string(),
        ];

        // No panic, and the present base still works.
        write_project(base, "proj", &valid_frontmatter("ID0001", "proj"));
        let found = library::discover(&cfg);
        assert_eq!(found.len(), 1);
        assert_eq!(library::max_id(&cfg), 1);
        let _ = provisioning::reconcile_unlocked(&cfg);
    });
}

/// Creating into a base that disappears between plan and create must fail
/// cleanly — the plan holds a path, not a guarantee.
#[test]
fn base_vanishing_between_plan_and_create_fails_cleanly() {
    sandbox(|install, base| {
        write_hostile_template(install, "t");
        let cfg = config_for(base);
        let tmpl = template::find_by_slug("t").unwrap();
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();

        // The base goes away after planning.
        fs::remove_dir_all(base).unwrap();

        // Either the create recreates the tree and succeeds, or it fails — both
        // are acceptable. What is not acceptable is a panic or a half-project.
        match project::create(&plan, &tmpl, &mut counters, &cfg, false) {
            Ok(_realized) => {
                assert!(plan.root_path.join("README.md").is_file());
                assert!(!project_info::is_provisioning(&plan.root_path));
            }
            Err(_) => {
                assert!(
                    !plan.root_path.exists(),
                    "a failed create must leave no partial project"
                );
                assert_eq!(Counters::load().unwrap().get(), 0, "no ID burned");
            }
        }
    });
}

/// A read-only base must produce an error, not a panic or a partial write.
#[cfg(unix)]
#[test]
fn read_only_base_errors_cleanly() {
    use std::os::unix::fs::PermissionsExt;

    sandbox(|install, base| {
        write_hostile_template(install, "t");
        let cfg = config_for(base);
        let tmpl = template::find_by_slug("t").unwrap();
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();

        let mut perms = fs::metadata(base).unwrap().permissions();
        perms.set_mode(0o555); // r-x, no write
        fs::set_permissions(base, perms).unwrap();

        let result = project::create(&plan, &tmpl, &mut counters, &cfg, false);

        // Restore before asserting so the tempdir can always be cleaned up.
        let mut perms = fs::metadata(base).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(base, perms).unwrap();

        assert!(result.is_err(), "writing into a read-only base must fail");
        assert_eq!(Counters::load().unwrap().get(), 0, "no ID burned");
    });
}

/// The counter file being destroyed must not hand out an ID that already exists.
#[test]
fn destroyed_counter_self_heals_from_the_projects_on_disk() {
    sandbox(|install, base| {
        write_hostile_template(install, "t");
        write_project(base, "a", &valid_frontmatter("H007", "a"));
        write_project(base, "b", &valid_frontmatter("H042", "b"));

        let cfg = config_for(base);
        for garbage in ["", "global = ", "not toml at all", "global = -1"] {
            fs::write(install.join("counters.toml"), garbage).unwrap();
            // A corrupt counter file may fail to parse; either way the floor
            // must come from the projects that actually exist.
            let counters = Counters::load().unwrap_or_default();
            let plan = project::plan(
                &template::find_by_slug("t").unwrap(),
                &HashMap::new(),
                &cfg,
                &counters,
            )
            .unwrap();
            assert_eq!(
                plan.id_str, "H043",
                "counter {garbage:?} must self-heal above the highest existing ID"
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Path fidelity and bounded recursion (v1.7.1)
// ---------------------------------------------------------------------------

/// A pathologically deep tree degrades instead of blowing the stack.
///
/// A design guard, not a regression: 300 frames does not overflow a thread
/// stack, so this could not be made to fail against the old build without
/// building a tree deep enough to crash the test runner. What it pins is that
/// the limit exists and reports where it stopped — every walk in the tool is
/// plain recursion, and two of them (`tree_size`, and the browser's size scan)
/// run over whatever folder a user points at.
#[test]
fn a_very_deep_tree_is_refused_rather_than_overflowing_the_stack() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let deep = tmp.path().join("Deep");
    let mut path = deep.clone();
    for level in 0..300 {
        path = path.join(format!("l{level}"));
    }
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("leaf.txt"), b"x").unwrap();

    // The walk reports a failure; it does not abort, and it does not return a
    // number that is missing most of the tree.
    let walked = fastf::core::assets::walk(&deep);
    assert!(
        walked.is_err(),
        "a tree past the depth limit should be refused"
    );
    let message = format!("{:#}", walked.unwrap_err());
    assert!(
        message.contains("too deep") || message.contains("depth"),
        "the error should say what happened: {message}"
    );
}
