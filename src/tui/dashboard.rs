//! The guided menu's frame: a rule, what fastf is pointed at, how big the
//! library is, and the last few things this session did.
//!
//! Two rules shape everything here.
//!
//! **The numbers are cached on purpose.** Computing them walks the library
//! (`library::discover`) and the counter floor (`Counters::next_value`, which
//! consults `library::max_id`). On a spun-down or network base that is not
//! free, and a menu that re-derives it on every keypress-return feels broken.
//! So the stats are computed once at startup and refreshed only after an action
//! that can change them — never per loop iteration.
//!
//! **The activity log lives in memory and nowhere else.** It is a session
//! scratchpad, not a record: nothing is written to disk, and the durable
//! history is what it always was — the projects themselves and their journals.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use colored::Colorize;

use crate::core::config::Config;
use crate::core::counter::Counters;
use crate::core::library::{self, Project};
use crate::core::template::IdConfig;

/// How many entries the log keeps and shows. Small on purpose — this is a
/// glance, not a scrollback.
const ACTIVITY_SHOWN: usize = 3;

/// Frame width when the terminal reports nothing usable, and the cap that keeps
/// the rule from stretching across an ultrawide window.
const FALLBACK_WIDTH: usize = 60;
const MAX_WIDTH: usize = 100;

/// What the header reports about the library.
pub struct Stats {
    pub projects: usize,
    pub bases_mounted: usize,
    pub bases_configured: usize,
    pub next_id: String,
}

/// One line of session history.
pub struct Entry {
    time: String,
    ok: bool,
    verb: String,
    subject: String,
}

impl Entry {
    pub fn done(verb: &str, subject: impl Into<String>) -> Self {
        Self::at(now_hhmm(), true, verb, subject)
    }

    pub fn failed(verb: &str, subject: impl Into<String>) -> Self {
        Self::at(now_hhmm(), false, verb, subject)
    }

    /// Timestamp injected, so the formatting can be tested without a clock.
    fn at(time: String, ok: bool, verb: &str, subject: impl Into<String>) -> Self {
        Self {
            time,
            ok,
            verb: verb.to_string(),
            subject: subject.into(),
        }
    }
}

fn now_hhmm() -> String {
    // Local, not UTC: this is a log a human reads inside one sitting.
    chrono::Local::now().format("%H:%M").to_string()
}

/// What changed in the library between two refreshes.
pub struct Diff {
    pub added: Vec<Project>,
}

pub struct SessionState {
    stats: Stats,
    activity: VecDeque<Entry>,
    known: HashSet<PathBuf>,
}

impl SessionState {
    pub fn new(cfg: &Config) -> Self {
        let projects = library::discover(cfg);
        Self {
            stats: compute_stats(cfg, &projects),
            activity: VecDeque::new(),
            known: projects.iter().map(|p| p.path.clone()).collect(),
        }
    }

    /// Recompute the stats and report which projects appeared since last time.
    ///
    /// The diff is how create and register learn what to log: `new::run` and
    /// `register::run` return `Result<()>` and print their own output, so the
    /// alternative was either refactoring the whole CLI command layer or
    /// logging a coarse "created a project" — which would be a lie, because
    /// both return `Ok(())` when the user declines at the confirmation. An
    /// aborted create adds nothing to the library, so it correctly logs
    /// nothing.
    ///
    /// Known imprecision, and acceptable for a cosmetic session log: a project
    /// created by another process while this menu was open is attributed to
    /// this session.
    pub fn refresh(&mut self, cfg: &Config) -> Diff {
        let projects = library::discover(cfg);
        let added: Vec<Project> = projects
            .iter()
            .filter(|p| !self.known.contains(&p.path))
            .cloned()
            .collect();
        self.stats = compute_stats(cfg, &projects);
        self.known = projects.iter().map(|p| p.path.clone()).collect();
        Diff { added }
    }

    pub fn log(&mut self, entry: Entry) {
        self.activity.push_back(entry);
        while self.activity.len() > ACTIVITY_SHOWN {
            self.activity.pop_front();
        }
    }

    /// Log whatever a create/register arm turned out to have done.
    pub fn log_added(&mut self, verb: &str, diff: &Diff) {
        match diff.added.as_slice() {
            [] => {}
            [project] => self.log(Entry::done(verb, project.name.clone())),
            many => self.log(Entry::done(verb, format!("{} projects", many.len()))),
        }
    }

    pub fn render(&self, cfg: &Config) {
        let width = frame_width(crate::cli::recent::terminal_columns());

        println!("{}", "─".repeat(width).dimmed());
        print_identity(cfg, width);
        println!("  {}", stats_line(&self.stats).dimmed());

        if !self.activity.is_empty() {
            println!();
            println!("  {}", "recent".dimmed());
            for entry in &self.activity {
                let mark = if entry.ok { "✓".green() } else { "✗".red() };
                println!(
                    "    {}  {} {:<11} {}",
                    entry.time.dimmed(),
                    mark,
                    entry.verb,
                    entry.subject
                );
            }
        }
        println!();
    }
}

