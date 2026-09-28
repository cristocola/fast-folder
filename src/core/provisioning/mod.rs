//! Provisioning journals and recovery.
//!
//! Version-1 create/move markers hold arbitrary absolute paths. They are
//! discovered by filename only, reported as obsolete, and never parsed or
//! mutated. Version 2 uses validated relative create paths and private move
//! transactions whose target/staging locations are derived from their owned
//! directory.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::core::assets::{self, CopyJob, JobPhase, Progress};
use crate::core::config::Config;
use crate::core::library;
use crate::core::move_cleanup::{self, Cleanup, SetAside, SourceFate};
use crate::core::progress::Ticker;
use crate::core::template;
use crate::core::transactions::{self, MoveJournal, MoveManifest, MovePhase, Operation};
use crate::core::validated::TemplateSlug;

mod base;
mod create;
mod finish;
mod incomplete;
mod moves;
mod pass;
mod recordless;
mod report;

use base::*;
pub use create::*;
use finish::*;
pub use incomplete::*;
pub(crate) use moves::*;
pub use pass::*;
use recordless::*;
pub use report::*;

/// Filename of an obsolete pre-v2 per-project create marker.
pub(crate) const MARKER_CREATE: &str = ".fastf-provisioning.json";
/// Prefix of obsolete pre-v2 move markers at a base root.
pub(crate) const MARKER_MOVE_PREFIX: &str = ".fastf-move-";
/// Filename of the scoped create journal introduced in v2.
pub const CREATE_JOURNAL_V2: &str = ".fastf-create-v2.json";

const CREATE_VERSION: u32 = 2;

fn remove_owned_file(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.file_type().is_file() => {
            crate::util::fs_retry::remove_file(path)
                .with_context(|| format!("removing {label} {}", path.display()))
        }
        Ok(_) => bail!("refusing to remove replaced {label}: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspecting {label} {}", path.display())),
    }
}

/// Whether something answers at `path` — **for finding work only**. Where a
/// wrong "absent" would remove something or clear a record, ask
/// [`crate::util::paths::presence`] and treat its `Unknown` as a reason to
/// wait.
fn entry_exists_quiet(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests;
