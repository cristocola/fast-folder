//! What is using a folder — asked before a verb takes a folder away from its
//! path: a move (the rename on one filesystem and the staged copy alike), a
//! rename, a delete.
//!
//! **Generic, from what the operating system reports, never per tool.** A
//! container, a dev server, a database, an editor, a shell, a service: each
//! is seen through what every program leaves in the same places, so one fastf
//! has never heard of is seen the same way.
//!
//! - **Linux** (`linux.rs`, `units.rs`, judged in `judge.rs`): every
//!   process's working folder, executable, open files and how each was
//!   opened, loaded libraries and arguments — this user's processes, which a
//!   person can stop; every process's mount table, whoever owns it, so a
//!   container or sandbox that mounts part of the folder is seen; another
//!   filesystem mounted inside it; a systemd unit file that names it or is a
//!   link into it. Every process is read at once under one deadline, and one
//!   that does not answer — stuck behind a dead mount — is named as such,
//!   never waited for.
//! - **Windows** (`windows.rs`): what holds anything in the tree. Windows
//!   refuses the rename itself then, so that is physical, and asked when the
//!   rename refuses, or after a staged move's scan.
//!
//! **One table weighs every use** ([`Use::weight`]): a physical use refuses
//! whatever, an advisory one until the verb is forced, a note only goes in
//! the result. fastf's own processes never count, nor does whoever asked for
//! the verb and every process above them ([`Guard::asking`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::config::Config;
use crate::core::library::Project;
use crate::core::transactions::MoveManifest;

#[cfg(target_os = "linux")]
mod judge;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(test, target_os = "linux"))]
mod tests;
#[cfg(target_os = "linux")]
mod units;
#[cfg(windows)]
mod windows;

/// What takes a folder away from its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Move,
    Rename,
    Delete,
}

impl Verb {
    pub fn word(self) -> &'static str {
        match self {
            Verb::Move => "move",
            Verb::Rename => "rename",
            Verb::Delete => "delete",
        }
    }

    pub fn past(self) -> &'static str {
        match self {
            Verb::Move => "moved",
            Verb::Rename => "renamed",
            Verb::Delete => "deleted",
        }
    }

    /// What the folder is called once the verb went ahead.
    fn old_folder(self) -> &'static str {
        match self {
            Verb::Delete => "the deleted folder",
            Verb::Move | Verb::Rename => "the old folder",
        }
    }
}

/// One process of the chain that asked for a verb: its pid and when it
/// started (clock ticks after boot, `util::process::Stat::started`), so a pid
/// reused by somebody else is not taken for it. `0` is "not known" — the pid
/// alone then.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asking {
    pub pid: u32,
    #[serde(default)]
    pub started: u64,
}

impl Asking {
    #[cfg(target_os = "linux")]
    fn is(&self, pid: u32, started: u64) -> bool {
        self.pid == pid && (self.started == 0 || self.started == started)
    }
}

/// How a verb weighs what uses its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    /// Go ahead past what only advises: what used the folder becomes notes,
    /// and a line in the log.
    pub force: bool,
    /// Whoever asked for the verb and every process above them — the shell
    /// that typed the command, the agent that ran it, the app and its
    /// terminal — which never count. A job carries its starter's
    /// (`core::jobs::JobRequest::asking`), since the worker's own parents are
    /// the service manager.
    pub asking: Vec<Asking>,
    /// Whether to look at all. Off only for the library's compatibility entry
    /// points, which tests drive and no surface reaches; Windows still
    /// refuses what it refuses there.
    pub look: bool,
}

impl Guard {
    /// Asked from this process: its own chain.
    pub fn here(force: bool) -> Self {
        Self::asked_by(force, asking_chain())
    }

    /// Asked by `asking`, recorded elsewhere — a job's starter.
    pub fn asked_by(force: bool, asking: Vec<Asking>) -> Self {
        Self {
            force,
            asking,
            look: true,
        }
    }

    /// No look: the library's compatibility entry points.
    pub fn off() -> Self {
        Self {
            force: false,
            asking: Vec::new(),
            look: false,
        }
    }
}

