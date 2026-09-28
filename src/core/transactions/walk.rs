//! The walk: every entry of a tree, and beside it every problem it met.

use super::*;

/// Everything a walk of a tree found: what a manifest can hold, and what it
/// cannot.
///
/// **A walk never stops at an odd entry.** The scan used to: the first link,
/// socket or unreadable name ended it with that one name, so a tree with three
/// problems took three attempts to learn about, and an entry the filesystem
/// lists but cannot examine read as a bare `No such file or directory`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Walk {
    /// Sorted by path.
    pub entries: Vec<ManifestEntry>,
    /// Sorted by path.
    pub problems: Vec<WalkProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkProblem {
    /// Relative to the walked root.
    pub path: PathBuf,
    pub problem: Problem,
}

/// Something a walk found that a manifest cannot record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The folder lists it, and `lstat` cannot examine it. A network mount
    /// that resolves links on the server (sshfs `follow_symlinks`) shows a
    /// dangling link exactly like this.
    Unexaminable { error: String, not_found: bool },
    /// A folder whose entries cannot be listed.
    Unreadable(String),
    /// A link fastf cannot make again, and why: on Windows, a reparse point
    /// of a kind other than a symbolic link or a junction.
    UnsupportedLink(String),
    /// A link whose target the filesystem will not hand over. sshfs does this
    /// by default (`contain_symlinks`) for every link that is absolute or
    /// climbs with `..` — which is every link in `node_modules/.bin`.
    LinkNotReadable(String),
    /// A socket, FIFO or device node.
    Special,
    /// A folder on a different filesystem from the walked root: a mount, or a
    /// separate btrfs subvolume. Walking into it copies another filesystem,
    /// and removing the tree would delete through it.
    OtherFilesystem,
    /// A name that is not valid Unicode, which a JSON manifest cannot hold.
    NotUnicode,
    /// Deeper than [`crate::util::paths::MAX_WALK_DEPTH`].
    TooDeep,
}

impl Problem {
    /// The short form, for "a link now, was a 312-byte file".
    pub(super) fn what(&self) -> String {
        match self {
            Self::Unexaminable { error, .. } => format!("listed but not examinable ({error})"),
            Self::Unreadable(error) => format!("a folder that cannot be listed ({error})"),
            Self::UnsupportedLink(what) => what.clone(),
            Self::LinkNotReadable(error) => format!("a link that cannot be read ({error})"),
            Self::Special => "a socket, pipe or device".to_string(),
            Self::OtherFilesystem => "a different filesystem".to_string(),
            Self::NotUnicode => "a name that is not valid Unicode".to_string(),
            Self::TooDeep => "too deep to walk".to_string(),
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unexaminable {
                error,
                not_found: true,
            } => write!(
                f,
                "listed by the filesystem, but it cannot be examined ({error}); \
                 a network mount that resolves links itself hides a dangling link this way"
            ),
            Self::Unexaminable { error, .. } => write!(
                f,
                "listed by the filesystem, but it cannot be examined ({error})"
            ),
            Self::Unreadable(error) => write!(f, "its contents cannot be listed ({error})"),
            Self::UnsupportedLink(what) => write!(f, "{what}"),
            Self::LinkNotReadable(error) => write!(
                f,
                "a link the filesystem will not let fastf read ({error}); sshfs refuses \
                 every link that is absolute or climbs with `..` unless it is mounted with \
                 `-o no_contain_symlinks`"
            ),
            Self::Special => {
                f.write_str("a socket, pipe or device, which is not a file fastf can copy")
            }
            Self::OtherFilesystem => f.write_str(
                "a different filesystem inside the project (a mount, or a separate btrfs subvolume)",
            ),
            Self::NotUnicode => {
                f.write_str("its name is not valid Unicode, which the move record cannot hold")
            }
            Self::TooDeep => write!(
                f,
                "the tree is too deep here (more than {} levels)",
                crate::util::paths::MAX_WALK_DEPTH
            ),
        }
    }
}

