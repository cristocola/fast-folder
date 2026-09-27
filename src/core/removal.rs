//! Removing a tree fastf is done with: a moved project's retired original, a
//! deleted project, a copy that was never published.
//!
//! Not `std::fs::remove_dir_all`: that stops at the first error, and on unix
//! silently steps over an entry it cannot find, which is the pair that left
//! the 3.11 husk. This carries on past a failure (everything here is already
//! somewhere else, or was asked to go), gives a folder it may not write its
//! owner's permission back, **never follows a link or crosses onto another
//! filesystem**, and — given the move's record — takes only entries still
//! exactly as it records them.
//!
//! **On the pool** (`util::pool`), because on a cloud mount each removal is a
//! request: 3.13 removed an old copy from R2 at about 120 ms an entry, one at
//! a time. Folders are listed and their entries taken by several workers at
//! once; each entry is examined and removed by the same worker, one right
//! after the other, so nothing changes between the look that allows a removal
//! and the removal. The folders go last, deepest first, a level at a time.
//!
//! Three calls an entry at most: its share of a listing, one `lstat`, one
//! unlink. What is left is counted only when something is.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::core::move_cleanup::confirm_identity;
use crate::core::progress::{CANCELLED, Ticker};
use crate::core::transactions::{self, LISTED, MoveManifest, Walk};
use crate::util::pool::Queue;

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

/// What a removal asks of each entry before it takes it — on the worker that
/// then removes it, right after the look that `metadata` is.
pub(crate) trait Judge: Sync {
    /// Whether the entry at `path` (`relative` to the root) may go. For a
    /// folder, [`Verdict::Take`] means: walk into it, and remove it once it
    /// is empty.
    fn judge(&self, path: &Path, relative: &Path, metadata: &fs::Metadata) -> Verdict;
}

/// A [`Judge`]'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Take,
    /// Leave it, and so every folder above it. `on_purpose` when it is kept
    /// because removing it is not provably safe, rather than because
    /// something failed.
    Keep {
        why: String,
        on_purpose: bool,
    },
}

/// Takes everything: a deleted project, a copy never published.
pub(crate) struct Everything;

impl Judge for Everything {
    fn judge(&self, _path: &Path, _relative: &Path, _metadata: &fs::Metadata) -> Verdict {
        Verdict::Take
    }
}

/// Takes only what a move recorded, still exactly as it recorded it.
pub(crate) struct Recorded<'m>(pub &'m MoveManifest);

impl Judge for Recorded<'_> {
    fn judge(&self, path: &Path, relative: &Path, metadata: &fs::Metadata) -> Verdict {
        let still = self.0.entry(relative).is_some_and(|recorded| {
            matches!(
                transactions::entry_of(path, relative, metadata),
                Some(found) if transactions::agrees(recorded, &found)
            )
        });
        if still {
            Verdict::Take
        } else {
            Verdict::Keep {
                why: "not as the move recorded it, so it was kept".to_string(),
                on_purpose: true,
            }
        }
    }
}

/// Remove `root` and everything in it — a retired source, or a deleted
/// project — and, when `recorded` is given, only entries still exactly as it
/// records them; anything else is kept, and so is every folder above it.
///
/// `ticker` counts every entry removed — on a cloud mount each one is an API
/// call, and 1473 of them took ten minutes that used to show as nothing — and
/// a cancel it honours stops the removal where it is; what is left is a
/// redundant leftover the next reconcile finishes. The caller starts the
/// step, since it knows the total.
pub(crate) fn remove_tree(
    root: &Path,
    recorded: Option<&MoveManifest>,
    purpose: Purpose,
    ticker: Ticker,
) -> Removal {
    remove_tree_guarded(root, recorded, purpose, ticker, None)
}

/// How many entries a guarded removal takes between two looks at the moved
/// copy.
pub(crate) const GUARD_EVERY: usize = 500;

