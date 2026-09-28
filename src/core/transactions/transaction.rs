//! `MoveTransaction`: one record, from its first file to its removal.

use super::*;

/// Whether a move's copy has become the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Publication {
    Published,
    NotPublished,
    /// The filesystem did not answer the question.
    Unknown(String),
}

/// A claimed transaction directory owned by the current move.
#[derive(Debug)]
pub struct MoveTransaction {
    pub target_base: PathBuf,
    pub operation_dir: PathBuf,
    pub journal: MoveJournal,
}

impl MoveTransaction {
    pub fn begin(
        source_base: &Path,
        source_folder: &Path,
        target_base: &Path,
        target_folder: &Path,
        project_id: &str,
        operation: Operation,
    ) -> Result<Self> {
        crate::util::paths::require_real_directory(source_base, "source base")?;
        crate::util::paths::require_real_directory(target_base, "target base")?;
        validate_folder(source_folder, "source")?;
        validate_folder(target_folder, "target")?;

        let transaction_root = ensure_transaction_root(target_base)?;
        for _ in 0..OPERATION_RETRIES {
            let operation_id = next_operation_id();
            let operation_dir = transaction_root.join(&operation_id);
            match fs::create_dir(&operation_dir) {
                Ok(()) => {
                    // Where the record is, for a reconcile that walks only the
                    // configured bases (`core::records`).
                    crate::core::records::add(&crate::core::records::Entry {
                        operation: operation_id.clone(),
                        kind: match operation {
                            Operation::Move => crate::core::records::MOVE,
                            Operation::Copy => crate::core::records::COPY,
                        }
                        .to_string(),
                        project_id: project_id.to_string(),
                        record: operation_dir.clone(),
                        source_base: source_base.to_path_buf(),
                        source_folder: source_folder.to_path_buf(),
                        target_base: target_base.to_path_buf(),
                        target_folder: target_folder.to_path_buf(),
                        source_mount: crate::util::fs_kind::mount_identity(source_base),
                        ..Default::default()
                    });
                    let result = (|| -> Result<Self> {
                        crate::util::faults::check("move:after-transaction-create")?;
                        let journal = MoveJournal {
                            version: MOVE_VERSION,
                            operation_id,
                            project_id: project_id.to_string(),
                            source_base: source_base.to_path_buf(),
                            source_folder: source_folder.to_path_buf(),
                            target_folder: target_folder.to_path_buf(),
                            phase: MovePhase::Copying,
                            operation,
                            host: this_host(),
                            machine: crate::util::machine::id(),
                            legacy_cleanup: false,
                            in_place: true,
                            retire: match operation {
                                Operation::Move => RetireStrategy::for_base(source_base),
                                Operation::Copy => RetireStrategy::Rename,
                            },
                        };
                        write_record_file(&operation_dir.join(JOURNAL_FILE), &journal)
                            .context("writing Copying move journal")?;
                        Ok(Self {
                            target_base: target_base.to_path_buf(),
                            operation_dir: operation_dir.clone(),
                            journal,
                        })
                    })();
                    if result.is_err() {
                        let _ = crate::util::fs_retry::remove_dir_all(&operation_dir);
                        if let Some(name) = operation_dir.file_name().and_then(|name| name.to_str())
                        {
                            crate::core::records::remove(name);
                        }
                    }
                    return result;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("claiming move transaction {}", operation_dir.display())
                    });
                }
            }
        }
        bail!(
            "could not allocate a unique move operation under {}",
            transaction_root.display()
        )
    }

    /// Where the copy is made: the final path itself, or, for a record 3.12.0
    /// wrote, a folder under the transaction.
    pub fn staging_path(&self) -> PathBuf {
        if self.journal.in_place {
            self.final_path()
        } else {
            self.operation_dir.join(STAGING_DIR)
        }
    }

    /// The staging folder a 3.12.0 record renamed into place, where a cloud
    /// mount may still have put files after the rename.
    pub fn old_staging_path(&self) -> Option<PathBuf> {
        (!self.journal.in_place).then(|| self.operation_dir.join(STAGING_DIR))
    }

    pub fn final_path(&self) -> PathBuf {
        self.target_base.join(&self.journal.target_folder)
    }

    pub fn source_path(&self) -> PathBuf {
        self.journal.source_base.join(&self.journal.source_folder)
    }

    /// Where the source goes when it leaves the library.
    pub fn retired_path(&self) -> PathBuf {
        retired_path(&self.journal.source_base, &self.journal.operation_id)
    }

    /// The old copy once the original is out of the library: its retired
    /// folder, or — retired in place — the original's own path.
    pub fn old_copy_path(&self) -> PathBuf {
        match self.journal.retire {
            RetireStrategy::Rename => self.retired_path(),
            RetireStrategy::InPlace => self.source_path(),
        }
    }

    /// An in-place retire's pointer, beside the original.
    pub fn pointer_path(&self) -> PathBuf {
        pointer_path(&self.journal.source_base, &self.journal.operation_id)
    }

    /// Write the pointer an in-place retire leaves while it empties the
    /// original. Written once, never renamed; one already there is this
    /// move's own from a pass that stopped.
    pub fn write_pointer(&self) -> Result<()> {
        let path = self.pointer_path();
        let pointer = RetirePointer {
            version: 1,
            operation: self.journal.operation_id.clone(),
            project_id: self.journal.project_id.clone(),
            folder: self.journal.source_folder.clone(),
            target_base: self.target_base.clone(),
            target_folder: self.journal.target_folder.clone(),
        };
        match write_record_file(&path, &pointer) {
            Ok(()) => Ok(()),
            Err(error)
                if read_pointer(&path)
                    .is_ok_and(|found| found.operation == self.journal.operation_id) =>
            {
                let _ = error;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// When the copy was published: its published list's time. `None` for a
    /// record without one, or one that does not answer.
    pub fn published_at(&self) -> Option<SystemTime> {
        fs::symlink_metadata(self.operation_dir.join(PUBLISHED_FILE))
            .and_then(|metadata| metadata.modified())
            .ok()
    }

    /// The record's own folder.
    pub fn operation_dir(&self) -> &Path {
        &self.operation_dir
    }

    /// Record the destination as it is about to be published.
    pub fn write_published(&self, walk: &Walk) -> Result<MoveManifest> {
        let published = MoveManifest {
            version: MANIFEST_VERSION,
            entries: walk.entries.clone(),
        };
        published.validate()?;
        write_record_file(&self.operation_dir.join(PUBLISHED_FILE), &published)
            .context("writing the published record")?;
        Ok(published)
    }

    /// The destination as it was published, or `None` for a transaction that
    /// never recorded one (3.11 did not).
    pub fn read_published(&self) -> Result<Option<MoveManifest>> {
        let path = self.operation_dir.join(PUBLISHED_FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => {}
        }
        crate::util::paths::require_real_file(&path, "published record")?;
        let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let published: MoveManifest =
            serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
        published.validate()?;
        Ok(Some(published))
    }

    pub fn write_manifest(&self, manifest: &MoveManifest) -> Result<()> {
        manifest.validate()?;
        write_record_file(&self.operation_dir.join(MANIFEST_FILE), manifest)
            .context("writing move manifest")
    }

    pub fn read_manifest(&self) -> Result<MoveManifest> {
        read_manifest(&self.operation_dir)
    }

    /// Record `phase`: a marker file, created and never renamed (the doc on
    /// `PHASE_PREFIX` says why). A version-2 journal is also rewritten as version 3
    /// here, before its source is renamed, so an older binary cannot then find
    /// no source, call the move finished and leave the retired folder behind
    /// with nothing recording it.
    pub fn set_phase(&mut self, phase: MovePhase) -> Result<()> {
        let mut next = self.journal.clone();
        next.phase = phase;
        if self.journal.version < MOVE_VERSION {
            next.version = MOVE_VERSION;
            crate::util::atomic::write_json(&self.operation_dir.join(JOURNAL_FILE), &next)
                .with_context(|| format!("writing {:?} move phase", phase))?;
        }
        let marker = self.operation_dir.join(phase.marker_name());
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(file) => {
                let _ = file.sync_all();
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("recording {:?} move phase", phase));
            }
        }
        // The retire this comes before is a rename on another filesystem:
        // only a synced folder keeps a power loss from keeping the retire and
        // losing the phase.
        crate::core::move_cleanup::sync_dir(&self.operation_dir);
        self.journal = next;
        Ok(())
    }

    /// Record that the rename setting the original aside stopped part of the
    /// way: a later pass that finds the retired copy gone still knows that
    /// what is at the original's path is this move's residue. A file created
    /// once; a 3.13 binary reads only `phase.*` markers and ignores it.
    pub fn mark_split(&self) -> Result<()> {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.operation_dir.join(SPLIT_MARKER))
        {
            Ok(file) => {
                let _ = file.sync_all();
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error).context("recording that the retire stopped part of the way"),
        }
    }

    /// See [`Self::mark_split`].
    pub fn is_split(&self) -> bool {
        self.operation_dir.join(SPLIT_MARKER).is_file()
    }

    /// Record that the move paused before its publish — a mount that did not
    /// answer for [`crate::util::fs_retry::MOUNT_WAIT`] — keeping the copy
    /// made so far, which a resume adopts ([`adopt_staging`]).
    pub fn mark_paused(&self) -> Result<()> {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.operation_dir.join(PAUSED_MARKER))
        {
            Ok(file) => {
                let _ = file.sync_all();
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error).context("recording that the move paused"),
        }
    }

    /// Whether the move paused before its publish ([`Self::mark_paused`]).
    pub fn is_paused(&self) -> bool {
        self.operation_dir.join(PAUSED_MARKER).is_file()
    }

    pub fn claim_staging(&self) -> Result<PathBuf> {
        let staging = self.staging_path();
        fs::create_dir(&staging)
            .with_context(|| format!("claiming private staging {}", staging.display()))?;
        Ok(staging)
    }

    /// Whether the copy at the final path has been published: it holds a
    /// `PROJECT_INFO.md` that reads as this move's project.
    pub fn final_is_published(&self) -> bool {
        matches!(self.publication(), Publication::Published)
    }

    /// [`Self::final_is_published`], with the third answer a mount can give:
    /// it did not say. Removing an unpublished copy asks this, because a
    /// published one read through a mount that failed the read is the project,
    /// and must not be taken for fastf's own unfinished copy.
    pub fn publication(&self) -> Publication {
        let pinfo = crate::core::project_info::pinfo_path(&self.final_path());
        match crate::util::paths::presence(&pinfo) {
            crate::util::paths::Presence::Absent => return Publication::NotPublished,
            crate::util::paths::Presence::Unknown(error) => {
                return Publication::Unknown(error.to_string());
            }
            crate::util::paths::Presence::Present(_) => {}
        }
        match crate::core::project_info::read_metadata(&self.final_path()) {
            Ok(Some(metadata)) if metadata.id == self.journal.project_id => Publication::Published,
            Ok(_) => Publication::NotPublished,
            // A read that failed says nothing; a file that reads but does not
            // parse is a publish that never finished writing.
            Err(error) if error.chain().any(|cause| cause.is::<std::io::Error>()) => {
                Publication::Unknown(format!("{error:#}"))
            }
            Err(_) => Publication::NotPublished,
        }
    }

    /// Move into place any file a cloud mount uploaded to a 3.12.0 record's
    /// staging folder after that folder was renamed away. Each one the move
    /// recorded, at its recorded size, is renamed over whatever the final path
    /// holds there — the same bytes, and the one durable copy if the mount's
    /// cache was lying — and **nothing is ever deleted**: anything else stays,
    /// and the record with it. Returns what was moved and what was left.
    ///
    /// Without the manifest (the same mount may have misplaced that too), a
    /// stray is moved where the final path holds nothing, or a file with the
    /// same bytes: the staging folder was fastf's own, so what is in it is
    /// what the copy wrote, and neither can take anything away — and the
    /// same bytes at the final path may be only the mount's cache, while the
    /// stray is what the remote has.
    pub fn sweep_strays(&self, manifest: Option<&MoveManifest>) -> Result<(usize, Vec<String>)> {
        let Some(staging) = self.old_staging_path() else {
            return Ok((0, Vec::new()));
        };
        match crate::util::paths::presence(&staging) {
            crate::util::paths::Presence::Absent => return Ok((0, Vec::new())),
            crate::util::paths::Presence::Unknown(error) => {
                bail!(
                    "the old staging folder {} does not answer ({error})",
                    staging.display()
                )
            }
            crate::util::paths::Presence::Present(_) => {}
        }
        let final_path = self.final_path();
        let walk = Walk::of(&staging, "old staging")?;
        let mut moved = 0;
        let mut left = Vec::new();
        for entry in walk
            .entries
            .iter()
            .filter(|entry| entry.kind != ManifestKind::Directory)
        {
            let placeable = match manifest {
                Some(manifest) => manifest.entry(&entry.path).is_some_and(|recorded| {
                    recorded.kind == entry.kind
                        && recorded.bytes == entry.bytes
                        && recorded.link_target == entry.link_target
                }),
                None => {
                    let there = final_path.join(&entry.path);
                    match fs::symlink_metadata(&there) {
                        Err(_) => true,
                        Ok(metadata) => {
                            entry.kind == ManifestKind::File
                                && metadata.file_type().is_file()
                                && metadata.len() == entry.bytes
                                && same_bytes(&staging.join(&entry.path), &there)
                        }
                    }
                }
            };
            if !placeable {
                left.push(format!(
                    "{}: {}",
                    entry.path.display(),
                    if manifest.is_some() {
                        "not as the move recorded it"
                    } else {
                        "the moved copy holds something different there, and the record of \
                         what was moved is missing"
                    }
                ));
                continue;
            }
            let destination =
                match crate::util::paths::contained_destination(&final_path, &entry.path) {
                    Ok(destination) => destination,
                    Err(error) => {
                        left.push(format!("{}: {error:#}", entry.path.display()));
                        continue;
                    }
                };
            if let Some(parent) = destination.parent()
                && let Err(error) = fs::create_dir_all(parent)
            {
                left.push(format!("{}: {error}", entry.path.display()));
                continue;
            }
            match crate::util::fs_retry::rename(&staging.join(&entry.path), &destination) {
                Ok(()) => moved += 1,
                Err(error) => left.push(format!("{}: {error}", entry.path.display())),
            }
        }
        for problem in &walk.problems {
            left.push(format!("{}: {}", problem.path.display(), problem.problem));
        }
        // Empty folders go; a folder something was left in stays.
        for entry in walk
            .entries
            .iter()
            .rev()
            .filter(|entry| entry.kind == ManifestKind::Directory)
        {
            let _ = fs::remove_dir(staging.join(&entry.path));
        }
        if left.is_empty() {
            let _ = fs::remove_dir(&staging);
        }
        Ok((moved, left))
    }

    /// Remove only this exclusively-created operation directory — and, for a
    /// copy made in its final place that was never published, that copy. A
    /// published one (its `PROJECT_INFO.md` reads as this project, whatever
    /// phase the record reached) is left exactly as it is.
    pub fn remove(self) -> Result<()> {
        if self.journal.in_place && self.journal.phase == MovePhase::Copying {
            let staging = self.final_path();
            let there = match crate::util::paths::presence(&staging) {
                crate::util::paths::Presence::Absent => false,
                crate::util::paths::Presence::Present(_) => true,
                crate::util::paths::Presence::Unknown(error) => {
                    bail!(
                        "the copy at {} does not answer ({error})",
                        staging.display()
                    )
                }
            };
            let unpublished = there
                && match self.publication() {
                    Publication::NotPublished => true,
                    Publication::Published => false,
                    Publication::Unknown(error) => bail!(
                        "cannot tell whether the copy at {} is published ({error})",
                        staging.display()
                    ),
                };
            if unpublished {
                crate::util::paths::require_real_directory(&staging, "unpublished copy")?;
                if let crate::core::removal::Removal::Leftover { reason, .. } =
                    crate::core::removal::remove_tree(
                        &staging,
                        None,
                        crate::core::removal::Purpose::Delete,
                        Ticker::none(),
                    )
                {
                    bail!(
                        "removing the unpublished copy at {}: {reason}",
                        staging.display()
                    );
                }
            }
        }
        // A published record's old staging folder may hold the one durable
        // copy of a file a cloud mount uploaded there late; those are swept
        // into place first (`sweep_strays`), and whatever that could not take
        // keeps the record.
        if let Some(staging) = self.old_staging_path()
            && self.journal.phase.rank() >= MovePhase::CleanupPending.rank()
            && Walk::of(&staging, "old staging").is_ok_and(|walk| {
                walk.entries
                    .iter()
                    .any(|e| e.kind != ManifestKind::Directory)
            })
        {
            bail!(
                "refusing to remove the record while its old staging folder {} still holds files",
                staging.display()
            );
        }
        let expected_root = transaction_root(&self.target_base);
        if self.operation_dir.parent() != Some(expected_root.as_path())
            || self
                .operation_dir
                .file_name()
                .and_then(|name| name.to_str())
                != Some(self.journal.operation_id.as_str())
        {
            bail!(
                "refusing to remove transaction outside its owned location: {}",
                self.operation_dir.display()
            );
        }
        crate::util::paths::require_real_directory(&self.operation_dir, "move transaction")?;
        // A cloud mount refuses to remove a folder whose files it is still
        // uploading — the record's last files were written seconds ago — and
        // says so with an I/O error that clears once they are up.
        remove_record_folder(std::thread::sleep, &self.operation_dir, || {
            crate::util::fs_retry::remove_dir_all(&self.operation_dir)
        })
        .with_context(|| format!("removing move transaction {}", self.operation_dir.display()))?;
        crate::core::records::remove(&self.journal.operation_id);
        Ok(())
    }
}

