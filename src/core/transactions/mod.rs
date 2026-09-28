//! Scoped move transactions.
//!
//! A transaction lives below the target base at
//! `.fastf-transactions/<operation-id>/`.  The journal deliberately contains
//! no target-base or staging path: both are derived from that owned location.
//! Source and target folder names are validated single path components before
//! they are ever joined to a base. So is where a source goes when it leaves
//! the library: `<source base>/.fastf-moved-<operation-id>`, named by nothing
//! but the operation.
//!
//! **A copy is made in its final place, and `PROJECT_INFO.md` is written
//! last.** Until that file lands the folder is not a project — discovery
//! never lists it, and the record says whose it is — so the publish is still
//! one step, and **no folder on the target is ever renamed**. 3.12.0 staged
//! under the transaction and renamed the tree into place, and on a cloud
//! mount that renames a folder while its uploads are still in flight some
//! of them land at the old path: found on a real rclone Drive mount, which
//! put three files of a moved project back under the staging path it had
//! just left, and whose cache said everything was fine.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::assets::Progress;
use crate::core::progress::Ticker;
use crate::util::pool::Queue;

mod copy;
mod journal;
mod manifest;
mod staging;
mod transaction;
mod walk;

pub(crate) use copy::*;
pub use journal::*;
pub use manifest::*;
pub use staging::*;
pub use transaction::*;
pub use walk::*;

pub const TRANSACTIONS_DIR: &str = ".fastf-transactions";
pub const JOURNAL_FILE: &str = "move.json";
/// A phase reached, as a file whose name says which: `phase.CleanupPending`.
/// **Created, never renamed.** A phase used to be a rewrite of the journal
/// through an atomic sibling and a rename, and a cloud mount that misplaces
/// renames left `move.json` on the remote saying `Copying` about a move that
/// had long since retired its original. A file that is only ever created
/// cannot be misplaced, and the phase is the highest marker present.
const PHASE_PREFIX: &str = "phase.";

/// See [`MoveTransaction::mark_split`].
const SPLIT_MARKER: &str = "split";
/// A move that paused before its publish, its copy kept for a resume.
const PAUSED_MARKER: &str = "paused";
pub(crate) const MANIFEST_FILE: &str = "manifest.json";
pub const STAGING_DIR: &str = "staging";
/// The destination as it was published: the walk of the verified staging tree,
/// times included, so a later cleanup can tell the user's edits from a copy
/// restored out of an older backup.
pub(crate) const PUBLISHED_FILE: &str = "published.json";
/// A source that has left the library, beside it in its base, until it is
/// removed. Dot-prefixed, so discovery never lists it.
pub const RETIRED_PREFIX: &str = ".fastf-moved-";

/// The journal this build writes. Version 3 added the `Retired` phase, the
/// operation and the host; a version-2 journal — what 3.11 and older wrote —
/// is still read, and is rewritten as version 3 by its first phase change,
/// before anything is renamed. An older binary refuses version 3, which is
/// what keeps it from finishing a transaction whose source it cannot see.
const MOVE_VERSION: u32 = 3;
const MOVE_VERSION_OLDEST: u32 = 2;
/// The manifest this build writes. Version 2 added the link kinds; a version-1
/// manifest — what 3.11 and older wrote — is version 2 without them, and is
/// still read, or a transaction an older binary left could never be finished.
const MANIFEST_VERSION: u32 = 2;
const MANIFEST_VERSION_OLDEST: u32 = 1;
const OPERATION_RETRIES: usize = 64;
const COPY_BUFFER_BYTES: usize = 1024 * 1024;
/// How many paths a refusal or a difference names before "and N more".
pub(crate) const LISTED: usize = 10;

static OPERATION_COUNTER: AtomicU64 = AtomicU64::new(0);
/// How long a record's removal waits for a cloud mount to finish uploading
/// the record's own files.
const RECORD_REMOVAL_WAIT_MS: u64 = 20_000;

#[cfg(test)]
mod tests;
