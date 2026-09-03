//! What every pty suite needs: the sandbox launcher, the keys, and the fixtures.
//! Shared by the three suites in this binary.
//!
//! One binary, three files: `cargo test` runs test *binaries* sequentially, so
//! splitting the pty suite into three targets added nineteen seconds of wall
//! time — their fixed keystroke schedules stopped overlapping. Modules keep the
//! files navigable and the schedules interleaved.
//!
//! **The rules every suite in this binary follows**, stated once here rather
//! than at the top of each of them:
//!
//! - They are driven through a real terminal because the app cannot draw
//!   without one, and the defects they cover were only ever visible from a
//!   terminal. Unix only by construction.
//! - Keystrokes are **spaced**, never burst — the app redraws between them, and
//!   the dialoguer flows it bridges to lose most of a burst (`pty::Script`
//!   handles the cadence).
//! - Assertions match **stable text only**, never cursor-positioning escapes.
//!   ratatui redraws only the cells that changed, so the raw transcript is
//!   fragments, not screens: a word can arrive one letter at a time. What the
//!   app showed is read back through `app_screen`, which replays the
//!   transcript into a virtual terminal; `pty::plain` is for what a bridged
//!   flow printed on the main screen, and for a status line that changed
//!   wholesale.
//! - `Confirm` in a bridged flow takes a bare `y`/`n` with no Enter: a trailing
//!   `\r` survives into the next prompt and silently accepts its default.
//! - The app's keys are single characters: `q` quits, `n` creates, `e`
//!   registers, `T` opens templates, `,` opens settings, `Enter` opens the
//!   selected project's action menu. A bridged flow that prints a result
//!   (create, register) ends with `press Enter to return to fastf…`.

use crate::common::{self, Sandbox, pty};
use std::fs;
#[cfg(debug_assertions)]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) const DEADLINE: Duration = Duration::from_secs(25);

/// The app's own keys, named so a script reads as what it does.
#[allow(dead_code)]
pub(crate) const KEY_QUIT: &str = "q";
#[allow(dead_code)]
pub(crate) const KEY_CREATE: &str = "n";
#[allow(dead_code)]
pub(crate) const KEY_REGISTER: &str = "e";
#[allow(dead_code)]
pub(crate) const KEY_TEMPLATES: &str = "T";
#[allow(dead_code)]
pub(crate) const KEY_SETTINGS: &str = ",";
#[allow(dead_code)]
pub(crate) const KEY_SEARCH: &str = "/";
#[allow(dead_code)]
pub(crate) const KEY_COPY_PATH: &str = "y";

pub(crate) fn launch(sb: &Sandbox, script: Vec<pty::Keystroke>) -> (String, i32) {
    pty::run(
        common::FASTF,
        &[],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
        ],
        &script,
        DEADLINE,
    )
}

/// `launch`, with `util::trace` writing to `trace`. Debug builds only — the
/// tracer is compiled out of release, like the failpoints, and so are its
/// callers.
#[cfg(debug_assertions)]
#[allow(dead_code)]
pub(crate) fn launch_traced(
    sb: &Sandbox,
    script: Vec<pty::Keystroke>,
    trace: &Path,
) -> (String, i32) {
    pty::run(
        common::FASTF,
        &[],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("FASTF_TRACE_FILE", trace),
        ],
        &script,
        DEADLINE,
    )
}

/// Plant a project with a chosen creation date and a payload of a known size,
/// so ordering and the Size column are both assertable.
#[allow(dead_code)]
pub(crate) fn plant_dated_project(
    sb: &Sandbox,
    folder: &str,
    id: &str,
    created: &str,
    payload_bytes: usize,
) -> PathBuf {
    let root = sb.plant_project(&sb.base, folder, id);
    let pinfo = root.join("PROJECT_INFO.md");
    let raw = fs::read_to_string(&pinfo).unwrap();
    fs::write(&pinfo, raw.replace("2026-01-01T00:00:00Z", created)).unwrap();
    fs::write(root.join("payload.bin"), vec![7_u8; payload_bytes]).unwrap();
    root
}

/// `launch` on its own thread, for the cases that need a second process running
/// while the app sits on a prompt.
#[allow(dead_code)]
pub(crate) fn launch_detached(
    sb: &Sandbox,
    script: Vec<pty::Keystroke>,
) -> std::thread::JoinHandle<(String, i32)> {
    let install = sb.install.clone();
    let home = sb.tmp.path().to_path_buf();
    std::thread::spawn(move || {
        pty::run(
            common::FASTF,
            &[],
            &[("FASTF_INSTALL_DIR", install.as_path()), ("HOME", &home)],
            &script,
            DEADLINE,
        )
    })
}

/// The last frame the app showed before it left the alternate screen — the
/// transcript up to the final `LeaveAlternateScreen`, replayed into a
/// terminal of the pty's size. A bridged flow leaves and re-enters the
/// alternate screen too, so this is the app's frame after the last of those.
pub(crate) fn app_screen(transcript: &str) -> String {
    const LEAVE: &str = "\x1b[?1049l";
    let end = transcript.rfind(LEAVE).unwrap_or(transcript.len());
    let mut parser = vt100::Parser::new(pty::PTY_ROWS, pty::PTY_COLS, 0);
    parser.process(&transcript.as_bytes()[..end]);
    parser.screen().contents()
}

/// Where the terminal's own caret was left, as `(row, column)` — the thing a
/// person looks for to know where their typing will land. Read from the same
/// `vt100` replay as `app_screen`, because ratatui parks the caret with a
/// cursor-position escape that means nothing outside a terminal.
#[allow(dead_code)]
pub(crate) fn app_cursor(transcript: &str) -> (u16, u16) {
    const LEAVE: &str = "\x1b[?1049l";
    let end = transcript.rfind(LEAVE).unwrap_or(transcript.len());
    let mut parser = vt100::Parser::new(pty::PTY_ROWS, pty::PTY_COLS, 0);
    parser.process(&transcript.as_bytes()[..end]);
    parser.screen().cursor_position()
}

/// The frame the app showed at `until` into a chunked run: the transcript up
/// to that moment, replayed. For a screenshot of a state the script then
/// leaves — a dialog it closes on the way out.
pub(crate) fn screen_at(chunks: &[(Duration, Vec<u8>)], until: Duration) -> String {
    let mut parser = vt100::Parser::new(pty::PTY_ROWS, pty::PTY_COLS, 0);
    parser.process(&pty::until(chunks, until));
    parser.screen().contents()
}

/// How many times `name` was traced.
#[cfg(debug_assertions)]
#[allow(dead_code)]
pub(crate) fn traced(trace: &Path, name: &str) -> usize {
    fs::read_to_string(trace)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == name)
        .count()
}
