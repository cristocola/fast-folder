//! An old copy whose record is gone, finished by content.

use super::*;

/// An old copy whose move left no record — 3.13 cleared records while their
/// old copies were still there. One that holds nothing but folders goes; one
/// whose project fastf can find is removed where the project holds the same
/// thing, byte for byte (`merge::remove_identical`); what differs stays,
/// named, and so does all of it when the project cannot be found.
pub(super) fn reconcile_recordless(
    cfg: &Config,
    path: &Path,
    operation: &str,
    pointer: Option<PathBuf>,
    project_hint: Option<String>,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let shown = crate::util::paths::display_path(path);
    pass.ticker.item("an old copy with no record");
    // An empty folder a cloud mount put back after it was removed holds
    // nothing to lose.
    let holds_nothing = holds_only_folders(path);
    if holds_nothing {
        match crate::core::removal::remove_tree(
            path,
            None,
            crate::core::removal::Purpose::Delete,
            pass.ticker,
        ) {
            crate::core::removal::Removal::Removed => {
                if let Some(pointer) = &pointer {
                    let _ = crate::util::fs_retry::remove_file(pointer);
                }
                report.cleared += 1;
            }
            crate::core::removal::Removal::Leftover { reason, .. } => {
                report.leftovers.push(format!(
                    "{shown}: an empty old copy of a move could not be removed yet ({reason}); \
                     `fastf reconcile` tries again"
                ));
            }
        }
        return;
    }
    let owner = project_hint.or_else(|| orphan_owner(path, operation));
    let found = owner.as_ref().and_then(|id| {
        library::discover(cfg)
            .into_iter()
            .find(|project| project.id == *id)
    });
    let Some(project) = found else {
        let note = orphan_note(cfg, path, operation);
        report.needs(
            path,
            crate::core::attention::VerdictKind::UnknownProject,
            &match &owner {
                Some(id) => format!(
                    "An old copy of {id} whose move left no record. fastf finds {id} in no base \
                     now, so this may be its only copy: put it back as the project, or discard \
                     it."
                ),
                None => "An old copy whose move left no record, and that does not say which \
                         project it was: look inside, then put it back or discard it."
                    .to_string(),
            },
        );
        report.leftovers.push(format!(
            "{shown}: a moved project's old copy, with no record of the move left, and \
             fastf cannot find the project it held, so it keeps all of it.{note}"
        ));
        return;
    };
    let housekeeping = move_cleanup::Housekeeping::Recordless(Box::new(move_cleanup::Recordless {
        path: path.to_path_buf(),
        operation: operation.to_string(),
        moved: project.path.clone(),
        project_id: project.id.clone(),
        pointer,
    }));
    let subject = format!("{} ({})", project.id, project.name);
    match pass.deferred.as_mut() {
        Some(deferred) => deferred.push(Deferred {
            housekeeping,
            subject,
            source: path.to_path_buf(),
            final_path: project.path.clone(),
        }),
        None => {
            let fate = housekeeping.run(pass.ticker);
            report_recordless(path, fate, &subject, report);
        }
    }
}

/// Whether `path` holds nothing but folders, down to its deepest. Stops at
/// the first thing that is not a folder — one listing, for an old copy that
/// holds a project — since on a slow mount every listing is a request.
fn holds_only_folders(path: &Path) -> bool {
    let mut folders = vec![(path.to_path_buf(), 0)];
    while let Some((folder, depth)) = folders.pop() {
        if depth >= crate::util::paths::MAX_WALK_DEPTH {
            return false;
        }
        let Ok(entries) = fs::read_dir(&folder) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && !kind.is_symlink() => {
                    folders.push((entry.path(), depth + 1));
                }
                _ => return false,
            }
        }
    }
    true
}

pub(super) fn report_recordless(
    path: &Path,
    fate: SourceFate,
    subject: &str,
    report: &mut ReconcileReport,
) {
    match fate {
        SourceFate::Leftover { reason, .. } => {
            report.needs(
                path,
                crate::core::attention::VerdictKind::Differs,
                &format!(
                    "an old copy of {subject} with no record of its move; what the project \
                     holds the same went, and what is left differs: {reason}"
                ),
            );
            report.leftovers.push(format!(
                "{}: an old copy of {subject}, whose move left no record; what the project \
                 holds the same was removed, and what differs stays: {reason}",
                crate::util::paths::display_path(path)
            ))
        }
        _ => report.cleared += 1,
    }
}

/// The project an old copy with no record held: its own `PROJECT_INFO.md`,
/// or the job that moved it, when that job moved one project.
fn orphan_owner(path: &Path, operation: &str) -> Option<String> {
    crate::core::project_info::read_metadata(path)
        .ok()
        .flatten()
        .map(|metadata| metadata.id)
        .or_else(|| {
            crate::core::jobs::list().into_iter().find_map(|job| {
                let state = job.state.as_ref()?;
                let request = job.request.as_ref()?;
                (state.operations.iter().any(|op| op == operation) && request.items.len() == 1)
                    .then(|| request.items[0].id.clone())
            })
        })
}

/// What is known about a retired folder whose record is gone: which project
/// it held — by its own `PROJECT_INFO.md`, or by the job that moved it — and
/// where that project is now.
fn orphan_note(cfg: &Config, path: &Path, operation: &str) -> String {
    let Some(id) = orphan_owner(path, operation) else {
        return String::new();
    };
    match library::discover(cfg)
        .into_iter()
        .find(|project| project.id == id)
    {
        Some(project) => format!(
            " It holds {id}, which is now at {}.",
            crate::util::paths::display_path(&project.path)
        ),
        None => {
            format!(" It holds {id}, which fastf finds in no base now — it may be the only copy.")
        }
    }
}
