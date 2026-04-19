use anyhow::Result;
use colored::Colorize;
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

/// Run every enabled post-create action. Individual failures are logged to
/// stderr and do NOT abort — the project on disk is already real and correct;
/// post-create actions are conveniences.
pub fn run(actions: &PostCreate, project_path: &Path, config: &Config) -> Result<()> {
    if actions.is_empty() {
        return Ok(());
    }

    // git_init: idempotent. Silent on success; warn on failure.
    if actions.git_init {
        match Command::new("git")
            .arg("init")
            .current_dir(project_path)
            .status()
        {
            Ok(s) if s.success() => {
                println!("  {} git init", "✓".green());
            }
            Ok(s) => eprintln!(
                "{} git init exited with status {}",
                "warning:".yellow().bold(),
                s
            ),
            Err(e) => eprintln!(
                "{} could not run git: {} (is git installed and on PATH?)",
                "warning:".yellow().bold(),
                e
            ),
        }
    }

    // reveal: open the folder in the system file manager.
    if actions.reveal
        && let Err(e) = reveal_folder(project_path)
    {
        eprintln!(
            "{} could not reveal folder: {}",
            "warning:".yellow().bold(),
            e
        );
    }

    // open_in_editor: spawn the configured editor with the folder.
    if actions.open_in_editor {
        let editor = config.resolve_editor();
        match spawn_editor(&editor, project_path) {
            Ok(()) => println!("  {} opened in {}", "✓".green(), editor),
            Err(e) => eprintln!(
                "{} could not open editor '{}': {}",
                "warning:".yellow().bold(),
                editor,
                e
            ),
        }
    }

    // commands: run arbitrary shell commands with {path} substitution.
    for raw in &actions.commands {
        let cmd = raw.replace("{path}", &project_path.display().to_string());
        match run_shell(&cmd, project_path) {
            Ok(status) if status.success() => {
                println!("  {} {}", "✓".green(), raw.dimmed());
            }
            Ok(status) => eprintln!(
                "{} command exited with status {}: {}",
                "warning:".yellow().bold(),
                status,
                raw
            ),
            Err(e) => eprintln!(
                "{} command failed: {} ({})",
                "warning:".yellow().bold(),
                raw,
                e
            ),
        }
    }

    // print_path: emit the absolute path on its own line so shell pipelines can use it.
    // Done last so noisy command output never trails it.
    if actions.print_path {
        let canonical = project_path
            .canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf());
        println!("{}", canonical.display());
    }

    Ok(())
}

/// Ask the user "Open project folder? [Y/n]" and reveal on Yes.
///
/// Caller is expected to have already filtered out cases where the prompt
/// shouldn't fire (`--yes`, `--no-post`, `prompt_open_after_create=false`,
/// reveal already in resolved post_create, non-TTY stdout). This helper
/// just owns the prompt + reveal call.
pub fn prompt_and_reveal(path: &Path) -> Result<()> {
    let open = dialoguer::Confirm::new()
        .with_prompt("Open project folder?")
        .default(true)
        .interact()?;
    if open {
        reveal_folder(path)?;
    }
    Ok(())
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
