//! A job's worker: `fastf --fastf-job <id>`, started detached by
//! `core::jobs::start` and by nothing else.
//!
//! It holds the job's lock for its whole life — which is how every other fastf
//! knows it is alive — reads the request, runs it, and keeps `state.json`
//! current: a watcher thread copies the engine's progress in a few times a
//! second and turns a `cancel` file into the engine's cancel flag. It never
//! prints: its output goes nowhere. Everything it has to say is in its state,
//! its messages and its own log.
//!
//! **Moves first, housekeeping after.** A batch sets every project in its new
//! base before removing any old copy, so a slow cloud mount's removals never
//! hold up the next project; and the data lock is taken per item, so other
//! fastf processes get in between.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::core::assets::{JobPhase, JobStatus, Progress};
use crate::core::jobs::{ItemReport, JobItem, JobKind, JobRequest, JobState};
use crate::core::library::Project;
use crate::core::move_cleanup::Housekeeping;
use crate::core::progress::Ticker;
use crate::util::messages::{Level, Message};

/// How often the state file is rewritten while the job runs.
const WRITE_EVERY: Duration = Duration::from_millis(250);

/// Run job `id`, then leave. The exit code says only whether the worker ran:
/// the job's own outcome is in its state.
pub fn run(id: &str) -> i32 {
    let Ok(dir) = crate::core::jobs::dir(id) else {
        return 2;
    };
    // Held until this process ends. A second worker for the same job waits a
    // moment and leaves.
    let Ok(_lock) =
        crate::util::lockfile::DataLock::acquire_at(&dir.join("lock"), Duration::from_secs(2))
    else {
        return 3;
    };
    let Some(request) = crate::core::jobs::read_request(id) else {
        return 4;
    };
    // A terminal closing does not concern a job; a SIGTERM is a cancel.
    #[cfg(unix)]
    // SAFETY: setting a signal's disposition to ignore is always sound.
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    if let Ok(config) = crate::core::config::Config::load()
        && let Some(level) = crate::util::log::Level::parse(&config.log_level)
    {
        crate::util::log::set_level(level);
    }
    crate::util::log::set_job(Some((id.to_string(), dir.join("log"))));
    crate::util::log::info(format!(
        "job {id}: {:?} of {} item(s)",
        request.kind,
        request.items.len()
    ));

    let cancel: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
    crate::util::lockfile::wait_patiently(cancel);
    let progress = Arc::new(Mutex::new(Progress::new(&[])));
    let state = Arc::new(Mutex::new(JobState {
        version: crate::core::jobs::STATE_VERSION,
        id: id.to_string(),
        kind: Some(request.kind),
        pid: std::process::id(),
        started: crate::util::time::now_iso8601(),
        updated: crate::util::time::now_iso8601(),
        status: JobStatus::Running,
        items: vec![ItemReport::default(); request.items.len()],
        ..JobState::default()
    }));
    write(&state, &progress);

    let finished = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (state, progress, finished) = (state.clone(), progress.clone(), finished.clone());
        let id = id.to_string();
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if crate::core::jobs::cancel_requested(&id) || crate::util::interrupt::is_set() {
                    cancel.store(true, Ordering::Relaxed);
                }
                write(&state, &progress);
                std::thread::sleep(WRITE_EVERY);
            }
        })
    };

    let job = Job {
        id,
        said: Mutex::new(Vec::new()),
        state: &state,
        progress: &progress,
        cancel,
    };
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run(&request)));
    finished.store(true, Ordering::Relaxed);
    let _ = watcher.join();

    let mut settled = state.lock().unwrap_or_else(|e| e.into_inner());
    settled.progress = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match ran {
        Ok(status) => settled.status = status,
        Err(_) => {
            settled.status = JobStatus::Failed;
            settled.summary = "the job's worker stopped on an internal error; `fastf log` \
                               has what it did, and `fastf reconcile` finishes it"
                .to_string();
            crate::util::log::error(format!("job {id}: panicked"));
        }
    }
    settled.updated = crate::util::time::now_iso8601();
    let _ = crate::core::jobs::write_state(&settled);
    crate::util::log::info(format!("job {id}: {}", settled.summary));
    0
}

