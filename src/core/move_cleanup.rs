//! How a source leaves the library, and how what it leaves behind is removed.
//!
//! **A source is never deleted where it stands.** Deleting a tree is not one
//! operation: it walks, and anything that stops the walk part of the way — a
//! folder it may not write, a file a program holds open, a network drop, an
//! entry the filesystem lists but will not let it examine — leaves a tree that
//! still holds its `PROJECT_INFO.md`, so the library lists a husk as the
//! project. That is how 3.11 left 13 of 1473 files of a moved project behind
//! on an sshfs mount and then called the source untouched.
//!
//! So a source is **retired**: renamed, in one step, to
//! `<source base>/.fastf-moved-<operation>` — same folder, same filesystem,
//! and dot-prefixed, so discovery never lists it. If the rename fails, the
//! source is whole where it was. Only then is the retired folder removed, and
//! if *that* stops part of the way, what is left is a hidden copy of what the
//! destination already holds, which the next `fastf reconcile` finishes.
//!
//! **One rule decides what may be removed**, here and in recovery
//! ([`check_removable`]): every entry is one the move recorded, unchanged,
//! and the moved copy still holds an entry of the same kind at its path that
//! is either what was published or newer. Nothing is removed that exists
//! nowhere else.

use anyhow::{Result, bail};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::transactions::{
    self, LISTED, ManifestEntry, ManifestKind, Match, MoveManifest, MovePhase, MoveTransaction,
    Walk,
};

/// A deleted project, beside the others in its base, until it is removed.
/// Dot-prefixed, so discovery never lists it.
pub const DELETED_PREFIX: &str = ".fastf-deleted-";

/// What became of a source after publication.
#[derive(Debug)]
pub(crate) enum SourceFate {
    /// Retired and removed. `record_kept` says why the transaction could not
    /// be cleared too, when it could not; the next reconcile clears it.
    Removed { record_kept: Option<String> },
    /// Out of the library; its retired copy at `path` is not removed yet.
    /// `redundant` says everything in it is also in the moved copy — removing
    /// it stopped part of the way — rather than that it was kept whole because
    /// it holds something the move did not record.
    Leftover {
        path: PathBuf,
        reason: String,
        redundant: bool,
    },
    /// Still at its path, whole.
    KeptWhole { reason: String },
    /// Whether it left could not be told.
    Unknown { reason: String },
}

/// The facts a cleanup works from.
pub(crate) struct Cleanup<'a> {
    pub manifest: &'a MoveManifest,
    /// The destination as published; `None` for a transaction 3.11 wrote.
    pub published: Option<&'a MoveManifest>,
    pub source: &'a Path,
    pub final_path: &'a Path,
    pub project_id: &'a str,
    /// A 3.11 transaction's source may already be partly removed: 3.11
    /// deleted in place. Its residue may be retired if everything left is as
    /// recorded. A version-3 source is never partly removed, so it must be
    /// whole.
    pub residue_allowed: bool,
}

/// The project at `path` carries `expected` as its id.
pub(crate) fn confirm_identity(path: &Path, expected: &str, label: &str) -> Result<()> {
    crate::util::paths::require_real_directory(path, label)?;
    let pinfo = crate::core::project_info::pinfo_path(path);
    crate::util::paths::require_real_file(&pinfo, "PROJECT_INFO.md")?;
    let metadata = crate::core::project_info::read_metadata(path)?
        .ok_or_else(|| anyhow::anyhow!("{label} project has no readable identity"))?;
    if metadata.id != expected {
        bail!(
            "{label} project identity mismatch (expected {expected}, found {})",
            metadata.id
        );
    }
    Ok(())
}

/// Whether `tree` may be removed: why not, or the walk that says it may.
///
/// `residue_allowed` accepts entries missing from `tree` (a 3.11 half-delete,
/// a retired folder removed part of the way); otherwise it must be whole.
pub(crate) fn check_removable(
    tree: &Path,
    residue_allowed: bool,
    cleanup: &Cleanup,
) -> std::result::Result<Walk, String> {
    let walk = Walk::of(tree, "folder").map_err(|error| format!("{error:#}"))?;
    let diff = cleanup.manifest.compare(&walk, Match::Whole);
    let fits = if residue_allowed {
        diff.is_residue()
    } else {
        diff.is_clean()
    };
    if !fits {
        let held = if diff.missing.is_empty() {
            String::new()
        } else {
            format!(
                "it holds {} of the {} entries the move recorded, and ",
                diff.present(),
                diff.recorded
            )
        };
        return Err(format!(
            "{held}it is not what was moved: {}",
            diff.summary(LISTED)
        ));
    }
    destination_covers(&walk.entries, cleanup)?;
    Ok(walk)
}

