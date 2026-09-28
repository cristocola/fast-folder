//! Finishing a record: settling or deferring its removal, in-place and paused
//! moves, and what became of the old copy.

use super::*;

/// A record set aside is finished now, or — when the pass hands removals on
/// until its lock is released — later, by [`finish_deferred`].
pub(super) fn settle_or_defer(
    set: SetAside,
    cleanup: &Cleanup,
    pass: &mut Pass,
    subject: &str,
    report: &mut ReconcileReport,
) {
    match set {
        SetAside::Settled(fate) => {
            record_fate(fate, subject, cleanup.source, cleanup.final_path, report)
        }
        SetAside::Retired(transaction) => match pass.deferred.as_mut() {
            Some(deferred) => deferred.push(Deferred {
                housekeeping: move_cleanup::Housekeeping::old_copy(*transaction, cleanup),
                subject: subject.to_string(),
                source: cleanup.source.to_path_buf(),
                final_path: cleanup.final_path.to_path_buf(),
            }),
            None => {
                let fate = move_cleanup::remove_retired(*transaction, cleanup);
                record_fate(fate, subject, cleanup.source, cleanup.final_path, report);
            }
        },
    }
}

/// Clear a completed move's record — after any file a cloud mount put in a
/// 3.12.0 record's staging folder late has been moved into place. One that
/// cannot be keeps the record, and is named; nothing under it is deleted.
pub(super) fn finish_record(
    transaction: transactions::MoveTransaction,
    operation_dir: &Path,
    subject: &str,
    report: &mut ReconcileReport,
) {
    let old_staging = transaction
        .old_staging_path()
        .map(|staging| (crate::util::paths::presence(&staging), staging));
    if let Some((crate::util::paths::Presence::Unknown(error), staging)) = &old_staging {
        report.waiting.push(format!(
            "{subject}: the move's old staging folder at {} does not answer ({error}); fastf \
             keeps the record until it does",
            crate::util::paths::display_path(staging)
        ));
        return;
    }
    if old_staging.is_some_and(|(presence, _)| presence.is_present()) {
        let staging_shown = crate::util::paths::display_path(&transaction.staging_path());
        // The same mount may have lost the manifest; the sweep then places
        // only what the moved copy lacks.
        let manifest = transactions::read_manifest(operation_dir).ok();
        match transaction.sweep_strays(manifest.as_ref()) {
            Ok((moved, left)) if left.is_empty() => {
                if moved > 0 {
                    report.repaired.push(format!(
                        "{subject}: {moved} file{} the mount had put in the move's old staging \
                         folder after it was renamed away {} moved into place.",
                        if moved == 1 { "" } else { "s" },
                        if moved == 1 { "was" } else { "were" }
                    ));
                }
            }
            Ok((moved, left)) => {
                report.leftovers.push(format!(
                    "{subject}: {moved} moved into place; {} left in the move's old staging \
                     folder at {staging_shown} that fastf will not remove: {}",
                    left.len(),
                    left.iter().take(10).cloned().collect::<Vec<_>>().join("; ")
                ));
                return;
            }
            Err(error) => {
                report.leftovers.push(format!(
                    "{subject}: could not look through the move's old staging folder at \
                     {staging_shown} ({error:#}); fastf keeps it."
                ));
                return;
            }
        }
    }
    let pointer = (transaction.journal.retire == transactions::RetireStrategy::InPlace)
        .then(|| transaction.pointer_path());
    match transaction.remove() {
        Ok(()) => {
            if let Some(pointer) = pointer {
                let _ = crate::util::fs_retry::remove_file(&pointer);
            }
            report.completed += 1
        }
        Err(error) => report.unrecoverable.push(format!(
            "{subject}: the move is complete, but its record could not be cleared ({error:#})"
        )),
    }
}

