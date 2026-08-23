//! `fastf register <path>` — onboarding folders that already exist.
//!
//! Split out of the single 2700-line `integration.rs`, whose 67 tests all
//! queued behind one mutex in one binary.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, project_info};

mod common;

use common::env::with_fresh_install;
use common::fixtures::{minimal_template_yaml, write_template};

/// This binary's lock over the process environment — see `common::env`.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
        let target = install.join("a-file");
        fs::write(&target, "im a file").unwrap();
        let err = register_run(register_args(&target)).expect_err("should bail");
        assert!(err.to_string().contains("not a directory"), "got: {err:#}");
    });
}

#[test]
fn register_recovers_id_from_folder_name() {
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
