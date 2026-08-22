//! Integration tests for fastf.
//!
//! Each test drops a `FASTF_INSTALL_DIR` env override so that config,
//! counters, templates, and the project index all live in a fresh tempdir.
//! No test touches the real installed fastf folder.
//!
//! Tests run serially in a single-threaded runner (see the `serial` helper).
//! This is deliberate: `FASTF_INSTALL_DIR` is process-wide, so parallel tests
//! in the same binary would race. Compared to pulling in `serial_test`, a
//! Mutex we own is leaner and explicit.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastf::core::{
    config::Config, counter::Counters, library, naming, project, project_info, query, template,
};

static SERIAL: Mutex<()> = Mutex::new(());

/// Acquire the serial-test lock and install a fresh `FASTF_INSTALL_DIR`.
fn with_fresh_install<R>(body: impl FnOnce(&Path) -> R) -> R {
    // Recover from poisoned lock — we don't hold any invariants that panics could violate.
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    // Safe here: the SERIAL mutex guarantees no other test thread races on these env vars.
    // Home is redirected into the sandbox too: an unconfigured base_dir falls
    // back to the home directory, and tests must never scan the real one.
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let old_home = std::env::var_os(home_var);
    unsafe {
        std::env::set_var("FASTF_INSTALL_DIR", tmp.path());
        std::env::set_var(home_var, tmp.path());
    }
    fs::create_dir_all(tmp.path().join("templates")).unwrap();
    let result = body(tmp.path());
    unsafe {
        std::env::remove_var("FASTF_INSTALL_DIR");
        match old_home {
            Some(value) => std::env::set_var(home_var, value),
            None => std::env::remove_var(home_var),
        }
    }
    drop(guard);
    result
}

/// Install a template in v0.8 folder form: `templates/<slug>/template.yaml`
/// plus a `files/` subtree. For test convenience the fixture YAML may still
/// carry an inline `files:` block (as older flat templates did); this helper
/// splits it out onto disk exactly like the real conversion, so the copy engine
/// (which walks `files/`) sees the files. The `files:` key left in the manifest
/// is an unknown field ignored by `Template`'s deserializer.
fn write_template(install: &Path, slug: &str, yaml: &str) {
    #[derive(serde::Deserialize)]
    struct InlineFiles {
        #[serde(default)]
        files: Vec<InlineFile>,
    }
    #[derive(serde::Deserialize)]
    struct InlineFile {
        path: String,
        #[serde(default)]
        template: String,
        #[serde(default)]
        content: String,
    }
    let dir = install.join("templates").join(slug);
    fs::create_dir_all(dir.join("files")).unwrap();
    fs::write(dir.join("template.yaml"), yaml).unwrap();
    if let Ok(inline) = serde_yaml::from_str::<InlineFiles>(yaml) {
        for f in inline.files {
            let body = if !f.template.is_empty() {
                f.template
            } else {
                f.content
            };
            let dest = dir.join("files").join(&f.path);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::write(dest, body).unwrap();
        }
    }
}

/// A minimal valid template with one text var, one folder, and one templated file.
fn minimal_template_yaml(slug: &str) -> String {
    format!(
        r#"name: Test
slug: {slug}
description: fixture
naming_pattern: "{{id}}_{{name}}"
id:
  prefix: T
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: title_underscore
structure:
  - name: src
    children:
      - name: core
files:
  - path: README.md
    template: |
      # {{name}}
      id: {{id}}
"#
    )
}

// ---------------------------------------------------------------------------

#[test]
fn create_project_basic_round_trip() {
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        // The default is now to append `_2`, since a naming pattern need not
        // contain `{id}`. `error` restores the old refuse-a-duplicate guard,
        // which is what this test is about.
        cfg.on_name_collision = "error".to_string();
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
    with_fresh_install(|install| {
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
// Tags — write_frontmatter + auto-tag + tag CLI
// ---------------------------------------------------------------------------

/// Template with `tags` and `tag_from` should produce combined tags in frontmatter.
#[test]
fn auto_tag_from_template_tag_from() {
    with_fresh_install(|install| {
        let yaml = r#"name: Tagged
slug: tagged
naming_pattern: "{id}_{name}"
id:
  prefix: T
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: title_underscore
  - slug: client_type
    label: Client type
    type: text
tags:
  - creative
tag_from:
  - client_type
"#;
        write_template(install, "tagged", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("tagged").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "My Project".to_string());
        vars.insert("client_type".to_string(), "Indie".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();

        assert!(
            meta.tags.contains(&"creative".to_string()),
            "literal tag should be present: {:?}",
            meta.tags
        );
        assert!(
            meta.tags.contains(&"client_type/Indie".to_string()),
            "derived tag should be present: {:?}",
            meta.tags
        );
        assert_eq!(meta.tags.len(), 2);
    });
}

/// Empty tag_from value should not produce an orphan `slug/` tag.
#[test]
fn auto_tag_skips_empty_variable_value() {
    with_fresh_install(|install| {
        let yaml = r#"name: Tagged2
slug: tagged2
naming_pattern: "{id}"
variables:
  - slug: client_type
    label: Client type
    type: text
tag_from:
  - client_type
"#;
        write_template(install, "tagged2", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("tagged2").unwrap();
        // leave client_type empty
        let vars = HashMap::new();
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        assert!(
            meta.tags.is_empty(),
            "should have no tags when variable is empty: {:?}",
            meta.tags
        );
    });
}

/// write_frontmatter preserves the body bytes unchanged when only tags are mutated.
#[test]
fn write_frontmatter_body_bytes_preserved() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "roundtrip".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        // Record the body section before mutation.
        let before = fs::read_to_string(&pinfo).unwrap();
        let (_, body_before) = project_info::split_frontmatter_body(&before).unwrap();
        let body_before = body_before.to_string();

        // Mutate via write_frontmatter.
        project_info::write_frontmatter(&pinfo, |meta| {
            meta.tags.push("draft".to_string());
        })
        .unwrap();

        // Read back and compare body.
        let after = fs::read_to_string(&pinfo).unwrap();
        let (_, body_after) = project_info::split_frontmatter_body(&after).unwrap();

        assert_eq!(
            body_before, body_after,
            "body bytes must be identical after frontmatter mutation"
        );

        // Tag must be present.
        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        assert!(meta.tags.contains(&"draft".to_string()));
    });
}

/// write_frontmatter returns a structured error when frontmatter is missing.
#[test]
fn write_frontmatter_errors_on_missing_frontmatter() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("plain.md");
    fs::write(&path, "# No frontmatter here\n\nJust text.\n").unwrap();

    let result = project_info::write_frontmatter(&path, |_| {});
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("no YAML frontmatter"),
        "error should mention missing frontmatter: {msg}"
    );
}

