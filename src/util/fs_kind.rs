//! What kind of filesystem a folder is on — as much as a move needs: how many
//! requests to keep in flight there, whether a folder rename is one step, and
//! whether a removed folder can come back.
//!
//! fastf still only ever sees a folder. This reads what the operating system
//! says about the mount that folder is on (`/proc/self/mountinfo` on Linux,
//! the volume's drive type and file system name on Windows) — never the
//! network or the cloud behind it.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    /// A local disk.
    Local,
    Nfs,
    /// SMB/CIFS, or a Windows network drive.
    Smb,
    /// sshfs: one SFTP request per call, and a rename is one request.
    Sshfs,
    /// rclone: an object store behind a cache that uploads in the background.
    /// A folder rename is a copy and a delete per object, and a removed
    /// folder can be put back by an upload still in flight.
    Rclone,
    /// Another FUSE mount, whose semantics fastf cannot know.
    OtherFuse,
    Unknown,
}

impl FsKind {
    /// Whether every call there is a request to somewhere else.
    pub fn is_remote(self) -> bool {
        !matches!(self, FsKind::Local)
    }
}

/// The kind of filesystem `path` is on. Memoised by mount: bases are few, and
/// a move asks about them several times.
pub fn of(path: &Path) -> FsKind {
    // A decision, like `move:force-staged`: the suites take the rclone paths
    // on a local disk.
    if crate::util::faults::is_armed("fs:as-rclone") {
        return FsKind::Rclone;
    }
    static MEMO: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, FsKind>>> =
        std::sync::Mutex::new(None);
    if let Ok(memo) = MEMO.lock()
        && let Some(kind) = memo.as_ref().and_then(|memo| memo.get(path))
    {
        return *kind;
    }
    let kind = imp::of(path);
    if let Ok(mut memo) = MEMO.lock() {
        memo.get_or_insert_with(Default::default)
            .insert(path.to_path_buf(), kind);
    }
    kind
}

/// The kind a mount's type name stands for (`fuse.rclone`, `nfs4`, …).
pub fn from_type_name(name: &str) -> FsKind {
    match name {
        "fuse.rclone" => FsKind::Rclone,
        "fuse.sshfs" => FsKind::Sshfs,
        "nfs" | "nfs4" => FsKind::Nfs,
        "cifs" | "smb3" | "smbfs" => FsKind::Smb,
        name if name.starts_with("fuse") => FsKind::OtherFuse,
        _ => FsKind::Local,
    }
}

/// The mount `path` is on, from `mountinfo`'s text: the longest mount point
/// that contains it, and that mount's type.
pub fn mount_of<'a>(mountinfo: &'a str, path: &Path) -> Option<(PathBuf, &'a str)> {
    let mut best: Option<(PathBuf, &str)> = None;
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(dash) = fields.iter().position(|field| *field == "-") else {
            continue;
        };
        let (Some(point), Some(kind)) = (fields.get(4), fields.get(dash + 1)) else {
            continue;
        };
        let point = PathBuf::from(unescape(point));
        if path.starts_with(&point)
            && best
                .as_ref()
                .is_none_or(|(longest, _)| point.as_os_str().len() >= longest.as_os_str().len())
        {
            best = Some((point, kind));
        }
    }
    best
}

/// The mount points strictly inside `path`, where the system can say without
/// a walk (Linux: `/proc/self/mountinfo`); `None` where it cannot, and the
/// caller has to walk the tree to find out.
pub fn mounts_inside(path: &Path) -> Option<Vec<PathBuf>> {
    imp::mounts_inside(path)
}

/// The mount points in `mountinfo`'s text that are strictly inside `path`.
pub fn mount_points_inside(mountinfo: &str, path: &Path) -> Vec<PathBuf> {
    mountinfo
        .lines()
        .filter_map(|line| line.split_whitespace().nth(4))
        .map(|point| PathBuf::from(unescape(point)))
        .filter(|point| point != path && point.starts_with(path))
        .collect()
}

