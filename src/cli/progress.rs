//! The command line's side of a long job: the job on a worker thread, and on
//! this one a live line naming the step and its count, with a line of record
//! for every step as it finishes.
//!
//! `move`, `copy-to` and `reconcile` all run through [`run_watched`], so a
//! step is worded the same by each ([`JobPhase::as_str`], [`JobPhase::past`])
//! and the same as the app says it.

use anyhow::Result;
use colored::Colorize;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::core::assets::{FinishedStep, JobPhase, Progress, count_text};

/// How often the progress line is redrawn. Fast enough to look live, slow
/// enough that a network copy is not competing with the terminal for I/O.
const TICK: Duration = Duration::from_millis(200);

/// Run `work` on a worker, drawing its progress here until it answers.
///
/// Ctrl-C feeds the job's cancel flag rather than killing the process, so an
/// interrupted move stops *before* its original is touched. From the publish
/// on a cancel cannot undo anything (`Progress::committed`), and the line
/// says so once rather than pretending to stop. Every finished step is printed
/// as a line of its own, on a terminal or not, so a log of the run says what
/// happened in order; the live line is for a terminal only.
pub(crate) fn run_watched<T: Send>(
    what: &str,
    work: impl FnOnce(&Mutex<Progress>, &AtomicBool) -> Result<T> + Send,
) -> Result<T> {
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    let live = std::io::stdout().is_terminal();

    let answered = std::thread::scope(|scope| {
        let worker = scope.spawn(|| work(&progress, &cancel));
        let mut printed = 0;
        let mut drew = false;
        let mut said_too_late = false;
        let mut watch = |finished_too: bool| {
            let snapshot = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if crate::util::interrupt::is_set() {
                cancel.store(true, Ordering::Relaxed);
                if snapshot.committed && !said_too_late {
                    said_too_late = true;
                    clear_line(live && drew);
                    drew = false;
                    println!(
                        "  {} too late to cancel: the {what} is past its point of no \
                         return, and what is left carries on",
                        "note:".cyan().bold()
                    );
                }
            }
            if snapshot.finished.len() > printed {
                clear_line(live && drew);
                drew = false;
                for step in &snapshot.finished[printed..] {
                    println!("  {} {}", "✓".green(), finished_line(step));
                }
                printed = snapshot.finished.len();
            }
            if live && !finished_too && !matches!(snapshot.phase, JobPhase::Starting) {
                draw(&snapshot);
                drew = true;
            }
        };
        while !worker.is_finished() {
            watch(false);
            std::thread::sleep(TICK);
        }
        watch(true);
        clear_line(live && drew);
        worker.join()
    });

    match answered {
        Ok(result) => result,
        Err(_) => anyhow::bail!("the {what} thread panicked"),
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
