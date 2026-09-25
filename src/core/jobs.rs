//! Jobs: a long operation as a process of its own, described by files.
//!
//! A move, a copy out of the library, a delete and a reconcile each run in a
//! **worker** — this same binary started as `fastf --fastf-job <id>`, detached
//! from the terminal and from whoever started it — so closing or killing the
//! app or the command line never stops one part of the way, and any other
//! fastf can see it, follow it and cancel it. Nobody owns a job; the files do.
//!
//! `<data dir>/jobs/<id>/`, not a base, because a base may be a cloud mount that
//! uploads every write:
//!
//! - `request.json` — what to do, written once by whoever started the job;
//! - `lock` — held by the worker for its whole life. **A job is alive while
//!   its lock is held**: the OS releases it when the process ends however it
//!   ends, so a reused pid cannot lie and nothing stale is left to clean;
//! - `state.json` — where it has got to, rewritten atomically a few times a
//!   second ([`JobState`]); a job whose state says running while its lock is
//!   free was killed, and reconcile finishes what it left;
//! - `cancel` — created by anyone who wants it stopped;
//! - `seen` — created by the first surface that showed its outcome;
//! - `log` — its own log, every line at every level.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::core::assets::{JobStatus, Progress};
use crate::util::lockfile::DataLock;

/// The argument that makes this binary a job's worker. Taken off argv before
/// clap sees it, like `--relaunched`, so no shell completion ever offers it.
pub const WORKER_FLAG: &str = "--fastf-job";

/// The version of [`JobState`] this build writes; any version reads.
pub const STATE_VERSION: u32 = 1;

/// Finished jobs beyond this many, or older than [`KEEP_DAYS`], are pruned.
const KEEP: usize = 50;
const KEEP_DAYS: i64 = 30;

/// What a job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobKind {
    Move,
    Copy,
    Delete,
    Reconcile,
}

impl JobKind {
    pub fn verb(self) -> &'static str {
        match self {
            JobKind::Move => "move",
            JobKind::Copy => "copy",
            JobKind::Delete => "delete",
            JobKind::Reconcile => "reconcile",
        }
    }
}

/// One project a job acts on, as its starter saw it; the worker revalidates
/// it under the lock like every mutation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobItem {
    pub id: String,
    pub name: String,
    pub path: String,
    pub base: String,
    /// A move's target base, a copy's destination folder.
    pub target: String,
}

impl JobItem {
    pub fn of(project: &crate::core::library::Project, target: Option<&Path>) -> Result<Self> {
        let text = |path: &Path| crate::util::paths::storable(path, "a job's path");
        Ok(Self {
            id: project.id.clone(),
            name: project.name.clone(),
            path: text(&project.path)?,
            base: text(&project.base)?,
            target: match target {
                Some(target) => text(target)?,
                None => String::new(),
            },
        })
    }
}

/// What to do: written once, before the worker starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRequest {
    pub version: u32,
    pub kind: JobKind,
    #[serde(default)]
    pub items: Vec<JobItem>,
}

/// What became of one item, in the words both surfaces print.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemReport {
    /// `Moved ID0047 2026-07-26_Shoot_ID0047`.
    pub headline: String,
    /// The lines under it: from, to, what was copied.
    pub lines: Vec<String>,
    pub notes: Vec<String>,
    pub warning: Option<String>,
    /// Why it failed, when it did.
    pub error: Option<String>,
    /// The item is finished, housekeeping included.
    pub done: bool,
}

/// Where a job has got to — `state.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobState {
    pub version: u32,
    pub id: String,
    pub kind: Option<JobKind>,
    pub pid: u32,
    /// UTC, ISO-8601.
    pub started: String,
    pub updated: String,
    /// `Running` until the worker settles it.
    pub status: JobStatus,
    /// The current item's current step.
    pub progress: Progress,
    pub items: Vec<ItemReport>,
    /// Every move record and deleted folder this job owns: no reconcile
    /// touches one while the job is alive.
    pub operations: Vec<String>,
    /// The job in one sentence, once it has ended.
    pub summary: String,
    /// A reconcile's report.
    pub reconcile: Option<crate::core::provisioning::ReconcileReport>,
}

/// A job as a reader finds it.
#[derive(Debug, Clone)]
pub struct JobView {
    pub id: String,
    pub request: Option<JobRequest>,
    pub state: Option<JobState>,
    /// Its worker holds its lock.
    pub alive: bool,
    pub seen: bool,
    pub cancel_asked: bool,
}

impl JobView {
    /// The worker ended without saying how the job ended: killed.
    pub fn interrupted(&self) -> bool {
        !self.alive
            && self
                .state
                .as_ref()
                .is_none_or(|state| state.status == JobStatus::Running)
    }

    /// Running, as far as anyone can tell.
    pub fn running(&self) -> bool {
        self.alive
    }

    pub fn kind(&self) -> Option<JobKind> {
        self.request
            .as_ref()
            .map(|request| request.kind)
            .or_else(|| self.state.as_ref().and_then(|state| state.kind))
    }

