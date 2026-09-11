//! `fastf init <shell>` — the shell function that makes `fastf cd` change
//! directory.
//!
//! A program cannot change the working directory of the shell that ran it:
//! the directory is the shell's own, and a child process only ever has a copy.
//! Every tool that offers a `cd` — zoxide, autojump, `z` — is therefore two
//! parts: a binary that resolves a query to a path, and a function *inside the
//! shell* that captures the path and calls the shell's own `cd` on it. This
//! module is the second part, one text per shell, printed for `eval`.
//!
//! The function shadows `fastf` and intercepts exactly one verb. Everything
//! else is passed to the binary untouched, so the app, the prompts and every
//! other command behave as they do without the hook. For `cd` it captures
//! stdout — the picker draws on stderr, so an ambiguous query still asks — and
//! enters the one line the binary printed when that line is a directory.
//! Anything else on stdout (help text, the "Cancelled" line) is printed as it
//! would have been, so `fastf cd --help` still helps.
//!
//! **The texts are the contract.** They are pasted into `.bashrc`s and read by
//! people who will never read this file, so they say what they are at the top,
//! carry no machine-specific path (the binary is found on `PATH`, as the user
//! found it), and are exercised end to end by `tests/cd_cmd.rs` under every
//! shell the runner has.

use anyhow::{Result, bail};

/// A shell the hook exists for — the same four `fastf completions` knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl Shell {
    /// The spellings `fastf init` accepts, matching `completions` plus the
    /// name PowerShell 7 goes by on its own command line.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            "powershell" | "ps" | "pwsh" => Some(Self::PowerShell),
            _ => None,
        }
    }

    /// The shell the user is most likely typing into: `$SHELL`'s basename on
    /// unix, PowerShell on Windows. A guess for a hint, never a decision.
    pub fn current() -> Option<Self> {
        if cfg!(windows) {
            return Some(Self::PowerShell);
        }
        let shell = std::env::var_os("SHELL")?;
        let name = std::path::Path::new(&shell).file_name()?.to_str()?;
        Self::from_name(name)
    }

    /// The name `fastf init` is given for this shell.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        }
    }

    /// The one line to add, and the file it goes in — what the docs and the
    /// unhooked `fastf cd` both show.
    pub fn setup(self) -> (&'static str, &'static str) {
        match self {
            Self::Bash => (r#"eval "$(fastf init bash)""#, "~/.bashrc"),
            Self::Zsh => (r#"eval "$(fastf init zsh)""#, "~/.zshrc"),
            Self::Fish => ("fastf init fish | source", "~/.config/fish/config.fish"),
            Self::PowerShell => (
                "Invoke-Expression (& fastf init powershell | Out-String)",
                "$PROFILE",
            ),
        }
    }

    /// The function itself.
    pub fn script(self) -> &'static str {
        match self {
            Self::Bash | Self::Zsh => POSIX,
            Self::Fish => FISH,
            Self::PowerShell => POWERSHELL,
        }
    }
}

/// Print the hook for `shell` on stdout.
pub fn run(shell: &str) -> Result<()> {
    let Some(shell) = Shell::from_name(shell) else {
        bail!("unknown shell '{shell}'. Valid: bash, zsh, fish, powershell");
    };
    print!("{}", shell.script());
    Ok(())
}

/// bash and zsh. POSIX apart from `local`, which both have.
///
/// `__fastf_dir` is declared on its own line: `local x=$(cmd)` is one command
/// whose status is `local`'s, and the binary's exit code would be lost.
const POSIX: &str = r#"# fastf shell integration: `fastf cd <query>` changes this shell's directory.
# Printed by `fastf init`. Every other fastf command passes straight through.
fastf() {
    if [ "$1" != "cd" ]; then
        command fastf "$@"
        return
    fi
    shift
    local __fastf_dir
    __fastf_dir=$(command fastf cd "$@") || return
    if [ -d "$__fastf_dir" ]; then
        builtin cd -- "$__fastf_dir"
    elif [ -n "$__fastf_dir" ]; then
        printf '%s\n' "$__fastf_dir"
    fi
}
"#;

/// fish. `set` passes a command substitution's status through, so `or return`
/// after it is the binary's exit code; a multi-line capture is a list, which
/// `test -d` joins and `printf` prints one element per line.
const FISH: &str = r#"# fastf shell integration: `fastf cd <query>` changes this shell's directory.
# Printed by `fastf init`. Every other fastf command passes straight through.
function fastf --wraps fastf --description 'fastf, with cd'
    if test "$argv[1]" != cd
        command fastf $argv
        return
    end
    set -e argv[1]
    set -l __fastf_dir (command fastf cd $argv)
    or return
    if test -d "$__fastf_dir"
        builtin cd -- $__fastf_dir
    else if test -n "$__fastf_dir"
        printf '%s\n' $__fastf_dir
    end
end
"#;

/// PowerShell, 5.1 and 7. The binary is looked up as an *application* each
/// time, because inside a function named `fastf` the bare name is the
/// function. `-LiteralPath` throughout: a project folder can hold `[` and `]`.
const POWERSHELL: &str = r#"# fastf shell integration: `fastf cd <query>` changes this shell's directory.
# Printed by `fastf init`. Every other fastf command passes straight through.
function fastf {
    $exe = Get-Command -Name fastf -CommandType Application -ErrorAction Stop | Select-Object -First 1
    if ($args.Count -eq 0 -or $args[0] -ne 'cd') {
        & $exe @args
        return
    }
    $rest = @($args | Select-Object -Skip 1)
    $lines = @(& $exe cd @rest)
    if ($LASTEXITCODE -ne 0) { return }
    if ($lines.Count -eq 1 -and (Test-Path -LiteralPath $lines[0] -PathType Container)) {
        Set-Location -LiteralPath $lines[0]
    } elseif ($lines.Count -gt 0) {
        $lines
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::Shell;

    const ALL: [Shell; 4] = [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell];

    /// The four names `completions` takes are the four `init` takes, so the
    /// docs can say "the same shells" and be right.
    #[test]
    fn the_names_match_completions() {
        for name in ["bash", "zsh", "fish", "powershell"] {
            assert!(Shell::from_name(name).is_some(), "{name}");
        }
        assert!(Shell::from_name("tcsh").is_none());
        for shell in ALL {
            assert_eq!(Shell::from_name(shell.name()), Some(shell));
        }
    }

    /// Every hook says what it is on its first line, intercepts `cd` and hands
    /// everything else to the binary by a route that cannot recurse into the
    /// function itself.
    #[test]
    fn every_hook_names_itself_intercepts_cd_and_passes_the_rest_through() {
        for shell in ALL {
            let script = shell.script();
            let first = script.lines().next().unwrap_or_default();
            assert!(
                first.starts_with("# fastf shell integration"),
                "{}: a pasted block must say what it is: {first}",
                shell.name()
            );
            assert!(script.contains("cd"), "{}", shell.name());
            let bypass = match shell {
                Shell::PowerShell => "-CommandType Application",
                _ => "command fastf",
            };
            assert!(
                script.contains(bypass),
                "{}: the passthrough must bypass the function: {script}",
                shell.name()
            );
        }
    }

    /// The setup line names the shell it is for, so the hint an unhooked
    /// `fastf cd` prints can be pasted as it is.
    #[test]
    fn the_setup_line_names_its_shell() {
        for shell in ALL {
            let (line, file) = shell.setup();
            assert!(
                line.contains(&format!("fastf init {}", shell.name())),
                "{line}"
            );
            assert!(!file.is_empty());
        }
    }
}
