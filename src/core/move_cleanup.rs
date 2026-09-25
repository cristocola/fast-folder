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
//!
//! **And the original is the authority until it is retired.** A moved copy
//! that turns out to be missing entries — a cloud mount that misplaced
//! uploads while its folder was renamed, a file deleted there by hand — is
//! completed from the original ([`complete_destination`]) rather than
//! reported for someone to repair by hand: every missing entry the original
//! still holds exactly as recorded is copied across again, and only then is
//! the rule asked once more.

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

/// The operation a deleted project's hidden folder is named by, if `name` is
/// one `fastf delete` writes — the prefix and an operation id, nothing else.
pub fn deleted_operation(name: &str) -> Option<&str> {
    name.strip_prefix(DELETED_PREFIX)
        .filter(|operation| transactions::is_operation_id(operation))
}

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
    match destination_covers(&walk.entries, cleanup) {
        Ok(()) => {}
        // Only missing: the original holds them, so put them there and ask
        // again. Anything else that differs is somebody's decision, not ours.
        Err(gaps) if gaps.only_missing() => {
            complete_destination(&gaps.missing, cleanup)?;
            destination_covers(&walk.entries, cleanup).map_err(|gaps| gaps.message())?;
        }
        Err(gaps) => return Err(gaps.message()),
    }
    Ok(walk)
}

/// Where the moved copy falls short of the original.
#[derive(Debug, Default)]
struct Gaps {
    /// Recorded entries the moved copy does not hold at all, in manifest
    /// order — parents before children.
    missing: Vec<PathBuf>,
    /// Entries it holds as something else, or older than published.
    other: Vec<String>,
}

impl Gaps {
    fn only_missing(&self) -> bool {
        !self.missing.is_empty() && self.other.is_empty()
    }

    fn message(&self) -> String {
        let count = self.missing.len() + self.other.len();
        let mut message = format!(
            "the moved copy does not hold everything the original does ({count} {}):",
            if count == 1 { "entry" } else { "entries" }
        );
        let lines = self.other.iter().cloned().chain(
            self.missing
                .iter()
                .map(|path| format!("{}: not in the moved copy", path.display())),
        );
        for line in lines.take(LISTED) {
            message.push_str("\n  ");
            message.push_str(&line);
        }
        if count > LISTED {
            message.push_str(&format!("\n  and {} more", count - LISTED));
        }
        message
    }
}

