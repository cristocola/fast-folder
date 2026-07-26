//! Cross-process concurrency.
//!
//! These spawn **real processes**, not threads, and that is the whole point.
//! The browser UI's `WRITE_LOCK` is an in-process `Mutex`: a thread-based test
//! would have passed against it while production stayed broken, because the
//! actual collision is a `fastf new` in a terminal racing the UI — the workflow
//! the docs recommend. Ten concurrent creates reliably minted duplicate IDs.
//!
//! Each test drives the built binary with its own `FASTF_INSTALL_DIR`, so the
//! only thing shared between the processes is the sandbox on disk — exactly the
//! situation on a real machine.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

const FASTF: &str = env!("CARGO_BIN_EXE_fastf");

/// How many processes to race. Enough to lose reliably when unsynchronized —
/// the original bug showed up as 8 distinct IDs out of 10.
const RACERS: usize = 10;

struct Sandbox {
    _tmp: tempfile::TempDir,
    install: PathBuf,
    base: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let install = tmp.path().join("install");
        let base = tmp.path().join("base");
        fs::create_dir_all(install.join("templates")).unwrap();
        fs::create_dir_all(&base).unwrap();
        let sb = Sandbox {
            _tmp: tmp,
            install,
            base,
        };
        sb.write_template("race");
        let out = sb.run(&["config", "set", "base-dir", &sb.base.display().to_string()]);
        assert!(out.status.success(), "config set base-dir failed: {out:?}");
        sb
    }

    fn write_template(&self, slug: &str) {
        let dir = self.install.join("templates").join(slug);
        fs::create_dir_all(dir.join("files")).unwrap();
        fs::write(
            dir.join("template.yaml"),
            format!(
                "name: Race\nslug: {slug}\nnaming_pattern: \"{{id}}_{{name}}\"\n\
                 id:\n  prefix: R\n  digits: 4\n\
                 variables:\n  - slug: name\n    label: Name\n    type: text\n\
                 \x20   required: true\n    transform: none\n"
            ),
        )
        .unwrap();
        fs::write(dir.join("files/README.md"), "# {name}\n").unwrap();
    }

    /// Environment shared by every spawned process: same data dir, and HOME
    /// redirected so an unconfigured base can never reach the real home.
    fn command(&self) -> Command {
        let mut cmd = Command::new(FASTF);
        cmd.env("FASTF_INSTALL_DIR", &self.install).env(
            if cfg!(windows) { "USERPROFILE" } else { "HOME" },
            self._tmp.path(),
        );
        cmd
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().expect("running fastf")
    }

    fn spawn(&self, args: &[&str]) -> Child {
        self.command()
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawning fastf")
    }

    /// Every project's id, read straight from the metadata on disk.
    fn ids_on_disk(&self) -> Vec<String> {
        project_dirs(&self.base)
            .iter()
            .filter_map(|dir| {
                let text = fs::read_to_string(dir.join("PROJECT_INFO.md")).ok()?;
                text.lines()
                    .find_map(|l| l.strip_prefix("id:"))
                    .map(|v| v.trim().to_string())
            })
            .collect()
    }
}

fn project_dirs(base: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("PROJECT_INFO.md").is_file())
        .collect();
    out.sort();
    out
}

/// The headline regression: ten simultaneous creates must mint ten distinct IDs.
///
/// Before the cross-process lock this produced eight — `ID0012` and `ID0015`
/// each minted twice — silently breaking the tool's central promise that IDs are
/// unique across every project.
#[test]
fn concurrent_creates_mint_distinct_ids() {
    let sb = Sandbox::new();

    let children: Vec<Child> = (0..RACERS)
        .map(|i| {
            sb.spawn(&[
                "new",
                "race",
                &format!("--name=R{i}"),
                "--yes",
                "--no-preview",
            ])
        })
        .collect();
    for mut child in children {
        let _ = child.wait();
    }

    let ids = sb.ids_on_disk();
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        RACERS,
        "expected {RACERS} projects, got {}: {ids:?}",
        ids.len()
    );
    assert_eq!(
        unique.len(),
        ids.len(),
        "duplicate IDs minted under concurrency: {ids:?}"
    );

    // The counter must also agree with reality, or the next create collides.
    let counters = fs::read_to_string(sb.install.join("counters.toml")).unwrap();
    assert!(
        counters.contains(&format!("global = {RACERS}")),
        "counter out of step with {RACERS} projects: {counters}"
    );
}

