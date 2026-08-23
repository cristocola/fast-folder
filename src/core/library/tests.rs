//! `library`'s own unit tests, kept together: they were written against
//! the module as a whole and reach across what are now submodule boundaries.

use super::*;
use crate::core::assets::Progress;
use crate::core::config::Config;
use crate::core::move_engine::{is_cross_device_error, staged_copy_verify_commit};
use crate::core::project_info;
#[cfg(debug_assertions)]
use crate::core::provisioning;
use crate::core::transactions::{self, MoveManifest};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::thread::sleep;
use std::time::Duration;

/// Write a project folder with a valid `PROJECT_INFO.md` frontmatter block.
fn write_project(base: &Path, folder: &str, id: &str, template: &str, created: &str) {
    let dir = base.join(folder);
    fs::create_dir_all(&dir).unwrap();
    // Backslashes in a double-quoted YAML scalar are escape sequences —
    // a raw Windows path (`C:\Users\...`) makes the whole frontmatter
    // unparseable, so escape them.
    let path_yaml = dir.display().to_string().replace('\\', "\\\\");
    let fm = format!(
        "---\nid: {id}\ntemplate: {template}\ntemplate_name: \"{template} name\"\n\
         created: \"{created}\"\nfolder: {folder}\npath: \"{path_yaml}\"\nvariables: {{}}\ntags: []\n\
         ---\n\n# Project Info\n"
    );
    fs::write(dir.join(project_info::RESERVED_FILENAME), fm).unwrap();
}

fn cfg_for(base: &Path, extra: &[&Path]) -> Config {
    Config {
        base_dir: base.display().to_string(),
        bases: extra.iter().map(|p| p.display().to_string()).collect(),
        ..Default::default()
    }
}

fn v2_transaction_count(base: &Path) -> usize {
    let root = transactions::transaction_root(base);
    fs::read_dir(root)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}

#[test]
fn scan_finds_only_project_info_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    // A folder without PROJECT_INFO.md is not a project.
    fs::create_dir_all(base.join("not_a_project/sub")).unwrap();
    // A loose file is ignored.
    fs::write(base.join("loose.txt"), "hi").unwrap();

    let projects = scan_base(base);
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].id, "ID0001");
    assert_eq!(projects[0].name, "proj_a");
}

#[test]
fn cache_round_trips_base_relative() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

    let projects = scan_base(base);
    write_cache(base, &projects).unwrap();

    // The on-disk cache stores a base-relative `dir`, never an absolute path.
    let raw = fs::read_to_string(cache_path(base)).unwrap();
    assert!(raw.contains("\"dir\": \"proj_a\""), "raw cache: {raw}");
    assert!(
        !raw.contains(&base.display().to_string()),
        "cache must not contain absolute base path"
    );

    // Loading reconstructs the absolute path via base.join(dir).
    let cache = load_cache(base).unwrap();
    assert_eq!(cache.entries.len(), 1);
    let reconstructed = cache.entries[0].clone().into_project(base).unwrap();
    assert_eq!(reconstructed.path, base.join("proj_a"));
}

/// A cache entry is a hint, and a hint may not name a path outside its base.
///
/// `dir` used to be joined onto the base with no validation: `Path::join`
/// *replaces* the base when given an absolute path, so `/etc` produced a
/// "project" at `/etc`. Caches travel with the projects by design, and
/// overwriting one in place does not bump the base's mtime, so a planted cache
/// reads as fresh.
#[test]
fn a_cache_entry_that_leaves_its_base_is_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    let hostile = [
        "/etc", "../../x", "..", ".", "D:/x", r"D:\x", ".hidden", "a/b", r"a\b",
        "",
        // Not here: "   ". It is a single contained component, and containment
        // is the rule. If no such directory exists the `is_dir()` check on the
        // fast path drops it like any other stale entry.
    ];
    for dir in hostile {
        let entry = CacheEntry {
            dir: dir.to_string(),
            id: "ID0001".to_string(),
            template: "gen".to_string(),
            template_name: "General".to_string(),
            name: "forged".to_string(),
            created: "2026-01-01T00:00:00Z".to_string(),
            tags: vec![],
        };
        assert!(
            entry.into_project(base).is_none(),
            "dir {dir:?} should have been dropped"
        );
    }

    // And an ordinary name still works, or the rule would be useless.
    let entry = CacheEntry {
        dir: "proj_a".to_string(),
        id: "ID0001".to_string(),
        template: "gen".to_string(),
        template_name: "General".to_string(),
        name: "proj_a".to_string(),
        created: "2026-01-01T00:00:00Z".to_string(),
        tags: vec![],
    };
    let project = entry.into_project(base).expect("a plain name is valid");
    assert_eq!(project.path, base.join("proj_a"));
    assert_eq!(project.base, base);
}

