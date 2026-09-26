//! How an old copy leaves once its project is the moved copy: **entry by
//! entry, each removed only once the moved copy provably holds it** — or holds
//! it because this put it there.
//!
//! 3.13 asked one question of the whole old copy — is every entry exactly what
//! the move recorded? — and on a single "no" kept all of it, for a person, for
//! ever. A dev server that wrote one log line into the original between the
//! scan and the retire was enough, and so was a cloud mount whose rename moved
//! one file's time. The merge asks per entry instead ([`decide`]), and what is
//! left afterwards is exactly what needs a person: an entry changed both in
//! the old copy and in the moved one, or one the moved copy holds as something
//! else.
//!
//! **Two policies**, because what may be written into the moved copy depends
//! on how sure fastf is of what the old copy is:
//!
//! - [`Policy::Full`] — the old copy as the retire left it, within an hour of
//!   the publish. What the moved copy lacks is completed from it; what changed
//!   in it since the scan is carried into the moved copy when the moved copy
//!   has not changed too; what is new in it is copied across.
//! - [`Policy::Residue`] — anything else: the part of the original a split
//!   rename left behind, a folder that came back after it was gone, a 3.11
//!   record, anything past the hour. Only what the move recorded, unchanged,
//!   and what the moved copy holds as published or newer, is removed; nothing
//!   is written into the moved copy — a program that made the folder again
//!   must not make the project's files.
//!
//! **Writes into the moved copy are never renamed into place**: a cloud mount
//! misplaces renames with uploads in flight. Each is `create_new` at its final
//! path, announced first by a create-only marker in the move's record
//! (`write.<n>`), so a write a crash tore is redone by the next pass instead of
//! being taken for the user's newer file. `PROJECT_INFO.md` is never written:
//! the moved copy's own is the project's identity.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use crate::core::progress::Ticker;
use crate::core::project_info::RESERVED_FILENAME;
use crate::core::removal::{self, Judge, Purpose, Removal, Verdict};
use crate::core::transactions::{self, ManifestEntry, ManifestKind, MoveManifest, Walk};

/// How long after the publish the old copy may still write into the moved
/// one ([`Policy::Full`]).
pub(crate) const FULL_WINDOW: Duration = Duration::from_secs(60 * 60);

/// What the merge may do; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Policy {
    Full,
    Residue,
}

impl Policy {
    /// [`Policy::Full`] for an old copy the retire left whole, published less
    /// than [`FULL_WINDOW`] ago; [`Policy::Residue`] otherwise.
    pub(crate) fn for_old_copy(published_at: Option<SystemTime>, residue: bool) -> Self {
        let recent = published_at.is_some_and(|at| {
            SystemTime::now()
                .duration_since(at)
                .is_ok_and(|age| age < FULL_WINDOW)
        });
        if recent && !residue {
            Self::Full
        } else {
            Self::Residue
        }
    }
}

/// What to do with one entry of the old copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// The moved copy holds it: remove it.
    Remove,
    /// The moved copy lacks it: put it there, then remove it.
    CompleteThenRemove,
    /// It changed here since the scan, and the moved copy did not: carry this
    /// version into the moved copy, then remove it.
    ReplaceThenRemove,
    /// It is new here and the moved copy has nothing at its path: copy it
    /// across, then remove it.
    CopyNewThenRemove,
    /// Its size agrees and only a time differs — a cloud mount's rename or
    /// upload moves times — or it is new here and the moved copy holds the
    /// same size at its path: compare the bytes before anything is decided.
    CompareContent,
    /// Leave it, for this reason.
    Keep(String),
}