impl Walk {
    /// Walk `root`, which must be a real directory; `label` names it when it
    /// is not.
    pub fn of(root: &Path, label: &str) -> Result<Self> {
        Self::of_with(root, label, Ticker::none())
    }

    /// [`Self::of`], ticking once per entry examined; a ticker that has been
    /// cancelled stops the walk with [`crate::core::progress::CANCELLED`].
    ///
    /// **On the pool** (`util::pool`), as wide as the filesystem is worth:
    /// folders are listed and their entries examined by several workers at
    /// once, and the result is sorted at the end, so it is exactly what one
    /// thread walking depth-first would find.
    pub fn of_with(root: &Path, label: &str, ticker: Ticker) -> Result<Self> {
        // Counted by what it walks: a move's walks of each tree are few by
        // design (`a_staged_move_walks_each_tree_as_few_times_as_it_can`).
        crate::util::trace::hit(&format!("walk {label}"));
        crate::util::paths::require_real_directory(root, label)?;
        let device = RootDevice::of(root)
            .with_context(|| format!("reading metadata for {}", root.display()))?;
        let found = Mutex::new(Self::default());
        let walker = Walker {
            root,
            device,
            ticker,
            found: &found,
        };
        crate::util::pool::expand(
            crate::util::pool::width_for(root),
            vec![Step::List(root.to_path_buf(), 0)],
            |step, queue| match step {
                Step::List(dir, depth) => walker.list(&dir, depth, queue),
                Step::Examine(paths, depth) => walker.examine(paths, depth, queue),
            },
        )?;
        let mut walk = found
            .into_inner()
            .unwrap_or_else(|error| error.into_inner());
        walk.entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        walk.problems
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(walk)
    }

    /// The entries, or a refusal naming every problem (the first
    /// [`LISTED`]).
    pub(super) fn into_entries(self, root: &Path) -> Result<Vec<ManifestEntry>> {
        if self.problems.is_empty() {
            return Ok(self.entries);
        }
        let count = self.problems.len();
        let mut message = format!(
            "{} holds {count} {} that cannot be copied to another drive:",
            crate::util::paths::display_path(root),
            if count == 1 { "entry" } else { "entries" }
        );
        for problem in self.problems.iter().take(LISTED) {
            message.push_str(&format!(
                "\n  {}: {}",
                problem.path.display(),
                problem.problem
            ));
        }
        if count > LISTED {
            message.push_str(&format!("\n  and {} more", count - LISTED));
        }
        bail!("{message}")
    }
}

/// The device a folder lives on, where the platform says. Only folders are
/// compared: on overlayfs a file reports the device of the layer it came from,
/// so comparing files would refuse every tree inside a container.
#[cfg(unix)]
pub(crate) fn device_of(metadata: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(not(unix))]
pub(crate) fn device_of(_metadata: &fs::Metadata) -> Option<u64> {
    None
}

/// **The filesystem a tree's root is on, as a walk and a removal ask it.**
/// A folder on another device is another filesystem mounted inside the tree,
/// and is not walked into — unless the root moved with it: a FUSE mount that
/// dropped and came back (sshfs, rclone) is a new device for every entry
/// under it, the root's own included. So a folder that disagrees is asked of
/// the root again, and if the root agrees with it now, that is the same
/// filesystem remounted, not another. Found by the lab: an sshfs killed
/// mid-removal came back as a new device, and the removal kept 1352 entries
/// "on another filesystem".
pub(crate) struct RootDevice<'r> {
    root: &'r Path,
    device: Mutex<Option<u64>>,
    now: fn(&Path) -> Option<u64>,
}

impl<'r> RootDevice<'r> {
    /// The root's device as it is now.
    pub(crate) fn of(root: &'r Path) -> std::io::Result<Self> {
        let device = device_of(&fs::symlink_metadata(root)?);
        Ok(Self::recorded(root, device, current_device))
    }

    /// With the device already read, and how to read it again.
    pub(crate) fn recorded(
        root: &'r Path,
        device: Option<u64>,
        now: fn(&Path) -> Option<u64>,
    ) -> Self {
        Self {
            root,
            device: Mutex::new(device),
            now,
        }
    }