#[test]
fn staleness_triggers_rescan_on_base_mtime_bump() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

    let cfg = cfg_for(base, &[]);
    let first = discover(&cfg);
    assert_eq!(first.len(), 1);

    // Add a second project after the cache was written; creating a new
    // subdir bumps the base dir's mtime past the cache's.
    sleep(Duration::from_millis(20));
    write_project(base, "proj_b", "ID0002", "gen", "2026-02-01T00:00:00Z");

    let second = discover(&cfg);
    assert_eq!(second.len(), 2, "stale cache should have rescanned");
    let cache = load_cache(base).unwrap();
    assert_eq!(cache.entries.len(), 2, "cache should have been rewritten");
}

#[test]
fn existence_check_drops_missing_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

    // Seed a cache that includes a phantom entry for a folder that never
    // existed. Building projects directly lets us plant the phantom.
    let real = scan_base(base);
    let phantom = Project {
        id: "ID0099".to_string(),
        template: "gen".to_string(),
        template_name: "gen name".to_string(),
        name: "proj_ghost".to_string(),
        path: base.join("proj_ghost"),
        base: base.to_path_buf(),
        created: "2026-03-01T00:00:00Z".to_string(),
        tags: vec![],
        exists: true,
    };
    let mut planted = real.clone();
    planted.push(phantom);
    write_cache(base, &planted).unwrap();

    // Re-touch the cache in place (no dir-entry change) so cache mtime is
    // strictly newer than the base mtime → the fast (non-stale) path runs,
    // exercising the existence-check drop rather than a full rescan.
    sleep(Duration::from_millis(20));
    let raw = fs::read_to_string(cache_path(base)).unwrap();
    fs::write(cache_path(base), raw).unwrap();
    assert!(!cache_is_stale(base), "cache should read as fresh");

    let cfg = cfg_for(base, &[]);
    let projects = discover(&cfg);
    assert_eq!(projects.len(), 1, "phantom entry should be dropped");
    assert_eq!(projects[0].id, "ID0001");
    // The drop is persisted.
    let cache = load_cache(base).unwrap();
    assert_eq!(cache.entries.len(), 1);
}

#[test]
fn multi_base_union_sorted_newest_first() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    write_project(
        tmp1.path(),
        "proj_old",
        "ID0010",
        "gen",
        "2026-01-01T00:00:00Z",
    );
    write_project(
        tmp2.path(),
        "proj_new",
        "ID0020",
        "gen",
        "2026-06-01T00:00:00Z",
    );

    let cfg = cfg_for(tmp1.path(), &[tmp2.path()]);
    let projects = discover(&cfg);
    assert_eq!(projects.len(), 2);
    // Newest first.
    assert_eq!(projects[0].id, "ID0020");
    assert_eq!(projects[1].id, "ID0010");
}

#[test]
fn max_id_across_bases() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    // Inconsistent padding on purpose — value is what matters.
    write_project(
        tmp1.path(),
        "a_ID007",
        "ID007",
        "gen",
        "2026-01-01T00:00:00Z",
    );
    write_project(
        tmp2.path(),
        "b_ID0030",
        "ID0030",
        "gen",
        "2026-02-01T00:00:00Z",
    );

    let cfg = cfg_for(tmp1.path(), &[tmp2.path()]);
    assert_eq!(max_id(&cfg), 30);
}

#[test]
fn max_id_empty_is_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = cfg_for(tmp.path(), &[]);
    assert_eq!(max_id(&cfg), 0);
}

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
    // No match.
    assert!(resolve(&cfg, "nope").is_err());
}

#[test]
fn discover_populates_base() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");

    let cfg = cfg_for(base, &[]);
    let canon = base.canonicalize().unwrap();
    // Fresh scan path.
    let projects = discover(&cfg);
    assert_eq!(projects[0].base, canon);
    // Cached path (second discover reads the cache written by the first).
    let projects = discover(&cfg);
    assert_eq!(projects[0].base, canon);
}

