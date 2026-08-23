//! What a create is about to do, as data.
//!
//! Its own module because it sits between two that need it and must not need
//! each other: `project` builds one, and `project_info` writes a
//! `PROJECT_INFO.md` from one. With the struct in `project`, those two imported
//! each other.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::naming::RenderContext;

#[derive(Clone, Debug)]
pub struct ProjectPlan {
    /// The resolved root folder name (after pattern interpolation).
    pub folder_name: String,
    /// Full path where the project root will be created.
    pub root_path: PathBuf,
    /// Resolved variable map (slug → final value, after transforms).
    pub vars: HashMap<String, String>,
    /// The ID string used (e.g. "ID0047").
    pub id_str: String,
    /// Counter value used.
    pub counter_value: u64,
    /// The date tokens this create resolves to, sampled once when the plan was
    /// built.
    ///
    /// Carried on the plan rather than re-read per file so a create that spans
    /// midnight cannot name the folder with one date and the files inside it
    /// with another, and so the plan a user approves is the plan that is
    /// committed.
    pub ctx: RenderContext,
}
