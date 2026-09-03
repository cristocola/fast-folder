//! What the workers hand the app: the library summary and one project's detail.

use std::path::PathBuf;

use crate::core::project_info::Metadata;
use crate::util::paths::Probe;

/// One configured base, as the header shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseInfo {
    pub path: PathBuf,
    pub label: String,
    pub probe: Probe,
    /// Projects according to the base's own index; `None` when it has none yet.
    pub indexed: Option<usize>,
    pub is_default: bool,
}

impl BaseInfo {
    /// `9`, `not indexed yet`, `not mounted`, `unresponsive`.
    pub fn note(&self) -> String {
        match (self.probe, self.indexed) {
            (Probe::Mounted, Some(n)) => n.to_string(),
            (Probe::Mounted, None) => "not indexed yet".to_string(),
            (other, _) => other.note().trim().trim_matches(['(', ')']).to_string(),
        }
    }
}

/// One template, as the strip shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateCard {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub variables: usize,
    pub folders: usize,
    pub naming_pattern: String,
}

/// The header's numbers. Counts come from each base's index and nothing else:
/// no directory is scanned to draw the first frame, so opening the app does not
/// get slower as the library grows. The live count replaces them once discovery
/// answers.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Summary {
    pub bases: Vec<BaseInfo>,
    pub projects: usize,
    pub max_id: Option<String>,
    pub newest: Option<(String, String)>,
    pub templates: Vec<TemplateCard>,
    /// Interrupted creates and moves that `fastf reconcile` would deal with.
    pub attention: usize,
}

/// One entry of a project folder's top level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// What the detail pane shows for the selected project.
#[derive(Clone, Debug, Default)]
pub struct ProjectDetail {
    pub meta: Option<Metadata>,
    /// The most recent entries, newest last, `(date, message)`.
    pub journal: Vec<(String, String)>,
    pub journal_count: usize,
    /// Directories first, then files, both sorted; `PROJECT_INFO.md` hidden.
    pub listing: Vec<Entry>,
    /// The first lines of the `## Notes` section.
    pub notes: Vec<String>,
    /// The read that failed, if one did.
    pub error: Option<String>,
}