/// Renaming only the capitalisation is legitimate and used to be refused:
/// `exists()` is case-insensitive on Windows, so the target "already
/// existed" — it was the source.
#[test]
fn rename_allows_case_only_change() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let dir = base.join("myproject");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        project_info::pinfo_path(&dir),
        "---\nid: ID0001\ntemplate: t\ntemplate_name: T\n\
         created: 2026-01-01T00:00:00Z\nfolder: myproject\npath: x\n\
         variables: {}\ntags: []\n---\n",
    )
    .unwrap();
    fs::write(dir.join("keep.txt"), "content").unwrap();

    let project = scan_base(base).into_iter().next().unwrap();
    let renamed = rename_project_unlocked(&project, "MyProject").unwrap();

    assert_eq!(renamed.name, "MyProject");
    assert!(renamed.path.join("keep.txt").is_file(), "content survived");
    // The folder really carries the new casing on disk.
    let on_disk = fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.eq_ignore_ascii_case("myproject"))
        .expect("project folder present");
    assert_eq!(on_disk, "MyProject");
    // No staging folder stranded — a dot-prefixed name is invisible to
    // discovery, so a leftover would make the project disappear.
    assert!(
        !fs::read_dir(base)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("fastf-case")),
        "case-rename staging folder left behind"
    );
    assert_eq!(scan_base(base).len(), 1, "still exactly one project");
}

#[test]
fn move_project_round_trip() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    // Extra content so the copy fallback path (if hit) is exercised on a tree.
    fs::create_dir_all(old_base.join("proj_a/assets")).unwrap();
    fs::write(old_base.join("proj_a/assets/raw_{x}.txt"), "keep {braces}").unwrap();

    let cfg = cfg_for(old_base, &[new_base]);
    let projects = discover(&cfg);
    assert_eq!(projects.len(), 1);

    let moved = move_project(&projects[0], new_base).unwrap();

    let new_canon = new_base.canonicalize().unwrap();
    assert_eq!(moved.base, new_canon);
    assert_eq!(moved.path, new_canon.join("proj_a"));
    assert!(moved.path.is_dir(), "moved folder should exist");
    assert!(!old_base.join("proj_a").exists(), "source should be gone");
    // Bytes untouched.
    assert_eq!(
        fs::read_to_string(moved.path.join("assets/raw_{x}.txt")).unwrap(),
        "keep {braces}"
    );
    // Metadata `path` patched — in the readable form, not the `\\?\`
    // verbatim one that `canonicalize` hands back on Windows.
    let meta = read_project_meta(&moved.path).unwrap();
    assert_eq!(meta.path, crate::util::paths::display_path(&moved.path));
    assert!(
        !meta.path.starts_with(r"\\?\"),
        "verbatim prefix leaked into metadata: {}",
        meta.path
    );
    assert_eq!(meta.id, "ID0001");
    // Caches on both sides are fresh.
    let old_cache = load_cache(&old_base.canonicalize().unwrap()).unwrap();
    assert!(old_cache.entries.iter().all(|e| e.dir != "proj_a"));
    let new_cache = load_cache(&new_canon).unwrap();
    assert!(new_cache.entries.iter().any(|e| e.dir == "proj_a"));
    // Discovery now finds it under the new base only.
    let after = discover(&cfg);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].base, new_canon);
}

