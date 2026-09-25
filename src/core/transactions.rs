//! Scoped move transactions.
//!
//! A transaction lives below the target base at
//! `.fastf-transactions/<operation-id>/`.  The journal deliberately contains
//! no target-base or staging path: both are derived from that owned location.
//! Source and target folder names are validated single path components before
//! they are ever joined to a base. So is where a source goes when it leaves
//! the library: `<source base>/.fastf-moved-<operation-id>`, named by nothing
//! but the operation.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::assets::Progress;

pub const TRANSACTIONS_DIR: &str = ".fastf-transactions";
pub const JOURNAL_FILE: &str = "move.json";
pub(crate) const MANIFEST_FILE: &str = "manifest.json";
pub const STAGING_DIR: &str = "staging";
/// The destination as it was published: the walk of the verified staging tree,
/// times included, so a later cleanup can tell the user's edits from a copy
/// restored out of an older backup.
pub(crate) const PUBLISHED_FILE: &str = "published.json";
/// A source that has left the library, beside it in its base, until it is
/// removed. Dot-prefixed, so discovery never lists it.
pub const RETIRED_PREFIX: &str = ".fastf-moved-";

/// The journal this build writes. Version 3 added the `Retired` phase, the
/// operation and the host; a version-2 journal — what 3.11 and older wrote —
/// is still read, and is rewritten as version 3 by its first phase change,
/// before anything is renamed. An older binary refuses version 3, which is
/// what keeps it from finishing a transaction whose source it cannot see.
const MOVE_VERSION: u32 = 3;
const MOVE_VERSION_OLDEST: u32 = 2;
/// The manifest this build writes. Version 2 added the link kinds; a version-1
/// manifest — what 3.11 and older wrote — is version 2 without them, and is
/// still read, or a transaction an older binary left could never be finished.
const MANIFEST_VERSION: u32 = 2;
const MANIFEST_VERSION_OLDEST: u32 = 1;
const OPERATION_RETRIES: usize = 64;
const COPY_BUFFER_BYTES: usize = 1024 * 1024;
/// How many paths a refusal or a difference names before "and N more".
pub(crate) const LISTED: usize = 10;

static OPERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovePhase {
    Copying,
    ReadyToCommit,
    /// Published; the source is still at its path until it is retired.
    CleanupPending,
    /// The source has been renamed out of the library to its retired name;
    /// what is left is removing that. Written *after* the rename, so a crash
    /// between the two leaves `CleanupPending` with the retired folder present,
    /// which recovery reads the same way.
    Retired,
}

/// What the transaction is for. A copy keeps its source, so recovery must never
/// take a copy's transaction as licence to remove one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    /// Version-2 journals carry no operation. Only moves ever advanced past
    /// `Copying` then, so reading them as moves is what they were.
    #[default]
    Move,
    Copy,
}

/// The complete move journal schema. Unknown fields are rejected so a future
/// journal is never accidentally interpreted with older recovery semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveJournal {
    pub version: u32,
    pub operation_id: String,
    pub project_id: String,
    pub source_base: PathBuf,
    pub source_folder: PathBuf,
    pub target_folder: PathBuf,
    pub phase: MovePhase,
    #[serde(default)]
    pub operation: Operation,
    /// The machine that began it. A base can be reached from more than one
    /// machine, and the source path a journal names means something else on
    /// the other one; recovery there only reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Begun by a build that removed a source where it stood (3.11 and
    /// older), so its source may already be partly removed. Set when a
    /// version-2 journal is read and **carried** when it is rewritten as
    /// version 3 — otherwise a retire that failed after the rewrite would
    /// leave a half-removed source that the next pass, seeing version 3,
    /// demands whole.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_cleanup: bool,
}

impl MoveJournal {
    /// Whether the source may be a residue of an in-place removal.
    pub fn source_may_be_partial(&self) -> bool {
        self.legacy_cleanup
    }

    /// Whether this machine began the operation. A version-2 journal names
    /// no machine and is taken as this one's, which is how 3.11 treated it.
    pub fn is_from_this_host(&self) -> bool {
        match (&self.host, this_host()) {
            (Some(recorded), Some(here)) => *recorded == here,
            _ => true,
        }
    }
}

/// This machine's name, as the journal records it.
pub(crate) fn this_host() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buffer = [0_u8; 256];
        // SAFETY: the buffer is valid for its whole length, and the length
        // passed leaves room for the terminating NUL.
        let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len() - 1) };
        if status != 0 {
            return None;
        }
        let end = buffer.iter().position(|&byte| byte == 0)?;
        std::str::from_utf8(&buffer[..end])
            .ok()
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|name| !name.is_empty())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManifestKind {
    File,
    Directory,
    /// A symbolic link, recorded by its target text and never followed: every
    /// link on unix, a link to a file on Windows.
    Symlink,
    /// Windows: a symbolic link that carries the directory attribute. Making
    /// one again needs to know, and its target may not exist to be asked.
    DirSymlink,
    /// Windows: a directory junction.
    Junction,
}

impl ManifestKind {
    pub fn is_link(self) -> bool {
        matches!(self, Self::Symlink | Self::DirSymlink | Self::Junction)
    }
}

/// A lossless filesystem modification timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModifiedTime {
    before_epoch: bool,
    seconds: u64,
    nanoseconds: u32,
}

impl ModifiedTime {
    /// Nanoseconds from the epoch, signed, for ordering.
    pub fn nanos(&self) -> i128 {
        let magnitude = i128::from(self.seconds) * 1_000_000_000 + i128::from(self.nanoseconds);
        if self.before_epoch {
            -magnitude
        } else {
            magnitude
        }
    }

