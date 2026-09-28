//! The app's side of jobs: a move, a copy, a delete or a reconcile runs in a
//! process of its own (`core::jobs`), and the app follows it.
//!
//! The app starts a job (`Effect::StartJob`), and the runtime reads `jobs/`
//! a few times a second while one it shows is alive, once a second otherwise
//! (`Msg::Jobs`). Nothing here owns a job: quitting the app leaves it running,
//! a second app sees the same one, and a job that ended while no app was open
//! is reported when the next one starts. **The dialog follows one job and Esc
//! hides it**; the header's chip keeps saying what is running.

use std::collections::HashSet;
use std::path::PathBuf;

use super::App;
use crate::core::assets::{JobStatus, Progress};
use crate::core::jobs::{JobItem, JobKind, JobView};
use crate::core::library::Project;
use crate::tui::app::modal::{MessageLevel, Modal};
use crate::tui::effect::{Effect, ListChange};

/// Every job the app knows of, and which one its dialog follows.
#[derive(Debug, Default)]
pub struct Background {
    /// The latest read of `jobs/`, newest first.
    pub jobs: Vec<JobView>,
    /// The job the progress dialog follows.
    pub following: Option<String>,
    /// Esc hid the dialog; the job goes on.
    pub hidden: bool,
    /// A job being started: its worker has not answered yet.
    pub starting: Option<&'static str>,
    /// Jobs this session started, whose end it reports whatever else did.
    pub started_here: HashSet<String>,
    /// Jobs whose end has been reported, or that ended before this session and
    /// were already seen.
    pub reported: HashSet<String>,
    /// Whether a read of `jobs/` has landed yet: the first one reports what
    /// ended while no app was open.
    pub looked: bool,
    /// Ctrl-C came while the job was still starting: it is asked to stop the
    /// moment its worker answers.
    pub cancel_when_started: bool,
    /// The app's own reconcile, while it runs (`tui::app::attention`).
    pub auto: Option<String>,
    /// When the app last started one (`App::elapsed_ms`).
    pub auto_at: Option<u64>,
}

impl Background {
    pub fn job(&self, id: &str) -> Option<&JobView> {
        self.jobs.iter().find(|job| job.id == id)
    }

    /// The job the dialog follows, while it is not hidden.
    pub fn shown(&self) -> Option<&JobView> {
        if self.hidden {
            return None;
        }
        self.following.as_deref().and_then(|id| self.job(id))
    }

    /// Jobs whose worker is running.
    pub fn live(&self) -> impl Iterator<Item = &JobView> {
        self.jobs.iter().filter(|job| job.alive)
    }

    /// The live job holding the data lock, if one does: while it does, every
    /// change the app would make waits for it.
    pub fn lock_holder(&self) -> Option<&JobView> {
        self.live().find(|job| {
            job.state
                .as_ref()
                .is_some_and(|state| state.progress.holds_lock)
        })
    }

    /// Whether the runtime should read `jobs/` at the quick pace.
    pub fn watching(&self) -> bool {
        self.starting.is_some() || self.following.is_some() || self.live().next().is_some()
    }
}

/// A job in a few words, for the header's chip: `moving ID0248`, `moving 3
/// projects`, `reconciling`.
pub fn chip_title(job: &JobView) -> String {
    let verb = match job.kind() {
        Some(JobKind::Move) => "moving",
        Some(JobKind::Copy) => "copying",
        Some(JobKind::Delete) => "deleting",
        Some(JobKind::Reconcile) => "reconciling",
        None => "working",
    };
    match job.request.as_ref().map(|request| request.items.as_slice()) {
        Some([one]) => format!("{verb} {}", one.id),
        Some(many) if many.len() > 1 => format!("{verb} {} projects", many.len()),
        _ => verb.to_string(),
    }
}

/// A job in a few words as a list names it: `move ID0248`, `move 3
/// projects`, `reconcile`.
pub fn short_title(job: &JobView) -> String {
    let verb = job.kind().map_or("job", JobKind::verb);
    match job.request.as_ref().map(|request| request.items.as_slice()) {
        Some([one]) => format!("{verb} {}", one.id),
        Some(many) if many.len() > 1 => format!("{verb} {} projects", many.len()),
        _ => verb.to_string(),
    }
}

/// A job's current step, as the chip and the jobs page say it.
pub fn step_of(job: &JobView) -> String {
    let Some(state) = job.state.as_ref() else {
        return "starting".to_string();
    };
    let item = state.progress.item_text();
    let step = state.progress.step_text();
    if item.is_empty() {
        step
    } else {
        format!("{item} · {step}")
    }
}

