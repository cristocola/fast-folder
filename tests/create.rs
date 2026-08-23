//! Creating projects: plan, claim, provision, roll back.
//!
//! Split out of the single 2700-line `integration.rs`, whose 67 tests all
//! queued behind one mutex in one binary.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastf::core::{
    config::Config, counter::Counters, library, naming, project, project_info, template,
};

mod common;

use common::env::with_fresh_install;
use common::fixtures::{minimal_template_yaml, write_template};

/// This binary's own lock. `FASTF_INSTALL_DIR` and `HOME` are process-wide, so
/// every test in a binary shares one — and separate binaries are separate
/// processes, which is what lets these suites run in parallel with each other.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

#[test]
fn create_project_basic_round_trip() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "hello world".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        assert_eq!(plan.id_str, "T001");
        assert_eq!(plan.folder_name, "T001_Hello_World");

        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Verify folder tree exists.
        assert!(plan.root_path.join("src").join("core").is_dir());
        // Verify file interpolation happened.
        let readme = fs::read_to_string(plan.root_path.join("README.md")).unwrap();
        assert!(readme.contains("# Hello_World"), "readme was: {readme}");
        assert!(readme.contains("id: T001"));

        // Counter persisted — into the base, not the data directory.
        assert_eq!(Counters::load_base(std::path::Path::new(&cfg.base_dir)), 1);
    });
}

#[test]
fn counter_increments_across_runs() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        for expected in 1..=3u64 {
            let tmpl = template::find_by_slug("test").unwrap();
            let mut vars = HashMap::new();
            vars.insert("name".to_string(), format!("run {expected}"));
            let counters = Counters::load().unwrap();
            let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
            assert_eq!(plan.counter_value, expected);
            let mut counters = counters;
            project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        }

        assert_eq!(Counters::load_base(std::path::Path::new(&cfg.base_dir)), 3);
    });
}

#[test]
fn existing_project_fails_cleanly() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        // The default is now to append `_2`, since a naming pattern need not
        // contain `{id}`. `error` restores the old refuse-a-duplicate guard,
        // which is what this test is about.
        cfg.on_name_collision = fastf::core::config::NameCollision::Error;
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "collide".to_string());

        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Second attempt at same path should fail.
        let counters2 = Counters::load().unwrap();
        let plan2 = project::plan(&tmpl, &vars, &cfg, &counters2).unwrap();
        // Force the same folder name as the first run — the claim derives the
        // path from `folder_name`, so that is the field to pin.
        let mut plan2 = plan2;
        plan2.folder_name = plan.folder_name.clone();
        plan2.root_path = plan.root_path.clone();
        let mut counters2 = counters2;
        let err = project::create(&plan2, &tmpl, &mut counters2, &cfg, false)
            .expect_err("second create should fail");
        assert!(err.to_string().contains("already exists"), "got: {err:#}");
    });
}

/// With the default `suffix` policy the same name twice is not an error — the
/// second lands on `_2`. Two real folders, two real projects, never a merge.
#[test]
fn a_repeated_name_gets_a_numbered_suffix() {
    sandboxed(|install| {
        // A pattern with no `{id}`, like the bundled templates now ship.
        write_template(
            install,
            "noid",
            r#"name: No Id
slug: noid
description: fixture
naming_pattern: "{name}"
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: title_underscore
structure:
  - name: src
"#,
        );
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("noid").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "collide".to_string());

        let mut counters = Counters::load().unwrap();
        let first = project::create(
            &project::plan(&tmpl, &vars, &cfg, &counters).unwrap(),
            &tmpl,
            &mut counters,
            &cfg,
            false,
        )
        .unwrap();
        let mut counters = Counters::load().unwrap();
        let second = project::create(
            &project::plan(&tmpl, &vars, &cfg, &counters).unwrap(),
            &tmpl,
            &mut counters,
            &cfg,
            false,
        )
        .unwrap();

        assert_eq!(first.folder_name, "Collide");
        assert_eq!(second.folder_name, "Collide_2");
        assert_ne!(first.id_str, second.id_str, "each still gets its own ID");
        assert!(second.root_path.is_dir());
        assert_eq!(library::discover(&cfg).len(), 2);
    });
}

