//! Moving projects between bases.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, library, project, project_info, template};

mod common;

use common::env::with_fresh_install;
use common::fixtures::{minimal_template_yaml, write_template};

/// This binary's lock over the process environment — see `common::env`.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

// ---------------------------------------------------------------------------
// Moving projects between bases
// ---------------------------------------------------------------------------

/// **A move says which kind it was.** Same-filesystem is an atomic rename that
/// finishes before a frame can be drawn, however large the folder is; a message
/// naming only the destination reads the same whether two hundred gigabytes
/// were copied or nothing was, which is exactly the doubt an instant finish
/// creates. `MoveOutcome::staged` and `copied` are what both surfaces report
/// from.
#[test]
fn a_move_reports_whether_it_renamed_or_copied() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let base_a = install.join("projects");
        let base_b = install.join("projects_b");
        fs::create_dir_all(&base_a).unwrap();
        fs::create_dir_all(&base_b).unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = base_a.display().to_string();
        cfg.bases = vec![base_b.display().to_string()];
        // `operations::move_project` reloads the configuration under the lock
        // and revalidates both ends against it, so an in-memory `Config` is not
        // enough here.
        cfg.save().unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut counters = Counters::load().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "reporter".to_string());
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let project = library::discover(&cfg).remove(0);

        // Both bases are on one filesystem here, so this is the rename.
        let progress = Mutex::new(fastf::core::assets::Progress::new(&[]));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let outcome =
            fastf::core::operations::move_project(&project, &base_b, &progress, &cancel).unwrap();
        assert!(!outcome.staged, "one filesystem is a rename");
        assert!(outcome.copied.is_none(), "a rename copies nothing");
        // The job is over, and says so. `JobStatus` was assigned `Running` at
        // construction and never changed anywhere in the crate, so the
        // runtime's "is it done yet" was always false: a finished move kept
        // emitting progress and a later cancel set the flag on a dead handle.
        let state = progress.lock().unwrap();
        assert_eq!(state.status, fastf::core::assets::JobStatus::Done);
        drop(state);

        // The staged path, forced, reports what it copied. Debug only:
        // `move_project_staged_for_test` is deliberately absent from a release
        // build, so the assertion has to be too.
        #[cfg(debug_assertions)]
        {
            let moved = library::discover(&cfg).remove(0);
            let staged = library::move_project_staged_for_test(&moved, &base_a).unwrap();
            assert!(staged.staged, "the staged path staged");
            let (files, bytes) = staged.copied.expect("a staged move counts what it copied");
            assert!(files > 0, "the manifest had files in it");
            assert!(bytes > 0, "and bytes");
        }
    });
}

#[test]
fn move_project_between_bases_full_round_trip() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let base_a = install.join("projects");
        let base_b = install.join("projects_b");
        fs::create_dir_all(&base_a).unwrap();
        fs::create_dir_all(&base_b).unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = base_a.display().to_string();
        cfg.bases = vec![base_b.display().to_string()];

        let tmpl = template::find_by_slug("test").unwrap();
        let mut counters = Counters::load().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "mover".to_string());
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Discovery carries the base the project was found under.
        let projects = library::discover(&cfg);
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.base, base_a.canonicalize().unwrap());

        let moved = library::move_project(project, &base_b).unwrap();
        let base_b_canon = base_b.canonicalize().unwrap();
        assert_eq!(moved.base, base_b_canon);
        assert_eq!(moved.id, project.id);
        assert!(
            moved.path.join("README.md").is_file(),
            "bundled file must travel with the project"
        );
        assert!(!project.path.exists(), "source folder should be gone");

        // Metadata `path` is patched to the new location; identity unchanged.
        // Stored in readable form — `canonicalize` yields a `\\?\` path on
        // Windows and that prefix must not end up baked into the metadata.
        let meta = project_info::read_metadata(&moved.path).unwrap().unwrap();
        assert_eq!(meta.path, fastf::util::paths::display_path(&moved.path));
        assert!(
            !meta.path.starts_with(r"\\?\"),
            "verbatim prefix leaked into metadata: {}",
            meta.path
        );
        assert_eq!(meta.id, moved.id);

        // Discovery now finds it under the new base only, and resolve works.
        let after = library::discover(&cfg);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].base, base_b_canon);
        let resolved = library::resolve(&cfg, &moved.id).unwrap();
        assert_eq!(resolved.path, moved.path);
    });
}
