//! One base's own sweep: deleted projects, stranded case renames, creates to
//! resume, and its records.

use super::*;

pub(super) fn reconcile_base(
    cfg: &Config,
    base: &Path,
    report: &mut ReconcileReport,
    seen: &mut HashSet<String>,
    retired: &mut Vec<(PathBuf, String)>,
    pointers: &mut Vec<(PathBuf, String)>,
    pass: &mut Pass,
) {
    let ticker = pass.ticker;
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) => {
            report
                .unrecoverable
                .push(format!("could not read {}: {error}", base.display()));
            return;
        }
    };
    let mut transaction_root = None;
    for entry in entries.flatten() {
        if ticker.cancelled() {
            report.cancelled = true;
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                report
                    .unrecoverable
                    .push(format!("could not classify {}: {error}", path.display()));
                continue;
            }
        };
        if name == transactions::TRANSACTIONS_DIR {
            if file_type.is_dir() && !file_type.is_symlink() {
                transaction_root = Some(path);
            } else {
                report.unrecoverable.push(format!(
                    "{}: reserved transaction root is not a real directory; left untouched",
                    path.display()
                ));
            }
            continue;
        }
        if file_type.is_dir() && !file_type.is_symlink() {
            // fastf's own hidden folders are never looked inside for a create
            // to resume: a retired project may well hold one.
            // A case-only rename first: its staging name carries the
            // project's own name, which may begin like one of fastf's.
            if is_stranded_case_rename(&name, &path) {
                ticker.item(&name);
                reconcile_case_rename(base, &name, &path, report);
                continue;
            }
            if let Some(operation) = transactions::retired_operation(&name) {
                retired.push((path, operation.to_string()));
                continue;
            }
            if let Some(operation) = move_cleanup::deleted_operation(&name) {
                if pass.leaves(operation) {
                    continue;
                }
                ticker.item("a deleted project's folder");
                reconcile_deleted(&path, report, pass);
                continue;
            }
            if let Some(operation) = crate::core::move_preflight::probe_operation(&name) {
                // A live move is mid-probe: its own to clear.
                if pass.leaves(operation) {
                    continue;
                }
                // A move's probe outlives it only when the move was killed
                // mid-probe; this pass holds the lock, so no move is running.
                ticker.item("a move's probe folder");
                match crate::core::move_preflight::clear_probe(&path) {
                    Ok(()) => report.cleared += 1,
                    Err(why) => report.leftovers.push(format!(
                        "{}: a move's probe folder {why}, so it was left; delete it \
                         yourself once you have looked.",
                        crate::util::paths::display_path(&path)
                    )),
                }
                continue;
            }
            let legacy = legacy_create_marker_path(&path);
            if entry_exists_quiet(&legacy) {
                report.obsolete.push(legacy.display().to_string());
            }
            let create_v2 = create_journal_path(&path);
            if entry_exists_quiet(&create_v2) {
                ticker.item(&name);
                reconcile_create(&path, report);
            } else if crate::core::project_info::is_provisioning(&path) {
                report.incomplete.push(path.display().to_string());
            }
        } else if let Some(operation) = transactions::pointer_operation(&name) {
            pointers.push((path, operation.to_string()));
        } else if let Some(operation) = move_cleanup::deleted_record_operation(&name) {
            if pass.leaves(operation) {
                continue;
            }
            ticker.item("a deleted project's folder");
            reconcile_deleted_in_place(base, &path, operation, report, pass);
        } else if name.starts_with(MARKER_MOVE_PREFIX) && name.ends_with(".json") {
            report.obsolete.push(path.display().to_string());
        }
    }
    if let Some(root) = transaction_root {
        reconcile_transactions(cfg, base, &root, report, seen, pass);
    }
}

/// Finish a delete made in place (a cloud mount): the record beside the
/// folder lists what the project held when it was deleted, and only that
/// goes. The word was typed once, for the whole delete.
fn reconcile_deleted_in_place(
    base: &Path,
    record_path: &Path,
    operation: &str,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let shown = crate::util::paths::display_path(record_path);
    let record = match move_cleanup::read_delete_record(record_path) {
        Ok(record) => record,
        Err(error) => {
            report.leftovers.push(format!(
                "{shown}: a delete's record fastf cannot read ({error:#}); it keeps it"
            ));
            return;
        }
    };
    let folder = base.join(&record.folder);
    match crate::util::paths::presence(&folder) {
        crate::util::paths::Presence::Unknown(error) => {
            report.waiting.push(format!(
                "{}: a deleted project's folder does not answer ({error}); fastf finishes \
                 removing it once it does",
                crate::util::paths::display_path(&folder)
            ));
            return;
        }
        crate::util::paths::Presence::Absent => {
            if move_cleanup::settling(operation, &folder) {
                return;
            }
            if fs::remove_file(record_path).is_ok() {
                crate::core::records::remove(operation);
                report.cleared += 1;
            }
            return;
        }
        crate::util::paths::Presence::Present(_) => {}
    }
    crate::core::records::not_gone(operation);
    let pinfo = crate::core::project_info::pinfo_path(&folder);
    if pinfo.is_file() {
        let owner = crate::core::project_info::read_metadata(&folder)
            .ok()
            .flatten()
            .map(|metadata| metadata.id);
        if owner.as_deref() != Some(record.project_id.as_str()) {
            // Another project took the name: the delete has nothing left.
            if fs::remove_file(record_path).is_ok() {
                crate::core::records::remove(operation);
                report.cleared += 1;
            }
            return;
        }
        // Stopped before its `PROJECT_INFO.md` went: the delete goes on.
        if let Err(error) = crate::util::fs_retry::remove_file(&pinfo) {
            report.leftovers.push(format!(
                "{}: a deleted project's PROJECT_INFO.md could not be removed ({error}); \
                 `fastf reconcile` tries again",
                crate::util::paths::display_path(&folder)
            ));
            return;
        }
        library::refresh_cache(&folder);
    }
    let housekeeping =
        move_cleanup::Housekeeping::DeletedInPlace(Box::new(move_cleanup::DeletedInPlace {
            folder: folder.clone(),
            record_path: record_path.to_path_buf(),
            record,
        }));
    match pass.deferred.as_mut() {
        Some(deferred) => deferred.push(Deferred {
            housekeeping,
            subject: String::new(),
            source: folder.clone(),
            final_path: folder,
        }),
        None => {
            let fate = housekeeping.run(pass.ticker);
            report_deleted(&folder, fate, report);
        }
    }
}