#[test]
fn create_is_discoverable_without_jsonl() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "indexed".to_string());

        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Filesystem-as-truth: no projects.jsonl; discovery reads the folder.
        assert!(
            !install.join("projects.jsonl").exists(),
            "create must not write projects.jsonl"
        );
        let projects = library::discover(&cfg);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "T001");
        assert_eq!(projects[0].template, "test");
        assert!(projects[0].name.contains("Indexed"));

        // The base cache was written co-located with the project.
        let cache = std::path::Path::new(&cfg.base_dir).join(".fastf-index.json");
        assert!(cache.exists(), "base cache should exist after create");
    });
}

#[test]
fn counter_self_heals_from_existing_projects() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "first".to_string());

        // First project → T001; the counter advances to 1.
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        assert_eq!(plan.id_str, "T001");

        // The mark is written into the base, not the data directory — that is
        // what lets a dual-boot machine share one number without a symlink.
        let base = std::path::Path::new(&cfg.base_dir);
        assert_eq!(
            Counters::load_base(base),
            1,
            "the base should carry the high-water mark"
        );

        // Simulate the counter file being lost (drive reformatted, folder
        // copied without hidden files, base not yet written by this OS).
        fs::remove_file(Counters::base_path(base)).unwrap();
        assert_eq!(Counters::load_base(base), 0);

        // The next create must NOT reuse T001 — the floor falls back to the
        // highest ID actually present in the projects on disk.
        vars.insert("name".to_string(), "second".to_string());
        let mut counters = Counters::load().unwrap();
        let plan2 = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        assert_eq!(plan2.id_str, "T002", "counter should self-heal past T001");
        project::create(&plan2, &tmpl, &mut counters, &cfg, false).unwrap();

        // And the base's mark is rebuilt.
        assert!(Counters::load_base(base) >= 2);
    });
}

/// The point of the move: the counter lives with the projects, so a second
/// machine reading the same base sees the same number without any shared
/// config. Simulated by pointing a *fresh* data directory at the same base —
/// which is exactly what the other half of a dual-boot install looks like.
#[test]
fn the_counter_travels_with_the_base_not_the_data_dir() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();
        let base = std::path::Path::new(&cfg.base_dir).to_path_buf();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "first".to_string());
        let mut counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        assert_eq!(plan.id_str, "T001");

        // The mark goes to both places, for different reasons: the base so
        // another OS mounting the drive sees it, the data dir so it survives
        // that base being unplugged.
        assert_eq!(Counters::load_base(&base), 1);
        assert_eq!(Counters::load().unwrap().get(), 1);

        // A different install dir — the other OS — still sees the mark, because
        // it reads the base.
        assert_eq!(
            Counters::floor(&cfg),
            1,
            "the floor must come from the base, not from local state"
        );
    });
}

/// Unplugging a base must not restart numbering.
///
/// Storing the mark only in the base looks tidy and is wrong: work in an archive
/// base up to ID0005, unplug it, create in another base, and the next ID is
/// ID0001 — then plugging the archive back in gives two projects the same ID.
/// The data-directory counter spans every base the machine has written to, which
/// is what closes that hole.
#[test]
fn unplugging_a_base_does_not_restart_numbering() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let archive = install.join("archive");
        let main = install.join("main");
        fs::create_dir_all(&archive).unwrap();
        fs::create_dir_all(&main).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();

        // Five projects in the archive base.
        let mut cfg = Config::default();
        cfg.base_dir = archive.display().to_string();
        for i in 1..=5 {
            vars.insert("name".to_string(), format!("arch{i}"));
            let mut counters = Counters::load().unwrap();
            let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
            project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();
        }
        assert_eq!(Counters::load_base(&archive), 5);

        // Unplug it: gone from disk and from the config.
        fs::rename(&archive, install.join("archive.unplugged")).unwrap();
        let mut cfg = Config::default();
        cfg.base_dir = main.display().to_string();

        vars.insert("name".to_string(), "first on main".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        assert_eq!(
            plan.id_str, "T006",
            "numbering restarted after the archive base was unplugged — \
             reconnecting it would produce two projects with the same ID"
        );
    });
}

