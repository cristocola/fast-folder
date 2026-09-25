//! The log: everything fastf did, one line per event, kept on disk.
//!
//! **Messages and logs are two things.** A message ([`crate::util::messages`])
//! is a sentence for a person, with a recommendation. A log line is a fact with
//! a time on it: every step of a move with its count, every warning `diag`
//! gave, every file a job touched when the level asks for that much. The
//! messages are what the app's status line said; the log is what to read when
//! a message was not enough.
//!
//! `<data dir>/logs/fastf.log`, appended by every fastf process at once. Each
//! event is written with **one `write` on a file opened for appending**
//! (`O_APPEND`, `FILE_APPEND_DATA`), so lines from two processes interleave
//! whole and never tear into each other. Past [`ROTATE_BYTES`] the file is
//! rotated — `fastf.log.1`, `.2`, `.3`, the oldest dropped — by whichever
//! process takes the rotation lock first; one that finds it held skips, since
//! the holder is doing the same work.
//!
//! A line is `2026-09-25T14:03:11.123Z INFO  <job|-> <pid> text`. A text of
//! several lines keeps its line breaks, each further line indented four
//! spaces, so a reader tells a continuation from the next event.
//!
//! **The level is set, never read here** ([`set_level`]): `util` may not read
//! the configuration, so `main` passes `log_level` in once it has loaded it.
//! Nothing here may fail an operation or print: a log that cannot be written
//! is skipped.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

/// How much is written: a line at `level` is written when `level` is at or
/// above the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    /// As a threshold: nothing is written.
    Off = 4,
}

impl Level {
    /// The names `config set log-level` takes.
    pub const NAMES: [&'static str; 5] = ["debug", "info", "warn", "error", "off"];

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "debug" => Some(Self::Debug),
            "" | "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            "off" | "none" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    /// The word a log line carries, padded so the text lines up.
    fn tag(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO ",
            Self::Warn => "WARN ",
            Self::Error => "ERROR",
            Self::Off => "",
        }
    }
}

/// A log file past this is rotated.
pub const ROTATE_BYTES: u64 = 4 * 1024 * 1024;
/// How many rotated files are kept beside the live one.
pub const KEEP: usize = 3;

static THRESHOLD: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// The job this process is running, whose own log takes every line at every
/// level — a job's log is its whole story, whatever the central threshold.
static JOB: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);

/// Write lines at `level` and above to the central log from now on.
pub fn set_level(level: Level) {
    THRESHOLD.store(level as u8, Ordering::Relaxed);
}

/// Whether a line at `level` would reach the central log — so a caller can
/// skip building one that would not, a line per file of a large move.
pub fn enabled(level: Level) -> bool {
    if level == Level::Off {
        return false;
    }
    level as u8 >= THRESHOLD.load(Ordering::Relaxed)
        || JOB.lock().map(|job| job.is_some()).unwrap_or(false)
}

/// Every later line also goes to `log`, at every level, tagged `id`; `None`
/// stops it. A job's worker process says this once, at its start.
pub fn set_job(job: Option<(String, PathBuf)>) {
    if let Ok(mut slot) = JOB.lock() {
        *slot = job;
    }
}

pub fn debug(text: impl AsRef<str>) {
    write(Level::Debug, text.as_ref());
}

pub fn info(text: impl AsRef<str>) {
    write(Level::Info, text.as_ref());
}

pub fn warn(text: impl AsRef<str>) {
    write(Level::Warn, text.as_ref());
}

pub fn error(text: impl AsRef<str>) {
    write(Level::Error, text.as_ref());
}

/// The data directory's log folder, when there is a data directory.
pub fn dir() -> Option<PathBuf> {
    // A unit test that has not sandboxed the data directory must not write
    // into the developer's own: `cargo test` would fill the real log.
    #[cfg(test)]
    std::env::var_os("FASTF_INSTALL_DIR")?;
    crate::util::paths::try_install_dir()
        .ok()
        .map(|(dir, _)| dir.join("logs"))
}

/// The central log's path.
pub fn path() -> Option<PathBuf> {
    dir().map(|dir| dir.join("fastf.log"))
}

/// Write one event.
pub fn write(level: Level, text: &str) {
    if level == Level::Off {
        return;
    }
    let job = JOB.lock().ok().and_then(|job| job.clone());
    let central = level as u8 >= THRESHOLD.load(Ordering::Relaxed);
    if !central && job.is_none() {
        return;
    }
    let line = format_line(
        &crate::util::time::now_iso8601_millis(),
        level,
        job.as_ref().map(|(id, _)| id.as_str()),
        std::process::id(),
        text,
    );
    if central && let Some(path) = path() {
        append(&path, &line, ROTATE_BYTES);
    }
    if let Some((_, path)) = job {
        append(&path, &line, u64::MAX);
    }
}

/// One event as the bytes a log holds, its newline included.
pub fn format_line(stamp: &str, level: Level, job: Option<&str>, pid: u32, text: &str) -> String {
    let mut line = format!("{stamp} {} {} {pid} ", level.tag(), job.unwrap_or("-"));
    let mut lines = text.trim_end().split('\n');
    line.push_str(lines.next().unwrap_or("").trim_end_matches('\r'));
    for more in lines {
        line.push_str("\n    ");
        line.push_str(more.trim_end_matches('\r'));
    }
    line.push('\n');
    line
}

