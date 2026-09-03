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

/// The configuration the app itself needs to decide what to ask.
///
/// Not a copy of `Config`: only the fields a *screen* is a function of, read
/// once with the summary. `core::config` stays the authority — every commit
/// re-reads it on the worker — but the wizard has to know whether to offer a
/// preview before it can draw one.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Prefs {
    pub default_template: String,
    /// Whether a create shows its plan and waits for a yes.
    pub confirm_create: bool,
    pub register_naming_pattern: String,
}

/// Every setting the settings screen shows, read once with the screen.
///
/// Not a `Config`: only what a *row* is a function of, plus the two numbers
/// that come from elsewhere (the counter floor and how many projects need
/// attention). `core::config` stays the authority — every write goes through
/// `cli::config::apply` on a worker and this is read back afterwards.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Settings {
    pub base_dir: String,
    pub bases: Vec<String>,
    pub editor: String,
    pub terminal: String,
    pub default_template: String,
    pub date_format: String,
    /// Today, as `date_format` renders it.
    pub date_preview: String,
    pub preview_lines: usize,
    pub prompt_open_after_create: bool,
    pub confirm_create: bool,
    pub recent_default_limit: usize,
    pub register_naming_pattern: String,
    pub on_name_collision: String,
    pub git_init: bool,
    pub reveal: bool,
    pub open_in_editor: bool,
    pub print_path: bool,
    /// The highest ID seen anywhere, and what the next project would be called.
    pub counter_floor: u64,
    pub next_id: String,
    pub data_dir: String,
    /// Interrupted work `reconcile` would deal with.
    pub attention: usize,
}

/// One template read in full: what a form needs to ask for its variables.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TemplateInfo {
    pub slug: String,
    pub name: String,
    pub naming_pattern: String,
    pub variables: Vec<VarInfo>,
}

/// One template variable, as a form field asks for it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VarInfo {
    pub slug: String,
    pub label: String,
    pub required: bool,
    /// Non-empty for a `select` variable: the only answers it takes.
    pub options: Vec<String>,
    pub default: String,
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
    pub prefs: Prefs,
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