#[test]
fn apply_skips_existing_and_creates_missing() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        // Create a target folder with only README.md pre-populated.
        let target = install.join("existing");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("README.md"), "pre-existing content").unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "already there".to_string());

        project::apply(&tmpl, &target, &vars, &cfg).unwrap();

        // README was skipped (content unchanged).
        let readme = fs::read_to_string(target.join("README.md")).unwrap();
        assert_eq!(readme, "pre-existing content");
        // But src/core were created.
        assert!(target.join("src").join("core").is_dir());
    });
}

#[test]
fn create_rejects_parent_escape_via_variable() {
    // Folder-form templates can't author an escaping *path* on disk, but a file
    // name can interpolate a variable. A `..` value must never produce a write
    // outside the project root.
    //
    // Planning validates rendered paths before a folder is claimed. This
    // asserts the property — the plan fails and nothing lands outside the root
    // — rather than the wording of whichever guard fires first.
    sandboxed(|install| {
        let yaml = r#"name: Bad
slug: bad
naming_pattern: "{id}"
variables:
  - slug: sub
    label: Sub
    type: text
    required: true
files:
  - path: "{sub}/keep.txt"
    content: nope
"#;
        write_template(install, "bad", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("bad").unwrap();
        let mut vars = HashMap::new();
        vars.insert("sub".to_string(), "..".to_string());
        let counters = Counters::load().unwrap();
        let err = project::plan(&tmpl, &vars, &cfg, &counters)
            .expect_err("escaping file name must be rejected before folder claim");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("..") || msg.contains("relative") || msg.contains("empty"),
            "expected a path-safety rejection, got: {msg}"
        );

        // The guarantee that actually matters: nothing was written above the
        // base, and the rolled-back create left no partial project behind.
        assert!(
            !install.join("keep.txt").exists(),
            "a file escaped the project root"
        );
        assert!(
            !Path::new(&cfg.base_dir).join("keep.txt").exists(),
            "a file escaped the project root into the base"
        );
        assert!(
            fs::read_dir(&cfg.base_dir).unwrap().next().is_none(),
            "failed plan must not claim a project folder"
        );
    });
}

#[test]
fn template_validate_rejects_absolute_file_path() {
    // `validate()` still guards `self.files` against escaping paths — the safety
    // net for templates built in memory (e.g. the UI's save path), which never
    // touch the folder-form disk scan.
    let mut t = template::Template::default();
    t.name = "bad".to_string();
    t.slug = "bad".to_string();
    t.naming_pattern = "{id}".to_string();
    t.files = vec![template::FileEntry {
        path: "/etc/passwd".to_string(),
        template: String::new(),
        content: "nope".to_string(),
    }];
    let err = t.validate().expect_err("should reject");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("relative") || msg.contains("drive letter"),
        "got: {msg}"
    );
}