/// **The one decision**, for an entry of the old copy at `relative`: `here`
/// is what it is now (`None` for what a record cannot hold), `recorded` what
/// the move's scan saw, `published` what the moved copy was when it was
/// published, `moved` what the moved copy holds now. `has_published` says the
/// record has a published list at all (3.11's did not). Pure, so the whole
/// table is tested without a filesystem.
pub(crate) fn decide(
    relative: &Path,
    here: Option<&ManifestEntry>,
    recorded: Option<&ManifestEntry>,
    published: Option<&ManifestEntry>,
    moved: Option<&ManifestEntry>,
    has_published: bool,
    policy: Policy,
) -> Decision {
    use Decision::*;
    let full = policy == Policy::Full;
    let Some(here) = here else {
        return Keep(
            "fastf cannot copy it (a socket, pipe or device, or a name it cannot record)"
                .to_string(),
        );
    };
    let unchanged = recorded.is_some_and(|recorded| transactions::agrees(recorded, here));
    // A link is its target: its own time says nothing.
    let same_link = here.kind.is_link()
        && recorded.is_some_and(|recorded| {
            recorded.kind == here.kind && recorded.link_target == here.link_target
        });
    let only_time_moved = here.kind == ManifestKind::File
        && recorded.is_some_and(|recorded| {
            recorded.kind == ManifestKind::File && recorded.bytes == here.bytes
        })
        && !unchanged;
    if relative == Path::new(RESERVED_FILENAME) {
        // Never written into the moved copy: its own is the project's
        // identity, and the move's bookkeeping rewrote it.
        return match moved {
            Some(moved) if moved.kind == ManifestKind::File => {
                if unchanged {
                    Remove
                } else if only_time_moved || here.kind == ManifestKind::File {
                    CompareContent
                } else {
                    Keep(
                        "the project's PROJECT_INFO.md here is not what the move copied"
                            .to_string(),
                    )
                }
            }
            _ => Keep("the moved copy has no PROJECT_INFO.md".to_string()),
        };
    }
    if here.kind == ManifestKind::Directory {
        return match moved {
            Some(moved) if moved.kind == ManifestKind::Directory => Remove,
            Some(_) => {
                Keep("a folder here, and something else at its path in the moved copy".to_string())
            }
            None if full => CompleteThenRemove,
            None if recorded.is_some() => Keep("not in the moved copy".to_string()),
            None => Keep("made here after the move, and not in the moved copy".to_string()),
        };
    }
    if unchanged || same_link {
        return match moved {
            None if full => CompleteThenRemove,
            None => Keep("not in the moved copy".to_string()),
            Some(moved) if moved.kind != here.kind => {
                Keep("another kind of entry at its path in the moved copy".to_string())
            }
            Some(moved) if covers(moved, recorded, published, has_published) => Remove,
            Some(_) => Keep(
                "older in the moved copy than when it was moved (restored from a backup?)"
                    .to_string(),
            ),
        };
    }
    if only_time_moved
        && moved.is_some_and(|moved| moved.kind == ManifestKind::File && moved.bytes == here.bytes)
    {
        return CompareContent;
    }
    if recorded.is_some() {
        // Changed here since the scan.
        return match moved {
            _ if !full => Keep("changed here since the move scanned it".to_string()),
            None => CopyNewThenRemove,
            Some(moved) if moved.kind != here.kind => Keep(
                "changed here, and another kind of entry at its path in the moved copy".to_string(),
            ),
            Some(moved) if published.is_some_and(|then| transactions::agrees(then, moved)) => {
                ReplaceThenRemove
            }
            Some(_) => Keep("changed both here and in the moved copy since the move".to_string()),
        };
    }
    // New here: nothing the move recorded.
    match moved {
        _ if !full => Keep("not something the move recorded".to_string()),
        None => CopyNewThenRemove,
        Some(moved)
            if moved.kind == here.kind
                && moved.bytes == here.bytes
                && moved.link_target == here.link_target =>
        {
            if here.kind.is_link() {
                Remove
            } else {
                CompareContent
            }
        }
        Some(_) => Keep(
            "made here after the move, and the moved copy holds something else at its path"
                .to_string(),
        ),
    }
}

/// Whether the moved copy's entry is what was published there or newer —
/// "or newer" lets the user work on the moved copy while a cleanup waits,
/// "not older" refuses a moved copy restored from a backup taken before the
/// move. Without a published entry (3.11, or a publish that could not read its
/// own `PROJECT_INFO.md` back) it is measured against the original's time: a
/// copy is always written after what it copied.
fn covers(
    moved: &ManifestEntry,
    recorded: Option<&ManifestEntry>,
    published: Option<&ManifestEntry>,
    has_published: bool,
) -> bool {
    if moved.kind.is_link() {
        return true;
    }
    let baseline = match (published, has_published) {
        (Some(then), _) => then,
        (None, _) => match recorded {
            Some(recorded) => recorded,
            None => return false,
        },
    };
    moved.source_modified.nanos() >= baseline.source_modified.nanos()
}