#[test]
fn staged_move_copies_verifies_commits_and_removes_source() {
    // Exercises the cross-filesystem path directly (a same-fs test would take
    // the instant fs::rename fast path and never stage/verify).
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::create_dir_all(old_base.join("proj_a/assets")).unwrap();
    fs::create_dir_all(old_base.join("proj_a/empty")).unwrap();
    fs::write(old_base.join("proj_a/assets/big.bin"), vec![1u8; 8000]).unwrap();
    fs::write(old_base.join("proj_a/notes_{x}.md"), "keep {braces}").unwrap();
    fs::write(old_base.join("proj_a/real.tmp"), []).unwrap();
    fs::write(old_base.join("proj_a/real.part"), [0_u8, 1, 2, 255]).unwrap();

    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    staged_copy_verify_commit(&project, new_base, &new_path, &progress, &cancel).unwrap();

    // Progress must actually advance. The phase and the per-file counter are
    // the only feedback during a multi-minute network copy, so a counter
    // that silently stops updating looks exactly like a hung move.
    {
        let p = progress.lock().unwrap();
        assert_eq!(
            p.phase,
            crate::core::assets::JobPhase::Done,
            "the phase should have advanced"
        );
        assert!(p.total_files >= 3, "files counted: {}", p.total_files);
        assert_eq!(
            p.done_files, p.total_files,
            "every copied file must be reported done"
        );
        assert!(p.copied_bytes >= 8000, "bytes copied: {}", p.copied_bytes);
    }

    // Copied verbatim, verified, committed, source removed.
    assert_eq!(
        fs::read(new_path.join("assets/big.bin")).unwrap(),
        vec![1u8; 8000]
    );
    assert_eq!(
        fs::read_to_string(new_path.join("notes_{x}.md")).unwrap(),
        "keep {braces}"
    );
    assert_eq!(
        fs::read(new_path.join("real.tmp")).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(
        fs::read(new_path.join("real.part")).unwrap(),
        [0_u8, 1, 2, 255]
    );
    assert!(new_path.join("empty").is_dir());
    assert!(
        !old_base.join("proj_a").exists(),
        "source removed only after verify"
    );
    assert_eq!(v2_transaction_count(new_base), 0);
}

#[test]
fn cancelled_staged_move_leaves_source_intact() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let (old_base, new_base) = (tmp1.path(), tmp2.path());
    write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old_base.join("proj_a/data.bin"), vec![9u8; 4096]).unwrap();

    let cfg = cfg_for(old_base, &[new_base]);
    let project = discover(&cfg).remove(0);
    let new_path = new_base.join("proj_a");
    let progress = Mutex::new(Progress::new(&[]));
    // Pre-cancelled → copy aborts on the first chunk.
    let cancel = AtomicBool::new(true);

    let err = staged_copy_verify_commit(&project, new_base, &new_path, &progress, &cancel)
        .unwrap_err()
        .to_string();
    assert!(err.contains("cancelled"), "err: {err}");
    assert!(
        old_base.join("proj_a").is_dir(),
        "source untouched on cancel"
    );
    assert!(!new_path.exists(), "no target committed");
    assert_eq!(v2_transaction_count(new_base), 0);
}

#[cfg(debug_assertions)]
#[test]
fn cleanup_failure_is_a_reported_success_and_retains_the_marker() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    write_project(old.path(), "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(old.path().join("proj/payload.bin"), [0_u8, 1, 2, 255]).unwrap();
    let project = scan_base(old.path()).remove(0);
    let final_path = new.path().join("proj");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    let outcome = crate::util::faults::with_thread_fault("move:source-cleanup", || {
        staged_copy_verify_commit(&project, new.path(), &final_path, &progress, &cancel)
    })
    .expect("publication remains a successful move");

    assert!(outcome.cleanup_pending);
    assert_eq!(
        fs::read(final_path.join("payload.bin")).unwrap(),
        [0_u8, 1, 2, 255]
    );
    assert!(project.path.is_dir(), "failed cleanup leaves source intact");
    assert_eq!(
        v2_transaction_count(new.path()),
        1,
        "cleanup-pending move must retain its v2 transaction"
    );
}

#[test]
fn conventional_v1_staging_and_marker_are_payload_not_move_authority() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    write_project(old.path(), "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let project = scan_base(old.path()).remove(0);
    let final_path = new.path().join("proj");
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);

    // The v1 staging and marker names, written literally: fastf has no
    // writer for either format any more, and a v2 move must treat both as
    // ordinary payload it never reads, follows, or removes.
    let staging = new.path().join(".proj.fastf-part");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join("sentinel"), b"owned by someone else").unwrap();
    let marker = new.path().join(".fastf-move-proj.json");
    fs::write(&marker, b"foreign marker bytes").unwrap();
    let outcome =
        staged_copy_verify_commit(&project, new.path(), &final_path, &progress, &cancel).unwrap();
    assert!(!outcome.cleanup_pending);
    assert_eq!(
        fs::read(staging.join("sentinel")).unwrap(),
        b"owned by someone else"
    );
    assert_eq!(fs::read(marker).unwrap(), b"foreign marker bytes");
}

#[test]
fn only_the_cross_device_error_licenses_copy_fallback() {
    #[cfg(unix)]
    let cross_device = std::io::Error::from_raw_os_error(libc::EXDEV);
    #[cfg(windows)]
    let cross_device = std::io::Error::from_raw_os_error(17);

    assert!(is_cross_device_error(&cross_device));
    assert!(!is_cross_device_error(&std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied"
    )));
    assert!(!is_cross_device_error(&std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "missing"
    )));
}

