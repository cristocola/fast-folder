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

use crate::core::assets::JobPhase;
use crate::core::progress::Ticker;
use crate::core::removal::{Purpose, Removal, remove_tree, remove_tree_guarded};
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
    /// Retired and removed, on a mount that can put a removed folder back
    /// (`core::records::can_resurrect`): the record stays until a later pass
    /// finds the old copy still gone, and removes whatever came back.
    Settling,
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
    /// The rename that set the original aside stopped part of the way (an S3
    /// bucket through rclone renames object by object), so part of the
    /// original is still at `source` beside its retired copy. That part is the
    /// move's own residue: removed after the retired copy, under the same
    /// rule, and the record stays until both are gone.
    pub split: bool,
    /// Where each step says how far it has got. Its checks never stop for a
    /// cancel; the removal does when this ticker honours one, and a move hands
    /// the steps after its publish one that does not — they are housekeeping.
    pub ticker: Ticker<'a>,
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
    // A check only reads, and one stopped part of the way would be reported
    // as a tree that failed it: checks never stop for a cancel. The removal
    // after them does, when its ticker honours one.
    let walk = Walk::of_with(tree, "folder", cleanup.ticker.uncancellable())
        .map_err(|error| format!("{error:#}"))?;
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
            complete_destination(&gaps.missing, tree, cleanup)?;
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
fn complete_destination(
    missing: &[PathBuf],
    from: &Path,
    cleanup: &Cleanup,
) -> std::result::Result<(), String> {
    let final_path = cleanup.final_path;
    let mut failures = Vec::new();
    for relative in missing {
        let Some(recorded) = cleanup.manifest.entry(relative) else {
            failures.push(format!("{}: not in the move's record", relative.display()));
            continue;
        };
        // From the tree being checked: the original before its retire, its
        // retired copy after — which is where the original's entries are by
        // then. 3.13 read the original's old path either way, which a retired
        // copy has left, so a repair after the retire could never succeed.
        let source_path = from.join(relative);
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
    let moved = match Walk::of_with(
        cleanup.final_path,
        "moved copy",
        cleanup.ticker.uncancellable(),
    ) {
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
                // A record whose publish could not read its own
                // `PROJECT_INFO.md` back (a mount that failed the lstat) left
                // it out; measured against the original's time instead, the
                // way a record without `published.json` is — rather than
                // failing this check on every pass for ever.
                None if now.source_modified.nanos() >= entry.source_modified.nanos() => {}
                None => gaps.other.push(format!(
                    "{path}: older in the moved copy than here (restored from a backup?)"
                )),
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

/// How setting the original aside ended.
pub(crate) enum SetAside {
    /// Nothing more to do here: the fate is decided.
    Settled(SourceFate),
    /// The original is out of the library and `Retired` is recorded; its old
    /// copy is what is left, for [`remove_retired`] — as housekeeping, once
    /// the data lock is released.
    Retired(MoveTransaction),
}

/// Publish-then-retire, from `CleanupPending` with the source at its path:
/// check, retire, record `Retired`, keep the books — `bookkeeping` runs once
/// the source has left the library, since from that moment the project is the
/// moved copy. **The move is done here**; removing the old copy is
/// housekeeping ([`Housekeeping`]), which needs no data lock — the copy is
/// hidden and redundant by construction, and a record whose job is alive is
/// left alone by every reconcile.
pub(crate) fn set_aside(
    mut transaction: MoveTransaction,
    cleanup: &Cleanup,
    bookkeeping: impl FnOnce(),
) -> SetAside {
    use SetAside::Settled;
    let source = cleanup.source;
    // Two walks: the original, then the moved copy it is compared against.
    cleanup
        .ticker
        .phase(JobPhase::SettingAside, cleanup.manifest.entries.len() * 2);
    let has_identity = crate::core::project_info::pinfo_path(source).is_file();
    if (has_identity || !cleanup.residue_allowed)
        && let Err(error) = confirm_identity(source, cleanup.project_id, "original")
    {
        return Settled(SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        });
    }
    if let Err(reason) = check_removable(source, cleanup.residue_allowed, cleanup) {
        return Settled(SourceFate::KeptWhole { reason });
    }
    // Rewrites a 3.11 journal as version 3 *before* the rename — see
    // `MoveTransaction::set_phase`.
    if let Err(error) = transaction.set_phase(MovePhase::CleanupPending) {
        return Settled(SourceFate::KeptWhole {
            reason: format!("the cleanup could not be recorded ({error:#})"),
        });
    }
    if let Err(error) = crate::util::faults::check("move:before-retire") {
        return Settled(SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        });
    }
    let retired = transaction.retired_path();
    // A retire that fails, for a test to reach.
    if let Err(error) = crate::util::faults::check("move:source-cleanup") {
        return Settled(SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        });
    }
    // One rename, which fastf cannot count into: on an S3-style mount it is
    // a copy and a delete per object inside rclone, and took minutes on a
    // real R2 bucket with the count already full. Say what it is waiting on.
    cleanup.ticker.update(|state| {
        state.current_file = "renaming it aside in one step — on a cloud mount this can \
                              take a while"
            .to_string();
    });
    crate::util::log::info(
        "setting the original aside: one rename, which a cloud mount may take a while over",
    );
    match retire(source, &retired) {
        Retire::Done => {}
        Retire::KeptWhole(reason) => return Settled(SourceFate::KeptWhole { reason }),
        Retire::Unknown(reason) => return Settled(SourceFate::Unknown { reason }),
    }
    if let Err(error) = crate::util::faults::check("move:after-retire") {
        bookkeeping();
        return Settled(SourceFate::Leftover {
            path: retired,
            reason: format!("{error:#}"),
            redundant: true,
        });
    }
    if let Err(error) = transaction.set_phase(MovePhase::Retired) {
        bookkeeping();
        return Settled(SourceFate::Leftover {
            path: retired,
            reason: format!("the retirement could not be recorded ({error:#})"),
            redundant: true,
        });
    }
    bookkeeping();
    SetAside::Retired(transaction)
}

