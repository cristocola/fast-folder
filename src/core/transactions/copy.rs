//! One file's copy, and the attributes a copy keeps.

use super::*;

/// The second pass: each file, written once, into a folder the first pass
/// made — created new, and opened without following a link, though none can
/// be there yet. A name the target will not take is found here, still before
/// anything is published. Files are copied on the pool, as many at once as the
/// slower of the two filesystems is worth.
pub(super) fn copy_contents(
    manifest: &MoveManifest,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<Vec<ManifestEntry>> {
    let files: Vec<&ManifestEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == ManifestKind::File)
        .collect();
    let width = crate::util::pool::width_for_pair(source, staging);
    let copied = Mutex::new(Vec::with_capacity(files.len()));
    crate::util::pool::run(width, files, |entry| {
        if cancel.load(Ordering::Relaxed) {
            bail!("move cancelled");
        }
        if let Some(as_copied) = copy_file_again(entry, source, staging, progress, cancel)? {
            copied
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(as_copied);
        }
        Ok(())
    })?;
    Ok(copied
        .into_inner()
        .unwrap_or_else(|error| error.into_inner()))
}

/// Copy one recorded file into place, **keeping its permission bits and its
/// times**, as `mv` does: both are set on the new file's own handle before it
/// is synced, so what is published is what was verified. 3.13 kept neither,
/// and every moved script lost its `+x`.
///
/// **The file is copied as it is now**, and the answer is what was copied —
/// its size and time from the handle it was read through — so a file changed
/// since the scan costs one copy, not a failed move ([`settle_copy`] looks
/// again). `None` when it is no longer a file there: gone, or something else
/// now, which the next look finds too.
fn copy_file(
    entry: &ManifestEntry,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<Option<ManifestEntry>> {
    let source_path = source.join(&entry.path);
    let destination_path = staging.join(&entry.path);
    crate::util::faults::check("move:each-file")?;
    if let Ok(mut state) = progress.lock() {
        state.current_file = entry.path.to_string_lossy().into_owned();
        state.touch();
    }
    let mut reading = OpenOptions::new();
    reading.read(true);
    // Never through a link that took the file's place since the scan.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        reading.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        reading.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut reader = match reading.open(&source_path) {
        Ok(reader) => reader,
        Err(error) if is_not_a_file_now(&error) => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("opening {}", source_path.display()));
        }
    };
    // What was opened, from its own handle: the mode and times to keep, and
    // what the copy is of.
    let original = reader
        .metadata()
        .with_context(|| format!("reading metadata for {}", source_path.display()))?;
    if !original.file_type().is_file() {
        return Ok(None);
    }
    let as_copied =
        ManifestEntry {
            path: entry.path.clone(),
            kind: ManifestKind::File,
            bytes: original.len(),
            source_modified: ModifiedTime::from_system_time(original.modified().with_context(
                || format!("reading modification time for {}", source_path.display()),
            )?),
            link_target: None,
        };
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut writer = options.open(&destination_path).map_err(|error| {
        anyhow::anyhow!(
            "{} cannot be made where the project is going: {}",
            entry.path.display(),
            name_refusal(&error)
        )
    })?;
    // Small files are most files: a buffer the size of the file, up to the
    // usual megabyte.
    let size = usize::try_from(as_copied.bytes)
        .unwrap_or(COPY_BUFFER_BYTES)
        .clamp(1, COPY_BUFFER_BYTES);
    let mut buffer = vec![0_u8; size.saturating_add(1).min(COPY_BUFFER_BYTES)];
    let mut copied: u64 = 0;
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
        crate::util::faults::check_io("copy:write")
            .and_then(|()| writer.write_all(&buffer[..count]))
            .with_context(|| format!("writing {}", destination_path.display()))?;
        copied = copied.saturating_add(count as u64);
        if let Ok(mut state) = progress.lock() {
            state.copied_bytes = state.copied_bytes.saturating_add(count as u64);
            state.touch();
        }
    }
    writer
        .flush()
        .with_context(|| format!("flushing {}", destination_path.display()))?;
    keep_attributes(&writer, &original);
    // A file written while it was read holds more (or fewer) bytes than its
    // handle said at the start: the record is what the copy holds, and its
    // time the older one, so the next look sees the difference and copies it
    // again.
    let as_copied = ManifestEntry {
        bytes: copied,
        ..as_copied
    };
    writer
        .sync_all()
        .with_context(|| format!("syncing {}", destination_path.display()))?;
    if let Ok(mut state) = progress.lock() {
        state.done_files += 1;
        state.step_done += 1;
        state.touch();
    }
    Ok(Some(as_copied))
}