#[test]
fn stale_project_identity_cannot_authorize_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(base.join("proj/sentinel"), b"keep").unwrap();
    let stale = scan_base(base).remove(0);

    project_info::write_frontmatter(&project_info::pinfo_path(&stale.path), |metadata| {
        metadata.id = "ID9999".to_string();
    })
    .unwrap();
    let error = delete_project_unlocked(&stale).unwrap_err();
    assert!(error.to_string().contains("identity changed"));
    assert_eq!(fs::read(base.join("proj/sentinel")).unwrap(), b"keep");
}

#[test]
fn forged_cached_path_cannot_escape_a_configured_base() {
    let configured = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write_project(
        configured.path(),
        "real",
        "ID0001",
        "gen",
        "2026-01-01T00:00:00Z",
    );
    write_project(
        outside.path(),
        "sentinel",
        "ID0001",
        "gen",
        "2026-01-01T00:00:00Z",
    );
    fs::write(outside.path().join("sentinel/keep.bin"), b"keep").unwrap();

    let mut forged = scan_base(configured.path()).remove(0);
    forged.path = outside.path().join("sentinel");
    let config = cfg_for(configured.path(), &[]);
    let error = revalidate_project(&config, &forged).unwrap_err();

    assert!(error.to_string().contains("direct child"), "got: {error}");
    assert_eq!(
        fs::read(outside.path().join("sentinel/keep.bin")).unwrap(),
        b"keep"
    );
}

/// Removing a project must drop its cache entry, or `recent` keeps listing
/// something that is gone until the staleness gate happens to fire.
#[test]
fn delete_unregister_and_rename_all_drop_the_old_cache_entry() {
    let cached_dirs = |base: &Path| -> Vec<String> {
        load_cache(base)
            .map(|c| c.entries.into_iter().map(|e| e.dir).collect())
            .unwrap_or_default()
    };

    // delete
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "gone", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_project(base, "stays", "ID0002", "gen", "2026-01-02T00:00:00Z");
    write_cache(base, &scan_base(base)).unwrap();
    let doomed = scan_base(base)
        .into_iter()
        .find(|p| p.name == "gone")
        .unwrap();
    delete_project_unlocked(&doomed).unwrap();
    let dirs = cached_dirs(base);
    assert!(!dirs.contains(&"gone".to_string()), "stale entry: {dirs:?}");
    assert!(dirs.contains(&"stays".to_string()), "collateral: {dirs:?}");

    // unregister
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "dropme", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_cache(base, &scan_base(base)).unwrap();
    let p = scan_base(base).into_iter().next().unwrap();
    unregister_project_unlocked(&p).unwrap();
    assert!(
        !cached_dirs(base).contains(&"dropme".to_string()),
        "unregister must drop the cache entry"
    );
    assert!(base.join("dropme").is_dir(), "the folder itself stays");

    // rename
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "before", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_cache(base, &scan_base(base)).unwrap();
    let p = scan_base(base).into_iter().next().unwrap();
    rename_project_unlocked(&p, "after").unwrap();
    let dirs = cached_dirs(base);
    assert!(!dirs.contains(&"before".to_string()), "old entry: {dirs:?}");
    assert!(dirs.contains(&"after".to_string()), "new entry: {dirs:?}");
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

/// `max_id` must be read-only — it runs from `plan()`, and a preview that
/// writes a cache breaks the "dry run touches nothing" guarantee. It must
/// also see projects a stale cache does not mention.
#[test]
fn max_id_is_read_only_and_sees_past_a_stale_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "a", "ID0007", "gen", "2026-01-01T00:00:00Z");
    let cfg = cfg_for(base, &[]);

    // No cache yet: max_id must scan, and must not create one.
    assert_eq!(max_id(&cfg), 7);
    assert!(
        !cache_path(base).exists(),
        "max_id must never write a cache — plan()/preview calls it"
    );

    // With a cache that predates a newly added project, the staleness gate
    // must send it back to the folders rather than under-reporting.
    write_cache(base, &scan_base(base)).unwrap();
    let file = fs::File::options()
        .write(true)
        .open(cache_path(base))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
        .unwrap();
    drop(file);
    write_project(base, "b", "ID0042", "gen", "2026-01-03T00:00:00Z");
    assert_eq!(
        max_id(&cfg),
        42,
        "a stale cache must not hide a project from the counter floor"
    );
}

