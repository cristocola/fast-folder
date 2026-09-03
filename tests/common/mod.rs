//! A sandboxed fastf on disk, driven as a **real process**.
//!
//! Shared by every suite that needs to exercise the command surface rather than
//! the library: `concurrency.rs` (which races processes, because a thread test
//! passes against an in-process `Mutex` while production stays broken) and the
//! `cli_*` suites (which assert what commands actually do to disk, because the
//! argument-and-prompt layer is where a green suite kept missing bugs).
//!
//! **Why a process and not a function call:** the defects these cover live in
//! the plumbing between clap and the core — flags dropped into
//! `trailing_var_arg`, one caller computing an ID differently from another, a
//! config field read raw instead of resolved. Only a process sees that.
//!
//! Each sandbox owns its `FASTF_INSTALL_DIR` and redirects `HOME` into itself,
//! so nothing here can reach the developer's real config, templates, counter, or
//! projects. That redirect is not optional: an unconfigured `base_dir` falls
//! back to the home directory, and a harness that skips it scans the real one
//! and self-heals the counter from real projects.

#![allow(dead_code)] // each test binary uses a different subset

pub mod env;
pub mod fixtures;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};

pub const FASTF: &str = env!("CARGO_BIN_EXE_fastf");

pub struct Sandbox {
    pub tmp: tempfile::TempDir,
    pub install: PathBuf,
    pub base: PathBuf,
}

