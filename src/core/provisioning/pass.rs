//! One pass over every base and every record: the entry points, and the
//! removals a pass hands on until its lock is released.

use super::*;

/// Reconcile scoped v2 state and report obsolete v1 markers without parsing or
/// mutating them.
///
/// **Mutates without holding [`DataLock`]**, which the name is there to admit:
/// this pass resumes copies and removes sources, and doing that while another
/// process is mid-write is exactly what the lock exists to prevent. Every
/// application caller goes through [`reconcile_locked`]; this entry point is for
/// tests that supply their own configuration in memory.
///
/// [`DataLock`]: crate::util::lockfile::DataLock
#[doc(hidden)]
pub fn reconcile_unlocked(cfg: &Config) -> ReconcileReport {
    reconcile_unlocked_with(cfg, Ticker::none())
}

/// [`reconcile_unlocked`], saying how far it has got: its items are what
/// [`list_incomplete`] counts — the header's "needs attention" — and each
/// item's steps are the move's own. A cancel is honoured between items and
/// inside a removal, whose leftover the next pass finishes.
#[doc(hidden)]
pub fn reconcile_unlocked_with(cfg: &Config, ticker: Ticker) -> ReconcileReport {
    let mut pass = Pass {
        ticker,
        live: crate::core::jobs::live_workers(),
        deferred: None,
    };
    let report = reconcile_pass(cfg, &mut pass);
    keep_verdicts(&report);
    report
}

/// A complete pass's verdicts are what needs a person now; a cancelled one
/// did not look at everything, and changes nothing.
fn keep_verdicts(report: &ReconcileReport) {
    if !report.cancelled {
        crate::core::attention::write_verdicts(&report.verdicts);
    }
}

/// What one pass carries: its progress, the live jobs whose records it leaves
/// alone, and — when it is to hand removals on until its lock is released —
/// where it puts them.
pub(super) struct Pass<'a> {
    pub(super) ticker: Ticker<'a>,
    pub(super) live: crate::core::jobs::Live,
    pub(super) deferred: Option<Vec<Deferred>>,
}

impl Pass<'_> {
    /// A record or folder a live job made is that job's until it ends.
    pub(super) fn leaves(&self, operation: &str) -> bool {
        crate::core::jobs::owned_by(operation, &self.live)
    }
}

/// A removal a reconcile decided on under its lock and runs after it.
pub(super) struct Deferred {
    pub(super) housekeeping: move_cleanup::Housekeeping,
    /// For a move: what `record_fate` names.
    pub(super) subject: String,
    pub(super) source: PathBuf,
    pub(super) final_path: PathBuf,
}

fn reconcile_pass(cfg: &Config, pass: &mut Pass) -> ReconcileReport {
    let ticker = pass.ticker;
    let mut report = ReconcileReport::default();
    let expected = list_incomplete(cfg).len();
    ticker.update(|state| state.items = expected);
    // Operations with a transaction anywhere, and retired folders anywhere:
    // a retired folder lives in its source base and its transaction in the
    // target base, so which retired folders are orphans is only known once
    // every base has been walked.
    let mut seen = HashSet::new();
    let mut retired = Vec::new();
    let mut pointers = Vec::new();
    // Every base asked at once, under one deadline: one that stopped
    // answering is waited for by the report, never by the pass.
    let probed =
        crate::util::paths::probe_dirs(&cfg.effective_bases(), crate::util::paths::PROBE_TIMEOUT);
    for (configured, probe) in probed {
        // Asked only of a base that answered its probe.
        let base = probe
            .usable()
            .then(|| crate::util::paths::canonical(&configured).ok())
            .flatten()
            .filter(|base| {
                crate::util::paths::require_real_directory(base, "configured base").is_ok()
            });
        // An unplugged drive, a mount that dropped: ordinary, and nothing a
        // person has to act on. Whatever fastf left there waits for it.
        let Some(base) = base else {
            report.waiting.push(format!(
                "{} is not mounted or does not answer; anything fastf left there waits \
                 until it is back",
                crate::util::paths::display_path(&configured)
            ));
            continue;
        };
        if report.cancelled {
            break;
        }
        reconcile_base(
            cfg,
            &base,
            &mut report,
            &mut seen,
            &mut retired,
            &mut pointers,
            pass,
        );
    }
    // Records the configured bases do not hold: a copy's, beside a
    // destination outside them; a move's, in a base since dropped from
    // `bases`. The data dir's index is where they are known.
    for entry in crate::core::records::all() {
        if report.cancelled || seen.contains(&entry.operation) || pass.leaves(&entry.operation) {
            continue;
        }
        // A delete's record sits beside its folder, where the base's own
        // walk finds it; the index only keeps its settle.
        if entry.is_delete() {
            if crate::util::paths::presence(&entry.record).is_absent()
                && crate::util::paths::presence(&entry.target_base).is_present()
            {
                crate::core::records::remove(&entry.operation);
            }
            continue;
        }
        match crate::util::paths::presence(&entry.record) {
            crate::util::paths::Presence::Present(_) => {
                seen.insert(entry.operation.clone());
                reconcile_record(cfg, &entry.target_base, &entry.record, &mut report, pass);
            }
            // Gone, and the base it was in answers: the index outlived it.
            crate::util::paths::Presence::Absent
                if crate::util::paths::presence(&entry.target_base).is_present() =>
            {
                crate::core::records::remove(&entry.operation);
            }
            _ => {}
        }
    }
    for (path, operation) in retired {
        if !report.cancelled
            && !seen.contains(&operation)
            && !pass.leaves(&operation)
            && entry_exists_quiet(&path)
        {
            reconcile_recordless(cfg, &path, &operation, None, None, &mut report, pass);
        }
    }
    // An in-place retire's pointer whose record is gone: the folder it names
    // is the old copy, unless it is a project again.
    for (pointer_path, operation) in pointers {
        if report.cancelled || seen.contains(&operation) || pass.leaves(&operation) {
            continue;
        }
        let Ok(pointer) = transactions::read_pointer(&pointer_path) else {
            continue;
        };
        let Some(base) = pointer_path.parent() else {
            continue;
        };
        let folder = base.join(&pointer.folder);
        match crate::util::paths::presence(&folder) {
            crate::util::paths::Presence::Absent => {
                if crate::util::fs_retry::remove_file(&pointer_path).is_ok() {
                    report.cleared += 1;
                }
            }
            crate::util::paths::Presence::Present(_)
                if crate::core::project_info::pinfo_path(&folder).is_file() =>
            {
                // The retire never took its `PROJECT_INFO.md`: it is still a
                // project, and the pointer names nothing to remove.
                if crate::util::fs_retry::remove_file(&pointer_path).is_ok() {
                    report.cleared += 1;
                }
            }
            crate::util::paths::Presence::Present(_) => reconcile_recordless(
                cfg,
                &folder,
                &operation,
                Some(pointer_path.clone()),
                Some(pointer.project_id.clone()),
                &mut report,
                pass,
            ),
            crate::util::paths::Presence::Unknown(error) => report.waiting.push(format!(
                "{}: an old copy being emptied does not answer ({error}); fastf finishes \
                 it once it does",
                crate::util::paths::display_path(&folder)
            )),
        }
    }
    report
}