/// An upsert must leave every *other* entry alone.
///
/// The retain predicate drops the entry being replaced; inverted, it would
/// drop everything else instead and quietly reduce the cache to a single
/// project. Discovery would then self-heal on the next staleness check, so
/// the damage is invisible until someone wonders why `recent` went blank.
#[test]
fn cache_upsert_replaces_one_entry_and_preserves_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    for (folder, id) in [("a", "ID0001"), ("b", "ID0002"), ("c", "ID0003")] {
        write_project(base, folder, id, "gen", "2026-01-01T00:00:00Z");
    }
    // Seed a full cache.
    let all = scan_base(base);
    assert_eq!(all.len(), 3);
    write_cache(base, &all).unwrap();

    // Re-upsert one of them with changed metadata.
    let mut updated = all.iter().find(|p| p.name == "b").unwrap().clone();
    updated.tags = vec!["urgent".to_string()];
    cache_upsert(base, &updated);

    let cache = load_cache(base).expect("cache still readable");
    assert_eq!(
        cache.entries.len(),
        3,
        "upsert must not drop the other entries, got {:?}",
        cache.entries.iter().map(|e| &e.dir).collect::<Vec<_>>()
    );
    let names: std::collections::HashSet<&str> =
        cache.entries.iter().map(|e| e.dir.as_str()).collect();
    assert!(names.contains("a") && names.contains("b") && names.contains("c"));

    // Exactly one entry for the upserted project, carrying the new data.
    let b: Vec<_> = cache.entries.iter().filter(|e| e.dir == "b").collect();
    assert_eq!(b.len(), 1, "no duplicate entry for the upserted project");
    assert_eq!(b[0].tags, vec!["urgent".to_string()]);
}

/// `refresh_cache` must actually re-read the metadata and write it back —
/// silently doing nothing would leave `recent`/`search` showing stale tags
/// after every tag mutation.
#[test]
fn refresh_cache_picks_up_edited_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_cache(base, &scan_base(base)).unwrap();
    assert!(
        load_cache(base).unwrap().entries[0].tags.is_empty(),
        "starts untagged"
    );

    let dir = base.join("proj");
    project_info::write_frontmatter(&project_info::pinfo_path(&dir), |meta| {
        meta.tags = vec!["shipped".to_string()];
    })
    .unwrap();

    refresh_cache(&dir);

    let cache = load_cache(base).expect("cache readable");
    assert_eq!(
        cache.entries[0].tags,
        vec!["shipped".to_string()],
        "refresh_cache must write the edited metadata back"
    );
}

/// The staleness gate: a cache older than its base must be rescanned, and a
/// cache newer than its base must be trusted. Getting the comparison wrong
/// either way costs correctness or a rescan on every command.
#[test]
fn cache_staleness_gate_compares_the_right_way_round() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");

    write_cache(base, &scan_base(base)).unwrap();
    let cache_file = cache_path(base);

    // Set the cache's mtime explicitly rather than relying on write order:
    // writing the cache *into* the base bumps the base's own mtime to the
    // same instant, which makes "is it newer?" a coin flip.
    let set_cache_mtime = |offset_secs: i64| {
        let when = if offset_secs >= 0 {
            std::time::SystemTime::now() + std::time::Duration::from_secs(offset_secs as u64)
        } else {
            std::time::SystemTime::now()
                - std::time::Duration::from_secs(offset_secs.unsigned_abs())
        };
        let file = fs::File::options().write(true).open(&cache_file).unwrap();
        file.set_modified(when).unwrap();
    };

    set_cache_mtime(3600); // cache clearly newer than the base
    assert!(
        !cache_is_stale(base),
        "a cache newer than its base must be trusted"
    );

    set_cache_mtime(-3600); // cache clearly older than the base
    assert!(
        cache_is_stale(base),
        "a cache older than its base must be rescanned"
    );

    // And a missing cache is always stale.
    fs::remove_file(&cache_file).unwrap();
    assert!(cache_is_stale(base));
}

/// Metadata with an empty `created` falls back to the folder's own mtime, so
/// projects still sort sensibly instead of collapsing to one timestamp.
#[test]
fn empty_created_falls_back_to_the_folder_timestamp() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let dir = base.join("proj");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        project_info::pinfo_path(&dir),
        "---\nid: ID0001\ntemplate: t\ntemplate_name: T\ncreated: \"\"\n\
         folder: proj\npath: x\nvariables: {}\ntags: []\n---\n",
    )
    .unwrap();

    let found = scan_base(base);
    assert_eq!(found.len(), 1);
    let created = &found[0].created;
    assert!(!created.is_empty(), "must fall back, not stay blank");
    assert!(
        created.starts_with("20") && created.ends_with('Z'),
        "expected an ISO-8601 UTC timestamp, got {created:?}"
    );
}