impl Sandbox {
    /// A sandbox with one base, configured as `base_dir`.
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let install = tmp.path().join("install");
        let base = tmp.path().join("base");
        fs::create_dir_all(install.join("templates")).unwrap();
        fs::create_dir_all(&base).unwrap();
        let sb = Sandbox { tmp, install, base };
        let out = sb.run(&["config", "set", "base-dir", &sb.base.display().to_string()]);
        assert!(out.status.success(), "config set base-dir failed: {out:?}");
        sb
    }

    /// A sandbox with **no base configured at all** — a brand-new install, so
    /// the app asks where projects should live before it draws anything else.
    pub fn unconfigured() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let install = tmp.path().join("install");
        let base = tmp.path().join("base");
        fs::create_dir_all(install.join("templates")).unwrap();
        Sandbox { tmp, install, base }
    }

    /// Add extra library bases (`config set bases`), creating each directory.
    /// Returns their paths in the order given.
    pub fn with_bases(&self, names: &[&str]) -> Vec<PathBuf> {
        let paths: Vec<PathBuf> = names.iter().map(|n| self.tmp.path().join(n)).collect();
        for p in &paths {
            fs::create_dir_all(p).unwrap();
        }
        let joined = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(",");
        let out = self.run(&["config", "set", "bases", &joined]);
        assert!(out.status.success(), "config set bases failed: {out:?}");
        paths
    }

    /// A minimal template whose naming pattern carries the ID, so tests can read
    /// the minted number straight off the folder name.
    pub fn write_template(&self, slug: &str) {
        let dir = self.install.join("templates").join(slug);
        fs::create_dir_all(dir.join("files")).unwrap();
        fs::write(
            dir.join("template.yaml"),
            format!(
                "name: Race\nslug: {slug}\nnaming_pattern: \"{{id}}_{{name}}\"\n\
                 id:\n  prefix: R\n  digits: 4\n\
                 variables:\n  - slug: name\n    label: Name\n    type: text\n\
                 \x20   required: true\n    transform: none\n"
            ),
        )
        .unwrap();
        fs::write(dir.join("files/README.md"), "# {name}\n").unwrap();
    }

    /// Environment shared by every spawned process: same data dir, and HOME
    /// redirected so an unconfigured base can never reach the real home.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(FASTF);
        cmd.env("FASTF_INSTALL_DIR", &self.install).env(
            if cfg!(windows) { "USERPROFILE" } else { "HOME" },
            self.tmp.path(),
        );
        cmd
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("running fastf")
    }

    /// `run`, asserting success and returning stdout — for steps that are setup
    /// rather than the thing under test.
    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "expected `fastf {}` to succeed, got {out:?}",
            args.join(" ")
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `run`, with `util::trace` writing to a file, returning what it counted.
    ///
    /// Debug builds only — the tracer is compiled out of release, like the
    /// failpoints. The command must succeed.
    pub fn traced(&self, args: &[&str]) -> Trace {
        let path = self.tmp.path().join(format!("trace-{}", args.join("-")));
        let out = self
            .command()
            .args(args)
            .env("FASTF_TRACE_FILE", &path)
            .output()
            .expect("running fastf");
        assert!(
            out.status.success(),
            "expected `fastf {}` to succeed, got {out:?}",
            args.join(" ")
        );
        Trace {
            lines: fs::read_to_string(&path).unwrap_or_default(),
        }
    }

    /// `run`, asserting failure and returning stderr — for the refusals.
    pub fn fails(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            !out.status.success(),
            "expected `fastf {}` to fail, got {out:?}",
            args.join(" ")
        );
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// `run` with stdin closed as well as stdout and stderr piped — a process
    /// with no terminal anywhere, which is what a script or a CI runner gives
    /// fastf. `run` alone inherits the test runner's stdin, so "there is no
    /// terminal to prompt on" would depend on how `cargo test` was launched.
    pub fn run_headless(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("running fastf")
    }

    /// `run_headless`, asserting failure and returning stderr.
    pub fn fails_headless(&self, args: &[&str]) -> String {
        let out = self.run_headless(args);
        assert!(
            !out.status.success(),
            "expected `fastf {}` to fail without a terminal, got {out:?}",
            args.join(" ")
        );
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// Run fastf the way a desktop launcher does: **no terminal anywhere**.
    ///
    /// stdin is `/dev/null` (a character device) and stdout and stderr share one
    /// socket, which is byte-for-byte the shape systemd gives a launcher's
    /// children — journald is a socket, not a pipe. A pipe is what `run` uses
    /// and is deliberately different: a pipe means somebody is reading.
    ///
    /// The socket is also how the test can still see the output, which
    /// `/dev/null` would not allow — and "the parent printed nothing" is half
    /// of what the relaunch tests are checking.
    #[cfg(unix)]
    pub fn run_like_a_launcher(&self, args: &[&str], env: &[(&str, &str)]) -> LauncherRun {
        use std::io::Read;
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        let (ours, theirs) = UnixStream::pair().expect("socketpair");
        let out: OwnedFd = theirs.try_clone().expect("dup").into();
        let err: OwnedFd = theirs.into();

        let mut cmd = self.command();
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(out))
            .stderr(std::process::Stdio::from(err));
        for (key, value) in env {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().expect("running fastf");
        // Both of the child's ends are owned by `cmd`, which drops them here;
        // the read below then ends at EOF when the child exits.
        drop(cmd);

        let mut output = Vec::new();
        let mut ours = ours;
        ours.read_to_end(&mut output).expect("reading the socket");
        let status = child.wait().expect("waiting for fastf");
        LauncherRun {
            output: String::from_utf8_lossy(&output).into_owned(),
            code: status.code().unwrap_or(-1),
        }
    }

    pub fn spawn(&self, args: &[&str]) -> Child {
        self.command()
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawning fastf")
    }

    /// Every project's id in the primary base, read straight from metadata.
    pub fn ids_on_disk(&self) -> Vec<String> {
        ids_in(&self.base)
    }

    /// The counter value a base records, or 0 when it has no counter file.
    pub fn base_counter(&self, base: &Path) -> u64 {
        fs::read_to_string(base.join(".fastf-counter.toml"))
            .ok()
            .and_then(|raw| {
                raw.lines()
                    .find_map(|l| l.split_once('=').map(|(_, v)| v.trim().to_string()))
            })
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// This machine's data-directory counter.
    pub fn local_counter(&self) -> u64 {
        self.base_counter_at(&self.install.join("counters.toml"))
    }

    fn base_counter_at(&self, path: &Path) -> u64 {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| {
                raw.lines()
                    .find_map(|l| l.split_once('=').map(|(_, v)| v.trim().to_string()))
            })
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// Plant a project with a chosen ID, without going through `fastf new` —
    /// the fixture for "this base already holds ID0082".
    pub fn plant_project(&self, base: &Path, folder: &str, id: &str) -> PathBuf {
        let dir = base.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("PROJECT_INFO.md"),
            format!(
                "---\nid: {id}\ntemplate: general\ntemplate_name: General\n\
                 created: 2026-01-01T00:00:00Z\nfolder: {folder}\n\
                 path: {}\nvariables: {{}}\ntags: []\n---\n\n## Notes\n",
                dir.display()
            ),
        )
        .unwrap();
        dir
    }
}

