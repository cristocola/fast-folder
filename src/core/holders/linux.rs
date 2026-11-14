//! The Linux pass: every process in `/proc`, read on a few threads under one
//! deadline, keeping only what touches the folders asked about.
//!
//! **Nothing here waits on a process.** Reading another process's `cmdline`
//! or `maps` takes its memory lock, which a process stuck in a page fault on
//! a dead mount holds; the read then hangs with it. So every process is read
//! on a worker, a worker stuck on one has another started beside it, the
//! pass takes what answered by [`DEADLINE`], and a process still being read
//! is named as not answering — and not asked again by a later pass until its
//! read comes back ([`GIVEN_UP`]), so a stuck process costs one parked
//! worker, not one per pass. A process no worker reached by the deadline is
//! named as never asked, apart from those that did not answer. fastf itself
//! is recognised by the text of its executable's link, never by asking the
//! file it names.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use super::Asking;
use super::judge::{self, Fact, Kind, Pass, Seen, Table};
use crate::util::fs_kind::{MountLine, mount_lines};

/// How long a pass waits for every process to answer. A pass over about a
/// thousand processes takes a quarter of a second.
const DEADLINE: Duration = Duration::from_secs(5);

/// Processes read at once.
const WIDTH: usize = 8;

/// How long a worker is on one process before another is started beside it.
/// A process's files are read in a millisecond or two; one this slow is
/// stuck behind a mount.
const STUCK_AFTER: Duration = Duration::from_millis(250);

/// How often the pass looks for workers that are stuck.
const TICK: Duration = Duration::from_millis(100);

/// Workers started beside stuck ones in one pass, at most: a mount that
/// stopped answering under more processes than this leaves the rest unread,
/// and named as such.
const MORE: usize = 64;

/// Shells, where `/etc/shells` cannot say.
const SHELLS: [&str; 9] = [
    "sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "tcsh", "csh",
];

/// Processes a pass gave up on while they were being read: until that read
/// comes back, a pass names them as not answering without asking again.
static GIVEN_UP: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

/// What a pass reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Want {
    /// Everything that ties a process to a folder, and every mount table.
    Everything,
    /// Working folders only, and the names of the processes working there:
    /// what the desk shows, read every few seconds.
    WorkingFolders,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// What every worker reads with.
struct Context {
    folders: Vec<Vec<PathBuf>>,
    asking: Vec<Asking>,
    want: Want,
    self_exe: Option<OsString>,
    self_ns: Option<OsString>,
    host_text: Vec<u8>,
    host: Vec<MountLine>,
    /// The foreign mount tables found so far, each once: its text's hash,
    /// its text, the table.
    tables: Mutex<Vec<(u64, Vec<u8>, Table)>>,
    shells: HashSet<String>,
    /// The parent of every shell read so far.
    shell_parents: Mutex<HashSet<u32>>,
}

impl Context {
    /// The index of the foreign table `text` is, adding it the first time.
    fn table_for(&self, text: Vec<u8>) -> (usize, Table) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        let mut tables = lock(&self.tables);
        if let Some(index) = tables
            .iter()
            .position(|(known, kept, _)| *known == hash && *kept == text)
        {
            return (index, Arc::clone(&tables[index].2));
        }
        let table = Arc::new(mount_lines(&String::from_utf8_lossy(&text)));
        tables.push((hash, text, Arc::clone(&table)));
        (tables.len() - 1, table)
    }
}

