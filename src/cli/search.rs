//! `fastf search` — query projects by metadata fields and tags.
//!
//! # Grammar
//! Multiple clauses AND together.  No OR, no parentheses.
//!
//! | Clause        | Meaning                                              |
//! |---|---|
//! | `<term>`      | bare term — substring across vars, tags, folder,     |
//! |               | template, template_name, id (case-insensitive)       |
//! | `key=value`   | exact match (case-insensitive)                       |
//! | `key=pat*`    | wildcard: `pre*`, `*post`, `*mid*`                   |
//! | `key>date`    | ISO-date: field is lexicographically after           |
//! | `key<date`    | ISO-date: field is lexicographically before          |
//! | `tag:value`   | exact tag match                                      |
//! | `tag:pat*`    | tag wildcard, the same three shapes                  |
//!
//! # Examples
//! ```bash
//! fastf search ariana                        # default mode: searches across fields
//! fastf search ariana lullaby                # multi-term AND (both must appear)
//! fastf search tag:draft
//! fastf search tag:client/Acme*
//! fastf search template=music-video tag:draft
//! fastf search artist=Aria* created>2026-01-01
//! fastf search tag:draft --plain             # non-interactive / pipe-friendly
//! ```

use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

use crate::core::library::{self, Project};
use crate::core::{config::Config, project_info, query};

pub struct SearchArgs {
    /// Raw query terms (e.g. `["tag:draft", "template=music-video"]`).
    pub terms: Vec<String>,
    /// Force plain list output (also auto-engaged on non-TTY stdout).
    pub plain: bool,
}

pub fn run(args: SearchArgs) -> Result<()> {
    let cfg = Config::load()?;

    // Nothing below this line can be read from a desktop launcher: stdout and
    // stderr are journald sockets there, and the picker has no terminal to draw
    // on. Rather than working into the void, open a terminal and run this again
    // inside it. In every other context — a shell, a pipe, cron, CI — this is
    // false and nothing changes.
    if crate::cli::terminal::hand_off_to_a_terminal(&cfg, args.plain) {
        return Ok(());
    }

    // **A clause that cannot mean what it looks like is refused, not run.**
    // `query::parse` never rejects a term — a script may pass anything, and a
    // bare word is a free-text search — so `created<tomorrow` parsed as a
    // lexicographic compare against `2026-…` and matched *every* project, while
    // `created>`, `tag:` and `=x` printed "No projects match" and exited 0. The
    // guided app's search bar has refused all of these by name since it was
    // written; this is the same question, asked once, from the other surface.
    //
    // It sits below the hand-off deliberately: from a launcher the message
    // would go to a journald socket, and the relaunched process asks it again
    // in the window it opened, where it can be read.
    if let Some(problem) = args.terms.iter().find_map(|term| query::diagnose(term)) {
        anyhow::bail!("{problem}");
    }

    let predicates = query::parse(&args.terms);

    // Now that bare terms parse to Predicate::Free, predicates can only be
    // empty when every term was whitespace.  Skip silently in that case.
    if predicates.is_empty() {
        println!("{}", "No projects match that query.".dimmed());
        return Ok(());
    }

    let projects = library::discover(&cfg);

    if projects.is_empty() {
        println!(
            "{}",
            "No projects yet — create one with `fastf new`.".dimmed()
        );
        return Ok(());
    }

    // Read fresh from disk: a query may name a template variable, which only
    // the metadata holds.
    let owned_matches: Vec<Project> = projects
        .into_iter()
        .filter(|project| {
            project_info::read_metadata(&project.path)
                .ok()
                .flatten()
                .is_some_and(|meta| query::evaluate(&predicates, &meta))
        })
        .collect();
    let matches: Vec<&Project> = owned_matches.iter().collect();

    if matches.is_empty() {
        println!("{}", "No projects match that query.".dimmed());
        return Ok(());
    }

    // Two questions, both of which must say yes: stdout decides the *format*
    // (a pipe gets the plain list), and stderr decides whether the picker can
    // be drawn and answered at all. Without the second, `2>/dev/null` launched
    // a picker nobody could see and waited for a key.
    let interactive =
        !args.plain && std::io::stdout().is_terminal() && crate::util::tty::prompt_available();

    if interactive {
        crate::tui::run(crate::tui::Entry::Search {
            terms: args.terms.clone(),
            initial: owned_matches,
        })
    } else {
        // Shared with `fastf recent` — identical plain output (incl. base column).
        crate::cli::recent::print_plain(&matches);
        Ok(())
    }
}