/// append_journal_entry creates the section when it doesn't exist and appends
/// additional entries chronologically.
#[test]
fn append_journal_entry_creates_and_appends() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "journal-test".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        // No journal section yet.
        let content = fs::read_to_string(&pinfo).unwrap();
        assert!(
            !content.contains("## Journal"),
            "no journal before first append"
        );

        // First entry — should create the section.
        project_info::append_journal_entry(&pinfo, "first note").unwrap();
        let after_first = fs::read_to_string(&pinfo).unwrap();
        assert!(after_first.contains("## Journal"));
        assert!(after_first.contains("first note"));

        // Second entry — appended after first.
        project_info::append_journal_entry(&pinfo, "second note").unwrap();
        let after_second = fs::read_to_string(&pinfo).unwrap();
        assert!(after_second.contains("first note"));
        assert!(after_second.contains("second note"));
        // Chronological: first appears before second.
        let pos_first = after_second.find("first note").unwrap();
        let pos_second = after_second.find("second note").unwrap();
        assert!(pos_first < pos_second, "entries should be chronological");
    });
}

/// read_journal_entries parses entries from the file correctly.
#[test]
fn journal_entries_round_trip() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "journal-rtrip".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        project_info::append_journal_entry(&pinfo, "alpha").unwrap();
        project_info::append_journal_entry(&pinfo, "beta").unwrap();

        let entries = project_info::read_journal_entries(&plan.root_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "alpha");
        assert_eq!(entries[1].message, "beta");
    });
}

/// Tags added via write_frontmatter persist across a restart (re-parse).
#[test]
fn tag_add_persists() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "tag-persist".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);

        // Add twice — should be idempotent.
        project_info::write_frontmatter(&pinfo, |m| {
            if !m.tags.contains(&"draft".to_string()) {
                m.tags.push("draft".to_string());
            }
        })
        .unwrap();
        project_info::write_frontmatter(&pinfo, |m| {
            if !m.tags.contains(&"draft".to_string()) {
                m.tags.push("draft".to_string());
            }
        })
        .unwrap();

        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        let count = meta.tags.iter().filter(|t| t.as_str() == "draft").count();
        assert_eq!(
            count, 1,
            "idempotent add should not duplicate: {:?}",
            meta.tags
        );

        // Remove.
        project_info::write_frontmatter(&pinfo, |m| m.tags.retain(|t| t != "draft")).unwrap();
        let meta2 = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();
        assert!(!meta2.tags.contains(&"draft".to_string()));
    });
}

/// Older PROJECT_INFO.md without tags: field loads with an empty tags vec.
#[test]
fn legacy_metadata_without_tags_loads_cleanly() {
    // Simulate a file written before tagging was introduced.
    let legacy_yaml = r#"---
id: ID0001
template: old-template
template_name: Old Template
created: "2025-01-01T00:00:00Z"
folder: ID0001_Project
path: /projects/ID0001_Project
variables:
  name: hello
---

# Project Info

## Notes

"#;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let file = dir.join("PROJECT_INFO.md");
    fs::write(&file, legacy_yaml).unwrap();

    let meta = project_info::read_metadata(dir).unwrap().unwrap();
    assert_eq!(meta.id, "ID0001");
    assert!(
        meta.tags.is_empty(),
        "legacy file should deserialize with empty tags"
    );
}

// ---------------------------------------------------------------------------
// Search — query parser + evaluator
// ---------------------------------------------------------------------------