/// Racing creates that resolve to the *same* folder name: exactly one may win.
///
/// The old `exists()`-then-`create_dir_all()` pair let two racers both pass the
/// check and write into one folder, the second overwriting the first's files and
/// metadata. `create_dir` now fails atomically, so the filesystem arbitrates.
#[test]
fn concurrent_same_name_creates_produce_no_merged_folder() {
    let sb = Sandbox::new();

    let children: Vec<Child> = (0..RACERS)
        .map(|_| sb.spawn(&["new", "race", "--name=Twin", "--yes", "--no-preview"]))
        .collect();
    let mut succeeded = 0;
    for mut child in children {
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            succeeded += 1;
        }
    }

    let dirs = project_dirs(&sb.base);
    assert_eq!(
        dirs.len(),
        succeeded,
        "every success must correspond to exactly one folder \
         ({succeeded} succeeded, {} folders)",
        dirs.len()
    );

    // No folder may have been written into twice: each carries exactly one id,
    // and all ids across the base are distinct.
    let ids = sb.ids_on_disk();
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "a folder was clobbered: {ids:?}");
}

/// Concurrent `config set` of different keys must not lose an update.
///
/// `Config::save` was a read-modify-write with a bare `fs::write`; two of these
/// at once dropped one of the changes and a crash mid-write truncated the file.
#[test]
fn concurrent_config_writes_do_not_lose_updates() {
    let sb = Sandbox::new();

    let writes: Vec<(&str, &str)> = vec![
        ("date-format", "%Y%m%d"),
        ("recent-default-limit", "42"),
        ("preview-lines", "3"),
        ("show-banner", "false"),
        ("confirm-create", "false"),
    ];
    let children: Vec<Child> = writes
        .iter()
        .map(|(key, value)| sb.spawn(&["config", "set", key, value]))
        .collect();
    for mut child in children {
        let _ = child.wait();
    }

    let config = fs::read_to_string(sb.install.join("config.toml")).unwrap();
    for (key, value) in &writes {
        let field = key.replace('-', "_");
        let expected_present = config
            .lines()
            .any(|l| l.starts_with(&field) && l.contains(value));
        assert!(
            expected_present,
            "lost the update to {key} = {value}\n--- config.toml ---\n{config}"
        );
    }
    // base-dir set during setup must have survived them all.
    assert!(config.contains("base_dir"), "base_dir was lost:\n{config}");
}

/// A create racing a `register` — both mint IDs from the same counter, through
/// different code paths.
#[test]
fn concurrent_create_and_register_do_not_collide() {
    let sb = Sandbox::new();

    // Folders for register to adopt.
    let adoptees: Vec<PathBuf> = (0..4)
        .map(|i| {
            let dir = sb.base.join(format!("existing_{i}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("notes.txt"), "pre-existing work").unwrap();
            dir
        })
        .collect();

    let mut children: Vec<Child> = (0..4)
        .map(|i| {
            sb.spawn(&[
                "new",
                "race",
                &format!("--name=N{i}"),
                "--yes",
                "--no-preview",
            ])
        })
        .collect();
    for dir in &adoptees {
        children.push(sb.spawn(&["register", &dir.display().to_string(), "--yes"]));
    }
    for mut child in children {
        let _ = child.wait();
    }

    let ids = sb.ids_on_disk();
    let unique: HashSet<&String> = ids.iter().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "create and register minted colliding IDs: {ids:?}"
    );
}