/// A move that paused before its publish, waiting for a mount (`move_engine::
/// Paused`): every path it names answers again, so it goes on from the copy
/// it made — the rest of it copied, then published and set aside like any
/// move.
pub(super) fn reconcile_paused(
    source_base: &Path,
    target_base: &Path,
    final_path: &Path,
    transaction: transactions::MoveTransaction,
    subject: &str,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let source = transaction.source_path();
    let project = match crate::core::project_info::read_metadata(&source) {
        Ok(Some(metadata)) if metadata.id == transaction.journal.project_id => {
            library::project_from_meta(metadata, source_base, &source)
        }
        _ => {
            report.unrecoverable.push(format!(
                "{subject}: a paused move's original is not where it was; fastf changed nothing"
            ));
            return;
        }
    };
    let local_progress = std::sync::Mutex::new(crate::core::assets::Progress::new(&[]));
    let local_cancel = std::sync::atomic::AtomicBool::new(false);
    let progress = pass.ticker.progress().unwrap_or(&local_progress);
    let cancel = pass.ticker.cancel_flag().unwrap_or(&local_cancel);
    let resumed = crate::core::move_engine::resume_in_parts(
        &project,
        target_base,
        final_path,
        transaction,
        progress,
        cancel,
    );
    match resumed {
        Ok((outcome, housekeeping)) => {
            report.resumed += 1;
            match housekeeping {
                Some(housekeeping) => match pass.deferred.as_mut() {
                    Some(deferred) => deferred.push(Deferred {
                        housekeeping,
                        subject: subject.to_string(),
                        source: source.clone(),
                        final_path: final_path.to_path_buf(),
                    }),
                    None => {
                        let fate = housekeeping.run(pass.ticker.uncancellable());
                        record_fate(fate, subject, &source, final_path, report);
                    }
                },
                None => {
                    if let Some(warning) = outcome.source.warning(&source) {
                        report.leftovers.push(format!("{subject}: {warning}"));
                    }
                }
            }
        }
        Err(error)
            if error
                .downcast_ref::<crate::core::move_engine::Paused>()
                .is_some() =>
        {
            report.waiting.push(format!("{subject}: {error}"));
        }
        Err(error) => report.unrecoverable.push(format!(
            "{subject}: a paused move could not go on ({error:#})"
        )),
    }
}

/// A published move whose original leaves in place (`RetireStrategy::
/// InPlace`): its `PROJECT_INFO.md` first, then the rest through the merge.
/// The original's own path is the old copy, so whatever holds our identity
/// there is still the original, and whatever is there without it is the old
/// copy's remainder.
pub(super) fn reconcile_in_place(
    found: &Found,
    // The folder at the original's path holds another project.
    foreign: bool,
    transaction: transactions::MoveTransaction,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let (journal, subject) = (found.journal, found.subject);
    let mut notes = Vec::new();
    if foreign || !found.source_exists {
        if !foreign && move_cleanup::settling(&journal.operation_id, &found.source) {
            return;
        }
        // Nothing of the original is left for this move to remove —
        // whatever is at its path now is somebody else's.
        bookkeep(
            &found.source_base,
            journal,
            found.target_base,
            &found.final_path,
            &mut notes,
        );
        report.unrecoverable.append(&mut notes);
        if foreign {
            report.unrecoverable.push(format!(
                "{subject}: moved to {}; the folder now at {} is a different project, so \
                 fastf left it alone.",
                crate::util::paths::display_path(&found.final_path),
                crate::util::paths::display_path(&found.source)
            ));
        }
        finish_record(transaction, found.operation_dir, subject, report);
        return;
    }
    let Some(manifest) = read_manifest_or_report(found.operation_dir, subject, report) else {
        return;
    };
    let published = match transaction.read_published() {
        Ok(published) => published,
        Err(error) => {
            report.unrecoverable.push(format!(
                "{subject}: the move's published record is unreadable ({error:#}); \
                 fastf changed nothing"
            ));
            return;
        }
    };
    let reappeared = crate::core::records::get(&journal.operation_id)
        .is_some_and(|entry| entry.gone_at.is_some());
    crate::core::records::not_gone(&journal.operation_id);
    let cleanup = Cleanup {
        manifest: &manifest,
        published: published.as_ref(),
        source: &found.source,
        final_path: &found.final_path,
        project_id: &journal.project_id,
        residue_allowed: journal.source_may_be_partial(),
        split: false,
        reappeared,
        ticker: pass.ticker,
    };
    let still_the_original = crate::core::project_info::pinfo_path(&found.source).is_file();
    let fate = if still_the_original {
        // Its `PROJECT_INFO.md` is there: out of the library it goes first.
        move_cleanup::set_aside(transaction, &cleanup, || {
            bookkeep(
                &found.source_base,
                journal,
                found.target_base,
                &found.final_path,
                &mut notes,
            )
        })
    } else {
        let mut transaction = transaction;
        if journal.phase != MovePhase::Retired
            && let Err(error) = transaction.set_phase(MovePhase::Retired)
        {
            report.unrecoverable.push(format!(
                "{subject}: could not record that the original was retired ({error:#}); \
                 fastf changed nothing"
            ));
            return;
        }
        bookkeep(
            &found.source_base,
            journal,
            found.target_base,
            &found.final_path,
            &mut notes,
        );
        SetAside::Retired(Box::new(transaction))
    };
    report.unrecoverable.append(&mut notes);
    settle_or_defer(fate, &cleanup, pass, subject, report);
}

pub(super) fn read_manifest_or_report(
    operation_dir: &Path,
    subject: &str,
    report: &mut ReconcileReport,
) -> Option<MoveManifest> {
    match transactions::read_manifest(operation_dir) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            report.unrecoverable.push(format!(
                "{subject}: the move's record of what it copied is unreadable ({error:#}); \
                 fastf changed nothing"
            ));
            None
        }
    }
}