/// A folder asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Canonical, as the library holds a project.
    pub path: PathBuf,
    /// The configuration's spelling of it, when that differs — a base
    /// reached through a link: a unit file or an argument may spell the
    /// folder either way.
    pub spelled: Option<PathBuf>,
    /// What the sentences call it: the project's id.
    pub label: String,
}

impl Folder {
    /// `project`'s folder, both spellings: the configured one is rebuilt from
    /// the configured base whose canonical form is the folder's parent, asked
    /// under the deadline a base is given.
    pub fn of(cfg: &Config, project: &Project) -> Self {
        Self::configured(cfg, &project.path, &project.id)
    }

    /// [`Self::of`], for the folder at `path`, called `label`.
    pub fn configured(cfg: &Config, path: &Path, label: &str) -> Self {
        let path = resolved(path);
        let spelled = match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => {
                let configured: Vec<PathBuf> = cfg
                    .base_candidates()
                    .into_iter()
                    .map(|base| {
                        crate::core::config::expand_base_path(&base.to_string_lossy())
                            .unwrap_or(base)
                    })
                    .filter(|base| base != parent)
                    .collect();
                let canonical = crate::util::paths::answer_within(
                    &configured,
                    crate::util::paths::PROBE_TIMEOUT,
                    |base| crate::util::paths::canonical(base).ok(),
                );
                configured
                    .iter()
                    .zip(canonical)
                    .find(|(_, real)| real.as_ref().and_then(Option::as_deref) == Some(parent))
                    .map(|(base, _)| base.join(name))
            }
            _ => None,
        };
        Self {
            path,
            spelled,
            label: label.to_string(),
        }
    }

    /// A folder known by its path alone.
    pub fn at(path: &Path, label: &str) -> Self {
        Self {
            path: resolved(path),
            spelled: None,
            label: label.to_string(),
        }
    }

    /// Every spelling, the canonical first.
    pub fn spellings(&self) -> Vec<PathBuf> {
        std::iter::once(self.path.clone())
            .chain(self.spelled.clone())
            .collect()
    }
}

/// `path`, canonical, under the deadline a base is given; as given when it
/// does not answer. Canonicalizing a folder on a mount that stopped
/// answering blocks for the kernel's own timeout, and the question would
/// never come back.
fn resolved(path: &Path) -> PathBuf {
    crate::util::paths::canonical_in_time(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Everything using one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holders {
    /// The folder asked about, canonical. Every path in a use is relative to
    /// it (empty for the folder itself).
    pub root: PathBuf,
    /// What the sentences call it.
    pub label: String,
    pub uses: Vec<Use>,
}

/// One thing using the folder, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub by: By,
    pub how: How,
}

/// Who uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum By {
    Process {
        pid: u32,
        program: String,
    },
    /// A systemd unit file: `system` for one of the system manager's.
    Unit {
        name: String,
        system: bool,
    },
    /// Nothing that can be named: a mount, a folder Windows holds.
    Nobody,
}

/// How it uses it. Paths are relative to [`Holders::root`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    /// Its working folder is there. `shell`: a shell, whose only tie is its
    /// prompt (its children are judged themselves).
    WorkingIn {
        at: PathBuf,
        shell: bool,
    },
    Open {
        at: PathBuf,
        writing: bool,
    },
    /// Its executable is there.
    Runs(PathBuf),
    /// A library or another file mapped from there.
    Loaded(PathBuf),
    /// Started with a path there among its arguments.
    Named(PathBuf),
    /// A folder from inside it is mounted at `at` — a container's or a
    /// sandbox's mount, or a bind on this machine.
    Mounts {
        folder: PathBuf,
        at: PathBuf,
    },
    /// Another filesystem is mounted at a folder inside it.
    MountedInside(PathBuf),
    /// A unit file names it, or is a link into it. `state` is what systemd
    /// said of the unit; `dormant` a unit disabled, stopped and started by
    /// nothing.
    Unit {
        tie: Tie,
        state: String,
        dormant: bool,
    },
    /// These processes did not answer in time: any of them may use the
    /// folder. One use for all of them, so a mount that stopped answering
    /// under many processes is one note, not one each.
    Silent(Vec<u32>),
    /// These processes were never asked, since every reader was on one
    /// that did not answer: nothing is known of them.
    Unasked(Vec<u32>),
    /// Windows: a program holds this file.
    Held(PathBuf),
    /// Windows: a program works in this folder.
    BusyFolder(PathBuf),
}

