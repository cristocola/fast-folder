//! The journal (`move.json`): its schema, where a record lives, and how one is
//! read.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovePhase {
    Copying,
    ReadyToCommit,
    /// Published; the source is still at its path until it is retired.
    CleanupPending,
    /// The source has been renamed out of the library to its retired name;
    /// what is left is removing that. Written *after* the rename, so a crash
    /// between the two leaves `CleanupPending` with the retired folder present,
    /// which recovery reads the same way.
    Retired,
}

/// What the transaction is for. A copy keeps its source, so recovery must never
/// take a copy's transaction as licence to remove one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    /// Version-2 journals carry no operation. Only moves ever advanced past
    /// `Copying` then, so reading them as moves is what they were.
    #[default]
    Move,
    Copy,
}

/// The complete move journal schema. Unknown fields are rejected so a future
/// journal is never accidentally interpreted with older recovery semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveJournal {
    pub version: u32,
    pub operation_id: String,
    pub project_id: String,
    pub source_base: PathBuf,
    pub source_folder: PathBuf,
    pub target_folder: PathBuf,
    pub phase: MovePhase,
    #[serde(default)]
    pub operation: Operation,
    /// The machine that began it, by name, for a message. A base can be
    /// reached from more than one machine, and the source path a journal
    /// names means something else on the other one; recovery there only
    /// reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The same machine by its operating system's id (`util::machine`), which
    /// is what is compared: a hostname changes with DHCP or a rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// Begun by a build that removed a source where it stood (3.11 and
    /// older), so its source may already be partly removed. Set when a
    /// version-2 journal is read and **carried** when it is rewritten as
    /// version 3 — otherwise a retire that failed after the rewrite would
    /// leave a half-removed source that the next pass, seeing version 3,
    /// demands whole.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_cleanup: bool,
    /// The copy is made at its final path, `PROJECT_INFO.md` last (3.12.1
    /// and later). A record without it staged under the transaction and
    /// renamed into place, and recovery treats that staging the old way.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_place: bool,
    /// How the original leaves the library once the copy is published
    /// ([`RetireStrategy`]). A 3.13 binary refuses a record that says
    /// `in-place` (`deny_unknown_fields`), which is the point: it would finish
    /// it the rename way.
    #[serde(default, skip_serializing_if = "RetireStrategy::is_rename")]
    pub retire: RetireStrategy,
}

/// How an original leaves the library after its copy is published.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetireStrategy {
    /// Renamed aside in one step, to `.fastf-moved-<operation>`, then
    /// removed: one rename either happens or does not.
    #[default]
    Rename,
    /// Emptied where it stands, `PROJECT_INFO.md` first, a pointer beside it
    /// (`.fastf-moved-<operation>.json`) naming the record. On a cloud mount
    /// renaming a folder is a copy and a delete for every object in it —
    /// twelve minutes for one web project on an S3 bucket — and moves uploads
    /// still in flight to the old path; this is a third of the requests and
    /// renames nothing.
    InPlace,
}

impl RetireStrategy {
    fn is_rename(&self) -> bool {
        *self == Self::Rename
    }

    /// The strategy for an original in `source_base`: in place on a mount
    /// whose folder renames are not one step (rclone, and a FUSE mount fastf
    /// does not know), a rename everywhere else.
    pub fn for_base(source_base: &Path) -> Self {
        match crate::util::fs_kind::of(source_base) {
            crate::util::fs_kind::FsKind::Rclone | crate::util::fs_kind::FsKind::OtherFuse => {
                Self::InPlace
            }
            _ => Self::Rename,
        }
    }
}

/// What an in-place retire leaves beside the original's folder while it is
/// emptied: which move it is, and where its record lives, so an old copy
/// whose `PROJECT_INFO.md` is already gone is never a stranger's folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetirePointer {
    pub version: u32,
    pub operation: String,
    pub project_id: String,
    pub folder: PathBuf,
    pub target_base: PathBuf,
    pub target_folder: PathBuf,
}

/// Where an in-place retire's pointer lives.
pub fn pointer_path(source_base: &Path, operation_id: &str) -> PathBuf {
    source_base.join(format!("{RETIRED_PREFIX}{operation_id}.json"))
}

/// The operation an in-place retire's pointer is named by, if `name` is one.
pub fn pointer_operation(name: &str) -> Option<&str> {
    name.strip_prefix(RETIRED_PREFIX)
        .and_then(|rest| rest.strip_suffix(".json"))
        .filter(|operation| is_operation_id(operation))
}