/// The staged (copying) move must refuse a project containing links.
///
/// Reached through the private pre-flight because the public entry point
/// only consults it after `fs::rename` fails, and a test cannot conjure a
/// second filesystem. The same-filesystem path is covered separately in
/// `tests/windows_semantics.rs`, where the junction is expected to survive.
#[test]
fn staged_move_pre_flight_refuses_links() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    // Join components separately: `join("proj_a/linked")` yields a
    // mixed-separator path on Windows (`...\proj_a/linked`), and `cmd` then
    // reads `/linked` as a switch — which is precisely how this test came to
    // "skip" silently while reporting success.
    let link = base.join("proj_a").join("linked");
    let target = base.join("shared");
    fs::create_dir_all(&target).unwrap();

    // A silent skip here would be worse than no test: it reports "ok" while
    // asserting nothing, which is exactly how the mutation run found that
    // the transaction scanner's link refusal could be replaced with
    // `Ok(())` and stay
    // green. Junctions need no elevation on Windows and symlinks work
    // normally on Unix, so failing to create one is a real problem — say so
    // loudly rather than passing.
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .expect("running mklink");
        assert!(
            out.status.success(),
            "could not create a junction (needs no elevation on Windows):\n\
             stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).expect("creating a symlink");

    let project = scan_base(base).into_iter().next().unwrap();
    let err = MoveManifest::scan(&project.path)
        .expect_err("a copying move cannot reproduce a link and must refuse")
        .to_string();
    assert!(
        err.contains("linked"),
        "the error must name the offending link, got: {err}"
    );
    assert!(project.path.is_dir(), "manifest scanning must be read-only");

    // A project with no links is waved through.
    write_project(base, "proj_b", "ID0002", "gen", "2026-01-02T00:00:00Z");
    let plain = scan_base(base)
        .into_iter()
        .find(|p| p.name == "proj_b")
        .unwrap();
    assert!(MoveManifest::scan(&plain.path).is_ok());
}

/// The stranded-rename message must name the path the folder is actually at.
///
/// This branch is unreachable by a real filesystem failure in a test — it
/// needs the commit *and* the rollback to fail — so what is pinned here is
/// the thing that matters when it does happen: a user staring at an error
/// can find their project again. A dot-prefixed name is invisible to
/// discovery, so an error that omits it leaves nothing to go on.
#[test]
fn a_stranded_case_rename_names_the_folder_it_left_behind() {
    let staging = Path::new("/library/base/.MyProject.fastf-case");
    let message = stranded_rename_message(
        "renaming 'myproject' to 'MyProject'",
        staging,
        "Permission denied (os error 13)",
    );
    assert!(
        message.contains("renaming 'myproject' to 'MyProject'"),
        "{message}"
    );
    assert!(message.contains(".MyProject.fastf-case"), "{message}");
    assert!(message.contains("Permission denied"), "{message}");
    assert!(
        message.contains("by hand"),
        "the user needs a next step, not just a diagnosis: {message}"
    );
}