/// Put every entry in `missing` into the moved copy, from the original,
/// provided the original still holds it exactly as the move recorded it.
///
/// Files land through an atomic sibling and a rename, so a crash mid-copy
/// never leaves a half-written file at the real path — which the next pass
/// would take for the user's own newer file, and then remove the original.
/// Every write is checked for containment first: the moved copy is a
/// published project, and a link placed in it since must not be written
/// through.
fn complete_destination(missing: &[PathBuf], cleanup: &Cleanup) -> std::result::Result<(), String> {
    let final_path = cleanup.final_path;
    let mut failures = Vec::new();
    for relative in missing {
        let Some(recorded) = cleanup.manifest.entry(relative) else {
            failures.push(format!("{}: not in the move's record", relative.display()));
            continue;
        };
        let source_path = cleanup.source.join(relative);
        let holds = matches!(
            transactions::examine(&source_path, relative),
            Ok(Some(found)) if transactions::agrees(recorded, &found)
        );
        if !holds {
            failures.push(format!(
                "{}: the original no longer holds it as the move recorded it",
                relative.display()
            ));
            continue;
        }
        let destination = match crate::util::paths::contained_destination(final_path, relative) {
            Ok(destination) => destination,
            Err(error) => {
                failures.push(format!("{}: {error:#}", relative.display()));
                continue;
            }
        };
        let made = match recorded.kind {
            ManifestKind::Directory => match fs::create_dir(&destination) {
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                other => other.map_err(|error| error.to_string()),
            },
            ManifestKind::File => crate::util::atomic::copy(&source_path, &destination)
                .map_err(|error| format!("{error:#}"))
                .and_then(|()| match transactions::examine(&destination, relative) {
                    Ok(Some(found))
                        if found.kind == ManifestKind::File && found.bytes == recorded.bytes =>
                    {
                        Ok(())
                    }
                    _ => Err("did not read back as copied".to_string()),
                }),
            ManifestKind::Symlink | ManifestKind::DirSymlink | ManifestKind::Junction => {
                match &recorded.link_target {
                    Some(target) => transactions::make_link(recorded.kind, target, &destination)
                        .map_err(|error| transactions::link_refusal(&error)),
                    None => Err("the record of this link has no target".to_string()),
                }
            }
        };
        if let Err(why) = made {
            failures.push(format!(
                "{}: could not be put back ({why})",
                relative.display()
            ));
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    let count = failures.len();
    let mut message = format!(
        "the moved copy is missing {count} {} the original holds, and fastf could not put \
         {} back:",
        if count == 1 { "entry" } else { "entries" },
        if count == 1 { "it" } else { "them" }
    );
    for line in failures.iter().take(LISTED) {
        message.push_str("\n  ");
        message.push_str(line);
    }
    if count > LISTED {
        message.push_str(&format!("\n  and {} more", count - LISTED));
    }
    Err(message)
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
) -> std::result::Result<(), Gaps> {
    let mut gaps = Gaps::default();
    if let Err(error) = confirm_identity(cleanup.final_path, cleanup.project_id, "moved") {
        gaps.other.push(format!(
            "the moved copy is not this project any more: {error:#}"
        ));
        return Err(gaps);
    }
    let moved = match Walk::of(cleanup.final_path, "moved copy") {
        Ok(moved) => moved,
        Err(error) => {
            gaps.other.push(format!("{error:#}"));
            return Err(gaps);
        }
    };
    let at: HashMap<&Path, &ManifestEntry> = moved
        .entries
        .iter()
        .map(|entry| (entry.path.as_path(), entry))
        .collect();
    for entry in entries {
        let path = entry.path.display();
        let Some(now) = at.get(entry.path.as_path()) else {
            // In manifest order, so a missing folder comes before what it
            // holds, and the repair makes it first.
            gaps.missing.push(entry.path.clone());
            continue;
        };
        if now.kind != entry.kind {
            gaps.other.push(format!(
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
                Some(_) => gaps.other.push(format!(
                    "{path}: older in the moved copy than when it was moved \
                     (restored from a backup?)"
                )),
                None => gaps
                    .other
                    .push(format!("{path}: not in the copy that was published")),
            },
            None if now.source_modified.nanos() < entry.source_modified.nanos() => {
                gaps.other.push(format!(
                    "{path}: older in the moved copy than here (restored from a backup?)"
                ));
            }
            None => {}
        }
    }
    if gaps.missing.is_empty() && gaps.other.is_empty() {
        return Ok(());
    }
    Err(gaps)
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
        if let Removal::Leftover {
            remaining,
            reason,
            kept_on_purpose,
        } = remove_tree(&retired, Some(cleanup.manifest), Purpose::Move)
        {
            return SourceFate::Leftover {
                path: retired,
                reason: format!(
                    "{remaining} {} left: {reason}",
                    if remaining == 1 { "entry" } else { "entries" }
                ),
                redundant: !kept_on_purpose,
            };
        }
        if let Err(error) = crate::util::faults::check("move:after-source-cleanup") {
            return SourceFate::Removed {
                record_kept: Some(format!("{error:#}")),
            };
        }
    }
    // A 3.12.0 record's old staging folder may hold files the mount put
    // there late; they go into place before the record goes, and anything
    // that cannot keeps the record (`MoveTransaction::sweep_strays`).
    match transaction.sweep_strays(Some(cleanup.manifest)) {
        Ok((_, left)) if left.is_empty() => {}
        Ok((moved, left)) => {
            return SourceFate::Removed {
                record_kept: Some(format!(
                    "{moved} file{} the mount had put in the move's old staging folder \
                     late {} moved into place, and {} left there that fastf will not \
                     remove: {}",
                    if moved == 1 { "" } else { "s" },
                    if moved == 1 { "was" } else { "were" },
                    left.len(),
                    left.iter()
                        .take(LISTED)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
            };
        }
        Err(error) => {
            return SourceFate::Removed {
                record_kept: Some(format!(
                    "the move's old staging folder could not be looked through ({error:#})"
                )),
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
    /// Something is left. `kept_on_purpose` says some of it was kept because
    /// it was not provably safe to remove — changed since it was recorded, on
    /// another filesystem, behind links the mount hides — rather than because
    /// a removal failed; the next attempt will keep it again.
    Leftover {
        remaining: usize,
        reason: String,
        kept_on_purpose: bool,
    },
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
    let count = |root: &Path| {
        Walk::of(root, "folder")
            .map(|walk| walk.entries.len() + walk.problems.len())
            .unwrap_or(0)
    };
    // A mount that shows a link as what it points to would have this walk
    // step into a linked folder as if it were the project's own.
    if let Some(why) = root
        .parent()
        .and_then(crate::core::move_preflight::links_hidden_in)
    {
        return Removal::Leftover {
            remaining: count(root),
            reason: format!("{why}; fastf removed nothing"),
            kept_on_purpose: true,
        };
    }
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
        kept_on_purpose: false,
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
    let remaining = count(root);
    let mut reason = removing.stopped.take().unwrap_or_default();
    if reason.is_empty() {
        reason = if removing.notes.is_empty() {
            "the folder could not be removed".to_string()
        } else {
            removing.notes.join("; ")
        };
    }
    Removal::Leftover {
        remaining,
        reason,
        kept_on_purpose: removing.kept_on_purpose,
    }
}

struct Removing<'a> {
    root: &'a Path,
    recorded: Option<&'a MoveManifest>,
    device: Option<u64>,
    purpose: Purpose,
    removed_one: bool,
    /// Set when a failpoint stops the removal; everything after is left.
    stopped: Option<String>,
    /// Something was kept because removing it was not provably safe.
    kept_on_purpose: bool,
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
            self.kept_on_purpose = true;
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
                // Gone since it was listed — or listed and hidden by the
                // mount, which the folder's own removal then reports.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.note(&path, "listed, but not there to examine");
                    continue;
                }
                Err(error) => {
                    self.note(&path, &format!("cannot be examined ({error})"));
                    continue;
                }
            };
            if let Some(manifest) = self.recorded
                && !self.still_recorded(manifest, &path)
            {
                self.kept_on_purpose = true;
                self.note(&path, "not as the move recorded it, so it was kept");
                continue;
            }
            let file_type = metadata.file_type();
            if file_type.is_dir() {
                if self.device.is_some() && transactions::device_of(&metadata) != self.device {
                    self.kept_on_purpose = true;
                    self.note(&path, "on another filesystem, so it was kept");
                    continue;
                }
                self.children(&path, depth + 1);
                if self.stopped.is_some() {
                    return;
                }
                self.remove(&path, Removed::Folder);
            } else if is_folder_link(&metadata) {
                self.remove(&path, Removed::FolderLink);
            } else {
                self.remove(&path, Removed::Other);
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
    fn remove(&mut self, path: &Path, what: Removed) {
        let attempt = || match what {
            Removed::Folder | Removed::FolderLink => crate::util::fs_retry::remove_dir(path),
            Removed::Other => crate::util::fs_retry::remove_file(path),
        };
        let result = match attempt() {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                if let Some(parent) = path.parent() {
                    grant_owner(parent);
                }
                if what == Removed::Folder {
                    clear_read_only_folder(path);
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

/// What one removal takes away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Removed {
    /// A real folder, already emptied.
    Folder,
    /// A link Windows removes as a folder: a directory symlink or junction.
    FolderLink,
    Other,
}

/// Windows will not remove a folder carrying the read-only attribute —
/// customised folders with a `desktop.ini` carry it, and so do trees copied
/// off read-only media. Cleared only on a real folder: on a link it would
/// reach the link's target. A file's attribute is `fs_retry`'s business.
fn clear_read_only_folder(folder: &Path) {
    #[cfg(windows)]
    if let Ok(metadata) = fs::symlink_metadata(folder)
        && metadata.file_type().is_dir()
        && metadata.permissions().readonly()
    {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(folder, permissions);
    }
    #[cfg(not(windows))]
    let _ = folder;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) {
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("sub/b.txt"), "b").unwrap();
    }

    /// Removal never steps through a link: the link goes, what it points at
    /// stays.
    #[cfg(unix)]
    #[test]
    fn removing_a_tree_takes_a_link_and_not_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let root = temp.path().join(".fastf-deleted-1-1");
        tree(&root);
        std::os::unix::fs::symlink(&outside, root.join("sub/linked")).unwrap();

        assert_eq!(remove_tree(&root, None, Purpose::Delete), Removal::Removed);
        assert!(!root.exists());
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "keep"
        );
    }

    /// An entry changed since the move recorded it is kept, and the leftover
    /// says it was kept on purpose — so nobody is told it is redundant.
    #[test]
    fn a_changed_entry_is_kept_on_purpose() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".fastf-moved-1-2");
        tree(&root);
        let manifest = MoveManifest::scan(&root).unwrap();
        fs::write(root.join("sub/b.txt"), "changed, and longer").unwrap();

        let Removal::Leftover {
            kept_on_purpose,
            reason,
            ..
        } = remove_tree(&root, Some(&manifest), Purpose::Move)
        else {
            panic!("a changed entry must be kept");
        };
        assert!(kept_on_purpose);
        let changed = Path::new("sub").join("b.txt").display().to_string();
        assert!(reason.contains(&changed), "{reason}");
        assert_eq!(
            fs::read_to_string(root.join("sub/b.txt")).unwrap(),
            "changed, and longer"
        );
        assert!(!root.join("a.txt").exists(), "what was as recorded went");
    }

    /// On an ordinary folder links show as links, and asking leaves nothing.
    #[cfg(unix)]
    #[test]
    fn an_ordinary_folder_hides_no_links_and_asking_leaves_nothing() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            crate::core::move_preflight::links_hidden_in(temp.path()),
            None
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
