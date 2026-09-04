//! Re-run fastf inside a terminal emulator when it was launched without one.
//!
//! fastf is started two ways. From a shell there is always a terminal. From a
//! desktop launcher — krunner, rofi, a `.desktop` entry, Win+R's equivalents —
//! there is **no terminal at all**: stdin is `/dev/null`, stdout and stderr are
//! journald sockets, and everything a command prints is written to nobody.
//! `fastf search rust` from a launcher was a bouncing cursor and then nothing.
//!
//! So where fastf has text to show or a question to ask and provably nowhere to
//! put it, it opens a terminal and runs itself again inside it. This is a
//! deliberate departure from the "refuse and name the escape hatch" convention
//! everywhere else in the crate, and it is fenced accordingly: see
//! [`headless_gui_session`] for the full rule.
//!
//! Unix only, like `shell_open` is Windows only. On Windows a console
//! application launched from the shell surface already gets a console.
//!
//! This module never prints — spawning from `util` is the `clipboard` precedent,
//! printing from it is not — and never reads `Config`; the caller resolves the
//! terminal preference and passes it in.

use anyhow::{Result, bail};
use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::util::paths;

/// Set on the child, so the rerun cannot relaunch itself. Internal.
///
/// It says only "do not relaunch", never "this process *is* the rerun": it is
/// inherited by every descendant, and a wrong inheritance here can only suppress
/// a window, which is the safe direction. The positive claim is [`RELAUNCHED_FLAG`].
pub const RELAUNCHED_VAR: &str = "FASTF_RELAUNCHED";

/// Put on the rerun's own command line, ahead of the argv the user typed.
///
/// argv, not an environment variable, because **this one is a claim about the
/// process rather than about the session**: only the program the emulator was
/// asked to run receives it, and nothing that program starts inherits it. Every
/// bug the variable caused came from a descendant answering a question only the
/// rerun itself may answer.
pub const RELAUNCHED_FLAG: &str = "--relaunched";
/// Set by the user to turn the whole mechanism off. Public, documented.
pub const NO_RELAUNCH_VAR: &str = "FASTF_NO_RELAUNCH";