impl App {
    /// Start a job over `projects` — a move or a copy to `target`, a delete —
    /// or, with no projects, a reconcile.
    pub(super) fn start_background(
        &mut self,
        kind: JobKind,
        projects: Vec<Project>,
        target: Option<PathBuf>,
    ) -> Vec<Effect> {
        if let Some(starting) = self.background.starting {
            self.warn(format!("still {starting}"));
            return Vec::new();
        }
        let mut items = Vec::new();
        for project in &projects {
            match JobItem::of(project, target.as_deref()) {
                Ok(item) => items.push(item),
                Err(error) => {
                    self.error(format!("error: {error:#}"));
                    return Vec::new();
                }
            }
        }
        self.background.starting = Some(match kind {
            JobKind::Move => "moving…",
            JobKind::Copy => "copying…",
            JobKind::Delete => "deleting…",
            JobKind::Reconcile => "reconciling…",
        });
        self.background.hidden = false;
        vec![Effect::StartJob { kind, items }]
    }

    pub(super) fn on_job_started(&mut self, started: Result<String, String>) -> Vec<Effect> {
        self.background.starting = None;
        match started {
            Ok(id) => {
                self.background.started_here.insert(id.clone());
                self.background.following = Some(id.clone());
                let mut effects = vec![Effect::WatchJobs];
                if std::mem::take(&mut self.background.cancel_when_started) {
                    effects.push(Effect::CancelJob(id));
                }
                effects
            }
            Err(error) => {
                self.error(format!("error: {error}"));
                Vec::new()
            }
        }
    }

    /// The app's own reconcile started, or could not: either way quietly.
    pub(super) fn on_auto_reconcile_started(
        &mut self,
        started: Result<String, String>,
    ) -> Vec<Effect> {
        match started {
            Ok(id) => {
                self.background.auto = Some(id);
                vec![Effect::WatchJobs]
            }
            Err(error) => {
                crate::util::log::warn(format!("the app's reconcile did not start: {error}"));
                Vec::new()
            }
        }
    }

    /// The app's own reconcile ended: a word only when it found something that
    /// needs you, or finished something; the header says the rest.
    fn report_auto_reconcile(&mut self, job: &JobView) -> Vec<Effect> {
        self.background.auto = None;
        let mut effects = vec![Effect::MarkSeen(job.id.clone())];
        if let Some(report) = job
            .state
            .as_ref()
            .and_then(|state| state.reconcile.as_ref())
        {
            let finished = report.completed + report.cleared + report.rolled_back + report.restored;
            let key = crate::tui::command::key_of(crate::tui::command::CommandId::Attention);
            if !report.verdicts.is_empty() {
                let n = report.verdicts.len();
                self.warn(format!(
                    "{n} unfinished thing{} need{} you — {key} shows {}",
                    crate::util::plural::s(n),
                    crate::util::plural::of(n, "s", ""),
                    crate::util::plural::of(n, "it", "them")
                ));
            } else if finished > 0 {
                self.info(format!(
                    "finished {finished} leftover{} of earlier moves and deletes",
                    crate::util::plural::s(finished)
                ));
            }
        }
        effects.extend(self.apply_change(ListChange::Reload));
        effects
    }

    /// A read of `jobs/` landed: keep it, and report every job that has ended
    /// and not been reported — this session's, and any nobody has seen.
    pub(super) fn on_jobs(&mut self, jobs: Vec<JobView>) -> Vec<Effect> {
        let first = !self.background.looked;
        self.background.looked = true;
        self.background.jobs = jobs;
        let mut effects = Vec::new();
        let ended: Vec<JobView> = self
            .background
            .jobs
            .iter()
            // Ended: finished and said so, or killed. A job still starting —
            // no worker yet, no state — is neither, and is looked at again.
            .filter(|job| {
                !job.alive
                    && (job.interrupted()
                        || job
                            .state
                            .as_ref()
                            .is_some_and(|state| state.status != JobStatus::Running))
            })
            .filter(|job| !self.background.reported.contains(&job.id))
            .cloned()
            .collect();
        let mut reload = false;
        for job in ended {
            self.background.reported.insert(job.id.clone());
            if self.background.auto.as_deref() == Some(job.id.as_str()) {
                effects.extend(self.report_auto_reconcile(&job));
                continue;
            }
            let ours = self.background.started_here.contains(&job.id);
            // This session's jobs are reported, and — once, on the first
            // look — any that ended while no app was open and nobody has
            // seen. One another surface is following reports itself there;
            // here it only changes the list.
            if ours || (first && !job.seen) {
                effects.extend(self.report_job(&job, !ours));
            } else if !first {
                reload = true;
            }
        }
        if reload {
            effects.extend(self.apply_change(ListChange::Reload));
        }
        self.refresh_activity_jobs();
        effects
    }