/// Make way for a move or copy landing at `path`: nothing there, or an
/// empty folder — which a cloud mount leaves behind for a while after a
/// folder is removed (rclone re-creates directory markers), and which holds
/// nothing to lose — is removed. Anything else refuses, and says what it is
/// when it is an earlier move's old copy still being emptied in place.
pub fn clear_target(path: &Path) -> Result<()> {
    use crate::util::paths::{Presence, presence};
    match presence(path) {
        Presence::Absent => return Ok(()),
        Presence::Unknown(error) => {
            bail!("move target {} does not answer ({error})", path.display());
        }
        Presence::Present(metadata) if metadata.file_type().is_dir() => {
            let empty = fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none());
            if empty && fs::remove_dir(path).is_ok() {
                return Ok(());
            }
        }
        Presence::Present(_) => {}
    }
    if let (Some(base), Some(folder)) = (path.parent(), path.file_name())
        && let Ok(entries) = fs::read_dir(base)
    {
        let emptying = entries.flatten().any(|entry| {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            (pointer_operation(name).is_some()
                && read_pointer(&entry.path()).is_ok_and(|pointer| pointer.folder == folder))
                || (crate::core::move_cleanup::deleted_record_operation(name).is_some()
                    && crate::core::move_cleanup::read_delete_record(&entry.path())
                        .is_ok_and(|record| record.folder == folder))
        });
        if emptying {
            bail!(
                "move target already exists: {} is a folder fastf is still removing (an \
                 earlier move's old copy, or a deleted project); try again once it is gone",
                path.display()
            );
        }
    }
    bail!("move target already exists: {}", path.display())
}

/// Read an in-place retire's pointer.
pub fn read_pointer(path: &Path) -> Result<RetirePointer> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let pointer: RetirePointer =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    validate_operation_id(&pointer.operation)?;
    validate_folder(&pointer.folder, "pointer")?;
    Ok(pointer)
}

impl MoveJournal {
    /// Whether the source may be a residue of an in-place removal.
    pub fn source_may_be_partial(&self) -> bool {
        self.legacy_cleanup
    }

    /// Whether this machine began the operation: by machine id when both
    /// sides have one, else by name. A version-2 journal names no machine and
    /// is taken as this one's, which is how 3.11 treated it.
    pub fn is_from_this_host(&self) -> bool {
        if let (Some(recorded), Some(here)) = (&self.machine, crate::util::machine::id()) {
            return *recorded == here;
        }
        match (&self.host, this_host()) {
            (Some(recorded), Some(here)) => *recorded == here,
            _ => true,
        }
    }
}

/// This machine's name, as the journal records it.
pub(crate) fn this_host() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buffer = [0_u8; 256];
        // SAFETY: the buffer is valid for its whole length, and the length
        // passed leaves room for the terminating NUL.
        let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len() - 1) };
        if status != 0 {
            return None;
        }
        let end = buffer.iter().position(|&byte| byte == 0)?;
        std::str::from_utf8(&buffer[..end])
            .ok()
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|name| !name.is_empty())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

impl MovePhase {
    /// The order phases are reached in.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::Copying => 0,
            Self::ReadyToCommit => 1,
            Self::CleanupPending => 2,
            Self::Retired => 3,
        }
    }

    pub(super) fn marker_name(self) -> String {
        format!("{PHASE_PREFIX}{self:?}")
    }

    fn from_marker(name: &str) -> Option<Self> {
        match name.strip_prefix(PHASE_PREFIX)? {
            "Copying" => Some(Self::Copying),
            "ReadyToCommit" => Some(Self::ReadyToCommit),
            "CleanupPending" => Some(Self::CleanupPending),
            "Retired" => Some(Self::Retired),
            _ => None,
        }
    }
}

pub fn ensure_transaction_root(target_base: &Path) -> Result<PathBuf> {
    crate::util::paths::require_real_directory(target_base, "target base")?;
    let root = target_base.join(TRANSACTIONS_DIR);
    match fs::create_dir(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("creating transaction root {}", root.display()));
        }
    }
    crate::util::paths::require_real_directory(&root, "transaction root")?;
    Ok(root)
}

pub fn transaction_root(target_base: &Path) -> PathBuf {
    target_base.join(TRANSACTIONS_DIR)
}

/// `<source base>/.fastf-moved-<operation>`: derived, never stored.
pub fn retired_path(source_base: &Path, operation_id: &str) -> PathBuf {
    source_base.join(format!("{RETIRED_PREFIX}{operation_id}"))
}

/// The operation a retired folder's name carries, if it is one fastf writes.
pub fn retired_operation(name: &str) -> Option<&str> {
    name.strip_prefix(RETIRED_PREFIX)
        .filter(|operation| is_operation_id(operation))
}

