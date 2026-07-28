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
    let cfg = Config::load().unwrap_or_default();
    let project = library::resolve(&cfg, &args.query)?;
    let pinfo = project_info::pinfo_path(&project.path);

    if !pinfo.exists() {
        bail!(
            "no {} found for project {}",
            project_info::RESERVED_FILENAME,
            project.id
        );
    }

    // `resolve_editor()`, not the raw field: an unset `editor` (the default)
    // must fall back to $EDITOR, exactly as post-create does. Passing the raw
    // field made the documented "omit the message to open your editor" mode fail
    // with `launching editor ''` on every default install.
    let message = resolve_message(args.message.as_deref(), &cfg.resolve_editor())?;
    let message = message.trim().to_string();

    if message.is_empty() {
        bail!("journal entry is empty — nothing written");
    }

    // Read-modify-write of the journal section — atomic on disk, but two
    // appends at once (a terminal and the browser UI) would otherwise keep only
    // one. Taken after the editor closes, never across it.
    let _data_lock = crate::util::lockfile::DataLock::acquire()?;
    project_info::append_journal_entry(&pinfo, &message)
        .with_context(|| format!("appending journal entry to {}", pinfo.display()))?;

    println!(
        "{}  Journal entry added to {}",
        "✓".green().bold(),
        project.id.green().bold()
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
    let cfg = Config::load().unwrap_or_default();
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

/// Resolve the message text from the three input modes.
fn resolve_message(raw: Option<&str>, editor: &str) -> Result<String> {
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
        None => open_in_editor(editor),
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

/// Create a scratch file that did not exist a moment ago.
///
/// `create_new` is the load-bearing part: it opens with `O_CREAT | O_EXCL`,
/// which refuses an existing path and does **not** follow a symlink — so a
/// pre-planted link cannot redirect the write. The name only has to be unlikely
/// enough to avoid honest collisions; the exclusivity is what provides safety,
/// and a collision just means trying again.
fn create_scratch_file() -> Result<(ScratchFile, std::fs::File)> {
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
                // A prompt comment so the editor opens with some context.
                file.write_all(b"# Enter your journal note. Lines starting with # are ignored.\n")
                    .context("writing editor temp file")?;
                file.flush().context("writing editor temp file")?;
                return Ok((ScratchFile(path), file));
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
fn open_in_editor(editor: &str) -> Result<String> {
    let (scratch, _handle) = create_scratch_file()?;

    let status = std::process::Command::new(editor)
        .arg(&scratch.0)
        .status()
        .with_context(|| {
            format!(
                "launching editor '{editor}'. Set one with `fastf config set editor <cmd>` \
                 or $EDITOR, or pass the message inline: fastf note add <id> \"...\""
            )
        })?;

    if !status.success() {
        bail!("editor exited with non-zero status — nothing written");
    }

    let raw = std::fs::read_to_string(&scratch.0).context("reading editor temp file")?;

    // Strip comment lines
    Ok(raw
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n"))
}
