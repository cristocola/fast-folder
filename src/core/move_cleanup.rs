//! How a source leaves the library, and how what it leaves behind is removed.
//!
//! **A source is never deleted where it stands while it is still the
//! project.** Deleting a tree is not one operation: it walks, and anything
//! that stops the walk part of the way — a folder it may not write, a file a
//! program holds open, a network drop, an entry the filesystem lists but will
//! not let it examine — leaves a tree that still holds its `PROJECT_INFO.md`,
//! so the library lists a husk as the project. That is how 3.11 left 13 of
//! 1473 files of a moved project behind on an sshfs mount and then called the
//! source untouched.
//!
//! So a source is **retired** first, in one step that either happens or does
//! not, and only then emptied:
//!
//! - **renamed** to `<source base>/.fastf-moved-<operation>` — same folder,
//!   same filesystem, dot-prefixed so discovery never lists it — on a local
//!   disk, sshfs, SMB and NFS;
//! - **or, in place, its `PROJECT_INFO.md` removed first** (`RetireStrategy::
//!   InPlace`), a pointer `.fastf-moved-<operation>.json` beside it naming the
//!   record — on rclone and FUSE mounts fastf does not know, where a folder
//!   rename is a copy and a delete for every object and moves uploads still in
//!   flight to the old path.
//!
//! Either way the old copy then leaves through the merge (`core::merge`):
//! entry by entry, each removed only once the moved copy provably holds it,
//! what the moved copy lacks completed from it, what changed in it since the
//! scan carried across while the moved copy has not changed too. What is left
//! is exactly what needs a person. If that stops part of the way, what is left
//! is hidden and redundant, and the next `fastf reconcile` finishes it.

use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::assets::JobPhase;
use crate::core::merge::{self, Merge, Policy};
use crate::core::progress::Ticker;
use crate::core::removal::{Purpose, Removal, remove_tree};
use crate::core::transactions::{
    self, LISTED, Match, MoveManifest, MovePhase, MoveTransaction, RetireStrategy, Walk,
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

/// What an in-place delete writes beside the project before it takes the
/// project's `PROJECT_INFO.md`: which folder, whose, and every entry it held
/// then — the only entries the delete removes, so nothing written there by
/// path afterwards goes with it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeleteRecord {
    pub version: u32,
    pub operation: String,
    pub project_id: String,
    pub folder: PathBuf,
    pub entries: Vec<transactions::ManifestEntry>,
}

/// Where an in-place delete's record lives: beside the project's folder.
pub fn deleted_record_path(base: &Path, operation: &str) -> PathBuf {
    base.join(format!("{DELETED_PREFIX}{operation}.json"))
}

/// The operation an in-place delete's record is named by, if `name` is one.
pub fn deleted_record_operation(name: &str) -> Option<&str> {
    name.strip_prefix(DELETED_PREFIX)
        .and_then(|rest| rest.strip_suffix(".json"))
        .filter(|operation| transactions::is_operation_id(operation))
}

/// Read an in-place delete's record.
pub fn read_delete_record(path: &Path) -> Result<DeleteRecord> {
    let text = fs::read_to_string(path)?;
    let record: DeleteRecord = serde_json::from_str(&text)?;
    if !transactions::is_operation_id(&record.operation) {
        bail!("an in-place delete record with an invalid operation");
    }
    let mut components = record.folder.components();
    if !matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) {
        bail!("an in-place delete record whose folder is not one plain name");
    }
    Ok(record)
}

/// Takes only what an in-place delete listed, by path and kind: the
/// project as it was when the delete began.
struct Listed(std::collections::HashMap<PathBuf, transactions::ManifestKind>);

impl crate::core::removal::Judge for Listed {
    fn judge(
        &self,
        path: &Path,
        relative: &Path,
        metadata: &fs::Metadata,
    ) -> crate::core::removal::Verdict {
        let now = transactions::entry_of(path, relative, metadata).map(|entry| entry.kind);
        if now.is_some() && self.0.get(relative) == now.as_ref() {
            crate::core::removal::Verdict::Take
        } else {
            crate::core::removal::Verdict::Keep {
                why: "not part of the project when it was deleted, so it was kept".to_string(),
                on_purpose: true,
            }
        }
    }
}

