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

/// Resolve `query` to exactly one project, asking when it is ambiguous.
///
/// `Ok(None)` means the user cancelled the picker: the caller reports it and
/// returns `Ok(())`, because deciding not to act is not a failure.
///
/// `prompt` heads the picker; `how` names the way to answer the same question
/// without a prompt, both for `require_tty`'s refusal and for the error a
/// script sees.
pub fn one_project(cfg: &Config, query: &str, prompt: &str, how: &str) -> Result<Option<Project>> {
    match library::resolve_matches(cfg, query) {
        Resolution::NoProjects => Err(library::no_projects_error()),
        Resolution::NoMatch => Err(library::no_match_error(query)),
        Resolution::One(project) => Ok(Some(*project)),
        Resolution::Many(candidates) => {
            if can_ask() {
                crate::tui::pickers::pick_project(prompt, &candidates, how)
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