/// The project is the moved copy: patch its metadata and the two caches.
/// Idempotent, so a pass that repeats it after a crash changes nothing.
pub(super) fn bookkeep(
    source_base: &Path,
    journal: &MoveJournal,
    target_base: &Path,
    final_path: &Path,
    notes: &mut Vec<String>,
) {
    if let Err(error) =
        library::finish_recovered_move(source_base, &journal.source_folder, target_base, final_path)
    {
        notes.push(format!(
            "{}: the move is complete, but its bookkeeping failed ({error:#}); \
             `fastf reindex` repairs the listing",
            crate::util::paths::display_path(final_path)
        ));
    }
}

/// Say what became of an original, in the report's words.
pub(super) fn record_fate(
    fate: SourceFate,
    subject: &str,
    source: &Path,
    final_path: &Path,
    report: &mut ReconcileReport,
) {
    let shown = |path: &Path| crate::util::paths::display_path(path);
    match fate {
        SourceFate::Removed { record_kept: None } | SourceFate::Settling => report.completed += 1,
        SourceFate::Removed {
            record_kept: Some(reason),
        } => report.unrecoverable.push(format!(
            "{subject}: moved to {}, and its original is removed, but the move's record \
             could not be cleared ({reason}); the next `fastf reconcile` clears it",
            shown(final_path)
        )),
        SourceFate::Leftover {
            path,
            reason,
            redundant: true,
        } => report.leftovers.push(format!(
            "{subject}: moved to {}; the original's old copy at {} is not removed yet \
             ({reason}). Everything in it is in the moved copy too; `fastf reconcile` \
             finishes it.",
            shown(final_path),
            shown(&path)
        )),
        SourceFate::Leftover {
            path,
            reason,
            redundant: false,
        } => {
            report.needs(
                &path,
                crate::core::attention::VerdictKind::Conflict,
                &format!("what is left of the original differs from the moved copy: {reason}"),
            );
            report.leftovers.push(format!(
                "{subject}: moved to {}; what is left of the original at {} differs from the \
                 moved copy, so fastf keeps it, with the move's record, until you decide: \
                 {reason}",
                shown(final_path),
                shown(&path)
            ))
        }
        SourceFate::KeptWhole { reason } => report.unrecoverable.push(format!(
            "{subject}: moved to {}, but the original at {} is still there and fastf \
             removed nothing: {reason}. `fastf reconcile` tries again once that is \
             resolved; until then both copies are listed.",
            shown(final_path),
            shown(source)
        )),
        SourceFate::Unknown { reason } => report.unrecoverable.push(format!(
            "{subject}: moved to {}; fastf could not tell whether the original at {} was \
             set aside ({reason}). fastf removed nothing; run `fastf reconcile` again.",
            shown(final_path),
            shown(source)
        )),
    }
}

/// A published move whose old copy was removed on a mount that can put it
/// back, waiting out the settle (`core::records`).
pub(super) fn is_settling(journal: &MoveJournal) -> bool {
    let old_copy = match journal.retire {
        transactions::RetireStrategy::Rename => {
            transactions::retired_path(&journal.source_base, &journal.operation_id)
        }
        transactions::RetireStrategy::InPlace => journal.source_base.join(&journal.source_folder),
    };
    journal.phase.rank() >= MovePhase::CleanupPending.rank()
        && crate::core::records::get(&journal.operation_id)
            .is_some_and(|entry| entry.gone_at.is_some())
        && crate::util::paths::presence(&old_copy).is_absent()
}

/// Whether `wanted` is still one of the configured bases — by the path the
/// record holds, without asking the filesystem, which has no answer for an
/// unplugged drive or a mount that dropped.
pub(super) fn is_configured_base(cfg: &Config, wanted: &Path) -> bool {
    cfg.effective_bases().iter().any(|base| base == wanted)
        || cfg
            .bases
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(cfg.base_dir.as_str()))
            .map(str::trim)
            .any(|raw| !raw.is_empty() && Path::new(raw) == wanted)
}

pub(super) fn configured_real_base(cfg: &Config, wanted: &Path) -> Result<PathBuf> {
    let wanted = crate::util::paths::canonical_in_time(wanted).with_context(|| {
        format!(
            "resolving configured base {}",
            crate::util::paths::display_path(wanted)
        )
    })?;
    for candidate in cfg.answering_bases() {
        let Ok(candidate) = crate::util::paths::canonical(&candidate) else {
            continue;
        };
        if candidate == wanted {
            crate::util::paths::require_real_directory(&candidate, "configured base")?;
            return Ok(candidate);
        }
    }
    bail!(
        "{} is not a configured real base",
        crate::util::paths::display_path(&wanted)
    )
}
