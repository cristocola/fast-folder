//! `fastf move <query> [base]` — move a project folder into another
//! configured base.
//!
//! Targets are restricted to the effective bases (`base_dir` + config `bases`)
//! so a moved project always stays discoverable. Only EXDEV enables the private
//! v2 copy transaction; it verifies topology/lengths and source stability before
//! publication, then removes the source.

use anyhow::Result;
use colored::Colorize;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::core::assets::Progress;
use crate::core::config::Config;
use crate::core::library;

/// How often the progress line is redrawn. Fast enough to look live, slow
/// enough that a network copy is not competing with the terminal for I/O.
const TICK: Duration = Duration::from_millis(200);

pub struct MoveArgs {
    /// Project query — exact ID, ID prefix, or name substring.
    pub query: String,
    /// Target base directory. Omit on a TTY to pick interactively.
    pub base: Option<String>,
    /// Skip the confirmation prompt.
    pub yes: bool,
}

pub fn run(args: MoveArgs) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, &args.query)?;

    let current = project
        .base
        .canonicalize()
        .unwrap_or_else(|_| project.base.clone());
    // Mounted configured bases the project could move to. Probed rather than
    // `is_dir`-ed: a dead network mount answers `is_dir()` only after the
    // operating system's own timeout, and nothing on screen says why.
    let (mounted, unusable) = crate::util::paths::mounted_bases(&cfg.effective_bases());
    for (path, probe) in &unusable {
        eprintln!(
            "{} skipping base {}{}",
            "note:".yellow(),
            crate::util::paths::display_path(path),
            probe.note()
        );
    }
    let candidates: Vec<PathBuf> = mounted.into_iter().filter(|b| *b != current).collect();

    if candidates.is_empty() {
        anyhow::bail!(
            "no other bases configured — add one with `fastf config set bases <dir,...>` \
             or in Settings → Library bases"
        );
    }

    let target = match &args.base {
        Some(raw) => {
            let wanted = PathBuf::from(raw);
            let wanted = wanted.canonicalize().unwrap_or(wanted);
            if wanted == current {
                anyhow::bail!(
                    "'{}' is already in base {}",
                    project.name,
                    current.display()
                );
            }
            // Accept a full path or a base's short label (its folder name).
            candidates
                .iter()
                .find(|b| **b == wanted || library::base_label(b) == raw.trim_end_matches('/'))
                .cloned()
                .ok_or_else(|| {
                    let list = candidates
                        .iter()
                        .map(|b| format!("  {}", b.display()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    anyhow::anyhow!(
                        "'{}' is not a configured base. Valid targets:\n{}",
                        raw,
                        list
                    )
                })?
        }
        None => {
            let default_base = Config::load()?.effective_bases().first().cloned();
            let picked = crate::tui::pickers::pick_base(
                &format!("Move '{}' to which base?", project.name),
                &candidates,
                default_base.as_deref(),
                "name the target instead: `fastf move <query> <base>`",
                true,
            )?;
            match picked {
                Some(base) => base,
                None => {
                    println!("{}", "Cancelled — nothing moved.".dimmed());
                    return Ok(());
                }
            }
        }
    };

    // Confirm before touching anything. Deliberately no size figure: getting one
    // means walking the whole tree, which is wasted on the same-filesystem
    // rename that handles most moves, and slow over NTFS. The progress line
    // below reports real numbers once there is actually something to copy.
    if !args.yes {
        // A confirmation that cannot be shown is not a confirmation. Skipping it
        // here moved the folder on the strength of a question nobody was asked.
        crate::util::tty::require_tty("confirm", "pass --yes to move without confirming")?;
        println!(
            "  {} {}  {} {}",
            "move".dimmed(),
            project.name.bold(),
            "→".cyan(),
            crate::util::paths::display_path(&target)
        );
        let ok = crate::tui::prompt::confirm("Move this project?", true)?.unwrap_or(false);
        if !ok {
            println!("Aborted.");
            return Ok(());
        }
    }

    let outcome = run_with_progress(&project, &target)?;
    let moved = &outcome.project;

    println!(
        "{}  Moved {} {}",
        "✓".green().bold(),
        moved.id.green().bold(),
        moved.name.bold()
    );
    println!(
        "   {} {}",
        "from".dimmed(),
        crate::util::paths::display_path(&project.path).dimmed()
    );
    println!(
        "   {} {}",
        "to  ".dimmed(),
        crate::util::paths::display_path(&moved.path)
    );
    if outcome.cleanup_pending {
        eprintln!(
            "{} destination is complete, but the original could not be removed. \
             Cleanup is pending at {} and the transaction was retained.",
            "warning:".yellow().bold(),
            crate::util::paths::display_path(&project.path)
        );
    }
    Ok(())
}

/// Run the move on a worker thread and report progress from this one.
///
/// A cross-filesystem move can copy for minutes; before this the CLI sat
/// completely silent for the whole of it, while the browser UI showed phase,
/// bytes and a cancel button for the same operation.
///
/// Ctrl-C feeds the engine's cancel flag rather than killing the process, so an
/// interrupted move aborts *before* the source is removed — the invariant the
/// staged path is built around. Same-filesystem moves take the atomic rename
/// fast path and finish before there is anything to draw.
fn run_with_progress(
    project: &library::Project,
    target: &std::path::Path,
) -> Result<library::MoveOutcome> {
    let progress = Mutex::new(Progress::new(&[]));
    let cancel = AtomicBool::new(false);
    let live = std::io::stdout().is_terminal();

    let moved = std::thread::scope(|scope| {
        let worker = scope
            .spawn(|| crate::core::operations::move_project(project, target, &progress, &cancel));

        let mut drew = false;
        while !worker.is_finished() {
            if crate::util::interrupt::is_set() {
                cancel.store(true, Ordering::Relaxed);
            }
            if live {
                let snapshot = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
                // Totals stay zero until the staged path has scanned the tree —
                // and never fill in at all for a rename. Nothing to say yet.
                if snapshot.total_files > 0 {
                    draw(&snapshot);
                    drew = true;
                }
            }
            std::thread::sleep(TICK);
        }
        if drew {
            // Leave the cursor on a fresh line for whatever prints next.
            println!();
        }
        worker.join()
    });

    match moved {
        Ok(result) => result,
        Err(_) => anyhow::bail!("the move thread panicked"),
    }
}

/// One carriage-returned line: phase, file count, bytes.
///
/// Single-line and ANSI-free for the same reason `recent::clamp_label` exists —
/// the legacy Windows console miscounts wrapped rows and leaves ghosted
/// characters behind when a redraw spans more than one.
fn draw(p: &Progress) {
    let line = format!(
        "  {:<10} {}/{} files  {}",
        p.phase,
        p.done_files,
        p.total_files,
        human_bytes(p.copied_bytes, p.total_bytes)
    );
    let width = dialoguer::console::Term::stdout().size().1 as usize;
    let clamped = if width > 1 {
        dialoguer::console::truncate_str(&line, width - 1, "…").into_owned()
    } else {
        line
    };
    print!("\r{clamped}\x1b[K");
    let _ = std::io::stdout().flush();
}

fn human_bytes(done: u64, total: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if total == 0 {
        return String::new();
    }
    format!("{:.0}/{:.0} MB", done as f64 / MB, total as f64 / MB)
}