/// Each query operator returns correct matches on a synthesised metadata set.
#[test]
fn query_predicates_each_operator() {
    use std::collections::BTreeMap;

    let make_meta = |id: &str, tmpl: &str, created: &str, tags: &[&str], vars: &[(&str, &str)]| {
        project_info::Metadata {
            id: id.to_string(),
            template: tmpl.to_string(),
            template_name: tmpl.to_string(),
            created: created.to_string(),
            folder: format!("{id}_proj"),
            path: format!("/projects/{id}_proj"),
            variables: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            provisioning: false,
        }
    };

    // exact field
    let meta = make_meta(
        "ID0001",
        "music-video",
        "2026-03-01T00:00:00Z",
        &["draft"],
        &[("artist", "Ariana")],
    );
    assert!(query::evaluate(
        &query::parse(&["template=music-video".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["template=other".to_string()]),
        &meta
    ));

    // prefix glob
    assert!(query::evaluate(
        &query::parse(&["template=music*".to_string()]),
        &meta
    ));

    // date after
    assert!(query::evaluate(
        &query::parse(&["created>2026-01-01".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["created>2027-01-01".to_string()]),
        &meta
    ));

    // date before
    assert!(query::evaluate(
        &query::parse(&["created<2027-01-01".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["created<2025-01-01".to_string()]),
        &meta
    ));

    // exact tag
    assert!(query::evaluate(
        &query::parse(&["tag:draft".to_string()]),
        &meta
    ));
    assert!(!query::evaluate(
        &query::parse(&["tag:urgent".to_string()]),
        &meta
    ));

    // tag glob
    assert!(query::evaluate(
        &query::parse(&["tag:dra*".to_string()]),
        &meta
    ));

    // variable field
    assert!(query::evaluate(
        &query::parse(&["artist=Ariana".to_string()]),
        &meta
    ));
    assert!(query::evaluate(
        &query::parse(&["artist=Aria*".to_string()]),
        &meta
    ));

    // multi-clause AND
    assert!(query::evaluate(
        &query::parse(&["template=music-video".to_string(), "tag:draft".to_string()]),
        &meta,
    ));
    assert!(!query::evaluate(
        &query::parse(&["template=music-video".to_string(), "tag:urgent".to_string()]),
        &meta,
    ));

    // unknown key → false, not error
    assert!(!query::evaluate(
        &query::parse(&["nonexistent=anything".to_string()]),
        &meta
    ));
}

/// Bare-term default mode searches across vars, tags, folder, template, and id
/// — and explicitly excludes `path`.  Drives the end-to-end create→read→evaluate
/// path so we know the predicate works against real frontmatter on disk.
#[test]
fn query_free_term_searches_across_fields() {
    with_fresh_install(|install| {
        // Template with one variable so we can verify variable-value matching.
        let yaml = r#"name: Free Search
slug: free-search
naming_pattern: "{id}_{title}"
id:
  prefix: F
  digits: 3
variables:
  - slug: title
    label: Title
    type: text
    required: true
    transform: title_underscore
  - slug: artist
    label: Artist
    type: text
    transform: title_underscore
tags:
  - creative
"#;
        write_template(install, "free-search", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("free-search").unwrap();
        let mut vars = HashMap::new();
        vars.insert("title".to_string(), "Lullaby".to_string());
        vars.insert("artist".to_string(), "Ariana Grande".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Read the on-disk metadata for predicate evaluation.
        let meta = project_info::read_metadata(&plan.root_path)
            .unwrap()
            .unwrap();

        // Variable value match (case-insensitive)
        assert!(
            query::evaluate(&query::parse(&["ariana".to_string()]), &meta),
            "should match variable value 'Ariana_Grande'"
        );

        // Tag match
        assert!(
            query::evaluate(&query::parse(&["creative".to_string()]), &meta),
            "should match tag 'creative'"
        );

        // Folder name (the resolved naming pattern)
        assert!(
            query::evaluate(&query::parse(&["lullaby".to_string()]), &meta),
            "should match folder name '{}'",
            plan.folder_name
        );

        // Template slug
        assert!(
            query::evaluate(&query::parse(&["free-search".to_string()]), &meta),
            "should match template slug 'free-search'"
        );

        // ID
        assert!(
            query::evaluate(&query::parse(std::slice::from_ref(&plan.id_str)), &meta),
            "should match ID '{}'",
            plan.id_str
        );

        // Multi-term AND: both must appear somewhere
        assert!(
            query::evaluate(
                &query::parse(&["ariana".to_string(), "lullaby".to_string()]),
                &meta
            ),
            "two bare terms should AND across different fields"
        );

        // Free + explicit clause AND
        assert!(
            query::evaluate(
                &query::parse(&["ariana".to_string(), "tag:creative".to_string()]),
                &meta
            ),
            "free term should AND with explicit tag clause"
        );

        // No match
        assert!(
            !query::evaluate(&query::parse(&["xyzzy".to_string()]), &meta),
            "unmatched bare term should return false"
        );

        // Path is excluded — find a substring that exists ONLY in the path,
        // not in folder name / vars / tags / template / id / template_name.
        // The base_dir component "projects" appears in the path but should
        // NOT be searchable as a free term.
        // (Defensive: only assert this if "projects" is genuinely absent
        // from the other fields, which it is for this fixture.)
        assert!(!plan.folder_name.to_lowercase().contains("projects"));
        assert!(!meta.template.to_lowercase().contains("projects"));
        assert!(!meta.template_name.to_lowercase().contains("projects"));
        assert!(
            !query::evaluate(&query::parse(&["projects".to_string()]), &meta),
            "bare term that lives only in path must NOT match"
        );
    });
}

/// Every YAML in `examples/templates/` must parse, validate, and plan — it's the
/// public gallery users copy from, so broken YAML would be very visible.
#[test]
fn gallery_templates_parse_and_plan() {
    let gallery = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("templates");
    let entries = fs::read_dir(&gallery)
        .unwrap_or_else(|e| panic!("missing gallery at {}: {}", gallery.display(), e));

    let mut seen = 0;
    for entry in entries {
        let entry = entry.unwrap();
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest = dir.join("template.yaml");
        if !manifest.exists() {
            continue;
        }
        seen += 1;
        let tmpl = template::Template::load_from_file(&manifest)
            .unwrap_or_else(|e| panic!("failed to parse {}: {}", manifest.display(), e));
        tmpl.validate()
            .unwrap_or_else(|e| panic!("failed to validate {}: {}", manifest.display(), e));
    }
    assert!(
        seen >= 5,
        "expected at least 5 gallery templates, found {seen}"
    );
}

// ---------------------------------------------------------------------------
// PROJECT_INFO.md is a reserved fastf-managed filename.  Templates may declare
// it (e.g. old user-built templates from before the safety net), but the
// entry is silently stripped on load and save so the auto-gen always wins
// at write time.
// ---------------------------------------------------------------------------

#[test]
fn template_load_strips_reserved_project_info_entry() {
    with_fresh_install(|install| {
        // Mirrors the user's `general.yaml`: a real template with a
        // `PROJECT_INFO.md` file entry left over from the pre-fix builder.
        let yaml = r#"name: General
slug: general
naming_pattern: "{id}_{title}"
id:
  prefix: ID
  digits: 4
variables:
  - slug: title
    label: Title
    type: text
    required: true
files:
  - path: PROJECT_INFO.md
    template: |
      Notes
  - path: NOTES.md
    content: |
      hand-edited
"#;
        write_template(install, "general", yaml);

        let t = template::find_by_slug("general").unwrap();
        // The reserved entry is gone; the NOTES.md entry survives.
        assert_eq!(
            t.files.len(),
            1,
            "expected only NOTES.md, got {:?}",
            t.files
        );
        assert_eq!(t.files[0].path, "NOTES.md");
    });
}

#[test]
fn template_save_strips_reserved_project_info_entry() {
    with_fresh_install(|install| {
        // Build a template in memory with a reserved entry and save it.
        // Save-time strip should rewrite the YAML without that entry.
        let mut t = template::Template::default();
        t.name = "x".to_string();
        t.slug = "x".to_string();
        t.naming_pattern = "{id}".to_string();
        t.files = vec![
            template::FileEntry {
                path: "PROJECT_INFO.md".to_string(),
                template: "ignored\n".to_string(),
                content: String::new(),
            },
            template::FileEntry {
                path: "KEEP.md".to_string(),
                template: String::new(),
                content: "yes\n".to_string(),
            },
        ];

        let dir = install.join("templates").join("x");
        let path = dir.join("template.yaml");
        t.save_to_file(&path).unwrap();

        // Files live on disk now: the reserved root entry must not be flushed,
        // while KEEP.md is written into files/.
        assert!(
            !dir.join("files").join("PROJECT_INFO.md").exists(),
            "reserved PROJECT_INFO.md was written to files/"
        );
        assert!(
            dir.join("files").join("KEEP.md").exists(),
            "expected KEEP.md to be flushed into files/"
        );
    });
}

#[test]
fn template_file_content_interpolates_multiple_custom_variables() {
    // End-to-end: a placeholder file with three `{token}` markers (one used
    // twice) should produce the fully-substituted text in the created project.
    // Mirrors the user-facing example exactly.
    with_fresh_install(|install| {
        let yaml = r#"name: Greeting
slug: greeting
naming_pattern: "{id}_{project_name}"
id:
  prefix: ID
  digits: 4
variables:
  - slug: client_name
    label: Client name
    type: text
    required: true
  - slug: project_name
    label: Project name
    type: text
    required: true
  - slug: producer
    label: Producer
    type: text
    required: true
files:
  - path: NOTES.md
    template: |
      This is a project for {client_name}. Thank you {client_name} for working with us on {project_name}. Also thank you {producer}.
"#;
        write_template(install, "greeting", yaml);

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("greeting").unwrap();
        let mut vars = HashMap::new();
        vars.insert("client_name".to_string(), "John Doe".to_string());
        vars.insert("project_name".to_string(), "Amazing Project".to_string());
        vars.insert("producer".to_string(), "Steven Spielberg".to_string());

        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let notes = fs::read_to_string(plan.root_path.join("NOTES.md")).unwrap();
        let expected = "This is a project for John Doe. Thank you John Doe for working with us on Amazing Project. Also thank you Steven Spielberg.\n";
        assert_eq!(notes, expected, "interpolation produced unexpected output");
    });
}

#[test]
fn copy_engine_handles_binary_verbatim_and_globs() {
    // v0.8 files/ subtree: interpolated text, byte-identical binaries, a
    // `verbatim` glob that preserves literal braces, and `exclude` globs.
    with_fresh_install(|install| {
        let yaml = r#"name: Assets
slug: assets
naming_pattern: "{id}_{name}"
id:
  prefix: A
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
    transform: title_underscore
verbatim: ["*.tmpl"]
exclude: [".DS_Store", "*.tmp"]
"#;
        let tdir = install.join("templates").join("assets");
        fs::create_dir_all(tdir.join("files")).unwrap();
        fs::write(tdir.join("template.yaml"), yaml).unwrap();
        fs::write(
            tdir.join("files").join("Note_{name}.md"),
            "Hello {name} ({id})\n",
        )
        .unwrap();
        let blob: [u8; 5] = [0x00, 0xFF, 0x10, 0x80, 0x07];
        fs::write(tdir.join("files").join("logo.bin"), blob).unwrap();
        fs::write(tdir.join("files").join("raw.tmpl"), "literal {name}\n").unwrap();
        fs::write(tdir.join("files").join(".DS_Store"), "junk").unwrap();
        fs::write(tdir.join("files").join("scratch.tmp"), "junk").unwrap();

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("assets").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "aurora".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let root = &plan.root_path;
        // Name + contents interpolated (title_underscore → Aurora).
        assert_eq!(
            fs::read_to_string(root.join("Note_Aurora.md")).unwrap(),
            "Hello Aurora (A001)\n"
        );
        // Binary copied byte-for-byte.
        assert_eq!(fs::read(root.join("logo.bin")).unwrap(), blob);
        // verbatim glob: braces left literal.
        assert_eq!(
            fs::read_to_string(root.join("raw.tmpl")).unwrap(),
            "literal {name}\n"
        );
        // exclude globs: never copied.
        assert!(!root.join(".DS_Store").exists());
        assert!(!root.join("scratch.tmp").exists());
    });
}

#[test]
fn template_keeps_subfolder_project_info() {
    with_fresh_install(|install| {
        // PROJECT_INFO.md *in a subfolder* doesn't collide with the auto-gen
        // (which lives at the project root) — the reservation is leaf-only.
        let yaml = r#"name: subdoc
slug: subdoc
naming_pattern: "{id}"
files:
  - path: docs/PROJECT_INFO.md
    content: |
      sub-doc
"#;
        write_template(install, "subdoc", yaml);
        let t = template::find_by_slug("subdoc").unwrap();
        assert_eq!(t.files.len(), 1);
        assert_eq!(t.files[0].path, "docs/PROJECT_INFO.md");
    });
}

// ---------------------------------------------------------------------------
// `fastf register <path>` — onboard existing folders into the index.
// ---------------------------------------------------------------------------

use fastf::cli::register::{
    PinfoConflict, RecursiveArgs, RegisterArgs, RegisterOptions, register_core,
    run as register_run, run_recursive,
};

fn register_args(path: &Path) -> RegisterArgs {
    RegisterArgs {
        path: path.to_path_buf(),
        template_slug: None,
        vars: HashMap::new(),
        apply_structure: false,
        rename: false,
        use_today: false,
        created_override: None,
        yes: true,
    }
}

#[test]
fn register_minimal_no_template() {
    with_fresh_install(|install| {
        let target = install.join("old-project");
        fs::create_dir_all(&target).unwrap();

        register_run(register_args(&target)).unwrap();

        // Counter bumped to 1 (minted fresh — no ID token in the folder name)
        assert_eq!(Counters::load_base(install), 1);

        // The written metadata carries the registered slug + minted ID.
        let meta = project_info::read_metadata(&target).unwrap().unwrap();
        assert_eq!(meta.id, "ID0001");
        assert_eq!(meta.template, "(registered)");
        assert_eq!(meta.folder, "old-project");

        // PROJECT_INFO.md exists with frontmatter
        let pinfo = target.join("PROJECT_INFO.md");
        assert!(
            pinfo.exists(),
            "expected PROJECT_INFO.md in {}",
            target.display()
        );
        let body = fs::read_to_string(&pinfo).unwrap();
        assert!(body.starts_with("---\n"), "missing frontmatter");
        assert!(body.contains("id: ID0001"), "missing id");
        assert!(
            body.contains("template: (registered)")
                || body.contains("template: '(registered)'")
                || body.contains("template: \"(registered)\""),
            "missing template slug; body:\n{body}"
        );
    });
}

#[test]
fn register_with_template_full_metadata() {
    with_fresh_install(|install| {
        write_template(install, "music", &minimal_template_yaml("music"));
        let target = install.join("LegacyAlbum");
        fs::create_dir_all(&target).unwrap();

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Lullaby".to_string());

        let mut args = register_args(&target);
        args.template_slug = Some("music".to_string());
        args.vars = vars;

        register_run(args).unwrap();

        // Frontmatter reflects the template + variables
        let meta = project_info::read_metadata(&target).unwrap().unwrap();
        assert_eq!(meta.template, "music");
        assert_eq!(meta.template_name, "Test");
        assert_eq!(meta.id, "T001");
        assert_eq!(
            meta.variables.get("name").map(|s| s.as_str()),
            Some("Lullaby")
        );
        // No tag_from in the minimal template, so tags should be empty.
        assert!(meta.tags.is_empty());
    });
}

#[test]
fn register_with_tag_from_emits_auto_tags() {
    with_fresh_install(|install| {
        let yaml = r#"name: Client work
slug: client
naming_pattern: "{id}_{name}"
id:
  prefix: C
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
  - slug: tier
    label: Tier
    type: select
    options: [Indie, Major]
    default: Indie
tag_from:
  - tier
"#;
        write_template(install, "client", yaml);
        let target = install.join("AcmeWork");
        fs::create_dir_all(&target).unwrap();

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Acme".to_string());
        vars.insert("tier".to_string(), "Indie".to_string());

        let mut args = register_args(&target);
        args.template_slug = Some("client".to_string());
        args.vars = vars;

        register_run(args).unwrap();

        let meta = project_info::read_metadata(&target).unwrap().unwrap();
        assert!(
            meta.tags.iter().any(|t| t == "tier/Indie"),
            "expected auto-tag tier/Indie, got: {:?}",
            meta.tags
        );
    });
}

#[test]
fn register_rejects_missing_path() {
    with_fresh_install(|install| {
        let target = install.join("does-not-exist");
        let err = register_run(register_args(&target)).expect_err("should bail");
        let msg = err.to_string();
        assert!(
            msg.contains("does not exist") || msg.contains("not accessible"),
            "got: {msg}"
        );
    });
}

#[test]
fn register_rejects_non_directory() {
    with_fresh_install(|install| {
        let target = install.join("a-file");
        fs::write(&target, "im a file").unwrap();
        let err = register_run(register_args(&target)).expect_err("should bail");
        assert!(err.to_string().contains("not a directory"), "got: {err:#}");
    });
}

#[test]
fn register_recovers_id_from_folder_name() {
    with_fresh_install(|install| {
        // Folder name carries an ID token (inconsistent padding is fine — value
        // is what matters). Register recovers it rather than minting fresh.
        let target = install.join("2026-04-19_Old_ID0030");
        fs::create_dir_all(&target).unwrap();

        register_run(register_args(&target)).unwrap();

        let meta = project_info::read_metadata(&target).unwrap().unwrap();
        assert_eq!(meta.id, "ID0030");
        // Counter self-heals up to the recovered value.
        assert!(Counters::load_base(install) >= 30);
    });
}

#[test]
fn register_recursive_onboards_children_and_previews() {
    with_fresh_install(|install| {
        let base = install.join("bulk");
        fs::create_dir_all(base.join("Alpha_ID0005")).unwrap();
        fs::create_dir_all(base.join("Beta")).unwrap();
        Config {
            base_dir: base.display().to_string(),
            ..Config::default()
        }
        .save()
        .unwrap();
        // A child that already has metadata must be skipped, untouched.
        let gamma = base.join("Gamma");
        fs::create_dir_all(&gamma).unwrap();
        fs::write(
            gamma.join("PROJECT_INFO.md"),
            "---\nid: ID0009\ntemplate: x\ntemplate_name: X\ncreated: \"2026-01-01T00:00:00Z\"\nfolder: Gamma\npath: x\nvariables: {}\ntags: []\n---\n\n# Project Info\n",
        )
        .unwrap();

        // Dry-run writes nothing.
        run_recursive(RecursiveArgs {
            base: base.clone(),
            template_slug: None,
            vars: Default::default(),
            use_today: false,
            dry_run: true,
        })
        .unwrap();
        assert!(!base.join("Alpha_ID0005/PROJECT_INFO.md").exists());
        assert!(!base.join("Beta/PROJECT_INFO.md").exists());

        // Real run onboards the two metadata-less children.
        run_recursive(RecursiveArgs {
            base: base.clone(),
            template_slug: None,
            vars: Default::default(),
            use_today: false,
            dry_run: false,
        })
        .unwrap();

        // Alpha recovered its ID from the folder name; Beta got a fresh one.
        let alpha = project_info::read_metadata(&base.join("Alpha_ID0005"))
            .unwrap()
            .unwrap();
        assert_eq!(alpha.id, "ID0005");
        assert!(base.join("Beta/PROJECT_INFO.md").exists());
        // Gamma was skipped — its original metadata is intact.
        let gamma_meta = project_info::read_metadata(&gamma).unwrap().unwrap();
        assert_eq!(gamma_meta.id, "ID0009");
    });
}

#[test]
fn register_existing_metadata_aborts_under_abort_policy() {
    with_fresh_install(|install| {
        let target = install.join("dup");
        fs::create_dir_all(&target).unwrap();

        // First register writes PROJECT_INFO.md.
        register_run(register_args(&target)).unwrap();

        // A folder that already has metadata is already a project. The Abort
        // policy (the UI default) refuses to re-register before touching disk.
        let err = register_core(RegisterOptions {
            path: target.clone(),
            template_slug: None,
            vars: HashMap::new(),
            apply_structure: false,
            rename: false,
            use_today: false,
            created_override: None,
            on_pinfo_conflict: PinfoConflict::Abort,
        })
        .expect_err("existing metadata should abort");
        assert!(err.to_string().contains("already"), "got: {err:#}");
    });
}

#[test]
fn register_created_override_lands_in_frontmatter() {
    with_fresh_install(|install| {
        let target = install.join("historic");
        fs::create_dir_all(&target).unwrap();

        let mut args = register_args(&target);
        args.created_override = Some("2024-06-15".to_string());
        register_run(args).unwrap();

        let meta = project_info::read_metadata(&target).unwrap().unwrap();
        assert_eq!(meta.created, "2024-06-15T00:00:00Z");
    });
}

#[test]
fn register_use_today_overrides_default_and_explicit_conflicts() {
    with_fresh_install(|install| {
        let target = install.join("today");
        fs::create_dir_all(&target).unwrap();

        // use_today + created_override are mutually exclusive.
        let mut args = register_args(&target);
        args.use_today = true;
        args.created_override = Some("2024-06-15".to_string());
        let err = register_run(args).expect_err("conflict should bail");
        assert!(
            err.to_string().contains("mutually exclusive"),
            "got: {err:#}"
        );
    });
}

#[test]
fn register_apply_fills_missing_structure() {
    with_fresh_install(|install| {
        write_template(install, "music", &minimal_template_yaml("music"));
        let target = install.join("existing");
        fs::create_dir_all(&target).unwrap();

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "x".to_string());

        let mut args = register_args(&target);
        args.template_slug = Some("music".to_string());
        args.apply_structure = true;
        args.vars = vars;

        register_run(args).unwrap();

        // Template's structure: src/core → both should exist after --apply
        assert!(target.join("src").join("core").is_dir());
        // And the README templated file should have been created.
        assert!(target.join("README.md").exists());
    });
}

#[test]
fn register_rename_moves_folder() {
    with_fresh_install(|install| {
        write_template(install, "music", &minimal_template_yaml("music"));
        let original = install.join("OldName");
        fs::create_dir_all(&original).unwrap();

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "hello".to_string());

        let mut args = register_args(&original);
        args.template_slug = Some("music".to_string());
        args.rename = true;
        args.vars = vars;

        register_run(args).unwrap();

        // naming_pattern is "{id}_{name}" with id=T001 and name transformed to "Hello".
        let renamed = install.join("T001_Hello");
        assert!(
            renamed.is_dir(),
            "expected renamed folder at {}",
            renamed.display()
        );
        assert!(!original.exists(), "old folder should be gone");

        // The metadata in the renamed folder reflects the new folder name.
        let meta = project_info::read_metadata(&renamed).unwrap().unwrap();
        assert_eq!(meta.folder, "T001_Hello");
    });
}

#[test]
fn register_rename_without_template_uses_default_pattern() {
    with_fresh_install(|install| {
        let target = install.join("random project");
        fs::create_dir_all(&target).unwrap();

        let mut args = register_args(&target);
        args.rename = true;
        // Force today's date so the assertion is deterministic about the date prefix.
        args.use_today = true;
        register_run(args).unwrap();

        // Default pattern is "{date}_{name}_{id}". Date = today (use_today=true);
        // {name} = sanitize_name("random project") = "random_project";
        // {id} = "ID0001". Assert on the stable suffix.
        let parent = target.parent().unwrap();
        let mut found = None;
        for entry in fs::read_dir(parent).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with("_random_project_ID0001") {
                found = Some(entry.path());
                break;
            }
        }
        assert!(
            found.is_some(),
            "expected a folder ending with '_random_project_ID0001' under {}",
            parent.display()
        );
        // Original folder name should be gone (renamed).
        assert!(!target.exists(), "original folder should have been renamed");
        // The metadata in the renamed folder reflects the new folder name.
        let meta = project_info::read_metadata(found.as_ref().unwrap())
            .unwrap()
            .unwrap();
        assert!(meta.folder.ends_with("_random_project_ID0001"));
    });
}

#[test]
fn register_rename_uses_custom_config_pattern() {
    with_fresh_install(|install| {
        // Write a config file with a non-default register_naming_pattern.
        let mut cfg = Config::default();
        cfg.register_naming_pattern = "{id}-{name}".to_string();
        let toml = toml::to_string_pretty(&cfg).unwrap();
        fs::write(install.join("config.toml"), toml).unwrap();

        let target = install.join("my folder");
        fs::create_dir_all(&target).unwrap();

        let mut args = register_args(&target);
        args.rename = true;
        register_run(args).unwrap();

        let renamed = install.join("ID0001-my_folder");
        assert!(renamed.is_dir(), "expected {}", renamed.display());
    });
}

#[test]
fn register_rename_sanitizes_spaces_in_folder_name() {
    with_fresh_install(|install| {
        let target = install.join("Old Project With Spaces");
        fs::create_dir_all(&target).unwrap();

        let mut args = register_args(&target);
        args.rename = true;
        args.use_today = true;
        register_run(args).unwrap();

        let parent = target.parent().unwrap();
        let renamed = fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .find(|n| n.contains("Old_Project_With_Spaces"));
        assert!(
            renamed.is_some(),
            "expected sanitized folder name with underscores"
        );
    });
}

#[test]
fn register_apply_requires_template() {
    with_fresh_install(|install| {
        let target = install.join("no-template");
        fs::create_dir_all(&target).unwrap();

        let mut args = register_args(&target);
        args.apply_structure = true;
        let err = register_run(args).expect_err("should bail");
        assert!(
            err.to_string().contains("--apply requires --template"),
            "got: {err:#}"
        );
    });
}

#[test]
fn register_rejects_nested_targets_outside_a_configured_base_child() {
    with_fresh_install(|install| {
        let base = install.join("projects");
        let target = base.join("group/project");
        fs::create_dir_all(&target).unwrap();
        Config {
            base_dir: base.display().to_string(),
            ..Config::default()
        }
        .save()
        .unwrap();

        let err = register_core(RegisterOptions {
            path: target.clone(),
            template_slug: None,
            vars: HashMap::new(),
            apply_structure: false,
            rename: false,
            use_today: false,
            created_override: None,
            on_pinfo_conflict: PinfoConflict::Abort,
        })
        .expect_err("nested target must be rejected");
        assert!(err.to_string().contains("direct child"), "got: {err:#}");
        assert!(!target.join("PROJECT_INFO.md").exists());
    });
}

#[test]
fn register_rejects_a_duplicate_recovered_id() {
    with_fresh_install(|install| {
        let first = install.join("Alpha_ID0005");
        let duplicate = install.join("Beta_ID0005");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&duplicate).unwrap();
        register_run(register_args(&first)).unwrap();

        let err = register_run(register_args(&duplicate)).expect_err("duplicate ID must fail");
        assert!(err.to_string().contains("already used"), "got: {err:#}");
        assert!(!duplicate.join("PROJECT_INFO.md").exists());
    });
}

#[test]
fn register_skip_is_an_immediate_no_op() {
    with_fresh_install(|install| {
        let target = install.join("already-registered");
        fs::create_dir_all(&target).unwrap();
        register_run(register_args(&target)).unwrap();
        let pinfo = target.join("PROJECT_INFO.md");
        let before = fs::read(&pinfo).unwrap();
        let counter_before = Counters::load_base(install);

        let outcome = fastf::core::operations::register(fastf::core::operations::RegisterOptions {
            path: target.clone(),
            template_slug: None,
            vars: HashMap::new(),
            apply_structure: false,
            rename: true,
            use_today: false,
            created_override: None,
            on_pinfo_conflict: fastf::core::operations::PinfoConflict::Skip,
        })
        .unwrap();
        assert!(!outcome.pinfo_written);
        assert!(outcome.renamed_to.is_none());
        assert!(outcome.rename_error.is_none());
        assert_eq!(fs::read(pinfo).unwrap(), before);
        assert_eq!(Counters::load_base(install), counter_before);
        assert!(target.is_dir());
    });
}

#[test]
fn register_reports_rename_failure_after_committing_registration() {
    with_fresh_install(|install| {
        write_template(install, "music", &minimal_template_yaml("music"));
        let target = install.join("legacy");
        let collision = install.join("T001_Hello");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&collision).unwrap();
        let outcome = fastf::core::operations::register(fastf::core::operations::RegisterOptions {
            path: target.clone(),
            template_slug: Some("music".to_string()),
            vars: HashMap::from([("name".to_string(), "hello".to_string())]),
            apply_structure: false,
            rename: true,
            use_today: false,
            created_override: None,
            on_pinfo_conflict: fastf::core::operations::PinfoConflict::Abort,
        })
        .unwrap();
        assert!(outcome.pinfo_written);
        assert!(outcome.rename_error.is_some());
        assert!(outcome.renamed_to.is_none());
        assert!(target.join("PROJECT_INFO.md").is_file());
        assert!(collision.is_dir());
    });
}