/// Whether `text` is shaped like an operation id: hex digits and dashes.
/// fastf's hidden folders are recognised by this, never by their prefix
/// alone — a project may be named `fastf-deleted-Scenes`, and a case-only
/// rename of it passes through `.fastf-deleted-Scenes.fastf-case`.
pub fn is_operation_id(text: &str) -> bool {
    validate_operation_id(text).is_ok()
}

pub fn read_journal(operation_dir: &Path) -> Result<MoveJournal> {
    let path = operation_dir.join(JOURNAL_FILE);
    crate::util::paths::require_real_file(&path, "move journal")?;
    let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let journal: MoveJournal =
        serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
    if !(MOVE_VERSION_OLDEST..=MOVE_VERSION).contains(&journal.version) {
        bail!(
            "unsupported move journal version {} at {}",
            journal.version,
            path.display()
        );
    }
    let mut journal = journal;
    if journal.version < 3 {
        if journal.phase == MovePhase::Retired
            || journal.host.is_some()
            || journal.machine.is_some()
            || journal.legacy_cleanup
            || journal.in_place
            || journal.operation != Operation::Move
        {
            bail!(
                "a version-{} move journal cannot record what only version 3 has: {}",
                journal.version,
                path.display()
            );
        }
        journal.legacy_cleanup = true;
    }
    validate_operation_id(&journal.operation_id)?;
    validate_folder(&journal.source_folder, "source")?;
    validate_folder(&journal.target_folder, "target")?;
    if !journal.source_base.is_absolute() {
        bail!("move journal source base is not absolute");
    }
    let directory_name = operation_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("transaction directory has no UTF-8 operation id")?;
    if directory_name != journal.operation_id {
        bail!(
            "move journal operation id '{}' does not match directory '{}'",
            journal.operation_id,
            directory_name
        );
    }
    // The phase is the highest marker present, or the journal's own for a
    // record written before markers existed.
    if let Ok(entries) = fs::read_dir(operation_dir) {
        for entry in entries.flatten() {
            if let Some(phase) = entry.file_name().to_str().and_then(MovePhase::from_marker)
                && phase.rank() > journal.phase.rank()
            {
                journal.phase = phase;
            }
        }
    }
    Ok(journal)
}

/// A transaction directory that holds nothing a move writes after its
/// journal: no manifest, no marker, nothing but at most a `move.json` that did
/// not finish. A move makes its copy only once its journal is written, so one
/// killed this early left nothing anywhere else either.
pub fn is_bare_record(operation_dir: &Path) -> bool {
    // fastf names its records; anything else is somebody's, and a journal
    // that reads as JSON but not as a valid record is reported, never removed.
    if !operation_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_operation_id)
    {
        return false;
    }
    let Ok(entries) = fs::read_dir(operation_dir) else {
        return false;
    };
    for entry in entries {
        match entry {
            Ok(entry) if entry.file_name() == JOURNAL_FILE => {}
            _ => return false,
        }
    }
    match fs::read(operation_dir.join(JOURNAL_FILE)) {
        Ok(raw) => serde_json::from_slice::<serde_json::Value>(&raw).is_err(),
        Err(error) => crate::util::paths::is_absence(&error),
    }
}

pub fn read_manifest(operation_dir: &Path) -> Result<MoveManifest> {
    let path = operation_dir.join(MANIFEST_FILE);
    crate::util::paths::require_real_file(&path, "move manifest")?;
    let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: MoveManifest =
        serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn transaction_from_journal(
    target_base: &Path,
    operation_dir: &Path,
    journal: MoveJournal,
) -> MoveTransaction {
    MoveTransaction {
        target_base: target_base.to_path_buf(),
        operation_dir: operation_dir.to_path_buf(),
        journal,
    }
}

pub(super) fn validate_folder(path: &Path, label: &str) -> Result<()> {
    let mut components = path.components();
    let Some(Component::Normal(name)) = components.next() else {
        bail!("move {label} folder must be one safe path component");
    };
    if components.next().is_some() || name.is_empty() {
        bail!("move {label} folder must be one safe path component");
    }
    if name == TRANSACTIONS_DIR {
        bail!("move {label} folder uses the reserved transaction name");
    }
    Ok(())
}

fn validate_operation_id(operation_id: &str) -> Result<()> {
    if operation_id.is_empty()
        || !operation_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        bail!("invalid move operation id '{operation_id}'");
    }
    Ok(())
}

pub(crate) fn next_operation_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = OPERATION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:x}-{:x}-{counter:x}", std::process::id())
}
