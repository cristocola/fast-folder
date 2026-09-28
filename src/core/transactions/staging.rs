//! The copy as a whole: names first, then contents, then settled against an
//! original that may still move.

use super::*;

/// Copy from a manifest with one reusable bounded buffer. Files are written
/// directly into private staging, so no sibling `.part` convention exists.
///
/// **Every folder and every link first, then each file once.** A folder the
/// target's filesystem will not hold — two whose names differ only in case on
/// a drive that ignores it, a `:` on one that forbids it — is found before a
/// byte of content moves, and so is a clash between two *file* names, by
/// asking the target once whether it ignores case and then reading the record
/// ([`case_clashes`]). Each file is then written exactly once: making every
/// file empty first and filling it in a second pass, as 3.12.0 did, wrote each
/// one twice, which on a cloud mount is two uploads — the second cancelling
/// the first, a thousand times over — and on one that caches nothing, a second
/// open it refuses.
pub fn copy_to_staging(
    manifest: &MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<Vec<ManifestEntry>> {
    crate::util::paths::require_real_directory(source, "move source")?;
    crate::util::paths::require_real_directory(staging, "move staging")?;
    manifest.validate()?;
    if target_ignores_case(staging) {
        let clashes = case_clashes(manifest);
        if !clashes.is_empty() {
            let count = clashes.len();
            let mut message = format!(
                "the filesystem where the project is going ignores case, and the project \
                 holds {count} {} that differ only in case:",
                if count == 1 {
                    "pair of names"
                } else {
                    "pairs of names"
                }
            );
            for (one, other) in clashes.iter().take(LISTED) {
                message.push_str(&format!("\n  {} and {}", one.display(), other.display()));
            }
            if count > LISTED {
                message.push_str(&format!("\n  and {} more", count - LISTED));
            }
            message.push_str("\nNothing was copied.");
            bail!("{message}");
        }
    }
    create_names(manifest, staging, cancel)?;
    copy_contents(manifest, source, staging, progress, cancel)
}

/// Take over the copy a paused move left at `staging`: every entry that is
/// already what `body` records — a folder, a link to the same target, a file
/// of the same size and time, which only a finished copy has, since each
/// file's time is set last — is kept; anything else there is removed. Answers
/// what was kept, as copied, so only the rest is copied again: a mount that
/// dropped at 59 of 60 gigabytes costs one.
pub fn adopt_staging(body: &MoveManifest, staging: &Path) -> Result<Vec<ManifestEntry>> {
    crate::util::paths::require_real_directory(staging, "the paused copy")?;
    let there = Walk::of(staging, "the paused copy")?;
    let recorded: HashMap<&Path, &ManifestEntry> = body
        .entries
        .iter()
        .map(|entry| (entry.path.as_path(), entry))
        .collect();
    let mut kept = Vec::new();
    let mut stale: Vec<&ManifestEntry> = Vec::new();
    for entry in &there.entries {
        let keep = recorded.get(entry.path.as_path()).is_some_and(|wanted| {
            wanted.kind == entry.kind
                && match entry.kind {
                    ManifestKind::Directory => true,
                    ManifestKind::File => {
                        wanted.bytes == entry.bytes
                            && wanted.source_modified == entry.source_modified
                    }
                    _ => wanted.link_target == entry.link_target,
                }
        });
        if keep {
            kept.push(entry.clone());
        } else {
            stale.push(entry);
        }
    }
    // Deepest first, so a folder is emptied before it goes.
    stale.sort_by_key(|entry| std::cmp::Reverse(entry.path.components().count()));
    for entry in stale {
        // A folder kept by an ancestor that was not is already gone.
        let path = staging.join(&entry.path);
        let removed = if entry.kind == ManifestKind::Directory {
            match crate::core::removal::remove_tree(
                &path,
                None,
                crate::core::removal::Purpose::Delete,
                Ticker::none(),
            ) {
                crate::core::removal::Removal::Removed => Ok(()),
                crate::core::removal::Removal::Leftover { reason, .. } => {
                    Err(anyhow::anyhow!("{reason}"))
                }
            }
        } else {
            match crate::util::fs_retry::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            }
        };
        removed
            .with_context(|| format!("clearing {} from the paused copy", entry.path.display()))?;
    }
    // Anything the walk could not take is not part of a copy fastf made.
    if let Some(problem) = there.problems.first() {
        bail!(
            "the paused copy at {} holds something fastf did not put there: {}: {}",
            staging.display(),
            problem.path.display(),
            problem.problem
        );
    }
    kept.retain(|entry| {
        entry
            .path
            .ancestors()
            .skip(1)
            .all(|ancestor| ancestor.as_os_str().is_empty() || staging.join(ancestor).is_dir())
    });
    Ok(kept)
}

