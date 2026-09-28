//! Other processes, as far as fastf needs to know about them: is the one that
//! minted an operation still running.
//!
//! **A record belongs to its worker, whichever data dir started it.** A job's
//! own data dir knows its live jobs (`core::jobs::live_workers`), but a second
//! fastf on the same machine — another data dir, portable mode beside the
//! installed one, a test lab — does not, and its reconcile would take a live
//! move's record for an abandoned one and discard the copy while it is being
//! written. An operation id names the pid that minted it and the moment it
//! did (`transactions::next_operation_id`), which is enough to ask the
//! operating system instead.

/// Whether `pid` is a running fastf that started no later than
/// `minted_nanos` (nanoseconds since the Unix epoch). A process holding the pid
/// that started after that moment is a stranger who reused it, and so is one
/// that is not fastf.
pub fn is_live_fastf_since(pid: u32, minted_nanos: u128) -> bool {
    // A second of slack: the boot time the start is measured from is whole
    // seconds on Linux.
    const SLACK: u128 = 1_000_000_000;
    match imp::started_fastf(pid) {
        Some(started) => started <= minted_nanos.saturating_add(SLACK),
        None => false,
    }
}

/// Whether a file name is one fastf is started by: the installed `fastf`,
/// `fastf.exe`, or a renamed build (`fastf-3.14.0`).
fn is_fastf_name(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy()
        .to_ascii_lowercase()
        .starts_with("fastf")
}

#[cfg(target_os = "linux")]
mod imp {
    /// When `pid` started, in nanoseconds since the Unix epoch, if it is a
    /// running fastf of this user.
    pub(super) fn started_fastf(pid: u32) -> Option<u128> {
        let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        if !super::is_fastf_name(exe.file_name()?) {
            return None;
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command name may hold spaces and parentheses: the fields that
        // follow are counted from its last `)`, the state being field 3.
        let after = stat.get(stat.rfind(')')? + 1..)?;
        let fields: Vec<&str> = after.split_whitespace().collect();
        // A zombie has finished; it only waits to be reaped.
        if fields.first() == Some(&"Z") {
            return None;
        }
        let ticks: u128 = fields.get(22 - 3)?.parse().ok()?;
        // SAFETY: `sysconf` reads a constant of the running system.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz <= 0 {
            return None;
        }
        let boot = std::fs::read_to_string("/proc/stat").ok()?;
        let boot_secs: u128 = boot
            .lines()
            .find_map(|line| line.strip_prefix("btime "))?
            .trim()
            .parse()
            .ok()?;
        Some(boot_secs * 1_000_000_000 + ticks * 1_000_000_000 / hz as u128)
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        fn GetProcessTimes(
            handle: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn QueryFullProcessImageNameW(
            handle: *mut c_void,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
    }

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    /// 1601-01-01 to 1970-01-01, in the 100 ns units of a FILETIME.
    const EPOCH_DIFFERENCE: u64 = 116_444_736_000_000_000;

    pub(super) fn started_fastf(pid: u32) -> Option<u128> {
        use std::os::windows::ffi::OsStringExt;
        // SAFETY: every call gets a handle this function opened and closes,
        // and buffers it owns, sized as the calls are told.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let answer = (|| {
                let mut code = 0u32;
                if GetExitCodeProcess(handle, &mut code) == 0 || code != STILL_ACTIVE {
                    return None;
                }
                let mut name = vec![0u16; 1024];
                let mut size = name.len() as u32;
                if QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut size) == 0 {
                    return None;
                }
                name.truncate(size as usize);
                let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&name));
                if !super::is_fastf_name(path.file_name()?) {
                    return None;
                }
                let (mut created, mut exited) = (FileTime::default(), FileTime::default());
                let (mut kernel, mut user) = (FileTime::default(), FileTime::default());
                if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
                    return None;
                }
                let ticks = (u64::from(created.high) << 32) | u64::from(created.low);
                Some(u128::from(ticks.checked_sub(EPOCH_DIFFERENCE)?) * 100)
            })();
            CloseHandle(handle);
            answer
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    /// No way to ask here: nothing is claimed for another process.
    pub(super) fn started_fastf(_pid: u32) -> Option<u128> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    /// This test binary is a fastf (`fastf-<hash>`), started before now.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn this_process_is_live_since_it_started() {
        assert!(is_live_fastf_since(std::process::id(), now_nanos()));
    }

    /// An operation minted before the process holding its pid started was
    /// minted by somebody else, who has gone.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_pid_reused_after_the_mint_is_not_the_minter() {
        assert!(!is_live_fastf_since(std::process::id(), 1_000_000_000));
    }

    #[test]
    fn a_process_that_is_not_there_is_not_live() {
        assert!(!is_live_fastf_since(u32::MAX - 7, now_nanos()));
    }
}