/// The moved copy still holds, for every one of `entries`, an entry of the
/// same kind, which is what was published or newer — so removing `entries`
/// takes nothing that exists nowhere else.
///
/// "Or newer" is what lets the user work on the moved copy while a cleanup
/// waits — and lets the move's own bookkeeping rewrite `PROJECT_INFO.md`;
/// "not older" is what refuses a moved copy restored out of a backup taken
/// before the move. A transaction without a published record (3.11) measures
/// against the original's own time instead: a copy is always written after
/// what it copied. That compares two filesystems' clocks, so it errs, when it
/// errs, towards keeping.
fn destination_covers(
    entries: &[ManifestEntry],
    cleanup: &Cleanup,
) -> std::result::Result<(), String> {
    confirm_identity(cleanup.final_path, cleanup.project_id, "moved")
        .map_err(|error| format!("the moved copy is not this project any more: {error:#}"))?;
    let moved = Walk::of(cleanup.final_path, "moved copy").map_err(|error| format!("{error:#}"))?;
    let at: HashMap<&Path, &ManifestEntry> = moved
        .entries
        .iter()
        .map(|entry| (entry.path.as_path(), entry))
        .collect();
    let mut gaps = Vec::new();
    for entry in entries {
        let path = entry.path.display();
        let Some(now) = at.get(entry.path.as_path()) else {
            gaps.push(format!("{path}: not in the moved copy"));
            continue;
        };
        if now.kind != entry.kind {
            gaps.push(format!(
                "{path}: not the same kind of entry in the moved copy"
            ));
            continue;
        }
        if entry.kind == ManifestKind::Directory {
            continue;
        }
        match cleanup.published {
            Some(published) => match published.entry(&entry.path) {
                Some(then) if now.source_modified.nanos() >= then.source_modified.nanos() => {}
                Some(_) => gaps.push(format!(
                    "{path}: older in the moved copy than when it was moved \
                     (restored from a backup?)"
                )),
                None => gaps.push(format!("{path}: not in the copy that was published")),
            },
            None if now.source_modified.nanos() < entry.source_modified.nanos() => {
                gaps.push(format!(
                    "{path}: older in the moved copy than here (restored from a backup?)"
                ));
            }
            None => {}
        }
    }
    if gaps.is_empty() {
        return Ok(());
    }
    let count = gaps.len();
    let mut message = format!(
        "the moved copy no longer holds everything this folder does ({count} {}):",
        if count == 1 { "entry" } else { "entries" }
    );
    for gap in gaps.iter().take(LISTED) {
        message.push_str("\n  ");
        message.push_str(gap);
    }
    if count > LISTED {
        message.push_str(&format!("\n  and {} more", count - LISTED));
    }
    Err(message)
}

/// Publish-then-retire, from `CleanupPending` with the source at its path:
/// check, retire, record `Retired`, keep the books, remove the retired copy.
///
/// `bookkeeping` runs once the source has left the library, whether or not
/// removing its retired copy then succeeds: from that moment the project is
/// the moved copy.
pub(crate) fn retire_and_remove(
    mut transaction: MoveTransaction,
    cleanup: &Cleanup,
    bookkeeping: impl FnOnce(),
) -> SourceFate {
    let source = cleanup.source;
    let has_identity = crate::core::project_info::pinfo_path(source).is_file();
    if (has_identity || !cleanup.residue_allowed)
        && let Err(error) = confirm_identity(source, cleanup.project_id, "original")
    {
        return SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        };
    }
    if let Err(reason) = check_removable(source, cleanup.residue_allowed, cleanup) {
        return SourceFate::KeptWhole { reason };
    }
    // Rewrites a 3.11 journal as version 3 *before* the rename — see
    // `MoveTransaction::set_phase`.
    if let Err(error) = transaction.set_phase(MovePhase::CleanupPending) {
        return SourceFate::KeptWhole {
            reason: format!("the cleanup could not be recorded ({error:#})"),
        };
    }
    if let Err(error) = crate::util::faults::check("move:before-retire") {
        return SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        };
    }
    let retired = transaction.retired_path();
    // A retire that fails, for a test to reach.
    if let Err(error) = crate::util::faults::check("move:source-cleanup") {
        return SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        };
    }
    match retire(source, &retired) {
        Retire::Done => {}
        Retire::KeptWhole(reason) => return SourceFate::KeptWhole { reason },
        Retire::Unknown(reason) => return SourceFate::Unknown { reason },
    }
    if let Err(error) = crate::util::faults::check("move:after-retire") {
        bookkeeping();
        return SourceFate::Leftover {
            path: retired,
            reason: format!("{error:#}"),
            redundant: true,
        };
    }
    if let Err(error) = transaction.set_phase(MovePhase::Retired) {
        bookkeeping();
        return SourceFate::Leftover {
            path: retired,
            reason: format!("the retirement could not be recorded ({error:#})"),
            redundant: true,
        };
    }
    bookkeeping();
    remove_retired(transaction, cleanup)
}