/// One pass over every process for `folders` (each its spellings), leaving
/// out `asking`, fastf's own processes and this one. `shells` are the
/// programs whose parents the pass names ([`Pass::shell_parents`]).
pub(super) fn read(
    folders: &[Vec<PathBuf>],
    asking: &[Asking],
    want: Want,
    shells: &HashSet<String>,
) -> Pass {
    let host_text = std::fs::read("/proc/self/mountinfo").unwrap_or_default();
    let host = mount_lines(&String::from_utf8_lossy(&host_text));
    let me = std::process::id();
    let pids: Vec<u32> = std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
                .filter(|pid| *pid != me)
                .collect()
        })
        .unwrap_or_default();
    let context = Arc::new(Context {
        folders: folders.to_vec(),
        asking: asking.to_vec(),
        want,
        self_exe: std::fs::read_link("/proc/self/exe")
            .ok()
            .map(PathBuf::into_os_string),
        self_ns: std::fs::read_link("/proc/self/ns/mnt")
            .ok()
            .map(PathBuf::into_os_string),
        host_text,
        host: host.clone(),
        tables: Mutex::new(Vec::new()),
        shells: shells.clone(),
        shell_parents: Mutex::new(HashSet::new()),
    });
    let reader = Arc::clone(&context);
    let looked = within(pids, DEADLINE, move |pid| read_one(&reader, pid));
    let foreign = lock(&context.tables)
        .iter()
        .map(|(_, _, table)| Arc::clone(table))
        .collect();
    let shell_parents = lock(&context.shell_parents).clone();
    // The desk asks for names only; a process that did not say is none.
    let names_only = want == Want::WorkingFolders;
    Pass {
        seen: looked.answers,
        silent: if names_only {
            Vec::new()
        } else {
            looked.silent
        },
        unasked: if names_only {
            Vec::new()
        } else {
            looked.unasked
        },
        shell_parents,
        host,
        foreign,
    }
}

/// What [`within`] found.
#[derive(Debug)]
pub(super) struct Looked<R> {
    pub answers: Vec<R>,
    /// Asked, with no answer by the deadline — or given up on by an earlier
    /// pass and not back yet.
    pub silent: Vec<u32>,
    /// Never asked: every worker was on a process that did not answer.
    pub unasked: Vec<u32>,
}

/// The pids waiting to be read, and what the workers are doing.
struct Queue {
    waiting: VecDeque<u32>,
    /// Each pid a worker is reading now, and since when.
    reading: HashMap<u32, Instant>,
    /// Every pid a worker took.
    taken: HashSet<u32>,
    /// Workers started and not finished.
    alive: usize,
    /// The deadline passed: nothing more is taken.
    stop: bool,
}

/// Run `work` over `pids`, [`WIDTH`] at a time on threads of their own, and
/// take what answered by `deadline`: the answers, the pids that did not
/// answer — those a pass gave up on earlier included, without asking them
/// again — and the pids never asked. A worker on one pid for
/// [`STUCK_AFTER`] has another started beside it, [`MORE`] at most, so a few
/// reads that do not come back leave the rest to be read. The work left
/// over goes on behind and is dropped as it lands.
pub(super) fn within<R: Send + 'static>(
    pids: Vec<u32>,
    deadline: Duration,
    work: impl Fn(u32) -> Option<R> + Send + Sync + 'static,
) -> Looked<R> {
    let given_up = lock(&GIVEN_UP).clone();
    let (mut silent, asked): (Vec<u32>, Vec<u32>) =
        pids.into_iter().partition(|pid| given_up.contains(pid));
    let queue = Arc::new(Mutex::new(Queue {
        waiting: asked.iter().copied().collect(),
        reading: HashMap::new(),
        taken: HashSet::new(),
        alive: 0,
        stop: false,
    }));
    let work = Arc::new(work);
    let (tx, rx) = mpsc::channel::<(u32, Option<R>)>();
    // A system out of threads reads with the ones it gave; with none, the
    // pids are never asked, rather than read without a deadline.
    for _ in 0..WIDTH.min(asked.len()) {
        if !start(&queue, &work, &tx) {
            break;
        }
    }
    let end = Instant::now() + deadline;
    let mut waiting: HashSet<u32> = asked.into_iter().collect();
    let mut answers = Vec::new();
    let mut more = 0;
    let mut next_look = Instant::now() + TICK;
    while !waiting.is_empty() {
        let now = Instant::now();
        if now >= end {
            break;
        }
        if let Ok((pid, answer)) = rx.recv_timeout(end.min(next_look) - now)
            && waiting.remove(&pid)
        {
            answers.extend(answer);
        }
        if Instant::now() >= next_look {
            next_look = Instant::now() + TICK;
            more += beside_the_stuck(&queue, &work, &tx, MORE - more);
        }
    }
    let taken = {
        // Under the queue's lock, so a read that finishes now either is
        // still listed here and clears itself from `GIVEN_UP` after, or is
        // not listed and never put there.
        let mut queue = lock(&queue);
        queue.stop = true;
        let mut given_up = lock(&GIVEN_UP);
        given_up.extend(queue.reading.keys().filter(|pid| waiting.contains(pid)));
        std::mem::take(&mut queue.taken)
    };
    while let Ok((pid, answer)) = rx.try_recv() {
        if waiting.remove(&pid) {
            answers.extend(answer);
        }
    }
    let (late, mut unasked): (Vec<u32>, Vec<u32>) =
        waiting.into_iter().partition(|pid| taken.contains(pid));
    silent.extend(late);
    silent.sort_unstable();
    unasked.sort_unstable();
    Looked {
        answers,
        silent,
        unasked,
    }
}

