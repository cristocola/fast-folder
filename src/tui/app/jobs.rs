//! A batch job: one verb applied to every marked project, one item at a time.
//!
//! The runtime already runs one `Action` per worker; a job is the app-level
//! sequencing on top — the next item is sent only when the previous one's
//! `Msg::ActionDone` lands, so each row is patched as its item finishes, the
//! progress modal always shows the truth, and a failure stops nothing. The
//! marks carry the retry state: a row whose item failed or never ran keeps its
//! mark, and one whose item succeeded loses it when its outcome lands.
//!
//! Every verb that means the same thing for each of several projects batches:
//! delete, unregister and move, and the tags and the notes — select three,
//! add a tag; select five, add the same note. Acting on a run of folders is
//! what the marks are for. Rename stays single: every row would need its own
//! name.

use std::path::PathBuf;

use crate::core::library::Project;
use crate::tui::effect::Action;

/// What one batch does to each of its items. The answer the verb needed —
/// the tag, the note, the base — was asked once and travels with the kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    Delete,
    Unregister,
    /// Move every item into the one picked base.
    Move,
    /// Add the one tag to every item.
    AddTag(String),
    /// Take the picked tags off every item that has them.
    RemoveTags(Vec<String>),
    /// Recompute every item's template-derived tags.
    ReautoTags,
    /// Append the one note to every item's journal.
    Note(String),
}

impl JobKind {
    /// The progress wording, in the imperative the runtime uses.
    pub fn verb(&self) -> &'static str {
        match self {
            JobKind::Delete => "deleting",
            JobKind::Unregister => "unregistering",
            JobKind::Move => "moving",
            JobKind::AddTag(_) => "tagging",
            JobKind::RemoveTags(_) => "untagging",
            JobKind::ReautoTags => "re-deriving tags for",
            JobKind::Note(_) => "noting",
        }
    }

    /// The busy label while one item runs, matching the single-verb wording.
    pub fn busy(&self) -> &'static str {
        match self {
            JobKind::Delete => "deleting…",
            JobKind::Unregister => "unregistering…",
            JobKind::Move => "moving…",
            JobKind::AddTag(_) => "tagging…",
            JobKind::RemoveTags(_) => "removing tags…",
            JobKind::ReautoTags => "re-deriving tags…",
            JobKind::Note(_) => "adding a note…",
        }
    }

    /// The finished report's headline, e.g. "3 deleted".
    pub fn done(&self, count: usize) -> String {
        let noun = match self {
            JobKind::Delete => "deleted",
            JobKind::Unregister => "unregistered",
            JobKind::Move => "moved",
            JobKind::AddTag(_) => "tagged",
            JobKind::RemoveTags(_) => "untagged",
            JobKind::ReautoTags => "re-derived",
            JobKind::Note(_) => "noted",
        };
        format!("{count} {noun}")
    }

    /// The report modal's title.
    pub fn report_title(&self) -> String {
        let noun = match self {
            JobKind::Delete => "delete",
            JobKind::Unregister => "unregister",
            JobKind::Move => "move",
            JobKind::AddTag(_) => "tag",
            JobKind::RemoveTags(_) => "untag",
            JobKind::ReautoTags => "re-derive",
            JobKind::Note(_) => "note",
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

    /// The item that just finished leaves `inflight` and is handed back: its id
    /// names it in a failure record, and its path is the key its mark is held
    /// under.
    pub fn take_inflight(&mut self) -> Option<Project> {
        self.inflight.take()
    }

    /// The `Action` one item of this job is.
    pub fn action_for(&self, project: &Project) -> Action {
        let project = Box::new(project.clone());
        match &self.kind {
            JobKind::Delete => Action::Delete(project),
            JobKind::Unregister => Action::Unregister(project),
            JobKind::Move => Action::Move {
                project,
                target: self.target.clone().expect("a move job carries its target"),
            },
            JobKind::AddTag(tag) => Action::AddTag {
                project,
                tag: tag.clone(),
            },
            JobKind::RemoveTags(tags) => Action::RemoveTags {
                project,
                tags: tags.clone(),
            },
            JobKind::ReautoTags => Action::ReautoTags(project),
            JobKind::Note(text) => Action::AppendNote {
                project,
                text: text.clone(),
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
            job.take_inflight();
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
        job.take_inflight();
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
        match Job::new(JobKind::Move, projects.clone(), Some(target.clone())).action_for(&item) {
            Action::Move {
                project,
                target: got,
            } => {
                assert_eq!(*project, item);
                assert_eq!(got, target);
            }
            other => panic!("expected a move, got {other:?}"),
        }
        // The tag and the note were asked once and ride with the kind.
        match Job::new(JobKind::AddTag("draft".into()), projects.clone(), None).action_for(&item) {
            Action::AddTag { project, tag } => {
                assert_eq!(*project, item);
                assert_eq!(tag, "draft");
            }
            other => panic!("expected a tag, got {other:?}"),
        }
        match Job::new(JobKind::Note("first cut".into()), projects, None).action_for(&item) {
            Action::AppendNote { project, text } => {
                assert_eq!(*project, item);
                assert_eq!(text, "first cut");
            }
            other => panic!("expected a note, got {other:?}"),
        }
        assert_eq!(JobKind::AddTag("x".into()).done(3), "3 tagged");
        assert_eq!(JobKind::Note("x".into()).report_title(), "note report");
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
        job.take_inflight();
        job.done += 1;
        job.begin_next();
        let second = job
            .inflight
            .as_ref()
            .expect("second item in flight")
            .clone();
        let second_id = second.id.clone();
        job.take_inflight();
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
            job.take_inflight();
            job.done += 1;
        }
        assert_eq!(job.finished(), 2);
        assert!(job.report().is_none(), "the status line is enough");
    }
}
