//! Windows **live** filesystem tests: a real local NTFS base and a real SMB
//! share, driven as real processes.
//!
//! # Why this suite exists, and why it is separate
//!
//! `windows_semantics.rs` covers what a single temporary directory can show —
//! reserved names, trailing dots, a sharing violation, a junction. What it
//! cannot show is anything that needs **two different filesystems**, because
//! a `TempDir` is always on one. The move engine's staged copy is reached
//! only by a genuine `ERROR_NOT_SAME_DEVICE`, so today it is exercised by a
//! synthetic error (`library/tests.rs`, `only_the_cross_device_error_licenses_
//! copy_fallback`) or by a failpoint, and never end to end. The ID counter is
//! designed around a project drive that two machines mount
//! (`core/counter.rs`), and that has never been driven over a network share
//! at all.
//!
//! Those are the gaps this suite fills, and they are the reason it is a
//! separate target rather than more cases in `windows_semantics.rs`: it needs
//! real infrastructure that no CI runner has.
//!
//! # It never runs unless it is told where to run
//!
//! Both bases come from the environment and there are **no defaults**:
//!
//! | Variable | What it must be |
//! |---|---|
//! | `FASTF_WIN_LOCAL_BASE` | a directory on a local NTFS volume |
//! | `FASTF_WIN_SHARE_BASE` | a directory on an SMB share, on a *different* volume from the local base |
//!
//! With either unset, or pointing somewhere unreachable, every test prints
//! what it wanted and returns. So this suite is inert on Linux (the whole
//! file is `cfg(windows)`), inert in CI, and inert on a contributor's machine
//! — it turns red only where it was actually pointed at something.
//!
//! **The paths are deliberately not in this file.** The repository is public
//! and `repo_hygiene.rs` fails the build on anything that describes the
//! maintainer's machine; a real base path and a real share address are
//! exactly that. They live in the runner script beside the sandbox itself.
//!
//! # What it will not touch
//!
//! Every run gets its own data directory under `%TEMP%` and its own uniquely
//! named subfolder under each base, removed afterwards. Nothing here can see
//! an installed fastf's `%APPDATA%\fastf`, and nothing writes to a base root
//! beyond its own subfolder — the machine this was written for has a real
//! fastf installed and in use.

#![cfg(windows)]
#![allow(clippy::field_reassign_with_default)]
// This suite sets and clears the Windows read-only attribute deliberately —
// it is the subject of one of the cases. The lint warns about the Unix
// meaning of the same call, which cannot be reached from a `cfg(windows)`
// file.
#![allow(clippy::permissions_set_readonly_false)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FASTF: &str = env!("CARGO_BIN_EXE_fastf");

const LOCAL_VAR: &str = "FASTF_WIN_LOCAL_BASE";
const SHARE_VAR: &str = "FASTF_WIN_SHARE_BASE";

/// Variables a spawned fastf must never inherit from whoever ran the suite —
/// the same list `tests/common` keeps, for the same reason: the developer's
/// own shell must not answer for fastf.
const NOT_INHERITED: &[&str] = &[
    "EDITOR",
    "FASTF_ASCII",
    "FASTF_FAULT",
    "FASTF_NO_RELAUNCH",
    "FASTF_PROJECT_PATH",
    "FASTF_RELAUNCHED",
    "FASTF_THEME",
    "FASTF_TRACE_FILE",
    "NO_COLOR",
    "TERMINAL",
];

/// One run's scaffolding: a private data directory, and one subfolder under
/// each configured base.
struct Live {
    _tmp: tempfile::TempDir,
    install: PathBuf,
    local: PathBuf,
    share: PathBuf,
}

