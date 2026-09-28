//! One move record, by the table in `src/core/CLAUDE.md` › Recovery.

use super::*;

pub(super) fn reconcile_transactions(
    cfg: &Config,
    target_base: &Path,
    root: &Path,
    report: &mut ReconcileReport,
    seen: &mut HashSet<String>,
    pass: &mut Pass,
) {
    let ticker = pass.ticker;
    if let Err(error) = crate::util::paths::require_real_directory(root, "transaction root") {
        report.unrecoverable.push(format!(
            "{}: {error:#}; fastf changed nothing",
            root.display()
        ));
        return;
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            report.unrecoverable.push(format!(
                "could not read transaction root {} ({error})",
                root.display()
            ));
            return;
        }
    };
    for entry in entries.flatten() {
        if ticker.cancelled() {
            report.cancelled = true;
            return;
        }
        let operation_dir = entry.path();
        if !entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            report.unrecoverable.push(format!(
                "{}: transaction entry is not a real directory; fastf changed nothing",
                operation_dir.display()
            ));
            continue;
        }
        // Every operation found here, readable or not, owns its retired copy:
        // an unreadable journal is still a record, and its retired folder is
        // not an orphan to report twice.
        if let Some(name) = operation_dir.file_name().and_then(|name| name.to_str()) {
            seen.insert(name.to_string());
            if pass.leaves(name) {
                continue;
            }
        }
        reconcile_record(cfg, target_base, &operation_dir, report, pass);
    }
}

/// One transaction directory: its journal read, or — when it holds nothing a
/// move ever gets past its first write to — removed.
/// Reconcile one move record, now, in this process — what a person's
/// decision about it ends with (`core::attention::resolve`).
pub(crate) fn reconcile_one_record(
    cfg: &Config,
    target_base: &Path,
    operation_dir: &Path,
) -> ReconcileReport {
    let mut report = ReconcileReport::default();
    let mut pass = Pass {
        ticker: Ticker::none(),
        live: crate::core::jobs::live_workers(),
        deferred: None,
    };
    reconcile_record(cfg, target_base, operation_dir, &mut report, &mut pass);
    report
}

pub(super) fn reconcile_record(
    cfg: &Config,
    target_base: &Path,
    operation_dir: &Path,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let journal = match transactions::read_journal(operation_dir) {
        Ok(journal) => journal,
        // Killed between making the folder and finishing `move.json`: a move
        // writes its manifest next and only then makes its copy, so without
        // one nothing else of it exists. 3.13 called this "invalid" for ever.
        Err(_) if transactions::is_bare_record(operation_dir) => {
            let name = operation_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            match fs::remove_dir_all(operation_dir) {
                Ok(()) => {
                    crate::core::records::remove(&name);
                    report.cleared += 1;
                }
                Err(error) => report.waiting.push(format!(
                    "{}: an unfinished move record that holds nothing could not be removed \
                     ({error}); fastf tries again",
                    crate::util::paths::display_path(operation_dir)
                )),
            }
            return;
        }
        Err(error) => {
            report.unrecoverable.push(format!(
                "{}: malformed/unknown move journal ({error:#}); fastf changed nothing",
                operation_dir.display()
            ));
            return;
        }
    };
    pass.ticker
        .item(&journal.target_folder.display().to_string());
    reconcile_transaction(cfg, target_base, operation_dir, journal, report, pass);
}

