//! The manifest: what a project holds, entry by entry, and the one comparison of two of them.

use super::*;

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

    /// A time `nanos` after the epoch, for tests that build entries by hand.
    #[cfg(test)]
    pub(crate) fn from_nanos_for_test(nanos: i64) -> Self {
        Self {
            before_epoch: nanos < 0,
            seconds: nanos.unsigned_abs() / 1_000_000_000,
            nanoseconds: (nanos.unsigned_abs() % 1_000_000_000) as u32,
        }
    }

    pub(super) fn from_system_time(value: SystemTime) -> Self {
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
    pub(super) version: u32,
    pub entries: Vec<ManifestEntry>,
}

impl MoveManifest {
    /// Scan exactly once before copying. Unsupported entries fail the entire
    /// move — every one of them named, not only the first — and links and
    /// special files are never followed or silently omitted.
    pub fn scan(root: &Path) -> Result<Self> {
        Self::scan_with(root, Ticker::none())
    }

    /// [`Self::scan`], counting every entry and stopping on a cancel.
    pub fn scan_with(root: &Path, ticker: Ticker) -> Result<Self> {
        let entries = Walk::of_with(root, "move source", ticker)?.into_entries(root)?;
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
        self.verify_destination_with(destination, Ticker::none())
    }

    /// [`Self::verify_destination`], counting every entry it walks.
    pub fn verify_destination_with(&self, destination: &Path, ticker: Ticker) -> Result<Walk> {
        let walk = Walk::of_with(destination, "move destination", ticker)?;
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
        self.verify_source_unchanged_with(source, Ticker::none())
    }

    /// [`Self::verify_source_unchanged`], counting every entry it walks.
    pub fn verify_source_unchanged_with(&self, source: &Path, ticker: Ticker) -> Result<()> {
        let diff = self.compare(&Walk::of_with(source, "move source", ticker)?, Match::Exact);
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
    pub fn verify_recovery_pair(
        &self,
        source: &Path,
        destination: &Path,
        ticker: Ticker,
    ) -> Result<()> {
        self.verify_source_unchanged_with(source, ticker)?;
        self.verify_destination_with(destination, ticker).map(drop)
    }

    /// The root `PROJECT_INFO.md`, which is written last: it is what makes the
    /// copy a project.
    pub fn root_metadata(&self) -> Option<&ManifestEntry> {
        self.entry(Path::new(crate::core::project_info::RESERVED_FILENAME))
    }

    /// This manifest without the root `PROJECT_INFO.md`: everything that is
    /// copied before the publish, and what the copy is verified against then.
    pub fn without_root_metadata(&self) -> MoveManifest {
        MoveManifest {
            version: self.version,
            entries: self
                .entries
                .iter()
                .filter(|entry| {
                    entry.path != Path::new(crate::core::project_info::RESERVED_FILENAME)
                })
                .cloned()
                .collect(),
        }
    }

    /// Only the root `PROJECT_INFO.md`, for the publish.
    pub fn only_root_metadata(&self) -> MoveManifest {
        MoveManifest {
            version: self.version,
            entries: self.root_metadata().cloned().into_iter().collect(),
        }
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
pub(super) fn difference(
    recorded: &ManifestEntry,
    found: &ManifestEntry,
    rule: Match,
) -> Option<String> {
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

impl MoveManifest {
    /// This record without the entries at `paths`: what is left to copy
    /// once a paused copy's are adopted.
    pub fn without_paths(&self, paths: &HashSet<PathBuf>) -> Self {
        Self {
            version: self.version,
            entries: self
                .entries
                .iter()
                .filter(|entry| !paths.contains(&entry.path))
                .cloned()
                .collect(),
        }
    }
}

impl MoveManifest {
    /// The same record, with each of `copied` — a file as it was when it was
    /// copied — in place of what the scan saw at its path.
    pub fn with_copied(mut self, copied: Vec<ManifestEntry>) -> Self {
        let copied: HashMap<PathBuf, ManifestEntry> = copied
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect();
        for entry in &mut self.entries {
            if let Some(as_copied) = copied.get(&entry.path) {
                *entry = as_copied.clone();
            }
        }
        self
    }

    /// This record with `entry` — the root `PROJECT_INFO.md` a settled copy's
    /// last look found — in its place, sorted as a walk sorts.
    pub fn with_entry(mut self, entry: Option<ManifestEntry>) -> Self {
        if let Some(entry) = entry {
            self.entries.retain(|existing| existing.path != entry.path);
            self.entries.push(entry);
            self.entries
                .sort_by(|left, right| left.path.cmp(&right.path));
        }
        self
    }
}