/// Resolve both bases, or explain what is missing and return `None`.
///
/// A skip is printed rather than silent: a suite that quietly does nothing
/// looks exactly like a suite that passed.
fn live(case: &str) -> Option<Live> {
    let read = |var: &str| -> Option<PathBuf> {
        match std::env::var(var) {
            Ok(value) if !value.trim().is_empty() => Some(PathBuf::from(value.trim())),
            _ => {
                eprintln!("skipping {case}: {var} is not set");
                None
            }
        }
    };
    let local_root = read(LOCAL_VAR)?;
    let share_root = read(SHARE_VAR)?;

    for (root, var) in [(&local_root, LOCAL_VAR), (&share_root, SHARE_VAR)] {
        if !root.is_dir() {
            eprintln!(
                "skipping {case}: {var} is {}, which is not a reachable directory",
                root.display()
            );
            return None;
        }
    }

    let tmp = tempfile::tempdir().expect("a temp dir for the data directory");
    let install = tmp.path().join("data");
    std::fs::create_dir_all(&install).unwrap();

    // A name unique per case, so two cases never share a folder and a failure
    // leaves something identifiable behind.
    let stamp = format!("{}-{}", std::process::id(), case);
    let local = local_root.join(format!("run-{stamp}"));
    let share = share_root.join(format!("run-{stamp}"));
    for dir in [&local, &share] {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("creating {}: {e}", dir.display()));
    }

    let it = Live {
        _tmp: tmp,
        install,
        local,
        share,
    };
    it.ok(&["config", "set", "base-dir", &it.local.display().to_string()]);
    it.ok(&[
        "config",
        "set",
        "bases",
        &format!("{},{}", it.local.display(), it.share.display()),
    ]);
    Some(it)
}

impl Drop for Live {
    fn drop(&mut self) {
        // Leave both bases as they were found. A failure is worth inspecting,
        // but the sandbox roots belong to somebody who is still using them.
        for dir in [&self.local, &self.share] {
            let _ = crate::remove_stubbornly(dir);
        }
    }
}

/// `remove_dir_all`, with the read-only attribute cleared first — a staged
/// copy can leave read-only payload behind, and Windows refuses to delete it.
fn remove_stubbornly(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let _ = Command::new("cmd")
        .args(["/c", "attrib", "-R", "/S", "/D"])
        .arg(dir.join("*"))
        .output();
    std::fs::remove_dir_all(dir)
}

impl Live {
    fn command(&self) -> Command {
        let mut cmd = Command::new(FASTF);
        cmd.env("FASTF_INSTALL_DIR", &self.install)
            .env("USERPROFILE", self._tmp.path())
            .env("HOME", self._tmp.path());
        for name in NOT_INHERITED {
            cmd.env_remove(name);
        }
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("running fastf")
    }

    /// Run and require success, returning stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "`fastf {}` failed:\nstdout: {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Run and require failure, returning stderr.
    fn fails(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            !out.status.success(),
            "`fastf {}` should have failed but did not:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// The single project directory in `base`, whatever it ended up called.
    fn only_project(&self, base: &Path) -> PathBuf {
        let mut found: Vec<PathBuf> = std::fs::read_dir(base)
            .unwrap_or_else(|e| panic!("reading {}: {e}", base.display()))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.join("PROJECT_INFO.md").is_file())
            .collect();
        assert_eq!(
            found.len(),
            1,
            "expected exactly one project in {}, found {found:?}",
            base.display()
        );
        found.remove(0)
    }

    fn projects_in(&self, base: &Path) -> usize {
        std::fs::read_dir(base)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().join("PROJECT_INFO.md").is_file())
                    .count()
            })
            .unwrap_or(0)
    }
}

