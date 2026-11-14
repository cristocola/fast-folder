//! From what the Linux pass read to what uses each folder: pure, so every
//! rule is a fixture test.
//!
//! The reader keeps only facts that touch a folder, every path already
//! spelled as this machine's mount table spells it (`to_host`); what is
//! decided here is which fact a process is named for, which mounts count,
//! and which units.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::{By, How, Tie, Use};
use crate::util::fs_kind::{MountLine, covering, host_paths};

/// A mount table, shared by every process that has it.
pub(super) type Table = Arc<Vec<MountLine>>;

/// What a namespace's mounts are named after: its oldest process's pid and
/// name.
pub(super) type Oldest = (u32, String);

/// What a process has in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// Its working folder.
    Cwd,
    /// Its executable.
    Exe,
    /// A file it has open.
    Open { writing: bool },
    /// A file it has mapped: a library, a database.
    Loaded,
    /// A path among its arguments.
    Arg,
}

impl Kind {
    /// The most telling first: what it runs, what it writes, what it holds,
    /// what it was started with, and last where it merely is.
    fn rank(self) -> u8 {
        match self {
            Kind::Exe => 0,
            Kind::Open { writing: true } => 1,
            Kind::Open { writing: false } => 2,
            Kind::Loaded => 3,
            Kind::Arg => 4,
            Kind::Cwd => 5,
        }
    }
}

/// One fact about a process: which folder, how, and the path inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Fact {
    pub folder: usize,
    pub kind: Kind,
    /// Relative to the folder.
    pub path: PathBuf,
}

/// A process the pass kept: one with a fact, or one living in a foreign
/// mount namespace, which the namespace's mounts are named after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub pid: u32,
    pub started: u64,
    pub program: String,
    /// The foreign namespace it lives in, by its index in [`Pass::foreign`].
    pub ns: Option<usize>,
    pub facts: Vec<Fact>,
}

/// Everything one pass read.
#[derive(Debug, Clone, Default)]
pub(super) struct Pass {
    pub seen: Vec<Seen>,
    /// Processes that did not answer by the deadline.
    pub silent: Vec<u32>,
    /// Processes never asked: the ones before them did not answer.
    pub unasked: Vec<u32>,
    /// Every process with a shell among its children, wherever that shell
    /// works: a terminal, a multiplexer, a script runner.
    pub shell_parents: HashSet<u32>,
    /// This process's mount table.
    pub host: Vec<MountLine>,
    /// Every other table some process has, once each.
    pub foreign: Vec<Table>,
}

/// A unit file that names a folder, and what systemd says of the unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UnitFound {
    pub folder: usize,
    pub name: String,
    pub system: bool,
    pub tie: Tie,
    pub state: String,
    pub dormant: bool,
}

/// Where `path` lies inside any of `folders`' spellings: each folder it is
/// in, with the rest of the path.
pub(super) fn inside(folders: &[Vec<PathBuf>], path: &Path) -> Vec<(usize, PathBuf)> {
    folders
        .iter()
        .enumerate()
        .filter_map(|(index, spellings)| {
            spellings.iter().find_map(|spelling| {
                path.strip_prefix(spelling)
                    .ok()
                    .map(|rest| (index, rest.to_path_buf()))
            })
        })
        .collect()
}

/// `path` as a process in `table`'s namespace reports it, spelled through
/// this machine's table `host`: the mount that holds it there, then that
/// mount's folder found again here. `table` is `None` for a process on the
/// host, whose paths need nothing.
pub(super) fn to_host(
    host: &[MountLine],
    table: Option<&[MountLine]>,
    path: &Path,
) -> Vec<PathBuf> {
    let Some(table) = table else {
        return vec![path.to_path_buf()];
    };
    let Some((line, rest)) = covering(table, path) else {
        return Vec::new();
    };
    let in_fs = if rest.as_os_str().is_empty() {
        line.root.clone()
    } else {
        line.root.join(rest)
    };
    host_paths(host, &line.dev, &in_fs)
}