/// [`remove_tree`], looking every [`GUARD_EVERY`] entries at the project at
/// `guard` (its path and id): **an old copy's removal runs without the data
/// lock**, for minutes on a cloud mount, and if the moved copy stops being
/// that project meanwhile, the old copy may be the only one left. The removal
/// then stops and keeps the rest, on purpose.
pub(crate) fn remove_tree_guarded(
    root: &Path,
    recorded: Option<&MoveManifest>,
    purpose: Purpose,
    ticker: Ticker,
    guard: Option<(&Path, &str)>,
) -> Removal {
    match recorded {
        Some(manifest) => remove_tree_judged(root, &Recorded(manifest), purpose, ticker, guard),
        None => remove_tree_judged(root, &Everything, purpose, ticker, guard),
    }
}

/// [`remove_tree_guarded`], asking `judge` of each entry whether it may go.
pub(crate) fn remove_tree_judged(
    root: &Path,
    judge: &dyn Judge,
    purpose: Purpose,
    ticker: Ticker,
    guard: Option<(&Path, &str)>,
) -> Removal {
    crate::util::trace::hit("remove_tree");
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
    // An unmounted mount point is an empty folder: everything under it
    // would read as removed. What it was on at the start is what it must
    // still be on at the end for a removal to count.
    let mount = crate::util::fs_kind::mount_identity(root);
    let removing = Removing {
        root,
        judge,
        device: transactions::RootDevice::recorded(
            root,
            transactions::current_device(root),
            transactions::current_device,
        ),
        purpose,
        ticker,
        guard,
        removed: AtomicUsize::new(0),
        first: AtomicBool::new(true),
        stop: AtomicBool::new(false),
        stopped: Mutex::new(None),
        kept_on_purpose: AtomicBool::new(false),
        notes: Mutex::new(Vec::new()),
        folders: Mutex::new(Vec::new()),
        holding: Mutex::new(HashSet::new()),
        first_failure: Mutex::new(None),
    };
    let width = crate::util::pool::width_for(root);
    let _ = crate::util::pool::expand(
        width,
        vec![Step::List(root.to_path_buf(), 0)],
        |step, queue| {
            if removing.stop.load(Ordering::Relaxed) {
                return Err(());
            }
            match step {
                Step::List(dir, depth) => removing.list(&dir, depth, queue),
                Step::Take(paths, depth) => removing.take(paths, depth, queue),
            }
            Ok(())
        },
    );
    // The folders, emptied, deepest first: a level at a time, each on the
    // pool. A folder something was left in is not asked at all.
    let mut levels: Vec<Vec<PathBuf>> = Vec::new();
    for (depth, folder) in removing
        .folders
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
    {
        if levels.len() < depth {
            levels.resize_with(depth, Vec::new);
        }
        levels[depth - 1].push(folder);
    }
    for level in levels.into_iter().rev() {
        if removing.stop.load(Ordering::Relaxed) {
            break;
        }
        let _ = crate::util::pool::run(width, level, |folder| {
            if removing.stop.load(Ordering::Relaxed) {
                return Err(());
            }
            if !removing.is_holding(&folder) {
                removing.remove(&folder, Removed::Folder);
            }
            Ok(())
        });
    }
    if !removing.stop.load(Ordering::Relaxed) && !removing.is_holding(root) {
        match crate::util::fs_retry::remove_dir(root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => removing.note(root, &error.to_string()),
        }
    }
    let unmounted = mount.is_some() && crate::util::fs_kind::mount_identity(root) != mount;
    if unmounted {
        removing.note(
            root,
            "the filesystem it is on is not mounted any more, so what is left there cannot be told",
        );
    }
    match crate::util::paths::presence(root) {
        crate::util::paths::Presence::Absent if !unmounted => return Removal::Removed,
        crate::util::paths::Presence::Absent => {}
        // Not "removed": the mount did not say so. What is left, if anything,
        // is for the next pass once it answers.
        crate::util::paths::Presence::Unknown(error) => {
            removing.note(root, &format!("it does not answer ({error})"));
        }
        crate::util::paths::Presence::Present(_) => {}
    }
    let remaining = count(root);
    let stopped = removing
        .stopped
        .into_inner()
        .unwrap_or_else(|error| error.into_inner());
    let mut notes = removing
        .notes
        .into_inner()
        .unwrap_or_else(|error| error.into_inner());
    notes.sort();
    notes.truncate(LISTED);
    let first = removing
        .first_failure
        .into_inner()
        .unwrap_or_else(|error| error.into_inner())
        .and_then(crate::util::fs_retry::sentence);
    let reason = match (stopped, first) {
        (Some(reason), _) => reason,
        (None, _) if notes.is_empty() => "the folder could not be removed".to_string(),
        (None, Some(sentence)) => format!("{sentence} ({})", notes.join("; ")),
        (None, None) => notes.join("; "),
    };
    Removal::Leftover {
        remaining,
        reason,
        kept_on_purpose: removing.kept_on_purpose.load(Ordering::Relaxed),
    }
}

