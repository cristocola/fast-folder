//! The Windows API's numbers that fastf names in more than one place: error
//! codes, and the flags a folder or a link is opened with. A number used in
//! one place stays beside its use.

/// A permission refuses — or a program holds the file while it is deleted or
/// replaced, which Windows reports the same way.
pub const ERROR_ACCESS_DENIED: i32 = 5;
/// Another process has the file open and will not share it.
pub const ERROR_SHARING_VIOLATION: i32 = 32;
/// A byte-range lock is held on the file.
pub const ERROR_LOCK_VIOLATION: i32 = 33;
/// A folder still has entries: for a moment, while a scanner lets go of them.
pub const ERROR_DIR_NOT_EMPTY: i32 = 145;

/// `CreateFileW`: open what is there, create nothing.
pub const OPEN_EXISTING: u32 = 3;
/// A folder opens only with this.
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
/// Open a link itself, never what it points at.
pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`: hold nobody up.
pub const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