/// Finish removing a deleted project's folder. The user confirmed the delete
/// by typing the word, and only `fastf delete` writes the name.
fn reconcile_deleted(path: &Path, report: &mut ReconcileReport, pass: &mut Pass) {
    let housekeeping = move_cleanup::Housekeeping::Deleted(path.to_path_buf());
    if let Some(deferred) = pass.deferred.as_mut() {
        deferred.push(Deferred {
            housekeeping,
            subject: String::new(),
            source: path.to_path_buf(),
            final_path: path.to_path_buf(),
        });
        return;
    }
    let fate = housekeeping.run(pass.ticker);
    report_deleted(path, fate, report);
}

pub(super) fn report_deleted(path: &Path, fate: SourceFate, report: &mut ReconcileReport) {
    match fate {
        SourceFate::Leftover {
            reason, redundant, ..
        } => report.leftovers.push(format!(
            "{}: a deleted project's folder is not fully removed yet ({reason}); {}",
            crate::util::paths::display_path(path),
            if redundant {
                "`fastf reconcile` finishes it."
            } else {
                "fastf keeps what it cannot remove safely."
            }
        )),
        _ => report.cleared += 1,
    }
}

/// Is this dot-folder a project a case-only rename left behind?
///
/// Both halves matter. The name has to be one `library::lifecycle` writes, and
/// the folder has to hold a `PROJECT_INFO.md` — otherwise a directory somebody
/// else named `.Album.fastf-case` would be renamed on top of whatever `Album`
/// is, which is a great deal worse than the state being repaired.
pub(super) fn is_stranded_case_rename(name: &str, path: &Path) -> bool {
    crate::core::library::case_staging_target(name).is_some()
        && entry_exists_quiet(&crate::core::project_info::pinfo_path(path))
}

/// Finish a case-only rename that was killed between its two renames.
///
/// `library::lifecycle::rename_project_inner` renames the project to
/// `.<target>.fastf-case` and then to `<target>`; every *error* path puts it
/// back, and a failed rollback says where it left it. A hard kill or a power
/// loss reaches neither, and the project is then parked under a dot-prefixed
/// name that `scan_base` skips — invisible to `recent`, `search`, `reindex`,
/// `resolve` and the app, with nothing anywhere recording that it happened. It
/// was the one multi-step mutation in the crate with no recovery story.
///
/// Finishing forward rather than rolling back is the only option and the right
/// one: the staging name carries the *target*, and the name the project had
/// before is not written down anywhere by then.
fn reconcile_case_rename(base: &Path, name: &str, path: &Path, report: &mut ReconcileReport) {
    let Some(target) = crate::core::library::case_staging_target(name) else {
        return;
    };
    let destination = base.join(target);
    // A case-only rename means the two names differ only in case, so on a
    // case-insensitive filesystem `destination` is a different path from
    // `path` — the staging folder is dot-prefixed. An occupied destination is
    // therefore somebody else's, and is not ours to overwrite.
    // Not only an occupied destination: one that does not answer may be
    // occupied too, and a rename over it is not ours to risk.
    if !crate::util::paths::presence(&destination).is_absent() {
        report.unrecoverable.push(format!(
            "{}: an interrupted rename left this project here, and {} is already \
             taken; rename it by hand to make the project visible again",
            crate::util::paths::display_path(path),
            crate::util::paths::display_path(&destination)
        ));
        return;
    }
    match crate::util::fs_retry::rename_dir(path, &destination) {
        Ok(()) => {
            // The base's cache has to learn, exactly as the create arm's resume
            // does. Leaving it to the staleness gate is not enough: a rename
            // within a directory does not reliably move that directory's mtime
            // on Windows, and `write_cache` deliberately re-stamps the index
            // *after* the rename that publishes it — so a cache written a
            // moment ago can still read as current, and the project stays
            // missing from a library it has just been put back into. Found by
            // the Windows leg of CI, on Linux's own green run.
            crate::core::library::refresh_cache(&destination);
            report.restored += 1;
        }
        Err(error) => report.unrecoverable.push(format!(
            "{}: an interrupted rename left this project here and it could not be \
             finished ({error}); rename it to {} by hand to make it visible again",
            crate::util::paths::display_path(path),
            crate::util::paths::display_path(&destination)
        )),
    }
}