/// One transaction, by the table in `src/core/CLAUDE.md` › Recovery.
///
/// **Every message says what is on disk.** "Left source untouched" was about
/// this pass, and it was printed about a source 3.11 had already removed most
/// of; what a reader needs is where the project is, what the original holds,
/// and that fastf removed nothing.
fn reconcile_transaction(
    cfg: &Config,
    target_base: &Path,
    operation_dir: &Path,
    journal: MoveJournal,
    report: &mut ReconcileReport,
    pass: &mut Pass,
) {
    let ticker = pass.ticker;
    // Which project, and where its record is and what state it was left in:
    // a record fastf cannot finish is one the user may have to remove.
    let subject = format!(
        "{} ({}; move record {}, {:?})",
        journal.project_id,
        journal.target_folder.display(),
        crate::util::paths::display_path(operation_dir),
        journal.phase
    );
    if !journal.is_from_this_host() {
        report.unrecoverable.push(format!(
            "{subject}: this move was started on '{}'; run `fastf reconcile` there. fastf \
             changed nothing here. If that machine is gone for good, check the original and \
             the moved copy yourself, then delete the move's record.",
            journal.host.as_deref().unwrap_or("another machine")
        ));
        return;
    }
    let source_base = match configured_real_base(cfg, &journal.source_base) {
        Ok(base) => base,
        // Still one of the bases, only not there now: an unplugged drive, a
        // mount that dropped. The move waits for it, untouched.
        Err(error) if is_configured_base(cfg, &journal.source_base) => {
            report.waiting.push(format!(
                "{subject}: its source base {} is not mounted or does not answer ({error:#}); \
                 fastf changed nothing, and finishes the move once the base is back",
                crate::util::paths::display_path(&journal.source_base)
            ));
            return;
        }
        Err(error) => {
            report.unrecoverable.push(format!(
                "{subject}: its source base is no longer configured ({error:#}); fastf \
                 changed nothing"
            ));
            return;
        }
    };
    // A mount point whose mount is gone is an empty folder, and everything
    // under it would read as gone: wait for the mount instead.
    if let Some(why) = crate::core::records::source_unmounted(&journal.operation_id) {
        report.waiting.push(format!(
            "{subject}: {why}; fastf changed nothing, and finishes the move once it is back"
        ));
        return;
    }
    let source = source_base.join(&journal.source_folder);
    let final_path = target_base.join(&journal.target_folder);
    let staging = operation_dir.join(transactions::STAGING_DIR);
    let retired = transactions::retired_path(&source_base, &journal.operation_id);
    let mut transaction =
        transactions::transaction_from_journal(target_base, operation_dir, journal.clone());
    let shown = |path: &Path| crate::util::paths::display_path(path);

    // **A path that does not answer is not a path that is gone.** Every
    // decision below reads what is on disk, and one read through a mount that
    // has just dropped would decide on nothing: 3.13 took an `EIO` for "gone"
    // and cleared the record of an old copy that was still there. The record
    // waits, untouched, until every path it names answers.
    let staging_is = crate::util::paths::presence(&staging);
    let final_is = crate::util::paths::presence(&final_path);
    let retired_is = crate::util::paths::presence(&retired);
    let source_is = crate::util::paths::presence(&source);
    for (path, presence) in [
        (&staging, &staging_is),
        (&final_path, &final_is),
        (&retired, &retired_is),
        (&source, &source_is),
    ] {
        if let Some(error) = presence.unknown() {
            report.waiting.push(format!(
                "{subject}: {} does not answer ({error}); fastf changed nothing, and finishes \
                 the move once it does",
                shown(path)
            ));
            return;
        }
    }
    let staging_exists = staging_is.is_present();
    let final_exists = final_is.is_present();
    let retired_exists = retired_is.is_present();
    let source_exists = source_is.is_present();

    if journal.operation == Operation::Copy {
        // A copy keeps its source, and its journal never leaves `Copying`:
        // either it published or it did not, and neither touches the source.
        // Both there is a state no copy leaves; neither is a copy killed
        // before it staged anything, whose record is all there is to clear.
        if staging_exists && final_exists {
            report.unrecoverable.push(format!(
                "{subject}: an interrupted copy left an unexpected state at {}; \
                 fastf changed nothing",
                shown(operation_dir)
            ));
            return;
        }
        match transaction.remove() {
            Ok(()) if staging_exists => report.rolled_back += 1,
            Ok(()) => {}
            Err(error) => report.unrecoverable.push(format!(
                "{subject}: could not clear an interrupted copy's record ({error:#})"
            )),
        }
        return;
    }

    // A retired original exists only after a publish. With staging gone and
    // the destination there, a `ReadyToCommit` beside one is a publish whose
    // later phases a power loss took back — the rename reached the disk and
    // the journal did not — and it is finished as what it was.
    // A destination that already holds this project's `PROJECT_INFO.md` is
    // published, whatever the record says: a copy made in its final place is
    // published the moment that file lands, and a crash right after leaves
    // `Copying`; a record on a cloud mount may hold the phase the mount
    // managed to upload rather than the one fastf reached.
    let phase = if (journal.phase == MovePhase::ReadyToCommit
        && retired_exists
        && !staging_exists
        && final_exists)
        || (journal.phase == MovePhase::Copying && transaction.final_is_published())
    {
        MovePhase::CleanupPending
    } else {
        journal.phase
    };
    match phase {
        MovePhase::Copying | MovePhase::ReadyToCommit if retired_exists => {
            report.unrecoverable.push(format!(
                "{subject}: a move that never published has a retired original at {}, \
                 which fastf does not write; fastf changed nothing",
                shown(&retired)
            ));
        }
        MovePhase::Copying if transaction.is_paused() => {
            reconcile_paused(
                &source_base,
                target_base,
                &final_path,
                transaction,
                &subject,
                report,
                pass,
            );
        }
        MovePhase::Copying => {
            if let Err(error) =
                move_cleanup::confirm_identity(&source, &journal.project_id, "source")
            {
                report.unrecoverable.push(format!(
                    "{subject}: an unfinished move's original is not where it was ({error:#}); \
                     fastf changed nothing"
                ));
                return;
            }
            if final_exists {
                // Made in place and not published: fastf's own unfinished
                // copy, which `remove` takes with the record. Anything with a
                // readable identity of its own there is somebody else's.
                let ours = journal.in_place
                    && crate::core::project_info::read_metadata(&final_path)
                        .ok()
                        .flatten()
                        .is_none();
                if !ours {
                    report.unrecoverable.push(format!(
                        "{subject}: an unfinished move's destination {} is taken by something \
                         else; fastf changed nothing",
                        shown(&final_path)
                    ));
                    return;
                }
            }
            match transaction.remove() {
                Ok(()) => report.rolled_back += 1,
                Err(error) => report.unrecoverable.push(format!(
                    "{subject}: could not discard an unfinished move's copy ({error:#})"
                )),
            }
        }
        MovePhase::ReadyToCommit if journal.in_place => {
            report.unrecoverable.push(format!(
                "{subject}: the record is in a state this version never writes; fastf changed \
                 nothing"
            ));
        }
        MovePhase::ReadyToCommit => {
            if let Err(error) =
                move_cleanup::confirm_identity(&source, &journal.project_id, "source")
            {
                report.unrecoverable.push(format!(
                    "{subject}: the original is not where the move left it ({error:#}); \
                     fastf changed nothing"
                ));
                return;
            }
            if staging_exists && !final_exists {
                if crate::util::paths::require_real_directory(&staging, "move staging").is_err() {
                    report.unrecoverable.push(format!(
                        "{subject}: its staging is not a real directory; fastf changed nothing"
                    ));
                    return;
                }
                match transaction.remove() {
                    Ok(()) => report.rolled_back += 1,
                    Err(error) => report.unrecoverable.push(format!(
                        "{subject}: could not discard the verified copy that was never \
                         published ({error:#})"
                    )),
                }
                return;
            }
            if staging_exists || !final_exists {
                report.unrecoverable.push(format!(
                    "{subject}: the move was interrupted in a state fastf cannot read \
                     (staging {}, destination {}); fastf changed nothing",
                    if staging_exists { "present" } else { "absent" },
                    if final_exists { "present" } else { "absent" },
                ));
                return;
            }
            let Some(manifest) = read_manifest_or_report(operation_dir, &subject, report) else {
                return;
            };
            ticker.phase(JobPhase::Verifying, manifest.entries.len() * 2);
            if let Err(error) =
                manifest.verify_recovery_pair(&source, &final_path, ticker.uncancellable())
            {
                report.unrecoverable.push(format!(
                    "{subject}: moved to {}, but the original at {} cannot be matched to \
                     it ({error:#}); fastf removed nothing",
                    shown(&final_path),
                    shown(&source)
                ));
                return;
            }
            let published = transaction.read_published().ok().flatten();
            let cleanup = Cleanup {
                manifest: &manifest,
                published: published.as_ref(),
                source: &source,
                final_path: &final_path,
                project_id: &journal.project_id,
                residue_allowed: false,
                split: false,
                reappeared: false,
                ticker,
            };
            let mut notes = Vec::new();
            let set = move_cleanup::set_aside(transaction, &cleanup, || {
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes)
            });
            report.unrecoverable.append(&mut notes);
            settle_or_defer(set, &cleanup, pass, &subject, report);
        }
        MovePhase::CleanupPending | MovePhase::Retired => {
            if let Err(error) =
                move_cleanup::confirm_identity(&final_path, &journal.project_id, "moved")
            {
                // The original put back where it was, and no moved copy at
                // all: the move is undone, and there is nothing left to
                // finish. A moved copy that is there but reads as another
                // project stays report-only — the record is what ties the two.
                if !retired_exists
                    && !final_exists
                    && move_cleanup::confirm_identity(&source, &journal.project_id, "original")
                        .is_ok()
                {
                    library::refresh_cache(&source);
                    match transaction.remove() {
                        Ok(()) => report.rolled_back += 1,
                        Err(error) => report.unrecoverable.push(format!(
                            "{subject}: the original is back at {}, but the move's record \
                             could not be cleared ({error:#})",
                            shown(&source)
                        )),
                    }
                    return;
                }
                let whole = if retired_exists {
                    format!(
                        "; the original's retired copy at {} may be the only one left — \
                         rename it back to {} to restore the project",
                        shown(&retired),
                        shown(&source)
                    )
                } else {
                    String::new()
                };
                let record = if retired_exists {
                    String::new()
                } else {
                    " Neither it nor the original is where the move left them, so there is \
                     nothing left to finish: delete the move's record once you have looked."
                        .to_string()
                };
                if retired_exists {
                    report.needs(
                        &retired,
                        crate::core::attention::VerdictKind::OnlyCopy,
                        &format!(
                            "the moved copy at {} is not this project any more, so this old \
                             copy may be the only one",
                            shown(&final_path)
                        ),
                    );
                }
                report.unrecoverable.push(format!(
                    "{subject}: the moved copy at {} is not this project any more \
                     ({error:#}){whole}. fastf changed nothing.{record}",
                    shown(&final_path)
                ));
                return;
            }
            // Another project at the original's path is not the original,
            // whatever it holds: it is left alone, and so the move is done.
            let foreign = source_exists
                && crate::core::project_info::read_metadata(&source)
                    .ok()
                    .flatten()
                    .is_some_and(|metadata| metadata.id != journal.project_id);
            // Both the retired copy and something of ours at the original's
            // path: the rename that set it aside stopped part of the way (an
            // S3 bucket renames object by object). What is left at the
            // original's path is this move's residue, and is recorded so, so a
            // later pass that finds the retired copy gone still knows it.
            // Only `CleanupPending` can show it first: `Retired` is written
            // after a rename that finished, and what is at the original's path
            // then is somebody else's (a program that made the folder again).
            let split = !foreign
                && source_exists
                && ((retired_exists && journal.phase == MovePhase::CleanupPending)
                    || transaction.is_split());
            if split
                && retired_exists
                && let Err(error) = transaction.mark_split()
            {
                report.waiting.push(format!(
                    "{subject}: the original was set aside only part of the way, and that could \
                     not be recorded ({error:#}); fastf changed nothing and tries again"
                ));
                return;
            }
            if foreign && !retired_exists {
                let mut notes = Vec::new();
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes);
                report.unrecoverable.append(&mut notes);
                report.unrecoverable.push(format!(
                    "{subject}: moved to {}; the folder now at {} is a different project, so \
                     fastf left it alone.",
                    shown(&final_path),
                    shown(&source)
                ));
                finish_record(transaction, operation_dir, &subject, report);
                return;
            }
            if journal.retire == transactions::RetireStrategy::InPlace {
                reconcile_in_place(
                    InPlace {
                        source_base: &source_base,
                        target_base,
                        operation_dir,
                        source: &source,
                        final_path: &final_path,
                        source_exists,
                        foreign,
                    },
                    &journal,
                    transaction,
                    &subject,
                    report,
                    pass,
                );
                return;
            }
            // Back after it was removed: whatever came back is merged as a
            // residue, and the settle's clock starts again once it is gone.
            let reappeared = retired_exists
                && crate::core::records::get(&journal.operation_id)
                    .is_some_and(|entry| entry.gone_at.is_some());
            if retired_exists {
                crate::core::records::not_gone(&journal.operation_id);
            }
            if !retired_exists
                && !split
                && (journal.phase == MovePhase::Retired || !source_exists)
                && move_cleanup::settling(&journal.operation_id, &source)
            {
                return;
            }
            if !retired_exists && !split && (journal.phase == MovePhase::Retired || !source_exists)
            {
                // Nothing of the original is left for this move to remove —
                // whatever is at its path now is somebody else's.
                let mut notes = Vec::new();
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes);
                report.unrecoverable.append(&mut notes);
                finish_record(transaction, operation_dir, &subject, report);
                return;
            }
            let Some(manifest) = read_manifest_or_report(operation_dir, &subject, report) else {
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
            let cleanup = Cleanup {
                manifest: &manifest,
                published: published.as_ref(),
                source: &source,
                final_path: &final_path,
                project_id: &journal.project_id,
                // 3.11 deleted in place, so its transactions' originals may be
                // part-removed already; a version-3 original never is.
                residue_allowed: journal.source_may_be_partial(),
                split,
                reappeared,
                ticker,
            };
            let mut notes = Vec::new();
            let fate = if retired_exists || split {
                // The rename happened, whole or in part. Whatever is at the
                // original path now is not the original — unless the rename
                // was split, when it is the original's residue, which the
                // housekeeping removes after the retired copy.
                if journal.phase != MovePhase::Retired
                    && let Err(error) = transaction.set_phase(MovePhase::Retired)
                {
                    report.unrecoverable.push(format!(
                        "{subject}: could not record that the original was retired \
                         ({error:#}); fastf changed nothing"
                    ));
                    return;
                }
                bookkeep(&source_base, &journal, target_base, &final_path, &mut notes);
                SetAside::Retired(Box::new(transaction))
            } else {
                move_cleanup::set_aside(transaction, &cleanup, || {
                    bookkeep(&source_base, &journal, target_base, &final_path, &mut notes)
                })
            };
            report.unrecoverable.append(&mut notes);
            settle_or_defer(fate, &cleanup, pass, &subject, report);
        }
    }
}