/// One piece of a removal's work.
enum Step {
    /// A folder to list, at its depth (the root's is 0).
    List(PathBuf, usize),
    /// Entries one listing found, each examined and removed by the worker
    /// that takes them; the depth is their folder's.
    Take(Vec<PathBuf>, usize),
}

/// Entries of one folder one worker takes at a time; see `transactions`'s
/// walk, which batches the same way for the same reason.
const TAKE_BATCH: usize = 32;

/// What every worker of one removal shares.
struct Removing<'a> {
    root: &'a Path,
    judge: &'a dyn Judge,
    device: transactions::RootDevice<'a>,
    purpose: Purpose,
    ticker: Ticker<'a>,
    /// The moved copy — path and id — looked at every [`GUARD_EVERY`]
    /// entries.
    guard: Option<(&'a Path, &'a str)>,
    removed: AtomicUsize,
    /// No entry removed yet: the first one trips `move:mid-gc`.
    first: AtomicBool,
    /// Set with [`Self::stopped`]: nothing more is removed.
    stop: AtomicBool,
    /// Why the removal stopped: a failpoint, a cancel, or the guard.
    stopped: Mutex<Option<String>>,
    /// Something was kept because removing it was not provably safe.
    kept_on_purpose: AtomicBool,
    /// Some of what was left, and why.
    notes: Mutex<Vec<String>>,
    /// Every folder walked into, with its depth, to remove once emptied.
    folders: Mutex<Vec<(usize, PathBuf)>>,
    /// Folders something was left in, and every folder above them: none of
    /// them can go, so none is asked to.
    holding: Mutex<HashSet<PathBuf>>,
    /// What kind of error the first entry that could not be removed met:
    /// what the leftover's reason opens with, in one sentence.
    first_failure: Mutex<Option<crate::util::fs_retry::ErrorClass>>,
}