    /// Say how a job ended, put the list right, and mark it seen.
    fn report_job(&mut self, job: &JobView, while_away: bool) -> Vec<Effect> {
        if self.background.following.as_deref() == Some(job.id.as_str()) {
            self.background.following = None;
            self.background.hidden = false;
        }
        let title = job.title();
        let away = if while_away {
            "while fastf was closed: "
        } else {
            ""
        };
        let mut effects = vec![Effect::MarkSeen(job.id.clone())];
        let Some(state) = job.state.as_ref().filter(|_| !job.interrupted()) else {
            self.warn(format!(
                "{away}the {title} stopped when its process ended; Reconcile ({}) finishes it",
                crate::tui::command::key_of(crate::tui::command::CommandId::Reconcile)
            ));
            effects.extend(self.apply_change(ListChange::Reload));
            return effects;
        };

        // A mark is the retry list: an item that went through loses its mark.
        // A deleted project's row goes by itself, without a rescan; anything
        // else a job did lands elsewhere, and the list is read again.
        let mut removed = Vec::new();
        if let Some(request) = &job.request {
            for (item, report) in request.items.iter().zip(&state.items) {
                if report.done && report.error.is_none() {
                    let path = PathBuf::from(&item.path);
                    self.library.marks.remove(&path);
                    removed.push(path);
                }
            }
        }
        let change = |app: &mut Self| -> Vec<Effect> {
            if job.kind() == Some(JobKind::Delete) {
                removed
                    .iter()
                    .flat_map(|path| app.apply_change(ListChange::Removed { path: path.clone() }))
                    .chain([Effect::LoadSummary])
                    .collect()
            } else {
                app.apply_change(ListChange::Reload)
            }
        };
        let warnings: Vec<String> = state
            .items
            .iter()
            .filter_map(|item| item.warning.clone())
            .chain(
                state
                    .items
                    .iter()
                    .flat_map(|item| item.notes.iter().map(|note| format!("note: {note}"))),
            )
            .collect();
        let failures: Vec<String> = state
            .items
            .iter()
            .filter_map(|item| {
                item.error
                    .as_ref()
                    .map(|error| format!("{}: {error}", item.headline))
            })
            .collect();
        let summary = for_the_app(&format!("{away}{}", state.summary));
        let mut body = Vec::new();
        if !failures.is_empty() {
            body.push(failures.join("\n"));
        }
        if !warnings.is_empty() {
            body.push(warnings.join("\n\n"));
        }
        if let Some(report) = &state.reconcile {
            body.extend(reconcile_notes(report));
        }
        let body = for_the_app(&body.join("\n\n"));
        match state.status {
            JobStatus::Done if body.is_empty() => self.good(summary),
            JobStatus::Done | JobStatus::Cancelled | JobStatus::Paused | JobStatus::Unknown => {
                if body.is_empty() {
                    self.warn(summary);
                } else {
                    self.warn(format!("{summary}  —  see the report"));
                    self.modals
                        .push(Modal::message("needs a look", body, MessageLevel::Warn));
                }
            }
            JobStatus::Failed | JobStatus::Running => {
                self.error(format!("error: {summary}"));
                if !body.is_empty() {
                    self.modals
                        .push(Modal::message("error", body, MessageLevel::Error));
                }
            }
        }
        effects.extend(change(self));
        effects
    }

    /// Esc on the dialog: hide it. The job goes on, and the chip says so.
    pub(super) fn hide_job_dialog(&mut self) -> Vec<Effect> {
        self.background.hidden = true;
        Vec::new()
    }

    /// Ctrl-C on the dialog: ask the job to stop — or, past its point of no
    /// return, say that it is too late.
    pub(super) fn cancel_followed_job(&mut self) -> Vec<Effect> {
        if self.background.starting.is_some() {
            self.background.cancel_when_started = true;
            self.info("cancelling…");
            return Vec::new();
        }
        let Some(job) = self.background.shown() else {
            return Vec::new();
        };
        let (id, committed) = (
            job.id.clone(),
            job.state
                .as_ref()
                .is_some_and(|state| state.progress.committed),
        );
        let progress = job
            .state
            .as_ref()
            .map(|state| state.progress.clone())
            .unwrap_or_default();
        if committed {
            let late = crate::tui::app::jobs::too_late(&progress);
            self.info(late);
            return Vec::new();
        }
        self.info("cancelling…");
        vec![Effect::CancelJob(id)]
    }

