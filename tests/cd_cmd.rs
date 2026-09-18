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

/// bash under a pty with no startup file of its own, running `fastf cd`
/// without the function: the unhooked case every new user meets. `keys` answer
/// the offer and then type into the shell fastf opens, and must end it.
#[cfg(unix)]
fn unhooked_bash(sb: &Sandbox, keys: common::pty::Script) -> String {
    let path = path_with_fastf();
    let script = "fastf cd alpha; echo \"AFTER=$(pwd)\"";
    let (out, code) = common::pty::run(
        "/bin/bash",
        &["--noprofile", "--norc", "-c", script],
        &[
            ("FASTF_INSTALL_DIR", sb.install.as_path()),
            ("HOME", sb.tmp.path()),
            ("PATH", std::path::Path::new(&path)),
            ("PS1", std::path::Path::new("$ ")),
        ],
        &keys.build(),
        std::time::Duration::from_secs(25),
    );
    assert_eq!(code, 0, "{out}");
    common::pty::plain(&out)
}

/// The line the shell fastf opened printed about where it is.
#[cfg(unix)]
fn marker<'a>(plain: &'a str, prefix: &str) -> &'a str {
    plain
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no {prefix} line:\n{plain}"))
}

/// **Nobody edits a file.** In a shell without the function, `fastf cd`
/// offers to add it; `y` writes the guarded line into `~/.bashrc` and opens a
/// new bash *in the project* — which has read that line, so it can move — and
/// `exit` returns to the shell it was typed into, which never moved.
#[cfg(unix)]
#[test]
fn without_the_function_fastf_cd_offers_it_and_opens_a_shell_there() {
    let (sb, dir) = library();
    let keys = common::pty::Script::new()
        .pause(1200)
        .key("y")
        .pause(1500)
        .line("echo \"CWD=$(pwd) KIND=$(type -t fastf)\"; exit");
    let plain = unhooked_bash(&sb, keys);
    assert!(plain.contains("Add it to"), "the offer was made:\n{plain}");
    let rc = std::fs::read_to_string(sb.tmp.path().join(".bashrc")).expect("~/.bashrc written");
    assert!(rc.contains("eval \"$(fastf init bash)\""), "{rc}");
    assert!(rc.contains("command -v fastf"), "guarded: {rc}");
    assert_eq!(
        marker(&plain, "CWD="),
        format!("{} KIND=function", shown_path(&dir)),
        "the new shell is in the project and already has the function:\n{plain}"
    );
    assert_ne!(
        marker(&plain, "AFTER="),
        shown_path(&dir),
        "the shell `fastf cd` was typed into is where it was:\n{plain}"
    );
}

/// `n` is remembered: nothing is written, the new shell opens anyway, and the
/// next `fastf cd` does not ask again.
#[cfg(unix)]
#[test]
fn a_declined_offer_is_not_made_twice_and_the_shell_still_opens() {
    let (sb, dir) = library();
    let keys = common::pty::Script::new()
        .pause(1200)
        .key("n")
        .pause(1500)
        .line("echo \"CWD=$(pwd)\"; exit");
    let plain = unhooked_bash(&sb, keys);
    assert!(plain.contains("Add it to"), "{plain}");
    assert!(!sb.tmp.path().join(".bashrc").exists(), "no means no file");
    assert_eq!(marker(&plain, "CWD="), shown_path(&dir), "{plain}");
    let state = std::fs::read_to_string(sb.install.join("state.toml")).unwrap();
    assert!(
        state.contains("declined_shell_setup = [\"bash\"]"),
        "{state}"
    );

    let keys = common::pty::Script::new()
        .pause(1500)
        .line("echo \"CWD=$(pwd)\"; exit");
    let plain = unhooked_bash(&sb, keys);
    assert!(!plain.contains("Add it to"), "asked twice:\n{plain}");
    assert_eq!(marker(&plain, "CWD="), shown_path(&dir), "{plain}");
}

// ---------------------------------------------------------------------------
// `fastf init` with no shell: setting up the one it was typed into
// ---------------------------------------------------------------------------

/// Run `setup` then, in a fresh shell, `check`, both under `shell` in the
/// sandbox; `None` when the shell is not installed.
#[cfg(unix)]
fn set_up_then_check(
    sb: &Sandbox,
    shell: &str,
    setup: (&[&str], &str),
    check: (&[&str], &str),
) -> Option<(String, String)> {
    let first = shell_session(sb, shell, setup.0, setup.1)?;
    assert!(first.status.success(), "{shell} setup: {first:?}");
    let second = shell_session(sb, shell, check.0, check.1)?;
    assert!(second.status.success(), "{shell} check: {second:?}");
    Some((
        String::from_utf8_lossy(&first.stdout).into_owned(),
        String::from_utf8_lossy(&second.stdout).into_owned(),
    ))
}