    /// Whether a folder with `metadata` is on another filesystem than the
    /// root is on now.
    pub(crate) fn elsewhere(&self, metadata: &fs::Metadata) -> bool {
        self.elsewhere_than(device_of(metadata))
    }

    pub(super) fn elsewhere_than(&self, folder: Option<u64>) -> bool {
        let mut known = self
            .device
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (Some(root), Some(folder)) = (*known, folder) else {
            return false;
        };
        if root == folder {
            return false;
        }
        match (self.now)(self.root) {
            Some(now) if now == folder => {
                *known = Some(now);
                false
            }
            _ => true,
        }
    }
}

/// A path's device as it is now; `None` when it cannot be read.
pub(crate) fn current_device(path: &Path) -> Option<u64> {
    fs::symlink_metadata(path)
        .ok()
        .and_then(|metadata| device_of(&metadata))
}

/// One piece of a walk's work.
enum Step {
    /// A folder to list, at its depth (the root's is 0).
    List(PathBuf, usize),
    /// Entries one listing found, examined together; the depth is their
    /// folder's.
    Examine(Vec<PathBuf>, usize),
}

/// How many entries of one folder one worker examines at a time: enough that
/// the queue is never what a walk waits on, few enough that a flat folder of
/// twenty thousand files still spreads over every worker.
const EXAMINE_BATCH: usize = 32;

/// What every worker of one walk shares.
struct Walker<'w> {
    root: &'w Path,
    /// The root's device: a folder on another is not walked into.
    device: RootDevice<'w>,
    ticker: Ticker<'w>,
    found: &'w Mutex<Walk>,
}

impl Walker<'_> {
    fn keep(&self, local: Walk) {
        if local.entries.is_empty() && local.problems.is_empty() {
            return;
        }
        let mut found = self.found.lock().unwrap_or_else(|error| error.into_inner());
        found.entries.extend(local.entries);
        found.problems.extend(local.problems);
    }

    fn problem(&self, path: PathBuf, problem: Problem) {
        self.keep(Walk {
            entries: Vec::new(),
            problems: vec![WalkProblem { path, problem }],
        });
    }

    /// List `dir` and queue what it holds for examining. **Only the root is
    /// at depth 0**; a listing that fails there fails the walk, anywhere else
    /// it is that folder's problem.
    fn list(&self, dir: &Path, depth: usize, queue: &Queue<'_, Step>) -> Result<()> {
        let here = relative_to(self.root, dir)?;
        if depth >= crate::util::paths::MAX_WALK_DEPTH {
            self.problem(here, Problem::TooDeep);
            return Ok(());
        }
        // Whole or not at all, asked again when a mount fails part of the
        // way: a listing that stopped half-way cannot say what it did not
        // list, so a folder that never lists whole is a problem.
        let listing =
            crate::util::fs_retry::list_dir(dir, || crate::util::faults::check_io("walk:readdir"));
        let children = match listing {
            Ok(children) => children,
            Err(error) if depth == 0 => {
                return Err(error).with_context(|| format!("reading {}", dir.display()));
            }
            Err(error) => {
                self.problem(here, Problem::Unreadable(error.to_string()));
                return Ok(());
            }
        };
        for batch in children.chunks(EXAMINE_BATCH) {
            queue.push(Step::Examine(batch.to_vec(), depth));
        }
        Ok(())
    }

    /// Examine entries of one folder at `depth`: record each, and queue each
    /// folder among them for listing.
    fn examine(&self, paths: Vec<PathBuf>, depth: usize, queue: &Queue<'_, Step>) -> Result<()> {
        let mut local = Walk::default();
        for path in paths {
            let relative = relative_to(self.root, &path)?;
            if !self.ticker.tick(&relative) {
                bail!("{}", crate::core::progress::CANCELLED);
            }
            crate::util::paths::require_native_relative(&relative, "move manifest path")?;
            let mut problem = |problem| {
                local.problems.push(WalkProblem {
                    path: relative.clone(),
                    problem,
                })
            };
            if relative.to_str().is_none() {
                problem(Problem::NotUnicode);
                continue;
            }
            let looked = crate::util::fs_retry::with_retry(&path, || {
                crate::util::faults::check_io("walk:lstat")?;
                fs::symlink_metadata(&path)
            });
            let metadata = match looked {
                Ok(metadata) => metadata,
                Err(error) => {
                    problem(Problem::Unexaminable {
                        not_found: error.kind() == std::io::ErrorKind::NotFound,
                        error: error.to_string(),
                    });
                    continue;
                }
            };
            match entry_for(&path, &relative, &metadata) {
                Err(found) => problem(found),
                Ok(entry) if entry.kind == ManifestKind::Directory => {
                    if self.device.elsewhere(&metadata) {
                        problem(Problem::OtherFilesystem);
                        continue;
                    }
                    local.entries.push(entry);
                    queue.push(Step::List(path, depth + 1));
                }
                Ok(entry) => local.entries.push(entry),
            }
        }
        self.keep(local);
        Ok(())
    }
}

