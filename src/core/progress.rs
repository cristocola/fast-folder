//! How the engine says where a long job has got to.
//!
//! A [`Ticker`] is a job's [`Progress`] and cancel flag, handed down to every
//! walk, copy and removal that can take a while, so each one counts what it
//! touches as it touches it. [`Ticker::none`] is the one every caller that has
//! nobody to tell passes; nothing it is handed to behaves differently.
//!
//! **A ticker that cannot cancel is a decision, not a missing flag.** Before a
//! move publishes, stopping undoes it, so its walks stop on a cancel. From the
//! publish on, the moved copy *is* the project and what remains is
//! housekeeping, which a cancel cannot take back: the engine hands those steps
//! [`Ticker::uncancellable`] and marks the job [`Progress::committed`], and a
//! surface says "too late" instead.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::assets::{FinishedStep, JobPhase, JobStatus, Progress};

/// The message a job stopped by its cancel flag ends with.
pub const CANCELLED: &str = "cancelled";

/// A job's progress and cancel flag, borrowed. Copy, so it is passed by value
/// down every call that counts.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ticker<'a> {
    progress: Option<&'a Mutex<Progress>>,
    cancel: Option<&'a AtomicBool>,
}

impl<'a> Ticker<'a> {
    /// Counts nothing and never cancels.
    pub fn none() -> Ticker<'static> {
        Ticker::default()
    }

    pub fn new(progress: &'a Mutex<Progress>, cancel: &'a AtomicBool) -> Self {
        Self {
            progress: Some(progress),
            cancel: Some(cancel),
        }
    }

    /// The same progress, with no cancel: for the steps after a point of no
    /// return.
    pub fn uncancellable(self) -> Self {
        Self {
            cancel: None,
            ..self
        }
    }

    /// Whether a cancel has been asked for, and this ticker honours one.
    pub fn cancelled(self) -> bool {
        self.cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// Change the job's progress in place.
    pub fn update(self, change: impl FnOnce(&mut Progress)) {
        if let Some(progress) = self.progress {
            let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
            change(&mut state);
            state.touch();
        }
    }

    /// Say what the job is, for its log: every step it starts and finishes is
    /// a line there, every entry it touches one more at debug.
    pub fn subject(self, subject: String) {
        crate::util::log::info(format!("{subject}: started"));
        self.update(|state| state.subject = subject);
    }

    /// Say which steps the job will pass through.
    pub fn plan(self, steps: &[JobPhase]) {
        self.update(|state| state.steps = steps.to_vec());
    }

    /// Start a step, `total` long (0 when that is not known). The step before
    /// it is kept in [`Progress::finished`] with what it counted, when it was
    /// one of the planned steps.
    pub fn phase(self, phase: JobPhase, total: usize) {
        self.update(|state| {
            let previous = state.phase;
            if previous != phase
                && state.steps.contains(&previous)
                && !state.finished.iter().any(|step| step.phase == previous)
            {
                state.finished.push(FinishedStep {
                    phase: previous,
                    count: state.step_done,
                });
                log_finished(state, previous, state.step_done);
            }
            if previous != phase && !state.subject.is_empty() {
                let count = if total > 0 {
                    crate::core::assets::count_text(phase, 0, total).replacen("0 of ", "", 1)
                } else {
                    String::new()
                };
                crate::util::log::info(if count.is_empty() {
                    format!("{}: {}", state.subject, phase.as_str())
                } else {
                    format!("{}: {} ({count})", state.subject, phase.as_str())
                });
            }
            state.phase = phase;
            state.step_done = 0;
            state.step_total = total;
            state.current_file.clear();
        });
    }

    /// Keep the step under way among the finished ones, when it was planned
    /// and is not there yet: what a job does when it hands the rest of its
    /// work to a later part with a progress of its own.
    pub fn close_step(self) {
        self.update(|state| {
            let phase = state.phase;
            if state.steps.contains(&phase)
                && !state.finished.iter().any(|step| step.phase == phase)
            {
                let count = state.step_done;
                state.finished.push(FinishedStep { phase, count });
                log_finished(state, phase, count);
            }
        });
    }

    /// One more entry done, at `current`. Answers `false` when the job has
    /// been cancelled and this ticker honours it, so the caller stops.
    pub fn tick(self, current: &Path) -> bool {
        self.update(|state| {
            state.step_done += 1;
            state.current_file.clear();
            state.current_file.push_str(&current.to_string_lossy());
            if !state.subject.is_empty()
                && crate::util::log::enabled(crate::util::log::Level::Debug)
            {
                crate::util::log::debug(format!(
                    "{}: {} {}",
                    state.subject,
                    state.phase.as_str(),
                    state.current_file
                ));
            }
        });
        !self.cancelled()
    }

    /// Start the next of the job's items, about `label`. The total grows when
    /// the job finds more to do than it expected.
    pub fn item(self, label: &str) {
        self.update(|state| {
            state.item += 1;
            state.items = state.items.max(state.item);
            state.item_label.clear();
            state.item_label.push_str(label);
            if !state.subject.is_empty() {
                crate::util::log::info(format!("{}: {} {label}", state.subject, state.item_text()));
            }
        });
    }
}