impl Removing<'_> {
    fn note(&self, path: &Path, why: &str) {
        let mut notes = self.notes.lock().unwrap_or_else(|error| error.into_inner());
        // Kept for sorting, then cut to `LISTED`: a few more than are shown.
        if notes.len() < LISTED * 4 {
            let shown = path
                .strip_prefix(self.root)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .unwrap_or(path);
            notes.push(format!("{}: {why}", shown.display()));
        }
    }

    /// Keep the class of the first error that left something.
    fn failed(&self, error: &std::io::Error) {
        let mut first = self
            .first_failure
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if first.is_none() {
            *first = Some(crate::util::fs_retry::classify(error));
        }
    }

    fn stop_with(&self, reason: String) {
        let mut stopped = self
            .stopped
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if stopped.is_none() {
            *stopped = Some(reason);
        }
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Something stays in `dir`, so `dir` and every folder above it stay too.
    fn hold(&self, dir: &Path) {
        let mut holding = self
            .holding
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut current = dir;
        // A folder already held has its parents held: stop there.
        while holding.insert(current.to_path_buf()) && current != self.root {
            match current.parent() {
                Some(parent) => current = parent,
                None => break,
            }
        }
    }

    fn is_holding(&self, dir: &Path) -> bool {
        self.holding
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(dir)
    }

    /// Keep `path`, whose folder therefore stays, saying why.
    fn leave(&self, path: &Path, why: &str, on_purpose: bool) {
        if on_purpose {
            self.kept_on_purpose.store(true, Ordering::Relaxed);
        }
        self.note(path, why);
        if let Some(parent) = path.parent() {
            self.hold(parent);
        }
    }

    /// List `dir` and hand what it holds to the workers. `dir` itself is
    /// removed later, with the folders.
    fn list(&self, dir: &Path, depth: usize, queue: &Queue<'_, Step>) {
        if depth >= crate::util::paths::MAX_WALK_DEPTH {
            self.kept_on_purpose.store(true, Ordering::Relaxed);
            self.note(dir, "too deep to walk");
            self.hold(dir);
            return;
        }
        let list = || crate::util::fs_retry::list_dir(dir, || Ok(()));
        let listing = match list() {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                grant_owner(dir);
                list()
            }
            other => other,
        };
        // A listing that fails, whole, even asked again, cannot say what it
        // did not list: the folder is noted and kept, and the next pass
        // finishes it.
        let entries = match listing {
            Ok(entries) => entries,
            Err(error) => {
                self.failed(&error);
                self.note(dir, &format!("its listing failed ({error})"));
                self.hold(dir);
                return;
            }
        };
        for batch in entries.chunks(TAKE_BATCH) {
            queue.push(Step::Take(batch.to_vec(), depth));
        }
    }

    /// Examine each of `paths` and remove it — a folder is walked into
    /// instead, and removed with the folders.
    fn take(&self, paths: Vec<PathBuf>, depth: usize, queue: &Queue<'_, Step>) {
        for path in paths {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let metadata =
                match crate::util::fs_retry::with_retry(&path, || fs::symlink_metadata(&path)) {
                    Ok(metadata) => metadata,
                    // Gone since it was listed — or listed and hidden by the
                    // mount, which the folder's own removal then reports.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        self.note(&path, "listed, but not there to examine");
                        continue;
                    }
                    Err(error) => {
                        self.leave(&path, &format!("cannot be examined ({error})"), false);
                        continue;
                    }
                };
            let file_type = metadata.file_type();
            if file_type.is_dir() && self.device.elsewhere(&metadata) {
                self.leave(&path, "on another filesystem, so it was kept", true);
                continue;
            }
            let relative = path.strip_prefix(self.root).unwrap_or(&path);
            if let Verdict::Keep { why, on_purpose } = self.judge.judge(&path, relative, &metadata)
            {
                self.leave(&path, &why, on_purpose);
                continue;
            }
            if file_type.is_dir() {
                self.folders
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push((depth + 1, path.clone()));
                queue.push(Step::List(path, depth + 1));
            } else if is_folder_link(&metadata) {
                self.remove(&path, Removed::FolderLink);
            } else {
                self.remove(&path, Removed::Other);
            }
        }
    }

    /// Remove one entry: a folder (already emptied) or a folder link with
    /// `remove_dir`, anything else with `remove_file`. Never through a link.
    fn remove(&self, path: &Path, what: Removed) {
        if let Err(error) = crate::util::faults::check("remove:each-entry") {
            self.stop_with(format!("{error:#}"));
            return;
        }
        // Asked again by what an error means (`fs_retry::with_retry`): a
        // mount that restarts mid-removal is waited for, not given up on.
        let attempt = || {
            crate::util::fs_retry::with_retry(path, || {
                crate::util::faults::check_io("remove:unlink")?;
                match what {
                    Removed::Folder | Removed::FolderLink => {
                        crate::util::fs_retry::remove_dir(path)
                    }
                    Removed::Other => crate::util::fs_retry::remove_file(path),
                }
            })
        };
        let mut result = match attempt() {
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
        if what == Removed::Folder
            && let Err(error) = &result
            && crate::util::fs_retry::classify(error) == crate::util::fs_retry::ErrorClass::NotEmpty
        {
            result = self.empty_again(path);
        }
        match result {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                self.failed(&error);
                self.leave(path, &error.to_string(), false);
                if what == Removed::Folder {
                    self.hold(path);
                }
                return;
            }
        }
        let removed = self.removed.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some((moved, id)) = self.guard
            && removed.is_multiple_of(GUARD_EVERY)
            && let Err(error) = confirm_identity(moved, id, "moved")
        {
            self.kept_on_purpose.store(true, Ordering::Relaxed);
            self.stop_with(format!(
                "the moved copy is not this project any more ({error:#}), so what is \
                 left of the original was kept"
            ));
            return;
        }
        let relative = path.strip_prefix(self.root).unwrap_or(path);
        if !self.ticker.tick(relative) {
            self.stop_with(CANCELLED.to_string());
            return;
        }
        if self.first.swap(false, Ordering::Relaxed) {
            let tripped = match self.purpose {
                Purpose::Move => crate::util::faults::check("move:mid-gc"),
                Purpose::Delete => Ok(()),
            };
            if let Err(error) = tripped {
                self.stop_with(format!("{error:#}"));
            }
        }
    }
}