fn compute_stats(cfg: &Config, projects: &[Project]) -> Stats {
    let bases = cfg.effective_bases();
    // `next_value` is the one expression for "which ID comes next", and it is
    // read-only: `floor` consults the counter files and `library::max_id`
    // without writing. Never reach for `Counters::converge` here — that
    // propagates, i.e. writes, and a header must not mutate anything.
    let counters = Counters::load().unwrap_or_default();
    let next = Counters::next_value(cfg, &counters);
    let defaults = IdConfig::default();
    Stats {
        projects: projects.len(),
        bases_mounted: bases.iter().filter(|b| b.is_dir()).count(),
        bases_configured: bases.len(),
        // The library-wide next number, rendered with the default prefix and
        // width. A template carrying its own `IdConfig` will mint a different
        // string — this is a preview of the number, not of the folder name.
        next_id: Counters::format_id(&defaults.prefix, defaults.digits, next),
    }
}

fn frame_width(columns: usize) -> usize {
    // `terminal_columns` reports 0 off-terminal, which is also the width the
    // clamp helpers treat as "unknown".
    match columns {
        0 => FALLBACK_WIDTH,
        n => n.min(MAX_WIDTH),
    }
}

fn stats_line(stats: &Stats) -> String {
    let projects = format!(
        "{} project{}",
        stats.projects,
        if stats.projects == 1 { "" } else { "s" }
    );
    let bases = if stats.bases_configured > stats.bases_mounted {
        format!(
            "{} base{} ({} unmounted)",
            stats.bases_mounted,
            if stats.bases_mounted == 1 { "" } else { "s" },
            stats.bases_configured - stats.bases_mounted
        )
    } else {
        format!(
            "{} base{}",
            stats.bases_mounted,
            if stats.bases_mounted == 1 { "" } else { "s" }
        )
    };
    format!("{projects} · {bases} · next {}", stats.next_id)
}

/// `  fastf 1.5.1      base → /mnt/proj/01_PROJECTS`
///
/// Printed rather than returned because the parent directory and the base name
/// are styled differently. Colour is fine here — the ANSI-free rule applies to
/// `Select` item labels, which are redrawn as the cursor moves; this line is
/// written once.
fn print_identity(cfg: &Config, width: usize) {
    let base = cfg.resolve_base_dir();
    let version = format!("fastf {}", env!("CARGO_PKG_VERSION"));
    let prefix = format!("  {version}      base → ");
    let (parent, name) = split_base(&base);

    // Only the path can overflow, so that is the only part budgeted.
    let budget = width.saturating_sub(dialoguer::console::measure_text_width(&prefix));
    let full = format!("{parent}{name}");
    if budget > 0 && dialoguer::console::measure_text_width(&full) > budget {
        let clipped = dialoguer::console::truncate_str(&full, budget, "…");
        println!(
            "  {}      {} {}",
            version.dimmed(),
            "base →".dimmed(),
            clipped.cyan().bold()
        );
        return;
    }
    println!(
        "  {}      {} {}{}",
        version.dimmed(),
        "base →".dimmed(),
        parent.dimmed(),
        name.cyan().bold()
    );
}

/// Split a base into its dimmable parent prefix and its own name.
fn split_base(base: &Path) -> (String, String) {
    let parent = base
        .parent()
        .map(|p| {
            format!(
                "{}{}",
                crate::util::paths::display_path(p),
                std::path::MAIN_SEPARATOR
            )
        })
        .unwrap_or_default();
    let name = base
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| crate::util::paths::display_path(base));
    (parent, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(projects: usize, mounted: usize, configured: usize) -> Stats {
        Stats {
            projects,
            bases_mounted: mounted,
            bases_configured: configured,
            next_id: "ID0048".to_string(),
        }
    }

    #[test]
    fn the_stats_line_counts_and_pluralizes() {
        assert_eq!(
            stats_line(&stats(47, 3, 3)),
            "47 projects · 3 bases · next ID0048"
        );
        assert_eq!(
            stats_line(&stats(1, 1, 1)),
            "1 project · 1 base · next ID0048"
        );
        assert_eq!(
            stats_line(&stats(0, 1, 1)),
            "0 projects · 1 base · next ID0048"
        );
    }

    /// An unplugged drive is a normal state, not an error — it shows as a
    /// smaller mounted count with the difference named, so the numbers never
    /// silently disagree with what the library can actually see.
    #[test]
    fn the_stats_line_names_unmounted_bases() {
        assert_eq!(
            stats_line(&stats(12, 2, 3)),
            "12 projects · 2 bases (1 unmounted) · next ID0048"
        );
    }

    #[test]
    fn the_frame_width_survives_an_unknown_terminal() {
        assert_eq!(frame_width(0), FALLBACK_WIDTH);
        assert_eq!(frame_width(80), 80);
        assert_eq!(frame_width(400), MAX_WIDTH);
    }

    #[test]
    fn the_activity_log_keeps_only_the_newest_entries() {
        let mut log = VecDeque::new();
        let mut state = SessionState {
            stats: stats(0, 0, 0),
            activity: VecDeque::new(),
            known: HashSet::new(),
        };
        for n in 1..=5 {
            state.log(Entry::at(
                "12:00".to_string(),
                true,
                "created",
                format!("Project_{n}"),
            ));
        }
        log.extend(state.activity.iter().map(|e| e.subject.clone()));
        assert_eq!(log, vec!["Project_3", "Project_4", "Project_5"]);
    }

    #[test]
    fn a_base_splits_into_a_dimmable_parent_and_its_own_name() {
        let (parent, name) = split_base(Path::new("/mnt/proj/01_PROJECTS"));
        assert!(parent.ends_with(std::path::MAIN_SEPARATOR));
        assert_eq!(name, "01_PROJECTS");
    }
}
