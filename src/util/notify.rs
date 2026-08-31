//! A desktop notification, best-effort.
//!
//! When fastf acts on a request from a launcher there is no terminal to say so
//! in, and an action nobody can see is indistinguishable from nothing having
//! happened — the exact complaint that produced `fastf copy`. `notify-send` is
//! present on every desktop that has a launcher worth the name; where it is
//! missing, the answer is that the user gets no notification, not an error.
//!
//! Unix only, and it never prints: a notifier that wrote to stdout would be
//! doing the thing it exists to work around.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::util::paths;

/// Show a desktop notification. `true` if one was dispatched.
pub fn notify(summary: &str, body: &str) -> bool {
    if paths::find_on_path("notify-send").is_none() {
        return false;
    }
    Command::new("notify-send")
        .arg("-a")
        .arg("fastf")
        .arg(summary)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Same reason as the clipboard and the relaunch: the launcher reaps its
        // process group, and the notification daemon's client should not be in
        // ours when it does.
        .process_group(0)
        .spawn()
        .is_ok()
}
