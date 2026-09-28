//! `ReconcileReport`: what a pass did, what waits, and what needs a look.

use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ReconcileReport {
    pub resumed: usize,
    pub completed: usize,
    pub rolled_back: usize,
    /// Case-only renames finished off — see [`IncompleteKind::RenameStaging`].
    pub restored: usize,
    /// Old folders of projects that had already left the library, removed.
    pub cleared: usize,
    pub incomplete: Vec<String>,
    /// Old folders of projects that have left the library — hidden, so not
    /// listed — that are not removed yet, and what to do about each.
    pub leftovers: Vec<String>,
    pub unrecoverable: Vec<String>,
    pub obsolete: Vec<String>,
    /// Work fastf finishes by itself once a base answers again: a mount that
    /// is not mounted or does not answer, a path it could not look at. Nothing
    /// was changed, and nothing about it needs a person yet.
    pub waiting: Vec<String>,
    /// The pass stopped for a cancel before it had looked at everything;
    /// what it did not reach is as it was, and the next pass takes it up.
    pub cancelled: bool,
    /// What the pass could not settle and a person has to decide, by the
    /// folder it is about: kept as `attention.json` (`core::attention`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verdicts: Vec<crate::core::attention::Verdict>,
}

impl ReconcileReport {
    /// Keep a verdict: `path` needs a person, for `reason`.
    pub(crate) fn needs(
        &mut self,
        path: &Path,
        kind: crate::core::attention::VerdictKind,
        reason: &str,
    ) {
        self.verdicts.push(crate::core::attention::Verdict {
            path: path.to_path_buf(),
            kind,
            reason: reason.to_string(),
        });
    }
}

impl ReconcileReport {
    pub fn is_empty(&self) -> bool {
        self.resumed == 0
            && self.completed == 0
            && self.rolled_back == 0
            && self.restored == 0
            && self.cleared == 0
            && self.incomplete.is_empty()
            && self.leftovers.is_empty()
            && self.unrecoverable.is_empty()
            && self.obsolete.is_empty()
            && self.waiting.is_empty()
            && !self.cancelled
    }

    /// The pass in one sentence, the same on every surface: what it did, and
    /// whether it stopped before the end.
    pub fn summary(&self) -> String {
        if self.cancelled {
            format!(
                "Reconcile stopped: {} resumed, {} finished, {} rolled back, {} restored, \
                 {} cleared before it did; the rest is as it was",
                self.resumed, self.completed, self.rolled_back, self.restored, self.cleared
            )
        } else if self.is_empty() {
            "Nothing to reconcile — every project is fully provisioned.".to_string()
        } else {
            format!(
                "Reconciled: {} resumed, {} finished, {} rolled back, {} restored, {} cleared{}{}",
                self.resumed,
                self.completed,
                self.rolled_back,
                self.restored,
                self.cleared,
                match self.waiting.len() {
                    0 => String::new(),
                    n => format!("; {n} waiting for a base to answer"),
                },
                match self.unrecoverable.len() + self.leftovers.len() {
                    0 => String::new(),
                    1 => "; 1 needs a look".to_string(),
                    n => format!("; {n} need a look"),
                }
            )
        }
    }

    /// Whether anything is left for a person to look at.
    pub fn needs_a_look(&self) -> bool {
        self.cancelled || !self.unrecoverable.is_empty() || !self.leftovers.is_empty()
    }
}