/// How an emulator wants the command to run appended to its own argv.
pub(crate) enum ArgStyle {
    /// A flag introducing the command: `konsole -e fastf search x`.
    Flag(&'static str),
    /// The command with no flag at all: `kitty fastf search x`.
    Trailing,
}

/// How an emulator is told which directory its shell starts in — the other
/// thing fastf asks of a terminal (`term_open`). Always the two-token
/// `--flag <dir>` form: GOption and getopt_long both take it, and it keeps the
/// argv free of `OsString` concatenation.
pub(crate) enum CwdStyle {
    /// A flag naming the directory: `konsole --workdir <dir>`.
    Flag(&'static str),
    /// A subcommand, then the flag: `wezterm start --cwd <dir>`.
    Subcommand(&'static str, &'static str),
    /// No flag at all (xterm): the spawner's `current_dir` is what carries it.
    Inherit,
}

/// One emulator fastf knows how to drive: its program name and both argv
/// conventions, kept in one row so the two never drift apart.
pub(crate) struct Emulator {
    pub(crate) program: &'static str,
    pub(crate) run: ArgStyle,
    pub(crate) cwd: CwdStyle,
}

const fn emulator(program: &'static str, run: ArgStyle, cwd: CwdStyle) -> Emulator {
    Emulator { program, run, cwd }
}

/// The emulators fastf knows how to drive, in probe order.
///
/// The `run` flags are not interchangeable and the wrong one silently does
/// something else:
/// - `gnome-terminal` wants `--`; its `-e` is deprecated and takes a *single
///   string* it then splits itself.
/// - `xfce4-terminal`'s `-e` shell-parses one string too, so it must be `-x`.
/// - `xterm`'s `-e` must be its last option.
/// - `kitty` and `foot` take the command as trailing argv with no flag.
///
/// The `cwd` flags are each emulator's own long option; xterm has none, and its
/// shell starts in the inherited working directory instead.
pub(crate) const TERMINALS: &[Emulator] = &[
    emulator("konsole", ArgStyle::Flag("-e"), CwdStyle::Flag("--workdir")),
    emulator(
        "gnome-terminal",
        ArgStyle::Flag("--"),
        CwdStyle::Flag("--working-directory"),
    ),
    emulator(
        "xfce4-terminal",
        ArgStyle::Flag("-x"),
        CwdStyle::Flag("--working-directory"),
    ),
    emulator(
        "alacritty",
        ArgStyle::Flag("-e"),
        CwdStyle::Flag("--working-directory"),
    ),
    emulator("kitty", ArgStyle::Trailing, CwdStyle::Flag("--directory")),
    emulator(
        "foot",
        ArgStyle::Trailing,
        CwdStyle::Flag("--working-directory"),
    ),
    emulator(
        "wezterm",
        ArgStyle::Flag("start"),
        CwdStyle::Subcommand("start", "--cwd"),
    ),
    emulator("xterm", ArgStyle::Flag("-e"), CwdStyle::Inherit),
];

/// The XDG default-terminal resolver, tried before probing the table: on a
/// system that has it, it knows the user's actual choice. Trailing argv.
const XDG_RESOLVER: &str = "xdg-terminal-exec";

/// Was this process started by a desktop launcher inside a graphical session,
/// with nowhere at all to write?
///
/// Every one of these must hold, and each is load-bearing:
///
/// 1. **None of the three streams is a terminal.** One that is means there is
///    somewhere to write already.
/// 2. **Both stdout and stderr are a socket, a character device, or closed.**
///    A regular file or a FIFO means somebody is reading — a redirect, a pipe,
///    `nohup`, cron, the test harness — and those must keep today's behaviour
///    exactly. A closed descriptor (`EBADF`) is provably nobody.
/// 3. **A display is set.** No display, no terminal to open.
/// 4. **`SSH_CONNECTION` is unset.** A remote session's display may be
///    forwarded, and opening a window over X11 forwarding for a command the
///    user piped is not a favour.
/// 5. **Neither relaunch variable is set** — the loop guard and the off switch.
///
/// `INVOCATION_ID` and `JOURNAL_STREAM` are *not* used: a systemd-managed
/// desktop sets them for everything the session spawns, so they say nothing
/// about who the caller is.
///
/// Two accepted misfires, both with three documented ways out (`--plain`,
/// `FASTF_NO_RELAUNCH=1`, `terminal = "none"`): a systemd user service running
/// an interactive fastf command with the session display imported, which is
/// byte-for-byte the launcher environment; and cron with `>/dev/null 2>&1`
/// *plus* an exported display.
pub fn headless_gui_session() -> bool {
    use std::io::IsTerminal;

    if std::io::stdin().is_terminal()
        || std::io::stdout().is_terminal()
        || std::io::stderr().is_terminal()
    {
        return false;
    }
    if !stream_has_no_reader(libc::STDOUT_FILENO) || !stream_has_no_reader(libc::STDERR_FILENO) {
        return false;
    }
    if !has_display() {
        return false;
    }
    if std::env::var_os("SSH_CONNECTION").is_some() {
        return false;
    }
    if std::env::var_os(RELAUNCHED_VAR).is_some() || std::env::var_os(NO_RELAUNCH_VAR).is_some() {
        return false;
    }
    true
}

fn has_display() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// Could anything be reading this descriptor?
///
/// `false` for a regular file or a FIFO — a redirect or a pipe, where the whole
/// point is that the bytes are kept. `true` for a socket (journald), a character
/// device (`/dev/null`, `/dev/console`) or a descriptor that is closed outright.
///
/// Raw `fstat` rather than anything from std: this asks about the *kind* of the
/// open file, which `File::metadata` would answer only by taking ownership of a
/// descriptor we do not own. `libc` is already a unix dependency.
fn stream_has_no_reader(fd: i32) -> bool {
    // SAFETY: `fstat` writes into a caller-provided `stat` and reads nothing
    // else; a bad fd is reported as -1/EBADF rather than being dereferenced.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        // EBADF: nothing is open there at all, so nothing can be reading it.
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::EBADF);
    }
    let kind = st.st_mode & libc::S_IFMT;
    kind == libc::S_IFSOCK || kind == libc::S_IFCHR
}