/// [`copy_file`], again from the start when it fails the way a mount does
/// (`fs_retry::classify`): a transient error or a lock after a pause, a
/// mount that dropped once it is back. What was written of it goes first —
/// the file is written once, whole, or not at all.
fn copy_file_again(
    entry: &ManifestEntry,
    source: &Path,
    staging: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> Result<Option<ManifestEntry>> {
    use crate::util::fs_retry::ErrorClass;
    const PAUSES_MS: [u64; 6] = [200, 400, 800, 1600, 3200, 5000];
    let destination = staging.join(&entry.path);
    let deadline = std::time::Instant::now() + crate::util::fs_retry::mount_wait();
    // Read at the first "not connected", as `fs_retry::with_retry` does: not
    // once for every file copied.
    let mut mounts = None;
    let mut pauses = 0;
    loop {
        let error = match copy_file(entry, source, staging, progress, cancel) {
            Ok(copied) => return Ok(copied),
            Err(error) => error,
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(error);
        }
        match crate::util::fs_retry::class_of(&error) {
            Some(ErrorClass::Transient | ErrorClass::Locked) if pauses < PAUSES_MS.len() => {
                std::thread::sleep(std::time::Duration::from_millis(PAUSES_MS[pauses]));
                pauses += 1;
            }
            Some(ErrorClass::NotConnected) if std::time::Instant::now() < deadline => {
                let (at_source, at_staging) = mounts.get_or_insert_with(|| {
                    (
                        crate::util::fs_kind::mount_identity(source),
                        crate::util::fs_kind::mount_identity(staging),
                    )
                });
                crate::util::fs_retry::wait_for_mount(source, at_source.as_deref(), deadline);
                crate::util::fs_retry::wait_for_mount(staging, at_staging.as_deref(), deadline);
            }
            _ => return Err(error),
        }
        crate::util::log::info(format!(
            "copying {} again after: {error:#}",
            entry.path.display()
        ));
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_file() => {
                crate::util::fs_retry::remove_file(&destination)
                    .with_context(|| format!("removing a part-copied {}", destination.display()))?;
            }
            _ => {}
        }
    }
}

/// An open of a source file that says it is not a file there any more:
/// gone, a folder now, or a link now (`O_NOFOLLOW`'s `ELOOP`).
fn is_not_a_file_now(error: &std::io::Error) -> bool {
    if crate::util::paths::is_absence(error) {
        return true;
    }
    #[cfg(unix)]
    if matches!(error.raw_os_error(), Some(libc::ELOOP) | Some(libc::EISDIR)) {
        return true;
    }
    false
}

/// Give a copied file the permission bits and times of `original`, on its
/// own handle. Best effort: a filesystem that keeps neither (a FAT drive, an
/// object store without metadata) still holds the right bytes, and the move
/// never fails over this.
pub(crate) fn keep_attributes(file: &fs::File, original: &fs::Metadata) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = original.permissions().mode() & 0o777;
        let _ = file.set_permissions(fs::Permissions::from_mode(mode));
    }
    // The read-only attribute is Windows' permission bit.
    #[cfg(windows)]
    if original.permissions().readonly() {
        let mut permissions = original.permissions();
        permissions.set_readonly(true);
        let _ = file.set_permissions(permissions);
    }
    let mut times = fs::FileTimes::new();
    if let Ok(modified) = original.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = original.accessed() {
        times = times.set_accessed(accessed);
    }
    let _ = file.set_times(times);
}

/// Give every folder of a published copy the permission bits and times of its
/// original, deepest first: **after** the publish, because a folder's time
/// moves with every name written into it, and a folder without write
/// permission (Go's module cache is mode 555) would have refused its own
/// files. The project folder itself is left as it is: the move's bookkeeping
/// writes its `PROJECT_INFO.md` next. Best effort, like
/// [`keep_attributes`]: the move is done whatever this manages.
pub(crate) fn keep_folder_attributes(manifest: &MoveManifest, source: &Path, destination: &Path) {
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
    let width = crate::util::pool::width_for_pair(source, destination);
    for level in levels.into_iter().rev() {
        let _ = crate::util::pool::run(width, level, |entry| {
            if let Ok(original) = fs::symlink_metadata(source.join(&entry.path))
                && original.file_type().is_dir()
            {
                set_folder_attributes(&destination.join(&entry.path), &original);
            }
            Ok::<(), ()>(())
        });
    }
}

#[cfg(unix)]
fn set_folder_attributes(folder: &Path, original: &fs::Metadata) {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Ok(metadata) = fs::symlink_metadata(folder) else {
        return;
    };
    // Only a folder fastf made: never through a link something put there.
    if !metadata.file_type().is_dir() {
        return;
    }
    let Ok(name) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
        return;
    };
    let times = [
        libc::timespec {
            tv_sec: original.atime() as libc::time_t,
            tv_nsec: original.atime_nsec() as _,
        },
        libc::timespec {
            tv_sec: original.mtime() as libc::time_t,
            tv_nsec: original.mtime_nsec() as _,
        },
    ];
    // SAFETY: a NUL-terminated path this function owns, and two timespecs.
    unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            name.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        );
    }
    let mode = original.permissions().mode() & 0o7777;
    let _ = fs::set_permissions(folder, fs::Permissions::from_mode(mode));
}

#[cfg(windows)]
fn set_folder_attributes(folder: &Path, original: &fs::Metadata) {
    use std::os::windows::fs::OpenOptionsExt;
    // A folder opens only for backup semantics; the handle asks for nothing
    // but its attributes, and never follows a link.
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let Ok(handle) = OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(folder)
    else {
        return;
    };
    let Ok(metadata) = handle.metadata() else {
        return;
    };
    if !metadata.file_type().is_dir() {
        return;
    }
    let mut times = fs::FileTimes::new();
    if let Ok(modified) = original.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = original.accessed() {
        times = times.set_accessed(accessed);
    }
    let _ = handle.set_times(times);
}

#[cfg(not(any(unix, windows)))]
fn set_folder_attributes(_folder: &Path, _original: &fs::Metadata) {}