/// Everything one merge works from.
pub(crate) struct Merge<'a> {
    /// The old copy: a retired folder, or the original's own path.
    pub old: &'a Path,
    /// The moved copy.
    pub moved: &'a Path,
    pub project_id: &'a str,
    pub manifest: &'a MoveManifest,
    /// `None` for a 3.11 record.
    pub published: Option<&'a MoveManifest>,
    pub policy: Policy,
    /// The move's record, where each write into the moved copy is announced
    /// first; `None` when nothing may be written.
    pub record: Option<&'a Path>,
    pub ticker: Ticker<'a>,
}

/// Merge `merge.old` into the moved copy and remove it; see the module docs.
pub(crate) fn merge_remove(merge: &Merge) -> Removal {
    if let Err(error) =
        crate::core::move_cleanup::confirm_identity(merge.moved, merge.project_id, "moved")
    {
        return Removal::Leftover {
            remaining: 0,
            reason: format!(
                "the moved copy is not this project any more ({error:#}), so all of it was kept"
            ),
            kept_on_purpose: true,
        };
    }
    let recorded = merge.manifest.entries.len();
    merge
        .ticker
        .phase(crate::core::assets::JobPhase::Checking, recorded);
    let moved = match Walk::of_with(merge.moved, "moved copy", merge.ticker.uncancellable()) {
        Ok(walk) => walk,
        Err(error) => {
            return Removal::Leftover {
                remaining: 0,
                reason: format!("the moved copy could not be read ({error:#})"),
                kept_on_purpose: false,
            };
        }
    };
    // A write a crash may have torn: the moved copy's entry there is not
    // evidence of anything until it is written again.
    let unfinished = merge.record.map(read_write_markers).unwrap_or_default();
    let policy = if merge.record.is_none() {
        Policy::Residue
    } else {
        merge.policy
    };
    let judge = MergeJudge {
        merge,
        policy,
        moved: moved
            .entries
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect(),
        unfinished: unfinished.iter().map(|(_, path)| path.clone()).collect(),
        markers: Mutex::new(unfinished),
    };
    merge
        .ticker
        .phase(crate::core::assets::JobPhase::Removing, recorded);
    let removal = removal::remove_tree_judged(
        merge.old,
        &judge,
        Purpose::Move,
        merge.ticker,
        Some((merge.moved, merge.project_id)),
    );
    // Every write whose old entry is gone is proven: its marker goes.
    let markers = judge
        .markers
        .into_inner()
        .unwrap_or_else(|e| e.into_inner());
    for (marker, relative) in markers {
        if crate::util::paths::presence(&merge.old.join(&relative)).is_absent() {
            let _ = fs::remove_file(marker);
        }
    }
    removal
}

struct MergeJudge<'a> {
    merge: &'a Merge<'a>,
    policy: Policy,
    moved: HashMap<PathBuf, ManifestEntry>,
    /// Paths whose moved entry a torn write may have left.
    unfinished: HashSet<PathBuf>,
    /// Every write marker: the file, and the path it announces.
    markers: Mutex<Vec<(PathBuf, PathBuf)>>,
}

impl Judge for MergeJudge<'_> {
    fn judge(&self, path: &Path, relative: &Path, metadata: &fs::Metadata) -> Verdict {
        let here = transactions::entry_of(path, relative, metadata);
        let recorded = self.merge.manifest.entry(relative);
        let published = self
            .merge
            .published
            .and_then(|published| published.entry(relative));
        let unfinished = self.unfinished.contains(relative);
        let moved_now = if unfinished {
            None
        } else {
            self.moved.get(relative)
        };
        let decision = decide(
            relative,
            here.as_ref(),
            recorded,
            published,
            moved_now,
            self.merge.published.is_some(),
            self.policy,
        );
        let keep = |why: String| Verdict::Keep {
            why,
            on_purpose: true,
        };
        let Some(here) = here else {
            return match decision {
                Decision::Keep(why) => keep(why),
                _ => keep("fastf cannot examine it".to_string()),
            };
        };
        match decision {
            Decision::Remove => Verdict::Take,
            Decision::Keep(why) => keep(why),
            Decision::CompareContent => {
                if self.same_content(path, relative) {
                    Verdict::Take
                } else if self.policy == Policy::Full
                    && recorded.is_some()
                    && relative != Path::new(RESERVED_FILENAME)
                    && moved_now
                        .zip(published)
                        .is_some_and(|(moved, then)| transactions::agrees(then, moved))
                {
                    self.write(path, relative, &here, true)
                } else {
                    keep("its contents differ from the moved copy's".to_string())
                }
            }
            Decision::CompleteThenRemove | Decision::CopyNewThenRemove => {
                self.write(path, relative, &here, unfinished)
            }
            Decision::ReplaceThenRemove => self.write(path, relative, &here, true),
        }
    }
}