    fn from_system_time(value: SystemTime) -> Self {
        match value.duration_since(UNIX_EPOCH) {
            Ok(duration) => Self {
                before_epoch: false,
                seconds: duration.as_secs(),
                nanoseconds: duration.subsec_nanos(),
            },
            Err(error) => {
                let duration = error.duration();
                Self {
                    before_epoch: true,
                    seconds: duration.as_secs(),
                    nanoseconds: duration.subsec_nanos(),
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    /// Native relative path. It is never converted through a lossy display
    /// string while scanning, comparing, or copying.
    pub path: PathBuf,
    pub kind: ManifestKind,
    pub bytes: u64,
    pub source_modified: ModifiedTime,
    /// A link's target, exactly as the link holds it. Absent for everything
    /// else, and then not written, so a manifest without links reads the way a
    /// version-1 one always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<PathBuf>,
}

impl ManifestEntry {
    /// What this entry is, for a message: "a 312-byte file", "a folder".
    fn describe(&self) -> String {
        match self.kind {
            ManifestKind::File if self.bytes == 1 => "a 1-byte file".to_string(),
            ManifestKind::File => format!("a {}-byte file", self.bytes),
            ManifestKind::Directory => "a folder".to_string(),
            ManifestKind::Symlink | ManifestKind::DirSymlink | ManifestKind::Junction => {
                let noun = if self.kind == ManifestKind::Junction {
                    "a junction"
                } else {
                    "a link"
                };
                match &self.link_target {
                    Some(target) => format!("{noun} to {}", target.display()),
                    None => noun.to_string(),
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveManifest {
    version: u32,
    pub entries: Vec<ManifestEntry>,
}

impl MoveManifest {
    /// Scan exactly once before copying. Unsupported entries fail the entire
    /// move — every one of them named, not only the first — and links and
    /// special files are never followed or silently omitted.
    pub fn scan(root: &Path) -> Result<Self> {
        let entries = Walk::of(root, "move source")?.into_entries(root)?;
        let manifest = Self {
            version: MANIFEST_VERSION,
            entries,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if !(MANIFEST_VERSION_OLDEST..=MANIFEST_VERSION).contains(&self.version) {
            bail!(
                "unsupported move manifest version {} (this build reads {} to {})",
                self.version,
                MANIFEST_VERSION_OLDEST,
                MANIFEST_VERSION
            );
        }
        let mut seen = HashSet::new();
        for entry in &self.entries {
            crate::util::paths::require_native_relative(&entry.path, "move manifest path")?;
            if !seen.insert(entry.path.clone()) {
                bail!(
                    "move manifest contains duplicate path {}",
                    entry.path.display()
                );
            }
            if entry.kind.is_link() {
                if self.version < 2 {
                    bail!(
                        "a version-{} move manifest cannot hold a link: {}",
                        self.version,
                        entry.path.display()
                    );
                }
                if entry
                    .link_target
                    .as_ref()
                    .is_none_or(|target| target.as_os_str().is_empty())
                {
                    bail!("move manifest link has no target: {}", entry.path.display());
                }
            } else if entry.link_target.is_some() {
                bail!(
                    "move manifest entry has a link target but is not a link: {}",
                    entry.path.display()
                );
            }
            if entry.kind != ManifestKind::File && entry.bytes != 0 {
                bail!(
                    "move manifest entry that is not a file has a non-zero byte length: {}",
                    entry.path.display()
                );
            }
        }
        Ok(())
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| entry.kind == ManifestKind::File)
            .map(|entry| entry.bytes)
            .sum()
    }

    pub fn total_files(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.kind == ManifestKind::File)
            .count()
    }

    pub fn total_links(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.kind.is_link())
            .count()
    }

    /// The links whose meaning the new place may change, as sentences. A link
    /// is kept exactly — that is the promise — so one that climbs out of the
    /// project now climbs out somewhere else, and one naming the original
    /// folder by its full path still names it.
    pub fn link_notes(&self, original: &Path) -> Vec<String> {
        let original_shown = crate::util::paths::display_path(original);
        self.entries
            .iter()
            .filter(|entry| entry.kind.is_link())
            .filter_map(|entry| {
                let target = entry.link_target.as_ref()?;
                let into_original = target.starts_with(original)
                    || Path::new(&crate::util::paths::display_path(target))
                        .starts_with(&original_shown);
                if target.is_absolute() || target.has_root() {
                    into_original.then(|| {
                        format!(
                            "{} points into the original folder by its full path ({}); it is \
                             kept exactly, so it still points there, not into the new copy",
                            entry.path.display(),
                            target.display()
                        )
                    })
                } else {
                    climbs_out(&entry.path, target).then(|| {
                        format!(
                            "{} points outside the project ({}); it is kept exactly, so from \
                             the new place it may point somewhere else",
                            entry.path.display(),
                            target.display()
                        )
                    })
                }
            })
            .collect()
    }

    /// Verify exact relative paths, entry types, regular-file lengths and link
    /// targets. Destination modification times are intentionally not compared:
    /// fastf promises content topology and byte lengths, not metadata
    /// preservation.
    ///
    /// Hands back the walk it compared, which is the destination as it is
    /// about to be published.
    pub fn verify_destination(&self, destination: &Path) -> Result<Walk> {
        let walk = Walk::of(destination, "move destination")?;
        let diff = self.compare(&walk, Match::Content);
        if !diff.is_clean() {
            bail!(
                "move verification failed: the copy does not match the source: {}",
                diff.summary(LISTED)
            );
        }
        Ok(walk)
    }

    /// Re-walk the source and compare path, type, length, link target and
    /// modification time against the scan.
    pub fn verify_source_unchanged(&self, source: &Path) -> Result<()> {
        let diff = self.compare(&Walk::of(source, "move source")?, Match::Exact);
        if !diff.is_clean() {
            bail!(
                "the source changed after it was scanned: {}",
                diff.summary(LISTED)
            );
        }
        Ok(())
    }

    /// Used by recovery before deleting a source: compare exact path/type/size
    /// manifests on both sides and also require the source to still match the
    /// original pre-copy metadata snapshot.
    pub fn verify_recovery_pair(&self, source: &Path, destination: &Path) -> Result<()> {
        self.verify_source_unchanged(source)?;
        self.verify_destination(destination).map(drop)
    }

    /// The recorded entry at `path`.
    pub fn entry(&self, path: &Path) -> Option<&ManifestEntry> {
        self.entries
            .binary_search_by(|entry| entry.path.as_path().cmp(path))
            .ok()
            .map(|index| &self.entries[index])
    }

    /// Compare what a walk found against what this manifest recorded.
    ///
    /// **Entries only**: the manifest's `version` is how it was written, not
    /// what it says, and a version-1 manifest compared whole against a fresh
    /// scan would never match — every transaction an older binary left would
    /// be stuck for good.
    pub fn compare(&self, walk: &Walk, rule: Match) -> ManifestDiff {
        let found: HashMap<&Path, &ManifestEntry> = walk
            .entries
            .iter()
            .map(|entry| (entry.path.as_path(), entry))
            .collect();
        let problem_at: HashMap<&Path, &Problem> = walk
            .problems
            .iter()
            .map(|problem| (problem.path.as_path(), &problem.problem))
            .collect();
        let recorded: HashSet<&Path> = self
            .entries
            .iter()
            .map(|entry| entry.path.as_path())
            .collect();

        let mut diff = ManifestDiff {
            recorded: self.entries.len(),
            ..ManifestDiff::default()
        };
        let mut explained = HashSet::new();
        for entry in &self.entries {
            let path = entry.path.as_path();
            if let Some(now) = found.get(path) {
                if let Some(change) = difference(entry, now, rule) {
                    diff.changed.push((entry.path.clone(), change));
                }
            } else if let Some(problem) = problem_at.get(path) {
                // Recorded as one thing, found as something the walk could not
                // take: "a link now, was a 312-byte file" says more than a
                // missing entry and an unrelated problem would.
                explained.insert(path);
                diff.changed.push((
                    entry.path.clone(),
                    format!("{} now, was {}", problem.what(), entry.describe()),
                ));
            } else if !path
                .ancestors()
                .skip(1)
                .any(|ancestor| problem_at.contains_key(ancestor))
            {
                // Beneath a folder the walk could not read, an entry is
                // unknown rather than missing; the folder's problem says so.
                diff.missing.push(entry.path.clone());
            }
        }
        diff.added = walk
            .entries
            .iter()
            .filter(|entry| !recorded.contains(entry.path.as_path()))
            .map(|entry| entry.path.clone())
            .collect();
        diff.problems = walk
            .problems
            .iter()
            .filter(|problem| !explained.contains(problem.path.as_path()))
            .cloned()
            .collect();
        diff
    }
}

/// What a staged copy carried, in words: "1473 files and 3 links, 60.2 MB".
pub fn copied_summary(files: usize, links: usize, bytes: u64) -> String {
    let files = format!("{files} file{}", if files == 1 { "" } else { "s" });
    let links = match links {
        0 => String::new(),
        1 => " and 1 link".to_string(),
        many => format!(" and {many} links"),
    };
    format!(
        "{files}{links}, {}",
        crate::util::human_bytes::human_bytes(bytes)
    )
}

/// Whether a relative link at `link` (relative to the project) resolves, by
/// its text alone, to somewhere above the project.
fn climbs_out(link: &Path, target: &Path) -> bool {
    let mut depth = link
        .parent()
        .map_or(0, |parent| parent.components().count());
    for component in target.components() {
        match component {
            Component::ParentDir if depth == 0 => return true,
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    false
}

/// How an entry a walk found must agree with the one recorded at its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// Everything recorded, folder times included: the source right after the
    /// copy, before anything is published.
    Exact,
    /// Everything but folder times, which removing or renaming a child moves.
    Whole,
    /// Path, kind, byte length and link target: a copy, whose times are its
    /// own.
    Content,
}

/// Why `found` does not agree with `recorded`, or `None` when it does.
fn difference(recorded: &ManifestEntry, found: &ManifestEntry, rule: Match) -> Option<String> {
    if recorded.kind != found.kind {
        return Some(format!(
            "{} now, was {}",
            found.describe(),
            recorded.describe()
        ));
    }
    if recorded.kind == ManifestKind::File && recorded.bytes != found.bytes {
        return Some(format!("{} bytes now, was {}", found.bytes, recorded.bytes));
    }
    if recorded.link_target != found.link_target {
        return Some(format!(
            "{} now, was {}",
            found.describe(),
            recorded.describe()
        ));
    }
    let times_count = match rule {
        Match::Exact => true,
        Match::Whole => recorded.kind != ManifestKind::Directory,
        Match::Content => false,
    };
    if times_count && recorded.source_modified != found.source_modified {
        return Some("modified since it was scanned".to_string());
    }
    None
}

/// What a walk found that the manifest does not say.
///
/// Every list keeps its paths, so a caller can say how much of a tree is left
/// ("holds 16 of the 1473 entries the move recorded") as well as what differs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ManifestDiff {
    /// How many entries the manifest recorded.
    pub recorded: usize,
    /// Recorded, and not there. An entry beneath a folder that could not be
    /// read is not counted: it is unknown, and the folder's problem says so.
    pub missing: Vec<PathBuf>,
    /// There, and never recorded.
    pub added: Vec<PathBuf>,
    /// There, but not as recorded: the path and how it differs.
    pub changed: Vec<(PathBuf, String)>,
    /// What the walk could not take, at paths the manifest did not record.
    pub problems: Vec<WalkProblem>,
}

impl ManifestDiff {
    /// The tree is what the manifest recorded, and nothing else.
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.is_residue()
    }

    /// Everything still there is recorded and unchanged, and nothing is there
    /// that was not recorded — what a tree removed part of the way looks like,
    /// and nothing else does. Entries may be missing.
    pub fn is_residue(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.problems.is_empty()
    }

    /// How many recorded entries are still there.
    pub fn present(&self) -> usize {
        self.recorded.saturating_sub(self.missing.len())
    }

    /// Counts, then up to `limit` paths with what is wrong with each — the
    /// ones that changed first, since those are what a reader must look at.
    pub fn summary(&self, limit: usize) -> String {
        let mut counts = Vec::new();
        if !self.changed.is_empty() {
            counts.push(format!("{} changed", self.changed.len()));
        }
        if !self.added.is_empty() {
            counts.push(format!("{} not in the record", self.added.len()));
        }
        if !self.problems.is_empty() {
            counts.push(format!("{} that cannot be compared", self.problems.len()));
        }
        if !self.missing.is_empty() {
            counts.push(format!(
                "{} of the {} recorded entries missing",
                self.missing.len(),
                self.recorded
            ));
        }
        if counts.is_empty() {
            return "no differences".to_string();
        }
        let lines: Vec<String> = self
            .changed
            .iter()
            .map(|(path, change)| format!("{}: {change}", path.display()))
            .chain(
                self.added
                    .iter()
                    .map(|path| format!("{}: not in the record", path.display())),
            )
            .chain(
                self.problems
                    .iter()
                    .map(|problem| format!("{}: {}", problem.path.display(), problem.problem)),
            )
            .chain(
                self.missing
                    .iter()
                    .map(|path| format!("{}: missing", path.display())),
            )
            .collect();
        let mut out = counts.join(", ");
        for line in lines.iter().take(limit) {
            out.push_str("\n  ");
            out.push_str(line);
        }
        if lines.len() > limit {
            out.push_str(&format!("\n  and {} more", lines.len() - limit));
        }
        out
    }
}

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
    fn what(&self) -> String {
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
        crate::util::paths::require_real_directory(root, label)?;
        let device = fs::symlink_metadata(root)
            .map(|metadata| device_of(&metadata))
            .with_context(|| format!("reading metadata for {}", root.display()))?;
        let mut walk = Self::default();
        walk_at(root, root, 0, device, &mut walk)?;
        walk.entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        walk.problems
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(walk)
    }

    /// The entries, or a refusal naming every problem (the first
    /// [`LISTED`]).
    fn into_entries(self, root: &Path) -> Result<Vec<ManifestEntry>> {
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

/// The recursive step. **Only [`Walk::of`] starts it at depth 0**; recursing
/// through an entry point would reset the count and make the limit
/// unreachable.
fn walk_at(
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
        // A listing that fails part of the way cannot say what it did not
        // list, so the whole folder is a problem.
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

/// A claimed transaction directory owned by the current move.
#[derive(Debug)]
pub struct MoveTransaction {
    pub target_base: PathBuf,
    pub operation_dir: PathBuf,
    pub journal: MoveJournal,
}

impl MoveTransaction {
    pub fn begin(
        source_base: &Path,
        source_folder: &Path,
        target_base: &Path,
        target_folder: &Path,
        project_id: &str,
        operation: Operation,
    ) -> Result<Self> {
        crate::util::paths::require_real_directory(source_base, "source base")?;
        crate::util::paths::require_real_directory(target_base, "target base")?;
        validate_folder(source_folder, "source")?;
        validate_folder(target_folder, "target")?;

        let transaction_root = ensure_transaction_root(target_base)?;
        for _ in 0..OPERATION_RETRIES {
            let operation_id = next_operation_id();
            let operation_dir = transaction_root.join(&operation_id);
            match fs::create_dir(&operation_dir) {
                Ok(()) => {
                    let result = (|| -> Result<Self> {
                        crate::util::faults::check("move:after-transaction-create")?;
                        let journal = MoveJournal {
                            version: MOVE_VERSION,
                            operation_id,
                            project_id: project_id.to_string(),
                            source_base: source_base.to_path_buf(),
                            source_folder: source_folder.to_path_buf(),
                            target_folder: target_folder.to_path_buf(),
                            phase: MovePhase::Copying,
                            operation,
                            host: this_host(),
                            legacy_cleanup: false,
                        };
                        crate::util::atomic::write_json(
                            &operation_dir.join(JOURNAL_FILE),
                            &journal,
                        )
                        .context("writing Copying move journal")?;
                        Ok(Self {
                            target_base: target_base.to_path_buf(),
                            operation_dir: operation_dir.clone(),
                            journal,
                        })
                    })();
                    if result.is_err() {
                        let _ = crate::util::fs_retry::remove_dir_all(&operation_dir);
                    }
                    return result;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("claiming move transaction {}", operation_dir.display())
                    });
                }
            }
        }
        bail!(
            "could not allocate a unique move operation under {}",
            transaction_root.display()
        )
    }

    pub fn staging_path(&self) -> PathBuf {
        self.operation_dir.join(STAGING_DIR)
    }

    pub fn final_path(&self) -> PathBuf {
        self.target_base.join(&self.journal.target_folder)
    }

    pub fn source_path(&self) -> PathBuf {
        self.journal.source_base.join(&self.journal.source_folder)
    }

    /// Where the source goes when it leaves the library.
    pub fn retired_path(&self) -> PathBuf {
        retired_path(&self.journal.source_base, &self.journal.operation_id)
    }

    /// Record the destination as it is about to be published.
    pub fn write_published(&self, walk: &Walk) -> Result<MoveManifest> {
        let published = MoveManifest {
            version: MANIFEST_VERSION,
            entries: walk.entries.clone(),
        };
        published.validate()?;
        crate::util::atomic::write_json(&self.operation_dir.join(PUBLISHED_FILE), &published)
            .context("writing the published record")?;
        Ok(published)
    }

    /// The destination as it was published, or `None` for a transaction that
    /// never recorded one (3.11 did not).
    pub fn read_published(&self) -> Result<Option<MoveManifest>> {
        let path = self.operation_dir.join(PUBLISHED_FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => {}
        }
        crate::util::paths::require_real_file(&path, "published record")?;
        let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let published: MoveManifest =
            serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
        published.validate()?;
        Ok(Some(published))
    }

    pub fn write_manifest(&self, manifest: &MoveManifest) -> Result<()> {
        manifest.validate()?;
        crate::util::atomic::write_json(&self.operation_dir.join(MANIFEST_FILE), manifest)
            .context("writing move manifest")
    }

    pub fn read_manifest(&self) -> Result<MoveManifest> {
        read_manifest(&self.operation_dir)
    }

    /// Record `phase`. **Always as this build's version**: a version-2 journal
    /// is rewritten as version 3 here, before its source is renamed, so an
    /// older binary cannot then find no source, call the move finished and
    /// leave the retired folder behind with nothing recording it.
    pub fn set_phase(&mut self, phase: MovePhase) -> Result<()> {
        let mut next = self.journal.clone();
        next.phase = phase;
        next.version = MOVE_VERSION;
        crate::util::atomic::write_json(&self.operation_dir.join(JOURNAL_FILE), &next)
            .with_context(|| format!("writing {:?} move phase", phase))?;
        self.journal = next;
        Ok(())
    }

    pub fn claim_staging(&self) -> Result<PathBuf> {
        let staging = self.staging_path();
        fs::create_dir(&staging)
            .with_context(|| format!("claiming private staging {}", staging.display()))?;
        Ok(staging)
    }

    /// Remove only this exclusively-created operation directory.
    pub fn remove(self) -> Result<()> {
        let expected_root = transaction_root(&self.target_base);
        if self.operation_dir.parent() != Some(expected_root.as_path())
            || self
                .operation_dir
                .file_name()
                .and_then(|name| name.to_str())
                != Some(self.journal.operation_id.as_str())
        {
            bail!(
                "refusing to remove transaction outside its owned location: {}",
                self.operation_dir.display()
            );
        }
        crate::util::paths::require_real_directory(&self.operation_dir, "move transaction")?;
        crate::util::fs_retry::remove_dir_all(&self.operation_dir)
            .with_context(|| format!("removing move transaction {}", self.operation_dir.display()))
    }
}

pub fn ensure_transaction_root(target_base: &Path) -> Result<PathBuf> {
    crate::util::paths::require_real_directory(target_base, "target base")?;
    let root = target_base.join(TRANSACTIONS_DIR);
    match fs::create_dir(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("creating transaction root {}", root.display()));
        }
    }
    crate::util::paths::require_real_directory(&root, "transaction root")?;
    Ok(root)
}

pub fn transaction_root(target_base: &Path) -> PathBuf {
    target_base.join(TRANSACTIONS_DIR)
}

/// `<source base>/.fastf-moved-<operation>`: derived, never stored.
pub fn retired_path(source_base: &Path, operation_id: &str) -> PathBuf {
    source_base.join(format!("{RETIRED_PREFIX}{operation_id}"))
}

/// The operation a retired folder's name carries, if it is one fastf writes.
pub fn retired_operation(name: &str) -> Option<&str> {
    name.strip_prefix(RETIRED_PREFIX)
        .filter(|operation| validate_operation_id(operation).is_ok())
}

pub fn read_journal(operation_dir: &Path) -> Result<MoveJournal> {
    let path = operation_dir.join(JOURNAL_FILE);
    crate::util::paths::require_real_file(&path, "move journal")?;
    let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let journal: MoveJournal =
        serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
    if !(MOVE_VERSION_OLDEST..=MOVE_VERSION).contains(&journal.version) {
        bail!(
            "unsupported move journal version {} at {}",
            journal.version,
            path.display()
        );
    }
    let mut journal = journal;
    if journal.version < 3 {
        if journal.phase == MovePhase::Retired
            || journal.host.is_some()
            || journal.legacy_cleanup
            || journal.operation != Operation::Move
        {
            bail!(
                "a version-{} move journal cannot record what only version 3 has: {}",
                journal.version,
                path.display()
            );
        }
        journal.legacy_cleanup = true;
    }
    validate_operation_id(&journal.operation_id)?;
    validate_folder(&journal.source_folder, "source")?;
    validate_folder(&journal.target_folder, "target")?;
    if !journal.source_base.is_absolute() {
        bail!("move journal source base is not absolute");
    }
    let directory_name = operation_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("transaction directory has no UTF-8 operation id")?;
    if directory_name != journal.operation_id {
        bail!(
            "move journal operation id '{}' does not match directory '{}'",
            journal.operation_id,
            directory_name
        );
    }
    Ok(journal)
}

pub fn read_manifest(operation_dir: &Path) -> Result<MoveManifest> {
    let path = operation_dir.join(MANIFEST_FILE);
    crate::util::paths::require_real_file(&path, "move manifest")?;
    let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: MoveManifest =
        serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn transaction_from_journal(
    target_base: &Path,
    operation_dir: &Path,
    journal: MoveJournal,
) -> MoveTransaction {
    MoveTransaction {
        target_base: target_base.to_path_buf(),
        operation_dir: operation_dir.to_path_buf(),
        journal,
    }
}

/// Copy from a manifest with one reusable bounded buffer. Files are written
/// directly into private staging, so no sibling `.part` convention exists.
///
/// **Two passes: every name first, then every file's contents.** A name the
/// target's filesystem will not hold — two that differ only in case on a drive
/// that ignores it, a `:` on one that forbids it, a name too long — used to be
/// found file by file during the copy, possibly hours in. Making every folder
/// and every (empty) file first finds all of them in seconds, before a byte of
/// content has moved, and names them together.
pub fn copy_to_staging(
    manifest: &MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<()> {
    crate::util::paths::require_real_directory(source, "move source")?;
    crate::util::paths::require_real_directory(staging, "move staging")?;
    manifest.validate()?;
    create_names(manifest, staging, cancel)?;
    copy_contents(manifest, source, staging, progress, cancel)
}

/// The first pass: every folder, every file (empty), and — last, so no later
/// write can pass through one — every link.
fn create_names(manifest: &MoveManifest, staging: &Path, cancel: &AtomicBool) -> Result<()> {
    let ordered = manifest
        .entries
        .iter()
        .filter(|entry| !entry.kind.is_link())
        .chain(manifest.entries.iter().filter(|entry| entry.kind.is_link()));
    let mut refused: Vec<(PathBuf, String)> = Vec::new();
    for entry in ordered {
        if cancel.load(Ordering::Relaxed) {
            bail!("move cancelled");
        }
        // Beneath a folder that could not be made, nothing can be; its own
        // refusal says why.
        if refused.iter().any(|(path, _)| entry.path.starts_with(path)) {
            continue;
        }
        let destination = staging.join(&entry.path);
        let made = match entry.kind {
            ManifestKind::Directory => fs::create_dir(&destination),
            ManifestKind::File => OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map(drop),
            ManifestKind::Symlink | ManifestKind::DirSymlink | ManifestKind::Junction => {
                match &entry.link_target {
                    Some(target) => make_link(entry.kind, target, &destination),
                    None => Err(std::io::Error::other(
                        "the record of this link has no target",
                    )),
                }
            }
        };
        if let Err(error) = made {
            let why = if entry.kind.is_link() {
                link_refusal(&error)
            } else {
                name_refusal(&error)
            };
            refused.push((entry.path.clone(), why));
        }
    }
    if refused.is_empty() {
        return Ok(());
    }
    let count = refused.len();
    let mut message = format!(
        "{count} {} cannot be made where the project is going:",
        if count == 1 { "name" } else { "names" }
    );
    for (path, why) in refused.iter().take(LISTED) {
        message.push_str(&format!("\n  {}: {why}", path.display()));
    }
    if count > LISTED {
        message.push_str(&format!("\n  and {} more", count - LISTED));
    }
    message.push_str("\nNo file's contents were copied.");
    bail!("{message}")
}

/// Why the target's filesystem would not make a name, in words.
///
/// `AlreadyExists` in a staging folder fastf made empty a moment ago can only
/// mean the filesystem took two of the project's names for one: it ignores
/// case, or folds these letters together.
pub(crate) fn name_refusal(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return "the filesystem there takes this name for another one in the same folder \
                (it ignores case, or folds these letters together)"
            .to_string();
    }
    #[cfg(unix)]
    match error.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::EILSEQ) => {
            return "the filesystem there does not allow this name".to_string();
        }
        Some(libc::ENAMETOOLONG) => {
            return "the name is too long for the filesystem there".to_string();
        }
        _ => {}
    }
    #[cfg(windows)]
    match error.raw_os_error() {
        // ERROR_INVALID_NAME
        Some(123) => return "the filesystem there does not allow this name".to_string(),
        // ERROR_FILENAME_EXCED_RANGE
        Some(206) => return "the name is too long for the filesystem there".to_string(),
        _ => {}
    }
    error.to_string()
}

/// Make `link` again, pointing exactly where the original did — the target
/// text, never resolved, and never required to exist.
fn make_link(kind: ManifestKind, target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = kind;
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        match kind {
            ManifestKind::Junction => crate::util::win_reparse::create_junction(target, link),
            ManifestKind::DirSymlink => std::os::windows::fs::symlink_dir(target, link),
            _ => std::os::windows::fs::symlink_file(target, link),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (kind, target, link);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Why the target's filesystem would not make a link, in words.
pub(crate) fn link_refusal(error: &std::io::Error) -> String {
    #[cfg(unix)]
    if matches!(
        error.raw_os_error(),
        Some(libc::EPERM) | Some(libc::EOPNOTSUPP) | Some(libc::ENOSYS)
    ) {
        return "the filesystem there cannot hold links".to_string();
    }
    #[cfg(windows)]
    match error.raw_os_error() {
        // ERROR_PRIVILEGE_NOT_HELD
        Some(1314) => {
            return "Windows lets this account make symbolic links only with Developer Mode \
                    on (Settings, System, For developers)"
                .to_string();
        }
        // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED
        Some(1) | Some(50) => return "the filesystem there cannot hold links".to_string(),
        _ => {}
    }
    name_refusal(error)
}

/// The second pass: each file's contents, into the empty file the first pass
/// made — opened without following a link, though none can be there.
fn copy_contents(
    manifest: &MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    for entry in &manifest.entries {
        if cancel.load(Ordering::Relaxed) {
            bail!("move cancelled");
        }
        if entry.kind != ManifestKind::File {
            continue;
        }
        let source_path = source.join(&entry.path);
        let destination_path = staging.join(&entry.path);
        let current = entry_from_path(&source_path, &entry.path)?;
        if &current != entry {
            bail!(
                "move source changed before copying {}",
                entry.path.display()
            );
        }
        if let Ok(mut state) = progress.lock() {
            state.current_file = entry.path.to_string_lossy().into_owned();
            state.touch();
        }
        let mut reader = fs::File::open(&source_path)
            .with_context(|| format!("opening {}", source_path.display()))?;
        let mut options = OpenOptions::new();
        options.write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut writer = options
            .open(&destination_path)
            .with_context(|| format!("opening {}", destination_path.display()))?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!("move cancelled");
            }
            crate::util::faults::check("move:mid-copy")?;
            let count = reader
                .read(&mut buffer)
                .with_context(|| format!("reading {}", source_path.display()))?;
            if count == 0 {
                break;
            }
            writer
                .write_all(&buffer[..count])
                .with_context(|| format!("writing {}", destination_path.display()))?;
            if let Ok(mut state) = progress.lock() {
                state.copied_bytes = state.copied_bytes.saturating_add(count as u64);
                state.touch();
            }
        }
        writer
            .flush()
            .with_context(|| format!("flushing {}", destination_path.display()))?;
        writer
            .sync_all()
            .with_context(|| format!("syncing {}", destination_path.display()))?;
        if let Ok(mut state) = progress.lock() {
            state.done_files += 1;
            state.touch();
        }
    }
    Ok(())
}

fn entry_from_path(path: &Path, relative: &Path) -> Result<ManifestEntry> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        bail!(
            "move source entry is no longer a regular file: {}",
            path.display()
        );
    }
    Ok(ManifestEntry {
        path: relative.to_path_buf(),
        kind: ManifestKind::File,
        bytes: metadata.len(),
        source_modified: ModifiedTime::from_system_time(
            metadata
                .modified()
                .with_context(|| format!("reading modification time for {}", path.display()))?,
        ),
        link_target: None,
    })
}

fn validate_folder(path: &Path, label: &str) -> Result<()> {
    let mut components = path.components();
    let Some(Component::Normal(name)) = components.next() else {
        bail!("move {label} folder must be one safe path component");
    };
    if components.next().is_some() || name.is_empty() {
        bail!("move {label} folder must be one safe path component");
    }
    if name == TRANSACTIONS_DIR {
        bail!("move {label} folder uses the reserved transaction name");
    }
    Ok(())
}

fn validate_operation_id(operation_id: &str) -> Result<()> {
    if operation_id.is_empty()
        || !operation_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        bail!("invalid move operation id '{operation_id}'");
    }
    Ok(())
}