/// Write the state as it stands, with the engine's latest progress in it.
fn write(state: &Mutex<JobState>, progress: &Mutex<Progress>) {
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.progress = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
    state.updated = crate::util::time::now_iso8601();
    let _ = crate::core::jobs::write_state(&state);
}

struct Job<'a> {
    id: &'a str,
    /// Every message the job has said, in order.
    said: Mutex<Vec<String>>,
    state: &'a Mutex<JobState>,
    progress: &'a Mutex<Progress>,
    cancel: &'static AtomicBool,
}

impl Job<'_> {
    fn run(&self, request: &JobRequest) -> JobStatus {
        match request.kind {
            JobKind::Move => self.moves(&request.items),
            JobKind::Copy => self.copies(&request.items),
            JobKind::Delete => self.deletes(&request.items),
            JobKind::Reconcile => self.reconcile(),
        }
    }

    fn ticker(&self) -> Ticker<'_> {
        Ticker::new(self.progress, self.cancel)
    }

    /// A fresh progress for item `index` of `count`.
    fn begin_item(&self, index: usize, count: usize, label: &str) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        *progress = Progress {
            item: if count > 1 { index + 1 } else { 0 },
            items: if count > 1 { count } else { 0 },
            item_label: label.to_string(),
            ..Progress::new(&[])
        };
    }

    fn report(&self, index: usize, report: ItemReport) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = state.items.get_mut(index) {
            *slot = report;
        }
        drop(state);
        write(self.state, self.progress);
    }

    fn own(&self, operation: String) {
        if operation.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.operations.push(operation);
    }

    /// The job in one sentence: a batch's count, or — for a job of one item —
    /// what was said about that item, which is the more useful of the two.
    fn summarise(&self, summary: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let said = self.said.lock().unwrap_or_else(|e| e.into_inner());
        state.summary = match said.as_slice() {
            [one] if state.items.len() <= 1 => one.clone(),
            _ => summary,
        };
    }

    fn say(&self, level: Level, text: &str) {
        crate::util::messages::append(&Message::now(level, self.id, text));
        self.said
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text.to_string());
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn moves(&self, items: &[JobItem]) -> JobStatus {
        let mut pending: Vec<(
            usize,
            Project,
            crate::core::library::MoveOutcome,
            Housekeeping,
        )> = Vec::new();
        let mut failed = 0;
        let mut moved = 0;
        let mut stopped = false;
        for (index, item) in items.iter().enumerate() {
            if self.cancelled() {
                stopped = true;
                break;
            }
            self.begin_item(index, items.len(), &item.name);
            let result = project_for(item).and_then(|project| {
                crate::core::move_engine::move_project_in_parts(
                    &project,
                    Path::new(&item.target),
                    self.progress,
                    self.cancel,
                )
                .map(|parts| (project, parts))
            });
            match result {
                Ok((project, (outcome, housekeeping))) => {
                    moved += 1;
                    self.ticker().close_step();
                    let mut report = move_report(&project, &outcome);
                    match housekeeping {
                        Some(housekeeping) => {
                            self.own(housekeeping.operation());
                            report.lines.push("removing the old copy".to_string());
                            pending.push((index, project, outcome, housekeeping));
                        }
                        None => {
                            report.done = true;
                            self.say_moved(&project, &outcome);
                            crate::core::progress::settle(self.progress, self.cancel, &Ok(()));
                        }
                    }
                    self.report(index, report);
                }
                Err(error) => {
                    crate::core::progress::settle::<()>(
                        self.progress,
                        self.cancel,
                        &Err(anyhow::anyhow!("{error:#}")),
                    );
                    if self.cancelled() {
                        stopped = true;
                        self.say(
                            Level::Warn,
                            &format!(
                                "the move of {} {} was cancelled; nothing was moved",
                                item.id, item.name
                            ),
                        );
                        self.report(index, failed_report(item, "move", &error, true));
                        break;
                    }
                    failed += 1;
                    self.say(
                        Level::Error,
                        &format!("the move of {} {} failed: {error:#}", item.id, item.name),
                    );
                    self.report(index, failed_report(item, "move", &error, false));
                }
            }
        }
        // The housekeeping, once every project is in its new place. It cannot
        // be cancelled: what is left is hidden and redundant, and a removal
        // stopped part of the way helps nobody.
        for (index, project, outcome, housekeeping) in pending {
            self.begin_item(
                index,
                items.len(),
                &format!("the old copy of {}", project.name),
            );
            let ticker = self.ticker();
            ticker.subject(format!(
                "move {} {}: the old copy",
                project.id, project.name
            ));
            ticker.plan(&[JobPhase::Checking, JobPhase::Removing, JobPhase::Clearing]);
            ticker.update(|state| state.committed = true);
            let outcome = crate::core::move_engine::finish_housekeeping(
                outcome,
                Some(housekeeping),
                ticker.uncancellable(),
            );
            crate::core::progress::settle(self.progress, self.cancel, &Ok(()));
            let mut report = move_report(&project, &outcome);
            report.done = true;
            self.say_moved(&project, &outcome);
            self.report(index, report);
        }
        self.summarise(batch_summary("moved", moved, failed, stopped, items.len()));
        status_of(failed, stopped)
    }

    fn say_moved(&self, project: &Project, outcome: &crate::core::library::MoveOutcome) {
        let warning = outcome.source.warning(&project.path);
        let text = format!(
            "moved {} {} to {}{}",
            outcome.project.id,
            outcome.project.name,
            crate::util::paths::display_path(&outcome.project.path),
            warning
                .as_ref()
                .map(|warning| format!("; {warning}"))
                .unwrap_or_default()
        );
        self.say(
            if warning.is_some() {
                Level::Warn
            } else {
                Level::Good
            },
            &text,
        );
    }

    fn copies(&self, items: &[JobItem]) -> JobStatus {
        let (mut copied, mut failed, mut stopped) = (0, 0, false);
        for (index, item) in items.iter().enumerate() {
            if self.cancelled() {
                stopped = true;
                break;
            }
            self.begin_item(index, items.len(), &item.name);
            let result = project_for(item).and_then(|project| {
                crate::core::operations::copy_project(
                    &project,
                    Path::new(&item.target),
                    self.progress,
                    self.cancel,
                )
                .map(|outcome| (project, outcome))
            });
            match result {
                Ok((project, outcome)) => {
                    copied += 1;
                    let (files, bytes) = outcome.copied;
                    let path = crate::util::paths::display_path(&outcome.path);
                    self.say(
                        Level::Good,
                        &format!(
                            "copied {} {} to {path}; the original is untouched",
                            project.id, project.name
                        ),
                    );
                    self.report(
                        index,
                        ItemReport {
                            headline: format!("Copied {} {}", project.id, project.name),
                            lines: vec![
                                format!("to {path}"),
                                format!(
                                    "{}, verified — the original is untouched",
                                    crate::core::transactions::copied_summary(
                                        files,
                                        outcome.links,
                                        bytes
                                    )
                                ),
                            ],
                            notes: outcome.link_notes.clone(),
                            done: true,
                            ..ItemReport::default()
                        },
                    );
                }
                Err(error) => {
                    if self.cancelled() {
                        stopped = true;
                        self.report(index, failed_report(item, "copy", &error, true));
                        break;
                    }
                    failed += 1;
                    self.say(
                        Level::Error,
                        &format!("the copy of {} {} failed: {error:#}", item.id, item.name),
                    );
                    self.report(index, failed_report(item, "copy", &error, false));
                }
            }
        }
        self.summarise(batch_summary(
            "copied",
            copied,
            failed,
            stopped,
            items.len(),
        ));
        status_of(failed, stopped)
    }

    fn deletes(&self, items: &[JobItem]) -> JobStatus {
        let mut pending = Vec::new();
        let (mut deleted, mut failed, mut stopped) = (0, 0, false);
        for (index, item) in items.iter().enumerate() {
            if self.cancelled() {
                stopped = true;
                break;
            }
            self.begin_item(index, items.len(), &item.name);
            match project_for(item)
                .and_then(|project| crate::core::library::delete_project_in_parts(&project))
            {
                Ok(housekeeping) => {
                    deleted += 1;
                    self.ticker().close_step();
                    self.own(housekeeping.operation());
                    let report = ItemReport {
                        headline: format!("Deleted {} ({})", item.id, item.path),
                        lines: vec!["removing its folder".to_string()],
                        ..ItemReport::default()
                    };
                    self.report(index, report);
                    pending.push((index, item, housekeeping));
                }
                Err(error) => {
                    failed += 1;
                    self.say(
                        Level::Error,
                        &format!("the delete of {} {} failed: {error:#}", item.id, item.name),
                    );
                    self.report(index, failed_report(item, "delete", &error, false));
                }
            }
        }
        for (index, item, housekeeping) in pending {
            self.begin_item(index, items.len(), &item.name);
            let ticker = self.ticker();
            ticker.subject(format!("delete {} {}", item.id, item.name));
            ticker.plan(&[JobPhase::Removing]);
            ticker.update(|state| state.committed = true);
            let warning = crate::core::library::finish_delete(
                &item.name,
                housekeeping,
                ticker.uncancellable(),
            );
            crate::core::progress::settle(self.progress, self.cancel, &Ok(()));
            self.say(
                if warning.is_some() {
                    Level::Warn
                } else {
                    Level::Good
                },
                &warning
                    .clone()
                    .unwrap_or_else(|| format!("deleted {} {}", item.id, item.name)),
            );
            self.report(
                index,
                ItemReport {
                    headline: format!("Deleted {} ({})", item.id, item.path),
                    warning,
                    done: true,
                    ..ItemReport::default()
                },
            );
        }
        self.summarise(batch_summary(
            "deleted",
            deleted,
            failed,
            stopped,
            items.len(),
        ));
        status_of(failed, stopped)
    }

    fn reconcile(&self) -> JobStatus {
        let result = crate::core::operations::reconcile_with(self.progress, self.cancel);
        match result {
            Ok(report) => {
                let summary = report.summary();
                self.say(
                    if report.needs_a_look() {
                        Level::Warn
                    } else {
                        Level::Good
                    },
                    &summary,
                );
                let cancelled = report.cancelled;
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.summary = summary;
                state.reconcile = Some(report);
                if cancelled {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Done
                }
            }
            Err(error) => {
                self.say(Level::Error, &format!("reconcile failed: {error:#}"));
                self.summarise(format!("reconcile failed: {error:#}"));
                JobStatus::Failed
            }
        }
    }
}