impl MergeJudge<'_> {
    /// Whether the old entry at `path` holds the moved copy's bytes —
    /// `PROJECT_INFO.md` without the two lines the move's bookkeeping
    /// rewrites.
    fn same_content(&self, path: &Path, relative: &Path) -> bool {
        let other = self.merge.moved.join(relative);
        if relative == Path::new(RESERVED_FILENAME) {
            return match (fs::read_to_string(path), fs::read_to_string(&other)) {
                (Ok(here), Ok(there)) => same_but_for_place(&here, &there),
                _ => false,
            };
        }
        same_bytes(path, &other)
    }

    /// Put the old copy's entry into the moved copy — replacing what is
    /// there when `replace` — then let it go. Announced first by a marker in
    /// the record, written at its final path, never renamed.
    fn write(&self, path: &Path, relative: &Path, here: &ManifestEntry, replace: bool) -> Verdict {
        let keep = |why: String| Verdict::Keep {
            why,
            on_purpose: true,
        };
        let Some(record) = self.merge.record else {
            return keep("not in the moved copy".to_string());
        };
        if relative == Path::new(RESERVED_FILENAME) {
            return keep("the project's PROJECT_INFO.md is never written by a cleanup".to_string());
        }
        let destination =
            match crate::util::paths::contained_destination(self.merge.moved, relative) {
                Ok(destination) => destination,
                Err(error) => {
                    return keep(format!("cannot be put into the moved copy ({error:#})"));
                }
            };
        if here.kind == ManifestKind::Directory {
            return match fs::create_dir(&destination) {
                Ok(()) => Verdict::Take,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Verdict::Take,
                Err(error) => keep(format!(
                    "its folder could not be made in the moved copy ({error})"
                )),
            };
        }
        let marker = match write_marker(record, relative) {
            Ok(marker) => marker,
            Err(error) => {
                return keep(format!(
                    "the write into the moved copy could not be announced in the move's record ({error:#})"
                ));
            }
        };
        self.markers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((marker, relative.to_path_buf()));
        if replace
            && let Err(error) = crate::util::fs_retry::remove_file(&destination)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return keep(format!(
                "the moved copy's version could not be replaced ({error})"
            ));
        }
        let written = match here.kind {
            ManifestKind::File => copy_whole(path, &destination, here),
            _ => match &here.link_target {
                Some(target) => transactions::make_link(here.kind, target, &destination)
                    .map_err(|error| transactions::link_refusal(&error)),
                None => Err("the link has no target".to_string()),
            },
        };
        match written {
            Ok(()) => Verdict::Take,
            Err(why) => keep(format!("could not be put into the moved copy ({why})")),
        }
    }
}

/// Remove an old copy whose move left no record — 3.13 cleared records
/// while their old copies were still there — **entry by entry where the
/// project at `moved` holds the same thing, byte for byte**: a file with the
/// same bytes (`PROJECT_INFO.md` without the two lines a move rewrites), a
/// link to the same target, a folder that ends empty. What differs, or is
/// not in the project, stays. Nothing is removed that exists nowhere else, so
/// no record is needed to prove it.
pub(crate) fn remove_identical(
    old: &Path,
    moved: &Path,
    project_id: &str,
    ticker: Ticker,
) -> Removal {
    if let Err(error) = crate::core::move_cleanup::confirm_identity(moved, project_id, "moved") {
        return Removal::Leftover {
            remaining: 0,
            reason: format!("the project is not where it was looked for ({error:#})"),
            kept_on_purpose: true,
        };
    }
    removal::remove_tree_judged(
        old,
        &Identical { moved },
        Purpose::Move,
        ticker,
        Some((moved, project_id)),
    )
}

/// Takes what the project holds the same, byte for byte.
struct Identical<'a> {
    moved: &'a Path,
}

