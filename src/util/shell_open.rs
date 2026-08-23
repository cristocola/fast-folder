//! Opening a folder in the user's shell, on Windows, without a shell.
//!
//! `cmd /c start "" <path>` worked, and quoted its argument correctly — but
//! `cmd.exe` reconstructs a command line and expands `%VAR%` inside it before
//! anything sees the quoting. A project folder called `%USERPROFILE%` therefore
//! opened the user's home directory instead of itself, and a folder name is
//! user data.
//!
//! `ShellExecuteW` takes the path as a UTF-16 argument. There is no command
//! line, so there is nothing to expand and nothing to split. It honours the
//! user's default folder handler exactly as `start` did.
//!
//! Declared by hand rather than pulling in `windows-sys`: this is one function
//! with three null arguments, and the crate already used the same pattern for
//! `MessageBoxW`.
//!
//! **No COM initialisation is required to open a folder.** `ShellExecuteW`
//! initialises what it needs on the calling thread for the shell verbs; the
//! documented `CoInitializeEx` requirement applies to `ShellExecuteEx` with
//! `SEE_MASK_INVOKEIDLIST`, which this does not use.

use anyhow::{Result, bail};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: *mut core::ffi::c_void,
        lpOperation: *const u16,
        lpFile: *const u16,
        lpParameters: *const u16,
        lpDirectory: *const u16,
        nShowCmd: i32,
    ) -> *mut core::ffi::c_void;
}

const SW_SHOWNORMAL: i32 = 1;

/// Ask the shell to open `path` with its default handler.
pub fn open(path: &Path) -> Result<()> {
    let file = wide(path.as_os_str())?;
    let operation = wide(OsStr::new("open"))?;

    // SAFETY: both pointers are to nul-terminated UTF-16 buffers that outlive
    // the call, and the three null arguments are documented as optional.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // The return value is an `HINSTANCE` for compatibility with 16-bit Windows
    // and is not a real handle. Anything **greater than 32** means success;
    // at or below 32 it is an error code.
    let code = result as usize;
    if code > 32 {
        return Ok(());
    }
    bail!(
        "the shell refused to open {} (ShellExecuteW returned {code})",
        crate::util::paths::display_path(path)
    );
}

/// A nul-terminated UTF-16 buffer.
///
/// An interior nul would silently truncate the path Windows actually opens, so
/// it is an error rather than something to trim.
fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut units: Vec<u16> = value.encode_wide().collect();
    if units.contains(&0) {
        bail!("path contains a NUL character");
    }
    units.push(0);
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::wide;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn a_wide_string_is_nul_terminated_and_round_trips() {
        let units = wide(OsStr::new(r"C:\Users\x\Projects\Ünïcødé")).unwrap();
        assert_eq!(units.last(), Some(&0));
        let back = std::ffi::OsString::from_wide(&units[..units.len() - 1]);
        assert_eq!(back, OsStr::new(r"C:\Users\x\Projects\Ünïcødé"));
    }

    #[test]
    fn an_interior_nul_is_refused_rather_than_truncating_the_path() {
        use std::ffi::OsString;

        let hostile = OsString::from_wide(&[0x0043, 0x003a, 0x0000, 0x0078]);
        let error = wide(&hostile).unwrap_err().to_string();
        assert!(error.contains("NUL"), "unexpected error: {error}");
    }
}
