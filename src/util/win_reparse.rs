//! Windows reparse points: which kind of link an entry is, and making a
//! junction.
//!
//! `std` tells a link from a file, and reads a symbolic link's or a junction's
//! target, but it cannot say which of the two it read, and it cannot make a
//! junction (`junction_point` is unstable). A move has to do both to carry a
//! junction across drives as a junction, and the reparse tag is also what
//! tells a link — a name surrogate — from a mounted volume or a kind fastf
//! cannot recreate. Hand-rolled, like `shell_open`: two calls do not justify a
//! bindings crate.

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;

type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = -1_isize as Handle;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
/// `FILE_INFO_BY_HANDLE_CLASS::FileAttributeTagInfo`.
const FILE_ATTRIBUTE_TAG_INFO: i32 = 9;
const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
/// `MAXIMUM_REPARSE_DATA_BUFFER_SIZE`.
const MAXIMUM_REPARSE_DATA: usize = 16 * 1024;

#[repr(C)]
struct FileAttributeTagInfo {
    file_attributes: u32,
    reparse_tag: u32,
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
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetFileInformationByHandleEx(
        handle: Handle,
        class: i32,
        info: *mut c_void,
        size: u32,
    ) -> i32;
    fn DeviceIoControl(
        handle: Handle,
        code: u32,
        input: *const c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

/// A handle closed when dropped.
struct Opened(Handle);

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful `CreateFileW` and is
        // closed exactly once, here.
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// Open the entry itself, never what it points to.
fn open(path: &Path, access: u32) -> io::Result<Opened> {
    let name = wide(path);
    // SAFETY: a NUL-terminated wide path; no security attributes, no template.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            FILE_SHARE_ALL,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(Opened(handle))
}

/// The entry's reparse tag, or 0 when it is not a reparse point.
pub fn reparse_tag(path: &Path) -> io::Result<u32> {
    let opened = open(path, FILE_READ_ATTRIBUTES)?;
    let mut info = FileAttributeTagInfo {
        file_attributes: 0,
        reparse_tag: 0,
    };
    // SAFETY: a live handle and a buffer of exactly the size given.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            opened.0,
            FILE_ATTRIBUTE_TAG_INFO,
            (&raw mut info).cast(),
            std::mem::size_of::<FileAttributeTagInfo>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.reparse_tag)
}

/// The two names a junction stores for `target`: the NT form it resolves
/// (`\??\C:\x`), and the DOS form Explorer shows (`C:\x`). A junction points
/// at a folder on a local volume by its full path, so anything else — a
/// relative path, a share, a volume named by GUID — is refused.
pub fn junction_names(target: &Path) -> io::Result<(Vec<u16>, Vec<u16>)> {
    let units: Vec<u16> = target.as_os_str().encode_wide().collect();
    let text = String::from_utf16_lossy(&units);
    let dos = text
        .strip_prefix(r"\\?\")
        .or_else(|| text.strip_prefix(r"\??\"))
        .unwrap_or(&text);
    let bytes = dos.as_bytes();
    let drive_path =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !drive_path {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a junction's target must be a folder on a drive, not {text}"),
        ));
    }
    let print: Vec<u16> = dos.encode_utf16().collect();
    let substitute: Vec<u16> = r"\??\"
        .encode_utf16()
        .chain(print.iter().copied())
        .collect();
    Ok((substitute, print))
}

/// The `MOUNT_POINT` reparse buffer for a junction to `target`.
pub fn junction_buffer(target: &Path) -> io::Result<Vec<u8>> {
    let (substitute, print) = junction_names(target)?;
    let substitute_bytes = substitute.len() * 2;
    let print_bytes = print.len() * 2;
    // Four u16 fields, then both names, each with its NUL.
    let data_length = 8 + substitute_bytes + 2 + print_bytes + 2;
    if 8 + data_length > MAXIMUM_REPARSE_DATA {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the junction's target is too long",
        ));
    }
    let as_u16 = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    let mut buffer = Vec::with_capacity(8 + data_length);
    buffer.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend_from_slice(&as_u16(data_length).to_le_bytes());
    buffer.extend_from_slice(&0_u16.to_le_bytes());
    buffer.extend_from_slice(&0_u16.to_le_bytes());
    buffer.extend_from_slice(&as_u16(substitute_bytes).to_le_bytes());
    buffer.extend_from_slice(&as_u16(substitute_bytes + 2).to_le_bytes());
    buffer.extend_from_slice(&as_u16(print_bytes).to_le_bytes());
    for unit in substitute
        .iter()
        .chain([&0])
        .chain(print.iter())
        .chain([&0])
    {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(buffer)
}

/// Make `link` a junction to `target`, which need not exist. `link` must not.
pub fn create_junction(target: &Path, link: &Path) -> io::Result<()> {
    let buffer = junction_buffer(target)?;
    std::fs::create_dir(link)?;
    let set = (|| {
        let opened = open(link, GENERIC_WRITE)?;
        let mut returned = 0_u32;
        // SAFETY: a live handle, an input buffer of exactly the size given,
        // and no output buffer, which this control code takes.
        let ok = unsafe {
            DeviceIoControl(
                opened.0,
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if set.is_err() {
        let _ = std::fs::remove_dir(link);
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_junction_stores_the_nt_name_it_resolves_and_the_dos_name_it_shows() {
        let (substitute, print) = junction_names(Path::new(r"\\?\C:\Assets\Library")).unwrap();
        assert_eq!(
            String::from_utf16(&substitute).unwrap(),
            r"\??\C:\Assets\Library"
        );
        assert_eq!(String::from_utf16(&print).unwrap(), r"C:\Assets\Library");
        let (_, root) = junction_names(Path::new(r"D:\")).unwrap();
        assert_eq!(
            String::from_utf16(&root).unwrap(),
            r"D:\",
            "a drive root keeps its separator"
        );
        for refused in [
            r"..\sibling",
            r"\\server\share\x",
            r"\\?\Volume{abc}\",
            r"\\?\UNC\s\x",
        ] {
            assert!(junction_names(Path::new(refused)).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_buffer_lengths_leave_out_the_terminators_they_carry() {
        let buffer = junction_buffer(Path::new(r"C:\x")).unwrap();
        let u16_at = |at: usize| u16::from_le_bytes([buffer[at], buffer[at + 1]]);
        let substitute = r"\??\C:\x".len() * 2;
        let print = r"C:\x".len() * 2;
        assert_eq!(
            u32::from_le_bytes(buffer[0..4].try_into().unwrap()),
            IO_REPARSE_TAG_MOUNT_POINT
        );
        assert_eq!(usize::from(u16_at(4)), 8 + substitute + 2 + print + 2);
        assert_eq!(u16_at(8), 0);
        assert_eq!(usize::from(u16_at(10)), substitute);
        assert_eq!(usize::from(u16_at(12)), substitute + 2);
        assert_eq!(usize::from(u16_at(14)), print);
        assert_eq!(buffer.len(), 8 + usize::from(u16_at(4)));
    }

    #[test]
    fn a_junction_made_here_reads_back_as_a_junction_to_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("payload.txt"), "through the junction").unwrap();
        let link = temp.path().join("link");
        let target = crate::util::paths::canonical(&target).unwrap();
        create_junction(&target, &link).unwrap();
        assert_eq!(reparse_tag(&link).unwrap(), IO_REPARSE_TAG_MOUNT_POINT);
        // `read_link` gives the plain form of `\??\C:\…` wherever one exists —
        // for the original junction as much as for this one, which is what
        // lets verification compare them.
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            std::path::PathBuf::from(crate::util::paths::display_path(&target))
        );
        assert_eq!(
            std::fs::read_to_string(link.join("payload.txt")).unwrap(),
            "through the junction"
        );
        assert_eq!(reparse_tag(&target).unwrap(), 0);
    }
}