impl Judge for Identical<'_> {
    fn judge(&self, path: &Path, relative: &Path, metadata: &fs::Metadata) -> Verdict {
        let keep = |why: &str| Verdict::Keep {
            why: why.to_string(),
            on_purpose: true,
        };
        let other = self.moved.join(relative);
        let Ok(there) = fs::symlink_metadata(&other) else {
            return keep("not in the project");
        };
        let (here_type, there_type) = (metadata.file_type(), there.file_type());
        if here_type.is_dir() {
            return if there_type.is_dir() {
                Verdict::Take
            } else {
                keep("something else at its path in the project")
            };
        }
        if here_type.is_symlink() {
            let same = there_type.is_symlink()
                && matches!((fs::read_link(path), fs::read_link(&other)), (Ok(a), Ok(b)) if a == b);
            return if same {
                Verdict::Take
            } else {
                keep("a link the project does not hold")
            };
        }
        if !here_type.is_file() || !there_type.is_file() {
            return keep("not a file the project holds");
        }
        let same = if relative == Path::new(RESERVED_FILENAME) {
            matches!(
                (fs::read_to_string(path), fs::read_to_string(&other)),
                (Ok(here), Ok(there)) if same_but_for_place(&here, &there)
            )
        } else {
            metadata.len() == there.len() && same_bytes(path, &other)
        };
        if same {
            Verdict::Take
        } else {
            keep("its contents differ from the project's")
        }
    }
}