/// Re-run this process inside a terminal emulator.
///
/// `preference` names a program the caller resolved (`terminal` in the config,
/// else `$TERMINAL`); `None` means probe. `Ok(())` means a terminal owns the
/// rerun and the caller should return without doing the work itself. `Err` means
/// no emulator could be started, and the caller falls through to today's plain
/// behaviour rather than failing.
pub fn respawn_in_terminal(preference: Option<&str>) -> Result<()> {
    let exe = std::env::current_exe()
        .ok()
        .map(OsString::from)
        // `current_exe` can fail on an unusual filesystem; argv[0] is what a
        // shell would have used anyway.
        .or_else(|| std::env::args_os().next())
        .unwrap_or_else(|| OsString::from("fastf"));
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();

    let mut tried = Vec::new();
    for candidate in candidate_commands(preference, &exe, &args) {
        let (program, rest) = candidate.split_first().expect("candidate is never empty");
        if paths::find_on_path(&program.to_string_lossy()).is_none() {
            continue;
        }
        tried.push(program.to_string_lossy().into_owned());

        // The argv is passed as argv. Nothing here is ever handed to a shell:
        // a project name may legally contain `;`, `&`, `$` and a backtick.
        let spawned = Command::new(program)
            .args(rest)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // The launcher reaps the process group it started; the terminal
            // must outlive us, so it gets its own — same reason as the
            // clipboard's fork.
            .process_group(0)
            .env(RELAUNCHED_VAR, "1")
            .spawn();

        // The child is deliberately dropped rather than waited on: the terminal
        // is now the user's window and this process is finished.
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

/// Every command that could open a terminal here, in the order to try them.
///
/// Pure: it probes nothing and spawns nothing, so the exact argv for each
/// emulator is unit-testable. `respawn_in_terminal` skips the ones that are not
/// installed.
fn candidate_commands(
    preference: Option<&str>,
    exe: &OsStr,
    args: &[OsString],
) -> Vec<Vec<OsString>> {
    let mut candidates = Vec::new();

    if let Some(name) = preference.map(str::trim).filter(|n| !n.is_empty()) {
        // A configured value names a *program*, not a command line: an embedded
        // argument would have to be split, and splitting a command line is how
        // a folder called `My Stuff` becomes two arguments.
        //
        // Matched by basename against the table so `/usr/bin/konsole` still gets
        // `-e`; anything unknown gets the xterm-compatible convention, which is
        // what almost every emulator accepts.
        let base = std::path::Path::new(name)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or(name);
        let style = TERMINALS
            .iter()
            .find(|e| e.program == base)
            .map(|e| &e.run)
            .unwrap_or(&ArgStyle::Flag("-e"));
        candidates.push(build(name, style, exe, args));
    }

    candidates.push(build(XDG_RESOLVER, &ArgStyle::Trailing, exe, args));
    for emulator in TERMINALS {
        candidates.push(build(emulator.program, &emulator.run, exe, args));
    }
    candidates
}

fn build(program: &str, style: &ArgStyle, exe: &OsStr, args: &[OsString]) -> Vec<OsString> {
    let mut argv = vec![OsString::from(program)];
    if let ArgStyle::Flag(flag) = style {
        argv.push(OsString::from(*flag));
        // `wezterm start --` is two tokens: the subcommand, then the separator.
        if *flag == "start" {
            argv.push(OsString::from("--"));
        }
    }
    argv.push(exe.to_os_string());
    // Ahead of everything the user typed: a global flag, so it is read whatever
    // the subcommand is, and never swallowed by a `trailing_var_arg` positional.
    argv.push(OsString::from(RELAUNCHED_FLAG));
    argv.extend(args.iter().cloned());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    fn argv(candidate: &[OsString]) -> Vec<String> {
        candidate
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn commands(preference: Option<&str>) -> Vec<Vec<String>> {
        let exe = OsString::from("/usr/bin/fastf");
        let args = vec![OsString::from("search"), OsString::from("rust project")];
        candidate_commands(preference, &exe, &args)
            .iter()
            .map(|c| argv(c))
            .collect()
    }

    /// Each emulator's flag is the one *it* means, and the command reaches it as
    /// argv — an argument containing a space stays one argument. `--relaunched`
    /// leads the rerun's own arguments: a global flag before the subcommand,
    /// where no `trailing_var_arg` positional can swallow it.
    #[test]
    fn every_emulator_gets_the_argv_convention_it_actually_wants() {
        let all = commands(None);
        let find = |program: &str| {
            all.iter()
                .find(|c| c[0] == program)
                .unwrap_or_else(|| panic!("{program} missing from the candidates"))
                .clone()
        };

        assert_eq!(
            find("konsole"),
            [
                "konsole",
                "-e",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
        // Not `-e`: gnome-terminal's is deprecated and takes one string.
        assert_eq!(
            find("gnome-terminal"),
            [
                "gnome-terminal",
                "--",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
        // Not `-e`: xfce4-terminal's shell-parses a single string.
        assert_eq!(
            find("xfce4-terminal"),
            [
                "xfce4-terminal",
                "-x",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
        assert_eq!(
            find("kitty"),
            [
                "kitty",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
        assert_eq!(
            find("foot"),
            [
                "foot",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
        assert_eq!(
            find("wezterm"),
            [
                "wezterm",
                "start",
                "--",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
        // xterm's -e must be its last option, so nothing may follow it.
        assert_eq!(
            find("xterm"),
            [
                "xterm",
                "-e",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
        assert_eq!(
            find(XDG_RESOLVER),
            [
                XDG_RESOLVER,
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );
    }

    /// A configured terminal is tried first, keeps its own argv convention when
    /// fastf knows it, and never displaces the fallbacks — an emulator that is
    /// configured but not installed must not leave the user with nothing.
    #[test]
    fn a_configured_terminal_leads_and_the_probe_list_still_follows() {
        let all = commands(Some("/usr/bin/konsole"));
        assert_eq!(
            all[0],
            [
                "/usr/bin/konsole",
                "-e",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ],
            "a full path must still be recognised as konsole by its basename"
        );
        assert_eq!(all[1][0], XDG_RESOLVER);
        assert!(all.iter().any(|c| c[0] == "xterm"));

        // An emulator fastf has never heard of gets the xterm convention.
        let unknown = commands(Some("my-terminal"));
        assert_eq!(
            unknown[0],
            [
                "my-terminal",
                "-e",
                "/usr/bin/fastf",
                "--relaunched",
                "search",
                "rust project"
            ]
        );

        // An empty or whitespace preference is not a program name.
        assert_eq!(commands(Some("   "))[0][0], XDG_RESOLVER);
    }

    /// The discriminator the whole rule rests on: a pipe or a file has a reader
    /// and must never trigger a relaunch; a character device or a closed
    /// descriptor provably does not.
    #[test]
    fn only_a_stream_nobody_can_read_counts_as_headless() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        assert!(
            !stream_has_no_reader(fds[0]) && !stream_has_no_reader(fds[1]),
            "a pipe is somebody reading — cron and every `| grep` depend on it"
        );
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }

        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(
            !stream_has_no_reader(file.as_file().as_raw_fd()),
            "a regular file is a redirect somebody meant to keep"
        );

        let null = std::fs::File::open("/dev/null").unwrap();
        assert!(
            stream_has_no_reader(null.as_raw_fd()),
            "/dev/null is a character device: written to nobody"
        );

        // 2^20 descriptors open is not a state this process can reach, so this
        // is EBADF — nothing is there at all.
        assert!(
            stream_has_no_reader(1 << 20),
            "a closed descriptor is provably unread"
        );
    }
}
