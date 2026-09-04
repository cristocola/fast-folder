//! Open a terminal emulator whose shell starts in a given directory.
//!
//! The other thing fastf asks of a terminal. `relaunch` reruns *fastf* inside
//! an emulator when a launcher gave it none; this module opens an emulator
//! running the user's *shell* at a project's folder — `fastf term` and the
//! action menu's "Open terminal here".
//!
//! The `cwd` conventions live in `relaunch::TERMINALS` beside the `run` ones,
//! so the two lists cannot drift apart. `xdg-terminal-exec` is deliberately
//! absent here although the relaunch tries it first: the spec hands it a
//! command line to run, not a directory, and a daemonized default terminal
//! (gnome-terminal's client/server split) ignores the inherited working
//! directory — a window that silently opens at `$HOME` is worse than skipping
//! the resolver.
//!
//! Like `relaunch`, this module never prints and never reads `Config`; the
//! caller resolves the terminal preference and passes it in.

use std::path::Path;

#[cfg(unix)]
use anyhow::{Result, bail};
#[cfg(unix)]
use std::ffi::OsString;

#[cfg(unix)]
use crate::util::paths;
#[cfg(unix)]
use crate::util::relaunch::{CwdStyle, TERMINALS};

/// Every argv that could open a terminal at `dir`, in the order to try them.
///
/// Pure: it probes nothing and spawns nothing, so the exact argv for each
/// emulator is unit-testable. An unknown preference gets the *bare* argv — no
/// guessed flag, unlike the relaunch's `-e` default, because a wrong flag
/// aborts an emulator that does not know it while the spawner's `current_dir`
/// usually carries the directory anyway.
#[cfg(unix)]
fn terminal_at_commands(preference: Option<&str>, dir: &Path) -> Vec<Vec<OsString>> {
    let mut candidates = Vec::new();

    if let Some(name) = preference.map(str::trim).filter(|n| !n.is_empty()) {
        // Matched by basename against the table so `/usr/bin/konsole` still
        // gets `--workdir`, exactly as the relaunch matches its `run` style.
        let base = std::path::Path::new(name)
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or(name);
        let style = TERMINALS
            .iter()
            .find(|e| e.program == base)
            .map(|e| &e.cwd)
            .unwrap_or(&CwdStyle::Inherit);
        candidates.push(build(name, style, dir));
    }

    for emulator in TERMINALS {
        candidates.push(build(emulator.program, &emulator.cwd, dir));
    }
    candidates
}

#[cfg(unix)]
fn build(program: &str, style: &CwdStyle, dir: &Path) -> Vec<OsString> {
    let mut argv = vec![OsString::from(program)];
    match style {
        CwdStyle::Flag(flag) => {
            argv.push(OsString::from(*flag));
            argv.push(dir.as_os_str().to_os_string());
        }
        CwdStyle::Subcommand(sub, flag) => {
            argv.push(OsString::from(*sub));
            argv.push(OsString::from(*flag));
            argv.push(dir.as_os_str().to_os_string());
        }
        CwdStyle::Inherit => {}
    }
    argv
}

/// Spawn a terminal emulator whose shell starts at `dir`.
///
/// `preference` names a program the caller resolved (`terminal` in the config,
/// else `$TERMINAL`); `None` means probe. The spawn discipline is the
/// relaunch's: candidates not on `PATH` are skipped, the child gets null stdio
/// and its own process group, and it is dropped rather than waited on — the
/// window is now the user's. Every spawn also sets `current_dir(dir)`, which is
/// what carries the directory for xterm, for an unknown preference, and past
/// any emulator that ignores its own flag.
///
/// **The relaunch marker is dropped here**, and in [`exec_shell_at`]. It says
/// "this fastf process is a rerun" and it is inherited, so a window opened from
/// a relaunched fastf would hand it to its shell, that shell would hand it to
/// everything typed into it, and one of those is a build that runs fastf's own
/// suite — where a set `FASTF_RELAUNCHED` turns off the very behaviour the
/// suite is checking. Nothing fastf gives the user should carry fastf's
/// bookkeeping.
#[cfg(unix)]
pub fn open_terminal_at(preference: Option<&str>, dir: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    // A terminal window needs a desktop to open on; without one every
    // emulator on PATH would start, die at once, and be reported as opened.
    #[cfg(not(target_os = "macos"))]
    if !crate::util::tty::has_display() {
        bail!(
            "no display — a terminal window needs a desktop session (DISPLAY or WAYLAND_DISPLAY)"
        );
    }
    let mut tried = Vec::new();
    for candidate in terminal_at_commands(preference, dir) {
        let (program, rest) = candidate.split_first().expect("candidate is never empty");
        if paths::find_on_path(&program.to_string_lossy()).is_none() {
            continue;
        }
        tried.push(program.to_string_lossy().into_owned());

        let spawned = Command::new(program)
            .args(rest)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // The terminal must outlive us — same reason as the relaunch.
            .process_group(0)
            .env_remove(crate::util::relaunch::RELAUNCHED_VAR)
            .spawn();

        if spawned.is_ok() {
            return Ok(());
        }
    }

    if tried.is_empty() {
        bail!("no terminal emulator found on PATH");
    }
    bail!(
        "no terminal emulator could be started (tried {})",
        tried.join(", ")
    )
}

