//! Which program started this one — the shell a verb was typed into.
//!
//! `$SHELL` names the *login* shell, which is not the shell somebody is typing
//! into after `bash` inside fish, or on Windows, where there is no such
//! variable at all. The parent process is the honest answer, and `fastf cd`
//! needs it twice: to know which startup file would teach that shell the
//! function, and which program to start when it cannot be taught.
//!
//! Linux reads `/proc`; Windows walks a toolhelp snapshot. Anywhere else the
//! answer is `None` and the caller falls back to `$SHELL`. A wrapper that
//! carries fastf's own name — a package manager's shim — is stepped over, so
//! the answer is the program that ran the shim.

use std::path::PathBuf;

/// The executable of the nearest ancestor that is not a fastf of some kind,
/// looking at most four generations up.
pub fn parent_executable() -> Option<PathBuf> {
    let own = std::env::current_exe().ok().and_then(|exe| stem(&exe));
    let mut pid = parent_of(std::process::id())?;
    for _ in 0..4 {
        let exe = executable_of(pid)?;
        if stem(&exe) != own {
            return Some(exe);
        }
        pid = parent_of(pid)?;
    }
    None
}

/// A program's name without a directory or `.exe`, lowercased, and without
/// the `-` a login shell's name carries — the name to compare shells by.
pub fn stem(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    Some(stem.trim_start_matches('-').to_ascii_lowercase())
}

#[cfg(target_os = "linux")]
fn parent_of(pid: u32) -> Option<u32> {
    if pid == std::process::id() {
        return Some(std::os::unix::process::parent_id());
    }
    // Field 4 of `stat`, counted after the parenthesised name, which may
    // itself hold spaces and parentheses.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn executable_of(pid: u32) -> Option<PathBuf> {
    // `exe` is the program's real path; a process owned by somebody else
    // refuses it, and `comm` — the name alone — still says which shell it is.
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .or_else(|| {
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
            Some(PathBuf::from(comm.trim_end()))
        })
}

#[cfg(windows)]
mod win {
    use std::path::PathBuf;

    type Handle = *mut core::ffi::c_void;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct ProcessEntry32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    /// The parent's id and the image name the snapshot holds for `pid`.
    fn entry_of(pid: u32) -> Option<(u32, PathBuf)> {
        // SAFETY: plain Win32 calls on a snapshot this function owns and
        // closes; the entry is zeroed with its size set, as the API requires.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: ProcessEntry32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut found = None;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                if entry.th32ProcessID == pid {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                    found = Some((entry.th32ParentProcessID, PathBuf::from(name)));
                    break;
                }
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            found
        }
    }

    pub fn parent_of(pid: u32) -> Option<u32> {
        entry_of(pid).map(|(parent, _)| parent)
    }

    /// The full path when the process lets itself be asked, the snapshot's
    /// bare name otherwise — enough to know which shell it is, and `PATH`
    /// finds the rest.
    pub fn executable_of(pid: u32) -> Option<PathBuf> {
        // SAFETY: the handle is checked and closed; the buffer's length is
        // passed in and the written length read back.
        let full = unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                None
            } else {
                let mut buf = [0u16; 1024];
                let mut size = buf.len() as u32;
                let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size);
                CloseHandle(process);
                (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])))
            }
        };
        full.or_else(|| entry_of(pid).map(|(_, name)| name))
    }
}

#[cfg(windows)]
use win::{executable_of, parent_of};

#[cfg(not(any(target_os = "linux", windows)))]
fn parent_of(_pid: u32) -> Option<u32> {
    None
}

#[cfg(not(any(target_os = "linux", windows)))]
fn executable_of(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_shells_dash_and_an_exe_suffix_are_not_its_name() {
        assert_eq!(stem("-bash".as_ref()).as_deref(), Some("bash"));
        assert_eq!(
            stem("PowerShell.exe".as_ref()).as_deref(),
            Some("powershell")
        );
        assert_eq!(stem("/usr/bin/fish".as_ref()).as_deref(), Some("fish"));
    }

    /// The test runner has a parent, and it is not the test binary itself.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn the_test_runner_has_a_parent() {
        let parent = parent_executable().expect("cargo or a shell started the tests");
        assert!(!parent.as_os_str().is_empty());
    }
}
