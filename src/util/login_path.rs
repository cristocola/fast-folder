//! Where a program the user named is found: on fastf's own `PATH`, or on the
//! one the user's shell builds.
//!
//! A desktop launcher starts fastf with the session's `PATH`, which no shell
//! has touched: `~/.bashrc` adds `~/.local/bin` for an interactive shell only,
//! and a terminal emulator started from a menu runs fastf, not a shell. An
//! agent, an editor or a terminal installed there is then one the user can run
//! from any terminal and fastf cannot. So fastf asks the user's shell, once per
//! process, which `PATH` it builds, and starts the program with that one.
//! fastf's own environment is never changed (`tests/layering.rs`), only the
//! child's.
//!
//! **When the shell is asked.** Never without a display, over ssh, from inside
//! a shell fastf is asking, or with `$SHELL` unset: an ssh login already ran
//! the profile, and a console login's profile may `exec startx`. Otherwise:
//!
//! - **fastf leads its own session** — a terminal emulator, a launcher, the
//!   relaunch window or the `x` window started it, never a shell, which leads
//!   the session itself. Every line, editor and post-create command gets the
//!   shell's `PATH`, whatever it starts with: a program fastf *does* find may
//!   still run its own tools from `~/.local/bin`.
//! - **Otherwise**, the line's first plain word is a program fastf's `PATH`
//!   does not hold. A builtin, a path, or a word that cannot be judged is never
//!   a reason to ask.
//!
//! The child's environment changes only when the shell's `PATH` holds a folder
//! fastf's does not (`merged`), so a `PATH` a shell already built — a
//! virtualenv, mise, direnv — keeps its order.
//!
//! **What must stay true of the asking** (`ask`):
//!
//! - **`-i`, never `-l`.** An rc file returns early for a shell that is not
//!   interactive, and a terminal tab is interactive but not a login shell; `-l`
//!   runs `.bash_profile`, where `exec startx` and keychain prompts live, and
//!   `.bash_logout`.
//! - **The shell gets a session of its own** (`setsid`). An interactive zsh
//!   takes the foreground of the terminal it finds and leaves it to its dead
//!   process group, and the app then stops at its next read.
//! - **The answer is read up to the closing marker, never to the end**: a job
//!   an rc starts in the background holds the pipe open for as long as it
//!   lives.
//! - **This thread keeps the child** until the answer is in or the wait is
//!   over, so the group is killed while its leader is still unreaped.

use std::ffi::OsString;

/// Set on the shell fastf asks, so a fastf its rc starts does not ask again,
/// and so an rc can skip slow work. Public, documented.
pub const ASKING_VAR: &str = "FASTF_ASKING_SHELL";

/// The `PATH` to start `program` with, when it is not fastf's own.
///
/// `None` means "leave the child's environment alone": fastf's `PATH` is the
/// one to use, or the shell had nothing to add.
#[cfg(unix)]
pub fn path_for_program(program: &str) -> Option<OsString> {
    path_for(Some(program))
}

/// The `PATH` to run the shell line `line` with, when it is not fastf's own.
#[cfg(unix)]
pub fn path_for_line(line: &str) -> Option<OsString> {
    path_for(judged_program(line))
}

#[cfg(not(unix))]
pub fn path_for_program(_program: &str) -> Option<OsString> {
    None
}

#[cfg(not(unix))]
pub fn path_for_line(_line: &str) -> Option<OsString> {
    None
}

#[cfg(unix)]
pub use unix::{Found, Started, find_in_shell_path, first_that_starts};

#[cfg(unix)]
use unix::{judged_program, path_for};

#[cfg(unix)]
mod unix {
    use std::ffi::{OsStr, OsString};
    use std::io::BufRead;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::{Duration, Instant};

    use super::ASKING_VAR;
    use crate::util::{diag, faults, interrupt, log, paths, tty};

    /// How long a shell has to say its `PATH`. An rc that loads nvm or
    /// oh-my-zsh answers in about a second; one that has not answered in five
    /// is waiting for something that is not coming.
    const ANSWER_WAIT: Duration = Duration::from_secs(5);

    /// How often the wait looks at the interrupt flag.
    const SLICE: Duration = Duration::from_millis(100);

    /// Words a shell runs itself, which are never a program to look for.
    const BUILTINS: &[&str] = &[
        "alias", "builtin", "case", "cd", "command", "declare", "eval", "exec", "exit", "export",
        "for", "function", "if", "let", "local", "read", "readonly", "return", "select", "set",
        "shift", "source", "time", "trap", "type", "typeset", "ulimit", "umask", "unset", "until",
        "wait", "while",
    ];

