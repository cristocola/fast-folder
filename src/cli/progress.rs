//! The command line's side of a long job: a live line naming the step and its
//! count, with a line of record for every step as it finishes.
//!
//! `move`, `copy-to`, `delete` and `reconcile` all follow their job through a
//! [`Printer`] (`cli::jobs`), so a step is worded the same by each
//! ([`JobPhase::as_str`], [`JobPhase::past`]) and the same as the app says it.

use colored::Colorize;
use std::io::{IsTerminal, Write};

use crate::core::assets::{FinishedStep, JobPhase, Progress, count_text};

/// What the command line shows of a job's progress: a line of record for
/// every step as it finishes, and a live line for the one under way — which
/// only a terminal gets. Fed snapshots, however they arrive: from the worker's
/// state file, a few times a second.
pub(crate) struct Printer {
    live: bool,
    printed: usize,
    drew: bool,
    /// The item the finished steps counted belong to.
    item: (usize, String),
}

impl Printer {
    pub(crate) fn new() -> Self {
        Self {
            live: std::io::stdout().is_terminal(),
            printed: 0,
            drew: false,
            item: (0, String::new()),
        }
    }

    pub(crate) fn show(&mut self, snapshot: &Progress) {
        let item = (snapshot.item, snapshot.item_label.clone());
        if item != self.item {
            self.item = item;
            self.printed = 0;
            if snapshot.items > 1 || !snapshot.item_label.is_empty() && snapshot.item > 0 {
                self.clear();
                println!(
                    "  {} {}",
                    snapshot.item_text().dimmed(),
                    snapshot.item_label.bold()
                );
            }
        }
        if snapshot.finished.len() > self.printed {
            self.clear();
            for step in &snapshot.finished[self.printed..] {
                println!("  {} {}", "✓".green(), finished_line(step));
            }
            self.printed = snapshot.finished.len();
        }
        if self.live
            && !matches!(snapshot.phase, JobPhase::Starting | JobPhase::Done)
            && snapshot.status == crate::core::assets::JobStatus::Running
        {
            draw(snapshot);
            self.drew = true;
        }
    }

    /// Say something on a line of its own, above the live line.
    pub(crate) fn say(&mut self, text: &str) {
        self.clear();
        println!("{text}");
    }

    pub(crate) fn clear(&mut self) {
        clear_line(self.live && self.drew);
        self.drew = false;
    }
}

fn clear_line(drew: bool) {
    if drew {
        print!("\r\x1b[K");
        let _ = std::io::stdout().flush();
    }
}

/// `scanned 1473 entries`.
pub(crate) fn finished_line(step: &FinishedStep) -> String {
    let count = count_text(step.phase, step.count, 0);
    if count.is_empty() {
        step.phase.past().to_string()
    } else {
        format!("{} {count}", step.phase.past())
    }
}

/// The live line: which item, the step, its count, and bytes while copying.
///
/// One carriage-returned line, single and ANSI-free for the same reason
/// `recent::clamp_label` exists — the legacy Windows console miscounts wrapped
/// rows and leaves ghosted characters behind when a redraw spans more than
/// one.
pub(crate) fn draw(p: &Progress) {
    let line = format!("  {}", live_line(p));
    let width = ratatui::crossterm::terminal::size()
        .map(|(columns, _rows)| columns as usize)
        .unwrap_or(0);
    let clamped = if width > 1 {
        crate::tui::view::fit(&line, width - 1, "…")
    } else {
        line
    };
    print!("\r{clamped}\x1b[K");
    let _ = std::io::stdout().flush();
}

/// What the live line says, without its indent.
pub(crate) fn live_line(p: &Progress) -> String {
    let mut line = String::new();
    let item = p.item_text();
    if !item.is_empty() {
        line.push_str(&item);
        if !p.item_label.is_empty() {
            line.push_str(&format!(" {}", p.item_label));
        }
        line.push_str(": ");
    }
    line.push_str(p.phase.as_str());
    let count = p.count_text();
    if !count.is_empty() {
        line.push_str(&format!("  {count}"));
    }
    if p.phase == JobPhase::Copying && p.total_bytes > 0 {
        line.push_str(&format!(
            "  {} of {}",
            crate::util::human_bytes::human_bytes(p.copied_bytes),
            crate::util::human_bytes::human_bytes(p.total_bytes)
        ));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_line_names_the_item_the_step_and_its_count() {
        let mut p = Progress::new(&[]);
        p.phase = JobPhase::Removing;
        p.step_done = 312;
        p.step_total = 1473;
        assert_eq!(live_line(&p), "removing the old copy  312 of 1473 entries");
        p.item = 1;
        p.items = 2;
        p.item_label = "Lullaby".to_string();
        assert_eq!(
            live_line(&p),
            "1 of 2 Lullaby: removing the old copy  312 of 1473 entries"
        );
    }

    #[test]
    fn a_finished_step_says_what_it_did() {
        assert_eq!(
            finished_line(&FinishedStep {
                phase: JobPhase::Scanning,
                count: 1473
            }),
            "scanned 1473 entries"
        );
        assert_eq!(
            finished_line(&FinishedStep {
                phase: JobPhase::Publishing,
                count: 1
            }),
            "published PROJECT_INFO.md"
        );
    }
}