/// Replace this process with the user's shell, started at `dir`.
///
/// For the window fastf already owns: a relaunch opened it just to show a
/// picker, and becoming the shell there *is* "open a terminal at the project" —
/// a second window would strand this one. `$SHELL`, then `/bin/sh` if that
/// fails to exec. Returns only on failure.
///
/// The shell is the user's from here on, so it does not inherit the relaunch
/// marker — see [`open_terminal_at`] for what inheriting it costs.
#[cfg(unix)]
pub fn exec_shell_at(dir: &Path) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let marker = crate::util::relaunch::RELAUNCHED_VAR;
    let shell = std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/sh"));
    let err = Command::new(&shell)
        .current_dir(dir)
        .env_remove(marker)
        .exec();
    if shell.as_os_str() != "/bin/sh" {
        let err = Command::new("/bin/sh")
            .current_dir(dir)
            .env_remove(marker)
            .exec();
        return anyhow::Error::from(err).context("could not start /bin/sh");
    }
    anyhow::Error::from(err).context(format!("could not start {}", shell.to_string_lossy()))
}

/// Spawn a terminal at `dir` on Windows: Windows Terminal when it is there, a
/// new `cmd` console otherwise.
///
/// `wt` is tried by spawning rather than probed on `PATH`: `wt.exe` is an app
/// execution alias — a reparse point whose metadata says nothing useful — so
/// `find_on_path`'s executable check may refuse a `wt` that runs fine.
#[cfg(windows)]
pub fn open_terminal_at(_preference: Option<&str>, dir: &Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    if Command::new("wt.exe")
        .arg("-d")
        .arg(dir)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // `/K` keeps the console open with a prompt; `current_dir` is the start
    // directory, so no `cd /d` string needs building.
    Command::new("cmd.exe")
        .arg("/K")
        .current_dir(dir)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow::Error::from(e).context("could not start a console"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn commands(preference: Option<&str>) -> Vec<Vec<String>> {
        let dir = PathBuf::from("/mnt/projects/my stuff");
        terminal_at_commands(preference, &dir)
            .iter()
            .map(|c| c.iter().map(|a| a.to_string_lossy().into_owned()).collect())
            .collect()
    }

    /// Each emulator's directory flag is the one *it* means, and a directory
    /// containing a space stays one argument.
    #[test]
    fn every_emulator_gets_the_cwd_convention_it_actually_wants() {
        let all = commands(None);
        let find = |program: &str| {
            all.iter()
                .find(|c| c[0] == program)
                .unwrap_or_else(|| panic!("{program} missing from the candidates"))
                .clone()
        };

        assert_eq!(
            find("konsole"),
            ["konsole", "--workdir", "/mnt/projects/my stuff"]
        );
        assert_eq!(
            find("gnome-terminal"),
            [
                "gnome-terminal",
                "--working-directory",
                "/mnt/projects/my stuff"
            ]
        );
        assert_eq!(
            find("xfce4-terminal"),
            [
                "xfce4-terminal",
                "--working-directory",
                "/mnt/projects/my stuff"
            ]
        );
        assert_eq!(
            find("alacritty"),
            ["alacritty", "--working-directory", "/mnt/projects/my stuff"]
        );
        assert_eq!(
            find("kitty"),
            ["kitty", "--directory", "/mnt/projects/my stuff"]
        );
        assert_eq!(
            find("foot"),
            ["foot", "--working-directory", "/mnt/projects/my stuff"]
        );
        assert_eq!(
            find("wezterm"),
            ["wezterm", "start", "--cwd", "/mnt/projects/my stuff"]
        );
        // xterm has no directory flag: the spawner's `current_dir` carries it.
        assert_eq!(find("xterm"), ["xterm"]);
    }

    /// `xdg-terminal-exec` resolves a *command runner*, not a directory, so it
    /// must never appear here — see the module doc.
    #[test]
    fn the_xdg_resolver_is_deliberately_absent() {
        assert!(
            commands(None).iter().all(|c| c[0] != "xdg-terminal-exec"),
            "xdg-terminal-exec cannot be told a directory"
        );
    }

    /// A configured terminal leads and keeps its own flag when fastf knows it;
    /// an unknown one gets the bare argv — no guessed flag, `current_dir` does
    /// the work — and never displaces the fallbacks.
    #[test]
    fn a_configured_terminal_leads_and_an_unknown_one_gets_no_flag() {
        let all = commands(Some("/usr/bin/konsole"));
        assert_eq!(
            all[0],
            ["/usr/bin/konsole", "--workdir", "/mnt/projects/my stuff"],
            "a full path must still be recognised as konsole by its basename"
        );
        assert!(all.iter().any(|c| c[0] == "xterm"));

        let unknown = commands(Some("my-terminal"));
        assert_eq!(unknown[0], ["my-terminal"]);

        // An empty or whitespace preference is not a program name.
        assert_eq!(commands(Some("   "))[0][0], "konsole");
    }
}