/// What one traced command did, by operation name.
pub struct Trace {
    lines: String,
}

impl Trace {
    pub fn count(&self, name: &str) -> usize {
        self.lines.lines().filter(|line| *line == name).count()
    }

    /// Every name that appeared, with its count — for a failure message that
    /// says what actually happened rather than only what did not.
    pub fn summary(&self) -> String {
        let mut names: Vec<&str> = self.lines.lines().collect();
        names.sort();
        names.dedup();
        names
            .iter()
            .map(|name| format!("{name}={}", self.count(name)))
            .collect::<Vec<_>>()
            .join("  ")
    }
}

/// Drive fastf through a real terminal.
///
/// `dialoguer` refuses to prompt without a TTY, so every confirmation, picker
/// and interactive preview is invisible to a pipe-based test — which is exactly
/// where the rename prompt once spent a release offering one folder name and committing
/// another. A pty is the only way to see what the user sees.
///
/// Unix only, which matches how it is used: the prompts themselves are
/// cross-platform and covered by the non-interactive paths.
#[cfg(unix)]
pub mod pty {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::time::{Duration, Instant};

    /// One scripted keystroke: how long after launch to send it, and what.
    pub type Keystroke = (Duration, Vec<u8>);

    /// The terminal every pty test runs in.
    pub const PTY_COLS: u16 = 120;
    pub const PTY_ROWS: u16 = 40;

