//! What holds a folder on Windows: the Restart Manager names the programs
//! holding files in it, and a folder opened for delete answers "in use"
//! while a program works in it, which the Restart Manager cannot see.
//!
//! **Both are physical**: Windows will not rename a folder while anything
//! under it is open — a file, whatever its sharing, or a folder a console
//! works in; measured in the VM — nor remove a file a program holds, so the
//! old copy could never be set aside. Nothing overrides them.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::{By, How, Use};
use crate::core::transactions::{ManifestKind, MoveManifest};

/// How many files one Restart Manager session is asked about.
const BATCH: usize = 1000;
/// Held files named, at most: a refusal names the first, and bisecting a
/// batch costs a session per step.
const NAMED: usize = 8;

/// What holds anything in `root`: the entries a move's scan found, or the
/// tree walked when there is no scan.
pub(super) fn in_tree(root: &Path, entries: Option<&MoveManifest>) -> Vec<Use> {
    let (files, folders) = match entries {
        Some(manifest) => of_manifest(root, manifest),
        None => walk(root),
    };
    let mut held = Vec::new();
    let me = std::process::id();
    for batch in files.chunks(BATCH) {
        if held.len() >= NAMED {
            break;
        }
        name_holders(batch, me, &mut held);
    }
    let mut uses: Vec<Use> = held
        .into_iter()
        .map(|(pid, program, file)| Use {
            by: By::Process { pid, program },
            how: How::Held(relative(root, &file)),
        })
        .collect();
    // A folder is asked on every filesystem that is a disk here; across a
    // network each open is a round trip, and the project folder alone is
    // asked.
    let local = matches!(
        crate::util::fs_kind::of(root),
        crate::util::fs_kind::FsKind::Local
    );
    for folder in folders.iter().filter(|folder| local || *folder == root) {
        if in_use(folder) {
            uses.push(Use {
                by: By::Nobody,
                how: How::BusyFolder(relative(root, folder)),
            });
        }
    }
    uses
}

fn relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// The files and folders a scan found, as full paths, the project folder
/// first among the folders.
fn of_manifest(root: &Path, manifest: &MoveManifest) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    for entry in &manifest.entries {
        match entry.kind {
            ManifestKind::File => files.push(root.join(&entry.path)),
            ManifestKind::Directory => folders.push(root.join(&entry.path)),
            _ => {}
        }
    }
    (files, folders)
}

/// The same, walked: no link followed, as deep as any walk goes.
fn walk(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((folder, depth)) = stack.pop() {
        if depth > crate::util::paths::MAX_WALK_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if crate::util::paths::is_link_like(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                folders.push(path.clone());
                stack.push((path, depth + 1));
            } else if metadata.is_file() {
                files.push(path);
            }
        }
    }
    (files, folders)
}

