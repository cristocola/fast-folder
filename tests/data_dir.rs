//! Data-dir resolution, bootstrap, and unknown-key preservation.
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

/// This binary's own lock. `FASTF_INSTALL_DIR` and `HOME` are process-wide, so
/// every test in a binary shares one — and separate binaries are separate
/// processes, which is what lets these suites run in parallel with each other.
static SERIAL: Mutex<()> = Mutex::new(());

fn sandboxed<R>(body: impl FnOnce(&Path) -> R) -> R {
    with_fresh_install(&SERIAL, body)
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
    sandboxed(|install| {
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