/// A template whose folder name carries the id, so a test can read the minted
/// number straight off the directory.
fn write_template(install: &Path, slug: &str) {
    let dir = install.join("templates").join(slug);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    std::fs::write(
        dir.join("template.yaml"),
        format!(
            "name: Live\nslug: {slug}\nnaming_pattern: \"{{name}}_{{id}}\"\n\
             id:\n  prefix: L\n  digits: 4\n\
             variables:\n  - slug: name\n    label: Name\n    type: text\n\
             \x20   required: true\n    transform: none\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("files/NOTES.md"), "# {name}\n").unwrap();
}

// ---------------------------------------------------------------------------
// The move engine across two real filesystems
// ---------------------------------------------------------------------------

/// **The staged copy, for real.** A move between two volumes is the only path
/// that copies, verifies and publishes rather than renaming, and it is
/// reached only by a genuine `ERROR_NOT_SAME_DEVICE` from the OS. Until this
/// suite it was exercised by a synthetic `io::Error` and a failpoint, never
/// by two filesystems.
///
/// This is half of the ROADMAP's outstanding Windows validation item.
#[test]
fn a_move_to_the_share_copies_verifies_and_publishes() {
    let Some(live) = live("move-to-share") else {
        return;
    };
    write_template(&live.install, "live");
    live.ok(&["new", "live", "--name=Payload", "--yes"]);

    let source = live.only_project(&live.local);
    std::fs::write(source.join("payload.bin"), vec![7u8; 4096]).unwrap();
    std::fs::create_dir_all(source.join("nested/deeper")).unwrap();
    std::fs::write(source.join("nested/deeper/leaf.txt"), "kept").unwrap();

    let out = live.ok(&["move", "L0001", &live.share.display().to_string(), "--yes"]);
    assert!(
        out.contains("copied") || out.contains("verified"),
        "a cross-volume move must report a copy, not a rename:\n{out}"
    );

    // The destination is complete, and the source is gone.
    let moved = live.only_project(&live.share);
    assert_eq!(
        std::fs::read(moved.join("payload.bin")).unwrap().len(),
        4096,
        "the payload crossed intact"
    );
    assert_eq!(
        std::fs::read_to_string(moved.join("nested/deeper/leaf.txt")).unwrap(),
        "kept",
        "and so did the nested tree"
    );
    assert!(!source.exists(), "the source is removed after publication");
    assert_eq!(live.projects_in(&live.local), 0);

    // **No transaction survives a clean move.** The `.fastf-transactions`
    // root itself is allowed to remain: it is a per-base directory the engine
    // creates once and reuses, `scan_base` skips dot-prefixed names so it can
    // never be mistaken for a project, and removing it would race another
    // move starting in the same base. What must not survive is anything
    // *inside* it.
    let staging = live.share.join(".fastf-transactions");
    let left: Vec<String> = std::fs::read_dir(&staging)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        left.is_empty(),
        "a published move left a transaction behind in {}: {left:?}",
        staging.display()
    );
    let report = live.ok(&["reconcile"]);
    assert!(
        !report.contains("recovered") && !report.contains("rolled back"),
        "a clean move leaves nothing for reconcile:\n{report}"
    );
}

/// The other half of the ROADMAP item: two bases on the **same** volume take
/// the rename path, which copies nothing however large the folder is.
#[test]
fn a_move_within_one_volume_renames_and_copies_nothing() {
    let Some(live) = live("move-same-volume") else {
        return;
    };
    // A second base beside the first, on the same local volume.
    let second = live.local.join("second-base");
    std::fs::create_dir_all(&second).unwrap();
    live.ok(&[
        "config",
        "set",
        "bases",
        &format!("{},{}", live.local.display(), second.display()),
    ]);
    write_template(&live.install, "live");
    live.ok(&["new", "live", "--name=Quick", "--yes"]);

    let out = live.ok(&["move", "L0001", &second.display().to_string(), "--yes"]);
    assert!(
        out.contains("renamed") || out.contains("nothing copied"),
        "a same-volume move must rename rather than copy:\n{out}"
    );
    assert!(second.join("Quick_L0001").is_dir());
}

/// A link inside a project cannot be reproduced by a copy, so the staged path
/// refuses the whole move rather than silently dropping it — the failure mode
/// that once deleted a source whose junctions never reached the destination.
/// The same-volume rename keeps it, because it copies nothing.
#[test]
fn a_junction_is_refused_by_a_cross_volume_move_and_kept_by_a_rename() {
    let Some(live) = live("junction") else {
        return;
    };
    write_template(&live.install, "live");
    live.ok(&["new", "live", "--name=Linked", "--yes"]);
    let source = live.only_project(&live.local);

    let target = live._tmp.path().join("junction-target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("shared.txt"), "outside").unwrap();
    let made = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(source.join("linked"))
        .arg(&target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !made {
        eprintln!("skipping: the OS refused to create a junction");
        return;
    }

    let error = live.fails(&["move", "L0001", &live.share.display().to_string(), "--yes"]);
    assert!(
        error.to_lowercase().contains("link") || error.to_lowercase().contains("junction"),
        "the refusal must name what it could not copy:\n{error}"
    );
    assert!(
        source.join("linked").exists(),
        "the source is untouched by a refused move"
    );
    assert_eq!(live.projects_in(&live.share), 0, "nothing was published");
}

// ---------------------------------------------------------------------------
// The counter, over a share two machines can mount
// ---------------------------------------------------------------------------

/// `.fastf-counter.toml` sits with the projects precisely so a drive two
/// machines mount hands out the same next id from either. The share is that
/// drive; this is the design's own scenario rather than a simulation of it.
#[test]
fn the_counter_converges_across_the_share() {
    let Some(live) = live("counter") else {
        return;
    };
    write_template(&live.install, "live");

    live.ok(&["new", "live", "--name=First", "--yes"]);
    live.ok(&[
        "new",
        "live",
        "--name=Second",
        "--yes",
        &format!("--base-dir={}", live.share.display()),
    ]);

    // Every mounted base carries the same high-water mark.
    for base in [&live.local, &live.share] {
        let counter = base.join(".fastf-counter.toml");
        assert!(
            counter.is_file(),
            "each base keeps its own counter: {}",
            counter.display()
        );
        let text = std::fs::read_to_string(&counter).unwrap();
        assert!(
            text.contains('2'),
            "the counter propagated to {}:\n{text}",
            base.display()
        );
    }

    // And the two ids are distinct — the whole point of propagating.
    let shown = live.ok(&["recent", "--plain"]);
    assert!(shown.contains("L0001"), "{shown}");
    assert!(shown.contains("L0002"), "{shown}");
}

// ---------------------------------------------------------------------------
// NTFS semantics the mutating verbs meet
// ---------------------------------------------------------------------------

/// NTFS is case-insensitive, so a case-only rename cannot go through the
/// ordinary "does the target already exist" path — it always does. The engine
/// parks the folder at a dot-prefixed staging name and commits from there.
///
/// ASCII is covered in `windows_semantics.rs`; this is the **non-ASCII** case,
/// which `eq_ignore_ascii_case` cannot see.
#[test]
fn a_case_only_rename_round_trips_for_non_ascii_names() {
    let Some(live) = live("case-rename") else {
        return;
    };
    write_template(&live.install, "live");
    live.ok(&["new", "live", "--name=проект", "--yes"]);

    let out = live.run(&["rename", "L0001", "ПРОЕКТ_L0001", "--yes"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "a non-ASCII case-only rename must work as the ASCII one does:\n{stderr}"
    );

    let renamed = live.only_project(&live.local);
    let name = renamed.file_name().unwrap().to_string_lossy().to_string();
    assert_eq!(
        name, "ПРОЕКТ_L0001",
        "the new casing reached the filesystem"
    );
    assert_eq!(
        live.projects_in(&live.local),
        1,
        "one project, not two, and no staging folder left behind"
    );
    assert!(
        !live.local.join(".ПРОЕКТ_L0001.fastf-case").exists(),
        "the case-staging folder is committed, not stranded"
    );
}

/// Every metadata mutation publishes through `atomic::write`, which finishes
/// with a rename over the existing file. Windows refuses that when the target
/// carries the read-only attribute, where Unix does not care — so a project
/// restored from a backup or copied off a share could not be tagged or noted.
#[test]
fn a_read_only_project_info_does_not_block_a_tag_or_a_note() {
    let Some(live) = live("read-only") else {
        return;
    };
    write_template(&live.install, "live");
    live.ok(&["new", "live", "--name=Frozen", "--yes"]);
    let pinfo = live.only_project(&live.local).join("PROJECT_INFO.md");

    let mut perms = std::fs::metadata(&pinfo).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&pinfo, perms).unwrap();

    let tag = live.run(&["tag", "add", "L0001", "archived"]);
    let note = live.run(&["note", "add", "L0001", "still reachable"]);

    // Put it back before asserting, so a failure cannot leave the sandbox
    // holding an undeletable file.
    let mut perms = std::fs::metadata(&pinfo).unwrap().permissions();
    perms.set_readonly(false);
    std::fs::set_permissions(&pinfo, perms).unwrap();

    assert!(
        tag.status.success(),
        "a read-only PROJECT_INFO.md must not stop a tag:\n{}",
        String::from_utf8_lossy(&tag.stderr)
    );
    assert!(
        note.status.success(),
        "nor a note:\n{}",
        String::from_utf8_lossy(&note.stderr)
    );
    let text = std::fs::read_to_string(&pinfo).unwrap();
    assert!(text.contains("archived"), "the tag landed:\n{text}");
    assert!(text.contains("still reachable"), "the note landed:\n{text}");
}

/// A create whose rendered name differs from an existing folder only in case
/// cannot claim it on NTFS, where `create_dir` sees the existing one. The
/// collision suffix is what keeps two projects from merging into one folder.
#[test]
fn a_case_insensitive_collision_takes_the_numbered_suffix() {
    let Some(live) = live("case-collision") else {
        return;
    };
    write_template(&live.install, "live");
    live.ok(&["new", "live", "--name=Alpha", "--yes"]);

    // Rename the folder to a different casing behind fastf's back, then make
    // a project whose rendered name collides with it case-insensitively.
    let first = live.only_project(&live.local);
    let shouty = live.local.join("ALPHA_L0001");
    std::fs::rename(&first, &shouty).unwrap();

    live.ok(&["reindex"]);
    live.ok(&["new", "live", "--name=Alpha", "--yes"]);

    assert_eq!(
        live.projects_in(&live.local),
        2,
        "two projects, in two folders — never merged into one"
    );
    assert!(
        live.local.join("Alpha_L0002").is_dir(),
        "the second create claimed its own folder"
    );
}

// ---------------------------------------------------------------------------
// The share as a base
// ---------------------------------------------------------------------------

/// Discovery, metadata reads and the index cache over SMB. The cache is
/// base-relative by design so it travels with the projects; a share is the
/// case that design exists for.
#[test]
fn a_share_base_discovers_indexes_and_mutates() {
    let Some(live) = live("share-base") else {
        return;
    };
    write_template(&live.install, "live");
    live.ok(&[
        "new",
        "live",
        "--name=Remote",
        "--yes",
        &format!("--base-dir={}", live.share.display()),
    ]);

    let listed = live.ok(&["recent", "--plain"]);
    assert!(
        listed.contains("L0001"),
        "the share's project lists:\n{listed}"
    );

    let index = live.share.join(".fastf-index.json");
    assert!(index.is_file(), "the share carries its own index");

    live.ok(&["tag", "add", "L0001", "remote"]);
    live.ok(&["note", "add", "L0001", "written over smb"]);
    let pinfo = live.only_project(&live.share).join("PROJECT_INFO.md");
    let text = std::fs::read_to_string(&pinfo).unwrap();
    assert!(
        text.contains("remote"),
        "the tag survived the share:\n{text}"
    );
    assert!(text.contains("written over smb"), "and the note:\n{text}");

    // The atomic publish left nothing half-written beside it.
    let strays: Vec<_> = std::fs::read_dir(live.only_project(&live.share))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".tmp") || n.ends_with('~'))
        .collect();
    assert!(
        strays.is_empty(),
        "no temporary files left over SMB: {strays:?}"
    );
}

/// A project that resolves but whose folder has gone is refused rather than
/// acted on — over a share, where a folder can vanish because somebody else
/// removed it.
#[test]
fn a_vanished_project_on_the_share_is_refused_by_name() {
    let Some(live) = live("vanished") else {
        return;
    };
    write_template(&live.install, "live");
    live.ok(&[
        "new",
        "live",
        "--name=Ghost",
        "--yes",
        &format!("--base-dir={}", live.share.display()),
    ]);
    let project = live.only_project(&live.share);
    live.ok(&["recent", "--plain"]); // warm the index
    std::fs::remove_dir_all(&project).unwrap();

    let error = live.fails(&["path", "L0001"]);
    assert!(
        !error.contains("panicked"),
        "a vanished project degrades, never panics:\n{error}"
    );
    assert!(
        error.contains("L0001") || error.to_lowercase().contains("no project"),
        "the refusal names what it could not find:\n{error}"
    );
}
