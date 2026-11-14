//! systemd unit files that name a folder: a service whose working folder or
//! command is in the project, a timer that starts one later, a mount, a
//! container a generator turned into a unit — whether or not it runs now.
//!
//! Text, read from the folders the managers load units from, and one
//! `systemctl show` per manager for the state of those that match. A unit
//! file that is a link into the folder (`systemctl --user link`) names it
//! too.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use super::Tie;
use super::judge::{self, UnitFound, Units};

/// How long `systemctl show` is given before it is killed and its units
/// counted as they would be with no answer.
const SHOW_WITHIN: Duration = Duration::from_secs(2);

/// A unit file worth reading: a file this large is no unit.
const LARGEST: u64 = 256 * 1024;

/// The kinds of unit a manager loads.
const UNIT_KINDS: [&str; 11] = [
    "service",
    "socket",
    "timer",
    "mount",
    "automount",
    "path",
    "target",
    "slice",
    "scope",
    "swap",
    "device",
];

/// Characters a path may follow in a unit file and still be one: a value
/// starts after `=`, a quote, or one of systemd's prefixes (`-` ignore
/// failure, `@` argv0, `+ !` privileges, `:` no specifiers); `;`, `,` and
/// `(` separate list items.
const BEFORE: &[char] = &['=', '"', '\'', '-', '@', '+', '!', ':', ';', ',', '('];

/// Characters that end a path: a folder inside it goes on with `/`.
const AFTER: &[char] = &['/', '"', '\'', ':', ';', ',', ')'];

/// The folders a manager loads unit files from, `(folder, system)`, in the
/// order it reads them (`systemd-analyze [--user] unit-paths`): a unit's
/// own file in an earlier folder hides one in a later folder, and a drop-in
/// one of the same name. Generators write into the `generator` folders:
/// Podman's Quadlet turns a `.container` into a service, fstab lines become
/// mounts.
pub(super) fn unit_dirs() -> Vec<(PathBuf, bool)> {
    let home = crate::util::paths::home_dir();
    let env_or = |key: &str, fallback: &str| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.as_ref().map(|home| home.join(fallback)))
    };
    let run = std::fs::metadata("/proc/self").ok().map(|me| {
        use std::os::unix::fs::MetadataExt;
        PathBuf::from(format!("/run/user/{}/systemd", me.uid()))
    });
    let generated = |kind: &str| run.as_ref().map(|run| run.join(kind));
    let user = [
        generated("generator.early"),
        env_or("XDG_CONFIG_HOME", ".config").map(|dir| dir.join("systemd/user")),
        Some(PathBuf::from("/etc/systemd/user")),
        generated("generator"),
        env_or("XDG_DATA_HOME", ".local/share").map(|dir| dir.join("systemd/user")),
        generated("generator.late"),
    ];
    let system = [
        "/run/systemd/generator.early",
        "/etc/systemd/system",
        "/run/systemd/generator",
        "/run/systemd/generator.late",
    ];
    user.into_iter()
        .flatten()
        .map(|dir| (dir, false))
        .chain(system.into_iter().map(|dir| (PathBuf::from(dir), true)))
        .collect()
}

/// Whether a file name is a unit's.
fn is_unit(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, kind)| !stem.is_empty() && UNIT_KINDS.contains(&kind))
}

/// What a file of a unit folder is to its unit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Part {
    /// The unit's own file.
    Own,
    /// A drop-in, by its name in `<unit>.d/`.
    DropIn(OsString),
    /// A `<unit>.d` folder that is a link: listed when the files are read,
    /// under their deadline, since what it leads to may not answer.
    LinkedDropIns,
}

/// One file of a unit folder.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct UnitFile {
    /// The unit it is for.
    pub unit: String,
    pub part: Part,
    pub path: PathBuf,
}

/// What reading one [`UnitFile`] found: what ties each file it held to the
/// folders — the file itself, or each drop-in a linked folder holds — by
/// drop-in name (`None` for a unit's own file).
type Ties = Vec<(Option<OsString>, Vec<(usize, Tie)>)>;

