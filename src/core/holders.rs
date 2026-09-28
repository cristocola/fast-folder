//! Which programs have something in a folder open — what a person has to
//! know when a move is about to leave one behind.
//!
//! **Only what would stop the move's end stops it**, and it stops it before
//! anything is copied:
//!
//! - **Linux**: a program writing a file. After the move its writes would
//!   land in the old copy, which is then removed. A program merely *running*
//!   there — a dev server, a shell sitting in the folder, a language server
//!   reading it — does not stop it: the move absorbs what it changes, and the
//!   result says to restart it from the new place. `/proc` says who: every
//!   process's open files (`fd`), how each was opened (`fdinfo`'s flags), and
//!   its working folder (`cwd`), for this user's processes, which are the ones
//!   a person can close.
//! - **Windows**: anything open in the tree at all. Windows will not rename a
//!   folder while anything under it is open — a file, whatever its sharing,
//!   or a folder a console works in; measured in the VM — nor remove a file a
//!   program holds, so the old copy could never be set aside. The Restart
//!   Manager names the programs holding files; a folder opened for delete
//!   answers "in use" while a program works in it, which the Restart Manager
//!   cannot see.
//!
//! fastf's own processes are left out. Elsewhere this answers nothing.

use std::path::{Path, PathBuf};

use crate::core::transactions::MoveManifest;

/// A program with something in the folder open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    /// Its name, as a person would recognise it: `node`, `bash`, `code`.
    pub program: String,
    /// The file it has open, or the folder it is working in.
    pub path: PathBuf,
}

impl Holder {
    /// `node (pid 4242)`.
    pub fn who(&self) -> String {
        format!("{} (pid {})", self.program, self.pid)
    }
}

/// Everything holding something in one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holders {
    /// The folder asked about, canonical, as every path below is spelled.
    pub root: PathBuf,
    /// Programs whose hold stops the move: a file open for writing (Linux),
    /// a file open at all (Windows).
    pub blocking: Vec<Holder>,
    /// Folders some program works in, program unknown (Windows: the Restart
    /// Manager sees files only). They stop the move too.
    pub busy_folders: Vec<PathBuf>,
    /// Programs whose working folder is in it (Linux), which do not.
    pub working_in: Vec<Holder>,
}

impl Holders {
    /// The refusal a move gives when something would stop it: one sentence,
    /// naming the program and the file, or the folder, and what to do.
    pub fn refusal(&self) -> Option<String> {
        let inside = |path: &Path| match path.strip_prefix(&self.root) {
            Ok(relative) if relative.as_os_str().is_empty() => "the project folder".to_string(),
            Ok(relative) => relative.display().to_string(),
            Err(_) => crate::util::paths::display_path(path),
        };
        if let Some(first) = self.blocking.first() {
            let more = match self.blocking.len() {
                1 => String::new(),
                n => format!(" (and {} more)", n - 1),
            };
            return Some(if cfg!(windows) {
                format!(
                    "{} has {} open{more}; close it, or quit {}, then try again — Windows \
                     will not move a folder while anything in it is open",
                    first.who(),
                    inside(&first.path),
                    first.program
                )
            } else {
                format!(
                    "{} has {} open for writing{more}; close it, or quit {}, then move again \
                     — its writes would otherwise land in the old copy after the move",
                    first.who(),
                    inside(&first.path),
                    first.program
                )
            });
        }
        let first = self.busy_folders.first()?;
        let more = match self.busy_folders.len() {
            1 => String::new(),
            n => format!(" (and {} more folders)", n - 1),
        };
        Some(format!(
            "a program is working in {}{more} — a console, or another program whose working \
             folder is there; leave it, then try again — Windows will not move a folder while \
             a program works in it",
            inside(first)
        ))
    }

    /// The note a finished move gives about programs still working in the
    /// old folder.
    pub fn notes(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.working_in
            .iter()
            .filter(|holder| seen.insert(holder.pid))
            .map(|holder| {
                format!(
                    "{} was running in the old folder; restart it from the new one",
                    holder.who()
                )
            })
            .collect()
    }
}