/// Start one worker on `queue`; whether the system gave it a thread.
fn start<R, W>(
    queue: &Arc<Mutex<Queue>>,
    work: &Arc<W>,
    tx: &mpsc::Sender<(u32, Option<R>)>,
) -> bool
where
    R: Send + 'static,
    W: Fn(u32) -> Option<R> + Send + Sync + 'static,
{
    lock(queue).alive += 1;
    let (shared, work, tx) = (Arc::clone(queue), Arc::clone(work), tx.clone());
    let spawned = std::thread::Builder::new()
        .name("fastf-holders".to_string())
        .spawn(move || worker(&shared, &*work, &tx));
    if spawned.is_err() {
        lock(queue).alive -= 1;
    }
    spawned.is_ok()
}

/// One worker: a pid at a time off the queue, until it is empty or the
/// deadline has passed.
fn worker<R>(
    queue: &Mutex<Queue>,
    work: &impl Fn(u32) -> Option<R>,
    tx: &mpsc::Sender<(u32, Option<R>)>,
) {
    loop {
        let pid = {
            let mut queue = lock(queue);
            let next = if queue.stop {
                None
            } else {
                queue.waiting.pop_front()
            };
            let Some(pid) = next else {
                queue.alive -= 1;
                return;
            };
            queue.taken.insert(pid);
            queue.reading.insert(pid, Instant::now());
            pid
        };
        let answer = work(pid);
        lock(queue).reading.remove(&pid);
        lock(&GIVEN_UP).remove(&pid);
        let _ = tx.send((pid, answer));
    }
}

/// While pids wait, start a worker beside each one that has been on a pid
/// for [`STUCK_AFTER`], `room` at most; how many it started.
fn beside_the_stuck<R, W>(
    queue: &Arc<Mutex<Queue>>,
    work: &Arc<W>,
    tx: &mpsc::Sender<(u32, Option<R>)>,
    room: usize,
) -> usize
where
    R: Send + 'static,
    W: Fn(u32) -> Option<R> + Send + Sync + 'static,
{
    let wanted = {
        let queue = lock(queue);
        let stuck = queue
            .reading
            .values()
            .filter(|since| since.elapsed() >= STUCK_AFTER)
            .count();
        WIDTH
            .saturating_sub(queue.alive.saturating_sub(stuck))
            .min(queue.waiting.len())
            .min(room)
    };
    let mut started = 0;
    while started < wanted && start(queue, work, tx) {
        started += 1;
    }
    started
}

fn trim_deleted(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_suffix(" (deleted)") {
        Some(kept) => PathBuf::from(kept),
        None => path.to_path_buf(),
    }
}