/// Every unit file naming one of `folders`, with what systemd says of it,
/// and those that did not answer. Only what a manager loads is read: a
/// unit's own file from the first folder holding one, each drop-in name from
/// the first folder holding it; and every file is checked against every
/// folder.
pub(super) fn find(folders: &[Vec<PathBuf>]) -> Units {
    let mut mains: HashSet<(String, bool)> = HashSet::new();
    let mut drop_ins: HashSet<(String, bool, OsString)> = HashSet::new();
    let mut loaded: Vec<(UnitFile, bool)> = Vec::new();
    for (dir, system) in unit_dirs() {
        for file in unit_files(&dir) {
            let first = match &file.part {
                Part::Own => mains.insert((file.unit.clone(), system)),
                Part::DropIn(name) => drop_ins.insert((file.unit.clone(), system, name.clone())),
                Part::LinkedDropIns => true,
            };
            if first {
                loaded.push((file, system));
            }
        }
    }
    let reads = read_all(folders, &loaded);
    // What a linked folder held is known only now: a drop-in of a name read
    // from an earlier folder hides it.
    let mut named: HashSet<(String, bool, OsString)> = HashSet::new();
    let mut found: Vec<(usize, String, bool, Tie)> = Vec::new();
    let mut silent = Vec::new();
    for ((file, system), read) in loaded.iter().zip(reads) {
        let Some(read) = read else {
            silent.push((file.unit.clone(), *system, file.path.clone()));
            continue;
        };
        for (name, ties) in read {
            if let Some(name) = name
                && !named.insert((file.unit.clone(), *system, name))
            {
                continue;
            }
            for (folder, tie) in ties {
                if !found.iter().any(|(kept, unit, manager, _)| {
                    *kept == folder && *unit == file.unit && manager == system
                }) {
                    found.push((folder, file.unit.clone(), *system, tie));
                }
            }
        }
    }
    Units {
        found: with_states(found),
        silent,
    }
}

/// Every file of `loaded` read under one deadline, each on a thread of its
/// own ([`crate::util::paths::answer_within`]): a unit file may be a link
/// onto a mount that stopped answering, and opening it would block for the
/// kernel's own timeout — inside a move's data lock. `None` for a file that
/// did not answer; one given up on answers `None` at once until it is back.
fn read_all(folders: &[Vec<PathBuf>], loaded: &[(UnitFile, bool)]) -> Vec<Option<Ties>> {
    let home = crate::util::paths::home_dir();
    let needles: [Vec<Vec<String>>; 2] = [false, true].map(|system| {
        folders
            .iter()
            .map(|spellings| needles(spellings, (!system).then_some(home.as_deref()).flatten()))
            .collect()
    });
    let parts: HashMap<PathBuf, (bool, Part)> = loaded
        .iter()
        .map(|(file, system)| (file.path.clone(), (*system, file.part.clone())))
        .collect();
    let paths: Vec<PathBuf> = loaded.iter().map(|(file, _)| file.path.clone()).collect();
    let folders = folders.to_vec();
    let read = move |path: &Path| -> Ties {
        crate::util::paths::stall_if_marked(path);
        let Some((system, part)) = parts.get(path) else {
            return Vec::new();
        };
        let needles = &needles[usize::from(*system)];
        match part {
            Part::Own => vec![(None, tie_of(path, &folders, needles))],
            Part::DropIn(name) => vec![(Some(name.clone()), tie_of(path, &folders, needles))],
            Part::LinkedDropIns => drop_ins_in(path)
                .into_iter()
                .map(|(name, file)| (Some(name), tie_of(&file, &folders, needles)))
                .collect(),
        }
    };
    crate::util::paths::answer_within(&paths, crate::util::paths::PROBE_TIMEOUT, read)
}