/// `path` with `.` and `..` taken out, as text: no filesystem is asked.
pub(super) fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A process's arguments from its `cmdline`: the NUL-separated fields, or —
/// for a program that rewrote its command line into one string with spaces
/// (`nginx: master process /usr/bin/nginx -c …`) — that string's words.
pub(super) fn argv(cmdline: &[u8]) -> Vec<String> {
    let mut fields: Vec<String> = cmdline
        .split(|byte| *byte == 0)
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect();
    while fields.last().is_some_and(String::is_empty) {
        fields.pop();
    }
    if let [one] = fields.as_slice()
        && one.contains(' ')
    {
        return one.split_whitespace().map(str::to_string).collect();
    }
    fields
}

/// The name a person knows a program by: its first argument's file name
/// (`claude`, where the executable is `2.1.323`), without a login shell's
/// `-` or a rewritten title's `:`; else its executable's name; else the
/// kernel's `comm`.
pub(super) fn program_name(argv: &[String], exe: Option<&Path>, comm: Option<&str>) -> String {
    let from_argv = argv.first().and_then(|first| {
        let first = first.trim_end_matches(':').trim_start_matches('-');
        let name = Path::new(first).file_name()?.to_string_lossy().into_owned();
        (!name.is_empty()).then_some(name)
    });
    from_argv
        .or_else(|| {
            exe.and_then(Path::file_name).map(|name| {
                name.to_string_lossy()
                    .trim_end_matches(" (deleted)")
                    .to_string()
            })
        })
        .or_else(|| {
            comm.map(str::trim)
                .filter(|comm| !comm.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "a program".to_string())
}

/// The program a process was started as, when it was started by a path
/// (`/p/.venv/bin/python`, `./serve`): a relative one is taken from the
/// working folder. Its executable link may say less — a virtualenv's
/// `python` is a link out of the folder, so `exe` names the system's.
pub(super) fn started_from(argv: &[String], cwd: Option<&Path>) -> Option<PathBuf> {
    let first = Path::new(argv.first()?);
    if !first.as_os_str().to_string_lossy().contains('/') {
        return None;
    }
    if first.is_absolute() {
        return Some(normalise(first));
    }
    cwd.map(|cwd| normalise(&cwd.join(first)))
}

/// The processes joined to the asker by its own pipes: those holding, as
/// their standard input, output or error, a pipe the asker holds as one of
/// its own, and so on along the pipeline (`fastf move … | grep x | tail`).
/// The shell started them with the command, so they work in its folder only
/// because it does. Each is a child of a process above the asker (`above`)
/// or of one already joined — never of the asker, whose own children inherit
/// its pipes without being its pipeline. A pipe a process above the asker
/// also holds (`inherited`) joins nothing: it is a whole session's output,
/// which a job started in the background shares too. `others` is each other
/// process with its parent and its pipes.
pub(super) fn pipeline(
    mine: &HashSet<String>,
    inherited: &HashSet<String>,
    above: &HashSet<u32>,
    others: &[(u32, u32, HashSet<String>)],
) -> Vec<u32> {
    let mut pipes: HashSet<&String> = mine.difference(inherited).collect();
    let mut joined: Vec<u32> = Vec::new();
    loop {
        let before = joined.len();
        for (pid, parent, theirs) in others {
            if joined.contains(pid)
                || !(above.contains(parent) || joined.contains(parent))
                || !theirs.iter().any(|pipe| pipes.contains(pipe))
            {
                continue;
            }
            joined.push(*pid);
            pipes.extend(theirs.iter().filter(|pipe| !inherited.contains(*pipe)));
        }
        if joined.len() == before {
            return joined;
        }
    }
}

/// The paths a process's arguments name: each argument after the program,
/// and its pieces between `=`, `,` and `:` (`--config=/p/c.toml`,
/// `-v /p/data:/app/data`), the pieces first. A relative one is taken from
/// the working folder, but only when that is outside every folder asked
/// about — inside one, the working folder already ties the process there,
/// and every word of `npm run dev` would read as a path in it.
pub(super) fn named_paths(argv: &[String], cwd: Option<&Path>, cwd_inside: bool) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for arg in argv.iter().skip(1) {
        let pieces = arg
            .split(['=', ',', ':'])
            .filter(|piece| !piece.is_empty() && *piece != arg.as_str());
        for candidate in pieces.chain(std::iter::once(arg.as_str())) {
            let path = Path::new(candidate);
            if path.is_absolute() {
                paths.push(normalise(path));
            } else if !cwd_inside && let Some(cwd) = cwd {
                paths.push(normalise(&cwd.join(path)));
            }
        }
    }
    paths
}

impl Fact {
    /// An argument naming the folder itself, not a path inside it.
    fn names_the_folder(&self) -> bool {
        self.kind == Kind::Arg && self.path.as_os_str().is_empty()
    }
}

/// Each process's most telling fact in each folder, as a use; among
/// arguments, one naming a path inside the folder before the folder itself.
/// A shell whose fact there is its working folder is named as one: its
/// prompt is the only tie, and whatever it started is judged itself. A
/// process whose only tie is the folder itself among its arguments was
/// opened on it — a file manager, a viewer — and holds nothing of it. A
/// process that is no shell, `hosts_shell` (a shell among its children),
/// tied only by its working folder and the folder itself among its
/// arguments, is that shell's terminal: its children are judged themselves.
pub(super) fn process_uses(
    seen: &Seen,
    shells: &HashSet<String>,
    hosts_shell: bool,
) -> Vec<(usize, Use)> {
    let shell = shells.contains(&seen.program);
    let mut folders: Vec<usize> = Vec::new();
    for fact in &seen.facts {
        if !folders.contains(&fact.folder) {
            folders.push(fact.folder);
        }
    }
    folders
        .into_iter()
        .filter_map(|folder| {
            let facts: Vec<&Fact> = seen
                .facts
                .iter()
                .filter(|fact| fact.folder == folder)
                .collect();
            let working = facts.iter().find(|fact| fact.kind == Kind::Cwd);
            let how = if facts.iter().all(|fact| fact.names_the_folder()) {
                How::OpenedOn
            } else if let Some(working) = working
                && hosts_shell
                && !shell
                && facts
                    .iter()
                    .all(|fact| fact.kind == Kind::Cwd || fact.names_the_folder())
            {
                How::Terminal(working.path.clone())
            } else {
                let best = facts
                    .iter()
                    .min_by_key(|fact| (fact.kind.rank(), fact.names_the_folder()))?;
                let path = best.path.clone();
                match best.kind {
                    Kind::Cwd => How::WorkingIn { at: path, shell },
                    Kind::Exe => How::Runs(path),
                    Kind::Open { writing } => How::Open { at: path, writing },
                    Kind::Loaded => How::Loaded(path),
                    Kind::Arg => How::Named(path),
                }
            };
            Some((
                folder,
                Use {
                    by: By::Process {
                        pid: seen.pid,
                        program: seen.program.clone(),
                    },
                    how,
                },
            ))
        })
        .collect()
}

/// What the mount tables say about each folder.
///
/// - **Another filesystem mounted inside it**, from this machine's table.
/// - **A bind on this machine**: a mount whose folder, found through another
///   mount of the same filesystem, is inside it, mounted somewhere outside.
/// - **A foreign namespace's mount** — a container's, a sandbox's — of a
///   folder from inside it, named after the namespace's oldest process. Only
///   a line this machine's table does not have, the same filesystem and
///   folder: a namespace starts as a copy of the one it came from, and a
///   mount it merely inherited is no choice of its own.
pub(super) fn mount_uses(
    folders: &[Vec<PathBuf>],
    host: &[MountLine],
    foreign: &[(Table, Option<Oldest>)],
) -> Vec<(usize, Use)> {
    let mut uses = Vec::new();
    for (index, line) in host.iter().enumerate() {
        for (folder, rest) in inside(folders, &line.point) {
            if !rest.as_os_str().is_empty() {
                uses.push((
                    folder,
                    Use {
                        by: By::Nobody,
                        how: How::MountedInside(rest),
                    },
                ));
            }
        }
        if !inside(folders, &line.point).is_empty() {
            continue;
        }
        let elsewhere = host
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .flat_map(|(_, other)| host_paths(std::slice::from_ref(other), &line.dev, &line.root));
        for spelled in elsewhere {
            for (folder, rest) in inside(folders, &spelled) {
                push_once(
                    &mut uses,
                    folder,
                    Use {
                        by: By::Nobody,
                        how: How::Mounts {
                            folder: rest,
                            at: line.point.clone(),
                        },
                    },
                );
            }
        }
    }
    let known: HashSet<(&str, &Path)> = host
        .iter()
        .map(|line| (line.dev.as_str(), line.root.as_path()))
        .collect();
    for (table, oldest) in foreign {
        let Some((pid, program)) = oldest else {
            continue;
        };
        for line in table.iter() {
            if known.contains(&(line.dev.as_str(), line.root.as_path())) {
                continue;
            }
            for spelled in host_paths(host, &line.dev, &line.root) {
                for (folder, rest) in inside(folders, &spelled) {
                    push_once(
                        &mut uses,
                        folder,
                        Use {
                            by: By::Process {
                                pid: *pid,
                                program: program.clone(),
                            },
                            how: How::Mounts {
                                folder: rest,
                                at: line.point.clone(),
                            },
                        },
                    );
                }
            }
        }
    }
    uses
}

fn push_once(uses: &mut Vec<(usize, Use)>, folder: usize, found: Use) {
    if !uses
        .iter()
        .any(|(kept, use_)| *kept == folder && *use_ == found)
    {
        uses.push((folder, found));
    }
}

/// Everything using each folder: the processes in pid order, then the
/// mounts, the unit files, the processes that did not answer, and those
/// never asked.
pub(super) fn judge(
    folders: &[Vec<PathBuf>],
    pass: &Pass,
    shells: &HashSet<String>,
    units: &[UnitFound],
) -> Vec<Vec<Use>> {
    let mut answer: Vec<Vec<Use>> = vec![Vec::new(); folders.len()];
    let mut seen: Vec<&Seen> = pass.seen.iter().collect();
    seen.sort_by_key(|seen| seen.pid);
    for process in &seen {
        let hosts_shell = pass.shell_parents.contains(&process.pid);
        for (folder, found) in process_uses(process, shells, hosts_shell) {
            answer[folder].push(found);
        }
    }
    // A namespace is named after its oldest process: a container's
    // entrypoint, a sandbox's first program.
    let foreign: Vec<(Table, Option<Oldest>)> = pass
        .foreign
        .iter()
        .enumerate()
        .map(|(index, table)| {
            let oldest = seen
                .iter()
                .filter(|seen| seen.ns == Some(index))
                .min_by_key(|seen| (seen.started, seen.pid))
                .map(|seen| (seen.pid, seen.program.clone()));
            (Arc::clone(table), oldest)
        })
        .collect();
    for (folder, found) in mount_uses(folders, &pass.host, &foreign) {
        answer[folder].push(found);
    }
    for unit in units {
        if let Some(uses) = answer.get_mut(unit.folder) {
            uses.push(Use {
                by: By::Unit {
                    name: unit.name.clone(),
                    system: unit.system,
                },
                how: How::Unit {
                    tie: unit.tie.clone(),
                    state: unit.state.clone(),
                    dormant: unit.dormant,
                },
            });
        }
    }
    for (pids, how) in [
        (&pass.silent, How::Silent as fn(Vec<u32>) -> How),
        (&pass.unasked, How::Unasked),
    ] {
        if pids.is_empty() {
            continue;
        }
        for uses in answer.iter_mut() {
            uses.push(Use {
                by: By::Nobody,
                how: how(pids.clone()),
            });
        }
    }
    answer
}