/// What ties a unit file to the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tie {
    /// The line that names it.
    Line(String),
    /// The unit file is a link to this file inside it.
    Link(PathBuf),
}

/// How much a use weighs against a verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Refuses whatever: the system itself would not let the verb finish.
    Physical,
    /// Refuses until the verb is forced.
    Advisory,
    /// Said in the result, never refuses.
    Note,
}

impl Use {
    /// **The one table.** Windows' holds are physical, and so is a
    /// filesystem mounted inside a folder being deleted — the removal would
    /// keep it, and the hidden folder with it, for good. A shell whose only
    /// tie is its prompt, a unit nothing will start, and a process that did
    /// not answer are notes. Everything else advises — a process never
    /// asked included, since nothing was learned of it.
    pub fn weight(&self, verb: Verb) -> Weight {
        match &self.how {
            How::Held(_) | How::BusyFolder(_) => Weight::Physical,
            How::MountedInside(_) if verb == Verb::Delete => Weight::Physical,
            How::WorkingIn { shell: true, .. } => Weight::Note,
            How::Unit { dormant: true, .. } => Weight::Note,
            How::Silent(_) => Weight::Note,
            _ => Weight::Advisory,
        }
    }

    /// `node (pid 4242)`, `coda.service`.
    pub fn who(&self) -> String {
        match &self.by {
            By::Process { pid, program } if program.is_empty() => format!("pid {pid}"),
            By::Process { pid, program } => format!("{program} (pid {pid})"),
            By::Unit { name, .. } => name.clone(),
            By::Nobody => String::new(),
        }
    }

    /// What it does, in the present tense, as a refusal lists it.
    pub fn line(&self) -> String {
        let who = self.who();
        match &self.how {
            How::WorkingIn { at: path, .. } => format!("{who} is working in {}", at(path)),
            How::Open {
                at: path,
                writing: true,
            } => format!("{who} has {} open for writing", at(path)),
            How::Open { at: path, .. } => format!("{who} has {} open", at(path)),
            How::Runs(path) => format!("{who} runs {}", at(path)),
            How::Loaded(path) => format!("{who} has {} loaded", at(path)),
            How::Named(path) => format!("{who} was started with {}", at(path)),
            How::Mounts { folder, at: point } if self.by == By::Nobody => format!(
                "{} is mounted at {} too",
                at(folder),
                crate::util::paths::display_path(point)
            ),
            How::Mounts { folder, at: point } => format!(
                "{who} has {} mounted at {}",
                at(folder),
                crate::util::paths::display_path(point)
            ),
            How::MountedInside(path) => {
                format!("{} is another filesystem, mounted inside it", at(path))
            }
            How::Unit {
                tie: Tie::Line(line),
                state,
                ..
            } => format!("{who} ({state}) names it: {line}"),
            How::Unit {
                tie: Tie::Link(path),
                state,
                ..
            } => format!("{who} ({state}) is a link to {}", at(path)),
            How::Silent(pids) => format!("{} did not answer", pids_text(pids)),
            How::Unasked(pids) => format!(
                "{} {} not looked at: the ones before {} did not answer",
                processes_text(pids),
                crate::util::plural::of(pids.len(), "was", "were"),
                crate::util::plural::of(pids.len(), "it", "them")
            ),
            How::Held(path) => format!("{who} has {} open", at(path)),
            How::BusyFolder(path) => format!("a program is working in {}", at(path)),
        }
    }