/// A record's folder, removed through a mount's uploads ([`through_uploads`])
/// and asked again by class inside that; pausing through `sleep`. **What a
/// mount still uploading answers is the outer loop's alone to wait out**, a
/// second apart for twenty seconds: asked again by class as well, inside each
/// of those tries, the two waits multiply into four minutes.
pub(super) fn remove_record_folder(
    sleep: impl FnMut(std::time::Duration),
    folder: &Path,
    mut remove: impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    use crate::util::fs_retry::{Next, by_class, mount_wait, still_uploading, with_retry_judged};
    let sleep = std::cell::RefCell::new(sleep);
    through_uploads(
        |pause| (sleep.borrow_mut())(pause),
        || {
            with_retry_judged(
                |pause| (sleep.borrow_mut())(pause),
                folder,
                mount_wait(),
                |error| match still_uploading(error) {
                    Next::Again => Next::Stop,
                    _ => by_class(error),
                },
                &mut remove,
            )
        },
    )
}

/// Ask `op` again for as long as a mount is still uploading
/// (`fs_retry::still_uploading`), on `schedule::RECORD_UPLOAD`; pausing
/// through `sleep`.
pub(super) fn through_uploads<T>(
    sleep: impl FnMut(std::time::Duration),
    op: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    use crate::util::fs_retry::{Retry, schedule, still_uploading};
    Retry::on(schedule::RECORD_UPLOAD).run_sleeping(
        sleep,
        still_uploading,
        |_, _| {},
        |_| Ok(()),
        op,
    )
}

/// Whether two files hold the same bytes, read side by side; `false` when
/// either cannot be read.
pub(crate) fn same_bytes(one: &Path, other: &Path) -> bool {
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

/// Write one of the record's files **once, in place**: created new and
/// synced, never renamed into position. The folder is fastf's own and was
/// created exclusively a moment ago, so a name is free; and on a cloud mount
/// a rename is what a background upload can misplace. A crash mid-write
/// leaves a file that does not parse, which recovery reports and never acts
/// on — unless it is a `move.json` alone in its record (`is_bare_record`),
/// which is removed.
pub(crate) fn write_record_file<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let raw = serde_json::to_string_pretty(value)
        .with_context(|| format!("serializing {}", path.display()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(raw.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", path.display()))?;
    Ok(())
}
