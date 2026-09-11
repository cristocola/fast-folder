//! `fastf cd` and `fastf init` — the verb, and the shell function that makes
//! it one.
//!
//! The binary's half is a process test like any other: what `fastf cd` prints
//! and when it refuses. The function's half is tested as the real thing — each
//! shell the runner has evaluates `fastf init <shell>`, runs `fastf cd`, and
//! its own `pwd` is the assertion — because the function is text pasted into
//! startup files, and no unit test of a string proves a shell accepts it. A
//! shell that is not installed skips with a word, never fails: CI's Linux
//! runner has bash and pwsh, its Windows runner has pwsh, a developer's box
//! has whatever it has.

mod common;

use std::process::{Output, Stdio};

use common::{Sandbox, shown_path};
use fastf::cli::shell_init::Shell;

/// A sandbox with one project to enter and two that share a name.
fn library() -> (Sandbox, std::path::PathBuf) {
    let sb = Sandbox::new();
    let dir = sb.plant_project(&sb.base, "2026-01-01_Alpha_ID0001", "ID0001");
    sb.plant_project(&sb.base, "2026-01-02_Shared_One_ID0002", "ID0002");
    sb.plant_project(&sb.base, "2026-01-03_Shared_Two_ID0003", "ID0003");
    (sb, dir)
}

// ---------------------------------------------------------------------------
// The binary's half
// ---------------------------------------------------------------------------

/// Into a capture — which is what the function is — stdout is the path and
/// nothing else, and the note about the function is not printed, because the
/// function is what is reading.
#[test]
fn cd_prints_the_path_and_nothing_else_into_a_capture() {
    let (sb, dir) = library();
    let out = sb.run(&["cd", "alpha"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        shown_path(&dir),
        "stdout is the path and nothing else"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("fastf init"),
        "a capture is the function reading; it needs no instructions:\n{stderr}"
    );
}

/// With nobody to ask, an ambiguous query is the error `path` gives, to the
/// byte: the candidates, and no picker.
#[test]
fn cd_with_no_terminal_and_an_ambiguous_query_lists_the_candidates() {
    let (sb, _) = library();
    let err = sb.fails_headless(&["cd", "shared"]);
    assert!(err.contains("ambiguous"), "{err}");
    assert!(err.contains("ID0002") && err.contains("ID0003"), "{err}");
    assert_eq!(err, sb.fails_headless(&["path", "shared"]));
}

/// No query means "pick from everything", and everything cannot be picked
/// from without a terminal. The message says that, not `'' is ambiguous`.
#[test]
fn cd_with_no_query_and_no_terminal_says_what_it_needs() {
    let (sb, _) = library();
    let err = sb.fails_headless(&["cd"]);
    assert!(err.contains("needs a terminal"), "{err}");
    assert!(err.contains("fastf cd ID"), "{err}");
    assert!(!err.contains("''"), "{err}");
}

/// The same four names `completions` takes, and the same refusal for a
/// fifth. The output is the module's text, so the docs can quote it.
#[test]
fn init_speaks_the_shells_completions_speak() {
    let sb = Sandbox::new();
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
        assert_eq!(sb.ok(&["init", shell.name()]), shell.script());
    }
    let err = sb.fails(&["init", "tcsh"]);
    assert!(err.contains("unknown shell 'tcsh'"), "{err}");
    assert!(err.contains("bash, zsh, fish, powershell"), "{err}");
}