/// From `Retired`: remove the retired copy if it is still provably redundant,
/// then the transaction. A retired copy that is not is kept whole, for the
/// user to look at — never removed part of the way on purpose.
pub(crate) fn remove_retired(transaction: MoveTransaction, cleanup: &Cleanup) -> SourceFate {
    let retired = transaction.retired_path();
    if entry_exists(&retired) {
        if let Err(reason) = check_removable(&retired, true, cleanup) {
            return SourceFate::Leftover {
                path: retired,
                reason,
                redundant: false,
            };
        }
        if let Removal::Leftover { remaining, reason } =
            remove_tree(&retired, Some(cleanup.manifest), Purpose::Move)
        {
            return SourceFate::Leftover {
                path: retired,
                reason: format!(
                    "{remaining} {} left: {reason}",
                    if remaining == 1 { "entry" } else { "entries" }
                ),
                redundant: true,
            };
        }
        if let Err(error) = crate::util::faults::check("move:after-source-cleanup") {
            return SourceFate::Removed {
                record_kept: Some(format!("{error:#}")),
            };
        }
    }
    match transaction.remove() {
        Ok(()) => SourceFate::Removed { record_kept: None },
        Err(error) => SourceFate::Removed {
            record_kept: Some(format!("{error:#}")),
        },
    }
}

/// How a retire went.
#[derive(Debug)]
pub(crate) enum Retire {
    Done,
    KeptWhole(String),
    Unknown(String),
}

/// Rename `source` to `retired`, in one step.
///
/// A rename that reports an error is checked rather than believed: over a
/// network the server can complete it while the client times out, so both
/// paths are looked at again before anything is said.
pub(crate) fn retire(source: &Path, retired: &Path) -> Retire {
    step_out_of(source);
    if entry_exists(retired) {
        return Retire::Unknown(format!(
            "{} is already there",
            crate::util::paths::display_path(retired)
        ));
    }
    match crate::util::fs_retry::rename_dir(source, retired) {
        Ok(()) => Retire::Done,
        Err(error) => match (entry_exists(source), entry_exists(retired)) {
            (false, true) => Retire::Done,
            (true, false) => {
                Retire::KeptWhole(crate::util::fs_retry::describe_rename_error(&error))
            }
            _ => Retire::Unknown(format!(
                "renaming it aside reported '{error}', and afterwards neither it nor {} \
                 could be found where expected",
                crate::util::paths::display_path(retired)
            )),
        },
    }
}

/// Windows will not rename a folder that is a process's working folder —
/// fastf's own included, which `fastf move .` makes it. Step out first.
fn step_out_of(tree: &Path) {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let inside = match (
        crate::util::paths::canonical(&cwd),
        crate::util::paths::canonical(tree),
    ) {
        (Ok(cwd), Ok(tree)) => cwd.starts_with(tree),
        _ => false,
    };
    if inside && let Some(parent) = tree.parent() {
        let _ = std::env::set_current_dir(parent);
    }
}

/// Push a folder's entries to disk, so a rename in it cannot outlive a power
/// loss that the rename before it did not. Unix only: Windows has no way to
/// open a folder for this.
pub(crate) fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// How removing a tree went.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Removal {
    Removed,
    Leftover { remaining: usize, reason: String },
}

/// What a tree is being removed for, which decides the failpoint it trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    Move,
    Delete,
}

/// Remove `root` and everything in it — a retired source, or a deleted
/// project — **without following a link or crossing onto another
/// filesystem**, and, when `recorded` is given, only entries still exactly as
/// it records them; anything else is kept, and so is every folder above it.
///
/// Not `std::fs::remove_dir_all`: that stops at the first error, and on unix
/// silently steps over an entry it cannot find, which is the pair that left
/// the 3.11 husk. This one carries on past a failure (everything here is
/// already somewhere else, or was asked to go), gives a folder it may not
/// write its owner's permission back, and counts what it had to leave.
pub(crate) fn remove_tree(
    root: &Path,
    recorded: Option<&MoveManifest>,
    purpose: Purpose,
) -> Removal {
    let device = fs::symlink_metadata(root)
        .ok()
        .and_then(|metadata| transactions::device_of(&metadata));
    let mut removing = Removing {
        root,
        recorded,
        device,
        purpose,
        removed_one: false,
        stopped: None,
        notes: Vec::new(),
    };
    removing.children(root, 0);
    if removing.stopped.is_none() {
        match crate::util::fs_retry::remove_dir(root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => removing.note(root, &error.to_string()),
        }
    }
    if !entry_exists(root) {
        return Removal::Removed;
    }
    let remaining = Walk::of(root, "folder")
        .map(|walk| walk.entries.len() + walk.problems.len())
        .unwrap_or(0);
    let mut reason = removing.stopped.take().unwrap_or_default();
    if reason.is_empty() {
        reason = if removing.notes.is_empty() {
            "the folder could not be removed".to_string()
        } else {
            removing.notes.join("; ")
        };
    }
    Removal::Leftover { remaining, reason }
}

