//! What every pty suite needs: the sandbox launcher, the menu indices, and the
//! fixtures. Shared by the three suites in this binary.
//!
//! One binary, three files: `cargo test` runs test *binaries* sequentially, so
//! splitting the pty suite into three targets added nineteen seconds of wall
//! time — their fixed keystroke schedules stopped overlapping. Modules keep the
//! files navigable and the schedules interleaved.

use crate::common::{self, Sandbox, pty};
use std::fs;
#[cfg(debug_assertions)]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) const DEADLINE: Duration = Duration::from_secs(25);

/// Main-menu indices: Create, Projects, Search, Register, Templates, Settings, Quit.
#[allow(dead_code)]
pub(crate) const MENU_PROJECTS: usize = 1;
#[allow(dead_code)]
pub(crate) const MENU_SEARCH: usize = 2;
#[allow(dead_code)]
pub(crate) const MENU_REGISTER: usize = 3;
#[allow(dead_code)]
pub(crate) const MENU_TEMPLATES: usize = 4;
#[allow(dead_code)]
pub(crate) const MENU_SETTINGS: usize = 5;
#[allow(dead_code)]
pub(crate) const MENU_QUIT: usize = 6;

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
/// while the menu sits on a prompt.
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