#[test]
fn template_slug_and_structure_paths_are_contained() {
    sandboxed(|install| {
        let outside = install.join("sentinel");
        fs::write(&outside, b"untouched").unwrap();
        let error = template::find_by_slug("../sentinel").unwrap_err();
        assert!(error.to_string().contains("slug"));
        assert_eq!(fs::read(&outside).unwrap(), b"untouched");

        let mut invalid_slug = template::Template {
            name: "Unsafe slug".to_string(),
            slug: "../escaped".to_string(),
            naming_pattern: "{id}".to_string(),
            ..template::Template::default()
        };
        let escaped_dir = install.join("escaped");
        let derived_path = install
            .join("templates")
            .join(&invalid_slug.slug)
            .join("template.yaml");
        assert!(invalid_slug.save_to_file(&derived_path).is_err());
        assert!(
            !escaped_dir.exists(),
            "slug rejection must happen before creating a derived directory"
        );

        invalid_slug.slug = "unsafe".to_string();
        let mut unsafe_template = template::Template {
            name: "Unsafe".to_string(),
            slug: "unsafe".to_string(),
            naming_pattern: "{id}".to_string(),
            structure: vec![template::FolderNode {
                name: "../outside".to_string(),
                children: vec![],
            }],
            ..template::Template::default()
        };
        assert!(unsafe_template.validate().is_err());

        unsafe_template.structure[0].name = "src/components".to_string();
        unsafe_template.validate().unwrap();
    });
}

#[test]
fn rendered_structure_escape_is_rejected_before_folder_claim() {
    sandboxed(|install| {
        let yaml = r#"name: Rendered safety
slug: rendered-safety
naming_pattern: "{id}"
structure:
  - name: "{date}"
"#;
        write_template(install, "rendered-safety", yaml);
        let base = install.join("projects");
        fs::create_dir_all(&base).unwrap();
        let cfg = Config {
            base_dir: base.display().to_string(),
            date_format: "../outside".to_string(),
            ..Config::default()
        };
        let tmpl = template::find_by_slug("rendered-safety").unwrap();
        let counters = Counters::load().unwrap();
        let error = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap_err();
        assert!(format!("{error:#}").contains(".."));
        assert!(!base.join("outside").exists());
        assert!(fs::read_dir(&base).unwrap().next().is_none());
    });
}

#[test]
fn from_folder_round_trip() {
    sandboxed(|install| {
        // Build a small fixture folder tree on disk.
        let src = install.join("fixture");
        fs::create_dir_all(src.join("subdir")).unwrap();
        fs::write(src.join("README.md"), "hello").unwrap();
        fs::write(src.join("subdir").join("nested.txt"), "deep").unwrap();
        // A noise dir that should be ignored.
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::write(src.join(".git").join("HEAD"), "noise").unwrap();

        let report = fastf::cli::template::from_folder(
            &src.display().to_string(),
            "generated",
            false,
            false,
        )
        .unwrap();
        assert_eq!(report.text_files, 2);
        assert_eq!(report.bundled, 0);

        let tmpl = template::find_by_slug("generated").unwrap();
        // The .git folder must be absent.
        assert!(
            tmpl.structure.iter().all(|n| n.name != ".git"),
            "structure: {:?}",
            tmpl.structure
        );
        // subdir should be a folder node.
        assert!(tmpl.structure.iter().any(|n| n.name == "subdir"));
        // Files captured with relative paths.
        assert!(tmpl.files.iter().any(|f| f.path == "README.md"));
        assert!(tmpl.files.iter().any(|f| f.path == "subdir/nested.txt"));
    });
}

#[test]
fn from_folder_bundles_binaries_when_requested() {
    sandboxed(|install| {
        let src = install.join("kit");
        fs::create_dir_all(src.join("assets")).unwrap();
        fs::write(src.join("brief.md"), "# Brief").unwrap();
        // A non-UTF-8 binary blob that must be bundled byte-for-byte.
        let blob: [u8; 5] = [0x00, 0xFF, 0x10, 0x80, 0x01];
        fs::write(src.join("assets").join("logo.bin"), blob).unwrap();

        // Without bundling: the binary is skipped, not reproduced.
        let plain = fastf::cli::template::from_folder(
            &src.display().to_string(),
            "kit-plain",
            false,
            false,
        )
        .unwrap();
        assert_eq!(plain.text_files, 1);
        assert_eq!(plain.bundled, 0);
        assert_eq!(plain.skipped, 1);
        assert!(
            !fastf::util::paths::template_files_dir("kit-plain")
                .join("assets/logo.bin")
                .exists()
        );

        // With bundling: the binary lands byte-for-byte under files/.
        let bundled =
            fastf::cli::template::from_folder(&src.display().to_string(), "kit-full", false, true)
                .unwrap();
        assert_eq!(bundled.bundled, 1);
        assert_eq!(bundled.bundled_bytes, blob.len() as u64);
        let landed = fastf::util::paths::template_files_dir("kit-full").join("assets/logo.bin");
        assert_eq!(fs::read(&landed).unwrap(), blob);
    });
}

