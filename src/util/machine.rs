//! Which machine this is, in a form that survives a rename.
//!
//! A move's journal records the machine that began it, because a base can be
//! reached from more than one machine and a source path means something else
//! on the other. A hostname is a poor record of that: a laptop takes one from
//! DHCP, and a renamed machine would find its own unfinished moves reported as
//! somebody else's for good. The operating system's machine id stays.

/// The machine id: `/etc/machine-id` on Linux, `MachineGuid` on Windows;
/// `None` where there is none to read.
pub fn id() -> Option<String> {
    imp::id()
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn id() -> Option<String> {
        ["/etc/machine-id", "/var/lib/dbus/machine-id"]
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok())
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;

    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x0000_0002;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: isize,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut std::ffi::c_void,
            size: *mut u32,
        ) -> i32;
    }

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain([0])
            .collect()
    }

    pub fn id() -> Option<String> {
        let subkey = wide(r"SOFTWARE\Microsoft\Cryptography");
        let value = wide("MachineGuid");
        let mut buffer = [0_u16; 64];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        // SAFETY: NUL-terminated names, a writable buffer of `size` bytes, and
        // a null type pointer, which the call allows.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let units = (size as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..units]);
        Some(text.trim_end_matches('\0').to_string())
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    pub fn id() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    /// Asked twice it answers the same, and an id is long enough to tell
    /// two machines apart. Windows always has one; a Linux machine may have
    /// none — a container with no `/etc/machine-id`, where a package is
    /// built and this suite runs — and `None` is the answer there.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn the_machine_id_is_steady_where_there_is_one() {
        let first = super::id();
        assert_eq!(super::id(), first);
        if cfg!(windows) {
            assert!(first.is_some(), "Windows has a MachineGuid");
        }
        if let Some(id) = first {
            assert!(id.len() >= 16, "{id}");
        }
    }
}