    /// Whether the progress dialog is up.
    pub fn job_dialog_up(&self) -> bool {
        self.background.shown().is_some()
            || (self.background.starting.is_some() && !self.background.hidden)
    }

    /// The progress the dialog draws: the followed job's, or a blank one
    /// while its worker is being started.
    pub fn shown_progress(&self) -> Option<Progress> {
        if let Some(job) = self.background.shown() {
            return Some(
                job.state
                    .as_ref()
                    .map(|state| state.progress.clone())
                    .unwrap_or_default(),
            );
        }
        self.background
            .starting
            .filter(|_| !self.background.hidden)
            .map(|_| Progress::new(&[]))
    }

    /// The dialog's title: the job's verb.
    pub fn shown_title(&self) -> &'static str {
        if let Some(starting) = self.background.starting {
            return starting;
        }
        match self.background.shown().and_then(JobView::kind) {
            Some(JobKind::Move) => "moving",
            Some(JobKind::Copy) => "copying",
            Some(JobKind::Delete) => "deleting",
            Some(JobKind::Reconcile) => "reconciling",
            None => "working",
        }
    }
}

/// The command line names `fastf reconcile`; in the app a reconcile starts by
/// itself, and `!` lists what is unfinished and can run one now.
pub(crate) fn for_the_app(text: &str) -> String {
    text.replace("`fastf reconcile`", "a reconcile (`!`)")
}

/// What a reconcile's report says beyond its summary, one paragraph each.
pub(crate) fn reconcile_notes(report: &crate::core::provisioning::ReconcileReport) -> Vec<String> {
    let mut notes = Vec::new();
    if !report.repaired.is_empty() {
        notes.push(format!(
            "{} put right, and nothing to do about:\n{}",
            report.repaired.len(),
            report.repaired.join("\n")
        ));
    }
    if !report.incomplete.is_empty() {
        notes.push(format!(
            "{} {} never finished being created and cannot be rebuilt \
             automatically: {}",
            report.incomplete.len(),
            crate::util::plural::of(report.incomplete.len(), "project was", "projects were"),
            report.incomplete.join(", ")
        ));
    }
    if !report.leftovers.is_empty() {
        notes.push(format!(
            "{} old folder{} not removed yet:\n{}",
            report.leftovers.len(),
            crate::util::plural::s(report.leftovers.len()),
            report.leftovers.join("\n")
        ));
    }
    if !report.unrecoverable.is_empty() {
        notes.push(format!(
            "{} {} a look:\n{}",
            report.unrecoverable.len(),
            crate::util::plural::of(report.unrecoverable.len(), "needs", "need"),
            report.unrecoverable.join("\n")
        ));
    }
    if !report.waiting.is_empty() {
        notes.push(format!(
            "{} waiting for a base to answer — fastf finishes {} once it does:\n{}",
            report.waiting.len(),
            crate::util::plural::of(report.waiting.len(), "it", "them"),
            report.waiting.join("\n")
        ));
    }
    if !report.obsolete.is_empty() {
        notes.push(format!(
            "{} obsolete v1 marker{} left alone for manual inspection: {}",
            report.obsolete.len(),
            crate::util::plural::s(report.obsolete.len()),
            report.obsolete.join(", ")
        ));
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::reconcile_notes;
    use crate::core::provisioning::ReconcileReport;

    /// A count reads `1 project`, and the verb after it agrees.
    #[test]
    fn a_report_of_one_is_written_in_the_singular() {
        let one = ReconcileReport {
            incomplete: vec!["Alpha".to_string()],
            leftovers: vec!["a".to_string()],
            unrecoverable: vec!["b".to_string()],
            waiting: vec!["c".to_string()],
            obsolete: vec!["d".to_string()],
            ..ReconcileReport::default()
        };
        let said = reconcile_notes(&one).join("\n");
        for sentence in [
            "1 project was never finished",
            "1 old folder not removed yet",
            "1 needs a look",
            "fastf finishes it once it does",
            "1 obsolete v1 marker left alone",
        ] {
            assert!(said.contains(sentence), "{sentence:?} in:\n{said}");
        }

        let two = ReconcileReport {
            incomplete: vec!["Alpha".to_string(), "Beta".to_string()],
            unrecoverable: vec!["a".to_string(), "b".to_string()],
            waiting: vec!["c".to_string(), "d".to_string()],
            ..ReconcileReport::default()
        };
        let said = reconcile_notes(&two).join("\n");
        for sentence in [
            "2 projects were never finished",
            "2 need a look",
            "fastf finishes them once it does",
        ] {
            assert!(said.contains(sentence), "{sentence:?} in:\n{said}");
        }
    }
}
