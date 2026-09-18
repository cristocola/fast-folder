//! Teaching a shell `fastf cd` without anybody editing a file.
//!
//! The function `fastf init` prints has to be evaluated by the shell at every
//! start, which means a line in that shell's startup file. Nobody installing
//! fastf should have to know that, so fastf writes the line itself: `fastf cd`
//! offers it the first time it runs in a shell that lacks it, and `fastf init`
//! with no shell named writes it without asking. This module is where the line
//! goes and what it says.
//!
//! **The shell is the one fastf was typed into** — the parent process — and
//! `$SHELL` only when the parent is not a shell at all. It decides both the
//! file and, when no file can help, the program `fastf cd` starts instead.
//!
//! **The line survives fastf.** An uninstall cannot reach into a home
//! directory, so every line is guarded by "is fastf on `PATH`", and a shell
//! whose fastf is gone starts as it did before. It says what it is and how to
//! undo it, in the file itself, because that is where somebody meeting it will
//! be. ASCII only: Windows PowerShell 5.1 reads a profile without a BOM in the
//! ANSI code page.
//!
//! **Files are appended to, never replaced.** A startup file is often a
//! symlink into a dotfiles repository; an atomic replace would swap the link
//! for a copy. fish is the exception that proves it: `conf.d/fastf.fish` is a
//! file fastf owns outright.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli::shell_init::Shell;
use crate::util::parent_process;
use crate::util::paths::display_path;

/// What every line fastf writes says about itself, in every shell's comment
/// syntax, which is `#` for all four.
const COMMENT: &str =
    "# Added by fastf: lets `fastf cd` change this shell's directory. Delete these lines to undo.";

/// Shells fastf recognises as the parent but cannot give a function to. Their
/// only use here is to say "the parent *is* a shell, start another of it",
/// rather than falling back to `$SHELL`.
const OTHER_SHELLS: &[&str] = &[
    "sh", "dash", "ash", "ksh", "mksh", "oksh", "yash", "tcsh", "csh", "nu", "elvish", "xonsh",
    "ion", "osh", "cmd",
];

/// The shell `fastf` was typed into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    /// The program to start for a new shell of the same kind.
    pub program: PathBuf,
    /// Which of the four `fastf init` speaks, if it is one.
    pub shell: Option<Shell>,
}

impl Host {
    /// The parent when it is a shell, else `$SHELL`, else nothing to say.
    pub fn detect() -> Option<Host> {
        if let Some(exe) = parent_process::parent_executable()
            && let Some(host) = Self::from_program(exe)
        {
            return Some(host);
        }
        #[cfg(unix)]
        if let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) {
            let program = PathBuf::from(shell);
            let name = parent_process::stem(&program);
            return Some(Host {
                shell: name.as_deref().and_then(Shell::from_name),
                program,
            });
        }
        None
    }

    /// A host for `program` if its name is a shell's.
    pub fn from_program(program: PathBuf) -> Option<Host> {
        let name = parent_process::stem(&program)?;
        let shell = Shell::from_name(&name);
        (shell.is_some() || OTHER_SHELLS.contains(&name.as_str()))
            .then_some(Host { program, shell })
    }

    /// The name to call it by in a sentence.
    pub fn name(&self) -> String {
        parent_process::stem(&self.program).unwrap_or_else(|| "this shell".to_string())
    }
}

/// Where a shell stands with the function.
#[derive(Debug)]
pub enum Setup {
    /// Its startup file already runs `fastf init` — so a shell that still
    /// cannot move started before the line was written.
    Present(PathBuf),
    /// One line away.
    Missing(Plan),
    /// Nothing fastf can write would help, and why.
    Unavailable(String),
}

/// The line, and the file it goes in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub shell: Shell,
    pub file: PathBuf,
    /// A file fastf writes whole (fish's `conf.d`), rather than appends to.
    owned: bool,
}

/// Look at `host`'s startup file without changing anything.
pub fn inspect(host: &Host) -> Setup {
    let Some(shell) = host.shell else {
        return Setup::Unavailable(format!(
            "{} has no shell functions fastf can give it",
            host.name()
        ));
    };
    let plan = match plan_for(shell, &host.program) {
        Ok(plan) => plan,
        Err(reason) => return Setup::Unavailable(reason),
    };
    // A line somebody added by hand counts: it is the same function.
    let mut places = vec![plan.file.clone()];
    if shell == Shell::Fish
        && let Some(config) = plan.file.parent().and_then(Path::parent)
    {
        places.push(config.join("config.fish"));
    }
    for place in places {
        if runs_init(&place, shell) {
            return Setup::Present(place);
        }
    }
    Setup::Missing(plan)
}