/// bash: the lines land in `~/.bashrc`, a shell that reads it moves, and a
/// second `fastf init` writes nothing more.
#[cfg(unix)]
#[test]
fn init_with_no_shell_sets_up_bash() {
    let (sb, dir) = library();
    let Some((said, moved)) = set_up_then_check(
        &sb,
        "bash",
        (&["--noprofile", "--norc", "-c"], "fastf init; true"),
        (
            &["--noprofile", "--norc", "-c"],
            "source ~/.bashrc; fastf cd alpha; pwd; fastf init",
        ),
    ) else {
        return;
    };
    assert!(said.contains(".bashrc"), "{said}");
    let mut lines = moved.lines();
    assert_eq!(lines.next(), Some(shown_path(&dir).as_str()), "{moved}");
    assert!(
        lines.next().unwrap_or_default().contains("already runs"),
        "{moved}"
    );
    let rc = std::fs::read_to_string(sb.tmp.path().join(".bashrc")).unwrap();
    assert_eq!(rc.matches("fastf init bash").count(), 1, "{rc}");
}

/// zsh follows `ZDOTDIR`; fish gets a `conf.d` file of its own, which it reads
/// without being told.
#[cfg(unix)]
#[test]
fn init_with_no_shell_sets_up_zsh_and_fish_where_they_look() {
    let (sb, dir) = library();
    let zdot = sb.tmp.path().join("zdot");
    std::fs::create_dir_all(&zdot).unwrap();
    let zsh = {
        let mut cmd = sb.command_named("zsh");
        cmd.env("ZDOTDIR", &zdot)
            .env("PATH", path_with_fastf())
            .current_dir(sb.tmp.path());
        cmd
    };
    run_pair(
        zsh,
        // `; true`: a lone command is exec'd, and fastf's parent would be
        // this test rather than zsh.
        &["-f", "-c", "fastf init; true"],
        &["-f", "-c", "source $ZDOTDIR/.zshrc; fastf cd alpha; pwd"],
        &shown_path(&dir),
        "zsh",
    );
    assert!(zdot.join(".zshrc").exists(), "zsh reads $ZDOTDIR/.zshrc");

    let xdg = sb.tmp.path().join("xdg");
    let fish = {
        let mut cmd = sb.command_named("fish");
        cmd.env("XDG_CONFIG_HOME", &xdg)
            .env("PATH", path_with_fastf())
            .current_dir(sb.tmp.path());
        cmd
    };
    run_pair(
        fish,
        &["-c", "fastf init; true"],
        &["-c", "fastf cd alpha; pwd"],
        &shown_path(&dir),
        "fish",
    );
}

/// Run `first` then `second` with the environment `base` carries; the last
/// stdout line of `second` is where the shell ended up. Skips with a word when
/// the shell is not installed.
#[cfg(unix)]
fn run_pair(base: std::process::Command, first: &[&str], second: &[&str], want: &str, name: &str) {
    let rerun = |args: &[&str]| {
        let mut cmd = std::process::Command::new(base.get_program());
        for (key, value) in base.get_envs() {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
        if let Some(dir) = base.get_current_dir() {
            cmd.current_dir(dir);
        }
        cmd.args(args).stdin(Stdio::null()).output()
    };
    let out = match rerun(first) {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping: {name} is not installed");
            return;
        }
        Err(e) => panic!("running {name}: {e}"),
    };
    assert!(out.status.success(), "{name} setup: {out:?}");
    let out = rerun(second).expect("the shell ran once already");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{name}: {out:?}");
    assert_eq!(stdout.lines().last(), Some(want), "{name}:\n{stdout}");
}

/// PowerShell is asked where its own profile is, and a new session that loads
/// it moves. Not on Windows, where `$PROFILE` follows the real Documents
/// folder whatever the sandbox says — the suite must not write there.
#[cfg(unix)]
#[test]
fn init_with_no_shell_sets_up_powershell_where_it_says_its_profile_is() {
    let (sb, dir) = library();
    let xdg = sb.tmp.path().join("xdg");
    let pwsh = {
        let mut cmd = sb.command_named("pwsh");
        cmd.env("XDG_CONFIG_HOME", &xdg)
            .env("PATH", path_with_fastf())
            .current_dir(sb.tmp.path());
        cmd
    };
    run_pair(
        pwsh,
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "fastf init",
        ],
        &[
            "-NoLogo",
            "-NonInteractive",
            "-Command",
            "fastf cd alpha; (Get-Location).Path",
        ],
        &shown_path(&dir),
        "pwsh",
    );
}

/// `cmd.exe` has no functions to give; `fastf init` typed there says so and
/// what `fastf cd` does instead. On Windows this is also the proof that the
/// parent process is read at all.
#[cfg(windows)]
#[test]
fn init_in_cmd_says_what_fastf_cd_does_there_instead() {
    let sb = Sandbox::new();
    let out = sb
        .command_named("cmd.exe")
        .args(["/d", "/c", "fastf init"])
        .env("PATH", path_with_fastf())
        .stdin(Stdio::null())
        .output()
        .expect("cmd.exe runs");
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cmd has no shell functions"), "{stderr}");
    assert!(
        stderr.contains("opens a new shell in the project"),
        "{stderr}"
    );
}
