//! The command line's side of a job: start one and follow it, and
//! `fastf jobs` to list, follow and cancel any.
//!
//! A long verb — `move`, `copy-to`, `delete`, `reconcile` — starts a job
//! (`core::jobs::start`) and follows its state file here, printing the same
//! lines it printed when the work ran in this process. **The work is not this
//! process's**: killing it, or closing its terminal, leaves the job running.
//! Ctrl-C asks the job to cancel while a cancel still undoes it; once the job
//! is past its point of no return, Ctrl-C leaves it to finish and says how to
//! follow it again. `--detach` starts it and returns at once.

use anyhow::{Result, bail};
use colored::Colorize;
use std::time::Duration;

use crate::cli::progress::Printer;
use crate::core::assets::JobStatus;
use crate::core::jobs::{JobItem, JobKind, JobState, JobView};

/// How often a followed job's state is read.
const TICK: Duration = Duration::from_millis(200);

/// How following a job ended.
pub(crate) enum Followed {
    /// It ended; its final state.
    Ended(Box<JobState>),
    /// Ctrl-C after its point of no return: it carries on without us.
    LeftRunning,
    /// Started with `--detach`.
    Detached,
}

/// Start a job and follow it, or only start it.
pub(crate) fn start(kind: JobKind, items: Vec<JobItem>, detach: bool) -> Result<Followed> {
    let id = crate::core::jobs::start(kind, items)?;
    if detach {
        println!(
            "{}  started job {}: {}",
            "✓".green().bold(),
            id.bold(),
            crate::core::jobs::view(&id).map_or_else(String::new, |job| job.title())
        );
        println!(
            "   {}",
            format!("`fastf jobs watch {id}` follows it, `fastf jobs cancel {id}` stops it")
                .dimmed()
        );
        return Ok(Followed::Detached);
    }
    follow(&id)
}

/// Follow job `id` until it ends, or until Ctrl-C lets it go.
pub(crate) fn follow(id: &str) -> Result<Followed> {
    let mut printer = Printer::new();
    let mut asked = false;
    loop {
        let Some(job) = crate::core::jobs::view(id) else {
            bail!("there is no job {id}");
        };
        if let Some(state) = &job.state {
            printer.show(&state.progress);
        }
        let ended = job
            .state
            .as_ref()
            .is_some_and(|state| state.status != JobStatus::Running);
        if ended {
            printer.clear();
            crate::core::jobs::mark_seen(id);
            return Ok(Followed::Ended(Box::new(job.state.unwrap_or_default())));
        }
        if job.interrupted() {
            printer.clear();
            crate::core::jobs::mark_seen(id);
            bail!(
                "the job stopped when its process ended, before it said how it went; \
                 `fastf log` has what it did, and `fastf reconcile` finishes what it left"
            );
        }
        // The terminal went away — a closed window, a dropped ssh session, a
        // `kill`: this process stops following and the job goes on, which is
        // the whole point of it being a job. Only Ctrl-C asks it to stop.
        if crate::util::interrupt::is_set() && crate::util::interrupt::hung_up() {
            return Ok(Followed::LeftRunning);
        }
        if crate::util::interrupt::is_set() {
            let committed = job
                .state
                .as_ref()
                .is_some_and(|state| state.progress.committed);
            if committed {
                printer.say(&format!(
                    "  {} past the point of no return, so the rest carries on without this \
                     terminal; `fastf jobs watch {id}` follows it again",
                    "note:".cyan().bold()
                ));
                return Ok(Followed::LeftRunning);
            }
            if !asked {
                asked = true;
                crate::core::jobs::request_cancel(id)?;
                printer.say(&format!("  {}", "cancelling…".dimmed()));
            }
        }
        std::thread::sleep(TICK);
    }
}