/// Does the file at `path` already evaluate `fastf init` for `shell`?
fn runs_init(path: &Path, shell: Shell) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let spellings: &[&str] = match shell {
        Shell::PowerShell => &["powershell", "pwsh", "ps"],
        _ => &[shell.name()],
    };
    spellings
        .iter()
        .any(|name| text.contains(&format!("fastf init {name}")))
}

fn plan_for(shell: Shell, program: &Path) -> Result<Plan, String> {
    let home =
        || crate::util::paths::home_dir().ok_or_else(|| "no home directory is set".to_string());
    let env_dir = |var: &str| {
        std::env::var_os(var)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let (file, owned) = match shell {
        Shell::Bash => (home()?.join(".bashrc"), false),
        Shell::Zsh => {
            let dir = match env_dir("ZDOTDIR") {
                Some(dir) => dir,
                None => home()?,
            };
            (dir.join(".zshrc"), false)
        }
        Shell::Fish => {
            let config = match env_dir("XDG_CONFIG_HOME") {
                Some(dir) => dir,
                None => home()?.join(".config"),
            };
            (config.join("fish").join("conf.d").join("fastf.fish"), true)
        }
        Shell::PowerShell => (powershell_profile(program)?, false),
    };
    Ok(Plan { shell, file, owned })
}

/// Ask PowerShell itself where its profile is and whether it will run one.
///
/// Only PowerShell knows: the path follows the Documents folder wherever
/// OneDrive or a policy has moved it, and differs between 5.1 and 7. A policy
/// of `Restricted` or `AllSigned` runs no unsigned profile at all — a line
/// written there would be an error at every start — so that is a no.
fn powershell_profile(program: &Path) -> Result<PathBuf, String> {
    let out = std::process::Command::new(program)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PROFILE; (Get-ExecutionPolicy).ToString()",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| {
            format!(
                "could not ask {} where its profile is: {e}",
                program.display()
            )
        })?;
    if !out.status.success() {
        return Err(format!(
            "{} would not say where its profile is",
            program.display()
        ));
    }
    parse_powershell_answer(&String::from_utf8_lossy(&out.stdout))
}

/// The two lines [`powershell_profile`] asked for: a path, then a policy.
fn parse_powershell_answer(text: &str) -> Result<PathBuf, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let (Some(profile), Some(policy)) = (lines.next(), lines.next()) else {
        return Err("PowerShell did not say where its profile is".to_string());
    };
    if policy.eq_ignore_ascii_case("Restricted") || policy.eq_ignore_ascii_case("AllSigned") {
        return Err(format!(
            "PowerShell's execution policy ({policy}) runs no profile script"
        ));
    }
    Ok(PathBuf::from(profile))
}

impl Plan {
    /// The lines to write.
    pub fn block(&self) -> String {
        let name = self.shell.name();
        let body = match self.shell {
            Shell::Bash | Shell::Zsh => format!(
                "if command -v fastf >/dev/null 2>&1; then eval \"$(fastf init {name})\"; fi"
            ),
            Shell::Fish => "if command -q fastf\n    fastf init fish | source\nend".to_string(),
            Shell::PowerShell => "if (Get-Command fastf -CommandType Application -ErrorAction \
                                  SilentlyContinue) { Invoke-Expression (& fastf init powershell \
                                  | Out-String) }"
                .to_string(),
        };
        format!("{COMMENT}\n{body}\n")
    }

    /// The file, as a person would write it.
    pub fn shown(&self) -> String {
        display_path(&self.file)
    }

    /// Write the line. Appends, after a blank line, to a file that has other
    /// things in it; creates the file and its directories when there is none.
    pub fn apply(&self) -> Result<()> {
        let file = &self.file;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating {}", display_path(dir)))?;
        }
        if self.owned {
            return crate::util::atomic::write(file, self.block())
                .with_context(|| format!("writing {}", display_path(file)));
        }
        let existing = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", display_path(file)));
            }
        };
        let mut text = String::new();
        if !existing.is_empty() {
            if !existing.ends_with(b"\n") {
                text.push('\n');
            }
            text.push('\n');
        }
        text.push_str(&self.block());
        let mut handle = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .with_context(|| format!("opening {}", display_path(file)))?;
        handle
            .write_all(text.as_bytes())
            .and_then(|()| handle.sync_all())
            .with_context(|| format!("writing {}", display_path(file)))
    }
}

