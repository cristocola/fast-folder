//! What a cross-drive move can learn before it copies anything.
//!
//! **A courtesy, not a guarantee.** Correctness rests on verification and on
//! the atomic retire (`move_cleanup`): a move that cannot take its source out
//! of the library after copying keeps the source whole and says so. What this
//! saves is the copy — sixty gigabytes across a network — that could never have
//! finished, and the half-state it leaves until someone runs reconcile.
//!
//! Each check asks the filesystem the real question in the real place rather
//! than reading permission bits, which mean nothing on a network mount whose
//! server decides: can fastf write and rename in the source base, and does the
//! source base show a link as a link?

use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

/// The source base's probe folder, named by the operation, until it is removed.
/// Dot-prefixed, so discovery never lists it.
pub const PROBE_PREFIX: &str = ".fastf-probe-";
/// The names a probe folder holds. Reconcile removes exactly these and nothing
/// else, so a folder that merely shares the prefix is never emptied.
const PROBE_FILE: &str = "f";
const PROBE_LINK: &str = "l";

/// The probe folder for `operation` in `source_base`.
pub fn probe_path(source_base: &Path, operation_id: &str) -> PathBuf {
    source_base.join(format!("{PROBE_PREFIX}{operation_id}"))
}

/// What renaming the probe folder adds to its name.
const RENAMED_SUFFIX: &str = "-renamed";

/// What renaming the probe folder makes of it.
fn renamed_probe(probe: &Path) -> PathBuf {
    let mut name = probe.file_name().unwrap_or_default().to_os_string();
    name.push(RENAMED_SUFFIX);
    probe.with_file_name(name)
}

/// The operation a probe folder is named by, if `name` is one a move writes
/// — the prefix, an operation id, and perhaps the renamed suffix.
pub fn probe_operation(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(PROBE_PREFIX)?;
    let operation = rest.strip_suffix(RENAMED_SUFFIX).unwrap_or(rest);
    crate::core::transactions::is_operation_id(operation).then_some(operation)
}

/// What the filesystem showed of a link the probe made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkProbe {
    /// The filesystem would not make one: it may hold none, which is fine.
    NotCreated,
    SeenAsLink,
    /// A link fastf just made, to a file beside it, reads as something else.
    SeenAsSomethingElse,
    /// A link fastf just made cannot even be examined.
    Unexaminable,
}

/// What the probe saw, from what making the link reported and what `lstat`
/// then found at its path.
///
/// **What is on disk wins over what the call said.** On an sshfs mount with
/// `follow_symlinks`, `symlink()` makes the link on the server and then fails
/// with `EIO`, because the mount reads the new entry back as the file it
/// points to and the kernel expected a link. Taking that error for "cannot
/// make links" let the incident's own mount through; something at the path
/// that is not a link is the answer.
pub(crate) fn observe(made: std::io::Result<()>, found: std::io::Result<bool>) -> LinkProbe {
    match found {
        Ok(true) => LinkProbe::SeenAsLink,
        Ok(false) => LinkProbe::SeenAsSomethingElse,
        Err(error) if made.is_err() && error.kind() == std::io::ErrorKind::NotFound => {
            LinkProbe::NotCreated
        }
        Err(_) => LinkProbe::Unexaminable,
    }
}

/// Whether what the probe saw means the filesystem resolves links itself —
/// sshfs's `follow_symlinks` does, on the server — so a link in a project
/// there looks like what it points to. A move would copy the target's content
/// in its place and then, removing the original, delete through it.
pub(crate) fn resolves_links(seen: LinkProbe) -> bool {
    matches!(
        seen,
        LinkProbe::SeenAsSomethingElse | LinkProbe::Unexaminable
    )
}

/// Before a move copies anything: can it take the source out of
/// `source_base` once it has? Creates, links, renames and removes a small
/// folder there — the very operations the retire will need.
pub(crate) fn probe_source_base(
    source_base: &Path,
    source: &Path,
    operation_id: &str,
) -> Result<()> {
    let probe = probe_path(source_base, operation_id);
    let renamed = renamed_probe(&probe);
    let outcome = run_probe(&probe, &renamed).and_then(|()| {
        // The probe is there, not yet removed: what a crash leaves for
        // reconcile to clear.
        crate::util::faults::check("move:after-probe").map_err(|error| format!("{error:#}"))
    });
    let _ = clear_probe(&probe);
    let cleared = clear_probe(&renamed);
    if let Err(refusal) = outcome {
        bail!("{refusal}");
    }
    // A base that lets fastf make and rename but not remove would take the
    // original out of the library and then keep it there for good.
    if let Err(why) = cleared {
        bail!(
            "a move removes the original after copying it, and fastf can write and rename in \
             {} but not remove there: its probe folder {why}. Nothing was copied.",
            crate::util::paths::display_path(source_base)
        );
    }
    if let Some(refusal) = sticky_refusal(source_base, source) {
        bail!("{refusal}");
    }
    Ok(())
}

