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
    let cfg = Config::load().unwrap_or_default();

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

    // For each project (newest first), load its metadata and evaluate. The
    // authoritative field values live in PROJECT_INFO.md, so read it fresh.
    let mut matches = Vec::new();
    for project in &projects {
        if let Ok(Some(meta)) = project_info::read_metadata(&project.path)
            && query::evaluate(&predicates, &meta)
        {
            matches.push(project);
        }
    }

    if matches.is_empty() {
        println!("{}", "No projects match that query.".dimmed());
        return Ok(());
    }

    let interactive = !args.plain && std::io::stdout().is_terminal();

    if interactive {
        crate::cli::recent::run_picker(&matches)
    } else {
        print_plain_results(&matches);
        Ok(())
    }
}

fn print_plain_results(matches: &[&Project]) {
    let id_w = matches.iter().map(|p| p.id.len()).max().unwrap_or(4);
    let tmpl_w = matches.iter().map(|p| p.template.len()).max().unwrap_or(8);
    let date_w = 10;

    for p in matches {
        let date = p.created.get(..date_w).unwrap_or(&p.created);
        let path_str = p.path.display().to_string();
        let missing = !p.path.exists();
        let marker = if missing { "✗".red() } else { "•".cyan() };
        println!(
            "  {} {:<id_w$}  {:<tmpl_w$}  {}  {}",
            marker,
            p.id.green().bold(),
            p.template.dimmed(),
            date.dimmed(),
            if missing {
                format!("{} {}", p.name, "(missing)".red())
            } else {
                p.name.clone()
            },
            id_w = id_w,
            tmpl_w = tmpl_w,
        );
        println!("      {} {}", "→".dimmed(), path_str.dimmed());
    }
}