/// The walk as one thread does it, depth-first: the reference the pool's walk
/// is held to (`the_parallel_walk_finds_what_one_thread_finds`).
#[cfg(all(test, unix))]
pub(super) fn walk_at(
    root: &Path,
    current: &Path,
    depth: usize,
    device: Option<u64>,
    walk: &mut Walk,
) -> Result<()> {
    let here = relative_to(root, current)?;
    if depth >= crate::util::paths::MAX_WALK_DEPTH {
        walk.problems.push(WalkProblem {
            path: here,
            problem: Problem::TooDeep,
        });
        return Ok(());
    }
    let children = match fs::read_dir(current) {
        Ok(children) => children,
        Err(error) if depth == 0 => {
            return Err(error).with_context(|| format!("reading {}", current.display()));
        }
        Err(error) => {
            walk.problems.push(WalkProblem {
                path: here,
                problem: Problem::Unreadable(error.to_string()),
            });
            return Ok(());
        }
    };
    for child in children {
        let child = match child {
            Ok(child) => child,
            Err(error) if depth == 0 => {
                return Err(error).with_context(|| format!("reading {}", current.display()));
            }
            Err(error) => {
                walk.problems.push(WalkProblem {
                    path: here,
                    problem: Problem::Unreadable(error.to_string()),
                });
                return Ok(());
            }
        };
        let path = child.path();
        let relative = relative_to(root, &path)?;
        crate::util::paths::require_native_relative(&relative, "move manifest path")?;
        let mut problem = |problem| {
            walk.problems.push(WalkProblem {
                path: relative.clone(),
                problem,
            })
        };
        if relative.to_str().is_none() {
            problem(Problem::NotUnicode);
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                problem(Problem::Unexaminable {
                    not_found: error.kind() == std::io::ErrorKind::NotFound,
                    error: error.to_string(),
                });
                continue;
            }
        };
        match entry_for(&path, &relative, &metadata) {
            Err(found) => problem(found),
            Ok(entry) if entry.kind == ManifestKind::Directory => {
                if device.is_some() && device_of(&metadata) != device {
                    problem(Problem::OtherFilesystem);
                    continue;
                }
                walk.entries.push(entry);
                walk_at(root, &path, depth + 1, device, walk)?;
            }
            Ok(entry) => walk.entries.push(entry),
        }
    }
    Ok(())
}

