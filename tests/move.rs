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

/// Two bases, one project in the first, the configuration saved — what
/// `operations::move_project` reloads under the lock.
fn one_project_two_bases(install: &Path) -> (library::Project, std::path::PathBuf) {
    write_template(install, "test", &minimal_template_yaml("test"));
    let base_a = install.join("projects");
    let base_b = install.join("projects_b");
    fs::create_dir_all(&base_a).unwrap();
    fs::create_dir_all(&base_b).unwrap();
    let mut cfg = Config::default();
    cfg.base_dir = base_a.display().to_string();
    cfg.bases = vec![base_b.display().to_string()];
    cfg.save().unwrap();
    let tmpl = template::find_by_slug("test").unwrap();
    let mut counters = Counters::load().unwrap();
    let mut vars = HashMap::new();
    vars.insert("name".to_string(), "ender".to_string());
    let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
    project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
    (library::discover(&cfg).remove(0), base_b)
}

/// **A job that stops says how.** A failed move used to keep `Running` for
/// ever, so anything watching it — the app polls until it is not — watched a
/// dead job.
#[test]
fn a_move_that_fails_ends_failed_and_says_why() {
    sandboxed(|install| {
        let (project, base_b) = one_project_two_bases(install);
        let blocker = base_b.join(project.path.file_name().unwrap());
        fs::create_dir_all(&blocker).unwrap();

        let progress = Mutex::new(fastf::core::assets::Progress::new(&[]));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let result = fastf::core::operations::move_project(&project, &base_b, &progress, &cancel);

        assert!(result.is_err());
        let state = progress.lock().unwrap();
        assert_eq!(state.status, fastf::core::assets::JobStatus::Failed);
        let why = state.error.clone().unwrap_or_default();
        assert!(why.contains("already exists"), "{why}");
        assert!(project.path.is_dir(), "a failed move leaves the original");
    });
}

/// A cancel before the publish undoes the move and ends `Cancelled`; one
/// after it is too late and changes nothing — the move finishes and the old
/// copy is removed whole, never left part of the way because somebody
/// pressed Ctrl-C once it no longer meant anything.
#[cfg(debug_assertions)]
#[test]
fn a_cancel_undoes_a_move_before_its_publish_and_changes_nothing_after() {
    use fastf::core::assets::{JobStatus, Progress};
    use std::sync::atomic::{AtomicBool, Ordering};

    common::env::with_sandbox(&SERIAL, |sandbox, guard| {
        let (project, base_b) = one_project_two_bases(&sandbox.install);
        guard.set("FASTF_FAULT", Path::new("move:force-staged"));

        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(true);
        let early = fastf::core::operations::move_project(&project, &base_b, &progress, &cancel);
        assert!(early.is_err());
        assert_eq!(progress.lock().unwrap().status, JobStatus::Cancelled);
        assert!(project.path.is_dir(), "the original is where it was");
        assert!(!base_b.join(project.path.file_name().unwrap()).exists());

        guard.set(
            "FASTF_FAULT",
            Path::new("move:force-staged,remove:each-entry:delay-5"),
        );
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);
        let late = std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..5000 {
                    if progress.lock().unwrap().committed {
                        cancel.store(true, Ordering::Relaxed);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            });
            fastf::core::operations::move_project(&project, &base_b, &progress, &cancel)
        })
        .expect("a cancel after the publish does not stop the move");
        assert!(cancel.load(Ordering::Relaxed), "the cancel was asked for");
        assert_eq!(
            late.source,
            fastf::core::move_engine::SourceOutcome::Removed
        );
        assert_eq!(progress.lock().unwrap().status, JobStatus::Done);
        assert!(!project.path.exists(), "the old copy is gone, whole");
        assert!(late.project.path.is_dir());
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

/// A same-filesystem move preserves a symlink inside the project.
///
/// A rename copies nothing, so it preserves links perfectly; the staged path
/// now carries them as links too, by their target text. The rename's half of
/// that guarantee was pinned only by
/// `windows_semantics.rs`'s `#[cfg(windows)]` junction test and by the opt-in
/// `windows_live.rs`, and `tests/CLAUDE.md` legislates against exactly that:
/// "a suite CI never runs cannot be the only guard on a fix". This is the unix
/// sibling, and it costs three lines.
#[cfg(unix)]
#[test]
fn a_same_filesystem_move_preserves_a_symlink_inside_the_project() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let base_a = install.join("projects");
        let base_b = install.join("projects_b");
        fs::create_dir_all(&base_a).unwrap();
        fs::create_dir_all(&base_b).unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = base_a.display().to_string();
        cfg.bases = vec![base_b.display().to_string()];
        cfg.save().unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut counters = Counters::load().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "linked".to_string());
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let project = library::discover(&cfg).remove(0);
        let link = project.path.join("shortcut");
        std::os::unix::fs::symlink("PROJECT_INFO.md", &link).unwrap();

        let progress = Mutex::new(fastf::core::assets::Progress::new(&[]));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let outcome =
            fastf::core::operations::move_project(&project, &base_b, &progress, &cancel).unwrap();
        assert!(!outcome.staged, "one filesystem is a rename");

        let moved = base_b.join(&project.name).join("shortcut");
        let kind = fs::symlink_metadata(&moved).expect("the link came with the project");
        assert!(
            kind.file_type().is_symlink(),
            "a rename preserves it as a link, not as a copy of its target"
        );
        assert_eq!(
            fs::read_link(&moved).unwrap(),
            std::path::Path::new("PROJECT_INFO.md"),
            "and pointing where it pointed"
        );
    });
}

/// A copy carries links as links, by their target text — a dangling one
/// included, as `node_modules/.bin` leaves them — and never copies what is
/// behind one.
#[cfg(unix)]
#[test]
fn a_copy_carries_links_as_links() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let base = install.join("projects");
        let backup = install.join("backup");
        let library_folder = install.join("asset_library");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::create_dir_all(&library_folder).unwrap();
        fs::write(library_folder.join("stock.mov"), vec![1_u8; 2048]).unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = base.display().to_string();
        cfg.save().unwrap();
        let tmpl = template::find_by_slug("test").unwrap();
        let mut counters = Counters::load().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "linked".to_string());
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        let project = library::discover(&cfg).remove(0);
        fs::create_dir_all(project.path.join("node_modules/.bin")).unwrap();
        std::os::unix::fs::symlink(
            "../vite/bin/vite.js",
            project.path.join("node_modules/.bin/vite"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&library_folder, project.path.join("assets")).unwrap();

        let progress = Mutex::new(fastf::core::assets::Progress::new(&[]));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let outcome =
            fastf::core::operations::copy_project(&project, &backup, &progress, &cancel).unwrap();

        let landed = outcome.path.clone();
        assert_eq!(outcome.links, 2);
        assert_eq!(
            fs::read_link(landed.join("node_modules/.bin/vite")).unwrap(),
            Path::new("../vite/bin/vite.js")
        );
        assert_eq!(
            fs::read_link(landed.join("assets")).unwrap(),
            library_folder
        );
        assert!(
            fs::symlink_metadata(landed.join("assets"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "a link, not the folder behind it"
        );
        let (_, bytes) = outcome.copied;
        assert!(
            bytes < 2048,
            "what is behind the link was not copied: {bytes}"
        );
    });
}