/// End a job's progress the way its result says: `Done`, or `Cancelled` when
/// the cancel flag stopped it, or `Failed` with the reason.
///
/// **Before this, a job that failed stayed `Running` for ever**, so anything
/// watching it — the app's runtime polls until it is not — watched a dead job.
pub fn settle<T>(progress: &Mutex<Progress>, cancel: &AtomicBool, result: &anyhow::Result<T>) {
    let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
    match result {
        Ok(_) => {
            let last = state.phase;
            if state.steps.contains(&last) && !state.finished.iter().any(|step| step.phase == last)
            {
                let count = state.step_done;
                state.finished.push(FinishedStep { phase: last, count });
                log_finished(&state, last, count);
            }
            state.status = JobStatus::Done;
            state.phase = JobPhase::Done;
            if !state.subject.is_empty() {
                crate::util::log::info(format!("{}: done", state.subject));
            }
        }
        Err(error) if cancel.load(Ordering::Relaxed) => {
            state.status = JobStatus::Cancelled;
            if !state.subject.is_empty() {
                crate::util::log::info(format!("{}: cancelled ({error:#})", state.subject));
            }
        }
        Err(error) => {
            state.status = JobStatus::Failed;
            state.error = Some(format!("{error:#}"));
            if !state.subject.is_empty() {
                crate::util::log::error(format!("{}: failed: {error:#}", state.subject));
            }
        }
    }
    state.touch();
}

/// A finished step's line in the log: `move … : copied 1301 files`.
fn log_finished(state: &Progress, phase: JobPhase, count: usize) {
    if state.subject.is_empty() {
        return;
    }
    let counted = crate::core::assets::count_text(phase, count, 0);
    crate::util::log::info(if counted.is_empty() {
        format!("{}: {}", state.subject, phase.past())
    } else {
        format!("{}: {} {counted}", state.subject, phase.past())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_kept_with_its_count_when_the_next_begins() {
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);
        let ticker = Ticker::new(&progress, &cancel);
        ticker.plan(&[JobPhase::Scanning, JobPhase::Removing]);
        ticker.phase(JobPhase::Scanning, 0);
        assert!(ticker.tick(Path::new("a")));
        assert!(ticker.tick(Path::new("b")));
        ticker.phase(JobPhase::Removing, 2);
        assert!(ticker.tick(Path::new("a")));
        let state = progress.lock().unwrap().clone();
        assert_eq!(
            state.finished,
            vec![FinishedStep {
                phase: JobPhase::Scanning,
                count: 2
            }]
        );
        assert_eq!(
            (state.phase, state.step_done, state.step_total),
            (JobPhase::Removing, 1, 2)
        );
        assert_eq!(state.current_file, "a");
        settle(&progress, &cancel, &Ok(()));
        let state = progress.lock().unwrap().clone();
        assert_eq!(state.status, JobStatus::Done);
        assert_eq!(state.finished.len(), 2, "the last step is kept too");
    }

    #[test]
    fn a_ticker_that_cannot_cancel_keeps_going() {
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(true);
        let ticker = Ticker::new(&progress, &cancel);
        assert!(!ticker.tick(Path::new("a")));
        assert!(ticker.uncancellable().tick(Path::new("b")));
        assert!(Ticker::none().tick(Path::new("c")));
    }

    #[test]
    fn a_failed_or_cancelled_job_says_so() {
        let progress = Mutex::new(Progress::new(&[]));
        let cancel = AtomicBool::new(false);
        settle::<()>(&progress, &cancel, &Err(anyhow::anyhow!("disk full")));
        let state = progress.lock().unwrap().clone();
        assert_eq!(state.status, JobStatus::Failed);
        assert_eq!(state.error.as_deref(), Some("disk full"));

        cancel.store(true, Ordering::Relaxed);
        settle::<()>(&progress, &cancel, &Err(anyhow::anyhow!("stopped")));
        assert_eq!(progress.lock().unwrap().status, JobStatus::Cancelled);
    }
}
