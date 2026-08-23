use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

use crate::core::config::Config;

/// Optional actions that fastf runs immediately after a project folder is
/// created successfully. All fields default to off — explicit opt-in only.
///
/// Resolution order (mirrors `default_template`):
///   1. If the template defines a `post_create` block, it is used verbatim.
///   2. Otherwise, the global `config.toml` `post_create` block is used.
///   3. If neither is set, nothing happens (current behavior).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PostCreate {
    /// Run `git init` inside the new project folder.
    #[serde(default)]
    pub git_init: bool,

    /// Open the new folder in the system file manager (Explorer / Finder / xdg-open).
    #[serde(default)]
    pub reveal: bool,

    /// Spawn the configured editor (`config.editor` or `$EDITOR`) on the new folder.
    #[serde(default)]
    pub open_in_editor: bool,

    /// Print ONLY the absolute path of the new project on its own line after
    /// creation — makes `cd "$(fastf new ... | tail -1)"` ergonomic in shell scripts.
    #[serde(default)]
    pub print_path: bool,

    /// Extra shell commands to run inside the project folder.
    ///
    /// Each command is passed to the system shell as-is. The shell runs with
    /// the project as its working directory and [`PROJECT_PATH_VAR`] in its
    /// environment, so `.` and `"$FASTF_PROJECT_PATH"` both name the project.
    /// A literal `{path}` is rewritten to a quoted reference to that variable
    /// (see [`rewrite_path_token`]) — the path itself never enters shell
    /// source. Examples:
    ///   - "code ."                       → open in VS Code after reveal
    ///   - "touch .gitkeep"                → drop a marker
    ///   - "echo {path} | clip"            → copy path to Windows clipboard
    #[serde(default)]
    pub commands: Vec<String>,
}

impl PostCreate {
    /// True when the block is entirely empty and running it would be a no-op.
    pub fn is_empty(&self) -> bool {
        !self.git_init
            && !self.reveal
            && !self.open_in_editor
            && !self.print_path
            && self.commands.is_empty()
    }
}

/// One thing a post-create action has to say. `core` collects them; a surface
/// decides how they look, because a script piping stdout does not want an ANSI
/// checkmark and the TUI does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// An action succeeded and is worth confirming.
    Done(String),
    /// An action failed. Never fatal — the project on disk is already real and
    /// correct; post-create actions are conveniences.
    Warning(String),
    /// `print_path`'s line. Not a message about the run: it is the run's
    /// **output**, meant for `$(fastf new ...)`, so it goes to stdout on its own
    /// and last.
    Path(String),
}

/// The environment variable every child fastf spawns for a project can read the
/// project's path from.
///
/// It exists so a command never has to interpolate the path into its own source.
/// A folder name is user data: `sanitize_name` leaves `;`, `&`, `$`, `(`, `)`,
/// backtick, `^` and `%` alone, all of which are legal in a folder name and all
/// of which are syntax to a shell.
pub const PROJECT_PATH_VAR: &str = "FASTF_PROJECT_PATH";

/// Rewrite `{path}` into a quoted reference to [`PROJECT_PATH_VAR`].
///
/// `{path}` is kept rather than deprecated: existing configs go on working, and
/// after this rewrite the path never appears in shell source at all. A shell
/// expands a double-quoted variable without re-parsing the result, so a folder
/// called `Live; rm -rf ~` becomes one argument rather than two commands.
///
/// A token already wrapped in a matching pair of quotes is replaced **as a
/// unit**, so `code "{path}"` does not come out double-quoted. On Windows the
/// expansion is `"%FASTF_PROJECT_PATH%"`: a Windows path cannot contain `"`, so
/// the quoting is safe for every legal path.
pub fn rewrite_path_token(raw: &str) -> String {
    let reference = if cfg!(windows) {
        concat!("\"%", "FASTF_PROJECT_PATH", "%\"")
    } else {
        concat!("\"$", "FASTF_PROJECT_PATH", "\"")
    };

    raw.replace("\"{path}\"", reference)
        .replace("'{path}'", reference)
        .replace("{path}", reference)
}

/// Run every enabled post-create action, returning what happened.
///
/// Individual failures are reported, never propagated: the folder exists and is
/// correct whatever the editor did.
pub fn run(actions: &PostCreate, project_path: &Path, config: &Config) -> Vec<Note> {
    let mut notes = Vec::new();
    if actions.is_empty() {
        return notes;
    }

    // git_init: idempotent.
    if actions.git_init {
        match project_command("git", project_path).arg("init").status() {
            Ok(s) if s.success() => notes.push(Note::Done("git init".to_string())),
            Ok(s) => notes.push(Note::Warning(format!("git init exited with status {s}"))),
            Err(e) => notes.push(Note::Warning(format!(
                "could not run git: {e} (is git installed and on PATH?)"
            ))),
        }
    }

    // reveal: open the folder in the system file manager.
    if actions.reveal
        && let Err(e) = reveal_folder(project_path)
    {
        notes.push(Note::Warning(format!("could not reveal folder: {e}")));
    }

    // open_in_editor: spawn the configured editor with the folder.
    if actions.open_in_editor {
        let editor = config.resolve_editor();
        match spawn_editor(&editor, project_path) {
            Ok(()) => notes.push(Note::Done(format!("opened in {editor}"))),
            Err(e) => notes.push(Note::Warning(format!(
                "could not open editor '{editor}': {e}"
            ))),
        }
    }

    // commands: run arbitrary shell commands. The project reaches the shell as
    // an environment variable and a working directory, never as source text.
    for raw in &actions.commands {
        let cmd = rewrite_path_token(raw);
        match run_shell(&cmd, project_path) {
            Ok(status) if status.success() => notes.push(Note::Done(raw.clone())),
            Ok(status) => notes.push(Note::Warning(format!(
                "command exited with status {status}: {raw}"
            ))),
            Err(e) => notes.push(Note::Warning(format!("command failed: {raw} ({e})"))),
        }
    }

    // print_path: the absolute path on its own line so shell pipelines can use
    // it. Last, so noisy command output never trails it.
    if actions.print_path {
        let canonical = project_path
            .canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf());
        notes.push(Note::Path(canonical.display().to_string()));
    }

    notes
}