fn run_probe(probe: &Path, renamed: &Path) -> std::result::Result<(), String> {
    let base = probe.parent().unwrap_or(probe);
    let cannot_write = |error: std::io::Error| {
        format!(
            "a move removes the original after copying it, so it needs to write in {}, \
             and it cannot: {error}. Nothing was copied.",
            crate::util::paths::display_path(base)
        )
    };
    fs::create_dir(probe).map_err(cannot_write)?;
    let seen = read_back_a_link(probe).map_err(cannot_write)?;
    if resolves_links(seen) {
        return Err(format!(
            "{}, then move again. Nothing was copied.",
            hidden_links_refusal(base)
        ));
    }
    fs::rename(probe, renamed).map_err(cannot_write)?;
    Ok(())
}

/// Make a file and a link to it in `probe`, which exists, and read the link
/// back.
fn read_back_a_link(probe: &Path) -> std::io::Result<LinkProbe> {
    fs::write(probe.join(PROBE_FILE), b"fastf")?;
    let link = probe.join(PROBE_LINK);
    let made = make_link(Path::new(PROBE_FILE), &link);
    let found = fs::symlink_metadata(&link).map(|metadata| metadata.file_type().is_symlink());
    Ok(observe(made, found))
}

/// Why nothing may be removed in `base`; the caller says what to do next.
fn hidden_links_refusal(base: &Path) -> String {
    format!(
        "the filesystem holding {} shows a link as what it points to — an sshfs mount with \
         `follow_symlinks` does this — so fastf cannot tell a link from what it points to \
         there, and removing anything could delete through one. Mount it without that option",
        crate::util::paths::display_path(base)
    )
}

/// Whether removing things in `base` could delete through a link fastf cannot
/// see — asked before **every** removal, not only a move's: `fastf delete` and
/// reconcile's removals walk a tree the same way. `None` when links show as
/// links, and when the base cannot be asked (a read-only one will refuse the
/// removal on its own).
pub(crate) fn links_hidden_in(base: &Path) -> Option<String> {
    let probe = probe_path(base, &crate::core::transactions::next_operation_id());
    fs::create_dir(&probe).ok()?;
    let seen = read_back_a_link(&probe);
    let _ = clear_probe(&probe);
    match seen {
        Ok(seen) if resolves_links(seen) => Some(hidden_links_refusal(base)),
        _ => None,
    }
}

#[cfg(unix)]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(any(unix, windows)))]
fn make_link(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Remove a probe folder: the names a probe holds, then the folder. Anything
/// else in it stays, and so does the folder — it is not only fastf's then.
/// The error says which of the two kept it.
pub(crate) fn clear_probe(probe: &Path) -> std::result::Result<(), String> {
    if fs::symlink_metadata(probe).is_err() {
        return Ok(());
    }
    for name in [PROBE_LINK, PROBE_FILE] {
        match crate::util::fs_retry::remove_file(&probe.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("could not be removed ({error})")),
        }
    }
    match crate::util::fs_retry::remove_dir(probe) {
        Ok(()) => Ok(()),
        Err(_) if fs::read_dir(probe).is_ok_and(|mut entries| entries.next().is_some()) => {
            Err("holds something fastf did not put there".to_string())
        }
        Err(error) => Err(format!("could not be removed ({error})")),
    }
}

/// In a folder with the sticky bit set (`/tmp`'s kind), only the owner of an
/// entry — or of the folder — may rename it. A shared base holding another
/// user's project folder lets fastf copy it and then never take it out.
#[cfg(unix)]
fn sticky_refusal(base: &Path, source: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let base_metadata = fs::metadata(base).ok()?;
    if base_metadata.mode() & 0o1000 == 0 {
        return None;
    }
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    let source_owner = fs::symlink_metadata(source).ok()?.uid();
    if me == 0 || base_metadata.uid() == me || source_owner == me {
        return None;
    }
    Some(format!(
        "{} is shared (its sticky bit is set) and the project folder belongs to another \
         user, so fastf could copy it but never take it out of the library. Nothing was \
         copied.",
        crate::util::paths::display_path(base)
    ))
}

