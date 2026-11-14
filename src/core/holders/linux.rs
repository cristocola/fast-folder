//! The Linux pass: every process in `/proc`, read on a few threads under one
//! deadline, keeping only what touches the folders asked about.
//!
//! **Nothing here waits on a process.** Reading another process's `cmdline`
//! or `maps` takes its memory lock, which a process stuck in a page fault on
//! a dead mount holds; the read then hangs with it. So every process is read
//! on a worker, the pass takes what answered by [`DEADLINE`], and a process
//! still being read is named as not answering — and not asked again by a
//! later pass until its read comes back ([`GIVEN_UP`]), so a stuck process
//! costs one parked worker, not one per pass. fastf itself is recognised by
//! the text of its executable's link, never by asking the file it names.

use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
/// out `asking`, fastf's own processes and this one.
pub(super) fn read(folders: &[Vec<PathBuf>], asking: &[Asking], want: Want) -> Pass {
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
    });
    let reader = Arc::clone(&context);
    let (seen, silent) = within(pids, DEADLINE, move |pid| read_one(&reader, pid));
    let foreign = lock(&context.tables)
        .iter()
        .map(|(_, _, table)| Arc::clone(table))
        .collect();
    Pass {
        seen,
        // The desk asks for names only; a process that did not say is none.
        silent: if want == Want::Everything {
            silent
        } else {
            Vec::new()
        },
        host,
        foreign,
    }
}

/// Run `work` over `pids`, [`WIDTH`] at a time on threads of their own, and
/// take what answered by `deadline`: the answers, and the pids that did not
/// answer — those a pass gave up on earlier included, without asking them
/// again. The work left over goes on behind and is dropped as it lands.
pub(super) fn within<R: Send + 'static>(
    pids: Vec<u32>,
    deadline: Duration,
    work: impl Fn(u32) -> Option<R> + Send + Sync + 'static,
) -> (Vec<R>, Vec<u32>) {
    let given_up = lock(&GIVEN_UP).clone();
    let (mut silent, asked): (Vec<u32>, Vec<u32>) =
        pids.into_iter().partition(|pid| given_up.contains(pid));
    let (tx, rx) = mpsc::channel::<(u32, Option<R>)>();
    let stop = Arc::new(AtomicBool::new(false));
    let in_flight: Arc<Mutex<HashSet<u32>>> = Arc::new(Mutex::new(HashSet::new()));
    let (worker_stop, worker_flight, queue) =
        (Arc::clone(&stop), Arc::clone(&in_flight), asked.clone());
    let spawned = std::thread::Builder::new()
        .name("fastf-holders".to_string())
        .spawn(move || {
            let _ = crate::util::pool::run(WIDTH, queue, |pid| {
                if worker_stop.load(Ordering::Relaxed) {
                    return Ok::<(), ()>(());
                }
                lock(&worker_flight).insert(pid);
                let answer = work(pid);
                lock(&worker_flight).remove(&pid);
                lock(&GIVEN_UP).remove(&pid);
                let _ = tx.send((pid, answer));
                Ok(())
            });
        });
    if spawned.is_err() {
        // No thread to wait on: nothing is read rather than read without a
        // deadline.
        silent.extend(asked);
        silent.sort_unstable();
        return (Vec::new(), silent);
    }
    let end = Instant::now() + deadline;
    let mut waiting: HashSet<u32> = asked.into_iter().collect();
    let mut answers = Vec::new();
    while !waiting.is_empty() {
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok((pid, answer)) => {
                waiting.remove(&pid);
                answers.extend(answer);
            }
            Err(_) => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
    {
        // Under the in-flight lock, so a read that finishes now either is
        // still listed here and clears itself from `GIVEN_UP` after, or is
        // not listed and never put there.
        let flight = lock(&in_flight);
        let mut given_up = lock(&GIVEN_UP);
        given_up.extend(flight.iter().filter(|pid| waiting.contains(pid)));
    }
    while let Ok((pid, answer)) = rx.try_recv() {
        if waiting.remove(&pid) {
            answers.extend(answer);
        }
    }
    silent.extend(waiting);
    silent.sort_unstable();
    (answers, silent)
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
        program: name_of(&base, &argv, exe.as_deref()),
        ns: foreign.map(|(index, _)| index),
        facts,
    })
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