    /// What a person should know once the verb went ahead anyway — or, for a
    /// note, went ahead past it.
    pub fn note(&self, verb: Verb) -> String {
        let who = self.who();
        let old = verb.old_folder();
        let again = match verb {
            Verb::Delete => "stop it",
            Verb::Move | Verb::Rename => "restart it from the new one",
        };
        match &self.how {
            How::WorkingIn { shell: true, .. } => match verb {
                Verb::Delete => format!("{who} has its prompt in {old}; cd out of it"),
                _ => format!("{who} has its prompt in {old}; cd to the new one"),
            },
            How::WorkingIn { .. } => format!("{who} was working in {old}; {again}"),
            How::Open { .. } | How::Runs(_) | How::Loaded(_) | How::Named(_) => {
                format!("{who} was using {old} ({}); {again}", self.what())
            }
            How::Mounts { folder, at: point } if self.by == By::Nobody => format!(
                "{} of {old} is still mounted at {}; mount it again from where it is now",
                at(folder),
                crate::util::paths::display_path(point)
            ),
            How::Mounts { folder, at: point } => format!(
                "{who} still has {} of {old} mounted at {}; {again}",
                at(folder),
                crate::util::paths::display_path(point)
            ),
            How::MountedInside(path) => format!(
                "{} was another filesystem mounted inside {old}; whatever mounts it there \
                 still names {old}",
                at(path)
            ),
            How::Unit {
                tie, dormant: true, ..
            } => format!(
                "{who} names {old} ({}) but is disabled and stopped; edit it before you start it",
                tie_text(tie)
            ),
            How::Unit { tie, .. } => match verb {
                Verb::Delete => format!("{who} still names {old} ({}); edit it", tie_text(tie)),
                _ => format!(
                    "{who} still names {old} ({}); edit it, then restart it",
                    tie_text(tie)
                ),
            },
            How::Silent(pids) => format!(
                "{} did not answer, so fastf could not tell whether {} {old}",
                pids_text(pids),
                crate::util::plural::of(pids.len(), "it used", "they used")
            ),
            How::Unasked(pids) => format!(
                "{} {} not looked at, so fastf could not tell whether {} {old}",
                processes_text(pids),
                crate::util::plural::of(pids.len(), "was", "were"),
                crate::util::plural::of(pids.len(), "it used", "they used")
            ),
            How::Held(path) | How::BusyFolder(path) => {
                format!("{} was in use", at(path))
            }
        }
    }

    /// The use in a few words, for a note: `working in src`, `data/x open for
    /// writing`.
    fn what(&self) -> String {
        match &self.how {
            How::WorkingIn { at: path, .. } => format!("working in {}", at(path)),
            How::Open {
                at: path,
                writing: true,
            } => format!("{} open for writing", at(path)),
            How::Open { at: path, .. } => format!("{} open", at(path)),
            How::Runs(path) => format!("running {}", at(path)),
            How::Loaded(path) => format!("{} loaded", at(path)),
            How::Named(path) => format!("started with {}", at(path)),
            _ => String::new(),
        }
    }
}

/// A path inside the folder as a sentence says it.
fn at(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        "the project folder".to_string()
    } else {
        path.display().to_string()
    }
}

/// `pid 777`, `pids 777 and 778`, `pids 1, 2, 3 and 9 more`.
fn pids_text(pids: &[u32]) -> String {
    const NAMED: usize = 3;
    let named: Vec<String> = pids.iter().take(NAMED).map(u32::to_string).collect();
    match (pids.len(), named.as_slice()) {
        (1, [one]) => format!("pid {one}"),
        (2, [one, two]) => format!("pids {one} and {two}"),
        (n, _) if n <= NAMED => format!("pids {} and {}", named[..n - 1].join(", "), named[n - 1]),
        (n, _) => format!("pids {} and {} more", named.join(", "), n - NAMED),
    }
}

/// `1 process`, `68 processes`.
fn processes_text(pids: &[u32]) -> String {
    format!(
        "{} {}",
        pids.len(),
        crate::util::plural::of(pids.len(), "process", "processes")
    )
}

fn tie_text(tie: &Tie) -> String {
    match tie {
        Tie::Line(line) => line.clone(),
        Tie::Link(path) => format!("a link to {}", at(path)),
    }
}

/// How many uses a refusal lists before `and k more`.
pub const LISTED: usize = 6;

