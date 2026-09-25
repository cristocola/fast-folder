//! `fastf log` and `fastf messages` — what fastf did, and what it said.
//!
//! The log is every event, one line each; messages are the sentences a person
//! was shown. Both are read from the data directory, so a move started in the
//! app is in the log a terminal reads, and yesterday's session is in both.

use anyhow::Result;
use colored::Colorize;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::Duration;

use crate::util::messages::Level;

/// `fastf log [-n N] [--follow]`.
pub fn log(lines: usize, follow: bool) -> Result<()> {
    let Some(path) = crate::util::log::path() else {
        anyhow::bail!("there is no data directory, so there is no log");
    };
    let events = crate::util::log::tail(&path, lines);
    if events.is_empty() && !follow {
        println!(
            "The log is empty: nothing has been written to {} yet.",
            crate::util::paths::display_path(&path)
        );
        return Ok(());
    }
    for event in &events {
        println!("{event}");
    }
    if follow {
        follow_from_the_end(&path)?;
    }
    Ok(())
}

/// Print what is appended to `path` until Ctrl-C, starting over from the top
/// when the file is rotated under it (it shrinks). Whole lines only: a line
/// still being written waits for its newline.
fn follow_from_the_end(path: &std::path::Path) -> Result<()> {
    let mut offset = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let mut pending = String::new();
    while !crate::util::interrupt::is_set() {
        std::thread::sleep(Duration::from_millis(300));
        let Ok(mut file) = std::fs::File::open(path) else {
            continue;
        };
        let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        if len < offset {
            offset = 0;
            pending.clear();
        }
        if len == offset {
            continue;
        }
        let mut bytes = Vec::new();
        if file.seek(SeekFrom::Start(offset)).is_err() || file.read_to_end(&mut bytes).is_err() {
            continue;
        }
        offset += bytes.len() as u64;
        pending.push_str(&String::from_utf8_lossy(&bytes));
        if let Some(end) = pending.rfind('\n') {
            print!("{}", &pending[..=end]);
            let _ = std::io::stdout().flush();
            pending.drain(..=end);
        }
    }
    Ok(())
}

/// `fastf messages [-n N]`.
pub fn messages(lines: usize) -> Result<()> {
    let messages = crate::util::messages::last(lines);
    if messages.is_empty() {
        println!("No messages yet.");
        return Ok(());
    }
    for message in &messages {
        let mark = match message.level {
            Level::Good => "✓".green().to_string(),
            Level::Warn => "⚠".yellow().to_string(),
            Level::Error => "✗".red().to_string(),
            Level::Info => " ".to_string(),
        };
        let mut text = message.text.lines();
        println!(
            "{}  {mark} {}  {}",
            crate::util::time::local_readable(&message.at).dimmed(),
            text.next().unwrap_or(""),
            message.source.dimmed()
        );
        for more in text {
            println!("{:23}{more}", "");
        }
    }
    Ok(())
}

/// Keep a message the command line is about to print, for `fastf messages`
/// and the app's `L`.
pub(crate) fn keep(level: Level, text: impl Into<String>) {
    crate::util::messages::append(&crate::util::messages::Message::now(level, "cli", text));
}
