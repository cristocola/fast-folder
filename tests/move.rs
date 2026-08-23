//! Moving projects between bases.
//!
//! Split out of the single 2700-line `integration.rs`, whose 67 tests all
//! queued behind one mutex in one binary.

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
// v0.10: move projects between bases
// ---------------------------------------------------------------------------

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
