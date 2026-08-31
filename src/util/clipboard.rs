//! Put a string on the system clipboard, if this system has one.
//!
//! No dependency and no daemon: fastf shells out to whichever of the usual tools
//! is actually installed, and says so. There is no portable clipboard on Linux —
//! Wayland and X11 disagree, and a headless session has neither — so "no
//! clipboard here" is a normal answer rather than a failure, and the caller
//! prints the value instead.

use std::io::Write;
use std::process::{Command, Stdio};

/// The tools to try, in order, with the arguments each needs.
///
/// Wayland before X11 because a Wayland session usually also has `xclip` through
/// XWayland, where it writes to a clipboard nothing reads.
const TOOLS: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
    ("clip.exe", &[]),
    ("clip", &[]),
    ("pbcopy", &[]),
];

/// Copy `text`, returning the tool that took it.
///
/// `None` means no tool was found or none accepted it — not an error.
pub fn copy(text: &str) -> Option<&'static str> {
    for (tool, args) in TOOLS {
        if crate::util::paths::find_on_path(tool).is_none() {
            continue;
        }
        if feed(tool, args, text) {
            return Some(tool);
        }
    }
    None
}

/// Run one tool with `text` on its stdin. `false` if it could not be run or
/// exited non-zero, so the next candidate gets a turn.
fn feed(tool: &str, args: &[&str], text: &str) -> bool {
    let mut command = Command::new(tool);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // A Wayland/X11 clipboard tool does not copy and exit — it forks and keeps
    // running, because on those systems the *process* that offered the
    // selection is what serves it to whoever pastes. That fork inherits our
    // process group, and a desktop launcher reaps the group when the command it
    // started exits: the owner dies, and the paste comes back empty. Its own
    // group detaches it from ours. Looks removable; is not.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let Ok(mut child) = command.spawn() else {
        return false;
    };
    if let Some(stdin) = child.stdin.as_mut()
        && stdin.write_all(text.as_bytes()).is_err()
    {
        let _ = child.kill();
        return false;
    }
    // Dropped before the wait, or a tool that reads to EOF never returns.
    drop(child.stdin.take());
    child.wait().map(|status| status.success()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use crate::util::paths::find_on_path;

    #[test]
    fn which_finds_a_real_executable_and_not_a_made_up_one() {
        assert!(
            find_on_path("fastf-definitely-not-installed-9f3a").is_none(),
            "a name nothing provides must not resolve"
        );
        #[cfg(unix)]
        assert!(
            find_on_path("sh").is_some(),
            "every unix has a shell on PATH; if this fails the lookup is broken"
        );
    }
}