/// Work left after a move or a delete that needs no data lock: removing an
/// old copy that is out of the library. Owned, so it outlives the lock and
/// the borrowed [`Cleanup`] it was made from.
#[derive(Debug)]
pub enum Housekeeping {
    /// A move's retired original, and the facts its removal is checked
    /// against again.
    OldCopy(Box<OldCopy>),
    /// A deleted project's hidden folder.
    Deleted(PathBuf),
}

#[derive(Debug)]
pub struct OldCopy {
    transaction: MoveTransaction,
    manifest: MoveManifest,
    published: Option<MoveManifest>,
    source: PathBuf,
    final_path: PathBuf,
    project_id: String,
    residue_allowed: bool,
    split: bool,
}

impl Housekeeping {
    /// The old copy of a move `cleanup` describes, retired by `transaction`.
    pub(crate) fn old_copy(transaction: MoveTransaction, cleanup: &Cleanup) -> Self {
        Self::OldCopy(Box::new(OldCopy {
            transaction,
            manifest: cleanup.manifest.clone(),
            published: cleanup.published.cloned(),
            source: cleanup.source.to_path_buf(),
            final_path: cleanup.final_path.to_path_buf(),
            project_id: cleanup.project_id.to_string(),
            residue_allowed: cleanup.residue_allowed,
            split: cleanup.split,
        }))
    }

    /// The operation whose record or folder this is — what a job lists as
    /// its own, so no reconcile touches it while the job is alive.
    pub fn operation(&self) -> String {
        match self {
            Self::OldCopy(old) => old.transaction.journal.operation_id.clone(),
            Self::Deleted(path) => path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(deleted_operation)
                .unwrap_or_default()
                .to_string(),
        }
    }

    /// The folder being removed.
    pub fn path(&self) -> PathBuf {
        match self {
            Self::OldCopy(old) => old.transaction.retired_path(),
            Self::Deleted(path) => path.clone(),
        }
    }

    /// Where the project is now, for a move; nothing for a delete.
    pub fn moved_to(&self) -> Option<&Path> {
        match self {
            Self::OldCopy(old) => Some(&old.final_path),
            Self::Deleted(_) => None,
        }
    }