    /// The transcript with every escape sequence removed, so an assertion can
    /// match text the way a person sees it. ratatui redraws only the cells
    /// that changed, so the result is a stream of fragments, not screens:
    /// match on words, never on a whole line.
    pub fn plain(transcript: &str) -> String {
        let mut out = String::with_capacity(transcript.len());
        let mut chars = transcript.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            match chars.next() {
                // CSI: parameters, intermediates, then one final byte.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ST.
                Some(']') => {
                    let mut previous = '\0';
                    for c in chars.by_ref() {
                        if c == '\x07' || (previous == '\x1b' && c == '\\') {
                            break;
                        }
                        previous = c;
                    }
                }
                // Two-byte escapes (charset selection and the like).
                Some('(') | Some(')') | Some('#') => {
                    chars.next();
                }
                _ => {}
            }
        }
        out
    }

    /// A keystroke script on a fixed cadence.
    ///
    /// Keys are spaced rather than burst because `dialoguer` redraws between
    /// them: sending six arrow presses in one `write` loses most of them, and
    /// the menu ends up somewhere unintended. The gaps are what make these
    /// tests deterministic.
    pub struct Script {
        steps: Vec<Keystroke>,
        at_ms: u64,
    }

    impl Default for Script {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Script {
        /// Starts after a beat, so the first menu has drawn.
        pub fn new() -> Self {
            Script {
                steps: Vec::new(),
                at_ms: 800,
            }
        }

        fn push(mut self, bytes: &[u8], gap: u64) -> Self {
            self.steps
                .push((Duration::from_millis(self.at_ms), bytes.to_vec()));
            self.at_ms += gap;
            self
        }

        /// Move the selection down `n` times.
        pub fn down(mut self, n: usize) -> Self {
            for _ in 0..n {
                self = self.push(b"\x1b[B", 200);
            }
            self
        }

        pub fn enter(self) -> Self {
            self.push(b"\r", 600)
        }

        /// Type a line and submit it. For `Input` prompts, which read until Enter.
        pub fn line(self, text: &str) -> Self {
            let mut bytes = text.as_bytes().to_vec();
            bytes.push(b'\r');
            self.push(&bytes, 600)
        }

        /// Send raw keys with no Enter. Use this for `Confirm`, which answers on
        /// the `y`/`n` keypress itself — a trailing `\r` would survive into the
        /// *next* prompt and silently accept its default.
        pub fn key(self, text: &str) -> Self {
            self.push(text.as_bytes(), 600)
        }

        /// Backspace, `n` times. For correcting text a validator has just
        /// rejected — which is the whole point of leaving it on the line.
        pub fn backspace(mut self, n: usize) -> Self {
            for _ in 0..n {
                self = self.push(b"\x7f", 200);
            }
            self
        }

        /// Tab — the next field of a form.
        pub fn tab(self) -> Self {
            self.push(b"\t", 300)
        }

        /// The arrows that change a form's choice in place.
        pub fn right(mut self, n: usize) -> Self {
            for _ in 0..n {
                self = self.push(b"\x1b[C", 250);
            }
            self
        }

        pub fn left(mut self, n: usize) -> Self {
            for _ in 0..n {
                self = self.push(b"\x1b[D", 250);
            }
            self
        }

        /// PageDown / PageUp — one viewport of the list.
        pub fn page_down(self) -> Self {
            self.push(b"\x1b[6~", 400)
        }

        pub fn page_up(self) -> Self {
            self.push(b"\x1b[5~", 400)
        }

        /// Home, for editing the front of a line a validator refused.
        pub fn home(self) -> Self {
            self.push(b"\x1b[H", 300)
        }

        /// Esc — the one cancel key.
        ///
        /// Sent alone, with a full gap after it: a lone `\x1b` immediately
        /// followed by `[` is an arrow-key sequence, so an Esc typed next to
        /// another key is a different key.
        pub fn esc(self) -> Self {
            self.push(b"\x1b", 700)
        }

        pub fn ctrl_c(self) -> Self {
            self.push(b"\x03", 400)
        }

        /// Wait before the next key — for a step that does real work.
        pub fn pause(mut self, ms: u64) -> Self {
            self.at_ms += ms;
            self
        }

        pub fn build(self) -> Vec<Keystroke> {
            self.steps
        }

        /// When the next keystroke would be sent — the moment a screenshot of
        /// "the state after everything so far" belongs to.
        pub fn elapsed(&self) -> Duration {
            Duration::from_millis(self.at_ms)
        }
    }

    /// Run `program` with `args` and `env` overrides under a pty, feeding
    /// `script` on schedule. Returns everything the program wrote (ANSI and
    /// all) plus its exit code.
    pub fn run(
        program: &str,
        args: &[&str],
        env: &[(&str, &std::path::Path)],
        script: &[Keystroke],
        deadline: Duration,
    ) -> (String, i32) {
        let (chunks, code) = run_with_stdout(program, args, env, script, deadline, None);
        (join(&chunks), code)
    }

    /// `run`, keeping every read as its own chunk with the moment it arrived,
    /// so a screen can be reconstructed as it was at any point of the script.
    pub fn run_chunked(
        program: &str,
        args: &[&str],
        env: &[(&str, &std::path::Path)],
        script: &[Keystroke],
        deadline: Duration,
    ) -> (Vec<(Duration, Vec<u8>)>, i32) {
        run_with_stdout(program, args, env, script, deadline, None)
    }

    /// The chunks that arrived before `until`, joined; what the screen was at
    /// that moment once replayed.
    pub fn until(chunks: &[(Duration, Vec<u8>)], until: Duration) -> Vec<u8> {
        chunks
            .iter()
            .filter(|(at, _)| *at < until)
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect()
    }

    fn join(chunks: &[(Duration, Vec<u8>)]) -> String {
        let bytes: Vec<u8> = chunks
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// `run`, but with the child's **stdout** pointed at `stdout_file` while
    /// stdin and stderr stay on the pty.
    ///
    /// This is the shape a user gets from `fastf new t > out.txt`: a terminal is
    /// right there, and only the output is redirected. fastf decided prompt
    /// availability by probing stdout, so it refused to prompt in exactly the
    /// case where prompting is fine. The file is what the pty transcript cannot
    /// show, so tests assert on both.
    pub fn run_stdout_to(
        program: &str,
        args: &[&str],
        env: &[(&str, &std::path::Path)],
        script: &[Keystroke],
        deadline: Duration,
        stdout_file: &std::path::Path,
    ) -> (String, i32) {
        let (chunks, code) =
            run_with_stdout(program, args, env, script, deadline, Some(stdout_file));
        (join(&chunks), code)
    }

    fn run_with_stdout(
        program: &str,
        args: &[&str],
        env: &[(&str, &std::path::Path)],
        script: &[Keystroke],
        deadline: Duration,
        stdout_file: Option<&std::path::Path>,
    ) -> (Vec<(Duration, Vec<u8>)>, i32) {
        // Everything the child needs is built *before* the fork: after it, only
        // async-signal-safe calls are legal, and `execve` is one — `setenv` is not.
        let prog = CString::new(program).unwrap();
        let argv: Vec<CString> = std::iter::once(CString::new(program).unwrap())
            .chain(args.iter().map(|a| CString::new(*a).unwrap()))
            .collect();
        let mut envp: Vec<CString> = std::env::vars_os()
            .filter(|(k, _)| !env.iter().any(|(name, _)| k.as_bytes() == name.as_bytes()))
            .map(|(k, v)| {
                let mut buf = k.as_bytes().to_vec();
                buf.push(b'=');
                buf.extend_from_slice(v.as_bytes());
                CString::new(buf).unwrap()
            })
            .collect();
        for (name, value) in env {
            let mut buf = name.as_bytes().to_vec();
            buf.push(b'=');
            buf.extend_from_slice(value.as_os_str().as_bytes());
            envp.push(CString::new(buf).unwrap());
        }
        let argv_ptr: Vec<*const libc::c_char> = argv
            .iter()
            .map(|a| a.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let envp_ptr: Vec<*const libc::c_char> = envp
            .iter()
            .map(|e| e.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        // Opened before the fork: after it only async-signal-safe calls are
        // legal, and `dup2` is one — the open is not worth arguing about.
        let redirect: libc::c_int = match stdout_file {
            Some(path) => {
                let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
                // SAFETY: a path we own, standard create/truncate flags.
                let fd = unsafe {
                    libc::open(
                        cpath.as_ptr(),
                        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                        0o644,
                    )
                };
                assert!(fd >= 0, "opening {} failed", path.display());
                fd
            }
            None => -1,
        };

        let mut master: libc::c_int = -1;
        // A real window size: the guided app lays itself out from it, and a
        // pty with no size reports 0×0, which draws nothing. 120×40 is the
        // large layout — the detail pane, the template strip, the tall header.
        let winsize = libc::winsize {
            ws_row: PTY_ROWS,
            ws_col: PTY_COLS,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: a null termios requests the defaults; `master` is written by
        // the call and only read in the parent branch.
        let pid = unsafe {
            libc::forkpty(
                &mut master,
                std::ptr::null_mut(),
                std::ptr::null(),
                &winsize,
            )
        };
        assert!(pid >= 0, "forkpty failed");
        if pid == 0 {
            // SAFETY: async-signal-safe only — the arrays above are already built
            // and the redirect descriptor is already open.
            unsafe {
                if redirect >= 0 {
                    libc::dup2(redirect, 1);
                    libc::close(redirect);
                }
                libc::execve(prog.as_ptr(), argv_ptr.as_ptr(), envp_ptr.as_ptr());
                libc::_exit(127);
            }
        }

        // Non-blocking, or the read below parks forever whenever the child is
        // thinking and the loop never reaches its own deadline check.
        // SAFETY: adjusting flags on a descriptor we own.
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL, 0);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let start = Instant::now();
        let mut out: Vec<(Duration, Vec<u8>)> = Vec::new();
        let mut sent = 0usize;
        let mut status: libc::c_int = 0;
        let code = loop {
            if sent < script.len() && start.elapsed() >= script[sent].0 {
                let bytes = &script[sent].1;
                // SAFETY: writing to the pty master we own.
                unsafe { libc::write(master, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
                sent += 1;
            }
            let mut buf = [0u8; 4096];
            // SAFETY: reading into a buffer we own, from a descriptor we own.
            let n = unsafe { libc::read(master, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n > 0 {
                out.push((start.elapsed(), buf[..n as usize].to_vec()));
            }
            // SAFETY: reaping our own child, non-blocking.
            let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if reaped == pid {
                break libc::WEXITSTATUS(status);
            }
            if start.elapsed() > deadline {
                // SAFETY: killing our own child, then reaping it.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    libc::waitpid(pid, &mut status, 0);
                }
                break -1;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // SAFETY: closing the descriptors we opened.
        unsafe {
            libc::close(master);
            if redirect >= 0 {
                libc::close(redirect);
            }
        };
        (out, code)
    }
}

pub fn project_dirs(base: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("PROJECT_INFO.md").is_file())
        .collect();
    out.sort();
    out
}

pub fn ids_in(base: &Path) -> Vec<String> {
    project_dirs(base)
        .iter()
        .filter_map(|dir| {
            let text = fs::read_to_string(dir.join("PROJECT_INFO.md")).ok()?;
            text.lines()
                .find_map(|l| l.strip_prefix("id:"))
                .map(|v| v.trim().to_string())
        })
        .collect()
}

/// The path a command will print for `dir`, canonicalized the way discovery
/// does and then shown the way `util::paths::display_path` does.
///
/// `canonicalize` on Windows returns the verbatim `\\?\C:\...` form, which is
/// what makes long paths work and is deliberately *not* what fastf prints — so
/// a test that compares raw `canonicalize` output passes on unix and fails on
/// Windows for a reason that has nothing to do with the behaviour under test.
pub fn shown_path(dir: &Path) -> String {
    let canonical = dir.canonicalize().expect("the folder should exist");
    let raw = canonical.display().to_string();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match raw.strip_prefix(r"\\?\") {
        // Only a plain drive path unwraps; a device path means something
        // specific — the same rule `display_path` follows.
        Some(rest)
            if rest.len() >= 2
                && rest.as_bytes()[0].is_ascii_alphabetic()
                && rest.as_bytes()[1] == b':' =>
        {
            rest.to_string()
        }
        _ => raw,
    }
}

/// A fake program that records the argv it was called with, one argument per
/// line, and exits 0.
///
/// Used wherever a test needs to prove *what fastf would have run* without
/// running it: a terminal emulator, `notify-send`. **Every relaunch test pins
/// one of these** — see `tests/CLAUDE.md` — so no suite can open a real window
/// on somebody's desktop or on a CI runner that happens to have a display.
#[cfg(unix)]
pub fn recorder(dir: &Path, name: &str) -> Recorder {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dir).unwrap();
    let log = dir.join(format!("{name}.log"));
    let cwd_log = dir.join(format!("{name}.cwd"));
    let program = dir.join(name);
    // The working directory goes to its own file, written before the argv log:
    // `wait_for_call` polls the argv log, so by the time it answers, the cwd is
    // already on disk.
    fs::write(
        &program,
        format!(
            "#!/bin/sh\npwd >> {}\nprintf '%s\\n' \"$@\" >> {}\nexit 0\n",
            cwd_log.display(),
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    Recorder {
        program,
        log,
        cwd_log,
    }
}

#[cfg(unix)]
pub struct Recorder {
    pub program: PathBuf,
    pub log: PathBuf,
    pub cwd_log: PathBuf,
}

#[cfg(unix)]
impl Recorder {
    /// The argv of the one invocation, or `None` if it was never called.
    ///
    /// **Polls.** fastf spawns the terminal and returns without waiting for it —
    /// that is the whole point, the window outlives the process — so the log is
    /// written some time after the parent has already exited. Reading it once
    /// tests the scheduler rather than fastf.
    pub fn argv(&self) -> Option<Vec<String>> {
        self.wait_for_call(std::time::Duration::from_secs(5))
    }

    /// Was it called within `budget`? Positive assertions should use the long
    /// budget `argv` picks; negative ones need only long enough that a call
    /// already in flight would have landed.
    pub fn wait_for_call(&self, budget: std::time::Duration) -> Option<Vec<String>> {
        let deadline = std::time::Instant::now() + budget;
        loop {
            // A log that exists but has no trailing newline is a `printf` still
            // in progress; waiting for the newline makes the read atomic enough.
            if let Ok(raw) = fs::read_to_string(&self.log)
                && raw.ends_with('\n')
            {
                return Some(raw.lines().map(str::to_string).collect());
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// For the negative assertions: a short settle, so "it was not called" is
    /// not just "it has not been called *yet*".
    pub fn was_called(&self) -> bool {
        self.wait_for_call(std::time::Duration::from_millis(400))
            .is_some()
    }

    /// The working directory of the one invocation, or `None` if it was never
    /// called. Polls, for the same reason `argv` does.
    pub fn cwd(&self) -> Option<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Ok(raw) = fs::read_to_string(&self.cwd_log)
                && raw.ends_with('\n')
            {
                return Some(raw.trim_end().to_string());
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// What a run with no terminal anywhere produced.
#[cfg(unix)]
pub struct LauncherRun {
    /// Everything written to stdout and stderr, which share one socket exactly
    /// as they do under journald.
    pub output: String,
    pub code: i32,
}