/// `found` with what systemd says of each unit.
fn with_states(found: Vec<(usize, String, bool, Tie)>) -> Vec<UnitFound> {
    if found.is_empty() {
        return Vec::new();
    }
    let mut states = HashMap::new();
    for system in [false, true] {
        let units: Vec<String> = found
            .iter()
            .filter(|(.., kept, _)| *kept == system)
            .map(|(_, name, ..)| name.clone())
            .collect();
        if units.is_empty() {
            continue;
        }
        for (name, shown) in show(&units, system)
            .map(|text| parse_show(&text))
            .unwrap_or_default()
        {
            states.insert((name, system), shown);
        }
    }
    found
        .into_iter()
        .map(|(folder, name, system, tie)| {
            let (state, dormant) = state_of(states.get(&(name.clone(), system)), system);
            UnitFound {
                folder,
                name,
                system,
                tie,
                state,
                dormant,
            }
        })
        .collect()
}

/// The unit files in `dir`: a unit's own file, and the drop-ins in
/// `<unit>.d/` — that folder itself when it is a link, which is listed only
/// under the deadline. The `.wants` and `.requires` folders only hold links
/// to units listed already.
pub(super) fn unit_files(dir: &Path) -> Vec<UnitFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if let Some(unit) = name.strip_suffix(".d") {
            if !is_unit(unit) {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
                files.push(UnitFile {
                    unit: unit.to_string(),
                    part: Part::LinkedDropIns,
                    path,
                });
                continue;
            }
            for (drop_name, drop_in) in drop_ins_in(&path) {
                files.push(UnitFile {
                    unit: unit.to_string(),
                    part: Part::DropIn(drop_name),
                    path: drop_in,
                });
            }
        } else if is_unit(&name) {
            files.push(UnitFile {
                unit: name,
                part: Part::Own,
                path,
            });
        }
    }
    files.sort();
    files
}

/// The drop-ins in a `<unit>.d` folder, by name.
fn drop_ins_in(folder: &Path) -> Vec<(OsString, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut drop_ins: Vec<(OsString, PathBuf)> = entries
        .flatten()
        .map(|entry| (entry.file_name(), entry.path()))
        .filter(|(name, _)| name.to_string_lossy().ends_with(".conf"))
        .collect();
    drop_ins.sort();
    drop_ins
}

/// How a unit file ties itself to a folder: a link into it, else the first
/// line naming it.
pub(super) fn tie_of(
    file: &Path,
    folders: &[Vec<PathBuf>],
    needles: &[Vec<String>],
) -> Vec<(usize, Tie)> {
    if let Ok(target) = std::fs::read_link(file) {
        let target = match file.parent() {
            Some(dir) if target.is_relative() => dir.join(target),
            _ => target,
        };
        let hits = judge::inside(folders, &judge::normalise(&target));
        if !hits.is_empty() {
            return hits
                .into_iter()
                .map(|(folder, rest)| (folder, Tie::Link(rest)))
                .collect();
        }
    }
    let mut text = Vec::new();
    let read =
        std::fs::File::open(file).and_then(|opened| opened.take(LARGEST).read_to_end(&mut text));
    if read.is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&text);
    needles
        .iter()
        .enumerate()
        .filter_map(|(folder, needles)| {
            text_names(&text, needles).map(|line| (folder, Tie::Line(line)))
        })
        .collect()
}

/// What a unit file would spell a folder as: each spelling, and — in a
/// user's unit folder, where `%h` is their home — the `%h/…` form of each
/// spelling under `home`.
pub(super) fn needles(spellings: &[PathBuf], home: Option<&Path>) -> Vec<String> {
    let mut needles: Vec<String> = spellings
        .iter()
        .map(|spelling| spelling.to_string_lossy().into_owned())
        .collect();
    if let Some(home) = home {
        for spelling in spellings {
            if let Ok(rest) = spelling.strip_prefix(home)
                && !rest.as_os_str().is_empty()
            {
                needles.push(format!("%h/{}", rest.to_string_lossy()));
            }
        }
    }
    needles
}