#[cfg(not(unix))]
fn sticky_refusal(_base: &Path, _source: &Path) -> Option<String> {
    None
}

/// Refuse a copy that cannot fit, when the filesystem says how much room it
/// has. An answer it cannot give never refuses (`util::disk_space`).
pub(crate) fn check_space(target_base: &Path, needed: u64) -> Result<()> {
    if let Some(free) = crate::util::disk_space::available(target_base)
        && needed > free
    {
        bail!(
            "{} has {} free, and the project needs {}. Nothing was copied.",
            crate::util::paths::display_path(target_base),
            crate::util::human_bytes::human_bytes(free),
            crate::util::human_bytes::human_bytes(needed)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link the filesystem shows as a file is the sshfs `follow_symlinks`
    /// case, and a link it cannot examine is no better; a filesystem that
    /// makes no links at all holds none to hide.
    #[test]
    fn what_is_at_the_path_decides_not_what_the_call_said() {
        use std::io::{Error, ErrorKind};
        let eio = || Err(Error::from(ErrorKind::Other));
        let absent = || Err(Error::from(ErrorKind::NotFound));
        // sshfs `follow_symlinks`: made on the server, EIO to the caller, and
        // what is at the path is the file it points to.
        assert_eq!(observe(eio(), Ok(false)), LinkProbe::SeenAsSomethingElse);
        assert_eq!(observe(Ok(()), Ok(false)), LinkProbe::SeenAsSomethingElse);
        assert_eq!(observe(Ok(()), Ok(true)), LinkProbe::SeenAsLink);
        // A filesystem that makes no links: refused, and nothing there.
        assert_eq!(observe(eio(), absent()), LinkProbe::NotCreated);
        // Made, and then not there to examine.
        assert_eq!(observe(Ok(()), absent()), LinkProbe::Unexaminable);
    }

    #[test]
    fn only_a_link_shown_as_something_else_means_links_are_resolved() {
        assert!(!resolves_links(LinkProbe::NotCreated));
        assert!(!resolves_links(LinkProbe::SeenAsLink));
        assert!(resolves_links(LinkProbe::SeenAsSomethingElse));
        assert!(resolves_links(LinkProbe::Unexaminable));
    }

    /// The probe leaves nothing behind, on an ordinary filesystem it passes,
    /// and the names it clears are its own.
    #[test]
    fn a_probe_passes_on_an_ordinary_folder_and_leaves_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("project");
        fs::create_dir(&source).unwrap();
        probe_source_base(temp.path(), &source, "18d863aff116f53c-1-1").unwrap();
        let left: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("project")]);
    }

    #[test]
    fn only_names_a_probe_is_given_are_probes() {
        assert_eq!(
            probe_operation(".fastf-probe-18d863af-1-2"),
            Some("18d863af-1-2")
        );
        assert_eq!(
            probe_operation(".fastf-probe-18d863af-1-2-renamed"),
            Some("18d863af-1-2")
        );
        assert_eq!(probe_operation(".fastf-probe-Scenes.fastf-case"), None);
        assert_eq!(probe_operation(".fastf-probe-"), None);
    }

    #[test]
    fn clearing_a_probe_keeps_what_is_not_a_probes() {
        let temp = tempfile::tempdir().unwrap();
        let probe = probe_path(temp.path(), "18d863aff116f53c-1-2");
        fs::create_dir(&probe).unwrap();
        fs::write(probe.join(PROBE_FILE), b"fastf").unwrap();
        fs::write(probe.join("somebody-elses.txt"), b"keep").unwrap();
        assert!(
            clear_probe(&probe)
                .unwrap_err()
                .contains("did not put there")
        );
        assert_eq!(fs::read(probe.join("somebody-elses.txt")).unwrap(), b"keep");
        assert!(!probe.join(PROBE_FILE).exists());
    }

    /// Room is only refused when the filesystem says how much it has.
    #[test]
    fn a_copy_that_cannot_fit_is_refused_by_the_numbers() {
        let temp = tempfile::tempdir().unwrap();
        check_space(temp.path(), 1).unwrap();
        let error = check_space(temp.path(), u64::MAX).unwrap_err().to_string();
        assert!(
            error.contains("free") && error.contains("Nothing was copied"),
            "{error}"
        );
    }
}