/// Copy the old copy's file at `from` to `to`, new, keeping its mode and
/// times; an error when the file changed while it was read — it is then left
/// for the next pass, whole.
fn copy_whole(from: &Path, to: &Path, expected: &ManifestEntry) -> Result<(), String> {
    let mut reader = fs::File::open(from).map_err(|error| error.to_string())?;
    let before = reader.metadata().map_err(|error| error.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut writer = options.open(to).map_err(|error| error.to_string())?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    transactions::keep_attributes(&writer, &before);
    writer.sync_all().map_err(|error| error.to_string())?;
    let after = fs::symlink_metadata(from).map_err(|error| error.to_string())?;
    let unchanged = after.len() == before.len()
        && after.modified().ok() == before.modified().ok()
        && before.len() == expected.bytes;
    if unchanged {
        Ok(())
    } else {
        Err("it changed while it was copied".to_string())
    }
}

/// Whether two files hold the same bytes, read side by side; `false` when
/// either cannot be read.
fn same_bytes(one: &Path, other: &Path) -> bool {
    let (Ok(mut one), Ok(mut other)) = (fs::File::open(one), fs::File::open(other)) else {
        return false;
    };
    match (one.metadata(), other.metadata()) {
        (Ok(a), Ok(b)) if a.len() == b.len() => {}
        _ => return false,
    }
    let mut left = vec![0_u8; 256 * 1024];
    let mut right = vec![0_u8; 256 * 1024];
    loop {
        let Ok(count) = one.read(&mut left) else {
            return false;
        };
        if count == 0 {
            return other.read(&mut right).is_ok_and(|n| n == 0);
        }
        if other.read_exact(&mut right[..count]).is_err() || left[..count] != right[..count] {
            return false;
        }
    }
}

/// Whether two `PROJECT_INFO.md` texts are the same project file but for
/// where it is: the frontmatter's `path:` and `folder:` lines, which a move's
/// bookkeeping rewrites, are left out; everything else must agree.
pub(crate) fn same_but_for_place(one: &str, other: &str) -> bool {
    fn without_place(text: &str) -> Option<(Vec<&str>, &str)> {
        let (frontmatter, body) = crate::core::project_info::split_frontmatter_body(text)?;
        let lines = frontmatter
            .lines()
            .filter(|line| !line.starts_with("path:") && !line.starts_with("folder:"))
            .collect();
        Some((lines, body))
    }
    match (without_place(one), without_place(other)) {
        (Some(one), Some(other)) => one == other,
        _ => one == other,
    }
}

/// The prefix of a write marker in a move's record.
const WRITE_MARKER: &str = "write.";

/// Announce a write of `relative` into the moved copy: a new file in the
/// record, created once and never renamed.
fn write_marker(record: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    let text = relative
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("a name that is not valid Unicode"))?;
    for attempt in 0..64u32 {
        let marker = record.join(format!(
            "{WRITE_MARKER}{}-{attempt}",
            transactions::next_operation_id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(mut file) => {
                file.write_all(serde_json::to_string(text)?.as_bytes())?;
                file.sync_all()?;
                return Ok(marker);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("no free marker name")
}

/// Every write marker a record holds, and the path each announces.
fn read_write_markers(record: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(entries) = fs::read_dir(record) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(WRITE_MARKER)
        })
        .filter_map(|entry| {
            let text = fs::read_to_string(entry.path()).ok()?;
            let relative: String = serde_json::from_str(&text).ok()?;
            let relative = PathBuf::from(relative);
            crate::util::paths::require_native_relative(&relative, "write marker")
                .ok()
                .map(|()| (entry.path(), relative))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::transactions::ModifiedTime;

    fn file(path: &str, bytes: u64, time: i64) -> ManifestEntry {
        ManifestEntry {
            path: PathBuf::from(path),
            kind: ManifestKind::File,
            bytes,
            source_modified: ModifiedTime::from_nanos_for_test(time),
            link_target: None,
        }
    }

    fn folder(path: &str) -> ManifestEntry {
        ManifestEntry {
            path: PathBuf::from(path),
            kind: ManifestKind::Directory,
            bytes: 0,
            source_modified: ModifiedTime::from_nanos_for_test(1),
            link_target: None,
        }
    }

    fn link(path: &str, target: &str, time: i64) -> ManifestEntry {
        ManifestEntry {
            path: PathBuf::from(path),
            kind: ManifestKind::Symlink,
            bytes: 0,
            source_modified: ModifiedTime::from_nanos_for_test(time),
            link_target: Some(PathBuf::from(target)),
        }
    }

    fn decide_full(
        here: Option<&ManifestEntry>,
        recorded: Option<&ManifestEntry>,
        published: Option<&ManifestEntry>,
        moved: Option<&ManifestEntry>,
    ) -> Decision {
        decide(
            Path::new("a.txt"),
            here,
            recorded,
            published,
            moved,
            true,
            Policy::Full,
        )
    }

    fn decide_residue(
        here: Option<&ManifestEntry>,
        recorded: Option<&ManifestEntry>,
        published: Option<&ManifestEntry>,
        moved: Option<&ManifestEntry>,
    ) -> Decision {
        decide(
            Path::new("a.txt"),
            here,
            recorded,
            published,
            moved,
            true,
            Policy::Residue,
        )
    }

    /// The table, for an entry the move recorded and nobody changed.
    #[test]
    fn an_unchanged_entry_goes_once_the_moved_copy_holds_it() {
        let scanned = file("a.txt", 3, 10);
        let published = file("a.txt", 3, 50);
        let edited_there = file("a.txt", 9, 90);
        let restored = file("a.txt", 3, 20);
        let all = [
            (Some(&published), Decision::Remove, Decision::Remove),
            (Some(&edited_there), Decision::Remove, Decision::Remove),
            (
                None,
                Decision::CompleteThenRemove,
                Decision::Keep("not in the moved copy".into()),
            ),
        ];
        for (moved, full, residue) in all {
            assert_eq!(
                decide_full(Some(&scanned), Some(&scanned), Some(&published), moved),
                full
            );
            assert_eq!(
                decide_residue(Some(&scanned), Some(&scanned), Some(&published), moved),
                residue
            );
        }
        assert!(matches!(
            decide_full(Some(&scanned), Some(&scanned), Some(&published), Some(&restored)),
            Decision::Keep(why) if why.contains("backup")
        ));
        let other_kind = link("a.txt", "b", 60);
        assert!(matches!(
            decide_full(Some(&scanned), Some(&scanned), Some(&published), Some(&other_kind)),
            Decision::Keep(why) if why.contains("another kind")
        ));
    }

    /// Changed here since the scan: carried into the moved copy while the
    /// moved copy is as published — and only under `Full`.
    #[test]
    fn a_change_here_is_carried_across_unless_the_moved_copy_changed_too() {
        let scanned = file("a.txt", 3, 10);
        let appended = file("a.txt", 7, 30);
        let published = file("a.txt", 3, 50);
        let edited_there = file("a.txt", 4, 90);
        assert_eq!(
            decide_full(
                Some(&appended),
                Some(&scanned),
                Some(&published),
                Some(&published)
            ),
            Decision::ReplaceThenRemove
        );
        assert_eq!(
            decide_full(Some(&appended), Some(&scanned), Some(&published), None),
            Decision::CopyNewThenRemove
        );
        assert!(matches!(
            decide_full(Some(&appended), Some(&scanned), Some(&published), Some(&edited_there)),
            Decision::Keep(why) if why.contains("both")
        ));
        assert!(matches!(
            decide_residue(
                Some(&appended),
                Some(&scanned),
                Some(&published),
                Some(&published)
            ),
            Decision::Keep(_)
        ));
    }

    /// Only a time moved: the bytes decide.
    #[test]
    fn a_moved_time_is_settled_by_the_contents() {
        let scanned = file("a.txt", 3, 10);
        let touched = file("a.txt", 3, 99);
        let published = file("a.txt", 3, 50);
        assert_eq!(
            decide_full(
                Some(&touched),
                Some(&scanned),
                Some(&published),
                Some(&published)
            ),
            Decision::CompareContent
        );
        assert_eq!(
            decide_residue(
                Some(&touched),
                Some(&scanned),
                Some(&published),
                Some(&published)
            ),
            Decision::CompareContent
        );
    }

    /// New here: copied across under `Full` when the moved copy has nothing
    /// there, compared when it holds the same size, kept otherwise.
    #[test]
    fn something_new_here_is_copied_across_or_compared() {
        let new = file("a.txt", 5, 70);
        assert_eq!(
            decide_full(Some(&new), None, None, None),
            Decision::CopyNewThenRemove
        );
        assert_eq!(
            decide_full(Some(&new), None, None, Some(&file("a.txt", 5, 80))),
            Decision::CompareContent
        );
        assert!(matches!(
            decide_full(Some(&new), None, None, Some(&file("a.txt", 6, 80))),
            Decision::Keep(_)
        ));
        assert!(matches!(
            decide_residue(Some(&new), None, None, None),
            Decision::Keep(_)
        ));
    }

    /// Folders: walked into when the moved copy has one; made there under
    /// `Full`; kept under `Residue`.
    #[test]
    fn folders_are_walked_into_or_made() {
        let here = folder("a.txt");
        assert_eq!(
            decide_full(Some(&here), Some(&here), None, Some(&here)),
            Decision::Remove
        );
        assert_eq!(
            decide_full(Some(&here), Some(&here), None, None),
            Decision::CompleteThenRemove
        );
        assert!(matches!(
            decide_residue(Some(&here), None, None, None),
            Decision::Keep(_)
        ));
        assert!(matches!(
            decide_full(Some(&here), Some(&here), None, Some(&file("a.txt", 1, 1))),
            Decision::Keep(_)
        ));
    }

    /// A link is its target; its time says nothing.
    #[test]
    fn a_link_is_judged_by_its_target() {
        let scanned = link("a.txt", "b", 10);
        let renamed = link("a.txt", "b", 99);
        assert_eq!(
            decide_full(
                Some(&renamed),
                Some(&scanned),
                Some(&scanned),
                Some(&scanned)
            ),
            Decision::Remove
        );
        let retargeted = link("a.txt", "c", 99);
        assert_eq!(
            decide_full(
                Some(&retargeted),
                Some(&scanned),
                Some(&scanned),
                Some(&scanned)
            ),
            Decision::ReplaceThenRemove
        );
    }

    /// `PROJECT_INFO.md` is never written into the moved copy.
    #[test]
    fn project_info_is_removed_or_compared_never_written() {
        let at = Path::new(RESERVED_FILENAME);
        let scanned = file(RESERVED_FILENAME, 40, 10);
        let rewritten = file(RESERVED_FILENAME, 44, 60);
        let decide_it = |here: &ManifestEntry, moved: Option<&ManifestEntry>| {
            decide(
                at,
                Some(here),
                Some(&scanned),
                Some(&scanned),
                moved,
                true,
                Policy::Full,
            )
        };
        assert_eq!(decide_it(&scanned, Some(&rewritten)), Decision::Remove);
        assert_eq!(
            decide_it(&file(RESERVED_FILENAME, 40, 99), Some(&rewritten)),
            Decision::CompareContent
        );
        assert!(matches!(decide_it(&scanned, None), Decision::Keep(_)));
    }

    #[test]
    fn project_info_compares_without_its_place() {
        let one = "---\nid: ID0001\npath: /a/x\nfolder: x\n---\n# Project Info\n";
        let moved = "---\nid: ID0001\npath: /b/x\nfolder: x\n---\n# Project Info\n";
        let edited = "---\nid: ID0001\npath: /b/x\nfolder: x\n---\n# Project Info\nnote\n";
        assert!(same_but_for_place(one, moved));
        assert!(!same_but_for_place(one, edited));
    }

    /// Only a new policy is `Full`, and only within the hour.
    #[test]
    fn the_policy_is_full_only_soon_after_the_publish() {
        let now = SystemTime::now();
        assert_eq!(Policy::for_old_copy(Some(now), false), Policy::Full);
        assert_eq!(Policy::for_old_copy(Some(now), true), Policy::Residue);
        assert_eq!(
            Policy::for_old_copy(Some(now - FULL_WINDOW - Duration::from_secs(1)), false),
            Policy::Residue
        );
        assert_eq!(Policy::for_old_copy(None, false), Policy::Residue);
    }

    /// **The merge never loses what exists nowhere else**, over random old
    /// copies and moved copies: every file it removed from the old copy is in
    /// the moved copy afterwards, and one that had changed in the old copy
    /// only is there byte for byte; under `Residue` the moved copy is not
    /// written at all.
    #[test]
    fn a_merge_removes_only_what_the_moved_copy_holds() {
        use proptest::prelude::*;
        let config = ProptestConfig {
            cases: 48,
            ..ProptestConfig::default()
        };
        // Per file: changed in the old copy, changed in the moved copy,
        // missing from the moved copy, new in the old copy.
        proptest!(config, |(
            files in prop::collection::vec((any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()), 1..24),
            full in any::<bool>(),
        )| {
            let temp = tempfile::tempdir().unwrap();
            let old = temp.path().join("old");
            let moved = temp.path().join("moved");
            let record = temp.path().join("record");
            for dir in [&old, &moved, &record] {
                fs::create_dir_all(dir.join("sub")).unwrap();
            }
            let identity = "---\nid: ID0001\n---\n";
            fs::write(old.join(RESERVED_FILENAME), identity).unwrap();
            fs::write(moved.join(RESERVED_FILENAME), identity).unwrap();
            let name = |n: usize| {
                PathBuf::from(if n.is_multiple_of(2) {
                    format!("f{n}")
                } else {
                    format!("sub/f{n}")
                })
            };
            for (n, (_, _, _, new_here)) in files.iter().enumerate() {
                if !new_here {
                    fs::write(old.join(name(n)), format!("scanned {n}")).unwrap();
                    fs::write(moved.join(name(n)), format!("scanned {n}")).unwrap();
                }
            }
            let manifest = MoveManifest::scan(&old).unwrap();
            let published = MoveManifest::scan(&moved).unwrap();
            for (n, (changed_here, changed_there, missing_there, new_here)) in files.iter().enumerate() {
                if *new_here {
                    fs::write(old.join(name(n)), format!("new here {n}")).unwrap();
                    continue;
                }
                if *changed_here {
                    fs::write(old.join(name(n)), format!("changed here {n}, longer")).unwrap();
                }
                if *missing_there {
                    fs::remove_file(moved.join(name(n))).unwrap();
                } else if *changed_there {
                    fs::write(moved.join(name(n)), format!("changed there {n}!")).unwrap();
                }
            }
            let before_old: HashMap<PathBuf, Vec<u8>> = (0..files.len())
                .map(|n| (name(n), fs::read(old.join(name(n))).unwrap()))
                .collect();
            let before_moved: HashMap<PathBuf, Option<Vec<u8>>> = (0..files.len())
                .map(|n| (name(n), fs::read(moved.join(name(n))).ok()))
                .collect();

            let policy = if full { Policy::Full } else { Policy::Residue };
            merge_remove(&Merge {
                old: &old,
                moved: &moved,
                project_id: "ID0001",
                manifest: &manifest,
                published: Some(&published),
                policy,
                record: Some(&record),
                ticker: Ticker::none(),
            });

            for (n, (changed_here, _, _, new_here)) in files.iter().enumerate() {
                let path = name(n);
                let removed = !old.join(&path).exists();
                let now_moved = fs::read(moved.join(&path)).ok();
                if removed {
                    prop_assert!(now_moved.is_some(), "{} removed but not in the moved copy", path.display());
                    if *changed_here || *new_here {
                        prop_assert_eq!(
                            now_moved.as_ref(),
                            before_old.get(&path),
                            "{} changed only here and was not carried across whole", path.display()
                        );
                    }
                }
                if !full {
                    prop_assert_eq!(&now_moved, &before_moved[&path], "a residue wrote into the moved copy");
                }
            }
        });
    }
}
