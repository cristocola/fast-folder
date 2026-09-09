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

/// This binary's lock over the process environment — see `common::env`.
static SERIAL: Mutex<()> = Mutex::new(());

/// Fresh install dir + base, with HOME redirected — see `common::env`.
fn sandbox<R>(body: impl FnOnce(&Path, &Path) -> R) -> R {
    common::env::with_sandbox(&SERIAL, |sb, _guard| body(&sb.install, &sb.base))
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

/// Everything `core` said through [`fastf::util::diag`] while `body` ran.
///
/// The sink is process-global, so this is only sound inside `sandbox()`, which
/// holds this binary's `SERIAL` for the length of the body. The guard clears
/// the sink in `Drop` rather than after the call, so a panicking assertion
/// cannot leave the next test collecting into a dead `Vec`.
fn collecting_diag<R>(body: impl FnOnce() -> R) -> (R, Vec<String>) {
    struct Installed(std::sync::Arc<Mutex<Vec<String>>>);
    impl Drop for Installed {
        fn drop(&mut self) {
            fastf::util::diag::clear_sink();
        }
    }

    let collected: std::sync::Arc<Mutex<Vec<String>>> = std::sync::Arc::default();
    let sink = std::sync::Arc::clone(&collected);
    fastf::util::diag::set_sink(Box::new(move |_level, message| {
        sink.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(message.to_string());
    }));
    let guard = Installed(collected);
    let out = body();
    let messages = guard.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
    (out, messages)
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
/// rest of the library down with them — **and each one says so.**
///
/// Skipping in silence is what made a project disappear: `docs/projects.md`
/// invites the user to edit this file, and a single bad line took the folder
/// out of `recent`, `search` and the app with nothing said anywhere and
/// `reindex` reporting the smaller count as a success. A folder fastf cannot
/// read is a folder the user still has.
#[test]
fn unparseable_metadata_is_skipped_but_never_in_silence() {
    sandbox(|_install, base| {
        write_project(base, "good", &valid_frontmatter("ID0001", "good"));
        write_project(base, "no_frontmatter", "just some notes, no YAML here\n");
        write_project(base, "truncated", "---\nid: ID0002\ntemplate: t\n");
        write_project(base, "bad_yaml", "---\nid: [unclosed\n---\n");
        write_project(base, "empty", "");

        let cfg = config_for(base);
        let (found, said) = collecting_diag(|| library::discover(&cfg));
        assert_eq!(
            found.len(),
            1,
            "only the well-formed project should surface, got {:?}",
            found.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
        assert_eq!(found[0].id, "ID0001");

        for folder in ["no_frontmatter", "truncated", "bad_yaml", "empty"] {
            assert!(
                said.iter().any(|m| m.contains(folder)),
                "{folder} was dropped without a word; said: {said:?}"
            );
        }
        assert!(
            said.iter().all(|m| !m.contains("good")),
            "the readable project must not be warned about: {said:?}"
        );

        // And the counter self-heal must not choke on the broken ones.
        assert_eq!(library::max_id(&cfg), 1);
    });
}

/// An ordinary folder is not a broken project. Only a `PROJECT_INFO.md` that
/// exists and will not open earns a warning — otherwise every base with a
/// `Downloads` in it would warn on every discovery, and a warning nobody can
/// act on teaches people to ignore the ones they can.
#[test]
fn a_folder_with_no_metadata_at_all_is_passed_over_in_silence() {
    sandbox(|_install, base| {
        write_project(base, "good", &valid_frontmatter("ID0001", "good"));
        fs::create_dir_all(base.join("just_a_folder")).unwrap();

        let cfg = config_for(base);
        let (found, said) = collecting_diag(|| library::discover(&cfg));
        assert_eq!(found.len(), 1);
        assert!(said.is_empty(), "nothing to report, but said {said:?}");
    });
}

/// A hand-edit that drops a field which is not the project's identity must not
/// drop the project.
///
/// Deserialization is all-or-nothing, so one missing `created:` line used to
/// mean `read_project_meta` answered `None` and the folder stopped being a
/// project. `created`, `folder`, `path` and `template_name` are none of them
/// identity — `created` already had a fallback, and `folder`/`path` are
/// re-derived from the directory and never read — so they default now, and
/// only `id` and `template` are load-bearing.
#[test]
fn metadata_missing_a_field_that_is_not_identity_is_still_a_project() {
    sandbox(|_install, base| {
        let full = valid_frontmatter("ID0001", "good");
        for (folder, dropped) in [
            ("no_created", "created:"),
            ("no_folder", "folder:"),
            ("no_path", "path:"),
            ("no_template_name", "template_name:"),
        ] {
            let trimmed: String = full
                .lines()
                .filter(|line| !line.starts_with(dropped))
                .map(|line| format!("{line}\n"))
                .collect();
            write_project(base, folder, &trimmed);
        }

        let cfg = config_for(base);
        let (found, said) = collecting_diag(|| library::discover(&cfg));
        let mut names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["no_created", "no_folder", "no_path", "no_template_name"],
            "a non-identity field is not what makes a folder a project"
        );
        assert!(said.is_empty(), "nothing is wrong here, but said {said:?}");
        // `created` falls back to the folder's own timestamp rather than being
        // left empty, so sorting the library still has something to sort on.
        assert!(
            found.iter().all(|p| !p.created.trim().is_empty()),
            "created should fall back to the folder's own date"
        );
    });
}

/// Bytes that are not UTF-8 are the realistic version of this: a shared drive
/// two operating systems mount, and one editor on the Windows side that saves
/// as the ANSI codepage. The project is not readable and must be named.
#[test]
fn metadata_that_is_not_utf8_is_named_rather_than_dropped_quietly() {
    sandbox(|_install, base| {
        write_project(base, "good", &valid_frontmatter("ID0001", "good"));
        let dir = base.join("ansi");
        fs::create_dir_all(&dir).unwrap();
        let mut bytes = valid_frontmatter("ID0002", "ansi").into_bytes();
        // A lone 0xE9 — `é` as cp1252 wrote it, invalid on its own in UTF-8.
        let at = bytes
            .windows(7)
            .position(|w| w == b"folder:")
            .expect("frontmatter has a folder key")
            + 8;
        bytes[at] = 0xE9;
        fs::write(project_info::pinfo_path(&dir), bytes).unwrap();

        let cfg = config_for(base);
        let (found, said) = collecting_diag(|| library::discover(&cfg));
        assert_eq!(found.len(), 1, "the unreadable one is not in the library");
        assert!(
            said.iter().any(|m| m.contains("ansi")),
            "the folder must be named: {said:?}"
        );
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
// Path fidelity and bounded recursion
// ---------------------------------------------------------------------------

/// A pathologically deep tree degrades instead of blowing the stack.
///
/// Unix only, and for two separate reasons that both belong to the setup rather
/// than to fastf. Building a 100-level tree needs a path past Windows'
/// MAX_PATH, which `create_dir_all` refuses without long-path support; and a
/// Windows test thread's 1 MiB stack is what proved the *old* 256-level limit
/// was too generous in the first place. The limit itself is cross-platform and
/// is what this pins: the walk stops and says where.
#[cfg(unix)]
#[test]
fn a_very_deep_tree_is_refused_rather_than_overflowing_the_stack() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let deep = tmp.path().join("Deep");
    let mut path = deep.clone();
    for level in 0..100 {
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

/// The same limit, on the walk a cross-device move and every `copy-to` run.
///
/// `assets::walk` above was the only one of the crate's five recursive walks
/// with a *live* guard. Three others checked `depth` in a `_at` function and
/// then recursed through the wrapper that starts at zero, so the limit could
/// never be reached: this one, `template_import::scan_dir_at` below, and
/// `util::tree_size`. Deny-by-default is the manifest's whole contract — it
/// fails a move rather than omitting anything — so an unbounded recursion here
/// is a stack overflow in the middle of the one operation that must not be
/// interrupted.
#[cfg(unix)]
#[test]
fn a_move_manifest_refuses_a_tree_past_the_depth_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let deep = tmp.path().join("Deep");
    let mut path = deep.clone();
    for level in 0..100 {
        path = path.join(format!("l{level}"));
    }
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("leaf.txt"), b"x").unwrap();

    let scanned = fastf::core::transactions::MoveManifest::scan(&deep);
    assert!(scanned.is_err(), "a tree past the limit must be refused");
    let message = format!("{:#}", scanned.unwrap_err());
    assert!(
        message.contains("too deep") || message.contains("depth"),
        "the error should say what happened: {message}"
    );
}

/// And on the walk `fastf template from-folder` reads a project with.
///
/// This one could *build* the deep tree as well as walk it: the `structure:`
/// it generates is as deep as the folder it read, which is then recursed over
/// again by validation, interpolation and the create.
#[cfg(unix)]
#[test]
fn reading_a_template_out_of_a_folder_refuses_a_tree_past_the_depth_limit() {
    sandbox(|_install, base| {
        let source = base.join("Deep");
        let mut path = source.clone();
        for level in 0..100 {
            path = path.join(format!("l{level}"));
        }
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("leaf.txt"), b"x").unwrap();

        let planned = fastf::core::template_import::from_folder(&source, "deep-one", false, false);
        assert!(planned.is_err(), "a tree past the limit must be refused");
        let message = format!("{:#}", planned.unwrap_err());
        assert!(
            message.contains("too deep") || message.contains("depth"),
            "the error should say what happened: {message}"
        );
    });
}

// ---------------------------------------------------------------------------
// Writes never follow a link out of the tree they are meant to fill
// ---------------------------------------------------------------------------

/// `SafeRelativePath` proves the *text* of `docs/new.md` cannot escape the apply
/// target. Nothing proved the same about the filesystem, and `create_dir_all`
/// walks straight through an existing `docs -> outside`: the file landed outside
/// the folder while every lexical check passed.
#[cfg(unix)]
#[test]
fn apply_refuses_to_write_through_a_link_in_the_target() {
    use std::os::unix::fs::symlink;

    sandbox(|install, base| {
        let dir = install.join("templates").join("linky");
        fs::create_dir_all(dir.join("files/docs")).unwrap();
        fs::write(
            dir.join("template.yaml"),
            "name: T\nslug: linky\nnaming_pattern: \"{id}\"\nid:\n  prefix: L\n  digits: 3\n",
        )
        .unwrap();
        fs::write(dir.join("files/docs/new.md"), "template body\n").unwrap();

        let target = base.join("target");
        let outside = base.join("outside");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, target.join("docs")).unwrap();

        let tmpl = template::find_by_slug("linky").unwrap();
        let error = project::apply(&tmpl, &target, &HashMap::new(), &Config::default())
            .expect_err("apply must refuse to write through the link")
            .to_string();

        assert!(
            error.contains("refusing to write through a link"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("docs"),
            "the error must name the link: {error}"
        );
        assert!(
            !outside.join("new.md").exists(),
            "the file was written outside the apply target"
        );
        assert_eq!(
            fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing may be created beyond the link"
        );
    });
}

/// The same rule on the way in. `template from-folder --force` reuses an
/// existing template directory, so a `files/sub -> outside` planted there would
/// send the bundle out of the templates directory.
#[cfg(unix)]
#[test]
fn template_ingestion_refuses_a_pre_planted_link_before_writing_a_byte() {
    use std::os::unix::fs::symlink;

    sandbox(|install, base| {
        // A source project with a nested binary worth bundling.
        let source = base.join("source");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("sub/asset.bin"), [0u8, 159, 146, 150]).unwrap();

        // A template directory that already exists, with a hostile link inside.
        let outside = install.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let files = install.join("templates").join("ingest").join("files");
        fs::create_dir_all(&files).unwrap();
        symlink(&outside, files.join("sub")).unwrap();

        let result = fastf::core::operations::template_from_folder(&source, "ingest", true, true);

        // Either the pre-existing directory is cleared first (in which case the
        // link is gone and the bundle is contained), or the write is refused.
        // What must never happen is a byte landing beyond the link.
        assert_eq!(
            fs::read_dir(&outside).unwrap().count(),
            0,
            "the bundle was written through the link: {result:?}"
        );
    });
}

// ---------------------------------------------------------------------------
// A cache entry is a hint, and never authorizes a path outside its base
// ---------------------------------------------------------------------------

/// Write a cache file whose entries name arbitrary directories, then make it
/// look **fresh**: the staleness gate compares the base's mtime to the cache
/// file's, and overwriting a file in place does not bump the directory's mtime,
/// so a planted cache is trusted without a rescan. This is the delivery route —
/// caches travel with the projects by design, so a synced folder or an unpacked
/// archive brings one along.
fn plant_cache(base: &Path, dirs: &[&str]) {
    let entries: Vec<String> = dirs
        .iter()
        .enumerate()
        .map(|(i, dir)| {
            let escaped = dir.replace('\\', "\\\\");
            format!(
                "{{\"dir\":\"{escaped}\",\"id\":\"ID900{i}\",\"template\":\"t\",\
                 \"template_name\":\"T\",\"name\":\"forged\",\
                 \"created\":\"2026-01-01T00:00:00Z\",\"tags\":[]}}"
            )
        })
        .collect();
    fs::write(
        base.join(library::CACHE_FILENAME),
        format!("{{\"version\":1,\"entries\":[{}]}}", entries.join(",")),
    )
    .unwrap();
}

/// The counter floor reads through a *different* function from discovery, and
/// it used to keep reading a cache it had already rejected an entry from.
///
/// `max_id` → `max_id_in_base` → `library::resolve::read_base_readonly`, which
/// `filter_map`ped the rejected entry away and carried on with the rest.
/// `discover_base` does the documented thing — abandons the file and goes back
/// to the folders — and the rule says why: an entry naming anything but a direct
/// child of its own base means the file is no longer fastf's own bookkeeping, so
/// nothing else in it is evidence either. Of all the readers, this is the one
/// where believing half a forged file matters most: it decides the next ID.
#[test]
fn a_forged_cache_is_abandoned_by_the_counter_floor_too() {
    sandbox(|_install, base| {
        write_project(base, "real", &valid_frontmatter("ID0001", "real"));
        assert_eq!(library::max_id(&config_for(base)), 1);

        // A cache claiming two projects, the higher-numbered of which names a
        // path outside the base. Planted last, so the staleness gate passes and
        // the cache is actually read.
        plant_cache(base, &["real", "../outside"]);

        let cfg = config_for(base);
        assert_eq!(
            library::max_id(&cfg),
            1,
            "the floor must come from the folders once the cache is rejected, \
             not from whichever forged entries happened to survive a filter"
        );
    });
}

/// And the read stays read-only while it does that. `max_id` runs on every
/// create *and* every preview, so a rescan that rewrote the cache would turn
/// previewing a project into a write.
#[test]
fn abandoning_a_forged_cache_still_writes_nothing() {
    sandbox(|_install, base| {
        write_project(base, "real", &valid_frontmatter("ID0001", "real"));
        plant_cache(base, &["real", "../outside"]);
        let cache = base.join(library::CACHE_FILENAME);
        let before = fs::read(&cache).unwrap();

        assert_eq!(library::max_id(&config_for(base)), 1);

        assert_eq!(
            fs::read(&cache).unwrap(),
            before,
            "the preview/plan path may not rewrite the cache, however bad it is"
        );
    });
}

#[test]
fn a_forged_cache_cannot_make_discovery_name_a_path_outside_the_base() {
    sandbox(|install, base| {
        // A sentinel outside the base that must never be reported as a project.
        let outside = install.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"untouched").unwrap();

        write_project(base, "real", &valid_frontmatter("ID0001", "real"));
        let cfg = config_for(base);
        assert_eq!(library::discover(&cfg).len(), 1);

        let escape = outside.display().to_string();
        plant_cache(
            base,
            &["/etc", "../outside", "..", ".hidden", "a/b", &escape],
        );

        // Every hostile entry is dropped. The drop rewrites the cache, and the
        // rescan that follows finds the one project that is really there.
        let found = library::discover(&cfg);
        assert_eq!(
            found.len(),
            1,
            "forged entries were served as projects: {:?}",
            found.iter().map(|p| p.path.clone()).collect::<Vec<_>>()
        );
        assert_eq!(found[0].id, "ID0001");
        // Canonicalized on both sides: discovery canonicalizes its bases, and on
        // Windows that adds the `\\?\` verbatim prefix and resolves 8.3 short
        // names, so a raw `base.join(...)` is a different string for the same
        // directory.
        assert_eq!(
            fs::canonicalize(&found[0].path).unwrap(),
            fs::canonicalize(base.join("real")).unwrap()
        );

        // Nothing outside the base was touched, listed, or reported.
        assert_eq!(
            fs::read_to_string(outside.join("sentinel")).unwrap(),
            "untouched"
        );
    });
}

/// The read side. `fastf open` hands a discovered path to the system file
/// manager, and until now took whatever the cache said.
#[test]
fn opening_a_project_checks_the_path_before_spawning_anything() {
    sandbox(|install, base| {
        let outside = install.join("outside");
        fs::create_dir_all(&outside).unwrap();

        let real = write_project(base, "real", &valid_frontmatter("ID0001", "real"));
        let project = library::Project {
            id: "ID0001".to_string(),
            id_number: None,
            template: "t".to_string(),
            template_name: "T".to_string(),
            name: "real".to_string(),
            path: real.clone(),
            base: base.to_path_buf(),
            created: "2026-01-01T00:00:00Z".to_string(),
            tags: vec![],
            exists: true,
        };
        // The real thing passes, or the guard would be useless.
        library::revalidate_for_read(&project).unwrap();

        // Not a direct child of its own base.
        let elsewhere = library::Project {
            path: outside.clone(),
            ..project.clone()
        };
        assert!(library::revalidate_for_read(&elsewhere).is_err());

        // A directory with no PROJECT_INFO.md is not a project folder.
        let bare = base.join("bare");
        fs::create_dir_all(&bare).unwrap();
        let bare_project = library::Project {
            path: bare,
            ..project.clone()
        };
        let error = library::revalidate_for_read(&bare_project)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("not a project folder"),
            "unexpected: {error}"
        );

        // A missing folder.
        let gone = library::Project {
            path: base.join("gone"),
            ..project.clone()
        };
        assert!(library::revalidate_for_read(&gone).is_err());
    });
}

/// A project directory replaced by a link to somewhere else must not be opened
/// — the folder fastf would spawn a file manager on is not the project.
#[cfg(unix)]
#[test]
fn a_project_directory_replaced_by_a_link_is_not_opened() {
    use std::os::unix::fs::symlink;

    sandbox(|install, base| {
        let outside = install.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            project_info::pinfo_path(&outside),
            valid_frontmatter("ID0001", "outside"),
        )
        .unwrap();

        let linked = base.join("linked");
        symlink(&outside, &linked).unwrap();

        let project = library::Project {
            id: "ID0001".to_string(),
            id_number: None,
            template: "t".to_string(),
            template_name: "T".to_string(),
            name: "linked".to_string(),
            path: linked,
            base: base.to_path_buf(),
            created: "2026-01-01T00:00:00Z".to_string(),
            tags: vec![],
            exists: true,
        };
        let error = library::revalidate_for_read(&project)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("not a real directory"),
            "unexpected error: {error}"
        );
    });
}