    /// `move ID0047 2026-07-26_Shoot_ID0047`, `move 3 projects`, `reconcile`.
    pub fn title(&self) -> String {
        let verb = self.kind().map_or("job", JobKind::verb);
        match self
            .request
            .as_ref()
            .map(|request| request.items.as_slice())
        {
            Some([one]) => format!("{verb} {} {}", one.id, one.name),
            Some(many) if many.len() > 1 => format!("{verb} {} projects", many.len()),
            _ => verb.to_string(),
        }
    }
}

/// Where jobs live, when there is a data directory.
pub fn root() -> Option<PathBuf> {
    // A unit test that has not sandboxed the data directory must not see the
    // developer's own jobs, let alone start one.
    #[cfg(test)]
    std::env::var_os("FASTF_INSTALL_DIR")?;
    crate::util::paths::try_install_dir()
        .ok()
        .map(|(dir, _)| dir.join("jobs"))
}

fn root_or_error() -> Result<PathBuf> {
    root().context("there is no data directory to keep a job in")
}

pub fn dir(id: &str) -> Result<PathBuf> {
    if !crate::core::transactions::is_operation_id(id) {
        bail!("'{id}' is not a job");
    }
    Ok(root_or_error()?.join(id))
}

/// Write a new job's request. Its worker is not started yet.
pub fn create(kind: JobKind, items: Vec<JobItem>) -> Result<String> {
    let root = root_or_error()?;
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    for _ in 0..64 {
        let id = crate::core::transactions::next_operation_id();
        let dir = root.join(&id);
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let request = JobRequest {
                    version: STATE_VERSION,
                    kind,
                    items,
                };
                let text = serde_json::to_string_pretty(&request)?;
                std::fs::write(dir.join("request.json"), text)
                    .with_context(|| format!("writing {}", dir.display()))?;
                return Ok(id);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("creating {}", dir.display()));
            }
        }
    }
    bail!("could not find a free name for a job in {}", root.display())
}

