//! The machine-readable shape of a project — what `--json` prints.
//!
//! Deliberately its own types rather than `Serialize` on `core`'s: the JSON is
//! a promise to whatever parses it, and the library's structs are free to move
//! without breaking that promise. Paths go through `util::paths::display_path`,
//! so what a script reads is what every other command prints.

use serde::Serialize;

use crate::core::library::{self, Project};

#[derive(Serialize)]
pub struct ProjectJson {
    pub id: String,
    pub number: Option<u64>,
    pub name: String,
    pub path: String,
    pub base: String,
    pub base_label: String,
    pub template: String,
    pub template_name: String,
    pub created: String,
    pub tags: Vec<String>,
    /// Whether the folder is still there. A cached project whose folder went
    /// away is dropped before it reaches here, so this is normally `true`.
    pub exists: bool,
}

impl ProjectJson {
    pub fn of(project: &Project) -> Self {
        Self {
            id: project.id.clone(),
            number: project.number(),
            name: project.name.clone(),
            path: crate::util::paths::display_path(&project.path),
            base: crate::util::paths::display_path(&project.base),
            base_label: library::base_label(&project.base),
            template: project.template.clone(),
            template_name: project.template_name.clone(),
            created: project.created.clone(),
            tags: project.tags.clone(),
            exists: project.path.exists(),
        }
    }
}

#[derive(Serialize)]
pub struct NoteJson {
    pub timestamp: Option<String>,
    pub text: String,
}

#[derive(Serialize)]
pub struct TodoJson {
    pub done: bool,
    pub text: String,
    pub phase: Option<String>,
}

/// One project, whole: what `fastf show --json` prints.
#[derive(Serialize)]
pub struct ProjectDetailJson {
    #[serde(flatten)]
    pub project: ProjectJson,
    pub variables: std::collections::BTreeMap<String, String>,
    pub notes: Vec<NoteJson>,
    pub todos: Vec<TodoJson>,
}

/// Print `value` as pretty JSON on stdout. Pretty because a person reads it
/// too, and no parser minds the whitespace.
pub fn print<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// The list shape: a bare array, so `fastf recent --json | jq '.[].id'` is the
/// obvious thing and stays the obvious thing.
pub fn print_projects(projects: &[&Project]) -> anyhow::Result<()> {
    let rows: Vec<ProjectJson> = projects.iter().map(|p| ProjectJson::of(p)).collect();
    print(&rows)
}