pub(crate) fn next_operation_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = OPERATION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:x}-{:x}-{counter:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_preserves_exact_topology_and_source_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let staging = temp.path().join("staging");
        fs::create_dir_all(source.join("empty")).unwrap();
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("zero.tmp"), []).unwrap();
        fs::write(source.join("nested/data.part"), [0_u8, 255, 7]).unwrap();
        // A move never interpolates: literal braces survive in names and bytes.
        fs::write(source.join("notes_{client}.md"), "hello {name}").unwrap();
        fs::create_dir(&staging).unwrap();

        let manifest = MoveManifest::scan(&source).unwrap();
        let progress = Mutex::new(Progress::new(&[]));
        copy_to_staging(
            &manifest,
            &source,
            &staging,
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        manifest.verify_destination(&staging).unwrap();
        manifest.verify_source_unchanged(&source).unwrap();
        assert!(staging.join("empty").is_dir());
        assert_eq!(fs::read(staging.join("zero.tmp")).unwrap(), b"");
        assert_eq!(
            fs::read(staging.join("nested/data.part")).unwrap(),
            [0_u8, 255, 7]
        );
        assert_eq!(
            fs::read_to_string(staging.join("notes_{client}.md")).unwrap(),
            "hello {name}"
        );
    }

    /// Create a directory link inside a test tree, cross-platform. Windows
    /// junctions need no elevation (unlike symlinks, which want Developer
    /// Mode), so `mklink /J` is the portable-enough choice there. Returns
    /// `false` when the OS refused, so a test can skip rather than fail on a
    /// machine with restrictive policy.
    fn make_dir_link(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    /// The data-loss regression, now guarded where the invariant lives. A
    /// junction inside a project was once invisible to the walk, so a staged
    /// move copied around it, verification walked the same blind way and
    /// reported success, and the source was deleted. A link is now recorded by
    /// its target — never followed, never silently omitted — so what is behind
    /// it is neither copied nor, when the original is removed, deleted.
    #[test]
    fn scan_records_a_link_by_its_target_and_never_follows_it() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("real_asset_library");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("payload.txt"), "irreplaceable").unwrap();

        let source = temp.path().join("project");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("normal.txt"), "ordinary").unwrap();
        if !make_dir_link(&source.join("linked"), &target) {
            eprintln!("skipping: OS refused to create a directory link");
            return;
        }

        let manifest = MoveManifest::scan(&source).unwrap();
        let linked = manifest.entry(Path::new("linked")).expect("recorded");
        assert!(linked.kind.is_link(), "{linked:?}");
        #[cfg(unix)]
        assert_eq!(linked.kind, ManifestKind::Symlink);
        #[cfg(windows)]
        assert_eq!(linked.kind, ManifestKind::Junction);
        assert_eq!(
            fs::read_link(source.join("linked")).ok(),
            linked.link_target.clone()
        );
        assert!(
            manifest.entries.iter().all(|entry| !entry
                .path
                .parent()
                .is_some_and(|parent| parent.starts_with("linked"))),
            "nothing behind the link is walked: {manifest:?}"
        );
    }

    /// Links cross as links — relative, absolute, dangling — and verification
    /// compares their target text: a retargeted link, or a file where a link
    /// was, is a copy that does not match.
    #[cfg(unix)]
    #[test]
    fn links_are_copied_as_links_and_verified_by_their_target() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let staging = temp.path().join("staging");
        fs::create_dir_all(source.join("node_modules/.bin")).unwrap();
        fs::write(source.join("real.txt"), "real").unwrap();
        let links = [
            ("node_modules/.bin/vite", "../vite/bin/vite.js"),
            ("to_real", "real.txt"),
            ("absolute", "/nonexistent/fastf/test/target"),
        ];
        for (link, target) in links {
            std::os::unix::fs::symlink(target, source.join(link)).unwrap();
        }
        fs::create_dir(&staging).unwrap();

        let manifest = MoveManifest::scan(&source).unwrap();
        assert_eq!(manifest.total_links(), 3);
        copy_to_staging(
            &manifest,
            &source,
            &staging,
            &Mutex::new(Progress::new(&[])),
            &AtomicBool::new(false),
        )
        .unwrap();
        manifest.verify_destination(&staging).unwrap();
        for (link, target) in links {
            assert_eq!(
                fs::read_link(staging.join(link)).unwrap(),
                Path::new(target)
            );
        }

        fs::remove_file(staging.join("to_real")).unwrap();
        std::os::unix::fs::symlink("elsewhere.txt", staging.join("to_real")).unwrap();
        let error = format!("{:#}", manifest.verify_destination(&staging).unwrap_err());
        assert!(
            error.contains("to_real: a link to elsewhere.txt now, was a link to real.txt"),
            "{error}"
        );
        fs::remove_file(staging.join("to_real")).unwrap();
        fs::write(staging.join("to_real"), "real").unwrap();
        let error = format!("{:#}", manifest.verify_destination(&staging).unwrap_err());
        assert!(error.contains("to_real: a 4-byte file now"), "{error}");
    }

    /// A link is kept exactly, so one that climbs out of the project, or names
    /// the original folder by its full path, may mean something else after the
    /// move — and the outcome says which.
    #[test]
    fn links_whose_meaning_the_new_place_may_change_are_named() {
        let original = Path::new("/projects/base/Album_ID0001");
        let link = |path: &str, target: &str| ManifestEntry {
            path: PathBuf::from(path),
            kind: ManifestKind::Symlink,
            bytes: 0,
            source_modified: ModifiedTime::from_system_time(UNIX_EPOCH),
            link_target: Some(PathBuf::from(target)),
        };
        let manifest = MoveManifest {
            version: MANIFEST_VERSION,
            entries: vec![
                link("inside", "sub/file"),
                link("sub/sibling", "../other"),
                link("sub/up_and_out", "../../shared"),
                link("out", "../shared_assets"),
                link("self", "/projects/base/Album_ID0001/sub/file"),
                link("elsewhere", "/mnt/library/stock"),
            ],
        };
        let notes = manifest.link_notes(original).join("\n");
        assert!(
            !notes.contains("inside") && !notes.contains("sub/sibling"),
            "{notes}"
        );
        assert!(notes.contains("sub/up_and_out points outside"), "{notes}");
        assert!(notes.contains("out points outside"), "{notes}");
        assert!(
            notes.contains("self points into the original folder"),
            "{notes}"
        );
        assert!(
            !notes.contains("elsewhere"),
            "an absolute link elsewhere still works: {notes}"
        );
    }

    /// sshfs's default `contain_symlinks` hands back `EPERM` for every link
    /// that climbs with `..`; the refusal names the option that lifts it.
    #[test]
    fn a_link_the_mount_will_not_read_names_the_option() {
        let problem = Problem::LinkNotReadable("Operation not permitted (os error 1)".into());
        let said = problem.to_string();
        assert!(said.contains("no_contain_symlinks"), "{said}");
        assert!(said.contains("Operation not permitted"), "{said}");
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_link_is_said_in_words() {
        let refusal = link_refusal(&std::io::Error::from_raw_os_error(libc::EPERM));
        assert!(refusal.contains("cannot hold links"), "{refusal}");
    }

    /// A missing source is an error, never an empty manifest that would verify
    /// against an empty destination.
    #[test]
    fn scan_of_a_missing_tree_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        assert!(MoveManifest::scan(&temp.path().join("missing")).is_err());
    }

    /// Verification is what stands between a move and deleting a good source,
    /// so it has to catch the real network-share failure modes: a truncated
    /// file and a dropped one.
    #[test]
    fn verify_destination_detects_short_and_missing_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let staging = temp.path().join("staging");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("a.txt"), "hello").unwrap();
        fs::write(source.join("sub/b.bin"), vec![0_u8; 2048]).unwrap();
        fs::create_dir(&staging).unwrap();

        let manifest = MoveManifest::scan(&source).unwrap();
        let progress = Mutex::new(Progress::new(&[]));
        copy_to_staging(
            &manifest,
            &source,
            &staging,
            &progress,
            &AtomicBool::new(false),
        )
        .unwrap();
        manifest.verify_destination(&staging).unwrap();

        // Truncated at the destination.
        fs::write(staging.join("sub/b.bin"), vec![0_u8; 1024]).unwrap();
        assert!(manifest.verify_destination(&staging).is_err());
        fs::write(staging.join("sub/b.bin"), vec![0_u8; 2048]).unwrap();
        manifest.verify_destination(&staging).unwrap();

        // Dropped at the destination.
        fs::remove_file(staging.join("a.txt")).unwrap();
        assert!(manifest.verify_destination(&staging).is_err());
    }

    #[test]
    fn source_metadata_changes_are_detected_after_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"one").unwrap();
        let manifest = MoveManifest::scan(&source).unwrap();
        fs::write(source.join("file"), b"two-two").unwrap();
        assert!(manifest.verify_source_unchanged(&source).is_err());
    }

    #[test]
    fn transaction_journal_derives_target_and_staging_from_location() {
        let temp = tempfile::tempdir().unwrap();
        let source_base = temp.path().join("source-base");
        let target_base = temp.path().join("target-base");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let transaction = MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
            Operation::Move,
        )
        .unwrap();
        assert_eq!(transaction.final_path(), target_base.join("project"));
        assert_eq!(
            transaction.staging_path(),
            transaction.operation_dir.join(STAGING_DIR)
        );
        let raw = fs::read_to_string(transaction.operation_dir.join(JOURNAL_FILE)).unwrap();
        assert!(!raw.contains("staging"));
        assert!(!raw.contains(&target_base.display().to_string()));
    }

    /// Running as root, a permission test proves nothing: root reads a
    /// mode-000 folder. The install lab's containers run as root.
    #[cfg(unix)]
    fn running_as_root() -> bool {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// A tree with several things a move cannot take used to be refused one
    /// name at a time: fix the first, run again, meet the second.
    #[cfg(unix)]
    #[test]
    fn a_scan_names_every_problem_not_just_the_first() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir_all(source.join("run")).unwrap();
        fs::write(source.join("notes.md"), "ordinary").unwrap();
        let fifo = std::ffi::CString::new(
            source
                .join("run/pipe")
                .as_os_str()
                .as_encoded_bytes()
                .to_vec(),
        )
        .unwrap();
        // SAFETY: a valid NUL-terminated path and a plain mode.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let _socket = std::os::unix::net::UnixListener::bind(source.join("run/app.sock")).unwrap();

        let error = MoveManifest::scan(&source).unwrap_err().to_string();
        assert!(error.contains("2 entries"), "{error}");
        assert!(
            error.contains("run/pipe") && error.contains("run/app.sock"),
            "{error}"
        );
        assert!(error.contains("socket, pipe or device"), "{error}");
    }

    /// The incident's first message was `classifying …: No such file or
    /// directory` — true, and no help. An entry the folder lists but `lstat`
    /// cannot examine is now named as exactly that, and a folder that cannot
    /// be listed at all is named as that.
    #[cfg(unix)]
    #[test]
    fn entries_that_cannot_be_examined_or_listed_are_named_as_such() {
        if running_as_root() {
            eprintln!("skipping: root can examine anything");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir_all(source.join("listed_only")).unwrap();
        fs::create_dir_all(source.join("sealed")).unwrap();
        fs::write(source.join("listed_only/inside.txt"), "x").unwrap();
        fs::write(source.join("sealed/hidden.txt"), "x").unwrap();
        set_mode(&source.join("listed_only"), 0o644);
        set_mode(&source.join("sealed"), 0o000);

        let result = MoveManifest::scan(&source);
        set_mode(&source.join("listed_only"), 0o755);
        set_mode(&source.join("sealed"), 0o755);
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains(
                "listed_only/inside.txt: listed by the filesystem, but it cannot be examined"
            ),
            "{error}"
        );
        assert!(
            error.contains("sealed: its contents cannot be listed"),
            "{error}"
        );
    }

    /// Beneath a folder that cannot be read, an entry is unknown, not missing:
    /// "1457 missing" about files nobody could look for would be false.
    #[cfg(unix)]
    #[test]
    fn entries_beneath_an_unreadable_folder_are_not_counted_missing() {
        if running_as_root() {
            eprintln!("skipping: root can list anything");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir_all(source.join("sealed")).unwrap();
        fs::write(source.join("sealed/a.txt"), "a").unwrap();
        fs::write(source.join("sealed/b.txt"), "b").unwrap();
        let manifest = MoveManifest::scan(&source).unwrap();
        set_mode(&source.join("sealed"), 0o000);
        let walk = Walk::of(&source, "tree");
        set_mode(&source.join("sealed"), 0o755);

        let diff = manifest.compare(&walk.unwrap(), Match::Exact);
        assert!(diff.missing.is_empty(), "{diff:?}");
        assert_eq!(diff.problems.len(), 1, "{diff:?}");
        assert!(!diff.is_residue());
    }

    /// 3.11 wrote version-1 manifests. Comparing whole manifests, `version`
    /// included, would read every transaction it left as "changed" forever.
    #[test]
    fn a_version_1_manifest_still_verifies_against_a_fresh_scan() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("sub/file.bin"), [1_u8, 2, 3]).unwrap();
        let mut stored = serde_json::to_value(MoveManifest::scan(&source).unwrap()).unwrap();
        stored["version"] = serde_json::json!(1);
        let operation = temp.path().join("operation");
        fs::create_dir(&operation).unwrap();
        fs::write(
            operation.join(MANIFEST_FILE),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();

        let recovered = read_manifest(&operation).unwrap();
        recovered.verify_source_unchanged(&source).unwrap();
        recovered.verify_destination(&source).unwrap();
    }

    /// A difference names the path and what is different about it, so the
    /// person reading a refused move or a stuck reconcile knows where to look.
    #[cfg(unix)]
    #[test]
    fn a_difference_names_each_path_and_what_changed() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("grown.txt"), "12345").unwrap();
        fs::write(source.join("gone.txt"), "x").unwrap();
        fs::write(source.join("sub/was_a_file"), "abc").unwrap();
        let manifest = MoveManifest::scan(&source).unwrap();

        fs::write(source.join("grown.txt"), "123456789012").unwrap();
        fs::remove_file(source.join("gone.txt")).unwrap();
        fs::write(source.join("new.txt"), "n").unwrap();
        fs::remove_file(source.join("sub/was_a_file")).unwrap();
        std::os::unix::fs::symlink("elsewhere", source.join("sub/was_a_file")).unwrap();

        let diff = manifest.compare(&Walk::of(&source, "tree").unwrap(), Match::Content);
        let summary = diff.summary(10);
        assert!(
            summary.contains("grown.txt: 12 bytes now, was 5"),
            "{summary}"
        );
        assert!(summary.contains("gone.txt: missing"), "{summary}");
        assert!(summary.contains("new.txt: not in the record"), "{summary}");
        assert!(
            summary.contains("sub/was_a_file: a link to elsewhere now, was a 3-byte file"),
            "{summary}"
        );
        assert!(!diff.is_residue());
        assert_eq!(diff.present(), diff.recorded - 1);
    }

    /// Removing part of a tree moves its folders' times and nothing else, so
    /// a half-removed tree is a residue: everything left is as recorded.
    /// Anything added or changed, and it is not.
    #[test]
    fn a_partly_removed_tree_is_a_residue_and_an_edited_one_is_not() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir_all(source.join("sub")).unwrap();
        for name in ["a.txt", "sub/b.txt", "sub/c.txt"] {
            fs::write(source.join(name), name).unwrap();
        }
        let manifest = MoveManifest::scan(&source).unwrap();
        // Folder times have whole-second resolution on some filesystems.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::remove_file(source.join("sub/b.txt")).unwrap();
        fs::remove_file(source.join("a.txt")).unwrap();

        let walk = Walk::of(&source, "tree").unwrap();
        let whole = manifest.compare(&walk, Match::Whole);
        assert!(whole.is_residue(), "{whole:?}");
        assert!(!whole.is_clean());
        assert_eq!(whole.missing.len(), 2);
        assert_eq!(whole.present(), 2, "sub and sub/c.txt are left");
        let exact = manifest.compare(&walk, Match::Exact);
        assert!(
            exact
                .changed
                .iter()
                .any(|(path, _)| path == Path::new("sub")),
            "an exact comparison sees the folder's time move: {exact:?}"
        );

        fs::write(source.join("sub/c.txt"), "edited, and longer").unwrap();
        let edited = manifest.compare(&Walk::of(&source, "tree").unwrap(), Match::Whole);
        assert!(!edited.is_residue(), "{edited:?}");
    }

    /// Every name the target will not hold is found before any content is
    /// copied, and named together. A name already taken in a staging folder
    /// fastf made empty is how a case-insensitive drive refuses `README` beside
    /// `readme`; planting the clash is how a test on a case-sensitive one gets
    /// there.
    #[test]
    fn the_names_pass_refuses_every_name_before_copying_a_byte() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let staging = temp.path().join("staging");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("a.txt"), "aaa").unwrap();
        fs::write(source.join("c.txt"), "ccc").unwrap();
        fs::write(source.join("sub/b.txt"), "bbb").unwrap();
        fs::create_dir(&staging).unwrap();
        let manifest = MoveManifest::scan(&source).unwrap();
        fs::write(staging.join("a.txt"), "").unwrap();
        fs::write(staging.join("sub"), "").unwrap();

        let error = copy_to_staging(
            &manifest,
            &source,
            &staging,
            &Mutex::new(Progress::new(&[])),
            &AtomicBool::new(false),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("2 names"),
            "the folder's own entries are not counted: {error}"
        );
        assert!(
            error.contains("a.txt: the filesystem there takes this name"),
            "{error}"
        );
        assert!(
            error.contains("sub: the filesystem there takes this name"),
            "{error}"
        );
        assert!(error.contains("No file's contents were copied"), "{error}");
        assert_eq!(
            fs::read(staging.join("c.txt")).unwrap(),
            b"",
            "made, and left empty: the contents pass never ran"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_name_is_said_in_words() {
        let from = |code| name_refusal(&std::io::Error::from_raw_os_error(code));
        assert!(from(libc::EINVAL).contains("does not allow this name"));
        assert!(from(libc::EILSEQ).contains("does not allow this name"));
        assert!(from(libc::ENAMETOOLONG).contains("too long"));
        assert!(from(libc::EEXIST).contains("takes this name for another"));
    }

    /// 3.11's journals are read, marked as ones whose source it may have
    /// half-removed, and rewritten as version 3 by the next phase change; a
    /// version-2 journal claiming what only version 3 records, and a journal
    /// from a future version, are refused.
    #[test]
    fn journals_are_read_across_versions_and_rewritten_as_the_newest() {
        let temp = tempfile::tempdir().unwrap();
        let source_base = temp.path().join("source-base");
        let target_base = temp.path().join("target-base");
        fs::create_dir(&source_base).unwrap();
        fs::create_dir(&target_base).unwrap();
        let transaction = MoveTransaction::begin(
            &source_base,
            Path::new("project"),
            &target_base,
            Path::new("project"),
            "ID0001",
            Operation::Move,
        )
        .unwrap();
        let operation = transaction.operation_dir.clone();
        let journal_path = operation.join(JOURNAL_FILE);
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
        assert_eq!(written["version"], 3);
        assert!(written.get("legacy_cleanup").is_none(), "{written}");

        let write = |edit: &dyn Fn(&mut serde_json::Map<String, serde_json::Value>)| {
            let mut journal = written.clone();
            edit(journal.as_object_mut().unwrap());
            fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
        };
        write(&|journal| {
            journal.insert("version".into(), serde_json::json!(2));
            journal.remove("operation");
            journal.remove("host");
        });
        let legacy = read_journal(&operation).unwrap();
        assert!(legacy.source_may_be_partial());
        let mut rewritten = transaction_from_journal(&target_base, &operation, legacy);
        rewritten.set_phase(MovePhase::CleanupPending).unwrap();
        let reread = read_journal(&operation).unwrap();
        assert_eq!(reread.version, 3);
        assert!(
            reread.source_may_be_partial(),
            "carried through the rewrite"
        );

        for (bad, why) in [
            (
                serde_json::json!({"version": 2, "phase": "Retired"}),
                "only version 3",
            ),
            (
                serde_json::json!({"version": 4}),
                "unsupported move journal version 4",
            ),
        ] {
            write(&|journal| {
                journal.remove("host");
                for (key, value) in bad.as_object().unwrap() {
                    journal.insert(key.clone(), value.clone());
                }
            });
            let error = format!("{:#}", read_journal(&operation).unwrap_err());
            assert!(error.contains(why), "expected '{why}', got: {error}");
        }
    }

    #[test]
    fn validate_refuses_link_entries_that_do_not_hold_together() {
        let entry = |kind, bytes, link_target: Option<&str>| ManifestEntry {
            path: PathBuf::from("entry"),
            kind,
            bytes,
            source_modified: ModifiedTime::from_system_time(UNIX_EPOCH),
            link_target: link_target.map(PathBuf::from),
        };
        let manifest = |version, entry| MoveManifest {
            version,
            entries: vec![entry],
        };
        manifest(2, entry(ManifestKind::Symlink, 0, Some("target")))
            .validate()
            .unwrap();
        for (bad, why) in [
            (
                manifest(1, entry(ManifestKind::Symlink, 0, Some("target"))),
                "cannot hold a link",
            ),
            (
                manifest(2, entry(ManifestKind::Junction, 0, None)),
                "has no target",
            ),
            (
                manifest(2, entry(ManifestKind::File, 3, Some("target"))),
                "is not a link",
            ),
            (
                manifest(2, entry(ManifestKind::Directory, 3, None)),
                "non-zero byte length",
            ),
            (
                manifest(2, entry(ManifestKind::Symlink, 6, Some("target"))),
                "non-zero byte length",
            ),
            (
                manifest(3, entry(ManifestKind::File, 0, None)),
                "unsupported move manifest version 3",
            ),
        ] {
            let error = bad.validate().unwrap_err().to_string();
            assert!(error.contains(why), "expected '{why}', got: {error}");
        }
    }
}