/// **What a move and a copy both do once the body is copied**, in the one
/// order that keeps the record true: settle the copy against the original
/// ([`settle_copy`]), write the manifest — what the copy holds, with the
/// `PROJECT_INFO.md` the publish will write — and only then verify the copy
/// against it. Answers that manifest, and the walk of the verified copy.
pub fn settle_record_verify(
    transaction: &MoveTransaction,
    body: &mut MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
    ticker: Ticker,
) -> Result<(MoveManifest, Walk)> {
    let root = settle_copy(body, source, staging, progress, cancel, ticker)?;
    let manifest = body.clone().with_entry(root);
    manifest.validate()?;
    transaction.write_manifest(&manifest)?;
    let staged = body.verify_destination_with(staging, ticker)?;
    Ok((manifest, staged))
}

/// **The publish**: the root `PROJECT_INFO.md`, written once, last, which is
/// what makes the copy a project. No folder on the target is renamed, and the
/// copy is handed a flag nobody sets: once it starts, it finishes.
pub fn publish(
    manifest: &MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
) -> Result<Vec<ManifestEntry>> {
    copy_to_staging(
        &manifest.only_root_metadata(),
        source,
        staging,
        progress,
        &AtomicBool::new(false),
    )
}

/// How many times [`settle_copy`] catches the copy up with the original
/// before it publishes what the copy holds.
pub const SETTLE_ROUNDS: usize = 3;

/// Bring the copy at `staging` level with the original at `source` as it is
/// now, and `body` — the record of what the copy holds, everything but the
/// root `PROJECT_INFO.md` — with it: re-copy what changed since it was
/// copied, copy what is new, remove what is gone, and look again, until a
/// look finds nothing the copy does not hold, or [`SETTLE_ROUNDS`] rounds have
/// caught up. Answers the root `PROJECT_INFO.md` as the last look found it.
///
/// **A change made to a project while it moves is kept.** 3.13 compared the
/// original with its scan once, after the whole copy, and a dev server's log
/// line or a temp folder it made and removed failed the move and threw the
/// copy away. **An original that never holds still does not stop the move
/// either** — a dev server appends to its log every tenth of a second: after
/// the last round the copy is published as it is, consistent with its record,
/// and what changes in the original from then on is carried into the moved
/// copy by the merge after the retire (`core::merge`, `Policy::Full`).
pub fn settle_copy(
    body: &mut MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
    ticker: Ticker,
) -> Result<Option<ManifestEntry>> {
    let root_info = Path::new(crate::core::project_info::RESERVED_FILENAME);
    for round in 0..=SETTLE_ROUNDS {
        #[cfg(test)]
        BEFORE_EACH_LOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook();
            }
        });
        let mut now = Walk::of_with(source, "move source", ticker)?;
        let root_entry = now
            .entries
            .iter()
            .position(|entry| entry.path == root_info)
            .map(|at| now.entries.remove(at));
        let diff = body.compare(&now, Match::Whole);
        if !now.problems.is_empty() {
            // Something appeared that no copy can hold: the scan's refusal.
            now.clone().into_entries(source)?;
        }
        if diff.is_clean() {
            return Ok(root_entry);
        }
        if round == SETTLE_ROUNDS {
            crate::util::log::info(format!(
                "the original was still changing after {SETTLE_ROUNDS} rounds; the copy is \
                 published as it is, and the merge after the retire carries the rest: {}",
                diff.summary(LISTED).replace('\n', ";")
            ));
            return Ok(root_entry);
        }
        crate::util::log::info(format!(
            "the original changed while it was copied; copying the difference: {}",
            diff.summary(LISTED).replace('\n', ";")
        ));
        catch_up(body, &now, &diff, source, staging, progress, cancel)?;
    }
    unreachable!("the last round returns or refuses")
}

#[cfg(test)]
thread_local! {
    /// Run before each of [`settle_copy`]'s looks: a test's way to be the
    /// program that keeps writing the project, deterministically.
    pub(super) static BEFORE_EACH_LOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        std::cell::RefCell::new(None);
}