/// Ask the Restart Manager who holds any of `files`; where somebody does,
/// halve the batch until each held file is named.
fn name_holders(files: &[PathBuf], me: u32, found: &mut Vec<(u32, String, PathBuf)>) {
    if found.len() >= NAMED {
        return;
    }
    let processes = restart_manager::holding(files);
    let processes: Vec<(u32, String)> = processes
        .into_iter()
        .filter(|(pid, program)| *pid != me && !program.to_lowercase().starts_with("fastf"))
        .collect();
    if processes.is_empty() {
        return;
    }
    if let [file] = files {
        for (pid, program) in processes {
            found.push((pid, program, file.clone()));
        }
        return;
    }
    let (first, second) = files.split_at(files.len() / 2);
    name_holders(first, me, found);
    name_holders(second, me, found);
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

/// A handle this module opened, closed on every way out.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by this module, is valid, and is
        // closed once, here.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

use crate::util::win::{
    ERROR_SHARING_VIOLATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_ALL, OPEN_EXISTING,
};

const DELETE: u32 = 0x0001_0000;

/// Whether a program has `folder` open without letting it be deleted — a
/// console working there does. Only "in use" counts: a folder fastf may not
/// open is the probe's to say, not this.
fn in_use(folder: &Path) -> bool {
    let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: a NUL-terminated path fastf owns; null security and template
    // are allowed; the handle is closed by `Handle`'s drop.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            DELETE,
            FILE_SHARE_ALL,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle as isize == -1 {
        return std::io::Error::last_os_error().raw_os_error() == Some(ERROR_SHARING_VIOLATION);
    }
    drop(Handle(handle));
    false
}

/// The Restart Manager, declared by hand (`rstrtmgr.dll`, part of Windows
/// since Vista, so the standalone check allows it).
mod restart_manager {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct UniqueProcess {
        pid: u32,
        started: FileTime,
    }

    const APP_NAME: usize = 255 + 1;
    const SERVICE_NAME: usize = 63 + 1;
    const SESSION_KEY: usize = 32 + 1;
    const ERROR_MORE_DATA: u32 = 234;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ProcessInfo {
        process: UniqueProcess,
        app_name: [u16; APP_NAME],
        service_name: [u16; SERVICE_NAME],
        application_type: i32,
        app_status: u32,
        session_id: u32,
        restartable: i32,
    }

    impl ProcessInfo {
        /// All zeros, as the call wants its buffer.
        fn empty() -> Self {
            Self {
                process: UniqueProcess::default(),
                app_name: [0; APP_NAME],
                service_name: [0; SERVICE_NAME],
                application_type: 0,
                app_status: 0,
                session_id: 0,
                restartable: 0,
            }
        }
    }

    #[link(name = "rstrtmgr")]
    unsafe extern "system" {
        fn RmStartSession(session: *mut u32, flags: u32, key: *mut u16) -> u32;
        fn RmRegisterResources(
            session: u32,
            files: u32,
            file_names: *const *const u16,
            applications: u32,
            processes: *const UniqueProcess,
            services: u32,
            service_names: *const *const u16,
        ) -> u32;
        fn RmGetList(
            session: u32,
            needed: *mut u32,
            count: *mut u32,
            processes: *mut ProcessInfo,
            reasons: *mut u32,
        ) -> u32;
        fn RmEndSession(session: u32) -> u32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(
            handle: *mut c_void,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
    }

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    /// A Restart Manager session, ended on every way out.
    struct Session(u32);

    impl Drop for Session {
        fn drop(&mut self) {
            // SAFETY: a session `RmStartSession` opened, ended once, here.
            unsafe {
                RmEndSession(self.0);
            }
        }
    }

    /// Every process holding any of `files`: pid and program name. An answer
    /// the Restart Manager cannot give is no holder — the move goes on as it
    /// would have without asking.
    pub(super) fn holding(files: &[PathBuf]) -> Vec<(u32, String)> {
        // It knows drive paths, not the verbatim `\\?\` form.
        let names: Vec<Vec<u16>> = files
            .iter()
            .map(|file| {
                std::ffi::OsString::from(crate::util::paths::display_path(file))
                    .encode_wide()
                    .chain([0])
                    .collect()
            })
            .collect();
        let pointers: Vec<*const u16> = names.iter().map(|name| name.as_ptr()).collect();
        let mut handle = 0u32;
        let mut key = [0u16; SESSION_KEY];
        // SAFETY: a session number and a key buffer of the size the call
        // documents, both owned here.
        if unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) } != 0 {
            return Vec::new();
        }
        let session = Session(handle);
        // SAFETY: `pointers` holds `pointers.len()` NUL-terminated names that
        // outlive the call; no applications or services are passed.
        let registered = unsafe {
            RmRegisterResources(
                session.0,
                pointers.len() as u32,
                pointers.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        if registered != 0 {
            return Vec::new();
        }
        // Asked again, with room for what it said, if the list grew between
        // the two answers.
        let mut room = 8u32;
        for _ in 0..4 {
            let mut buffer = vec![ProcessInfo::empty(); room as usize];
            let (mut needed, mut count, mut reasons) = (0u32, room, 0u32);
            // SAFETY: `buffer` holds `count` entries, as the call is told;
            // the three counters are owned here.
            let answer = unsafe {
                RmGetList(
                    session.0,
                    &mut needed,
                    &mut count,
                    buffer.as_mut_ptr(),
                    &mut reasons,
                )
            };
            match answer {
                0 => {
                    buffer.truncate(count as usize);
                    drop(session);
                    return buffer
                        .into_iter()
                        .map(|info| (info.process.pid, program_of(&info)))
                        .collect();
                }
                ERROR_MORE_DATA => room = needed.max(room * 2).min(4096),
                _ => return Vec::new(),
            }
        }
        Vec::new()
    }

    /// The program's own name — its executable's, without `.exe` — else what
    /// the Restart Manager calls it.
    fn program_of(info: &ProcessInfo) -> String {
        image_name(info.process.pid).unwrap_or_else(|| {
            let end = info
                .app_name
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(APP_NAME);
            match String::from_utf16_lossy(&info.app_name[..end]) {
                name if name.trim().is_empty() => "a program".to_string(),
                name => name,
            }
        })
    }

    /// The file stem of the program `pid` runs, if Windows will say.
    fn image_name(pid: u32) -> Option<String> {
        // SAFETY: opening a process for limited information has no
        // precondition; the handle is closed by `Handle`'s drop.
        let opened = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if opened.is_null() {
            return None;
        }
        let handle = super::Handle(opened);
        let mut name = [0u16; 1024];
        let mut size = name.len() as u32;
        // SAFETY: a valid handle, and a buffer of `size` units owned here.
        let ok = unsafe { QueryFullProcessImageNameW(handle.0, 0, name.as_mut_ptr(), &mut size) };
        drop(handle);
        if ok == 0 {
            return None;
        }
        let path = PathBuf::from(String::from_utf16_lossy(&name[..size as usize]));
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    /// A minute at most: a cold PowerShell on a busy CI runner is slow to
    /// open its file, and the wait costs nothing once it has.
    fn wait_until(mut found: impl FnMut() -> bool) {
        for _ in 0..600 {
            if found() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("waited a minute for the program to hold what it was started to hold");
    }

    fn held(found: &Holders) -> Vec<(u32, String, PathBuf)> {
        found
            .uses
            .iter()
            .filter_map(|found| match (&found.by, &found.how) {
                (By::Process { pid, program }, How::Held(path)) => {
                    Some((*pid, program.clone(), path.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn busy(found: &Holders) -> Vec<PathBuf> {
        found
            .uses
            .iter()
            .filter_map(|found| match &found.how {
                How::BusyFolder(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    /// A program holding a file — any sharing — is named by the Restart
    /// Manager, with the file.
    #[test]
    fn a_program_holding_a_file_is_named() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir_all(root.join("renders")).unwrap();
        let file = root.join("renders").join("final.mov");
        std::fs::write(&file, "frames").unwrap();
        std::fs::write(root.join("notes.txt"), "free").unwrap();
        let script = format!(
            "$f = [System.IO.File]::Open('{}', 'Open', 'Read', 'ReadWrite,Delete'); Start-Sleep 30",
            crate::util::paths::display_path(&file)
        );
        let mut holder = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut found = Holders::default();
        wait_until(|| {
            found = in_tree(&root);
            !held(&found).is_empty()
        });
        let _ = holder.kill();
        let _ = holder.wait();
        let named = held(&found);
        assert!(
            named
                .iter()
                .any(|(pid, _, path)| *pid == holder.id() && path.ends_with("final.mov")),
            "{found:?}"
        );
        assert_eq!(named[0].1.to_lowercase(), "powershell");
        let refusal = found.refusal(Verb::Move, true).unwrap();
        assert!(
            refusal.contains("powershell") && refusal.contains("final.mov"),
            "{refusal}"
        );
        assert!(held(&in_tree(&root)).is_empty(), "nobody holds it any more");
    }

    /// A folder something holds open without delete sharing — how a console
    /// holds its working folder — keeps the project folder from being
    /// renamed; the Restart Manager cannot see it, the folder's own open can.
    /// Held here by the test itself: a real console has to start and settle
    /// first, which a busy CI runner does not always do in time (the console
    /// itself is the VM's scenario).
    #[test]
    fn a_folder_held_without_delete_sharing_is_found() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let sub = root.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let open = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&sub)
            .unwrap();
        let found = in_tree(&root);
        drop(open);
        assert!(
            busy(&found).iter().any(|folder| folder.ends_with("src")),
            "{found:?}"
        );
        assert!(
            found
                .refusal(Verb::Move, true)
                .unwrap()
                .contains("working in src")
        );
        assert!(busy(&in_tree(&root)).is_empty(), "let go, nothing holds it");
    }
}
