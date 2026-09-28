//! `library`'s own unit tests. They reach across its submodules, so they are
//! kept together and sorted by subject; the fixtures are here.

use super::*;
use crate::core::assets::Progress;
use crate::core::config::Config;
use crate::core::move_engine::{SourceOutcome, is_cross_device_error, staged_copy_verify_commit};
use crate::core::project_info;
#[cfg(debug_assertions)]
use crate::core::provisioning;
use crate::core::transactions;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::thread::sleep;
use std::time::Duration;

mod discovery;
mod lifecycle;
// Every test in it arms a failpoint, and failpoints exist in debug builds only.
#[cfg(debug_assertions)]
mod mounts;
mod moves;
mod resolve;

/// Write a project folder with a valid `PROJECT_INFO.md` frontmatter block.
fn write_project(base: &Path, folder: &str, id: &str, template: &str, created: &str) {
    let dir = base.join(folder);
    fs::create_dir_all(&dir).unwrap();
    // Backslashes in a double-quoted YAML scalar are escape sequences —
    // a raw Windows path (`C:\Users\...`) makes the whole frontmatter
    // unparseable, so escape them.
    let path_yaml = dir.display().to_string().replace('\\', "\\\\");
    let fm = format!(
        "---\nid: {id}\ntemplate: {template}\ntemplate_name: \"{template} name\"\n\
         created: \"{created}\"\nfolder: {folder}\npath: \"{path_yaml}\"\nvariables: {{}}\ntags: []\n\
         ---\n\n# Project Info\n"
    );
    fs::write(dir.join(project_info::RESERVED_FILENAME), fm).unwrap();
}

fn cfg_for(base: &Path, extra: &[&Path]) -> Config {
    Config {
        base_dir: base.display().to_string(),
        bases: extra.iter().map(|p| p.display().to_string()).collect(),
        ..Default::default()
    }
}

/// fastf's hidden per-project folders in `base`: a retired original, a
/// deleted project on its way out.
fn retired_folders(base: &Path) -> Vec<String> {
    fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.starts_with(".fastf-moved-")
                || name.starts_with(".fastf-deleted-")
                || name.starts_with(".fastf-probe-")
        })
        .collect()
}

fn v2_transaction_count(base: &Path) -> usize {
    let root = transactions::transaction_root(base);
    fs::read_dir(root)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}