impl Holders {
    /// Nothing using `folder`.
    pub fn nothing(folder: &Folder) -> Self {
        Self {
            root: folder.path.clone(),
            label: folder.label.clone(),
            uses: Vec::new(),
        }
    }

    /// The uses that keep `verb` from going ahead: the physical ones, and the
    /// advisory ones unless forced.
    pub fn refusing(&self, verb: Verb, force: bool) -> Vec<&Use> {
        self.uses
            .iter()
            .filter(|found| match found.weight(verb) {
                Weight::Physical => true,
                Weight::Advisory => !force,
                Weight::Note => false,
            })
            .collect()
    }

    /// Whether anything but a note uses it.
    pub fn in_use(&self, verb: Verb) -> bool {
        !self.refusing(verb, false).is_empty()
    }

    /// Whether something uses it that nothing overrides.
    pub fn physical(&self, verb: Verb) -> bool {
        !self.refusing(verb, true).is_empty()
    }

    /// What refuses `verb`, each in a line, at most `most` of them and then
    /// `and k more`.
    pub fn listed(&self, verb: Verb, force: bool, most: usize) -> Vec<String> {
        let refusing = self.refusing(verb, force);
        let mut lines: Vec<String> = refusing.iter().take(most).map(|u| u.line()).collect();
        if refusing.len() > most {
            lines.push(format!("and {} more", refusing.len() - most));
        }
        lines
    }

    /// The refusal `verb` gives when something keeps it from going ahead —
    /// what, and what to do. Windows keeps its own two sentences.
    pub fn refusal(&self, verb: Verb, force: bool) -> Option<String> {
        let refusing = self.refusing(verb, force);
        if refusing.is_empty() {
            return None;
        }
        if let Some(said) = self.windows_refusal() {
            return Some(said);
        }
        let mounts_only = refusing
            .iter()
            .all(|found| matches!(found.how, How::MountedInside(_)));
        let (word, past) = (verb.word(), verb.past());
        let label = &self.label;
        Some(match refusing.as_slice() {
            // Nothing to stop: a look that could not be made is made again.
            [one] if matches!(one.how, How::Unasked(_)) => format!(
                "{}, so {label} was not {past}; {word} it again in a moment",
                one.line()
            ),
            [one] => {
                let stop = if mounts_only { "unmount it" } else { "stop it" };
                format!(
                    "{}, so {label} was not {past}; {stop}, then {word} it again",
                    one.line()
                )
            }
            _ => {
                let stop = if mounts_only {
                    "unmount them"
                } else {
                    "stop them"
                };
                format!(
                    "{label} is in use, so it was not {past}:\n  {}\n{stop}, then {word} it again",
                    self.listed(verb, force, LISTED).join("\n  ")
                )
            }
        })
    }

    /// Windows' sentences, unchanged: a held file, else a folder a program
    /// works in.
    fn windows_refusal(&self) -> Option<String> {
        let held: Vec<&Use> = self
            .uses
            .iter()
            .filter(|found| matches!(found.how, How::Held(_)))
            .collect();
        if let Some(first) = held.first() {
            let (How::Held(path), By::Process { program, .. }) = (&first.how, &first.by) else {
                return None;
            };
            let more = match held.len() {
                1 => String::new(),
                n => format!(" (and {} more)", n - 1),
            };
            return Some(format!(
                "{} has {} open{more}; close it, or quit {program}, then try again — Windows \
                 will not move a folder while anything in it is open",
                first.who(),
                at(path),
            ));
        }
        let busy: Vec<&PathBuf> = self
            .uses
            .iter()
            .filter_map(|found| match &found.how {
                How::BusyFolder(path) => Some(path),
                _ => None,
            })
            .collect();
        let first = busy.first()?;
        let more = match busy.len() {
            1 => String::new(),
            n => format!(" (and {} more folders)", n - 1),
        };
        Some(format!(
            "a program is working in {}{more} — a console, or another program whose working \
             folder is there; leave it, then try again — Windows will not move a folder while \
             a program works in it",
            at(first)
        ))
    }