#[test]
fn register_reports_incomplete_apply_after_committing_registration() {
    with_fresh_install(|install| {
        let yaml = r#"name: Apply failure
slug: blocked
naming_pattern: "{id}_{name}"
id:
  prefix: B
  digits: 3
variables:
  - slug: name
    label: Name
    type: text
    required: true
structure:
  - name: blocked
    children:
      - name: child
"#;
        write_template(install, "blocked", yaml);
        let target = install.join("legacy");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("blocked"), b"occupied by a file").unwrap();

        let outcome = fastf::core::operations::register(fastf::core::operations::RegisterOptions {
            path: target.clone(),
            template_slug: Some("blocked".to_string()),
            vars: HashMap::from([("name".to_string(), "legacy".to_string())]),
            apply_structure: true,
            rename: false,
            use_today: false,
            created_override: None,
            on_pinfo_conflict: fastf::core::operations::PinfoConflict::Abort,
        })
        .unwrap();
        assert!(outcome.pinfo_written);
        assert!(!outcome.applied);
        assert!(outcome.apply_error.is_some(), "{outcome:?}");
        assert!(target.join("PROJECT_INFO.md").is_file());
        assert_eq!(
            fs::read(target.join("blocked")).unwrap(),
            b"occupied by a file"
        );
    });
}