/// Start a job: write its request, start its worker, and wait until the
/// worker says it is running. A worker that never does is an error naming
/// where its log is.
pub fn start(kind: JobKind, items: Vec<JobItem>) -> Result<String> {
    prune();
    let id = create(kind, items)?;
    spawn_worker(&id)?;
    let started = Instant::now();
    loop {
        if read_state(&id).is_some_and(|state| state.pid != 0) {
            return Ok(id);
        }
        if started.elapsed() > Duration::from_secs(10) {
            bail!(
                "the job's worker did not start; its log may say why: {}",
                crate::util::paths::display_path(&dir(&id)?.join("log"))
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Start `fastf --fastf-job <id>`, detached: its own session on unix, so no
/// terminal's hang-up or Ctrl-C reaches it; no console and a process group of
/// its own on Windows, out of the starter's job object where that allows it
/// (OpenSSH kills its session's job object on disconnect).
fn spawn_worker(id: &str) -> Result<()> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().context("finding fastf's own program")?;
    let (install, _) = crate::util::paths::try_install_dir()?;
    let command = || {
        let mut command = Command::new(&exe);
        command
            .arg(WORKER_FLAG)
            .arg(id)
            .env("FASTF_INSTALL_DIR", &install)
            .env("FASTF_NO_RELAUNCH", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    };
    #[cfg(unix)]
    let child = {
        use std::os::unix::process::CommandExt;
        let mut command = command();
        // SAFETY: `setsid` is async-signal-safe and touches nothing of the
        // parent's; it is the one call made between fork and exec.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        command.spawn()
    };
    #[cfg(windows)]
    let child = {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        command()
            .creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            // A job object that does not allow breaking away refuses the
            // flag; the worker then lives as long as that job does.
            .or_else(|_| command().creation_flags(flags).spawn())
    };
    #[cfg(not(any(unix, windows)))]
    let child = command().spawn();
    let mut child = child.context("starting the job's worker")?;
    // Reaped on a thread, so a long-lived app leaves no zombie per job; the
    // thread ends with the worker, or with this process, when the worker is
    // handed to init and goes on.
    std::thread::Builder::new()
        .name("fastf-reap".to_string())
        .spawn(move || {
            let _ = child.wait();
        })
        .ok();
    Ok(())
}

pub fn read_request(id: &str) -> Option<JobRequest> {
    let text = std::fs::read_to_string(dir(id).ok()?.join("request.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// The job's state, when it has one that reads. A newer fastf's fields are
/// ignored and missing ones take their defaults.
pub fn read_state(id: &str) -> Option<JobState> {
    let text = std::fs::read_to_string(dir(id).ok()?.join("state.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_state(state: &JobState) -> Result<()> {
    crate::util::atomic::write_json(&dir(&state.id)?.join("state.json"), state)
}

/// The path of the lock a job's worker holds.
pub fn lock_path(id: &str) -> Result<PathBuf> {
    Ok(dir(id)?.join("lock"))
}

/// Whether the job's worker is running: whether its lock is held.
pub fn is_alive(id: &str) -> bool {
    let Ok(path) = lock_path(id) else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    matches!(DataLock::try_acquire_at(&path), Ok(None))
}

/// Ask a job to stop. Its worker looks a few times a second; what it does
/// depends on where it is — see `Progress::committed`.
pub fn request_cancel(id: &str) -> Result<()> {
    let path = dir(id)?.join("cancel");
    std::fs::write(&path, b"").with_context(|| format!("writing {}", path.display()))
}

pub fn cancel_requested(id: &str) -> bool {
    dir(id).is_ok_and(|dir| dir.join("cancel").exists())
}

/// Say that the job's outcome has been shown to someone.
pub fn mark_seen(id: &str) {
    if let Ok(dir) = dir(id) {
        let _ = std::fs::write(dir.join("seen"), b"");
    }
}

/// Look at one job.
pub fn view(id: &str) -> Option<JobView> {
    let dir = dir(id).ok()?;
    if !dir.is_dir() {
        return None;
    }
    Some(JobView {
        id: id.to_string(),
        request: read_request(id),
        state: read_state(id),
        alive: is_alive(id),
        seen: dir.join("seen").exists(),
        cancel_asked: dir.join("cancel").exists(),
    })
}

/// Every job there is, newest first.
pub fn list() -> Vec<JobView> {
    let Some(root) = root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| crate::core::transactions::is_operation_id(name))
        .collect();
    // An id begins with its time in hex nanoseconds, fixed width until 2554.
    ids.sort_by(|left, right| right.cmp(left));
    ids.iter().filter_map(|id| view(id)).collect()
}

/// The processes of every live job. **What reconcile leaves alone** is
/// whatever one of them made: an operation id carries the pid of the process
/// that minted it (`transactions::next_operation_id`), and a worker writes its
/// pid before it does anything — so there is no moment in which a job's record
/// exists and its job cannot be told, which a list of operations written a
/// few times a second would leave.
pub fn live_workers() -> HashSet<u32> {
    list()
        .into_iter()
        .filter(|job| job.alive)
        .filter_map(|job| job.state)
        .map(|state| state.pid)
        .filter(|pid| *pid != 0)
        .collect()
}

/// Whether `operation` was made by one of `live` — a live job's own.
pub fn owned_by(operation: &str, live: &HashSet<u32>) -> bool {
    operation
        .split('-')
        .nth(1)
        .and_then(|pid| u32::from_str_radix(pid, 16).ok())
        .is_some_and(|pid| live.contains(&pid))
}

/// A sentence naming the live job that holds the data lock, if one does:
/// what a wait for the lock says instead of "another fastf process".
pub fn lock_holder() -> Option<String> {
    list().into_iter().find_map(|job| {
        let state = job.state.as_ref()?;
        if !job.alive || !state.progress.holds_lock {
            return None;
        }
        let step = state.progress.step_text();
        Some(format!(
            "the {} ({step}; `fastf jobs` shows it)",
            job.title()
        ))
    })
}

/// Remove finished jobs nobody needs: ended (or killed), shown to someone or
/// old, beyond the newest fifty or older than thirty days. A live job,
/// and an interrupted one nobody has seen, are kept.
pub fn prune() {
    let now = chrono::Utc::now();
    for (index, job) in list().into_iter().enumerate() {
        if job.alive || (job.interrupted() && !job.seen) {
            continue;
        }
        let old = job
            .state
            .as_ref()
            .and_then(|state| chrono::DateTime::parse_from_rfc3339(&state.started).ok())
            .is_some_and(|started| {
                (now - started.with_timezone(&chrono::Utc)).num_days() > KEEP_DAYS
            });
        if (index >= KEEP || (old && job.seen))
            && let Ok(dir) = dir(&job.id)
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_is_owned_by_the_process_that_made_it() {
        let live: HashSet<u32> = [0x7395d].into_iter().collect();
        assert!(owned_by("18d8983cdf094ce9-7395d-1", &live));
        assert!(!owned_by("18d8983cdf094ce9-7395e-1", &live));
        assert!(!owned_by("not-an-id", &live));
    }

    #[test]
    fn a_state_from_another_version_reads() {
        let state: JobState =
            serde_json::from_str(r#"{"id":"1-2-3","status":"running","novel":true}"#).unwrap();
        assert_eq!(state.id, "1-2-3");
        assert_eq!(state.status, JobStatus::Running);
    }

    #[test]
    fn a_job_is_named_by_what_it_does() {
        let item = |id: &str| JobItem {
            id: id.to_string(),
            name: "Shoot".to_string(),
            ..JobItem::default()
        };
        let view = |kind, items| JobView {
            id: "1-2-3".to_string(),
            request: Some(JobRequest {
                version: 1,
                kind,
                items,
            }),
            state: None,
            alive: false,
            seen: false,
            cancel_asked: false,
        };
        assert_eq!(
            view(JobKind::Move, vec![item("ID1")]).title(),
            "move ID1 Shoot"
        );
        assert_eq!(
            view(JobKind::Copy, vec![item("ID1"), item("ID2")]).title(),
            "copy 2 projects"
        );
        assert_eq!(view(JobKind::Reconcile, vec![]).title(), "reconcile");
        assert!(
            view(JobKind::Reconcile, vec![]).interrupted(),
            "no state and no lock"
        );
    }
}