/// The first line of a unit file that names one of `needles`, cut to a
/// hundred characters. Comment lines name nothing.
pub(super) fn text_names(text: &str, needles: &[String]) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#') && !line.starts_with(';'))
        .find(|line| needles.iter().any(|needle| names(line, needle)))
        .map(|line| match line.char_indices().nth(100) {
            Some((cut, _)) => format!("{}…", &line[..cut]),
            None => line.to_string(),
        })
}

/// Whether `needle` stands in `line` as a path of its own: where a value
/// starts before it, and where a path ends or goes deeper after it — so
/// neither `file:///home/…` nor `/mnt/home/…` nor `…/X2` is `…/X`.
pub(super) fn names(line: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    line.match_indices(needle).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + needle.len()..].chars().next();
        before.is_none_or(|c| c.is_whitespace() || BEFORE.contains(&c))
            && after.is_none_or(|c| c.is_whitespace() || AFTER.contains(&c))
    })
}

/// What `systemctl show` said of one unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Shown {
    pub load: String,
    pub file_state: String,
    pub active: String,
    pub sub: String,
    pub triggered_by: String,
}

/// `systemctl show`'s blocks, one per unit, by `Id`.
pub(super) fn parse_show(text: &str) -> HashMap<String, Shown> {
    let mut units = HashMap::new();
    for block in text.split("\n\n") {
        let mut id = None;
        let mut shown = Shown::default();
        for line in block.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_string();
            match key.trim() {
                "Id" => id = Some(value),
                "LoadState" => shown.load = value,
                "UnitFileState" => shown.file_state = value,
                "ActiveState" => shown.active = value,
                "SubState" => shown.sub = value,
                "TriggeredBy" => shown.triggered_by = value,
                _ => {}
            }
        }
        if let Some(id) = id {
            units.insert(id, shown);
        }
    }
    units
}

/// The unit's state in a few words, and whether it is dormant: disabled,
/// stopped and started by nothing. A unit systemd did not answer about, or
/// does not know, is not dormant — it counts.
pub(super) fn state_of(shown: Option<&Shown>, system: bool) -> (String, bool) {
    let prefix = if system { "system, " } else { "" };
    let Some(shown) = shown.filter(|shown| !shown.load.is_empty() && shown.load != "not-found")
    else {
        return (format!("{prefix}state unknown"), false);
    };
    let running = match shown.active.as_str() {
        "active" if !shown.sub.is_empty() => shown.sub.clone(),
        "inactive" => "stopped".to_string(),
        other => other.to_string(),
    };
    let mut words: Vec<String> = Vec::new();
    if !shown.file_state.is_empty() {
        words.push(shown.file_state.clone());
    }
    words.push(running);
    if !shown.triggered_by.is_empty() {
        words.push(format!("started by {}", shown.triggered_by));
    }
    let dormant = shown.file_state == "disabled"
        && shown.active == "inactive"
        && shown.triggered_by.is_empty();
    (format!("{prefix}{}", words.join(", ")), dormant)
}

/// `systemctl show` for `units`, of the user's manager or the system's;
/// `None` when there is no answer within [`SHOW_WITHIN`], and then the
/// command is killed.
fn show(units: &[String], system: bool) -> Option<String> {
    let systemctl = crate::util::paths::find_on_path("systemctl")?;
    let mut command = Command::new(systemctl);
    if !system {
        command.arg("--user");
    }
    command
        .args([
            "show",
            "--no-pager",
            "--property=Id,LoadState,UnitFileState,ActiveState,SubState,TriggeredBy",
            "--",
        ])
        .args(units)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (answer, answered) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = answer.send(text);
    });
    let text = answered.recv_timeout(SHOW_WITHIN).ok();
    if text.is_none() {
        let _ = child.kill();
    }
    // Waited for on a thread of its own: no zombie, and nobody waits for a
    // command taking its time to exit.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    text.filter(|text| !text.trim().is_empty())
}
