//! A batch job: one verb applied to every marked project, one item at a time.
//!
//! The runtime already runs one `Action` per worker; a job is the app-level
//! sequencing on top — the next item is sent only when the previous one's
//! `Msg::ActionDone` lands, so each row is patched as its item finishes, the
//! progress modal always shows the truth, and a failure stops nothing. The
//! marks carry the retry state: a row whose item failed or never ran keeps its
//! mark, and one whose item succeeded loses it with its row change.
//!
//! Only the destructive and relocating verbs batch (delete, unregister, move):
//! acting on a run of folders is what the marks are for. Rename stays single —
//! every row would need its own name.

use std::path::PathBuf;

use crate::core::library::Project;
use crate::tui::effect::Action;

/// What one batch does to each of its items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Delete,
    Unregister,
    /// Move every item into the one picked base.
    Move,
}

impl JobKind {
    /// The progress wording, in the imperative the runtime uses.
    pub fn verb(self) -> &'static str {
        match self {
            JobKind::Delete => "deleting",
            JobKind::Unregister => "unregistering",
            JobKind::Move => "moving",
        }
    }

    /// The busy label while one item runs, matching the single-verb wording.
    pub fn busy(self) -> &'static str {
        match self {
            JobKind::Delete => "deleting…",
            JobKind::Unregister => "unregistering…",
            JobKind::Move => "moving…",
        }
    }

    /// The finished report's headline, e.g. "3 deleted".
    pub fn done(self, count: usize) -> String {
        let noun = match self {
            JobKind::Delete => "deleted",
            JobKind::Unregister => "unregistered",
            JobKind::Move => "moved",
        };
        format!("{count} {noun}")
    }

    /// The report modal's title.
    pub fn report_title(self) -> String {
        let noun = match self {
            JobKind::Delete => "delete",
            JobKind::Unregister => "unregister",
            JobKind::Move => "move",
        };
        format!("{noun} report")
    }
}

/// A running batch.
///
/// `pending` shrinks as items begin; `inflight` is the one running now (so a
/// failure can name it and the progress modal can show it); `done` and
/// `failed` record what came back. The items run in the order `targets()`
/// handed them over — display order, which is the order the user read them in.
#[derive(Debug)]
pub struct Job {
    pub kind: JobKind,
    /// The base every item moves to, for a `Move` job.
    pub target: Option<PathBuf>,
    /// Items that have not run yet.
    pub pending: Vec<Project>,
    /// The item a worker is running right now.
    pub inflight: Option<Project>,
    /// How many items finished cleanly.
    pub done: usize,
    /// Items that failed, in order: id, error.
    pub failed: Vec<(String, String)>,
    /// Clean items that came back with a warning (e.g. a move whose source
    /// cleanup is pending).
    pub warnings: Vec<String>,
    /// The user asked to stop: the current item finishes, the rest stay marked.
    pub cancelled: bool,
}

impl Job {
    pub fn new(kind: JobKind, targets: Vec<Project>, target: Option<PathBuf>) -> Self {
        Self {
            kind,
            target,
            pending: targets,
            inflight: None,
            done: 0,
            failed: Vec::new(),
            warnings: Vec::new(),
            cancelled: false,
        }
    }

    pub fn total(&self) -> usize {
        self.pending.len() + usize::from(self.inflight.is_some()) + self.done + self.failed.len()
    }

    /// How many items have run to an outcome.
    pub fn finished(&self) -> usize {
        self.done + self.failed.len()
    }

    /// Begin the next item, moving it from `pending` to `inflight`. `None`
    /// when the job was cancelled or ran out.
    pub fn begin_next(&mut self) -> Option<&Project> {
        if self.cancelled || self.pending.is_empty() {
            return None;
        }
        let project = self.pending.remove(0);
        self.inflight = Some(project);
        self.inflight.as_ref()
    }

    /// The item that just finished leaves `inflight`; its id is returned for
    /// the failure records.
    pub fn clear_inflight(&mut self) -> Option<String> {
        self.inflight.take().map(|p| p.id)
    }

    /// The `Action` one item of this job is.
    pub fn action_for(&self, project: &Project) -> Action {
        match self.kind {
            JobKind::Delete => Action::Delete(Box::new(project.clone())),
            JobKind::Unregister => Action::Unregister(Box::new(project.clone())),
            JobKind::Move => Action::Move {
                project: Box::new(project.clone()),
                target: self.target.clone().expect("a move job carries its target"),
            },
        }
    }