struct Removing<'a> {
    root: &'a Path,
    recorded: Option<&'a MoveManifest>,
    device: Option<u64>,
    purpose: Purpose,
    removed_one: bool,
    /// Set when a failpoint stops the removal; everything after is left.
    stopped: Option<String>,
    /// The first few things that were left, and why.
    notes: Vec<String>,
}

impl Removing<'_> {
    fn note(&mut self, path: &Path, why: &str) {
        if self.notes.len() < LISTED {
            let shown = path
                .strip_prefix(self.root)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .unwrap_or(path);
            self.notes.push(format!("{}: {why}", shown.display()));
        }
    }

    /// Remove what is inside `dir`, depth-first. `dir` itself is the caller's.
    fn children(&mut self, dir: &Path, depth: usize) {
        if depth >= crate::util::paths::MAX_WALK_DEPTH {
            self.note(dir, "too deep to walk");
            return;
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                grant_owner(dir);
                match fs::read_dir(dir) {
                    Ok(entries) => entries,
                    Err(error) => {
                        self.note(dir, &error.to_string());
                        return;
                    }
                }
            }
            Err(error) => {
                self.note(dir, &error.to_string());
                return;
            }
        };
        let children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        for path in children {
            if self.stopped.is_some() {
                return;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    self.note(&path, &format!("cannot be examined ({error})"));
                    continue;
                }
            };
            if let Some(manifest) = self.recorded
                && !self.still_recorded(manifest, &path)
            {
                self.note(&path, "not as the move recorded it, so it was kept");
                continue;
            }
            let file_type = metadata.file_type();
            if file_type.is_dir() {
                if self.device.is_some() && transactions::device_of(&metadata) != self.device {
                    self.note(&path, "on another filesystem, so it was kept");
                    continue;
                }
                self.children(&path, depth + 1);
                if self.stopped.is_some() {
                    return;
                }
                self.remove(&path, true);
            } else {
                self.remove(&path, is_folder_link(&metadata));
            }
        }
    }

    fn still_recorded(&self, manifest: &MoveManifest, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(self.root) else {
            return false;
        };
        let Some(recorded) = manifest.entry(relative) else {
            return false;
        };
        matches!(
            transactions::examine(path, relative),
            Ok(Some(found)) if transactions::agrees(recorded, &found)
        )
    }

    /// Remove one entry: a folder (already emptied) or a folder link with
    /// `remove_dir`, anything else with `remove_file`. Never through a link.
    fn remove(&mut self, path: &Path, as_folder: bool) {
        let attempt = || {
            if as_folder {
                crate::util::fs_retry::remove_dir(path)
            } else {
                crate::util::fs_retry::remove_file(path)
            }
        };
        let result = match attempt() {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                if let Some(parent) = path.parent() {
                    grant_owner(parent);
                }
                attempt()
            }
            other => other,
        };
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                self.note(path, &error.to_string());
                return;
            }
        }
        if !self.removed_one {
            self.removed_one = true;
            let tripped = match self.purpose {
                Purpose::Move => crate::util::faults::check("move:mid-gc"),
                Purpose::Delete => Ok(()),
            };
            if let Err(error) = tripped {
                self.stopped = Some(format!("{error:#}"));
            }
        }
    }
}

/// A link that Windows removes as a folder: a directory symlink or junction.
fn is_folder_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        metadata.file_type().is_symlink()
            && metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

/// Give a folder that is being removed its owner's full permission back, so
/// its entries can be listed and unlinked. A read-only folder inside a project
/// (Go's module cache is mode 555) otherwise stops the removal for good.
/// Unix only; Windows' read-only attribute is `fs_retry`'s business.
fn grant_owner(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::symlink_metadata(dir)
            && metadata.file_type().is_dir()
        {
            let mode = metadata.permissions().mode() | 0o700;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(mode));
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn entry_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