/// What became of a source after publication.
#[derive(Debug)]
pub(crate) enum SourceFate {
    /// Retired and removed. `record_kept` says why the transaction could not
    /// be cleared too, when it could not; the next reconcile clears it.
    Removed { record_kept: Option<String> },
    /// Out of the library; its old copy at `path` is not removed yet.
    /// `redundant` says everything in it is also in the moved copy — removing
    /// it stopped part of the way — rather than that part of it was kept
    /// because it holds something the moved copy does not.
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
    /// deleted in place. Its old copy is merged as a residue.
    pub residue_allowed: bool,
    /// The rename that set the original aside stopped part of the way (an S3
    /// bucket through rclone renames object by object), so part of the
    /// original is still at `source` beside its retired copy. That part is the
    /// move's own residue: merged after the retired copy, and the record stays
    /// until both are gone.
    pub split: bool,
    /// The old copy was found gone once and is back (a cloud mount put it
    /// back): whatever came back is merged as a residue.
    pub reappeared: bool,
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

/// How setting the original aside ended.
pub(crate) enum SetAside {
    /// Nothing more to do here: the fate is decided.
    Settled(SourceFate),
    /// The original is out of the library and `Retired` is recorded; its old
    /// copy is what is left, for [`remove_retired`] — as housekeeping, once
    /// the data lock is released. Boxed: a record is many times the size of a
    /// fate.
    Retired(Box<MoveTransaction>),
}

/// Publish-then-retire, from `CleanupPending` with the source at its path:
/// both copies are still this project, the original leaves the library
/// ([`retire`] or [`retire_in_place`]), `Retired` is recorded and the books
/// kept — `bookkeeping` runs once the source has left the library, since from
/// that moment the project is the moved copy. **The move is done here**;
/// merging the old copy away is housekeeping ([`Housekeeping`]), which needs
/// no data lock — the copy is hidden, and a record whose job is alive is left
/// alone by every reconcile.
///
/// Nothing is walked here: the merge that follows proves every removal entry
/// by entry, and absorbs what changed in the original meanwhile. 3.13 walked
/// the original and the moved copy first and kept the original whole on a
/// single difference — a dev server's log line — for ever.
pub(crate) fn set_aside(
    mut transaction: MoveTransaction,
    cleanup: &Cleanup,
    bookkeeping: impl FnOnce(),
) -> SetAside {
    use SetAside::Settled;
    let source = cleanup.source;
    cleanup.ticker.phase(JobPhase::SettingAside, 0);
    let has_identity = crate::core::project_info::pinfo_path(source).is_file();
    if (has_identity || !cleanup.residue_allowed)
        && let Err(error) = confirm_identity(source, cleanup.project_id, "original")
    {
        return Settled(SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        });
    }
    if let Err(error) = confirm_identity(cleanup.final_path, cleanup.project_id, "moved") {
        return Settled(SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        });
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
    // A retire that fails, for a test to reach.
    if let Err(error) = crate::util::faults::check("move:source-cleanup") {
        return Settled(SourceFate::KeptWhole {
            reason: format!("{error:#}"),
        });
    }
    let retired = transaction.old_copy_path();
    let outcome = match transaction.journal.retire {
        RetireStrategy::Rename => {
            // One rename, which fastf cannot count into: on an S3-style mount
            // it is a copy and a delete per object inside rclone, and took
            // minutes on a real R2 bucket. Say what it is waiting on.
            cleanup.ticker.update(|state| {
                state.current_file = "renaming it aside in one step — on a network mount this \
                                      can take a while"
                    .to_string();
            });
            crate::util::log::info("setting the original aside: one rename");
            retire(source, &retired)
        }
        RetireStrategy::InPlace => {
            crate::util::log::info(
                "setting the original aside in place: its PROJECT_INFO.md first",
            );
            retire_in_place(&transaction, cleanup)
        }
    };
    match outcome {
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
    SetAside::Retired(Box::new(transaction))
}

/// The in-place retire: the pointer beside the original, then its
/// `PROJECT_INFO.md` — only when it is still what the move copied, since the
/// moved copy's is the project's from here on. Once that file is gone the
/// folder is not a project; what is left in it is the merge's.
pub(crate) fn retire_in_place(transaction: &MoveTransaction, cleanup: &Cleanup) -> Retire {
    use crate::util::paths::{Presence, presence};
    if let Err(error) = transaction.write_pointer() {
        return Retire::KeptWhole(format!(
            "the note that it is being removed could not be written beside it ({error:#})"
        ));
    }
    let pinfo = crate::core::project_info::pinfo_path(cleanup.source);
    let relative = Path::new(crate::core::project_info::RESERVED_FILENAME);
    let recorded = cleanup.manifest.entry(relative);
    let unchanged = match (transactions::examine(&pinfo, relative), recorded) {
        (Ok(Some(found)), Some(recorded)) if transactions::agrees(recorded, &found) => true,
        // Only its time moved, or the moved copy's was rewritten since: the
        // text decides, without the two lines a move changes.
        (Ok(Some(_)), Some(_)) => match (
            fs::read_to_string(&pinfo),
            fs::read_to_string(crate::core::project_info::pinfo_path(cleanup.final_path)),
        ) {
            (Ok(here), Ok(there)) => merge::same_but_for_place(&here, &there),
            _ => false,
        },
        _ => false,
    };
    if !unchanged {
        return Retire::KeptWhole(
            "its PROJECT_INFO.md changed after the move copied it, so it is still the \
             original's"
                .to_string(),
        );
    }
    match crate::util::fs_retry::remove_file(&pinfo) {
        Ok(()) => Retire::Done,
        Err(error) => match presence(&pinfo) {
            Presence::Absent => Retire::Done,
            Presence::Present(_) => Retire::KeptWhole(format!(
                "its PROJECT_INFO.md could not be removed ({error})"
            )),
            Presence::Unknown(looked) => Retire::Unknown(format!(
                "removing its PROJECT_INFO.md reported '{error}', and it does not answer \
                 ({looked})"
            )),
        },
    }
}

/// Work left after a move or a delete that needs no data lock: removing an
/// old copy that is out of the library. Owned, so it outlives the lock and
/// the borrowed [`Cleanup`] it was made from.
#[derive(Debug)]
pub enum Housekeeping {
    /// A move's retired original, and the facts its merge is checked against
    /// again.
    OldCopy(Box<OldCopy>),
    /// A deleted project's hidden folder.
    Deleted(PathBuf),
    /// An old copy whose move left no record, removed where the project
    /// holds the same thing (`merge::remove_identical`).
    Recordless(Box<Recordless>),
    /// A deleted project emptied where it stands, on a cloud mount: only
    /// what its record lists goes.
    DeletedInPlace(Box<DeletedInPlace>),
}

/// A deleted project being emptied in place: its folder, its record, and
/// what the record lists.
#[derive(Debug)]
pub struct DeletedInPlace {
    pub(crate) folder: PathBuf,
    pub(crate) record_path: PathBuf,
    pub(crate) record: DeleteRecord,
}

/// An old copy with no record: where it is, where the project it held is
/// now, and the pointer that named it, if it was being emptied in place.
#[derive(Debug)]
pub struct Recordless {
    pub(crate) path: PathBuf,
    pub(crate) operation: String,
    pub(crate) moved: PathBuf,
    pub(crate) project_id: String,
    pub(crate) pointer: Option<PathBuf>,
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
    reappeared: bool,
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
            reappeared: cleanup.reappeared,
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
            Self::Recordless(old) => old.operation.clone(),
            Self::DeletedInPlace(deleted) => deleted.record.operation.clone(),
        }
    }

    /// The folder being removed.
    pub fn path(&self) -> PathBuf {
        match self {
            Self::OldCopy(old) => old.transaction.old_copy_path(),
            Self::Deleted(path) => path.clone(),
            Self::Recordless(old) => old.path.clone(),
            Self::DeletedInPlace(deleted) => deleted.folder.clone(),
        }
    }

    /// Where the project is now, for a move; nothing for a delete.
    pub fn moved_to(&self) -> Option<&Path> {
        match self {
            Self::OldCopy(old) => Some(&old.final_path),
            Self::Deleted(_) => None,
            Self::Recordless(old) => Some(&old.moved),
            Self::DeletedInPlace(_) => None,
        }
    }

    /// Do it. The merge re-checks everything it removes; see
    /// [`remove_retired`] and `core::removal`.
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
                    reappeared: old.reappeared,
                    ticker,
                };
                remove_retired(old.transaction, &cleanup)
            }
            Self::Recordless(old) => {
                ticker.phase(JobPhase::Removing, 0);
                match merge::remove_identical(&old.path, &old.moved, &old.project_id, ticker) {
                    Removal::Removed => {
                        if let Some(pointer) = &old.pointer {
                            let _ = crate::util::fs_retry::remove_file(pointer);
                        }
                        SourceFate::Removed { record_kept: None }
                    }
                    Removal::Leftover {
                        remaining,
                        reason,
                        kept_on_purpose,
                    } => SourceFate::Leftover {
                        path: old.path,
                        reason: format!(
                            "{remaining} {} left: {reason}",
                            if remaining == 1 { "entry" } else { "entries" }
                        ),
                        redundant: !kept_on_purpose,
                    },
                }
            }
            Self::DeletedInPlace(deleted) => {
                let deleted = *deleted;
                ticker.phase(JobPhase::Removing, deleted.record.entries.len());
                let listed = Listed(
                    deleted
                        .record
                        .entries
                        .iter()
                        .map(|entry| (entry.path.clone(), entry.kind))
                        .collect(),
                );
                match crate::core::removal::remove_tree_judged(
                    &deleted.folder,
                    &listed,
                    Purpose::Delete,
                    ticker,
                    None,
                ) {
                    Removal::Removed => {
                        // A cloud mount can put a removed folder back: the
                        // record waits out the settle, and removes it then.
                        if settling(&deleted.record.operation, &deleted.folder) {
                            return SourceFate::Settling;
                        }
                        match crate::util::fs_retry::remove_file(&deleted.record_path) {
                            Ok(()) => {
                                crate::core::records::remove(&deleted.record.operation);
                                SourceFate::Removed { record_kept: None }
                            }
                            Err(error) => SourceFate::Removed {
                                record_kept: Some(error.to_string()),
                            },
                        }
                    }
                    Removal::Leftover {
                        remaining,
                        reason,
                        kept_on_purpose,
                    } => SourceFate::Leftover {
                        path: deleted.folder,
                        reason: format!(
                            "{remaining} {} left: {reason}",
                            if remaining == 1 { "entry" } else { "entries" }
                        ),
                        redundant: !kept_on_purpose,
                    },
                }
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

/// From `Retired`: merge the old copy into the moved one and remove it
/// (`core::merge`), then the part a split rename left, then — once nothing of
/// the original is left, and a cloud mount has had its time to put anything
/// back — the record, and an in-place retire's pointer last.
pub(crate) fn remove_retired(transaction: MoveTransaction, cleanup: &Cleanup) -> SourceFate {
    let old = transaction.old_copy_path();
    match crate::util::paths::presence(&old) {
        crate::util::paths::Presence::Absent => {}
        // A mount that does not answer says nothing about the old copy: the
        // record stays, and the next pass finishes once it answers.
        crate::util::paths::Presence::Unknown(error) => {
            return SourceFate::Leftover {
                path: old,
                reason: format!("it does not answer ({error})"),
                redundant: true,
            };
        }
        crate::util::paths::Presence::Present(_) => {
            let residue =
                cleanup.residue_allowed || cleanup.reappeared || cleanup.published.is_none();
            let merged = merge::merge_remove(&Merge {
                old: &old,
                moved: cleanup.final_path,
                project_id: cleanup.project_id,
                manifest: cleanup.manifest,
                published: cleanup.published,
                policy: Policy::for_old_copy(transaction.published_at(), residue),
                record: Some(transaction.operation_dir()),
                ticker: cleanup.ticker,
            });
            if let Removal::Leftover {
                remaining,
                reason,
                kept_on_purpose,
            } = merged
            {
                return SourceFate::Leftover {
                    path: old,
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
    }
    if cleanup.split
        && let Some(fate) = remove_split_residue(&transaction, cleanup)
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
    let pointer =
        (transaction.journal.retire == RetireStrategy::InPlace).then(|| transaction.pointer_path());
    match transaction.remove() {
        Ok(()) => {
            // Last: the pointer is what names this record to a pass that
            // finds the old copy's folder without it.
            if let Some(pointer) = pointer {
                let _ = crate::util::fs_retry::remove_file(&pointer);
            }
            SourceFate::Removed { record_kept: None }
        }
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

/// What a split rename left at the original's path, merged as a residue:
/// only what the move recorded, unchanged, and the moved copy holds, and
/// nothing written into the moved copy. `None` when it is gone — or when it
/// is not the original's at all; otherwise why it is not, and the record
/// stays.
fn remove_split_residue(transaction: &MoveTransaction, cleanup: &Cleanup) -> Option<SourceFate> {
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
    // retire, and is left alone; with it, the merge keeps what is new.
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
    let ours = crate::core::project_info::read_metadata(&source)
        .ok()
        .flatten()
        .is_some_and(|metadata| metadata.id == cleanup.project_id);
    if !diff.is_residue() && !ours {
        return None;
    }
    match merge::merge_remove(&Merge {
        old: &source,
        moved: cleanup.final_path,
        project_id: cleanup.project_id,
        manifest: cleanup.manifest,
        published: cleanup.published,
        policy: Policy::Residue,
        record: Some(transaction.operation_dir()),
        ticker: cleanup.ticker,
    }) {
        Removal::Removed => None,
        Removal::Leftover {
            remaining,
            reason,
            kept_on_purpose,
        } => Some(SourceFate::Leftover {
            path: source,
            reason: format!(
                "part of the original is still there: {remaining} {} left: {reason}",
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