/// One round of [`settle_copy`]: make the copy hold what `now` found.
fn catch_up(
    body: &mut MoveManifest,
    now: &Walk,
    diff: &ManifestDiff,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<()> {
    let found: HashMap<&Path, &ManifestEntry> = now
        .entries
        .iter()
        .map(|entry| (entry.path.as_path(), entry))
        .collect();
    // What goes first: everything gone, and everything that changed, since
    // a changed entry is made again from what is there now.
    let mut gone: Vec<&Path> = diff
        .missing
        .iter()
        .map(PathBuf::as_path)
        .chain(diff.changed.iter().map(|(path, _)| path.as_path()))
        .collect();
    gone.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in gone {
        let staged = staging.join(path);
        let removed = match fs::symlink_metadata(&staged) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                match crate::core::removal::remove_tree(
                    &staged,
                    None,
                    crate::core::removal::Purpose::Delete,
                    Ticker::none(),
                ) {
                    crate::core::removal::Removal::Removed => Ok(()),
                    crate::core::removal::Removal::Leftover { reason, .. } => {
                        Err(anyhow::anyhow!("{reason}"))
                    }
                }
            }
            Ok(_) => crate::util::fs_retry::remove_file(&staged).map_err(anyhow::Error::from),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
        removed.with_context(|| format!("replacing {} in the copy", path.display()))?;
    }
    // Then everything new or changed, as a manifest of its own: folders and
    // links first, then the files, exactly as the first copy did.
    let fresh = MoveManifest {
        version: MANIFEST_VERSION,
        entries: diff
            .added
            .iter()
            .chain(diff.changed.iter().map(|(path, _)| path))
            .filter_map(|path| found.get(path.as_path()).map(|entry| (*entry).clone()))
            .collect(),
    };
    create_names(&fresh, staging, cancel)?;
    let copied = copy_contents(&fresh, source, staging, progress, cancel)?;
    // The record is now what was found, with each file as it was copied.
    let copied: HashMap<PathBuf, ManifestEntry> = copied
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect();
    body.entries = now
        .entries
        .iter()
        .map(|entry| {
            copied
                .get(&entry.path)
                .cloned()
                .unwrap_or_else(|| entry.clone())
        })
        .collect();
    Ok(())
}

/// Whether the filesystem holding `staging` — a folder fastf made empty a
/// moment ago — takes two names that differ only in case for one.
pub(super) fn target_ignores_case(staging: &Path) -> bool {
    let upper = staging.join(".Fastf-Case-Probe");
    let lower = staging.join(".fastf-case-probe");
    let Ok(()) = fs::write(&upper, b"") else {
        return false;
    };
    let ignores = fs::symlink_metadata(&lower).is_ok();
    let _ = fs::remove_file(&upper);
    ignores
}

/// Pairs of recorded names that a filesystem ignoring case would take for
/// one: the same folder, the same name once lowercased.
pub fn case_clashes(manifest: &MoveManifest) -> Vec<(PathBuf, PathBuf)> {
    let mut seen: HashMap<(PathBuf, String), &Path> = HashMap::new();
    let mut clashes = Vec::new();
    for entry in &manifest.entries {
        let parent = entry
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let name = entry
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        match seen.get(&(parent.clone(), name.clone())) {
            Some(first) => clashes.push((first.to_path_buf(), entry.path.clone())),
            None => {
                seen.insert((parent, name), entry.path.as_path());
            }
        }
    }
    clashes
}

/// The first pass: every folder, and — last, so no later write can pass
/// through one — every link. Folders are made a level at a time, each level on
/// the pool, so a parent is always there before its children; links, which
/// hold nothing, all at once.
fn create_names(manifest: &MoveManifest, staging: &Path, cancel: &AtomicBool) -> Result<()> {
    let width = crate::util::pool::width_for(staging);
    let refused: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());
    let make = |entry: &ManifestEntry| -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            bail!("move cancelled");
        }
        // Beneath a folder that could not be made, nothing can be; its own
        // refusal says why.
        if refused
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .any(|(path, _)| entry.path.starts_with(path))
        {
            return Ok(());
        }
        let destination = staging.join(&entry.path);
        let made = match entry.kind {
            ManifestKind::Directory => fs::create_dir(&destination),
            ManifestKind::File => Ok(()),
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
            refused
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((entry.path.clone(), why));
        }
        Ok(())
    };
    let mut levels: Vec<Vec<&ManifestEntry>> = Vec::new();
    for entry in manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == ManifestKind::Directory)
    {
        let depth = entry.path.components().count();
        if levels.len() < depth {
            levels.resize_with(depth, Vec::new);
        }
        levels[depth - 1].push(entry);
    }
    for level in levels {
        crate::util::pool::run(width, level, make)?;
    }
    let links: Vec<&ManifestEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind.is_link())
        .collect();
    crate::util::pool::run(width, links, make)?;
    let mut refused = refused
        .into_inner()
        .unwrap_or_else(|error| error.into_inner());
    if refused.is_empty() {
        return Ok(());
    }
    refused.sort();
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
pub(crate) fn make_link(kind: ManifestKind, target: &Path, link: &Path) -> std::io::Result<()> {
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
    match error.raw_os_error() {
        Some(libc::EPERM) | Some(libc::EOPNOTSUPP) | Some(libc::ENOSYS) => {
            return "the filesystem there cannot hold links".to_string();
        }
        // What an rclone mount answers for a link unless it was mounted with
        // `--links`, which stores each one as a `.rclonelink` file.
        Some(libc::EIO) => {
            return "the filesystem there could not make a link (Input/output error); an \
                    rclone mount holds links only when mounted with `--links`"
                .to_string();
        }
        _ => {}
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
