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
//! | `key=pat*`    | prefix/glob match                                    |
//! | `key>date`    | ISO-date: field is lexicographically after           |
//! | `key<date`    | ISO-date: field is lexicographically before          |
//! | `tag:value`   | exact tag match                                      |
//! | `tag:pat*`    | tag prefix/glob match                                |
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

    if args.terms.is_empty() {
        anyhow::bail!(
            "no search terms provided — try: fastf search ariana\n\
             Run `fastf search --help` for the full query grammar."
        );
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

    let owned_matches = matching_projects_from(projects, &predicates);
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
        crate::cli::recent::browse(owned_matches, "No projects match that query.")
    } else {
        // Shared with `fastf recent` — identical plain output (incl. base column).
        crate::cli::recent::print_plain(&matches);
        Ok(())
    }
}

/// Fresh, newest-first search results shared with the guided TUI's paged
/// browser. Metadata is read on every call so a tag mutation can immediately
/// add or remove a row from the result set.
pub(crate) fn matching_projects(cfg: &Config, predicates: &[query::Predicate]) -> Vec<Project> {
    matching_projects_from(library::discover(cfg), predicates)
}

fn matching_projects_from(projects: Vec<Project>, predicates: &[query::Predicate]) -> Vec<Project> {
    projects
        .into_iter()
        .filter(|project| still_matches(project, predicates))
        .collect()
}

/// Does one project still satisfy the query?
///
/// Read fresh from disk, because the guided browser asks this about a row it has
/// just patched in memory: a tag added there may have taken the project out of
/// its own search results, and the row has to go with it.
pub(crate) fn still_matches(project: &Project, predicates: &[query::Predicate]) -> bool {
    project_info::read_metadata(&project.path)
        .ok()
        .flatten()
        .is_some_and(|meta| query::evaluate(predicates, &meta))
}