#[test]
fn sanitize_and_safe_path_units_exposed_via_lib() {
    // Smoke-test that the lib re-exports the naming helpers as expected —
    // protects against someone pruning the module accidentally.
    assert_eq!(naming::sanitize_name("a/b"), "a_b");
    assert!(fastf::core::validated::SafeRelativePath::parse("foo/bar.txt").is_ok());
    assert!(fastf::core::validated::SafeRelativePath::parse("../bad").is_err());
}

#[test]
fn dry_run_does_not_write() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "dry".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();

        // plan() does not touch disk — verify counters.toml and project folder still absent.
        assert!(
            !PathBuf::from(&cfg.base_dir)
                .join(&plan.folder_name)
                .exists()
        );
        assert!(Counters::load_base(std::path::Path::new(&cfg.base_dir)) == 0);
    });
}

#[cfg(windows)]
#[test]
fn windows_forward_slash_paths_work() {
    sandboxed(|install| {
        let yaml = r#"name: Slashes
slug: slashes
naming_pattern: "{id}"
id:
  prefix: S
  digits: 2
files:
  - path: a/b/c.txt
    content: hi
"#;
        write_template(install, "slashes", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("slashes").unwrap();
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &HashMap::new(), &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // On Windows, join() converts to backslashes. File should exist either way.
        assert!(plan.root_path.join("a").join("b").join("c.txt").is_file());
    });
}

#[test]
fn project_info_md_written_on_new_with_resolved_variables() {
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "metadata test".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let path = plan.root_path.join("PROJECT_INFO.md");
        assert!(
            path.is_file(),
            "PROJECT_INFO.md should exist at {}",
            path.display()
        );

        let body = fs::read_to_string(&path).unwrap();
        // Frontmatter shape — the file MUST start with `---\n` and contain
        // a closing `---` line. This is the searchability guarantee.
        assert!(
            body.starts_with("---\n"),
            "must start with YAML frontmatter open: {body}"
        );
        assert!(
            body.contains("\n---\n"),
            "must close YAML frontmatter: {body}"
        );
        // Frontmatter content
        assert!(
            body.contains("id: T001"),
            "missing id in frontmatter: {body}"
        );
        assert!(
            body.contains("template: test"),
            "missing template slug: {body}"
        );
        assert!(
            body.contains("template_name: Test"),
            "missing template_name: {body}"
        );
        // Variable slug + transformed value, captured under `variables:`
        assert!(
            body.contains("name: Metadata_Test"),
            "missing transformed variable: {body}"
        );
        // Human body — Project Info header + variables table + Notes section
        assert!(body.contains("# Project Info"), "missing header: {body}");
        assert!(
            body.contains("| Variable"),
            "missing variables table: {body}"
        );
        assert!(body.contains("## Notes"), "missing Notes section: {body}");

        // Raw read round-trip
        let read_back = project_info::read(&plan.root_path).unwrap();
        assert_eq!(read_back, body);
    });
}

#[test]
fn project_info_metadata_round_trips_via_yaml() {
    // Parsing the file back via read_metadata should reconstruct the typed
    // Metadata struct cleanly — this is the contract that future search /
    // index tools will rely on.
    sandboxed(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "round trip".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .expect("read_metadata Ok")
            .expect("frontmatter present");

        assert_eq!(meta.id, "T001");
        assert_eq!(meta.template, "test");
        assert_eq!(meta.template_name, "Test");
        assert_eq!(meta.folder, plan.folder_name);
        assert_eq!(
            meta.variables.get("name").map(String::as_str),
            Some("Round_Trip")
        );
    });
}