/// `init` runs from every shell's startup file, so it must never bootstrap:
/// a data directory created, or a banner printed, at every new terminal.
#[test]
fn init_never_touches_the_data_directory() {
    let sb = Sandbox::unconfigured();
    std::fs::remove_dir_all(&sb.install).unwrap();
    let out = sb.run(&["init", "bash"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        !sb.install.exists(),
        "init created the data directory at shell startup"
    );
    assert!(
        out.stderr.is_empty(),
        "init said something at shell startup: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// The function's half, under real shells
// ---------------------------------------------------------------------------

/// `PATH` with this build's `fastf` first, so `command fastf` inside the
/// function is the binary under test and not whatever the developer installed.
fn path_with_fastf() -> std::ffi::OsString {
    let bin_dir = std::path::Path::new(common::FASTF)
        .parent()
        .expect("the test binary has a directory");
    let mut dirs = vec![bin_dir.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(dirs).expect("PATH joins")
}

/// Run `script` under `shell` in the sandbox's environment, started in the
/// sandbox's temp dir. `None` when the shell is not installed here.
fn shell_session(sb: &Sandbox, shell: &str, args: &[&str], script: &str) -> Option<Output> {
    let mut cmd = sb.command_named(shell);
    cmd.args(args)
        .arg(script)
        .env("PATH", path_with_fastf())
        .current_dir(sb.tmp.path())
        .stdin(Stdio::null());
    match cmd.output() {
        Ok(out) => Some(out),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping: {shell} is not installed");
            None
        }
        Err(e) => panic!("running {shell}: {e}"),
    }
}

/// Every session prints the same five lines, and each is one claim:
/// `fastf cd alpha` entered the project; a miss left the shell there; `fastf
/// cd --help` was printed rather than entered; and left the shell there; and a
/// verb that is not `cd` reached the binary.
fn assert_session(shell: &str, out: Output, project: &str) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{shell} exited {:?}:\n{stdout}\n{stderr}",
        out.status.code()
    );
    let lines: Vec<&str> = stdout.lines().map(str::trim_end).collect();
    assert_eq!(lines.len(), 5, "{shell}:\n{stdout}\n{stderr}");
    assert_eq!(
        lines[0], project,
        "{shell}: `fastf cd alpha` must enter the project"
    );
    assert_eq!(
        lines[1], project,
        "{shell}: a query that matches nothing must leave the shell where it was"
    );
    assert!(
        lines[2].contains("directory") && lines[2] != project,
        "{shell}: `fastf cd --help` must print the help, not enter something: {}",
        lines[2]
    );
    assert_eq!(
        lines[3], project,
        "{shell}: `--help` must not move the shell"
    );
    assert!(
        lines[4].starts_with("fastf "),
        "{shell}: every other verb passes through to the binary: {}",
        lines[4]
    );
}

/// bash and zsh share one text; each gets its own session, because `eval`
/// in one is no proof for the other.
#[cfg(unix)]
fn posix_script(shell: &str) -> String {
    format!(
        "eval \"$(fastf init {shell})\" || exit 99\n\
         fastf cd alpha || exit 98\n\
         pwd\n\
         fastf cd nothing-like-this && exit 97\n\
         pwd\n\
         fastf cd --help | head -n 1\n\
         pwd\n\
         fastf --version\n"
    )
}

/// The Windows runner has a `bash` too — Git's — whose `pwd` is a `/c/…`
/// path, so the three POSIX sessions are unix-only. Windows is PowerShell.
#[cfg(unix)]
#[test]
fn the_bash_function_changes_the_shells_directory() {
    let (sb, dir) = library();
    if let Some(out) = shell_session(
        &sb,
        "bash",
        &["--noprofile", "--norc", "-c"],
        &posix_script("bash"),
    ) {
        assert_session("bash", out, &shown_path(&dir));
    }
}

#[cfg(unix)]
#[test]
fn the_zsh_function_changes_the_shells_directory() {
    let (sb, dir) = library();
    if let Some(out) = shell_session(&sb, "zsh", &["-f", "-c"], &posix_script("zsh")) {
        assert_session("zsh", out, &shown_path(&dir));
    }
}

#[cfg(unix)]
#[test]
fn the_fish_function_changes_the_shells_directory() {
    let (sb, dir) = library();
    let script = "fastf init fish | source; or exit 99\n\
                  fastf cd alpha; or exit 98\n\
                  pwd\n\
                  fastf cd nothing-like-this; and exit 97\n\
                  pwd\n\
                  fastf cd --help | head -n 1\n\
                  pwd\n\
                  fastf --version\n";
    if let Some(out) = shell_session(&sb, "fish", &["--no-config", "-c"], script) {
        assert_session("fish", out, &shown_path(&dir));
    }
}

/// PowerShell 7 where it is, Windows PowerShell 5.1 otherwise — the text
/// claims both.
#[test]
fn the_powershell_function_changes_the_shells_directory() {
    let (sb, dir) = library();
    let script = "Invoke-Expression (& fastf init powershell | Out-String)\n\
                  fastf cd alpha\n\
                  if ($LASTEXITCODE -ne 0) { exit 98 }\n\
                  (Get-Location).Path\n\
                  fastf cd nothing-like-this\n\
                  if ($LASTEXITCODE -eq 0) { exit 97 }\n\
                  (Get-Location).Path\n\
                  (fastf cd --help)[0]\n\
                  (Get-Location).Path\n\
                  fastf --version\n";
    let args = ["-NoProfile", "-NonInteractive", "-Command"];
    let out = shell_session(&sb, "pwsh", &args, script)
        .or_else(|| shell_session(&sb, "powershell", &args, script));
    if let Some(out) = out {
        assert_session("powershell", out, &shown_path(&dir));
    }
}

// ---------------------------------------------------------------------------
// With a terminal
// ---------------------------------------------------------------------------

/// The reason the function captures stdout and not both streams: inside it, an
/// ambiguous query still asks. A pty runs bash with the function evaluated,
/// the picker draws on the terminal, Down and Enter choose the second row, and
/// bash's own `pwd` afterwards is the chosen project.
#[cfg(unix)]
#[test]
fn an_ambiguous_query_asks_from_inside_the_function() {
    let (sb, _) = library();
    let path = path_with_fastf();
    let script = "eval \"$(fastf init bash)\"; fastf cd shared; echo \"CWD=$(pwd)\"";
    // A longer beat than a bare fastf needs: bash starts, evaluates the
    // function and spawns fastf before the picker can draw.
    let keys = common::pty::Script::new()
        .pause(1200)
        .down(1)
        .enter()
        .build();
    // The pty harness execs a path, not a name.
    let (out, code) = common::pty::run(
        "/bin/bash",
        &["--noprofile", "--norc", "-c", script],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("PATH", std::path::Path::new(&path)),
        ],
        &keys,
        std::time::Duration::from_secs(20),
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("Change directory to which project?"),
        "the picker must have been shown:\n{out}"
    );
    let expected = shown_path(&sb.base.join("2026-01-03_Shared_Two_ID0003"));
    // Cooked-mode output, with the picker's cursor escapes stripped: the
    // marker shares its line with the sequence that showed the cursor again.
    let plain = common::pty::plain(&out);
    let cwd = plain
        .lines()
        .find_map(|line| line.trim_end().strip_prefix("CWD="))
        .expect("bash printed where it ended up");
    assert_eq!(cwd, expected, "Down then Enter is the second row:\n{plain}");
}

/// Without the function, stdout is the terminal, and a path printed to a
/// terminal is not a directory changed. The note says so, on stderr, and
/// names the setup line for the shell `$SHELL` says the user is in.
#[cfg(unix)]
#[test]
fn without_the_function_a_terminal_gets_the_path_and_the_note() {
    let (sb, dir) = library();
    let keys = common::pty::Script::new().build();
    let (out, code) = common::pty::run(
        common::FASTF,
        &["cd", "alpha"],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("SHELL", std::path::Path::new("/usr/bin/zsh")),
        ],
        &keys,
        std::time::Duration::from_secs(15),
    );
    assert_eq!(code, 0, "{out}");
    let plain = common::pty::plain(&out);
    assert!(
        plain.contains(&shown_path(&dir)),
        "the path is still the answer:\n{plain}"
    );
    assert!(plain.contains("printed, not entered"), "{plain}");
    assert!(
        plain.contains("fastf init zsh") && plain.contains("~/.zshrc"),
        "the note names the user's own shell:\n{plain}"
    );
}
