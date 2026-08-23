//! Scoped v2 move transactions.
//!
//! A transaction lives below the target base at
//! `.fastf-transactions/<operation-id>/`.  The journal deliberately contains
//! no target-base or staging path: both are derived from that owned location.
//! Source and target folder names are validated single path components before
//! they are ever joined to a base.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::assets::{self, Progress};

pub const TRANSACTIONS_DIR: &str = ".fastf-transactions";
pub const JOURNAL_FILE: &str = "move.json";
pub(crate) const MANIFEST_FILE: &str = "manifest.json";
pub const STAGING_DIR: &str = "staging";

const MOVE_VERSION: u32 = 2;
const MANIFEST_VERSION: u32 = 1;
const OPERATION_RETRIES: usize = 64;
const COPY_BUFFER_BYTES: usize = 1024 * 1024;

static OPERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovePhase {
    Copying,
    ReadyToCommit,
    CleanupPending,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManifestKind {
    File,
    Directory,
}

/// A lossless filesystem modification timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModifiedTime {
    before_epoch: bool,
    seconds: u64,
    nanoseconds: u32,
}

impl ModifiedTime {
    fn from_system_time(value: SystemTime) -> Self {
        match value.duration_since(UNIX_EPOCH) {
            Ok(duration) => Self {
                before_epoch: false,
                seconds: duration.as_secs(),
                nanoseconds: duration.subsec_nanos(),
            },
            Err(error) => {
                let duration = error.duration();
                Self {
                    before_epoch: true,
                    seconds: duration.as_secs(),
                    nanoseconds: duration.subsec_nanos(),
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    /// Native relative path. It is never converted through a lossy display
    /// string while scanning, comparing, or copying.
    pub path: PathBuf,
    pub kind: ManifestKind,
    pub bytes: u64,
    pub source_modified: ModifiedTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveManifest {
    version: u32,
    pub entries: Vec<ManifestEntry>,
}

impl MoveManifest {
    /// Scan exactly once before copying. Unsupported entries fail the entire
    /// move; links and special files are never followed or silently omitted.
    pub fn scan(root: &Path) -> Result<Self> {
        assets::require_real_directory(root, "move source")?;
        let mut entries = Vec::new();
        scan_inner(root, root, &mut entries)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = Self {
            version: MANIFEST_VERSION,
            entries,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != MANIFEST_VERSION {
            bail!(
                "unsupported move manifest version {} (expected {})",
                self.version,
                MANIFEST_VERSION
            );
        }
        let mut seen = HashSet::new();
        for entry in &self.entries {
            crate::util::paths::require_native_relative(&entry.path, "move manifest path")?;
            if !seen.insert(entry.path.clone()) {
                bail!(
                    "move manifest contains duplicate path {}",
                    entry.path.display()
                );
            }
            if entry.kind == ManifestKind::Directory && entry.bytes != 0 {
                bail!(
                    "move manifest directory has a non-zero byte length: {}",
                    entry.path.display()
                );
            }
        }
        Ok(())
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| entry.kind == ManifestKind::File)
            .map(|entry| entry.bytes)
            .sum()
    }

    pub fn total_files(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.kind == ManifestKind::File)
            .count()
    }

    /// Verify exact relative paths, entry types, and regular-file lengths.
    /// Destination modification times are intentionally not compared: fastf
    /// promises content topology and byte lengths, not metadata preservation.
    pub fn verify_destination(&self, destination: &Path) -> Result<()> {
        let actual = Self::scan(destination)?;
        if self.content_projection() != actual.content_projection() {
            bail!(
                "move verification failed: destination path/type/size manifest differs from source"
            );
        }
        Ok(())
    }

    /// Re-scan the source and compare path, type, length, and modification time.
    pub fn verify_source_unchanged(&self, source: &Path) -> Result<()> {
        let actual = Self::scan(source)?;
        if self != &actual {
            bail!("move source changed while it was being copied");
        }
        Ok(())
    }

    /// Used by recovery before deleting a source: compare exact path/type/size
    /// manifests on both sides and also require the source to still match the
    /// original pre-copy metadata snapshot.
    pub fn verify_recovery_pair(&self, source: &Path, destination: &Path) -> Result<()> {
        self.verify_source_unchanged(source)?;
        self.verify_destination(destination)
    }

    fn content_projection(&self) -> Vec<(&Path, ManifestKind, u64)> {
        self.entries
            .iter()
            .map(|entry| (entry.path.as_path(), entry.kind, entry.bytes))
            .collect()
    }
}

fn scan_inner(root: &Path, current: &Path, entries: &mut Vec<ManifestEntry>) -> Result<()> {
    scan_at(root, current, 0, entries)
}

fn scan_at(
    root: &Path,
    current: &Path,
    depth: usize,
    entries: &mut Vec<ManifestEntry>,
) -> Result<()> {
    if depth >= crate::util::paths::MAX_WALK_DEPTH {
        return Err(crate::util::paths::too_deep(current));
    }
    let children = fs::read_dir(current)
        .with_context(|| format!("reading move source {}", current.display()))?;
    for child in children {
        let child = child?;
        let path = child.path();
        let file_type = child
            .file_type()
            .with_context(|| format!("classifying {}", path.display()))?;
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("deriving relative path for {}", path.display()))?
            .to_path_buf();
        crate::util::paths::require_native_relative(&relative, "move manifest path")?;

        if file_type.is_symlink() {
            bail!(
                "cannot move '{}': links are not supported by cross-drive moves",
                relative.display()
            );
        }
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("reading metadata for {}", path.display()))?;
        let modified = metadata
            .modified()
            .with_context(|| format!("reading modification time for {}", path.display()))?;
        if file_type.is_dir() {
            entries.push(ManifestEntry {
                path: relative,
                kind: ManifestKind::Directory,
                bytes: 0,
                source_modified: ModifiedTime::from_system_time(modified),
            });
            scan_inner(root, &path, entries)?;
        } else if file_type.is_file() {
            entries.push(ManifestEntry {
                path: relative,
                kind: ManifestKind::File,
                bytes: metadata.len(),
                source_modified: ModifiedTime::from_system_time(modified),
            });
        } else {
            bail!(
                "cannot move '{}': special filesystem entries are not supported",
                relative.display()
            );
        }
    }
    Ok(())
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
    ) -> Result<Self> {
        assets::require_real_directory(source_base, "source base")?;
        assets::require_real_directory(target_base, "target base")?;
        validate_folder(source_folder, "source")?;
        validate_folder(target_folder, "target")?;

        let transaction_root = ensure_transaction_root(target_base)?;
        for _ in 0..OPERATION_RETRIES {
            let operation_id = next_operation_id();
            let operation_dir = transaction_root.join(&operation_id);
            match fs::create_dir(&operation_dir) {
                Ok(()) => {
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
                        };
                        crate::util::atomic::write_json(
                            &operation_dir.join(JOURNAL_FILE),
                            &journal,
                        )
                        .context("writing Copying move journal")?;
                        Ok(Self {
                            target_base: target_base.to_path_buf(),
                            operation_dir: operation_dir.clone(),
                            journal,
                        })
                    })();
                    if result.is_err() {
                        let _ = crate::util::fs_retry::remove_dir_all(&operation_dir);
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

    pub fn staging_path(&self) -> PathBuf {
        self.operation_dir.join(STAGING_DIR)
    }

    pub fn final_path(&self) -> PathBuf {
        self.target_base.join(&self.journal.target_folder)
    }

    pub fn source_path(&self) -> PathBuf {
        self.journal.source_base.join(&self.journal.source_folder)
    }

    pub fn write_manifest(&self, manifest: &MoveManifest) -> Result<()> {
        manifest.validate()?;
        crate::util::atomic::write_json(&self.operation_dir.join(MANIFEST_FILE), manifest)
            .context("writing move manifest")
    }

    pub fn read_manifest(&self) -> Result<MoveManifest> {
        read_manifest(&self.operation_dir)
    }

    pub fn set_phase(&mut self, phase: MovePhase) -> Result<()> {
        let mut next = self.journal.clone();
        next.phase = phase;
        crate::util::atomic::write_json(&self.operation_dir.join(JOURNAL_FILE), &next)
            .with_context(|| format!("writing {:?} move phase", phase))?;
        self.journal = next;
        Ok(())
    }

    pub fn claim_staging(&self) -> Result<PathBuf> {
        let staging = self.staging_path();
        fs::create_dir(&staging)
            .with_context(|| format!("claiming private staging {}", staging.display()))?;
        Ok(staging)
    }

    /// Remove only this exclusively-created operation directory.
    pub fn remove(self) -> Result<()> {
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
        assets::require_real_directory(&self.operation_dir, "move transaction")?;
        crate::util::fs_retry::remove_dir_all(&self.operation_dir)
            .with_context(|| format!("removing move transaction {}", self.operation_dir.display()))
    }
}

pub fn ensure_transaction_root(target_base: &Path) -> Result<PathBuf> {
    assets::require_real_directory(target_base, "target base")?;
    let root = target_base.join(TRANSACTIONS_DIR);
    match fs::create_dir(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("creating transaction root {}", root.display()));
        }
    }
    assets::require_real_directory(&root, "transaction root")?;
    Ok(root)
}

pub fn transaction_root(target_base: &Path) -> PathBuf {
    target_base.join(TRANSACTIONS_DIR)
}

pub fn read_journal(operation_dir: &Path) -> Result<MoveJournal> {
    let path = operation_dir.join(JOURNAL_FILE);
    crate::util::paths::require_real_file(&path, "move journal")?;
    let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let journal: MoveJournal =
        serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
    if journal.version != MOVE_VERSION {
        bail!(
            "unsupported move journal version {} at {}",
            journal.version,
            path.display()
        );
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
    Ok(journal)
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

/// Copy from a manifest with one reusable bounded buffer. Files are written
/// directly into private staging, so no sibling `.part` convention exists.
pub fn copy_to_staging(
    manifest: &MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<()> {
    assets::require_real_directory(source, "move source")?;
    assets::require_real_directory(staging, "move staging")?;
    manifest.validate()?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];

    for entry in &manifest.entries {
        if cancel.load(Ordering::Relaxed) {
            bail!("move cancelled");
        }
        let source_path = source.join(&entry.path);
        let destination_path = staging.join(&entry.path);
        match entry.kind {
            ManifestKind::Directory => {
                fs::create_dir_all(&destination_path).with_context(|| {
                    format!("creating staging directory {}", destination_path.display())
                })?;
            }
            ManifestKind::File => {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                let current = entry_from_path(&source_path, &entry.path)?;
                if &current != entry {
                    bail!(
                        "move source changed before copying {}",
                        entry.path.display()
                    );
                }
                if let Ok(mut state) = progress.lock() {
                    state.current_file = entry.path.to_string_lossy().into_owned();
                    state.touch();
                }
                let mut reader = fs::File::open(&source_path)
                    .with_context(|| format!("opening {}", source_path.display()))?;
                let mut writer = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination_path)
                    .with_context(|| format!("creating {}", destination_path.display()))?;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        bail!("move cancelled");
                    }
                    crate::util::faults::check("move:mid-copy")?;
                    let count = reader
                        .read(&mut buffer)
                        .with_context(|| format!("reading {}", source_path.display()))?;
                    if count == 0 {
                        break;
                    }
                    writer
                        .write_all(&buffer[..count])
                        .with_context(|| format!("writing {}", destination_path.display()))?;
                    if let Ok(mut state) = progress.lock() {
                        state.copied_bytes = state.copied_bytes.saturating_add(count as u64);
                        state.touch();
                    }
                }
                writer
                    .flush()
                    .with_context(|| format!("flushing {}", destination_path.display()))?;
                writer
                    .sync_all()
                    .with_context(|| format!("syncing {}", destination_path.display()))?;
                if let Ok(mut state) = progress.lock() {
                    state.done_files += 1;
                    state.touch();
                }
            }
        }
    }
    Ok(())
}

