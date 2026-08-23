//! The block under the main menu: what the library looks like right now.
//!
//! The menu used to print one line — the base directory — so the two questions
//! anybody opens fastf with ("is my drive mounted?", "what did I just do?") were
//! answered by leaving the menu and running commands.
//!
//! **The frame costs no scan.** Counts come from each base's own
//! `.fastf-index.json` and nothing else: no staleness check, no directory walk,
//! no `PROJECT_INFO.md` read. A summary whose cost grew with the library would
//! make the menu slower the more it had to say, which is backwards. Because the
//! numbers can be stale, they are labelled `from index`, and the one line that
//! must be live — whether a base is actually there — is the probe, which is
//! bounded by `paths::PROBE_TIMEOUT`.

use std::path::Path;
use std::sync::Mutex;

use colored::Colorize;

use crate::core::config::Config;
use crate::core::library::{self, IndexSummary};
use crate::util::paths::{self, Probe};

/// How many of this session's actions the frame remembers.
const RING: usize = 3;

/// The last few things this session did, newest last.
///
/// In memory and per process: it is a reminder, not a log. `PROJECT_INFO.md`'s
/// journal is where anything durable belongs.
static RECENT_ACTIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Record one action for the frame, e.g. `created ID0248`, `tagged ID0100
/// urgent`, `moved ID0017 → archive`.
pub fn record(action: impl Into<String>) {
    let Ok(mut ring) = RECENT_ACTIONS.lock() else {
        return;
    };
    ring.push(action.into());
    let overflow = ring.len().saturating_sub(RING);
    if overflow > 0 {
        ring.drain(..overflow);
    }
}

/// What `record` has collected, oldest first.
fn recent_actions() -> Vec<String> {
    RECENT_ACTIONS
        .lock()
        .map(|ring| ring.clone())
        .unwrap_or_default()
}

/// Print the frame. Silent when `show_frame` is off.
pub fn print(cfg: &Config) {
    if !cfg.show_frame {
        return;
    }

    let bases = cfg.effective_bases();
    let probed = paths::probe_dirs(&bases, paths::PROBE_TIMEOUT);
    let default_base = cfg.resolve_base_dir();

    println!(
        "  {}  {}",
        "fastf".cyan().bold(),
        format!("v{}", env!("CARGO_PKG_VERSION")).dimmed()
    );

    let mut totals = Totals::default();
    for (base, probe) in &probed {
        let summary = probe
            .usable()
            .then(|| library::index_summary(base))
            .flatten();
        totals.absorb(summary.as_ref());
        println!(
            "  {}",
            base_line(base, &default_base, *probe, summary.as_ref())
        );
    }

    if totals.projects > 0 {
        let mut line = format!("  {} projects", totals.projects);
        if let Some(id) = &totals.max_id {
            line.push_str(&format!("  ·  highest {id}"));
        }
        if let Some((id, name)) = &totals.newest {
            line.push_str(&format!("  ·  newest {id} {name}"));
        }
        println!("  {}{}", line.dimmed(), "  (from index)".dimmed());
    }

    let actions = recent_actions();
    if !actions.is_empty() {
        println!(
            "  {} {}",
            "this session:".dimmed(),
            actions.join("  ·  ").dimmed()
        );
    }
    println!();
}

/// One base's row: where it is, whether it answered, and what its index says.
fn base_line(
    base: &Path,
    default_base: &Path,
    probe: Probe,
    summary: Option<&IndexSummary>,
) -> String {
    let marker = if base == default_base { "→" } else { "·" };
    let shown = paths::display_path(base);
    let detail = match (probe, summary) {
        (Probe::Mounted, Some(summary)) => format!("  {} projects", summary.projects),
        // Mounted but never indexed: the first list will build the cache.
        (Probe::Mounted, None) => "  not indexed yet".to_string(),
        (other, _) => other.note().to_string(),
    };
    format!("{} {}{}", marker.dimmed(), shown.cyan(), detail.dimmed())
}

/// Library-wide numbers, accumulated across bases.
#[derive(Default)]
struct Totals {
    projects: usize,
    max_id: Option<String>,
    newest: Option<(String, String)>,
}

impl Totals {
    fn absorb(&mut self, summary: Option<&IndexSummary>) {
        let Some(summary) = summary else {
            return;
        };
        self.projects += summary.projects;
        if let Some(id) = &summary.max_id
            && self.max_id.as_ref().is_none_or(|held| {
                crate::core::naming::id_value(held) < crate::core::naming::id_value(id)
            })
        {
            self.max_id = Some(id.clone());
        }
        // Bases are absorbed in configured order and the first one with any
        // projects supplies "newest". Comparing across bases would need the
        // timestamp, which the summary deliberately does not carry — this line
        // is orientation, not a fact anything depends on.
        if let Some((id, name)) = &summary.newest
            && self.newest.is_none()
        {
            self.newest = Some((id.clone(), name.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RECENT_ACTIONS, RING, recent_actions, record};

    #[test]
    fn the_session_ring_keeps_the_last_few_actions() {
        RECENT_ACTIONS.lock().unwrap().clear();
        for n in 1..=6 {
            record(format!("created ID000{n}"));
        }
        let actions = recent_actions();
        assert_eq!(actions.len(), RING);
        assert_eq!(actions.first().unwrap(), "created ID0004");
        assert_eq!(actions.last().unwrap(), "created ID0006");
        RECENT_ACTIONS.lock().unwrap().clear();
    }
}