impl Removing<'_> {
    /// A folder that says it is not empty once everything in it was taken:
    /// a cloud mount whose listing has not caught up (rclone), or something
    /// that appeared in it since. Listed again: what is there goes through the
    /// judge like everything else, and a folder that lists empty is asked
    /// again after a moment, a few times.
    fn empty_again(&self, folder: &Path) -> std::io::Result<()> {
        let mut last = std::io::Error::from(std::io::ErrorKind::DirectoryNotEmpty);
        for round in 1..=4u64 {
            let entries = crate::util::fs_retry::list_dir(folder, || Ok(()))?;
            for path in entries {
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    continue;
                };
                let relative = path.strip_prefix(self.root).unwrap_or(&path);
                if metadata.file_type().is_dir() {
                    // A folder that appeared: the next pass walks it.
                    self.leave(&path, "appeared while its folder was removed", true);
                    continue;
                }
                match self.judge.judge(&path, relative, &metadata) {
                    Verdict::Take => self.remove(
                        &path,
                        if is_folder_link(&metadata) {
                            Removed::FolderLink
                        } else {
                            Removed::Other
                        },
                    ),
                    Verdict::Keep { why, on_purpose } => self.leave(&path, &why, on_purpose),
                }
            }
            if self.is_holding(folder) {
                return Err(last);
            }
            match crate::util::fs_retry::remove_dir(folder) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error)
                    if crate::util::fs_retry::classify(&error)
                        == crate::util::fs_retry::ErrorClass::NotEmpty =>
                {
                    last = error;
                    std::thread::sleep(std::time::Duration::from_millis(300 * round));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last)
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
pub(crate) fn grant_owner(dir: &Path) {
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

        assert_eq!(
            remove_tree(&root, None, Purpose::Delete, Ticker::none()),
            Removal::Removed
        );
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
        } = remove_tree(&root, Some(&manifest), Purpose::Move, Ticker::none())
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

    /// A long removal looks at the moved copy as it goes: once that is not
    /// the project any more, the old copy may be the only one, and the rest
    /// of it is kept — on purpose, so nobody is told it is redundant. The
    /// workers under way when the look fails finish the entry they hold.
    #[test]
    fn a_removal_stops_when_the_moved_copy_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".fastf-moved-1-3");
        fs::create_dir_all(&root).unwrap();
        for n in 0..(GUARD_EVERY + 100) {
            fs::write(root.join(format!("f{n}")), "x").unwrap();
        }
        let gone = temp.path().join("moved");

        let removal = remove_tree_guarded(
            &root,
            None,
            Purpose::Delete,
            Ticker::none(),
            Some((&gone, "ID0001")),
        );
        let Removal::Leftover {
            kept_on_purpose,
            remaining,
            ..
        } = removal
        else {
            panic!("the removal must stop");
        };
        assert!(kept_on_purpose);
        // The look is every `GUARD_EVERY` entries, not a barrier: workers part
        // of the way through an entry when it fails finish that one.
        assert!(
            (50..=100).contains(&remaining),
            "it stopped at the look: {remaining} left"
        );
    }

    /// A removal is `Removed` only when the folder is provably gone: a mount
    /// that answers the last look with an error has not said so. 3.13 took
    /// the error for "gone" and cleared the record.
    #[cfg(debug_assertions)]
    #[test]
    fn a_removal_is_not_done_until_the_folder_is_provably_gone() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".fastf-deleted-1-4");
        tree(&root);
        let removal = crate::util::faults::with_thread_fault("presence:lstat:eio", || {
            remove_tree(&root, None, Purpose::Delete, Ticker::none())
        });
        let Removal::Leftover { reason, .. } = removal else {
            panic!("an unanswered look is not a removal");
        };
        assert!(reason.contains("does not answer"), "{reason}");
    }

    /// Wide and deep at once — a flat folder of a thousand files beside a
    /// chain twenty folders deep — goes whole, on the pool.
    #[test]
    fn a_wide_and_deep_tree_goes_whole() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".fastf-deleted-1-5");
        fs::create_dir_all(root.join("flat")).unwrap();
        for n in 0..1000 {
            fs::write(root.join("flat").join(format!("f{n}")), "x").unwrap();
        }
        let mut deep = root.join("deep");
        for n in 0..20 {
            deep = deep.join(format!("d{n}"));
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("bottom"), "x").unwrap();
        let progress = Mutex::new(crate::core::assets::Progress::new(&[]));
        let cancel = AtomicBool::new(false);

        assert_eq!(
            remove_tree(
                &root,
                None,
                Purpose::Delete,
                Ticker::new(&progress, &cancel)
            ),
            Removal::Removed
        );
        assert!(!root.exists());
        let counted = progress.lock().unwrap().step_done;
        assert_eq!(counted, 1000 + 1 + 1 + 21, "every entry counted once");
    }

    /// Something kept deep inside keeps only its own folders: everything
    /// beside them goes, and no folder is asked to go while it holds it.
    #[test]
    fn what_is_kept_keeps_only_the_folders_above_it() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".fastf-moved-1-6");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join("c")).unwrap();
        fs::write(root.join("a/b/kept.txt"), "x").unwrap();
        fs::write(root.join("a/gone.txt"), "x").unwrap();
        fs::write(root.join("c/gone.txt"), "x").unwrap();
        let manifest = MoveManifest::scan(&root).unwrap();
        fs::write(root.join("a/b/kept.txt"), "changed").unwrap();

        let Removal::Leftover {
            remaining, reason, ..
        } = remove_tree(&root, Some(&manifest), Purpose::Move, Ticker::none())
        else {
            panic!("a changed entry is kept");
        };
        assert_eq!(remaining, 3, "a, a/b and a/b/kept.txt: {reason}");
        assert!(!root.join("c").exists());
        assert!(!root.join("a/gone.txt").exists());
        assert!(
            !reason.contains("not empty"),
            "no folder was asked to go while it held something: {reason}"
        );
    }

    /// On Windows the pool removes read-only files and a read-only folder,
    /// and takes a junction without reaching through it.
    #[cfg(windows)]
    #[test]
    fn read_only_entries_and_junctions_go_on_windows() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let root = temp.path().join(".fastf-deleted-1-7");
        fs::create_dir_all(root.join("locked")).unwrap();
        for n in 0..200 {
            let file = root.join("locked").join(format!("f{n}.txt"));
            fs::write(&file, "x").unwrap();
            let mut permissions = fs::metadata(&file).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&file, permissions).unwrap();
        }
        let mut permissions = fs::metadata(root.join("locked")).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(root.join("locked"), permissions).unwrap();
        crate::util::win_reparse::create_junction(&outside, &root.join("through")).unwrap();

        assert_eq!(
            remove_tree(&root, None, Purpose::Delete, Ticker::none()),
            Removal::Removed
        );
        assert!(!root.exists());
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "keep"
        );
    }
}
