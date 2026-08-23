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

    /// Extra shell commands to run inside the project folder. Each command is
    /// passed to the system shell as-is, with `{path}` replaced by the
    /// absolute project path. Examples:
    ///   - "code ."                 → open in VS Code after reveal
    ///   - "touch .gitkeep"          → drop a marker
    ///   - "echo {path} | clip"      → copy path to Windows clipboard
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

/// Run every enabled post-create action, returning what happened.
///
/// Individual failures are reported, never propagated: the folder exists and is
/// correct whatever the editor did.
pub fn run(actions: &PostCreate, project_path: &Path, config: &Config) -> Result<Vec<Note>> {
    let mut notes = Vec::new();
    if actions.is_empty() {
        return Ok(notes);
    }

    // git_init: idempotent.
    if actions.git_init {
        match Command::new("git")
            .arg("init")
            .current_dir(project_path)
            .status()
        {
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

    // commands: run arbitrary shell commands with {path} substitution.
    for raw in &actions.commands {
        let cmd = raw.replace("{path}", &project_path.display().to_string());
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

    Ok(notes)
}

#[cfg(windows)]
pub fn reveal_folder(path: &Path) -> Result<()> {
    // `start` is a cmd.exe builtin, not an executable.
    // The empty "" is the window title that `start` consumes as its first quoted arg.
    Command::new("cmd")
        .args(["/c", "start", "", &path.display().to_string()])
        .status()?;
    Ok(())
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
    // Editors like `code` on Windows are shipped as .cmd scripts that must go
    // through cmd.exe. Using `cmd /c start "" <editor> <path>` handles both
    // bare binaries and shell-script shims.
    Command::new("cmd")
        .args(["/c", "start", "", editor, &path.display().to_string()])
        .status()?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_editor(editor: &str, path: &Path) -> Result<()> {
    // Respect editors with embedded arguments (e.g. "code --wait").
    let mut parts = editor.split_whitespace();
    let bin = parts.next().unwrap_or(editor);
    let mut cmd = Command::new(bin);
    for arg in parts {
        cmd.arg(arg);
    }
    cmd.arg(path).status()?;
    Ok(())
}

#[cfg(windows)]
fn run_shell(cmd: &str, cwd: &Path) -> std::io::Result<std::process::ExitStatus> {
    Command::new("cmd")
        .args(["/c", cmd])
        .current_dir(cwd)
        .status()
}

#[cfg(not(windows))]
fn run_shell(cmd: &str, cwd: &Path) -> std::io::Result<std::process::ExitStatus> {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .status()
}