/// The project a request names, as it is on disk now; the engine revalidates
/// it again under the lock.
fn project_for(item: &JobItem) -> anyhow::Result<Project> {
    let path = PathBuf::from(&item.path);
    let base = PathBuf::from(&item.base);
    crate::core::library::project_at(&base, &path).ok_or_else(|| {
        anyhow::anyhow!(
            "{} {} is no longer a project at {}",
            item.id,
            item.name,
            crate::util::paths::display_path(&path)
        )
    })
}

/// A move's outcome in the lines the command line prints.
fn move_report(project: &Project, outcome: &crate::core::library::MoveOutcome) -> ItemReport {
    let moved = &outcome.project;
    ItemReport {
        headline: format!("Moved {} {}", moved.id, moved.name),
        lines: vec![
            format!("from {}", crate::util::paths::display_path(&project.path)),
            format!("to   {}", crate::util::paths::display_path(&moved.path)),
            match outcome.copied {
                Some((files, bytes)) => format!(
                    "copied {}, verified",
                    crate::core::transactions::copied_summary(files, outcome.links, bytes)
                ),
                None => "renamed on the same filesystem, nothing copied".to_string(),
            },
        ],
        notes: outcome.link_notes.clone(),
        warning: outcome.source.warning(&project.path),
        ..ItemReport::default()
    }
}

fn failed_report(item: &JobItem, verb: &str, error: &anyhow::Error, cancelled: bool) -> ItemReport {
    ItemReport {
        headline: if cancelled {
            format!("The {verb} of {} {} was cancelled", item.id, item.name)
        } else {
            format!("The {verb} of {} {} failed", item.id, item.name)
        },
        error: Some(format!("{error:#}")),
        done: true,
        ..ItemReport::default()
    }
}

fn batch_summary(verb: &str, done: usize, failed: usize, stopped: bool, total: usize) -> String {
    let mut summary = format!("{verb} {done} of {total}");
    if failed > 0 {
        summary.push_str(&format!(", {failed} failed"));
    }
    if stopped {
        summary.push_str(" — cancelled, the rest were not started");
    }
    summary
}

fn status_of(failed: usize, stopped: bool) -> JobStatus {
    if stopped {
        JobStatus::Cancelled
    } else if failed > 0 {
        JobStatus::Failed
    } else {
        JobStatus::Done
    }
}