    /// What to start, and the `PATH` to start it with when fastf's own is not
    /// the one that found it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Found {
        pub program: PathBuf,
        pub path: Option<OsString>,
    }

    /// What a search through candidate programs did.
    #[derive(Debug, Default)]
    pub struct Started {
        pub started: bool,
        /// Every program that was found and handed to `start`, in order.
        pub tried: Vec<String>,
    }

    pub(super) fn path_for(word: Option<&str>) -> Option<OsString> {
        let shell = may_ask()?;
        let own = std::env::var_os("PATH").unwrap_or_default();
        if leads_its_session() {
            return merged(&own, shell_path(&shell)?);
        }
        let word = word?;
        if word.contains('/') || paths::find_in(word, &own).is_some() {
            return None;
        }
        let path = merged(&own, shell_path(&shell)?)?;
        paths::find_in(word, &path).is_some().then_some(path)
    }

    /// `program` on the shell's `PATH`, for a caller that already looked on
    /// fastf's own: a terminal emulator, which is started by its full path.
    pub fn find_in_shell_path(program: &str) -> Option<Found> {
        if program.contains('/') {
            return None;
        }
        let shell = may_ask()?;
        let own = std::env::var_os("PATH").unwrap_or_default();
        let path = merged(&own, shell_path(&shell)?)?;
        let program = paths::find_in(program, &path)?;
        Some(Found {
            program,
            path: Some(path),
        })
    }

    /// Hand each candidate command that is installed to `start`, until one
    /// starts.
    ///
    /// The first candidate is the user's own choice when `preferred`: it is
    /// looked for on fastf's `PATH` and then on the shell's before anything
    /// else is tried. The rest are looked for on fastf's `PATH`, and on the
    /// shell's only once none of those started — a list of every emulator
    /// there is always has one missing, and a miss must not ask the shell when
    /// another emulator is right there.
    pub fn first_that_starts(
        candidates: &[Vec<OsString>],
        preferred: bool,
        mut start: impl FnMut(&Found, &[OsString]) -> bool,
    ) -> Started {
        let mut outcome = Started::default();
        let mut missing = Vec::new();
        for (index, candidate) in candidates.iter().enumerate() {
            let (program, rest) = candidate.split_first().expect("candidate is never empty");
            let name = program.to_string_lossy();
            let found = if paths::find_on_path(&name).is_some() {
                Some(Found {
                    program: PathBuf::from(program),
                    path: None,
                })
            } else if preferred && index == 0 {
                find_in_shell_path(&name)
            } else {
                missing.push(index);
                None
            };
            let Some(found) = found else { continue };
            outcome.tried.push(name.into_owned());
            if start(&found, rest) {
                outcome.started = true;
                return outcome;
            }
        }
        for index in missing {
            let (program, rest) = candidates[index]
                .split_first()
                .expect("candidate is never empty");
            let name = program.to_string_lossy();
            let Some(found) = find_in_shell_path(&name) else {
                continue;
            };
            outcome.tried.push(name.into_owned());
            if start(&found, rest) {
                outcome.started = true;
                return outcome;
            }
        }
        outcome
    }

    /// The shell to ask, when this process may ask one at all.
    fn may_ask() -> Option<OsString> {
        gates(
            tty::has_display(),
            std::env::var_os("SSH_CONNECTION").is_some(),
            std::env::var_os(ASKING_VAR).is_some(),
            std::env::var_os("SHELL"),
        )
    }

    pub(super) fn gates(
        display: bool,
        ssh: bool,
        asking: bool,
        shell: Option<OsString>,
    ) -> Option<OsString> {
        if !display || ssh || asking {
            return None;
        }
        shell.filter(|shell| !shell.is_empty())
    }

    /// Whether a terminal emulator or a launcher started this process rather
    /// than a shell: whatever starts a program in a terminal of its own makes
    /// it the leader of a new session, and a shell leads its session itself.
    fn leads_its_session() -> bool {
        // SAFETY: `getsid(0)` asks about this process and writes nothing.
        let session = unsafe { libc::getsid(0) };
        session != -1 && session as u32 == std::process::id()
    }

    /// The shell's `PATH`, asked once per process.
    fn shell_path(shell: &OsStr) -> Option<&'static OsString> {
        static ANSWER: OnceLock<Option<OsString>> = OnceLock::new();
        ANSWER
            .get_or_init(|| {
                let started = Instant::now();
                let wait = answer_wait();
                let shown = shell.to_string_lossy();
                match ask(shell, wait) {
                    Asked::Answered(path) => {
                        log::debug(format!(
                            "{shown} said its PATH in {} ms: {}",
                            started.elapsed().as_millis(),
                            path.to_string_lossy()
                        ));
                        Some(path)
                    }
                    Asked::Silent => {
                        log::debug(format!(
                            "{shown} -i -c printed no PATH; programs start with fastf's own"
                        ));
                        None
                    }
                    Asked::GaveUp => {
                        diag::warn(format!(
                            "{shown} did not say its PATH within {} s, so programs fastf starts \
                             get fastf's own; your shell's rc can skip slow work while {ASKING_VAR} is set",
                            wait.as_secs()
                        ));
                        None
                    }
                }
            })
            .as_ref()
    }

    /// [`ANSWER_WAIT`] — or a second, under the `shell:short-wait` decision, so
    /// a test reaches what happens once the wait runs out.
    fn answer_wait() -> Duration {
        if faults::is_armed("shell:short-wait") {
            Duration::from_secs(1)
        } else {
            ANSWER_WAIT
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum Asked {
        Answered(OsString),
        /// It finished, or stopped talking, without printing a `PATH` between
        /// the markers: a shell that does not take `-i -c`, or an rc that
        /// replaced the shell.
        Silent,
        /// The wait ran out, or the person interrupted it.
        GaveUp,
    }

    /// Ask `shell` which `PATH` an interactive shell of the user's builds.
    pub(super) fn ask(shell: &OsStr, wait: Duration) -> Asked {
        let marker = format!(
            "FASTF_PATH_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.subsec_nanos())
        );
        let script = format!("printf '%s\\n' {marker}; printenv PATH; printf '%s\\n' {marker}");
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_dir())
            .unwrap_or_else(|| PathBuf::from("/"));

        let mut command = Command::new(shell);
        command
            .args(["-i", "-c", &script])
            .current_dir(&home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env(ASKING_VAR, "1")
            // oh-my-zsh checks for its own updates on start; not inside the wait.
            .env("DISABLE_AUTO_UPDATE", "true");
        // SAFETY: `setsid` is async-signal-safe and touches nothing of the
        // parent's; it is the one call made between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let Ok(mut child) = command.spawn() else {
            return Asked::Silent;
        };
        let stdout = child.stdout.take().expect("stdout is piped");

        let (answer, answered) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            let _ = answer.send(between_markers(&mut reader, &marker));
            // What the rc left running keeps writing to this pipe; it is read
            // and dropped rather than closed under it while fastf runs.
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });

        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || interrupt::is_set() {
                break;
            }
            match answered.recv_timeout(left.min(SLICE)) {
                Ok(path) => {
                    reap(child);
                    return path.map_or(Asked::Silent, Asked::Answered);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    reap(child);
                    return Asked::Silent;
                }
            }
        }
        // The shell leads a session and a process group of its own, so the
        // group is everything its rc started that did not leave it.
        // SAFETY: `kill` sends a signal; the group is the unreaped child's.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        reap(child);
        Asked::GaveUp
    }

    /// Wait for the shell on a thread of its own, so it leaves no zombie and
    /// no caller waits for a shell that is taking its time to exit.
    fn reap(mut child: Child) {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }

    /// The one line between the two `marker` lines, read no further than the
    /// second. `None` for anything else: no markers, no line, or more than one.
    pub(super) fn between_markers(reader: &mut impl BufRead, marker: &str) -> Option<OsString> {
        let mut inside = None::<Vec<Vec<u8>>>;
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line).ok()? == 0 {
                return None;
            }
            while line
                .last()
                .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
            {
                line.pop();
            }
            if line == marker.as_bytes() {
                match inside {
                    Some(mut lines) => {
                        return match (lines.pop(), lines.is_empty()) {
                            (Some(path), true) if !path.is_empty() => {
                                Some(OsString::from_vec(path))
                            }
                            _ => None,
                        };
                    }
                    None => inside = Some(Vec::new()),
                }
            } else if let Some(lines) = inside.as_mut() {
                lines.push(line.clone());
            }
        }
    }

    /// The `PATH` a child gets: `None` when the shell's adds no folder fastf's
    /// lacks, else the shell's folders in the shell's order, then fastf's own
    /// the shell does not have, each once. A relative folder the shell adds is
    /// left out — it would be looked up in the project's folder.
    pub(super) fn merged(own: &OsStr, shell: &OsStr) -> Option<OsString> {
        let own: Vec<PathBuf> = std::env::split_paths(own)
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect();
        let shell: Vec<PathBuf> = std::env::split_paths(shell)
            .filter(|dir| dir.is_absolute())
            .collect();
        if shell.iter().all(|dir| own.contains(dir)) {
            return None;
        }
        let mut path: Vec<&Path> = Vec::new();
        for dir in shell.iter().chain(&own) {
            if !path.contains(&dir.as_path()) {
                path.push(dir);
            }
        }
        std::env::join_paths(path).ok()
    }

    /// The program a shell line starts, when it can be told: the first word
    /// after any `NAME=value` words, out of one pair of quotes, made of the
    /// characters program names are made of, and not a word the shell runs
    /// itself.
    pub(super) fn judged_program(line: &str) -> Option<&str> {
        let word = line.split_whitespace().find(|word| !is_assignment(word))?;
        let word = ['"', '\'']
            .iter()
            .find_map(|quote| {
                word.strip_prefix(*quote)
                    .and_then(|inner| inner.strip_suffix(*quote))
            })
            .unwrap_or(word);
        let plain = word.starts_with(|c: char| c.is_ascii_alphanumeric())
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
        (plain && !BUILTINS.contains(&word)).then_some(word)
    }

    fn is_assignment(word: &str) -> bool {
        word.split_once('=').is_some_and(|(name, _)| {
            name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::unix::{Asked, ask, between_markers, gates, judged_program, merged};
    use std::ffi::{OsStr, OsString};
    use std::time::{Duration, Instant};

    fn shell_script(dir: &std::path::Path, name: &str, body: &str) -> OsString {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.into_os_string()
    }

    /// Runs the line fastf hands it — its last argument — the way a shell
    /// would, after whatever `rc` does.
    fn answering_shell(dir: &std::path::Path, rc: &str) -> OsString {
        shell_script(
            dir,
            "answering-shell",
            &format!("{rc}\nfor line; do :; done\neval \"$line\"\necho goodbye"),
        )
    }

    /// What an rc prints around the answer is not the answer.
    #[test]
    fn the_path_is_the_line_between_the_markers() {
        let out = "Welcome to fish\nM\n/home/user/.local/bin:/usr/bin\nM\nbye\n";
        assert_eq!(
            between_markers(&mut out.as_bytes(), "M"),
            Some(OsString::from("/home/user/.local/bin:/usr/bin"))
        );
    }

    #[test]
    fn anything_but_one_line_between_the_markers_is_no_answer() {
        for out in [
            "",
            "M\n",
            "nothing at all\n",
            "M\nM\n",
            "M\na\nb\nM\n",
            "M\n/usr/bin\n",
        ] {
            assert_eq!(between_markers(&mut out.as_bytes(), "M"), None, "{out:?}");
        }
    }

    /// A shell started from a shell builds the same `PATH` again: nothing to
    /// add, so the child's environment is not touched and a virtualenv's
    /// order survives.
    #[test]
    fn a_shell_that_adds_nothing_changes_nothing() {
        let own = OsStr::new("/venv/bin:/home/user/.local/bin:/usr/bin");
        let shell = OsStr::new("/home/user/.local/bin:/venv/bin:/home/user/.local/bin:/usr/bin");
        assert_eq!(merged(own, shell), None);
    }

    #[test]
    fn what_the_shell_adds_comes_first_in_its_order() {
        let own = OsStr::new("/usr/local/bin:/usr/bin:/opt/cuda/bin");
        let shell =
            OsStr::new("/home/user/.local/bin:/home/user/.cargo/bin:/usr/local/bin:/usr/bin");
        assert_eq!(
            merged(own, shell),
            Some(OsString::from(
                "/home/user/.local/bin:/home/user/.cargo/bin:/usr/local/bin:/usr/bin:/opt/cuda/bin"
            ))
        );
    }

    /// `.` on the shell's `PATH` would be the project's folder for the child.
    #[test]
    fn a_relative_folder_the_shell_adds_is_left_out() {
        let merged = merged(
            OsStr::new("/usr/bin"),
            OsStr::new(".:bin:/home/user/bin:/usr/bin"),
        );
        assert_eq!(merged, Some(OsString::from("/home/user/bin:/usr/bin")));
        assert_eq!(
            super::unix::merged(OsStr::new("/usr/bin"), OsStr::new(".:/usr/bin")),
            None
        );
    }

    #[test]
    fn the_program_of_a_line_is_its_first_plain_word() {
        assert_eq!(
            judged_program("claude --dangerously-skip-permissions"),
            Some("claude")
        );
        assert_eq!(judged_program("  FOO=1 BAR=x  aider --yes"), Some("aider"));
        assert_eq!(judged_program("'claude' --resume"), Some("claude"));
        assert_eq!(
            judged_program("node_modules.bin-x+ y"),
            Some("node_modules.bin-x+")
        );
    }

    /// Only a word that names a program is a reason to ask the shell.
    #[test]
    fn a_word_that_is_not_a_program_name_is_not_judged() {
        for line in [
            "",
            "cd sub && claude",
            "trap '' INT; sleep 5",
            "exec claude",
            "/opt/agent/bin/agent",
            "./run.sh",
            "$HOME/bin/agent",
            "(cd sub; claude)",
            "\"claude --x\"",
            "FOO=1",
        ] {
            assert_eq!(judged_program(line), None, "{line:?}");
        }
    }

    #[test]
    fn the_shell_is_asked_only_with_a_display_outside_ssh_and_not_from_itself() {
        let bash = || Some(OsString::from("/bin/bash"));
        assert_eq!(gates(true, false, false, bash()), bash());
        assert_eq!(gates(false, false, false, bash()), None);
        assert_eq!(gates(true, true, false, bash()), None);
        assert_eq!(gates(true, false, true, bash()), None);
        assert_eq!(gates(true, false, false, None), None);
        assert_eq!(gates(true, false, false, Some(OsString::new())), None);
    }

    #[test]
    fn a_shell_says_the_path_its_rc_builds() {
        let dir = tempfile::tempdir().unwrap();
        let shell = answering_shell(
            dir.path(),
            "echo 'a banner on stdout'\nPATH=/hidden/bin:$PATH; export PATH",
        );
        let Asked::Answered(path) = ask(&shell, Duration::from_secs(5)) else {
            panic!("the shell did not answer");
        };
        assert!(
            path.to_string_lossy().starts_with("/hidden/bin:"),
            "{path:?}"
        );
    }

    /// The shell is asked as a terminal tab is, interactive and not a login
    /// shell, and without the terminal: it could not give that back.
    #[test]
    fn the_shell_is_asked_interactive_and_without_a_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let shell = answering_shell(
            dir.path(),
            &format!(
                "printf '%s\\n' \"$@\" > {log}\n( : < /dev/tty ) 2>/dev/null && echo tty >> {log}",
                log = dir.path().join("asked").display()
            ),
        );
        assert!(matches!(
            ask(&shell, Duration::from_secs(5)),
            Asked::Answered(_)
        ));
        let asked = std::fs::read_to_string(dir.path().join("asked")).unwrap();
        let args: Vec<&str> = asked.lines().collect();
        assert_eq!(args[..2], ["-i", "-c"], "{asked}");
        assert!(!args.contains(&"-l") && !args.contains(&"tty"), "{asked}");
    }

    /// A job an rc leaves running holds the pipe open; the answer is in long
    /// before it ends.
    #[test]
    fn a_job_the_rc_leaves_running_does_not_hold_the_answer() {
        let dir = tempfile::tempdir().unwrap();
        let shell = answering_shell(dir.path(), "sleep 5 &");
        let started = Instant::now();
        assert!(matches!(
            ask(&shell, Duration::from_secs(4)),
            Asked::Answered(_)
        ));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }

    /// A shell that never answers is given up on when the wait is over, and
    /// everything it started goes with it.
    #[test]
    fn a_shell_that_does_not_answer_is_given_up_on_whole() {
        let dir = tempfile::tempdir().unwrap();
        let late = dir.path().join("late");
        let shell = shell_script(
            dir.path(),
            "stuck-shell",
            &format!("(sleep 1; touch {}) &\nsleep 30", late.display()),
        );
        let started = Instant::now();
        assert_eq!(ask(&shell, Duration::from_millis(300)), Asked::GaveUp);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(1500));
        assert!(
            !late.exists(),
            "the shell's background job outlived the wait"
        );
    }

    #[test]
    fn a_shell_that_prints_no_markers_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let shell = shell_script(dir.path(), "odd-shell", "echo 'unknown option -i'; exit 2");
        assert_eq!(ask(&shell, Duration::from_secs(5)), Asked::Silent);
        assert_eq!(
            ask(OsStr::new("/nonexistent/shell"), Duration::from_secs(5)),
            Asked::Silent
        );
    }
}