/// Append `line` to `path` in one write, rotating first when the file has
/// grown past `rotate_at`. Best effort: every failure is swallowed.
pub(crate) fn append(path: &Path, line: &str, rotate_at: u64) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(path).is_ok_and(|meta| meta.len() > rotate_at) {
        rotate(path, rotate_at);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// `path` → `path.1` → … → `path.KEEP`, the oldest dropped, under a lock only
/// one process takes; a process that finds it held leaves the work to the
/// holder.
pub(crate) fn rotate(path: &Path, rotate_at: u64) {
    let lock = path.with_extension("rotate.lock");
    let Ok(Some(_held)) = crate::util::lockfile::DataLock::try_acquire_at(&lock) else {
        return;
    };
    // Rotated by another process a moment ago, between our look and the lock.
    if std::fs::metadata(path).map_or(true, |meta| meta.len() <= rotate_at) {
        return;
    }
    let numbered = |n: usize| PathBuf::from(format!("{}.{n}", path.display()));
    let _ = std::fs::remove_file(numbered(KEEP));
    for n in (1..KEEP).rev() {
        let _ = std::fs::rename(numbered(n), numbered(n + 1));
    }
    let _ = std::fs::rename(path, numbered(1));
}

/// The last `count` events of the log at `path`, oldest first, each with its
/// continuation lines. Reads only the end of the file, so a large log costs
/// what is shown. A last line with no newline is a write still in progress —
/// or torn by a crash — and is left out.
pub fn tail(path: &Path, count: usize) -> Vec<String> {
    const CHUNK: u64 = 256 * 1024;
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return Vec::new();
    };
    let mut start = len.saturating_sub(CHUNK);
    loop {
        let mut bytes = Vec::new();
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut events = events_of(&text, start > 0);
        if events.len() > count || start == 0 {
            let skip = events.len().saturating_sub(count);
            return events.split_off(skip);
        }
        start = start.saturating_sub(CHUNK * 4);
    }
}

/// Split log text into events: a line that begins with a space continues the
/// one before. `partial_head` drops the first line, which a read that began
/// mid-file may have cut.
fn events_of(text: &str, partial_head: bool) -> Vec<String> {
    let complete = match text.rfind('\n') {
        Some(end) => &text[..=end],
        None => return Vec::new(),
    };
    let mut lines = complete.lines();
    if partial_head {
        lines.next();
    }
    let mut events: Vec<String> = Vec::new();
    for line in lines {
        if line.starts_with(' ') {
            if let Some(last) = events.last_mut() {
                last.push('\n');
                last.push_str(line);
            }
        } else if !line.is_empty() {
            events.push(line.to_string());
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_says_when_how_bad_which_job_which_process_and_what() {
        let line = format_line(
            "2026-09-25T14:03:11.123Z",
            Level::Info,
            None,
            42,
            "moved ID0001",
        );
        assert_eq!(line, "2026-09-25T14:03:11.123Z INFO  - 42 moved ID0001\n");
        let line = format_line("T", Level::Warn, Some("job1"), 7, "first\nsecond\r\n");
        assert_eq!(line, "T WARN  job1 7 first\n    second\n");
    }

    #[test]
    fn levels_parse_the_names_config_takes() {
        for name in Level::NAMES {
            assert_eq!(Level::parse(name).unwrap().name(), name);
        }
        assert_eq!(Level::parse(""), Some(Level::Info));
        assert_eq!(Level::parse("loud"), None);
        assert!(Level::Debug < Level::Info && Level::Error < Level::Off);
    }

    #[test]
    fn the_tail_keeps_continuations_and_leaves_out_a_torn_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fastf.log");
        let mut text = String::new();
        for n in 0..5 {
            text.push_str(&format_line(
                "T",
                Level::Info,
                None,
                1,
                &format!("event {n}\nmore"),
            ));
        }
        text.push_str("T INFO  - 1 half-writ");
        std::fs::write(&path, text).unwrap();

        let events = tail(&path, 2);
        assert_eq!(events.len(), 2);
        assert!(events[0].contains("event 3") && events[0].ends_with("    more"));
        assert!(events[1].contains("event 4"));
        assert!(tail(&dir.path().join("none"), 3).is_empty());
    }

    #[test]
    fn a_tail_longer_than_one_read_is_found_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fastf.log");
        let mut text = String::new();
        for n in 0..20_000 {
            text.push_str(&format_line(
                "2026-09-25T14:03:11.123Z",
                Level::Debug,
                None,
                1,
                &format!("entry {n}"),
            ));
        }
        std::fs::write(&path, text).unwrap();
        let events = tail(&path, 15_000);
        assert_eq!(events.len(), 15_000);
        assert!(events[0].ends_with("entry 5000"), "{}", events[0]);
        assert!(events[14_999].ends_with("entry 19999"));
    }

    #[test]
    fn rotation_keeps_the_newest_and_drops_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fastf.log");
        for round in 0..(KEEP + 2) {
            std::fs::write(&path, format!("round {round}\n").repeat(400)).unwrap();
            rotate(&path, 0);
        }
        let read = |n: usize| {
            std::fs::read_to_string(format!("{}.{n}", path.display()))
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        };
        assert!(!path.exists(), "the live file was rotated");
        assert_eq!(read(1), format!("round {}", KEEP + 1));
        assert_eq!(read(KEEP), "round 2");
        assert!(!PathBuf::from(format!("{}.{}", path.display(), KEEP + 1)).exists());
    }
}