/// Who holds something in `root`, as far as this machine can tell. On
/// Windows this walks the folder for what to ask about; a move that has
/// scanned it already asks [`in_manifest`].
pub fn in_tree(root: &Path) -> Holders {
    let root = crate::util::paths::canonical(root).unwrap_or_else(|_| root.to_path_buf());
    Holders {
        root: root.clone(),
        ..imp::in_tree(&root, None)
    }
}

/// [`in_tree`], about the entries a move's scan found.
pub fn in_manifest(root: &Path, manifest: &MoveManifest) -> Holders {
    let root = crate::util::paths::canonical(root).unwrap_or_else(|_| root.to_path_buf());
    Holders {
        root: root.clone(),
        ..imp::in_tree(&root, Some(manifest))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Holder, Holders};
    use crate::core::transactions::MoveManifest;
    use std::path::{Path, PathBuf};

    pub(super) fn in_tree(root: &Path, _entries: Option<&MoveManifest>) -> Holders {
        let mut holders = Holders::default();
        let Ok(processes) = std::fs::read_dir("/proc") else {
            return holders;
        };
        let me = std::process::id();
        for process in processes.flatten() {
            let Some(pid) = process
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == me {
                continue;
            }
            let base = process.path();
            let program = program_name(&base);
            // fastf's own processes: a job's worker, the app, a reconcile.
            if program.starts_with("fastf") {
                continue;
            }
            if let Ok(cwd) = std::fs::read_link(base.join("cwd"))
                && cwd.starts_with(root)
            {
                holders.working_in.push(Holder {
                    pid,
                    program: program.clone(),
                    path: cwd,
                });
            }
            let Ok(descriptors) = std::fs::read_dir(base.join("fd")) else {
                continue;
            };
            for descriptor in descriptors.flatten() {
                let Ok(target) = std::fs::read_link(descriptor.path()) else {
                    continue;
                };
                let target = PathBuf::from(
                    target
                        .to_string_lossy()
                        .trim_end_matches(" (deleted)")
                        .to_string(),
                );
                if !target.starts_with(root) {
                    continue;
                }
                let info = base.join("fdinfo").join(descriptor.file_name());
                if opened_for_writing(&std::fs::read_to_string(info).unwrap_or_default()) {
                    holders.blocking.push(Holder {
                        pid,
                        program: program.clone(),
                        path: target,
                    });
                }
            }
        }
        holders.blocking.sort_by_key(|holder| holder.pid);
        holders.working_in.sort_by_key(|holder| holder.pid);
        holders
    }

    /// The program's own name: its executable's file name, else `comm`.
    fn program_name(process: &Path) -> String {
        std::fs::read_link(process.join("exe"))
            .ok()
            .and_then(|exe| {
                exe.file_name().map(|name| {
                    name.to_string_lossy()
                        .trim_end_matches(" (deleted)")
                        .to_string()
                })
            })
            .or_else(|| {
                std::fs::read_to_string(process.join("comm"))
                    .ok()
                    .map(|name| name.trim().to_string())
            })
            .unwrap_or_else(|| "a program".to_string())
    }

    /// Whether `fdinfo`'s `flags:` (octal) say the file was opened to write.
    pub(super) fn opened_for_writing(fdinfo: &str) -> bool {
        fdinfo
            .lines()
            .find_map(|line| line.strip_prefix("flags:"))
            .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
            .is_some_and(|flags| flags & (libc::O_ACCMODE as u32) != libc::O_RDONLY as u32)
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use super::Holders;
    use crate::core::transactions::MoveManifest;
    use std::path::Path;

    pub(super) fn in_tree(_root: &Path, _entries: Option<&MoveManifest>) -> Holders {
        Holders::default()
    }
}

#[cfg(windows)]
mod imp {
    use super::{Holder, Holders};
    use crate::core::transactions::{ManifestKind, MoveManifest};
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    /// How many files one Restart Manager session is asked about.
    const BATCH: usize = 1000;
    /// Held files named, at most: a refusal names the first, and bisecting
    /// a batch costs a session per step.
    const NAMED: usize = 8;

    pub(super) fn in_tree(root: &Path, entries: Option<&MoveManifest>) -> Holders {
        let (files, folders) = match entries {
            Some(manifest) => of_manifest(root, manifest),
            None => walk(root),
        };
        let mut holders = Holders::default();
        let me = std::process::id();
        for batch in files.chunks(BATCH) {
            if holders.blocking.len() >= NAMED {
                break;
            }
            name_holders(batch, me, &mut holders.blocking);
        }
        // A folder is asked on every filesystem that is a disk here; across
        // a network each open is a round trip, and the project folder alone
        // is asked.
        let local = matches!(
            crate::util::fs_kind::of(root),
            crate::util::fs_kind::FsKind::Local
        );
        for folder in folders.iter().filter(|folder| local || *folder == root) {
            if in_use(folder) {
                holders.busy_folders.push(folder.clone());
            }
        }
        holders
    }

    /// The files and folders a scan found, as full paths, the project
    /// folder first among the folders.
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

    /// Ask the Restart Manager who holds any of `files`; where somebody
    /// does, halve the batch until each held file is named.
    fn name_holders(files: &[PathBuf], me: u32, found: &mut Vec<Holder>) {
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
                found.push(Holder {
                    pid,
                    program,
                    path: file.clone(),
                });
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

    const DELETE: u32 = 0x0001_0000;
    const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    /// Whether a program has `folder` open without letting it be deleted —
    /// a console working there does. Only "in use" counts: a folder fastf
    /// may not open is the probe's to say, not this.
    fn in_use(folder: &Path) -> bool {
        let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: a NUL-terminated path fastf owns; the handle is closed at once.
        unsafe {
            let handle = CreateFileW(
                wide.as_ptr(),
                DELETE,
                SHARE_ALL,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            );
            if handle as isize == -1 {
                return std::io::Error::last_os_error().raw_os_error()
                    == Some(ERROR_SHARING_VIOLATION);
            }
            CloseHandle(handle);
            false
        }
    }

    /// The Restart Manager, declared by hand (`rstrtmgr.dll`, part of
    /// Windows since Vista, so the standalone check allows it).
    mod restart_manager {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStrExt;
        use std::path::PathBuf;

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct FileTime {
            low: u32,
            high: u32,
        }

        #[repr(C)]
        #[derive(Clone, Copy)]
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
            fn CloseHandle(handle: *mut c_void) -> i32;
            fn QueryFullProcessImageNameW(
                handle: *mut c_void,
                flags: u32,
                name: *mut u16,
                size: *mut u32,
            ) -> i32;
        }

        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

        /// Every process holding any of `files`: pid and program name. An
        /// answer the Restart Manager cannot give is no holder — the move
        /// goes on as it would have without asking.
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
            let mut session = 0u32;
            let mut key = [0u16; SESSION_KEY];
            // SAFETY: buffers fastf owns, sized as each call is told; the
            // session is ended on every path out.
            unsafe {
                if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
                    return Vec::new();
                }
                let listed = (|| {
                    if RmRegisterResources(
                        session,
                        pointers.len() as u32,
                        pointers.as_ptr(),
                        0,
                        std::ptr::null(),
                        0,
                        std::ptr::null(),
                    ) != 0
                    {
                        return Vec::new();
                    }
                    // Asked again, with room for what it said, if the list
                    // grew between the two answers.
                    let mut room = 8u32;
                    for _ in 0..4 {
                        let mut buffer: Vec<ProcessInfo> = vec![std::mem::zeroed(); room as usize];
                        let (mut needed, mut count, mut reasons) = (0u32, room, 0u32);
                        let answer = RmGetList(
                            session,
                            &mut needed,
                            &mut count,
                            buffer.as_mut_ptr(),
                            &mut reasons,
                        );
                        match answer {
                            0 => {
                                buffer.truncate(count as usize);
                                return buffer;
                            }
                            ERROR_MORE_DATA => room = needed.max(room * 2).min(4096),
                            _ => return Vec::new(),
                        }
                    }
                    Vec::new()
                })();
                RmEndSession(session);
                listed
                    .into_iter()
                    .map(|info| (info.process.pid, program_of(&info)))
                    .collect()
            }
        }

        /// The program's own name — its executable's, without `.exe` — else
        /// what the Restart Manager calls it.
        fn program_of(info: &ProcessInfo) -> String {
            let from_image = (|| {
                // SAFETY: a handle this function opens and closes, and a
                // buffer it owns, sized as the call is told.
                unsafe {
                    let handle =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, info.process.pid);
                    if handle.is_null() {
                        return None;
                    }
                    let mut name = [0u16; 1024];
                    let mut size = name.len() as u32;
                    let ok = QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut size);
                    CloseHandle(handle);
                    if ok == 0 {
                        return None;
                    }
                    let path = PathBuf::from(String::from_utf16_lossy(&name[..size as usize]));
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                }
            })();
            from_image.unwrap_or_else(|| {
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
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn fdinfo_flags_say_how_a_file_was_opened() {
        assert!(!imp::opened_for_writing(
            "pos:\t0\nflags:\t0100000\nmnt_id:\t26\n"
        ));
        assert!(imp::opened_for_writing(
            "pos:\t0\nflags:\t0102001\nmnt_id:\t26\n"
        ));
        assert!(imp::opened_for_writing("flags:\t02\n"));
        assert!(!imp::opened_for_writing("nothing here"));
    }

    /// A child holding a file open for writing is found, by name; one
    /// sitting in the folder is a working-folder holder, not a writer.
    #[test]
    fn a_writer_and_a_program_working_in_the_folder_are_told_apart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("session.txt"), "x").unwrap();
        let mut writer = std::process::Command::new("sh")
            .arg("-c")
            .arg("exec 3>>project/session.txt; exec sleep 30")
            .current_dir(temp.path())
            .spawn()
            .unwrap();
        let mut sitter = std::process::Command::new("sleep")
            .arg("30")
            .current_dir(&root)
            .spawn()
            .unwrap();
        // Let the shell open its file.
        let mut found = Holders::default();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            found = in_tree(&root);
            if !found.blocking.is_empty() && !found.working_in.is_empty() {
                break;
            }
        }
        let _ = writer.kill();
        let _ = sitter.kill();
        let _ = writer.wait();
        let _ = sitter.wait();
        assert!(
            found
                .blocking
                .iter()
                .any(|holder| holder.pid == writer.id() && holder.path.ends_with("session.txt")),
            "{found:?}"
        );
        assert!(
            found
                .working_in
                .iter()
                .any(|holder| holder.pid == sitter.id()),
            "{found:?}"
        );
        assert!(found.refusal().unwrap().contains("session.txt"));
        assert!(found.notes()[0].contains("restart it from the new one"));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

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

    /// A program holding a file — any sharing — is named by the Restart
    /// Manager, with the file.
    #[test]
    fn a_program_holding_a_file_is_named() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir_all(root.join("renders")).unwrap();
        let held = root.join("renders").join("final.mov");
        std::fs::write(&held, "frames").unwrap();
        std::fs::write(root.join("notes.txt"), "free").unwrap();
        let script = format!(
            "$f = [System.IO.File]::Open('{}', 'Open', 'Read', 'ReadWrite,Delete'); Start-Sleep 30",
            crate::util::paths::display_path(&held)
        );
        let mut holder = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut found = Holders::default();
        wait_until(|| {
            found = in_tree(&root);
            !found.blocking.is_empty()
        });
        let _ = holder.kill();
        let _ = holder.wait();
        assert!(
            found
                .blocking
                .iter()
                .any(|h| h.pid == holder.id() && h.path.ends_with("final.mov")),
            "{found:?}"
        );
        assert_eq!(found.blocking[0].program.to_lowercase(), "powershell");
        let refusal = found.refusal().unwrap();
        assert!(
            refusal.contains("powershell") && refusal.contains("final.mov"),
            "{refusal}"
        );
        assert!(
            in_tree(&root).blocking.is_empty(),
            "nobody holds it any more"
        );
    }

    /// A folder something holds open without delete sharing — how a console
    /// holds its working folder — keeps the project folder from being
    /// renamed; the Restart Manager cannot see it, the folder's own open can.
    /// Held here by the test itself: a real console has to start and settle
    /// first, which a busy CI runner did not always do in time (the console
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
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&sub)
            .unwrap();
        let found = in_tree(&root);
        drop(held);
        assert!(
            found
                .busy_folders
                .iter()
                .any(|folder| folder.ends_with("src")),
            "{found:?}"
        );
        assert!(found.refusal().unwrap().contains("working in src"));
        assert!(
            in_tree(&root).busy_folders.is_empty(),
            "let go, nothing holds it"
        );
    }
}
