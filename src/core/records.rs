//! The data dir's index of every move and copy record fastf starts: where the
//! record is, and when a removed old copy was last seen gone.
//!
//! **The record in the target base stays the authority**; this index only
//! finds it. Reconcile walks the configured bases, and without the index a
//! record anywhere else is out of its sight: a `copy-to`'s, beside a
//! destination outside every base, and a move's whose target base was later
//! dropped from `bases`, whose old copy would then read as having no record.
//! Written best effort, at `<data dir>/records/<operation>.json`, read without
//! trusting it: nothing here authorises a removal.
//!
//! It also keeps `gone_at` for the **settle**: a cloud mount that uploads in
//! the background (rclone) can put a removed folder back minutes later. On
//! such a mount a removal that ends with the folder gone keeps the record, and
//! the first pass at least [`SETTLE_SECS`] later that still finds it gone
//! clears it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How long a removed old copy on a mount that can put it back must stay gone
/// before its record goes.
pub const SETTLE_SECS: i64 = 600;

const VERSION: u32 = 1;

/// What [`Entry::kind`] holds for a move's record.
pub const MOVE: &str = "move";
/// For a copy's.
pub const COPY: &str = "copy";
/// For a delete that empties its folder in place.
pub const DELETE: &str = "delete";

/// One record, as the index knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub version: u32,
    pub operation: String,
    /// [`MOVE`], [`COPY`] or [`DELETE`]. A word, so an index another version
    /// wrote is read whatever it holds.
    pub kind: String,
    pub project_id: String,
    /// The transaction directory.
    pub record: PathBuf,
    pub source_base: PathBuf,
    pub source_folder: PathBuf,
    pub target_base: PathBuf,
    pub target_folder: PathBuf,
    /// Seconds since the Unix epoch when the old copy was first seen gone,
    /// on a mount that can put it back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gone_at: Option<i64>,
    /// The mount the source base was on when the record was made
    /// (`util::fs_kind::mount_identity`). An unmounted mount point is an
    /// ordinary empty folder, and everything under it reads as gone: a pass
    /// that finds the base on another mount now waits instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_mount: Option<String>,
}

/// Why the source base of the record `operation` cannot be looked at now —
/// it is not on the mount it was on when the operation began, so nothing
/// under it can be told gone — or `None` when it can, or the index does not
/// know.
pub fn source_unmounted(operation: &str) -> Option<String> {
    let entry = get(operation)?;
    let then = entry.source_mount?;
    let now = crate::util::fs_kind::mount_identity(&entry.source_base)?;
    (now != then).then(|| {
        format!(
            "{} is not on the mount it was on when this began ({then}; now {now})",
            crate::util::paths::display_path(&entry.source_base)
        )
    })
}

impl Entry {
    /// A delete's record sits beside its folder, where the base's own walk
    /// finds it; the index only keeps its settle.
    pub fn is_delete(&self) -> bool {
        self.kind == DELETE
    }
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            version: VERSION,
            operation: String::new(),
            kind: String::new(),
            project_id: String::new(),
            record: PathBuf::new(),
            source_base: PathBuf::new(),
            source_folder: PathBuf::new(),
            target_base: PathBuf::new(),
            target_folder: PathBuf::new(),
            gone_at: None,
            source_mount: None,
        }
    }
}

fn dir() -> Option<PathBuf> {
    // A unit test that has not sandboxed the data directory must not write
    // into the developer's own — nor read the index of a test beside it that
    // has, since the environment is the whole process's.
    #[cfg(test)]
    if !crate::util::test_env::holds_guard() {
        return None;
    }
    crate::util::paths::try_install_dir()
        .ok()
        .map(|(dir, _)| dir.join("records"))
}

fn path_of(operation: &str) -> Option<PathBuf> {
    if !crate::core::transactions::is_operation_id(operation) {
        return None;
    }
    dir().map(|dir| dir.join(format!("{operation}.json")))
}

/// Remember a record. Best effort: a data dir that cannot be written costs
/// only the lookup.
pub fn add(entry: &Entry) {
    let Some(path) = path_of(&entry.operation) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = crate::util::atomic::write_json(&path, entry);
}

pub fn get(operation: &str) -> Option<Entry> {
    let raw = std::fs::read(path_of(operation)?).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Forget a record whose transaction is gone.
pub fn remove(operation: &str) {
    if let Some(path) = path_of(operation) {
        let _ = crate::util::fs_retry::remove_file(&path);
    }
}

/// Every record the index knows, oldest first.
pub fn all() -> Vec<Entry> {
    let Some(dir) = dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = entries
        .flatten()
        .filter_map(|entry| std::fs::read(entry.path()).ok())
        .filter_map(|raw| serde_json::from_slice::<Entry>(&raw).ok())
        .filter(|entry| crate::core::transactions::is_operation_id(&entry.operation))
        .collect();
    out.sort_by(|left, right| left.operation.cmp(&right.operation));
    out
}

/// When the old copy of `operation` was first seen gone, setting it now if
/// it was not set: the settle's clock.
pub fn gone_since(operation: &str, now: i64) -> Option<i64> {
    let mut entry = get(operation)?;
    if let Some(at) = entry.gone_at {
        return Some(at);
    }
    entry.gone_at = Some(now);
    add(&entry);
    Some(now)
}

/// Forget the settle's clock: the old copy was seen again.
pub fn not_gone(operation: &str) {
    if let Some(mut entry) = get(operation)
        && entry.gone_at.is_some()
    {
        entry.gone_at = None;
        add(&entry);
    }
}

/// Whether the old copy of a move from `source_base` could be put back after
/// its removal by the mount it is on: an rclone mount, or a FUSE mount fastf
/// cannot tell apart from one.
pub fn can_resurrect(source_base: &Path) -> bool {
    matches!(
        crate::util::fs_kind::of(source_base),
        crate::util::fs_kind::FsKind::Rclone | crate::util::fs_kind::FsKind::OtherFuse
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_round_trips_and_is_forgotten() {
        let _env = crate::util::test_env::EnvGuard::sandbox();
        let entry = Entry {
            operation: "18d8e31be3d08289-eae64-0".to_string(),
            kind: "move".to_string(),
            project_id: "ID0001".to_string(),
            record: PathBuf::from("/b/.fastf-transactions/18d8e31be3d08289-eae64-0"),
            ..Entry::default()
        };
        add(&entry);
        assert_eq!(get(&entry.operation), Some(entry.clone()));
        assert_eq!(all(), vec![entry.clone()]);
        assert_eq!(gone_since(&entry.operation, 100), Some(100));
        assert_eq!(gone_since(&entry.operation, 900), Some(100), "set once");
        not_gone(&entry.operation);
        assert_eq!(get(&entry.operation).unwrap().gone_at, None);
        remove(&entry.operation);
        assert_eq!(get(&entry.operation), None);
    }

    #[test]
    fn a_name_that_is_not_an_operation_is_never_a_path() {
        let _env = crate::util::test_env::EnvGuard::sandbox();
        assert_eq!(path_of("../../etc/passwd"), None);
    }
}
