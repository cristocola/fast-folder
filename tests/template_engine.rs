//! The template engine: reserved names, the gallery, the `files/` subtree.
//!
//! Split out of the single 2700-line `integration.rs`, whose 67 tests all
//! queued behind one mutex in one binary.

#![allow(clippy::field_reassign_with_default)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use fastf::core::{config::Config, counter::Counters, project, template};

mod common;

use common::env::with_fresh_install;
use common::fixtures::write_template;

/// This binary's own lock. `FASTF_INSTALL_DIR` and `HOME` are process-wide, so
/// every test in a binary shares one — and separate binaries are separate
/// processes, which is what lets these suites run in parallel with each other.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
}

// ---------------------------------------------------------------------------
// PROJECT_INFO.md is a reserved fastf-managed filename.  Templates may declare
// it (e.g. old user-built templates from before the safety net), but the
// entry is silently stripped on load and save so the auto-gen always wins
// at write time.
// ---------------------------------------------------------------------------

#[test]
fn template_load_strips_reserved_project_info_entry() {
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
