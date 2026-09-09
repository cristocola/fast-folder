//! `fastf note add` and `fastf notes` — per-project journal.
//!
//! Entries are timestamped lines in the `## Journal` section of
//! `PROJECT_INFO.md`.  They are append-only — fastf never edits or deletes
//! existing entries.
//!
//! # Adding entries
//! ```bash
//! fastf note add ID0047 "finished final mix"    # inline message
//! fastf note add ID0047 -                        # read from stdin
//! fastf note add ID0047                          # open $EDITOR
//! ```
//!
//! # Viewing entries
//! ```bash
//! fastf notes ID0047                             # all entries
//! fastf notes ID0047 --since 2026-04-01          # entries on/after a date
//! ```

use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::io::{self, Read};
use std::path::Path;

use crate::core::library;
use crate::core::{config::Config, project_info};

// ---------------------------------------------------------------------------
// Add a journal entry
// ---------------------------------------------------------------------------

pub struct NoteAddArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// Inline message text, `-` to read from stdin, or `None` to open $EDITOR.
    pub message: Option<String>,
}

pub fn add(args: NoteAddArgs) -> Result<()> {
    let cfg = Config::load()?;
    let candidate = library::resolve(&cfg, &args.query)?;
    let pinfo = project_info::pinfo_path(&candidate.path);

    if !pinfo.exists() {
        // The same sentence `fastf tag` says about the same condition.
        bail!(
            "{}",
            crate::cli::tag::no_metadata_message(&candidate.id, &candidate.path)
        );
    }

    // `resolve_editor()`, not the raw field: an unset `editor` (the default)
    // must fall back to $EDITOR, exactly as post-create does. Passing the raw
    // field made the documented "omit the message to open your editor" mode fail
    // with `launching editor ''` on every default install.
    let message = resolve_message(
        args.message.as_deref(),
        &cfg.resolve_editor(),
        Some(&candidate.path),
    )?;
    let message = message.trim().to_string();

    if message.is_empty() {
        bail!("journal entry is empty — nothing written");
    }

    crate::core::operations::append_note(&candidate, &message)
        .with_context(|| format!("appending journal entry to {}", pinfo.display()))?;

    println!(
        "{}  Journal entry added to {}",
        "✓".green().bold(),
        candidate.id.green().bold()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// List / view journal entries
// ---------------------------------------------------------------------------

pub struct NotesArgs {
    /// Project ID, prefix, or name substring.
    pub query: String,
    /// Only show entries on or after this ISO-8601 date prefix (e.g. `2026-04-01`).
    pub since: Option<String>,
}

pub fn notes(args: NotesArgs) -> Result<()> {
    let cfg = Config::load()?;
    let project = library::resolve(&cfg, &args.query)?;

    let entries = project_info::read_journal_entries(&project.path)?;

    println!(
        "  {} {} {}",
        "→".cyan().bold(),
        project.id.green().bold(),
        project.name.bold()
    );

    let filtered: Vec<_> = entries
        .iter()
        .filter(|e| {
            if let Some(since) = &args.since {
                e.timestamp.as_str() >= since.as_str()
            } else {
                true
            }
        })
        .collect();

    if filtered.is_empty() {
        if args.since.is_some() {
            println!("    {}", "(no entries since that date)".dimmed());
        } else {
            println!(
                "    {}",
                "(no journal entries yet — use `fastf note add` to add one)".dimmed()
            );
        }
        return Ok(());
    }

    println!();
    for entry in &filtered {
        // `get`, not a byte slice: a hand-edited PROJECT_INFO.md can put any
        // text where the timestamp goes, and slicing to 10 bytes panicked
        // mid-character on the first multi-byte one.
        let date = entry.timestamp.get(..10).unwrap_or(&entry.timestamp);
        println!("  {} {}  {}", "•".dimmed(), date.dimmed(), entry.message);
    }
    println!();
    println!(
        "  {}",
        format!(
            "{} entr{}",
            filtered.len(),
            if filtered.len() == 1 { "y" } else { "ies" }
        )
        .dimmed()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the message text from the three input modes. `cwd` is the project's
/// folder, where the editor is started.
fn resolve_message(raw: Option<&str>, editor: &str, cwd: Option<&Path>) -> Result<String> {
    match raw {
        // stdin sentinel
        Some("-") => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .context("reading from stdin")?;
            Ok(buf)
        }
        // inline text
        Some(text) => Ok(text.to_string()),
        // open editor
        None => open_in_editor(editor, cwd),
    }
}

/// A scratch file that removes itself however the function exits.
///
/// The old path was a predictable `/tmp/fastf-note-<pid>.txt` written with
/// `fs::write`, which follows a symlink someone else planted there, and which
/// leaked whenever the editor exited non-zero.
struct ScratchFile(std::path::PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Create a scratch file that did not exist a moment ago, seed it with the
/// prompt comment, and **close it** before it is handed to an editor.
///
/// `create_new` is the load-bearing part: it opens with `O_CREAT | O_EXCL`,
/// which refuses an existing path and does **not** follow a symlink — so a
/// pre-planted link cannot redirect the write. The name only has to be unlikely
/// enough to avoid honest collisions; the exclusivity is what provides safety,
/// and a collision just means trying again.
///
/// The handle is dropped here rather than returned. Exclusivity is decided at
/// the moment of creation, and keeping the handle open for as long as the
/// editor ran added nothing to it — but on Windows it was a sharing violation
/// waiting to happen: a handle open for writing forbids any later open that
/// does not grant `FILE_SHARE_WRITE`, and Notepad, like most Win32 editors,
/// saves by reopening the file for writing with `FILE_SHARE_READ` alone. Every
/// save failed. Notepad answered with a Save As dialog opened in fastf's
/// working directory — the install folder under `Program Files`, from the
/// Start Menu shortcut, where the second attempt was refused as well — and
/// the note never reached the journal.
fn create_scratch_file() -> Result<ScratchFile> {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    let dir = std::env::temp_dir();
    for attempt in 0..8u32 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(attempt);
        let path = dir.join(format!(
            "fastf-note-{}-{}-{}.txt",
            std::process::id(),
            nanos,
            attempt
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                // The guard first, so a failed seed write still removes the
                // file it created.
                let scratch = ScratchFile(path);
                // A prompt comment so the editor opens with some context.
                file.write_all(b"# Enter your journal note. Lines starting with # are ignored.\n")
                    .context("writing editor temp file")?;
                file.flush().context("writing editor temp file")?;
                drop(file);
                return Ok(scratch);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err).with_context(|| format!("creating {}", path.display()));
            }
        }
    }
    bail!("could not create a scratch file in {}", dir.display())
}

/// Open the configured editor and return what the user wrote.
///
/// The TUI's "New note" action reaches the same scratch-file + editor flow
/// through `note_from_editor` while suspended on the main screen. `cwd` is
/// the project's folder — see `open_in_editor`.
pub(crate) fn note_from_editor(editor: &str, cwd: Option<&Path>) -> Result<String> {
    open_in_editor(editor, cwd)
}

/// Open the configured editor on a scratch file and return what the user
/// wrote.
///
/// The editor is started in `cwd`, the project's folder, so whatever it does
/// relative to its working directory — a Save As dialog, a plugin looking for
/// a workspace — begins in the project rather than wherever fastf happened to
/// be started, which from a desktop shortcut is the install folder. The same
/// rule post-create's editor and commands follow.
fn open_in_editor(editor: &str, cwd: Option<&Path>) -> Result<String> {
    let scratch = create_scratch_file()?;

    // `code --wait` is one editor: the first word is the program, the rest
    // its arguments — the same split post-create's editor gets.
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or(editor);
    let mut command = std::process::Command::new(program);
    command.args(parts).arg(&scratch.0);
    // Only a folder that is there: a project that vanished between the
    // listing and the keypress should fail on the journal write, in its own
    // words, not on an editor that could not start.
    if let Some(dir) = cwd.filter(|dir| dir.is_dir()) {
        command.current_dir(dir);
    }
    let status = command.status().with_context(|| {
        format!(
            "launching editor '{editor}'. Set one with `fastf config set editor <cmd>` \
             or $EDITOR, or pass the message inline: fastf note add <id> \"...\""
        )
    })?;

    if !status.success() {
        bail!("editor exited with non-zero status — nothing written");
    }

    let raw = match std::fs::read_to_string(&scratch.0) {
        Ok(raw) => raw,
        // An editor that saves by writing a new file and renaming it over the
        // old one leaves a file at the path; one that deleted it wrote nothing
        // anybody can read.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => bail!(
            "the editor left no file at {} — nothing written",
            scratch.0.display()
        ),
        Err(err) => return Err(err).context("reading editor temp file"),
    };

    // Strip comment lines
    Ok(raw
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n"))
}
