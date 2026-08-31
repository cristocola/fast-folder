//! Which project did the user mean?
//!
//! One flow, shared by every verb that acts on a single project and can afford
//! to ask: resolve, and where the query matched several, show a picker on a
//! terminal or report the candidates as an error anywhere else.
//!
//! The picker serves the verb it interrupted. Choosing from an ambiguous
//! `fastf copy` copies; from `fastf open` it opens. It never detours into the
//! project action menu — that is what `fastf` and `fastf recent` are for.

use anyhow::Result;

use crate::core::config::Config;
use crate::core::library::{self, Project, Resolution};

/// What a verb should do next, once the query has been resolved.
///
/// The two non-project answers are both exit 0 and are **not** the same thing:
/// one is the user declining, which is worth a line, and the other is this run
/// being superseded by one inside a terminal, where a second voice in the
/// journal would only be confusing.
pub enum Target {
    /// Act on this project. Boxed — `Project` is large, and the Windows clippy
    /// leg fires `large_enum_variant` where Linux does not.
    Project(Box<Project>),
    /// The picker was cancelled. Say so, and stop.
    Cancelled,
    /// A terminal emulator now owns a rerun of this exact command. Say nothing,
    /// and stop — doing the work here as well would do it twice.
    HandedOff,
}

/// Resolve `query` to exactly one project, asking when it is ambiguous.
///
/// `prompt` heads the picker; `how` names the way to answer the same question
/// without a prompt, both for `require_tty`'s refusal and for the error a
/// script sees.
pub fn one_project(cfg: &Config, query: &str, prompt: &str, how: &str) -> Result<Target> {
    match library::resolve_matches(cfg, query) {
        Resolution::NoProjects => Err(library::no_projects_error()),
        Resolution::NoMatch => Err(library::no_match_error(query)),
        Resolution::One(project) => Ok(Target::Project(project)),
        Resolution::Many(candidates) => {
            if can_ask() {
                match crate::tui::pickers::pick_project(prompt, &candidates, how)? {
                    Some(project) => Ok(Target::Project(Box::new(project))),
                    None => Ok(Target::Cancelled),
                }
            } else if crate::cli::terminal::hand_off_to_a_terminal(cfg, false) {
                // Launched from a desktop launcher: the candidate list would go
                // to the journal and the command would look like it did nothing.
                // A terminal now owns the rerun, and it will show the picker.
                Ok(Target::HandedOff)
            } else {
                // Piped, redirected, cron, CI: somebody is reading this and
                // nobody is answering it. The error text is unchanged.
                Err(library::ambiguous_error(query, &candidates))
            }
        }
    }
}

/// **Stderr decides, not stdout.**
///
/// `recent` and `search` gate on both because there stdout chooses the output
/// *format* — a pipe gets the plain list instead of the browser. This is a
/// different question: "can I ask?", which is what `util::tty` exists to answer,
/// and the answer is stderr, because that is where dialoguer draws and stdin is
/// where it reads.
///
/// Gating on stdout would make the picker unreachable in the one place `path`
/// is designed for: `cd "$(fastf path lullaby)"` redirects stdout by
/// construction, with a terminal sitting right there. It would also repeat the
/// exact mistake `util::tty` was written to fix, and which
/// `a_redirected_stdout_still_has_a_terminal_to_prompt_on` pins.
///
/// stdout's own contract survives either way, because the picker never writes
/// to it: a redirected `fastf path` still emits the path and nothing else. A
/// script, cron job or CI runner has no terminal on stderr either, so it gets
/// the ambiguity error unchanged.
fn can_ask() -> bool {
    crate::util::tty::prompt_available()
}

/// The hint every ambiguity carries: name the verb, so the example is one the
/// reader can retype.
pub fn full_id_hint(verb: &str) -> String {
    format!("give a full ID, e.g. `fastf {verb} ID0037`")
}