/// `mountinfo` writes a space, a tab, a newline and a backslash in a mount
/// point as three octal digits (`\040`).
fn unescape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let digits: String = chars.clone().take(3).collect();
            if digits.len() == 3
                && let Ok(code) = u8::from_str_radix(&digits, 8)
            {
                out.push(char::from(code));
                for _ in 0..3 {
                    chars.next();
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

#[cfg(target_os = "linux")]
mod imp {
    use super::FsKind;
    use std::path::{Path, PathBuf};

    pub(super) fn mounts_inside(path: &Path) -> Option<Vec<PathBuf>> {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
        Some(super::mount_points_inside(&mountinfo, path))
    }

    pub(super) fn of(path: &Path) -> FsKind {
        let Ok(mountinfo) = std::fs::read_to_string("/proc/self/mountinfo") else {
            return FsKind::Unknown;
        };
        match super::mount_of(&mountinfo, path) {
            Some((_, kind)) => super::from_type_name(kind),
            None => FsKind::Unknown,
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::FsKind;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    /// A mount inside a folder on Windows is a reparse point in it, which only
    /// a walk finds.
    pub(super) fn mounts_inside(_path: &Path) -> Option<Vec<PathBuf>> {
        None
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, length: u32) -> i32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_length: u32,
            serial: *mut u32,
            component_length: *mut u32,
            flags: *mut u32,
            file_system: *mut u16,
            file_system_length: u32,
        ) -> i32;
    }

    const DRIVE_REMOTE: u32 = 4;

    pub(super) fn of(path: &Path) -> FsKind {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let mut root = vec![0u16; 1024];
        // SAFETY: NUL-terminated input and an output buffer of the length the
        // calls are told; a failure answers `Unknown`.
        unsafe {
            if GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) == 0 {
                return FsKind::Unknown;
            }
            let mut file_system = vec![0u16; 64];
            let named = GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                file_system.as_mut_ptr(),
                file_system.len() as u32,
            ) != 0;
            let name = if named {
                let end = file_system.iter().position(|c| *c == 0).unwrap_or(0);
                String::from_utf16_lossy(&file_system[..end]).to_ascii_lowercase()
            } else {
                String::new()
            };
            // WinFsp's FUSE layer names itself; rclone says so in the name.
            if name.contains("rclone") {
                FsKind::Rclone
            } else if name.starts_with("fuse") {
                FsKind::OtherFuse
            } else if GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE {
                FsKind::Smb
            } else {
                FsKind::Local
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use super::FsKind;
    use std::path::{Path, PathBuf};

    pub(super) fn of(_path: &Path) -> FsKind {
        FsKind::Unknown
    }

    pub(super) fn mounts_inside(_path: &Path) -> Option<Vec<PathBuf>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
42 1 0:34 /@ / rw,noatime shared:1 - btrfs /dev/mapper/root rw
28 42 0:82 / /mnt/cloud_proj rw,nosuid shared:641 - fuse.rclone r2:proj/cloud_proj rw
540 42 0:124 / /mnt/laptop\\040projects rw,nosuid shared:264 - fuse.sshfs host:/p rw
541 42 0:125 / /mnt/share rw shared:265 - cifs //nas/share rw
";

    #[test]
    fn the_longest_mount_that_holds_the_path_decides() {
        let kind =
            |path: &str| mount_of(MOUNTINFO, Path::new(path)).map(|(_, kind)| from_type_name(kind));
        assert_eq!(kind("/mnt/cloud_proj/lab"), Some(FsKind::Rclone));
        assert_eq!(kind("/mnt/laptop projects/x"), Some(FsKind::Sshfs));
        assert_eq!(kind("/mnt/share"), Some(FsKind::Smb));
        assert_eq!(kind("/home/user/Projects"), Some(FsKind::Local));
        // A sibling whose name only starts the same is not inside the mount.
        assert_eq!(kind("/mnt/cloud_projects"), Some(FsKind::Local));
    }

    #[test]
    fn mount_type_names_map_to_kinds() {
        assert_eq!(from_type_name("fuse.rclone"), FsKind::Rclone);
        assert_eq!(from_type_name("fuse.sshfs"), FsKind::Sshfs);
        assert_eq!(from_type_name("fuse.gocryptfs"), FsKind::OtherFuse);
        assert_eq!(from_type_name("nfs4"), FsKind::Nfs);
        assert_eq!(from_type_name("ext4"), FsKind::Local);
    }

    #[test]
    fn mounts_inside_a_folder_are_read_from_mountinfo() {
        let mountinfo = "22 1 0:21 / / rw - btrfs /dev/x rw\n\
             40 22 0:40 / /home/user/proj/data rw - fuse.sshfs host: rw\n\
             41 22 0:41 / /home/user/project rw - fuse.rclone r2: rw\n\
             42 22 0:42 / /home/user/proj rw - tmpfs tmpfs rw\n";
        assert_eq!(
            mount_points_inside(mountinfo, Path::new("/home/user/proj")),
            vec![PathBuf::from("/home/user/proj/data")],
            "a mount at the folder itself, or beside it, is not inside it"
        );
    }
}