// ---------------------------------------------------------------------------
// v0.10: move projects between bases
// ---------------------------------------------------------------------------

#[test]
fn move_project_between_bases_full_round_trip() {
    with_fresh_install(|install| {
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
// v1.0: data-dir resolution (portable mode + user config dir fallback)
// ---------------------------------------------------------------------------

/// Serialize + point the user-config-dir fallback at a tempdir, with
/// `FASTF_INSTALL_DIR` unset — simulating a binary installed to a read-only
/// system path (e.g. /usr/bin via a package manager). The test binary lives in
/// `target/debug/deps/` with no `config.toml`/`templates/` beside it, so
/// portable mode cannot trigger and resolution must land in the user dir.
fn with_user_dir_env<R>(body: impl FnOnce(&Path) -> R) -> R {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    #[cfg(not(windows))]
    let var = "XDG_CONFIG_HOME";
    #[cfg(windows)]
    let var = "APPDATA";
    let saved = std::env::var(var).ok();
    // Safe: SERIAL guarantees no other test thread races on these env vars.
    unsafe {
        std::env::remove_var("FASTF_INSTALL_DIR");
        std::env::set_var(var, tmp.path());
    }
    let result = body(tmp.path());
    unsafe {
        match saved {
            Some(v) => std::env::set_var(var, v),
            None => std::env::remove_var(var),
        }
    }
    drop(guard);
    result
}

#[test]
fn data_dir_falls_back_to_user_config_dir() {
    with_user_dir_env(|tmp| {
        let (dir, mode) = fastf::util::paths::try_install_dir().expect("must resolve");
        assert_eq!(dir, tmp.join("fastf"));
        assert_eq!(mode, fastf::util::paths::DirMode::UserDir);
    });
}

#[test]
fn env_override_beats_user_config_dir() {
    with_user_dir_env(|_tmp| {
        let other = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("FASTF_INSTALL_DIR", other.path());
        }
        let (dir, mode) = fastf::util::paths::try_install_dir().expect("must resolve");
        assert_eq!(dir, other.path());
        assert_eq!(mode, fastf::util::paths::DirMode::EnvOverride);
        unsafe {
            std::env::remove_var("FASTF_INSTALL_DIR");
        }
    });
}

#[test]
fn bootstrap_lands_in_user_dir_for_system_install() {
    with_user_dir_env(|tmp| {
        fastf::bootstrap::ensure_bootstrapped().expect("bootstrap must succeed");
        let data = tmp.join("fastf");
        assert!(data.join("config.toml").is_file(), "config.toml written");
        for slug in ["general", "client-project"] {
            assert!(
                data.join("templates")
                    .join(slug)
                    .join("template.yaml")
                    .is_file(),
                "bundled template {slug} written"
            );
        }
        // Idempotent on a second run.
        fastf::bootstrap::ensure_bootstrapped().expect("second bootstrap is a no-op");
    });
}

#[test]
fn mangen_writes_man_pages() {
    // Drives the real binary (mangen lives in main.rs, not the lib). The env
    // override is explicit so the child never touches real user data — though
    // mangen also skips bootstrap entirely by design.
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastf"))
        .arg("mangen")
        .arg(tmp.path())
        .env("FASTF_INSTALL_DIR", tmp.path())
        .output()
        .expect("spawn fastf mangen");
    assert!(out.status.success(), "mangen failed: {out:?}");
    let main_page = tmp.path().join("fastf.1");
    assert!(main_page.is_file(), "fastf.1 must be generated");
    assert!(fs::metadata(&main_page).unwrap().len() > 0);
    // Bootstrap must NOT have run (no config.toml written next to the pages).
    assert!(!tmp.path().join("config.toml").exists());
}

#[test]
fn init_base_dir_shared_onboarding_core() {
    with_fresh_install(|install| {
        use fastf::core::config;

        // Suggestion is <home>/Projects (home is sandboxed to `install`).
        let suggested = config::suggested_base_dir().unwrap();
        assert_eq!(suggested, install.join("Projects"));

        // `~` expands against home; the folder is created and persisted.
        let resolved = config::init_base_dir("~/Client Work").unwrap();
        assert!(resolved.is_dir());
        assert_eq!(
            Config::load().unwrap().base_dir,
            resolved.display().to_string()
        );

        // Relative and empty paths are rejected.
        assert!(config::init_base_dir("relative/path").is_err());
        assert!(config::init_base_dir("   ").is_err());
    });
}

// ---------------------------------------------------------------------------
// Unknown YAML keys — fastf must not destroy what it does not own
// ---------------------------------------------------------------------------

/// Frontmatter keys fastf knows nothing about must survive every mutation that
/// rewrites the file, with their values *and their positions* intact.
///
/// The realistic trigger is two fastf versions over one library: a newer build
/// writes a key an older build has never heard of, and the older build then runs
/// `tag add` on Windows. Before this test, `write_frontmatter` parsed into
/// `Metadata` and re-serialised, so every such key was silently deleted.
///
/// The unquoted `year: 2026` is not decoration. It is the value shape that a
/// `#[serde(flatten)]` catch-all would have started rejecting, which would have
/// made the project invisible to discovery — the exact failure this phase closes.
#[test]
fn unknown_frontmatter_keys_survive_every_mutation() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();
        cfg.save().unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "keys".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        // Plant three things fastf has no field for: a scalar, a nested map, and
        // an unquoted number. Insert them *between* known keys so a merge that
        // appends rather than preserving position is visible.
        let pinfo = project_info::pinfo_path(&plan.root_path);
        let original = fs::read_to_string(&pinfo).unwrap();
        let (frontmatter, body) = project_info::split_frontmatter_body(&original).unwrap();
        let patched: String = frontmatter
            .lines()
            .flat_map(|line| {
                if line.starts_with("template:") {
                    vec![
                        "obsidian_folder: Clients/Acme".to_string(),
                        line.to_string(),
                        "year: 2026".to_string(),
                        "sync:".to_string(),
                        "  provider: dropbox".to_string(),
                        "  last: never".to_string(),
                    ]
                } else {
                    vec![line.to_string()]
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&pinfo, format!("---\n{patched}\n---\n{body}")).unwrap();

        let expect_intact = |stage: &str| {
            let content = fs::read_to_string(&pinfo).unwrap();
            let (fm, _) = project_info::split_frontmatter_body(&content)
                .unwrap_or_else(|| panic!("[{stage}] frontmatter must still parse:\n{content}"));
            let lines: Vec<&str> = fm.lines().collect();
            for key in [
                "obsidian_folder: Clients/Acme",
                "year: 2026",
                "sync:",
                "  provider: dropbox",
                "  last: never",
            ] {
                assert!(lines.contains(&key), "[{stage}] lost `{key}`:\n{fm}");
            }
            // Position, not just presence: the unknown scalar stays immediately
            // before the `template:` key it was written next to.
            let unknown = lines
                .iter()
                .position(|l| l.starts_with("obsidian_folder:"))
                .unwrap();
            let template = lines
                .iter()
                .position(|l| l.starts_with("template:"))
                .unwrap();
            assert_eq!(
                unknown + 1,
                template,
                "[{stage}] unknown key moved out of position:\n{fm}"
            );
            // And the project is still readable — an unquoted number in a
            // `String` field must not make it vanish from discovery.
            let meta = project_info::read_metadata(&plan.root_path)
                .unwrap_or_else(|e| panic!("[{stage}] metadata unreadable: {e:#}"))
                .unwrap_or_else(|| panic!("[{stage}] metadata has no frontmatter"));
            assert_eq!(meta.id, plan.id_str);
        };

        expect_intact("planted");

        let project = library::discover(&cfg)
            .into_iter()
            .find(|p| p.path == plan.root_path.canonicalize().unwrap())
            .expect("project must be discoverable");

        fastf::core::operations::add_tags(&project, &["urgent".to_string()]).unwrap();
        expect_intact("after tag add");

        fastf::core::operations::remove_tags(&project, &["urgent".to_string()]).unwrap();
        expect_intact("after tag remove");

        let renamed = fastf::core::operations::rename(&project, "renamed_by_test").unwrap();
        let pinfo = project_info::pinfo_path(&renamed.path);
        let content = fs::read_to_string(&pinfo).unwrap();
        for key in ["obsidian_folder: Clients/Acme", "year: 2026", "sync:"] {
            assert!(
                content.contains(key),
                "[after rename] lost `{key}`:\n{content}"
            );
        }
        assert!(
            content.contains("folder: renamed_by_test"),
            "[after rename] the known key must still be updated:\n{content}"
        );
    });
}

/// A no-op frontmatter mutation must leave the frontmatter bytes untouched.
///
/// The body has had this guarantee since v0.4; the frontmatter never did, which
/// is what let a rewrite quietly reorder or drop keys with nothing failing.
#[test]
fn write_frontmatter_bytes_preserved_on_no_op() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));

        let mut cfg = Config::default();
        cfg.base_dir = install.join("projects").display().to_string();
        fs::create_dir_all(&cfg.base_dir).unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "noop".to_string());
        let counters = Counters::load().unwrap();
        let plan = project::plan(&tmpl, &vars, &cfg, &counters).unwrap();
        let mut counters = counters;
        project::create(&plan, &tmpl, &mut counters, &cfg, false).unwrap();

        let pinfo = project_info::pinfo_path(&plan.root_path);
        let before = fs::read(&pinfo).unwrap();
        project_info::write_frontmatter(&pinfo, |_| {}).unwrap();
        let after = fs::read(&pinfo).unwrap();

        assert_eq!(
            String::from_utf8_lossy(&before),
            String::from_utf8_lossy(&after),
            "a no-op mutation must not rewrite a single byte"
        );
    });
}