/// Hold the coarse cross-process mutation lock for the whole pass and load the
/// configuration beneath it: which bases get walked is the whole question, and
/// a snapshot taken before the lock could already be stale.
pub fn reconcile_locked() -> ReconcileReport {
    reconcile_locked_with(Ticker::none())
}

/// [`reconcile_locked`], saying how far it has got; see
/// [`reconcile_unlocked_with`].
pub fn reconcile_locked_with(ticker: Ticker) -> ReconcileReport {
    let mut report = ReconcileReport::default();
    ticker.subject("reconcile".to_string());
    let data_lock = crate::util::lockfile::DataLock::acquire_then(|| {
        ticker.phase(JobPhase::Waiting, 0);
    });
    ticker.update(|state| state.holds_lock = data_lock.is_ok());
    let _data_lock = match data_lock {
        Ok(lock) => lock,
        Err(error) => {
            report
                .unrecoverable
                .push(format!("could not serialize reconcile: {error:#}"));
            return report;
        }
    };
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            report
                .unrecoverable
                .push(format!("could not reload configuration: {error:#}"));
            return report;
        }
    };
    let mut pass = Pass {
        ticker,
        live: crate::core::jobs::live_workers(),
        deferred: Some(Vec::new()),
    };
    let mut report = reconcile_pass(&config, &mut pass);
    // Each removal deferred past the lock is this job's own from here: a
    // second reconcile started while it runs leaves it alone. One whose
    // claim could not be written is run under the lock instead, where no
    // second reconcile can be.
    let mut past_the_lock = Vec::new();
    for deferred in pass.deferred.take().unwrap_or_default() {
        match crate::core::jobs::claim(&deferred.housekeeping.operation()) {
            Ok(()) => past_the_lock.push(deferred),
            Err(error) => {
                crate::util::log::info(format!("{error:#}; removing it under the lock"));
                finish_deferred(deferred, ticker, &mut report);
            }
        }
    }
    ticker.update(|state| state.holds_lock = false);
    drop(_data_lock);
    // The removals decided above, now that nothing else waits for them:
    // each re-checks everything it removes, and a record a job started since
    // is not one of these.
    for deferred in past_the_lock {
        if ticker.cancelled() {
            report.cancelled = true;
            break;
        }
        finish_deferred(deferred, ticker, &mut report);
    }
    keep_verdicts(&report);
    report
}

/// Run a deferred removal and report it as the pass would have.
fn finish_deferred(deferred: Deferred, ticker: Ticker, report: &mut ReconcileReport) {
    match deferred.housekeeping {
        move_cleanup::Housekeeping::Deleted(path) => {
            let fate = move_cleanup::Housekeeping::Deleted(path.clone()).run(ticker);
            report_deleted(&path, fate, report);
        }
        deleted @ move_cleanup::Housekeeping::DeletedInPlace(_) => {
            let path = deleted.path();
            let fate = deleted.run(ticker);
            report_deleted(&path, fate, report);
        }
        recordless @ move_cleanup::Housekeeping::Recordless(_) => {
            let path = recordless.path();
            let fate = recordless.run(ticker);
            report_recordless(&path, fate, &deferred.subject, report);
        }
        housekeeping => {
            ticker.update(|state| {
                state.item_label = format!("the old copy of {}", deferred.subject);
            });
            let fate = housekeeping.run(ticker);
            record_fate(
                fate,
                &deferred.subject,
                &deferred.source,
                &deferred.final_path,
                report,
            );
        }
    }
}