/// Print a job's items the way the command line always printed its outcome,
/// and fail the way it always failed.
pub(crate) fn finish(state: &JobState) -> Result<()> {
    let mut failures = Vec::new();
    for item in &state.items {
        if item.headline.is_empty() {
            continue;
        }
        if let Some(error) = &item.error {
            failures.push(error.clone());
            continue;
        }
        println!("{}  {}", "✓".green().bold(), bold_headline(&item.headline));
        for line in &item.lines {
            println!("   {}", line.dimmed());
        }
        for note in &item.notes {
            eprintln!("{} {note}", "note:".cyan().bold());
        }
        if let Some(warning) = &item.warning {
            eprintln!("{} {warning}", "warning:".yellow().bold());
        }
    }
    match state.status {
        JobStatus::Done => Ok(()),
        JobStatus::Cancelled => {
            // What an interrupted run has always ended with.
            crate::util::interrupt::raise();
            bail!(
                "{}",
                failures
                    .first()
                    .cloned()
                    .unwrap_or_else(|| state.summary.clone())
            )
        }
        _ => match failures.as_slice() {
            [] => bail!("{}", state.summary),
            [one] => bail!("{one}"),
            many => bail!("{}:\n  {}", state.summary, many.join("\n  ")),
        },
    }
}

/// `Moved ID0047 name` with the id and name bold, as the verbs printed it.
fn bold_headline(headline: &str) -> String {
    let mut words = headline.splitn(3, ' ');
    let (verb, id, rest) = (
        words.next().unwrap_or(""),
        words.next().unwrap_or(""),
        words.next().unwrap_or(""),
    );
    format!("{verb} {} {}", id.green().bold(), rest.bold())
}

/// `fastf jobs`: every job, newest first.
pub fn list() -> Result<()> {
    crate::core::jobs::prune();
    let jobs = crate::core::jobs::list();
    if jobs.is_empty() {
        println!("No jobs.");
        return Ok(());
    }
    for job in &jobs {
        let (state, detail) = describe(job);
        println!(
            "  {}  {:<9}  {}  {}",
            job.id.dimmed(),
            state,
            job.title().bold(),
            detail.dimmed()
        );
    }
    Ok(())
}

fn describe(job: &JobView) -> (String, String) {
    let state = job.state.as_ref();
    if job.interrupted() {
        return (
            "stopped".yellow().to_string(),
            "its process ended before it said how it went; `fastf reconcile` finishes \
             anything it left"
                .to_string(),
        );
    }
    match state.map(|state| state.status) {
        Some(JobStatus::Running) | None => (
            "running".cyan().to_string(),
            state
                .map(|state| {
                    let item = state.progress.item_text();
                    if item.is_empty() {
                        state.progress.step_text()
                    } else {
                        format!("{item}: {}", state.progress.step_text())
                    }
                })
                .unwrap_or_default(),
        ),
        Some(JobStatus::Done) => (
            "done".green().to_string(),
            state.map(|state| state.summary.clone()).unwrap_or_default(),
        ),
        Some(JobStatus::Failed) => (
            "failed".red().to_string(),
            state.map(|state| state.summary.clone()).unwrap_or_default(),
        ),
        Some(JobStatus::Cancelled) => (
            "cancelled".yellow().to_string(),
            state.map(|state| state.summary.clone()).unwrap_or_default(),
        ),
    }
}

/// The job `id` names, or the newest one running.
fn pick(id: Option<String>) -> Result<String> {
    if let Some(id) = id {
        return Ok(id);
    }
    crate::core::jobs::list()
        .into_iter()
        .find(|job| job.alive)
        .map(|job| job.id)
        .ok_or_else(|| anyhow::anyhow!("no job is running; `fastf jobs` lists the others"))
}

/// `fastf jobs watch [id]`.
pub fn watch(id: Option<String>) -> Result<()> {
    let id = pick(id)?;
    match follow(&id)? {
        Followed::Ended(state) => {
            finish(&state)?;
            if state.items.iter().all(|item| item.headline.is_empty()) && !state.summary.is_empty()
            {
                println!("{}  {}", "✓".green().bold(), state.summary);
            }
            Ok(())
        }
        Followed::LeftRunning | Followed::Detached => Ok(()),
    }
}

/// `fastf jobs cancel [id]`.
pub fn cancel(id: Option<String>) -> Result<()> {
    let id = pick(id)?;
    let Some(job) = crate::core::jobs::view(&id) else {
        bail!("there is no job {id}");
    };
    if !job.alive {
        println!("Job {id} is not running.");
        return Ok(());
    }
    if job
        .state
        .as_ref()
        .is_some_and(|state| state.progress.committed)
    {
        println!(
            "Too late to cancel {}: it is past the point of no return, and what is left \
             finishes by itself.",
            job.title()
        );
        return Ok(());
    }
    crate::core::jobs::request_cancel(&id)?;
    println!(
        "Asked {} to stop; `fastf jobs watch {id}` shows it stop.",
        job.title()
    );
    Ok(())
}