/// `fastf init` with no shell: set up the shell it was typed into.
pub fn install_here() -> Result<()> {
    let Some(host) = Host::detect() else {
        bail!(
            "could not tell which shell this is — name it: `fastf init bash`, `zsh`, `fish` or \
             `powershell` prints the function to add yourself"
        );
    };
    match inspect(&host) {
        Setup::Present(file) => {
            println!(
                "{} already runs `fastf init`; a new {} terminal changes directory with `fastf cd`.",
                display_path(&file),
                host.name()
            );
        }
        Setup::Missing(plan) => {
            plan.apply()?;
            println!(
                "Set up `fastf cd` in {}. New {} terminals change directory with it.",
                plan.shown(),
                host.name()
            );
        }
        Setup::Unavailable(reason) => {
            bail!("{reason}, so `fastf cd` opens a new shell in the project there instead")
        }
    }
    if let Some(shell) = host.shell {
        declines::forget(shell);
    }
    Ok(())
}

/// Shells the user has said no to, kept in the session state so `fastf cd`
/// asks once and not at every run. Best effort, as the session is: a lost
/// answer means one more question, never an error.
pub mod declines {
    use super::Shell;
    use crate::tui::session::Session;

    pub fn contains(shell: Shell) -> bool {
        // Read only when the file is there: `fastf cd` must not report a
        // missing state file it never needed.
        crate::tui::session::path().exists()
            && Session::load()
                .declined_shell_setup
                .iter()
                .any(|name| name == shell.name())
    }

    pub fn remember(shell: Shell) {
        let mut session = Session::load();
        if !session
            .declined_shell_setup
            .iter()
            .any(|n| n == shell.name())
        {
            session.declined_shell_setup.push(shell.name().to_string());
            if let Err(e) = session.save() {
                crate::util::diag::note(format!("could not remember the answer: {e:#}"));
            }
        }
    }

    pub fn forget(shell: Shell) {
        if !contains(shell) {
            return;
        }
        let mut session = Session::load();
        session.declined_shell_setup.retain(|n| n != shell.name());
        if let Err(e) = session.save() {
            crate::util::diag::note(format!("could not update the session state: {e:#}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(shell: Shell, owned: bool) -> Plan {
        Plan {
            shell,
            file: std::env::temp_dir().join("unused"),
            owned,
        }
    }

    /// Every block runs `fastf init` for its own shell — which is what
    /// [`runs_init`] looks for, so a written block is recognised next time —
    /// guards it on fastf being installed, says what it is, and is ASCII.
    #[test]
    fn every_block_is_guarded_recognisable_and_ascii() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
            let block = plan(shell, false).block();
            assert!(
                block.contains(&format!("fastf init {}", shell.name())),
                "{block}"
            );
            assert!(block.starts_with("# Added by fastf"), "{block}");
            assert!(
                block.contains("command -v fastf")
                    || block.contains("command -q fastf")
                    || block.contains("Get-Command fastf"),
                "an uninstalled fastf must not break the shell: {block}"
            );
            assert!(block.is_ascii(), "{block}");
        }
    }

    #[test]
    fn a_restricted_powershell_is_not_given_a_profile() {
        let ok = parse_powershell_answer("C:\\Users\\user\\Documents\\p.ps1\r\nRemoteSigned\r\n");
        assert_eq!(ok, Ok(PathBuf::from("C:\\Users\\user\\Documents\\p.ps1")));
        for policy in ["Restricted", "AllSigned"] {
            let err = parse_powershell_answer(&format!("C:\\p.ps1\n{policy}\n")).unwrap_err();
            assert!(err.contains(policy), "{err}");
        }
        assert!(parse_powershell_answer("").is_err());
    }

    #[test]
    fn only_shell_names_make_a_host() {
        let host = Host::from_program(PathBuf::from("/usr/bin/zsh")).unwrap();
        assert_eq!(host.shell, Some(Shell::Zsh));
        let cmd = Host::from_program(PathBuf::from("cmd.exe")).unwrap();
        assert_eq!(cmd.shell, None);
        assert!(Host::from_program(PathBuf::from("/usr/bin/cargo")).is_none());
        assert!(Host::from_program(PathBuf::from("/usr/bin/kitty")).is_none());
    }

    /// Appending keeps what was there, separates it by a blank line, and a
    /// second inspection finds the function present.
    #[test]
    fn apply_appends_and_is_then_found() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("dir").join(".bashrc");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "alias ll='ls -l'").unwrap();
        let plan = Plan {
            shell: Shell::Bash,
            file: file.clone(),
            owned: false,
        };
        plan.apply().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.starts_with("alias ll='ls -l'\n\n# Added by fastf"),
            "{text}"
        );
        assert!(runs_init(&file, Shell::Bash));
        assert!(!runs_init(&file, Shell::Zsh));
    }
}