#[test]
fn project_info_captures_variables_not_in_naming_pattern() {
    // The metadata file is the durable home for variables that don't make it
    // into the folder name — that's the user's stated workflow.
    sandboxed(|install| {
        // Naming pattern uses only {id}_{title}, but `artist` is also a variable.
        let yaml = r#"name: Music
slug: music
naming_pattern: "{id}_{title}"
id:
  prefix: M
  digits: 3
variables:
  - slug: title
    label: Song Title
    type: text
    required: true
    transform: title_underscore
  - slug: artist
    label: Artist Name
    type: text
    required: false
    transform: title_underscore
structure:
  - name: assets
"#;
        write_template(install, "music", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("music").unwrap();
        let mut vars = HashMap::new();
        vars.insert("title".to_string(), "lullaby".to_string());
        vars.insert("artist".to_string(), "ariana grande".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        // Folder name does NOT include artist
        assert!(!plan.folder_name.to_lowercase().contains("ariana"));

        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        // Both vars are recorded — even the one absent from the folder name.
        assert_eq!(
            meta.variables.get("title").map(String::as_str),
            Some("Lullaby")
        );
        assert_eq!(
            meta.variables.get("artist").map(String::as_str),
            Some("Ariana_Grande")
        );
    });
}

#[test]
fn config_ignores_removed_project_info_keys() {
    // v0.9 dropped the `project_info_*` / `pinfo_*` config knobs (metadata is now
    // mandatory and always named PROJECT_INFO.md). Old configs that still carry
    // those keys must keep parsing — serde ignores unknown fields — and the
    // surviving fields must load normally.
    let raw = r#"
base_dir = "/tmp/x"
editor = ""
default_template = ""
date_format = "%Y-%m-%d"
pinfo_enabled = false
pinfo_filename = ".legacy-info.md"
project_info_enabled = false
project_info_filename = ".fastf-info.md"
"#;
    let cfg: Config = toml::from_str(raw).expect("config with removed keys should still parse");
    assert_eq!(cfg.base_dir, "/tmp/x");
    assert!(cfg.confirm_create);
}

#[test]
fn config_defaults_are_backwards_compatible() {
    // An old config.toml that predates the new fields must still parse,
    // and the new fields must take their defaults.
    let raw = r#"
base_dir = ""
editor = ""
default_template = ""
date_format = "%Y-%m-%d"
"#;
    let cfg: Config = toml::from_str(raw).expect("old config should still parse");
    assert!(cfg.prompt_open_after_create, "default should be true");
    assert_eq!(cfg.recent_default_limit, 20);
    assert!(cfg.confirm_create);
    assert!(cfg.show_banner);
    assert!(cfg.bases.is_empty());
}

#[test]
fn bundled_templates_do_not_emit_duplicate_project_info() {
    // Auto-gen owns PROJECT_INFO.md — bundled templates must not also
    // declare it as a content file (would conflict / overwrite). This guards
    // against accidental re-introduction.
    use fastf::core::template::Template;
    let bundled_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("templates");
    // Also check the strings baked into bootstrap.rs by parsing each file
    // currently shipped in the gallery.
    for entry in fs::read_dir(&bundled_dir).unwrap() {
        let entry = entry.unwrap();
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest = dir.join("template.yaml");
        if !manifest.exists() {
            continue;
        }
        let tmpl = Template::load_from_file(&manifest)
            .unwrap_or_else(|e| panic!("parse {}: {}", manifest.display(), e));
        for f in &tmpl.files {
            assert_ne!(
                f.path,
                "PROJECT_INFO.md",
                "{} declares PROJECT_INFO.md but auto-gen now owns it",
                manifest.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Names that must never reach the filesystem
// ---------------------------------------------------------------------------

/// A gallery-style template: the folder name is the user's answer, with no
/// `{id}` to make it safe. Every domain template in `examples/templates/` is
/// shaped this way, so what a `{name}` renders to *is* the folder name.
fn name_only_template_yaml(slug: &str) -> String {
    format!(
        r#"name: Name Only
slug: {slug}
description: fixture
naming_pattern: "{{name}}"
id:
  prefix: T
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: none
"#
    )
}

/// `--name=.hidden` used to create a project fastf could not see: discovery
/// skips dot-prefixed directories, so the folder showed up once from the
/// write-through cache and then vanished at the next rescan.
///
/// `--name=..` was worse. It sanitizes to `""`, `base.join("")` is the base,
/// which `exists()` answers yes to, so the collision loop moved on to `_2` — and
/// `create_inner` resolved that against the base's *parent*, planting `_2`
/// one level above the library.
///
/// Both must fail in `plan`, before a single directory is created.
#[test]
fn a_name_that_would_be_invisible_or_empty_is_refused_before_anything_is_written() {
    sandboxed(|install| {
        write_template(install, "named", &name_only_template_yaml("named"));

        let mut cfg = Config::default();
        let base = install.join("projects");
        fs::create_dir_all(&base).unwrap();
        cfg.base_dir = base.display().to_string();

        let tmpl = template::find_by_slug("named").unwrap();
        let counters = Counters::load().unwrap();

        for (raw, expected) in [
            (".hidden", "may not start with '.'"),
            ("..", "leaves no usable folder name"),
            (".", "leaves no usable folder name"),
            // `"   "` is not here: the required-variable check refuses an
            // all-whitespace answer one layer earlier, which is the better
            // error. What matters is that nothing gets through, not which
            // gate stops it.
        ] {
            let mut vars = HashMap::new();
            vars.insert("name".to_string(), raw.to_string());
            let error = project::plan(&tmpl, &vars, &cfg, &counters)
                .expect_err("the plan must be refused")
                .chain()
                .map(|cause| cause.to_string())
                .collect::<Vec<_>>()
                .join(": ");

            assert!(error.contains(expected), "--name={raw:?} gave: {error}");
            // The error names the pattern too — the user typed a variable, not
            // a folder name, and needs to see how it became one.
            assert!(error.contains("{name}"), "--name={raw:?} gave: {error}");
        }

        // Nothing was created: not in the base, not beside it.
        assert_eq!(
            fs::read_dir(&base).unwrap().count(),
            0,
            "the base must still be empty"
        );
        let stray: Vec<String> = fs::read_dir(install)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('_'))
            .collect();
        assert!(stray.is_empty(), "planted beside the base: {stray:?}");

        // And no ID was burned.
        assert_eq!(Counters::load_base(&base), 0);
    });
}

/// The same rule, one layer earlier: a template whose pattern *starts* with `.`
/// renders a dot-prefixed name for every project it ever makes, so it is refused
/// when the template is saved rather than once per create.
#[test]
fn a_template_whose_pattern_starts_with_a_dot_cannot_be_saved() {
    sandboxed(|install| {
        let dir = install.join("templates").join("hidden");
        fs::create_dir_all(&dir).unwrap();

        let mut tmpl = template::Template {
            name: "Hidden".to_string(),
            slug: "hidden".to_string(),
            naming_pattern: ".{id}".to_string(),
            ..Default::default()
        };

        let manifest = dir.join("template.yaml");
        let error = tmpl
            .save_to_file(&manifest)
            .expect_err("a dot-prefixed pattern must be refused")
            .to_string();
        assert!(error.contains("may not start with '.'"), "{error}");
        assert!(
            !manifest.exists(),
            "validation must precede the write, not follow it"
        );

        // The same template with a visible pattern saves fine.
        tmpl.naming_pattern = "{id}".to_string();
        tmpl.save_to_file(&manifest).unwrap();
        assert!(manifest.exists());
    });
}