/// A template key fastf does not own survives an editor save; a legacy flat
/// `files:` block still does not.
///
/// `template.yaml` is user-owned and rewritten wholesale by the TUI builder, the
/// browser editor, and `template from-folder --force`. The `files:` half of this
/// is the reason preservation cannot be blanket: since v0.8 the `files/`
/// directory is the spec, and a flat `files:` block is a pre-v0.8 leftover that
/// must keep being dropped rather than newly resurrected.
#[test]
fn unknown_template_keys_survive_a_save_but_legacy_files_do_not() {
    with_fresh_install(|install| {
        write_template(install, "test", &minimal_template_yaml("test"));
        let manifest = install.join("templates/test/template.yaml");

        let raw = fs::read_to_string(&manifest).unwrap();
        fs::write(
            &manifest,
            raw.replace(
                "description: fixture",
                "description: fixture\nauthor_email: someone@example.com\nfuture:\n  nested: kept",
            ),
        )
        .unwrap();

        let tmpl = template::find_by_slug("test").unwrap();
        tmpl.save_to_file(&manifest).unwrap();

        let saved = fs::read_to_string(&manifest).unwrap();
        assert!(
            saved.contains("author_email: someone@example.com"),
            "unknown scalar dropped:\n{saved}"
        );
        assert!(
            saved.contains("nested: kept"),
            "unknown nested map dropped:\n{saved}"
        );
        assert!(
            !saved.contains("\nfiles:"),
            "a pre-v0.8 flat files: block must stay dropped:\n{saved}"
        );
        // Still a valid template afterwards.
        template::find_by_slug("test").unwrap();
    });
}
