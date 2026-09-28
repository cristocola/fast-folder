//! What is unfinished in the bases, listed without changing any of it.

use super::*;

/// What kind of unfinished work a marker or journal represents. It is what a
/// pass found on disk, and is written nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompleteKind {
    /// A v2 create journal that can be resumed or reported.
    Create,
    /// A v2 move transaction.
    Move,
    /// A pre-v2 create marker. Reported, never parsed — see the module docs.
    ObsoleteCreateV1,
    /// A pre-v2 move marker.
    ObsoleteMoveV1,
    CreateV2Invalid,
    MoveV2Invalid,
    /// A case-only rename that was killed between its two renames, leaving the
    /// project parked under `.<target>.fastf-case`. Discovery skips dot-prefixed
    /// folders, so until this is finished the project is simply gone from the
    /// library.
    RenameStaging,
    /// A project that has left the library — moved, or deleted — whose old
    /// folder, hidden beside the others, is not removed yet.
    Leftover,
}

#[derive(Debug, Clone)]
pub struct Incomplete {
    pub path: String,
    pub kind: IncompleteKind,
    pub pending: usize,
    /// A move's record folder, where the item is one.
    pub record: Option<String>,
}

impl Incomplete {
    /// An item at `path`, with nothing pending and no record of its own.
    fn at(path: &Path, kind: IncompleteKind) -> Self {
        Self {
            path: path.display().to_string(),
            kind,
            pending: 0,
            record: None,
        }
    }
}

/// Cheap read-only discovery used by CLI/UI state. Invalid v2 journals are
/// surfaced by their owned path and are never followed.
pub fn list_incomplete(cfg: &Config) -> Vec<Incomplete> {
    list_incomplete_in(&cfg.answering_bases())
}

/// [`list_incomplete`] over `bases` alone: the ones that answered a probe
/// (`core::attention`), so a base that does not answer never freezes it.
pub fn list_incomplete_in(bases: &[PathBuf]) -> Vec<Incomplete> {
    // A job running now is not something that needs attention: its records
    // are its own until it ends.
    let live = crate::core::jobs::live_workers();
    let mine = |operation: &str| crate::core::jobs::owned_by(operation, &live);
    let mut out = Vec::new();
    let mut operations = HashSet::new();
    let mut retired = Vec::new();
    let mut pointers = Vec::new();
    for configured in bases {
        let Ok(base) = crate::util::paths::canonical(configured) else {
            continue;
        };
        if crate::util::paths::require_real_directory(&base, "configured base").is_err() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if name == transactions::TRANSACTIONS_DIR {
                if file_type.is_dir() && !file_type.is_symlink() {
                    list_move_transactions(&base, &path, &mut out, &mut operations, &live);
                } else {
                    out.push(Incomplete::at(&path, IncompleteKind::MoveV2Invalid));
                }
                continue;
            }
            if file_type.is_dir() && !file_type.is_symlink() {
                if is_stranded_case_rename(&name, &path) {
                    out.push(Incomplete::at(&path, IncompleteKind::RenameStaging));
                    continue;
                }
                if let Some(operation) = transactions::retired_operation(&name) {
                    retired.push((path, operation.to_string()));
                    continue;
                }
                let hidden = move_cleanup::deleted_operation(&name)
                    .or_else(|| crate::core::move_preflight::probe_operation(&name));
                if hidden.is_some_and(mine) {
                    continue;
                }
                if hidden.is_some() {
                    out.push(Incomplete::at(&path, IncompleteKind::Leftover));
                    continue;
                }
                if entry_exists_quiet(&legacy_create_marker_path(&path)) {
                    out.push(Incomplete::at(
                        &legacy_create_marker_path(&path),
                        IncompleteKind::ObsoleteCreateV1,
                    ));
                }
                if entry_exists_quiet(&create_journal_path(&path)) {
                    match read_create_journal(&path) {
                        Ok(journal) => out.push(Incomplete {
                            pending: journal.jobs.len(),
                            ..Incomplete::at(&path, IncompleteKind::Create)
                        }),
                        Err(_) => out.push(Incomplete::at(
                            &create_journal_path(&path),
                            IncompleteKind::CreateV2Invalid,
                        )),
                    }
                } else if crate::core::project_info::is_provisioning(&path) {
                    out.push(Incomplete::at(&path, IncompleteKind::Create));
                }
            } else if let Some(operation) = move_cleanup::deleted_record_operation(&name) {
                // A delete emptying its folder in place, unless it is done
                // and only waits out the settle.
                let settling = crate::core::records::get(operation)
                    .is_some_and(|entry| entry.gone_at.is_some());
                if !mine(operation) && !settling {
                    out.push(Incomplete::at(&path, IncompleteKind::Leftover));
                }
            } else if let Some(operation) = transactions::pointer_operation(&name) {
                pointers.push((path, operation.to_string()));
            } else if name.starts_with(MARKER_MOVE_PREFIX) && name.ends_with(".json") {
                out.push(Incomplete::at(&path, IncompleteKind::ObsoleteMoveV1));
            }
        }
    }
    // A pointer whose record is gone: its old copy has no record either.
    for (path, operation) in pointers {
        if !operations.contains(&operation) && !mine(&operation) {
            out.push(Incomplete::at(&path, IncompleteKind::Leftover));
        }
    }
    // A retired folder with a transaction is that transaction's; one without
    // is a leftover of its own.
    for (path, operation) in retired {
        if !operations.contains(&operation) && !mine(&operation) {
            out.push(Incomplete::at(&path, IncompleteKind::Leftover));
        }
    }
    out
}

fn list_move_transactions(
    base: &Path,
    root: &Path,
    out: &mut Vec<Incomplete>,
    operations: &mut HashSet<String>,
    live: &crate::core::jobs::Live,
) {
    if crate::util::paths::require_real_directory(root, "transaction root").is_err() {
        out.push(Incomplete::at(root, IncompleteKind::MoveV2Invalid));
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let operation_dir = entry.path();
        let valid_dir = entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink());
        if let Some(name) = operation_dir.file_name().and_then(|name| name.to_str()) {
            operations.insert(name.to_string());
            if crate::core::jobs::owned_by(name, live) {
                continue;
            }
        }
        if valid_dir {
            match transactions::read_journal(&operation_dir) {
                // Its old copy is gone and it only waits out the settle: not
                // something anyone needs to look at.
                Ok(journal) if is_settling(&journal) => {}
                Ok(journal) => out.push(Incomplete {
                    path: base.join(journal.target_folder).display().to_string(),
                    kind: IncompleteKind::Move,
                    pending: 0,
                    record: Some(operation_dir.display().to_string()),
                }),
                Err(_) => out.push(Incomplete::at(
                    &operation_dir,
                    IncompleteKind::MoveV2Invalid,
                )),
            }
        } else {
            out.push(Incomplete::at(
                &operation_dir,
                IncompleteKind::MoveV2Invalid,
            ));
        }
    }
}