/// A child process that knows which project it is being run for.
///
/// **Every child fastf spawns for a project goes through this**: the project
/// arrives as `current_dir` and as [`PROJECT_PATH_VAR`], so a command never has
/// to interpolate a path into its own source to find the folder it was run for.
fn project_command(program: &str, project_path: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(project_path)
        .env(PROJECT_PATH_VAR, project_path);
    command
}

#[cfg(windows)]
pub fn reveal_folder(path: &Path) -> Result<()> {
    // Not `cmd /c start "" <path>`. std quotes the argument correctly, but
    // `cmd.exe` expands `%VAR%` inside the command line it reconstructs, so a
    // folder named `%USERPROFILE%` opened somewhere else entirely.
    // `ShellExecuteW` takes the path as an argument, with no command line to
    // expand, and honours the same default handler `start` did.
    crate::util::shell_open::open(path)
}

#[cfg(target_os = "macos")]
pub fn reveal_folder(path: &Path) -> Result<()> {
    Command::new("open").arg(path).status()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn reveal_folder(path: &Path) -> Result<()> {
    Command::new("xdg-open").arg(path).status()?;
    Ok(())
}

#[cfg(windows)]
fn spawn_editor(editor: &str, path: &Path) -> Result<()> {
    // Editors like `code` on Windows ship as .cmd shims that only cmd.exe can
    // resolve, so this one child still goes through a shell. The *path* does
    // not: it is passed as `"%FASTF_PROJECT_PATH%"`, which cmd expands to the
    // variable set below rather than to anything in the folder's own name.
    project_command("cmd", path)
        .args(["/c", "start", "", editor, "\"%FASTF_PROJECT_PATH%\""])
        .status()?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_editor(editor: &str, path: &Path) -> Result<()> {
    // Respect editors with embedded arguments (e.g. "code --wait"). The path is
    // already data here — `arg` never reaches a shell — so only the variable is
    // added.
    let mut parts = editor.split_whitespace();
    let bin = parts.next().unwrap_or(editor);
    let mut cmd = project_command(bin, path);
    for arg in parts {
        cmd.arg(arg);
    }
    cmd.arg(path).status()?;
    Ok(())
}

#[cfg(windows)]
fn run_shell(cmd: &str, cwd: &Path) -> std::io::Result<std::process::ExitStatus> {
    project_command("cmd", cwd).args(["/c", cmd]).status()
}

#[cfg(not(windows))]
fn run_shell(cmd: &str, cwd: &Path) -> std::io::Result<std::process::ExitStatus> {
    project_command("sh", cwd).arg("-c").arg(cmd).status()
}

#[cfg(test)]
mod tests {
    use super::{PROJECT_PATH_VAR, rewrite_path_token};

    /// What the token expands to on this platform, built from the same constant
    /// the code uses so a rename cannot make the test agree with itself.
    fn reference() -> String {
        if cfg!(windows) {
            format!("\"%{PROJECT_PATH_VAR}%\"")
        } else {
            format!("\"${PROJECT_PATH_VAR}\"")
        }
    }

    #[test]
    fn a_bare_token_becomes_a_quoted_variable() {
        assert_eq!(rewrite_path_token("{path}"), reference());
        assert_eq!(
            rewrite_path_token("ls -la {path}"),
            format!("ls -la {}", reference())
        );
        assert_eq!(
            rewrite_path_token("{path} | wc -l"),
            format!("{} | wc -l", reference())
        );
    }

    /// `code "{path}"` must not come out `code ""$FASTF_PROJECT_PATH""` — the
    /// wrapped token is replaced as a unit.
    #[test]
    fn an_already_quoted_token_is_replaced_as_a_unit() {
        assert_eq!(
            rewrite_path_token(r#"code "{path}""#),
            format!("code {}", reference())
        );
        assert_eq!(
            rewrite_path_token("code '{path}'"),
            format!("code {}", reference())
        );
    }

    #[test]
    fn every_occurrence_is_rewritten_and_nothing_else_is_touched() {
        assert_eq!(
            rewrite_path_token("cp -r {path} {path}.bak"),
            format!("cp -r {} {}.bak", reference(), reference())
        );
        assert_eq!(rewrite_path_token("npm install"), "npm install");
        // A near-miss is left alone: only the exact token is a token.
        assert_eq!(rewrite_path_token("echo {paths}"), "echo {paths}");
        assert_eq!(rewrite_path_token("echo $PATH"), "echo $PATH");
    }

    /// The point of the whole rewrite: after it, no part of the project's own
    /// name is shell source, so a folder called `Live; rm -rf ~` cannot split
    /// the command.
    #[test]
    fn the_rewritten_command_contains_no_path_at_all() {
        let rewritten = rewrite_path_token("test -d {path} && touch {path}/ok");
        assert!(!rewritten.contains('/') || !rewritten.contains("{path}"));
        assert!(
            rewritten.contains(PROJECT_PATH_VAR),
            "the variable must be what the shell sees: {rewritten}"
        );
        assert!(
            !rewritten.contains("{path}"),
            "no token may survive: {rewritten}"
        );
    }
}
