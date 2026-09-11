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

use super::App;
use crate::core::assets::Progress;
use crate::core::library::Project;
use crate::tui::app::modal::{MessageLevel, Modal};
use crate::tui::effect::{Action, ActionOutcome, Effect};

/// What one batch does to each of its items. The answer the verb needed —
/// the tag, the note, the base — was asked once and travels with the kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    Delete,
    Unregister,
    /// Move every item into the one picked base.
    Move,
    /// Copy every item into the one folder, keeping each id.
    CopyTo(PathBuf),
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
            JobKind::CopyTo(_) => "copying",
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
            JobKind::CopyTo(_) => "copying…",
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
            JobKind::CopyTo(_) => "copied",
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
            JobKind::CopyTo(_) => "copy",
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
            JobKind::CopyTo(destination) => Action::CopyTo {
                project,
                destination: destination.clone(),
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
        // A heading, like the failures above. A clean batch that reported
        // source-cleanup warnings opened its dialog with an indented list
        // under nothing at all.
        if !self.warnings.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(format!(
                "{} warning{}:",
                self.warnings.len(),
                if self.warnings.len() == 1 { "" } else { "s" }
            ));
            for warning in &self.warnings {
                lines.push(format!("  {warning}"));
            }
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

impl App {
    /// Run the verb over every marked project, one item at a time.
    pub(super) fn start_job(&mut self, kind: JobKind, target: Option<PathBuf>) -> Vec<Effect> {
        let targets = self.library.targets();
        if targets.is_empty() {
            return Vec::new();
        }
        self.job = Some(Job::new(kind, targets, target));
        self.job_advance()
    }

    /// Begin the next item of the running job. When nothing is left — every
    /// item ran, or the job was cancelled — finish it.
    fn job_advance(&mut self) -> Vec<Effect> {
        let kind = match self.job.as_ref() {
            Some(job) => job.kind.clone(),
            None => return Vec::new(),
        };
        let Some(project) = self.job.as_mut().and_then(|job| job.begin_next().cloned()) else {
            return self.job_finish();
        };
        // The progress modal is for a move that is actually running: arming it
        // here — after an item began — means the final advance, which only
        // finishes the job, cannot leave a stale modal behind for every later
        // quit gesture to read as "a move is running".
        if kind == JobKind::Move {
            self.move_progress = Some(Progress::new(&[]));
        }
        let action = {
            let job = self.job.as_ref().expect("the job is running");
            job.action_for(&project)
        };
        self.run_action(kind.busy(), action)
    }

    /// One item's outcome landed: record it, patch the row, and move on.
    pub(super) fn on_job_item_done(
        &mut self,
        outcome: Result<Box<ActionOutcome>, String>,
    ) -> Vec<Effect> {
        // The item that was running leaves `inflight`, whatever happened. Its
        // path is what the mark is keyed by.
        let finished = self.job.as_mut().and_then(|job| job.take_inflight());
        let (id, path) = match finished {
            Some(project) => (project.id, Some(project.path)),
            None => ("?".to_string(), None),
        };
        let mut effects = Vec::new();
        match outcome {
            Ok(outcome) => {
                let outcome = *outcome;
                if let Some(entry) = outcome.session {
                    crate::tui::frame::record(entry);
                    self.session = crate::tui::frame::recent_actions();
                }
                if let Some(warning) = outcome.warning
                    && let Some(job) = &mut self.job
                {
                    job.warnings.push(warning);
                }
                // **The effects a change asks for are the job's too.** They
                // were dropped here, where the single-action path returns them,
                // and `apply_change`'s `Reload` arm calls `discover`, which
                // sets `library.inflight` *before* handing back the effect that
                // would answer it. A dropped one left the app waiting on a
                // generation nothing would ever send, after which every patch
                // only set `dirty` and the list stopped changing: a batch
                // re-derive of tags rewrote every file and showed nothing, and
                // the list stayed frozen for the rest of the session.
                effects.extend(self.apply_change(outcome.change));
                // A mark is the retry list. An item that succeeded is not on
                // it any more, so "3 tagged" and the ✓ glyphs left on screen
                // cannot disagree — `jobs.rs` has always said so; nothing did
                // it, because `patch` only drops a mark when the path moved.
                if let Some(path) = &path {
                    self.library.marks.remove(path);
                }
                if let Some(job) = &mut self.job {
                    job.done += 1;
                }
            }
            Err(error) => {
                // A cancellation the user asked for is not a failure to list:
                // the report says how many were left instead. Either way the
                // mark stays: it is what a retry would act on.
                let cancelled = self.job.as_ref().is_some_and(|job| job.cancelled);
                if !cancelled && let Some(job) = &mut self.job {
                    job.failed.push((id, error));
                }
            }
        }
        effects.extend(self.job_advance());
        effects
    }

    /// The job has no items left to begin: report and clear it.
    fn job_finish(&mut self) -> Vec<Effect> {
        let Some(job) = self.job.take() else {
            return Vec::new();
        };
        self.move_progress = None;
        let mut headline = job.kind.done(job.done);
        if !job.failed.is_empty() {
            headline.push_str(&format!(", {} failed", job.failed.len()));
        }
        if job.cancelled {
            headline.push_str(" — cancelled");
        }
        if let Some((title, body)) = job.report() {
            // The rows the report names are the rows that still hold a mark,
            // so Esc closes the report straight back onto a consistent list.
            let level = if job.failed.is_empty() {
                MessageLevel::Warn
            } else {
                MessageLevel::Error
            };
            self.modals.push(Modal::message(title, body, level));
        }
        if job.failed.is_empty() && !job.cancelled {
            self.good(headline);
        } else {
            self.warn(headline);
        }
        Vec::new()
    }

    /// Stop after the current item: the in-flight move is told to cancel, and
    /// the job marks itself as cancelled so the rest stay marked. A bare
    /// single move (no job) just cancels at the runtime.
    pub(super) fn request_cancel(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.move_progress.is_some() {
            effects.push(Effect::CancelMove);
        }
        match &mut self.job {
            Some(job) => job.cancelled = true,
            None => return effects,
        }
        // Between items nothing is in flight: finish now. Otherwise the
        // current item's ActionDone finishes the job when it lands.
        if self.busy.is_none() {
            effects.extend(self.job_finish());
        }
        effects
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