fn entry_from_path(path: &Path, relative: &Path) -> Result<ManifestEntry> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        bail!(
            "move source entry is no longer a regular file: {}",
            path.display()
        );
    }
    Ok(ManifestEntry {
        path: relative.to_path_buf(),
        kind: ManifestKind::File,
        bytes: metadata.len(),
        source_modified: ModifiedTime::from_system_time(
            metadata
                .modified()
                .with_context(|| format!("reading modification time for {}", path.display()))?,
        ),
    })
}

fn validate_folder(path: &Path, label: &str) -> Result<()> {
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

fn next_operation_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = OPERATION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:x}-{:x}-{counter:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_preserves_exact_topology_and_source_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let staging = temp.path().join("staging");
        fs::create_dir_all(source.join("empty")).unwrap();
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("zero.tmp"), []).unwrap();
        fs::write(source.join("nested/data.part"), [0_u8, 255, 7]).unwrap();
        // A move never interpolates: literal braces survive in names and bytes.
        fs::write(source.join("notes_{client}.md"), "hello {name}").unwrap();
        fs::create_dir(&staging).unwrap();

        let manifest = MoveManifest::scan(&source).unwrap();
        let progress = Mutex::new(Progress::new(&[]));
        copy_to_staging(
            &manifest,
            &source,
            &staging,
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        manifest.verify_destination(&staging).unwrap();
        manifest.verify_source_unchanged(&source).unwrap();
        assert!(staging.join("empty").is_dir());
        assert_eq!(fs::read(staging.join("zero.tmp")).unwrap(), b"");
        assert_eq!(
            fs::read(staging.join("nested/data.part")).unwrap(),
            [0_u8, 255, 7]
        );
        assert_eq!(
            fs::read_to_string(staging.join("notes_{client}.md")).unwrap(),
            "hello {name}"
        );
    }

    /// Create a directory link inside a test tree, cross-platform. Windows
    /// junctions need no elevation (unlike symlinks, which want Developer
    /// Mode), so `mklink /J` is the portable-enough choice there. Returns
    /// `false` when the OS refused, so a test can skip rather than fail on a
    /// machine with restrictive policy.
    fn make_dir_link(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    /// The data-loss regression, now guarded where the invariant lives. A
    /// junction inside a project was once invisible to the walk, so a staged
    /// move copied around it, verification walked the same blind way and
    /// reported success, and the source was deleted. The scan refuses the whole
    /// move instead: a link is neither followed nor silently omitted.
    #[test]
    fn scan_refuses_a_link_rather_than_skipping_it() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("real_asset_library");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("payload.txt"), "irreplaceable").unwrap();

        let source = temp.path().join("project");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("normal.txt"), "ordinary").unwrap();
        if !make_dir_link(&source.join("linked"), &target) {
            eprintln!("skipping: OS refused to create a directory link");
            return;
        }

        let error = MoveManifest::scan(&source).unwrap_err().to_string();
        assert!(
            error.contains("links are not supported") && error.contains("linked"),
            "the scan must name the link it refuses, got: {error}"
        );
    }

    /// A missing source is an error, never an empty manifest that would verify
    /// against an empty destination.
    #[test]
    fn scan_of_a_missing_tree_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        assert!(MoveManifest::scan(&temp.path().join("missing")).is_err());
    }

    /// Verification is what stands between a move and deleting a good source,
    /// so it has to catch the real network-share failure modes: a truncated
    /// file and a dropped one.
    #[test]
    fn verify_destination_detects_short_and_missing_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let staging = temp.path().join("staging");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("a.txt"), "hello").unwrap();
        fs::write(source.join("sub/b.bin"), vec![0_u8; 2048]).unwrap();
        fs::create_dir(&staging).unwrap();

        let manifest = MoveManifest::scan(&source).unwrap();
        let progress = Mutex::new(Progress::new(&[]));
        copy_to_staging(
            &manifest,
            &source,
            &staging,
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        manifest.verify_destination(&staging).unwrap();

        // Truncated at the destination.
        fs::write(staging.join("sub/b.bin"), vec![0_u8; 1024]).unwrap();
        assert!(manifest.verify_destination(&staging).is_err());
        fs::write(staging.join("sub/b.bin"), vec![0_u8; 2048]).unwrap();
        manifest.verify_destination(&staging).unwrap();

        // Dropped at the destination.
        fs::remove_file(staging.join("a.txt")).unwrap();
        assert!(manifest.verify_destination(&staging).is_err());
    }

    #[test]
    fn source_metadata_changes_are_detected_after_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"one").unwrap();
        let manifest = MoveManifest::scan(&source).unwrap();
        fs::write(source.join("file"), b"two-two").unwrap();
        assert!(manifest.verify_source_unchanged(&source).is_err());
    }

    #[test]
    fn transaction_journal_derives_target_and_staging_from_location() {
        let temp = tempfile::tempdir().unwrap();
        let source_base = temp.path().join("source-base");
        let target_base = temp.path().join("target-base");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let transaction = MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
        )
        .unwrap();
        assert_eq!(transaction.final_path(), target_base.join("project"));
        assert_eq!(
            transaction.staging_path(),
            transaction.operation_dir.join(STAGING_DIR)
        );
        let raw = fs::read_to_string(transaction.operation_dir.join(JOURNAL_FILE)).unwrap();
        assert!(!raw.contains("staging"));
        assert!(!raw.contains(&target_base.display().to_string()));
    }
}