/// One process, as far as it touches a folder; `None` for one that is gone,
/// left out, or touches nothing and lives on the host.
fn read_one(context: &Context, pid: u32) -> Option<Seen> {
    let base = PathBuf::from(format!("/proc/{pid}"));
    let stat = crate::util::process::stat(pid)?;
    if stat.zombie
        || context
            .asking
            .iter()
            .any(|asking| asking.is(pid, stat.started))
    {
        return None;
    }
    // fastf by the link's text alone: asking the file it names would ask a
    // filesystem that may not answer.
    let exe = std::fs::read_link(base.join("exe")).ok();
    if let Some(exe) = &exe
        && (Some(exe.as_os_str()) == context.self_exe.as_deref()
            || exe
                .file_name()
                .is_some_and(crate::util::process::is_fastf_name))
    {
        return None;
    }
    let cwd = std::fs::read_link(base.join("cwd")).ok();
    if context.want == Want::WorkingFolders {
        let hits = cwd
            .as_deref()
            .map(|cwd| judge::inside(&context.folders, cwd))
            .unwrap_or_default();
        if hits.is_empty() {
            return None;
        }
        let argv = judge::argv(&std::fs::read(base.join("cmdline")).unwrap_or_default());
        return Some(Seen {
            pid,
            started: stat.started,
            program: name_of(&base, &argv, exe.as_deref()),
            ns: None,
            opaque: false,
            facts: hits
                .into_iter()
                .map(|(folder, path)| Fact {
                    folder,
                    kind: Kind::Cwd,
                    path,
                })
                .collect(),
        });
    }

    // Which mount namespace: this one when its link says so, or when its
    // table reads the same — another user's process lets nobody read the
    // link, but anybody read its table.
    let same_ns = std::fs::read_link(base.join("ns/mnt"))
        .ok()
        .is_some_and(|ns| Some(ns.as_os_str()) == context.self_ns.as_deref());
    let foreign = if same_ns {
        None
    } else {
        std::fs::read(base.join("mountinfo"))
            .ok()
            .filter(|text| *text != context.host_text)
            .map(|text| context.table_for(text))
    };
    let table = foreign.as_ref().map(|(_, table)| table.as_slice());
    let spell = |path: &Path| -> Vec<(usize, PathBuf)> {
        judge::to_host(&context.host, table, path)
            .iter()
            .flat_map(|spelled| judge::inside(&context.folders, spelled))
            .collect()
    };

    let cmdline = std::fs::read(base.join("cmdline")).unwrap_or_default();
    if cmdline.is_empty() && foreign.is_none() {
        // A kernel thread, or a process on its way out.
        return None;
    }
    let argv = judge::argv(&cmdline);
    let name = name_of(&base, &argv, exe.as_deref());
    // Wherever the shell works, its parent hosts one.
    if context.shells.contains(&name) {
        lock(&context.shell_parents).insert(stat.ppid);
    }
    let mut facts = Vec::new();
    let mut add = |hits: Vec<(usize, PathBuf)>, kind: Kind| {
        for (folder, path) in hits {
            facts.push(Fact { folder, kind, path });
        }
    };
    let cwd_hits = cwd.as_deref().map(spell).unwrap_or_default();
    let cwd_inside = !cwd_hits.is_empty();
    add(cwd_hits, Kind::Cwd);
    let exe_path = exe.as_deref().map(trim_deleted);
    if let Some(exe) = &exe_path {
        add(spell(exe), Kind::Exe);
    }
    if let Some(program) = judge::started_from(&argv, cwd.as_deref())
        && Some(&program) != exe_path.as_ref()
    {
        add(spell(&program), Kind::Exe);
    }
    if let Ok(descriptors) = std::fs::read_dir(base.join("fd")) {
        for descriptor in descriptors.flatten() {
            let Ok(target) = std::fs::read_link(descriptor.path()) else {
                continue;
            };
            if !target.is_absolute() {
                continue;
            }
            let hits = spell(&trim_deleted(&target));
            if hits.is_empty() {
                continue;
            }
            let info = base.join("fdinfo").join(descriptor.file_name());
            let writing = opened_for_writing(&std::fs::read_to_string(info).unwrap_or_default());
            add(hits, Kind::Open { writing });
        }
    }
    if let Ok(maps) = std::fs::read_to_string(base.join("maps")) {
        let mut mapped = HashSet::new();
        for line in maps.lines() {
            // The fields before the path hold no `/`.
            let Some(at) = line.find('/') else {
                continue;
            };
            let path = trim_deleted(Path::new(&line[at..]));
            if Some(&path) == exe_path.as_ref() || !mapped.insert(path.clone()) {
                continue;
            }
            add(spell(&path), Kind::Loaded);
        }
    }
    for path in judge::named_paths(&argv, cwd.as_deref(), cwd_inside) {
        add(spell(&path), Kind::Arg);
    }
    if facts.is_empty() && foreign.is_none() {
        return None;
    }
    Some(Seen {
        pid,
        started: stat.started,
        program: name,
        ns: foreign.map(|(index, _)| index),
        opaque: cwd.is_none(),
        facts,
    })
}

