//! Discovery, the index beside each base, and the counter's floor.

use super::*;

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
            id_number: None,
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
        id_number: None,
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
        id_number: None,
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

/// A script makes several projects inside one second, so they share a
/// `created` stamp; newest first still means the higher ID first, not the
/// name that sorts first (`Project_38` above `Project_60`).
#[test]
fn newest_first_breaks_a_shared_second_by_id() {
    let tmp = tempfile::tempdir().unwrap();
    for (folder, id) in [
        ("b_ID0009", "ID0009"),
        ("a_ID0010", "ID0010"),
        ("c_ID0008", "ID0008"),
    ] {
        write_project(tmp.path(), folder, id, "gen", "2026-06-01T10:00:00Z");
    }
    let cfg = cfg_for(tmp.path(), &[]);
    let ids: Vec<String> = discover(&cfg).into_iter().map(|p| p.id).collect();
    assert_eq!(ids, ["ID0010", "ID0009", "ID0008"]);
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

/// **Defect 21.** A scan that missed a project wrote its index after the
/// project arrived, so the index was newer than its base and read as fresh:
/// the project stayed hidden until a rescan. The index now remembers the
/// names its scan saw, and a base holding another name is rescanned —
/// whatever the times say.
#[test]
fn a_project_the_scan_missed_is_found_whatever_the_times_say() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "first", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let scanned = discovery::scan_listing(base);
    // The project lands while that scan's index is being written.
    write_project(base, "second", "ID0002", "gen", "2026-01-02T00:00:00Z");
    write_index(base, &scanned.projects, scanned.names).unwrap();
    let future = std::time::SystemTime::now() + Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(cache_path(base))
        .unwrap()
        .set_modified(future)
        .unwrap();
    assert!(!cache_is_stale(base), "the time gate alone would trust it");

    let found = discover(&cfg_for(base, &[]));
    let ids: Vec<&str> = found.iter().map(|project| project.id.as_str()).collect();
    assert_eq!(ids, ["ID0002", "ID0001"]);
}

/// On an rclone base a folder's time reads 2000-01-01 once the mount's
/// directory cache expires, whatever was added: the time gate never fired
/// there, and a project copied in from elsewhere stayed invisible.
#[cfg(unix)]
#[test]
fn a_base_whose_folder_time_never_moves_still_shows_a_new_project() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "first", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let cfg = cfg_for(base, &[]);
    assert_eq!(discover(&cfg).len(), 1);

    write_project(base, "copied_in", "ID0007", "gen", "2026-01-02T00:00:00Z");
    let y2k = std::time::UNIX_EPOCH + Duration::from_secs(946_684_800);
    fs::File::open(base).unwrap().set_modified(y2k).unwrap();
    assert!(!cache_is_stale(base));

    let ids: Vec<String> = discover(&cfg)
        .into_iter()
        .map(|project| project.id)
        .collect();
    assert_eq!(ids, ["ID0007", "ID0001"]);
}

/// fastf's own changes keep the index current, so the next discovery reads
/// it instead of the base; a change made beside them still does not.
#[test]
fn fastfs_own_writes_keep_the_index_current_and_nobody_elses() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "first", "ID0001", "gen", "2026-01-01T00:00:00Z");
    let cfg = cfg_for(base, &[]);
    discover(&cfg);
    let current = |base: &Path| {
        let cache = load_cache(base).unwrap();
        matches!(
            discovery::freshness(base, cache.seen.as_ref()),
            discovery::Freshness::Current(_)
        )
    };
    assert!(current(base));

    // A create: the folder, then its entry.
    write_project(base, "second", "ID0002", "gen", "2026-01-02T00:00:00Z");
    assert!(!current(base), "a folder the index has not heard of");
    let second = scan_base(base)
        .into_iter()
        .find(|project| project.id == "ID0002")
        .unwrap();
    cache_upsert(base, &second);
    assert!(current(base), "an upsert names its folder");

    // Unregistered: the entry goes, the folder and its name stay.
    fs::remove_file(base.join("first").join(project_info::RESERVED_FILENAME)).unwrap();
    cache_remove(base, "first");
    assert!(current(base), "the folder is still there");
    // Moved away: the name goes with the folder.
    fs::remove_dir_all(base.join("second")).unwrap();
    cache_remove(base, "second");
    assert!(current(base));

    // Someone else's folder, made beside fastf's own write.
    fs::create_dir(base.join("theirs")).unwrap();
    write_project(base, "third", "ID0003", "gen", "2026-01-03T00:00:00Z");
    let third = scan_base(base)
        .into_iter()
        .find(|project| project.id == "ID0003")
        .unwrap();
    cache_upsert(base, &third);
    assert!(!current(base), "a name only a listing knows about");
}

/// An index an older fastf wrote has no names: it is rescanned once, and the
/// rescan records them.
#[test]
fn an_index_without_names_is_rescanned_once() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "first", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_index(base, &scan_base(base), None).unwrap();
    assert!(matches!(
        discovery::freshness(base, None),
        discovery::Freshness::Stale
    ));
    assert_eq!(discover(&cfg_for(base, &[])).len(), 1);
    assert_eq!(
        load_cache(base).unwrap().seen,
        Some(vec!["first".to_string()])
    );
    let written = fs::read_to_string(cache_path(base)).unwrap();
    let older: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(older["version"], 1, "no version bump: older fastf reads it");
}

/// A cloud mount can put an old copy's `PROJECT_INFO.md` back after a move
/// removed it — an edit still uploading when the move ran. While the move's
/// pointer names the folder, the same project there is that old copy, not a
/// second listing of the project; another project there is someone's.
#[test]
fn an_old_copy_a_cloud_mount_put_back_is_not_listed_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    write_project(base, "proj", "ID0001", "gen", "2026-01-01T00:00:00Z");
    write_project(base, "other", "ID0002", "gen", "2026-01-02T00:00:00Z");
    let pointer = |folder: &str, id: &str| {
        serde_json::json!({
            "version": 1,
            "operation": "18d8e2f16082c791-e6a94-0",
            "project_id": id,
            "folder": folder,
            "target_base": "/mnt/elsewhere",
            "target_folder": folder,
        })
        .to_string()
    };
    let pointer_path = base.join(".fastf-moved-18d8e2f16082c791-e6a94-0.json");
    fs::write(&pointer_path, pointer("proj", "ID0001")).unwrap();
    let ids: Vec<String> = scan_base(base).into_iter().map(|p| p.id).collect();
    assert_eq!(
        ids,
        ["ID0002"],
        "the old copy is hidden, the neighbour is not"
    );

    // A pointer naming a folder that holds another project hides nothing.
    fs::write(&pointer_path, pointer("other", "ID0001")).unwrap();
    let mut ids: Vec<String> = scan_base(base).into_iter().map(|p| p.id).collect();
    ids.sort();
    assert_eq!(ids, ["ID0001", "ID0002"]);
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

/// A drive-root base has no last component, and its label is the whole path
/// as it reads — the list showed `\\?\S:\` for an rclone drive.
#[test]
fn a_root_base_is_labelled_as_it_reads() {
    #[cfg(windows)]
    assert_eq!(base_label(Path::new(r"\\?\S:\")), r"S:\");
    #[cfg(unix)]
    assert_eq!(base_label(Path::new("/")), "/");
    assert_eq!(
        base_label(Path::new("/mnt/projects/01_PROJECTS")),
        "01_PROJECTS"
    );
}