    /// What a verb that went ahead says about what used the folder: every
    /// note, and — when it was forced — every advisory use.
    pub fn notes(&self, verb: Verb) -> Vec<String> {
        self.uses
            .iter()
            .filter(|found| found.weight(verb) != Weight::Physical)
            .map(|found| found.note(verb))
            .collect()
    }

    /// What refuses `verb` in one sentence — the app's dialog, which wraps
    /// words and draws no list.
    pub fn sentence(&self, verb: Verb, most: usize) -> String {
        self.listed(verb, false, most).join("; ")
    }
}

/// A verb refused because something uses its folder, which forcing it would
/// go past: the refusal is the error, so the surfaces can offer that. A
/// physical refusal is a plain error, since nothing goes past it.
#[derive(Debug)]
pub struct InUse(pub String);

impl std::fmt::Display for InUse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InUse {}

/// Whether `error` is a refusal that forcing the verb would go past.
pub fn is_in_use(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<InUse>().is_some())
}

/// Whether a verb asks before it renames: Linux, where the answer is cheap
/// and advisory (the rename itself would succeed, and leave behind whatever
/// named the folder). Windows answers when its rename refuses, which is
/// physical, and a walk with the Restart Manager on every rename would cost
/// more than it tells.
pub const BEFORE_THE_RENAME: bool = cfg!(target_os = "linux");

/// Whether this platform can say who is working in a folder at all. Where it
/// cannot, a surface shows no `working` fact rather than `nobody`, which
/// would be a guess.
pub const WORKING_KNOWN: bool = cfg!(target_os = "linux");