    /// The progress modal's line: "moving 2 of 4", plus a live failure count.
    pub fn progress_line(&self) -> String {
        let mut line = format!(
            "{} {} of {}",
            self.kind.verb(),
            self.finished() + 1,
            self.total()
        );
        if !self.failed.is_empty() {
            line.push_str(&format!("  ·  {} failed", self.failed.len()));
        }
        line
    }

    /// The report modal's body when the job ended with failures, warnings or
    /// a cancel. `None` when everything ran clean — the status line is enough.
    pub fn report(&self) -> Option<(String, String)> {
        let mut lines: Vec<String> = Vec::new();
        if !self.failed.is_empty() {
            lines.push(format!("{} failed:", self.failed.len()));
            for (id, error) in &self.failed {
                lines.push(format!("  {id}: {error}"));
            }
        }
        for warning in &self.warnings {
            lines.push(format!("  warning: {warning}"));
        }
        if self.cancelled {
            let left = self.pending.len();
            if left > 0 {
                lines.push(format!(
                    "cancelled — {left} {} left marked",
                    if left == 1 {
                        "project is"
                    } else {
                        "projects are"
                    }
                ));
            }
        }
        if lines.is_empty() {
            return None;
        }
        Some((self.kind.report_title(), lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::testing::sample_projects;

    #[test]
    fn a_job_runs_its_items_front_to_back() {
        // The app hands `targets()` over in display order — newest first, as
        // the list shows them — and the job must keep that order.
        let projects = sample_projects(3);
        let mut job = Job::new(JobKind::Delete, projects.clone(), None);
        let mut order: Vec<String> = Vec::new();
        while let Some(item) = job.begin_next() {
            order.push(item.id.clone());
            job.clear_inflight();
        }
        let expected: Vec<String> = projects.iter().map(|p| p.id.clone()).collect();
        assert_eq!(order, expected, "the job runs in the order it was given");
        assert!(job.pending.is_empty());
        assert_eq!(job.finished(), 0, "records fill as outcomes land");
    }

    #[test]
    fn a_cancelled_job_stops_beginning_items() {
        let mut job = Job::new(JobKind::Delete, sample_projects(3), None);
        assert!(job.begin_next().is_some());
        job.clear_inflight();
        job.cancelled = true;
        assert!(job.begin_next().is_none(), "cancel stops the run");
        assert_eq!(job.pending.len(), 2, "the rest stay marked, not run");
    }

    #[test]
    fn the_action_matches_the_kind_and_target() {
        let projects = sample_projects(1);
        let item = projects[0].clone();
        assert!(matches!(
            Job::new(JobKind::Delete, projects.clone(), None).action_for(&item),
            Action::Delete(_)
        ));
        assert!(matches!(
            Job::new(JobKind::Unregister, projects.clone(), None).action_for(&item),
            Action::Unregister(_)
        ));
        let target = PathBuf::from("/mnt/archive");
        match Job::new(JobKind::Move, projects, Some(target.clone())).action_for(&item) {
            Action::Move {
                project,
                target: got,
            } => {
                assert_eq!(*project, item);
                assert_eq!(got, target);
            }
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn the_report_names_failures_and_leftover_marks() {
        let mut job = Job::new(
            JobKind::Move,
            sample_projects(3),
            Some("/mnt/archive".into()),
        );
        // One clean, one failed, one never run (cancelled).
        job.begin_next();
        job.clear_inflight();
        job.done += 1;
        job.begin_next();
        let second = job
            .inflight
            .as_ref()
            .expect("second item in flight")
            .clone();
        let second_id = second.id.clone();
        job.clear_inflight();
        job.failed
            .push((second.id, "injected fault at 'move:after-staging'".into()));
        job.cancelled = true;
        assert!(job.begin_next().is_none());

        let (title, body) = job.report().expect("a report is due");
        assert_eq!(title, "move report");
        assert!(body.contains("1 failed"), "{body}");
        assert!(body.contains(&second_id), "{body}");
        assert!(body.contains("injected fault"), "{body}");
        assert!(body.contains("1 project is left marked"), "{body}");
    }

    #[test]
    fn a_clean_job_needs_no_report() {
        let mut job = Job::new(JobKind::Delete, sample_projects(2), None);
        while job.begin_next().is_some() {
            job.clear_inflight();
            job.done += 1;
        }
        assert_eq!(job.finished(), 2);
        assert!(job.report().is_none(), "the status line is enough");
    }
}