/// The move invariant, at every failpoint: the source is intact **or** the
/// destination is complete — never neither, and never a silent half-state.
///
/// The failure is injected rather than raced, so each boundary is hit
/// deterministically instead of "wherever the kill happened to land". These
/// go through the private staged path directly: a same-filesystem test would
/// take the instant `fs::rename` fast path and never stage or verify.
///
/// Debug-only: failpoints are compiled out of release builds.
#[cfg(debug_assertions)]
#[test]
fn interrupted_staged_move_never_loses_data_at_any_failpoint() {
    const MOVE_POINTS: &[&str] = &[
        "move:before-marker-write",
        "move:after-staging",
        "move:after-verify",
        "move:before-commit-rename",
        "move:after-commit-before-source-removal",
    ];

    for point in MOVE_POINTS {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let (old_base, new_base) = (tmp1.path(), tmp2.path());
        write_project(old_base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
        fs::write(old_base.join("proj_a/payload.bin"), vec![7u8; 4096]).unwrap();

        let cfg = cfg_for(old_base, &[new_base]);
        let project = discover(&cfg).remove(0);
        let new_path = new_base.join("proj_a");
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        // Armed per-thread, so a move test running in parallel cannot see
        // this fault — an env var would fire inside every one of them.
        let result = crate::util::faults::with_thread_fault(point, || {
            staged_copy_verify_commit(&project, new_base, &new_path, &progress, &cancel)
        });

        if *point == "move:after-commit-before-source-removal" {
            assert!(
                result.as_ref().is_ok_and(|outcome| outcome.cleanup_pending),
                "[{point}] publication must be reported as cleanup pending"
            );
        } else {
            assert!(result.is_err(), "[{point}] should have failed");
        }

        // The invariant. `after-commit-before-source-removal` is the one
        // point where the commit already landed, so the destination holds
        // the data and the (still-present) source is redundant.
        let source_ok = old_base.join("proj_a/payload.bin").is_file();
        let dest_ok = new_path.join("payload.bin").is_file();
        assert!(
            source_ok || dest_ok,
            "[{point}] data exists in neither location — this is data loss"
        );

        if *point == "move:after-commit-before-source-removal" {
            assert!(dest_ok, "[{point}] commit landed, destination must hold it");
        } else {
            assert!(
                source_ok,
                "[{point}] nothing was committed, so the source must be intact"
            );
            assert!(
                !new_path.exists(),
                "[{point}] an uncommitted move must leave no destination"
            );
        }

        // Whatever happened, reconcile must reach a consistent end state
        // with the payload still present exactly once.
        let report = provisioning::reconcile_unlocked(&cfg);
        let after_source = old_base.join("proj_a/payload.bin").is_file();
        let after_dest = new_path.join("payload.bin").is_file();
        assert!(
            after_source || after_dest,
            "[{point}] reconcile lost the data"
        );
        assert_eq!(
            v2_transaction_count(new_base),
            0,
            "[{point}] reconcile left a transaction behind: {report:?}"
        );
    }
}

#[test]
fn move_project_rejects_same_base_and_collision() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    write_project(
        tmp1.path(),
        "proj_a",
        "ID0001",
        "gen",
        "2026-01-01T00:00:00Z",
    );
    let cfg = cfg_for(tmp1.path(), &[tmp2.path()]);
    let project = discover(&cfg).remove(0);

    // Same base → bail.
    let err = move_project(&project, tmp1.path()).unwrap_err().to_string();
    assert!(err.contains("already in base"), "err: {err}");

    // Target name collision → bail, source untouched.
    fs::create_dir_all(tmp2.path().join("proj_a")).unwrap();
    let err = move_project(&project, tmp2.path()).unwrap_err().to_string();
    assert!(err.contains("already exists"), "err: {err}");
    assert!(project.path.is_dir(), "source must be untouched on bail");
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

#[test]
fn rename_sanitizes_and_rejects_bad_names() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let cfg = cfg_for(base, &[]);
    let project = discover(&cfg).remove(0);

    // Illegal filesystem chars are sanitized, not fatal.
    let renamed = rename_project_unlocked(&project, "New: Name?").unwrap();
    assert_eq!(renamed.name, "New_ Name_");
    assert!(renamed.path.is_dir());
    assert!(!project.path.exists());

    // Dot-prefixed names would be invisible to discovery → rejected.
    let err = rename_project_unlocked(&renamed, ".hidden")
        .unwrap_err()
        .to_string();
    assert!(err.contains("may not start with '.'"), "err: {err}");
    // Same-name rename is a no-op error, not a silent success.
    let err = rename_project_unlocked(&renamed, "New_ Name_")
        .unwrap_err()
        .to_string();
    assert!(err.contains("already the folder's name"), "err: {err}");
    assert!(renamed.path.is_dir(), "folder intact after failed renames");
}

#[test]
fn unregister_and_delete_guard_rails() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    fs::write(base.join("proj_a").join("keep.txt"), "data").unwrap();
    let cfg = cfg_for(base, &[]);
    let project = discover(&cfg).remove(0);

    // Unregister removes only the metadata file.
    unregister_project_unlocked(&project).unwrap();
    assert!(project.path.join("keep.txt").is_file());
    assert!(!project_info::pinfo_path(&project.path).exists());
    // Double-unregister is a clean error.
    assert!(unregister_project_unlocked(&project).is_err());

    // Delete refuses a folder without PROJECT_INFO.md (the guard rail).
    let err = delete_project_unlocked(&project).unwrap_err().to_string();
    assert!(err.contains("no PROJECT_INFO.md"), "err: {err}");
    assert!(project.path.is_dir());

    // Re-register (rewrite metadata) → delete removes the whole folder.
    write_project(base, "proj_a", "ID0001", "gen", "2026-01-01T00:00:00Z");
    delete_project_unlocked(&project).unwrap();
    assert!(!project.path.exists());
}