/// This process, every process above it, and the processes its own pipes
/// join it to (`fastf move … | tail`), each with when it started: what a
/// verb asked from here records as [`Guard::asking`]. Linux only; empty
/// elsewhere.
pub fn asking_chain() -> Vec<Asking> {
    #[cfg(target_os = "linux")]
    {
        let mut chain = Vec::new();
        let mut pid = std::process::id();
        // A chain longer than any real one is a loop the table cannot have,
        // but is not trusted to lack.
        for _ in 0..64 {
            let Some(stat) = crate::util::process::stat(pid) else {
                break;
            };
            chain.push(Asking {
                pid,
                started: stat.started,
            });
            if stat.ppid == 0 || stat.ppid == pid {
                break;
            }
            pid = stat.ppid;
        }
        let partners = linux::pipeline_partners(&chain);
        chain.extend(partners);
        chain
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// What uses each of `folders`, in one pass. Empty where the platform is not
/// asked before a rename ([`BEFORE_THE_RENAME`]), so a surface behaves the
/// same everywhere.
pub fn in_use_all(folders: &[Folder], asking: &[Asking]) -> Vec<Holders> {
    #[cfg(target_os = "linux")]
    {
        crate::util::trace::hit("what uses a folder");
        let spellings: Vec<Vec<PathBuf>> = folders.iter().map(Folder::spellings).collect();
        let pass = linux::read(&spellings, asking, linux::Want::Everything);
        let units = units::find(&spellings);
        let shells = linux::shells();
        judge::judge(&spellings, &pass, &shells, &units)
            .into_iter()
            .zip(folders)
            .map(|(uses, folder)| Holders {
                uses,
                ..Holders::nothing(folder)
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = asking;
        folders.iter().map(Holders::nothing).collect()
    }
}

/// What uses `folder`.
pub fn in_use(folder: &Folder, asking: &[Asking]) -> Holders {
    in_use_all(std::slice::from_ref(folder), asking)
        .pop()
        .unwrap_or_else(|| Holders::nothing(folder))
}

/// **The authoritative question**, asked under the data lock before the
/// folder goes: what uses it, refused unless the guard goes past it
/// ([`weigh`]). Nothing is asked where the platform is not asked before a
/// rename, or with the guard off.
pub fn check(folder: &Folder, verb: Verb, guard: &Guard) -> anyhow::Result<Holders> {
    if !BEFORE_THE_RENAME || !guard.look {
        return Ok(Holders::nothing(folder));
    }
    weigh(in_use(folder, &guard.asking), verb, guard)
}

/// [`check`], asked by a staged move after its scan — on Windows, about
/// every file and folder it found, whatever the guard says, since a held one
/// would keep the old copy from ever being set aside.
pub fn check_scanned(
    folder: &Folder,
    manifest: &MoveManifest,
    verb: Verb,
    guard: &Guard,
) -> anyhow::Result<Holders> {
    #[cfg(windows)]
    {
        let found = Holders {
            uses: windows::in_tree(&folder.path, Some(manifest)),
            ..Holders::nothing(folder)
        };
        weigh(found, verb, guard)
    }
    #[cfg(not(windows))]
    {
        let _ = manifest;
        check(folder, verb, guard)
    }
}

/// Refuse when what was found keeps `verb` from going ahead; otherwise hand
/// it back for the notes. A forced verb that went past something says what
/// in the log, since its notes otherwise live only in the job's state.
pub fn weigh(found: Holders, verb: Verb, guard: &Guard) -> anyhow::Result<Holders> {
    if let Some(refusal) = found.refusal(verb, guard.force) {
        if found.physical(verb) {
            anyhow::bail!("{refusal}");
        }
        return Err(anyhow::Error::new(InUse(refusal)));
    }
    if guard.force && found.in_use(verb) {
        crate::util::log::warn(format!(
            "forced the {} of {} past what used it: {}",
            verb.word(),
            found.label,
            found.listed(verb, false, usize::MAX).join("; ")
        ));
    }
    Ok(found)
}

/// Who holds `folder` where Windows refused to rename it, as the refusal to
/// give instead of "access is denied"; `None` elsewhere, and where nothing
/// does.
pub fn held_on_windows(folder: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        in_tree(folder).refusal(Verb::Move, true)
    }
    #[cfg(not(windows))]
    {
        let _ = folder;
        None
    }
}

/// What holds anything in `root` on Windows, walking it.
#[cfg(windows)]
pub fn in_tree(root: &Path) -> Holders {
    let folder = Folder::at(root, "");
    Holders {
        uses: windows::in_tree(&folder.path, None),
        ..Holders::nothing(&folder)
    }
}

/// Which programs are working in each of `roots` — their working folder
/// inside it — by name, each once, for every root at once: one pass over the
/// processes answers for a whole desk. Linux only; elsewhere the answer is
/// empty, and a surface shows nothing rather than a guess. fastf's own
/// processes are left out, as everywhere here.
pub fn working_in_all(roots: &[PathBuf]) -> std::collections::HashMap<PathBuf, Vec<String>> {
    let mut answer: std::collections::HashMap<PathBuf, Vec<String>> = roots
        .iter()
        .map(|root| (root.clone(), Vec::new()))
        .collect();
    for (index, _, program) in working(roots) {
        if let Some(names) = answer.get_mut(&roots[index])
            && !names.contains(&program)
        {
            names.push(program);
        }
    }
    for names in answer.values_mut() {
        names.sort();
    }
    answer
}

/// Who is working in `root`, as `node (pid 4242)`, in pid order.
pub fn working_in(root: &Path) -> Vec<String> {
    let mut found: Vec<(u32, String)> = working(&[root.to_path_buf()])
        .into_iter()
        .map(|(_, pid, program)| (pid, program))
        .collect();
    found.sort();
    found.dedup();
    found
        .into_iter()
        .map(|(pid, program)| format!("{program} (pid {pid})"))
        .collect()
}

/// Every process working in one of `roots`: which root, its pid, its name.
fn working(roots: &[PathBuf]) -> Vec<(usize, u32, String)> {
    #[cfg(target_os = "linux")]
    {
        let spellings: Vec<Vec<PathBuf>> = roots
            .iter()
            .map(|root| vec![crate::util::paths::canonical(root).unwrap_or_else(|_| root.clone())])
            .collect();
        let pass = linux::read(&spellings, &[], linux::Want::WorkingFolders);
        pass.seen
            .into_iter()
            .flat_map(|seen| {
                seen.facts
                    .into_iter()
                    .filter(|fact| fact.kind == judge::Kind::Cwd)
                    .map(move |fact| (fact.folder, seen.pid, seen.program.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = roots;
        Vec::new()
    }
}