/// **The one classification of an entry**, from its `lstat`: what a manifest
/// records for it, or why it cannot. The walk and the removal of a retired
/// folder both ask here, so what one records the other recognises.
fn entry_for(
    path: &Path,
    relative: &Path,
    metadata: &fs::Metadata,
) -> std::result::Result<ManifestEntry, Problem> {
    let file_type = metadata.file_type();
    let modified = metadata
        .modified()
        .map(ModifiedTime::from_system_time)
        .map_err(|error| Problem::Unexaminable {
            not_found: false,
            error: error.to_string(),
        })?;
    if file_type.is_symlink() {
        // **A link is content: recorded by its target text, never followed.**
        // It is what `mv` does and what the same-filesystem rename already
        // did; a dangling link is as good as any, since nothing is read
        // through it.
        let kind = link_kind(path, metadata)?;
        let target = fs::read_link(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                Problem::LinkNotReadable(error.to_string())
            } else {
                Problem::Unexaminable {
                    not_found: error.kind() == std::io::ErrorKind::NotFound,
                    error: error.to_string(),
                }
            }
        })?;
        if target.to_str().is_none() {
            return Err(Problem::NotUnicode);
        }
        if target.as_os_str().is_empty() {
            return Err(Problem::UnsupportedLink(
                "a link with an empty target".to_string(),
            ));
        }
        return Ok(ManifestEntry {
            path: relative.to_path_buf(),
            kind,
            bytes: 0,
            source_modified: modified,
            link_target: Some(target),
        });
    }
    let (kind, bytes) = if file_type.is_dir() {
        (ManifestKind::Directory, 0)
    } else if file_type.is_file() {
        (ManifestKind::File, metadata.len())
    } else {
        return Err(Problem::Special);
    };
    Ok(ManifestEntry {
        path: relative.to_path_buf(),
        kind,
        bytes,
        source_modified: modified,
        link_target: None,
    })
}

/// Which kind of link `path` is. Every symbolic link on unix; on Windows the
/// reparse tag decides, because a junction and a directory symbolic link look
/// alike to `std` and are made differently — and a mounted volume or a WSL
/// link look like links too.
#[cfg(not(windows))]
fn link_kind(_path: &Path, _metadata: &fs::Metadata) -> std::result::Result<ManifestKind, Problem> {
    Ok(ManifestKind::Symlink)
}

#[cfg(windows)]
fn link_kind(path: &Path, metadata: &fs::Metadata) -> std::result::Result<ManifestKind, Problem> {
    use crate::util::win_reparse::{IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK};
    use std::os::windows::fs::FileTypeExt;
    let tag =
        crate::util::win_reparse::reparse_tag(path).map_err(|error| Problem::Unexaminable {
            not_found: error.kind() == std::io::ErrorKind::NotFound,
            error: error.to_string(),
        })?;
    match tag {
        IO_REPARSE_TAG_SYMLINK if metadata.file_type().is_symlink_dir() => {
            Ok(ManifestKind::DirSymlink)
        }
        IO_REPARSE_TAG_SYMLINK => Ok(ManifestKind::Symlink),
        IO_REPARSE_TAG_MOUNT_POINT => {
            // A mount point naming a volume by GUID is a whole other drive
            // mounted inside the project, not a link to a folder.
            let target = fs::read_link(path).unwrap_or_default();
            if target
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(r"\\?\volume{")
            {
                Err(Problem::OtherFilesystem)
            } else {
                Ok(ManifestKind::Junction)
            }
        }
        other => Err(Problem::UnsupportedLink(format!(
            "a Windows link of a kind fastf cannot make again (reparse tag {other:#010x})"
        ))),
    }
}

/// What is at `path` now, as a walk would record it; `None` for what a
/// manifest cannot hold.
pub(crate) fn examine(path: &Path, relative: &Path) -> std::io::Result<Option<ManifestEntry>> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(entry_for(path, relative, &metadata).ok())
}

/// What a look at `path` already taken (`metadata`) records it as; `None`
/// for what a manifest cannot hold.
pub(crate) fn entry_of(
    path: &Path,
    relative: &Path,
    metadata: &fs::Metadata,
) -> Option<ManifestEntry> {
    entry_for(path, relative, metadata).ok()
}

/// Whether `found` is still the entry `recorded` describes. Folder times do
/// not count: removing a child moves them.
pub(crate) fn agrees(recorded: &ManifestEntry, found: &ManifestEntry) -> bool {
    difference(recorded, found, Match::Whole).is_none()
}

fn relative_to(root: &Path, path: &Path) -> Result<PathBuf> {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .with_context(|| format!("deriving relative path for {}", path.display()))
}