/// The pipes a process holds as its standard input, output and error, by
/// the text of their links (`pipe:[4242]`).
fn stdio_pipes(pid: u32) -> HashSet<String> {
    let fd = PathBuf::from(format!("/proc/{pid}/fd"));
    (0..3)
        .filter_map(|n| std::fs::read_link(fd.join(n.to_string())).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .filter(|target| target.starts_with("pipe:["))
        .collect()
}

/// The processes this one's own pipes join it to ([`judge::pipeline`]),
/// each with when it started. `chain` is this process and those above it.
/// Reading a link under `fd` asks nothing of the process or of a mount.
pub(super) fn pipeline_partners(chain: &[Asking]) -> Vec<Asking> {
    let me = std::process::id();
    let mine = stdio_pipes(me);
    if mine.is_empty() {
        return Vec::new();
    }
    let above: HashSet<u32> = chain
        .iter()
        .map(|asking| asking.pid)
        .filter(|pid| *pid != me)
        .collect();
    let inherited: HashSet<String> = above.iter().flat_map(|pid| stdio_pipes(*pid)).collect();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let others: Vec<(u32, u32, HashSet<String>, u64)> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| *pid != me && !above.contains(pid))
        .filter_map(|pid| {
            let pipes = stdio_pipes(pid);
            if pipes.is_empty() {
                return None;
            }
            let stat = crate::util::process::stat(pid)?;
            Some((pid, stat.ppid, pipes, stat.started))
        })
        .collect();
    let candidates: Vec<(u32, u32, HashSet<String>)> = others
        .iter()
        .map(|(pid, parent, pipes, _)| (*pid, *parent, pipes.clone()))
        .collect();
    judge::pipeline(&mine, &inherited, &above, &candidates)
        .into_iter()
        .filter_map(|pid| {
            others
                .iter()
                .find(|(other, ..)| *other == pid)
                .map(|(_, _, _, started)| Asking {
                    pid,
                    started: *started,
                })
        })
        .collect()
}

/// The process's name ([`judge::program_name`]), reading `comm` only when
/// nothing else says.
fn name_of(base: &Path, argv: &[String], exe: Option<&Path>) -> String {
    let comm = (argv.is_empty() && exe.is_none())
        .then(|| std::fs::read_to_string(base.join("comm")).ok())
        .flatten();
    judge::program_name(argv, exe, comm.as_deref())
}

/// Whether `fdinfo`'s `flags:` (octal) say the file was opened to write.
pub(super) fn opened_for_writing(fdinfo: &str) -> bool {
    fdinfo
        .lines()
        .find_map(|line| line.strip_prefix("flags:"))
        .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
        .is_some_and(|flags| flags & (libc::O_ACCMODE as u32) != libc::O_RDONLY as u32)
}

/// The shells: the names `/etc/shells` lists, and the user's own; a short
/// list of the usual ones where that file is missing.
pub(super) fn shells() -> HashSet<String> {
    let mut shells: HashSet<String> = match std::fs::read_to_string("/etc/shells") {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| Path::new(line).file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect(),
        Err(_) => SHELLS.iter().map(|shell| shell.to_string()).collect(),
    };
    if let Some(shell) = std::env::var_os("SHELL")
        && let Some(name) = Path::new(&shell).file_name()
    {
        shells.insert(name.to_string_lossy().into_owned());
    }
    shells
}