    /// Do it. The removal re-checks everything it removes; see
    /// [`remove_retired`] and [`remove_tree`].
    pub(crate) fn run(self, ticker: Ticker) -> SourceFate {
        match self {
            Self::OldCopy(old) => {
                let old = *old;
                let cleanup = Cleanup {
                    manifest: &old.manifest,
                    published: old.published.as_ref(),
                    source: &old.source,
                    final_path: &old.final_path,
                    project_id: &old.project_id,
                    residue_allowed: old.residue_allowed,
                    split: old.split,
                    ticker,
                };
                remove_retired(old.transaction, &cleanup)
            }
            Self::Deleted(path) => {
                ticker.phase(JobPhase::Removing, 0);
                match remove_tree(&path, None, Purpose::Delete, ticker) {
                    Removal::Removed => SourceFate::Removed { record_kept: None },
                    Removal::Leftover {
                        remaining,
                        reason,
                        kept_on_purpose,
                    } => SourceFate::Leftover {
                        path,
                        reason: format!(
                            "{remaining} {} left: {reason}",
                            if remaining == 1 { "entry" } else { "entries" }
                        ),
                        redundant: !kept_on_purpose,
                    },
                }
            }
        }
    }
}

/// From `Retired`: remove the retired copy if it is still provably redundant,
/// then the transaction. A retired copy that is not is kept whole, for the
/// user to look at — never removed part of the way on purpose.
pub(crate) fn remove_retired(transaction: MoveTransaction, cleanup: &Cleanup) -> SourceFate {
    let retired = transaction.retired_path();
    let present = match crate::util::paths::presence(&retired) {
        crate::util::paths::Presence::Present(_) => true,
        crate::util::paths::Presence::Absent => false,
        // A mount that does not answer says nothing about the old copy: the
        // record stays, and the next pass finishes once it answers.
        crate::util::paths::Presence::Unknown(error) => {
            return SourceFate::Leftover {
                path: retired,
                reason: format!("it does not answer ({error})"),
                redundant: true,
            };
        }
    };
    if present {
        let recorded = cleanup.manifest.entries.len();
        cleanup.ticker.phase(JobPhase::Checking, recorded * 2);
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
        } = {
            cleanup.ticker.phase(JobPhase::Removing, recorded);
            remove_tree_guarded(
                &retired,
                Some(cleanup.manifest),
                Purpose::Move,
                cleanup.ticker,
                Some((cleanup.final_path, cleanup.project_id)),
            )
        } {
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
    if cleanup.split
        && let Some(fate) = remove_split_residue(cleanup)
    {
        return fate;
    }
    if settling(&transaction.journal.operation_id, cleanup.source) {
        return SourceFate::Settling;
    }
    cleanup.ticker.phase(JobPhase::Clearing, 0);
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

/// Whether the record of `operation`, whose old copy is gone from beside
/// `source`, waits out the settle: on a mount that uploads in the background
/// a removed folder can come back minutes later (rclone did, on R2, after a
/// move reported its old copy removed), and only a record can remove it then.
/// A record the data dir's index does not know is cleared at once, as before.
pub(crate) fn settling(operation: &str, source: &Path) -> bool {
    let Some(source_base) = source.parent() else {
        return false;
    };
    if !crate::core::records::can_resurrect(source_base) {
        return false;
    }
    let now = crate::util::time::now_unix();
    crate::core::records::gone_since(operation, now)
        .is_some_and(|since| now - since < crate::core::records::SETTLE_SECS)
}

/// What a split rename left at the original's path, removed under the rule
/// for a residue: every entry one the move recorded, unchanged, and in the
/// moved copy. `None` when it is gone; otherwise why it is not, and the record
/// stays.
fn remove_split_residue(cleanup: &Cleanup) -> Option<SourceFate> {
    let source = cleanup.source.to_path_buf();
    match crate::util::paths::presence(&source) {
        crate::util::paths::Presence::Absent => return None,
        crate::util::paths::Presence::Unknown(error) => {
            return Some(SourceFate::Leftover {
                path: source,
                reason: format!("it does not answer ({error})"),
                redundant: true,
            });
        }
        crate::util::paths::Presence::Present(_) => {}
    }
    // A residue holds only what the move recorded, unchanged. Anything else
    // there means the folder is not (only) the original's: without our
    // `PROJECT_INFO.md` it is one a program made again at that path after the
    // retire, and is left alone; with it, it is ours with something new in
    // it, and both it and the record stay for a person.
    let walk = match Walk::of(&source, "original") {
        Ok(walk) => walk,
        Err(error) => {
            return Some(SourceFate::Leftover {
                path: source,
                reason: format!("{error:#}"),
                redundant: true,
            });
        }
    };
    let diff = cleanup.manifest.compare(&walk, Match::Whole);
    if !diff.is_residue() {
        let ours = crate::core::project_info::read_metadata(&source)
            .ok()
            .flatten()
            .is_some_and(|metadata| metadata.id == cleanup.project_id);
        if !ours {
            return None;
        }
        return Some(SourceFate::Leftover {
            path: source,
            reason: format!(
                "part of the original is still there, and it is not only what was moved: {}",
                diff.summary(LISTED)
            ),
            redundant: false,
        });
    }
    if let Err(reason) = check_removable(&source, true, cleanup) {
        return Some(SourceFate::Leftover {
            path: source,
            reason,
            redundant: false,
        });
    }
    cleanup
        .ticker
        .phase(JobPhase::Removing, cleanup.manifest.entries.len());
    match remove_tree_guarded(
        &source,
        Some(cleanup.manifest),
        Purpose::Move,
        cleanup.ticker,
        Some((cleanup.final_path, cleanup.project_id)),
    ) {
        Removal::Removed => None,
        Removal::Leftover {
            remaining,
            reason,
            kept_on_purpose,
        } => Some(SourceFate::Leftover {
            path: source,
            reason: format!(
                "{remaining} {} left: {reason}",
                if remaining == 1 { "entry" } else { "entries" }
            ),
            redundant: !kept_on_purpose,
        }),
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
    use crate::util::paths::{Presence, presence};
    step_out_of(source);
    match presence(retired) {
        Presence::Absent => {}
        Presence::Present(_) => {
            return Retire::Unknown(format!(
                "{} is already there",
                crate::util::paths::display_path(retired)
            ));
        }
        // Nothing has been renamed yet, so the original is whole where it
        // was; a name that does not answer is no name to rename onto.
        Presence::Unknown(error) => {
            return Retire::KeptWhole(format!(
                "{} does not answer ({error})",
                crate::util::paths::display_path(retired)
            ));
        }
    }
    match crate::util::fs_retry::rename_dir(source, retired) {
        Ok(()) => Retire::Done,
        Err(error) => match (presence(source), presence(retired)) {
            (Presence::Absent, Presence::Present(_)) => Retire::Done,
            (Presence::Present(_), Presence::Absent) => {
                Retire::KeptWhole(crate::util::fs_retry::describe_rename_error(&error))
            }
            // A rename that is not one step — an S3 bucket through rclone
            // copies and deletes object by object — can stop with part of the
            // tree on each side.
            (Presence::Present(_), Presence::Present(_)) => Retire::Unknown(format!(
                "renaming it aside reported '{error}' part of the way: some of it is still \
                 here and some is at {}",
                crate::util::paths::display_path(retired)
            )),
            (Presence::Absent, Presence::Absent) => Retire::Unknown(format!(
                "renaming it aside reported '{error}', and afterwards neither it nor {} \
                 could be found",
                crate::util::paths::display_path(retired)
            )),
            (source, retired_is) => Retire::Unknown(format!(
                "renaming it aside reported '{error}', and the folders do not answer \
                 ({}{})",
                source.unknown().map(|e| e.to_string()).unwrap_or_default(),
                retired_is
                    .unknown()
                    .map(|e| format!("; {e}"))
                    .unwrap_or_default()
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// On an ordinary folder links show as links, and asking leaves nothing.
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
