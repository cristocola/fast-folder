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

// ---------------------------------------------------------------------------
// Copying a project out of the library
// ---------------------------------------------------------------------------

/// A copy is a move that keeps its source: same manifest, same staging, same
/// verification, same atomic publish — and the original untouched. It keeps
/// its ID too, because it is the same project on another drive.
#[test]
fn a_copy_lands_verified_and_leaves_the_original_alone() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let base = install.join("projects");
        let backup = install.join("backup");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&backup).unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = base.display().to_string();
        cfg.save().unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut counters = Counters::load().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "copier".to_string());
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let project = library::discover(&cfg).remove(0);
        fs::write(project.path.join("payload.bin"), vec![7_u8; 4096]).unwrap();

        let progress = Mutex::new(fastf::core::assets::Progress::new(&[]));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let outcome =
            fastf::core::operations::copy_project(&project, &backup, &progress, &cancel).unwrap();

        // **Both sides canonicalized.** `outcome.path` is derived from a
        // canonicalized destination, and on a Windows runner the tempdir the
        // test holds is the 8.3 short name (`RUNNER~1`) of the long one the
        // engine returns — the exact comparison that has broken this suite on
        // that platform before.
        let landed = backup
            .canonicalize()
            .unwrap()
            .join(project.path.file_name().unwrap());
        assert_eq!(outcome.path.canonicalize().unwrap(), landed);
        assert!(project.path.is_dir(), "the original is untouched");
        assert!(landed.join("README.md").is_file(), "the template file came");
        assert_eq!(
            fs::read(landed.join("payload.bin")).unwrap().len(),
            4096,
            "and so did the payload"
        );
        let (files, bytes) = outcome.copied;
        assert!(files >= 2 && bytes >= 4096, "{files} files, {bytes} bytes");

        // **The copy keeps the id.** It is the same project somewhere else.
        let copied = project_info::read_metadata(&landed).unwrap().unwrap();
        assert_eq!(copied.id, project.id);

        // Nothing is left behind: no transaction directory under the target.
        assert!(
            !backup.join(".fastf-transactions").exists()
                || fs::read_dir(backup.join(".fastf-transactions"))
                    .unwrap()
                    .next()
                    .is_none(),
            "the completed copy removes its own transaction"
        );

        // And the library still holds exactly one project — the backup is
        // outside every base, so nothing new is discoverable.
        assert_eq!(library::discover(&cfg).len(), 1);
    });
}

/// The one destination a copy may not have: inside a configured base. Two
/// projects with one id in one library is a library that cannot answer "which
/// one", and it would be made by a keystroke.
#[test]
fn a_copy_into_a_base_is_refused_by_name() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let base = install.join("projects");
        let archive = install.join("archive");
        let inside = base.join("nested");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&archive).unwrap();
        fs::create_dir_all(&inside).unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = base.display().to_string();
        cfg.bases = vec![archive.display().to_string()];
        cfg.save().unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut counters = Counters::load().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "refused".to_string());
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        let project = library::discover(&cfg).remove(0);

        for (target, what) in [(&archive, "another base"), (&inside, "inside a base")] {
            let error = fastf::core::copy_engine::resolve_destination(&cfg, &project, target)
                .expect_err(what)
                .to_string();
            assert!(
                error.contains("configured base") && error.contains(&project.id),
                "the refusal names the rule and the id: {error}"
            );
        }

        // Into the project itself, the obvious infinite one.
        let error = fastf::core::copy_engine::resolve_destination(&cfg, &project, &project.path)
            .expect_err("into itself")
            .to_string();
        assert!(error.contains("inside the project"), "{error}");

        // Nothing was written by any of those refusals. (Discovery leaves an
        // index at a base's root; a copy would have left a project folder.)
        let folder = project.path.file_name().unwrap();
        assert!(!archive.join(folder).exists());
        assert!(!inside.join(folder).exists());
        assert!(!archive.join(".fastf-transactions").exists());
    });
}

/// Two bases holding the same id list as two rows — the copy's whole point —
/// and each can be acted on independently: revalidation is by path *and* id,
/// so neither can be mistaken for the other.
#[test]
fn two_bases_with_one_id_list_as_two_rows() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let base = install.join("projects");
        let backup = install.join("backup");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&backup).unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = base.display().to_string();
        cfg.save().unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut counters = Counters::load().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "twinned".to_string());
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        let project = library::discover(&cfg).remove(0);

        let progress = Mutex::new(fastf::core::assets::Progress::new(&[]));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        fastf::core::operations::copy_project(&project, &backup, &progress, &cancel).unwrap();

        // Now adopt the backup as a base — the thing the copy exists to allow.
        cfg.bases = vec![backup.display().to_string()];
        cfg.save().unwrap();

        let rows = library::discover(&cfg);
        assert_eq!(rows.len(), 2, "both list");
        assert_eq!(rows[0].id, rows[1].id, "with one id between them");
        assert_ne!(rows[0].base, rows[1].base, "told apart by their base");

        // The ambiguity message names the bases rather than telling the reader
        // to be more specific about an id that is already exact.
        let error = library::resolve(&cfg, &project.id).expect_err("ambiguous");
        let text = error.to_string();
        assert!(text.contains("is in 2 bases"), "{text}");
        assert!(text.contains("name the base"), "{text}");

        // Each row mutates on its own: the tag lands on one and not the other.
        let first = rows
            .iter()
            .find(|p| p.base != backup.canonicalize().unwrap());
        let first = first.expect("the original is still under its base");
        fastf::core::operations::add_tags(first, &["kept".to_string()]).unwrap();
        for row in &rows {
            let tags = project_info::read_metadata(&row.path)
                .unwrap()
                .unwrap()
                .tags;
            assert_eq!(
                tags.contains(&"kept".to_string()),
                row.path == first.path,
                "only the row that was tagged carries it: {}",
                row.path.display()
            );
        }
    });
}
