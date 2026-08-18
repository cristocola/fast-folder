//! A `dialoguer::Select`-shaped picker whose rows may change while it waits.
//!
//! `dialoguer` cannot do this, for two reasons that are both dead ends:
//! `Select::interact` blocks in `Term::read_key`, which has no timeout, and a
//! `Term` built over a read/write pair reports `is_term() == false`, which
//! `Select` refuses outright. So the guided projects browser — whose folder sizes
//! arrive from background threads seconds after the list is drawn — owns its key
//! loop here. Everything else in the TUI stays on `dialoguer`, and this picker
//! deliberately matches it key for key.
//!
//! The key is read on a throwaway thread and collected with a timeout, so the
//! *wait* is interruptible without the *read* having to be. That is safe rather
//! than lucky: `console` holds the terminal in raw mode for the whole pending
//! read while explicitly preserving output post-processing (`unix_term.rs`
//! restores `c_oflag` after `cfmakeraw`), so a repaint from this thread is
//! neither echoed nor staircased, and the reader never writes.
//!
//! Three rules the caller must keep, all load-bearing:
//! 1. **Items are single-line and ANSI-free.** A repaint takes its own block back
//!    by line count (`clear_last_lines`), so one soft-wrapped row desynchronises
//!    every later redraw — the same reason `cli::recent::clamp_label` exists.
//! 2. **Nothing else may write to the terminal while this runs.** On Windows
//!    `move_cursor_up` derives its target from the *live* console cursor
//!    position, so a stray `println!` from another thread corrupts the redraw.
//! 3. **`frame` returns the same number of items every time.** It is called
//!    before each render and on every tick, and must stay cheap.
//!
//! Failures come back as `dialoguer::Error`, which is what keeps
//! `tui::menu::is_fatal` classifying a broken prompt as fatal instead of
//! containing it into a loop that would immediately fail again.

use std::io;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use dialoguer::console::{Key, Term};
use dialoguer::theme::Theme;

/// Show a picker that repaints whenever `frame` reports different rows.
///
/// `frame` receives the current selection index — the browser uses it to put the
/// row the user is about to open at the front of its scan queue — and returns the
/// full item list, navigation rows included.
pub(crate) fn select_live<F>(
    prompt: &str,
    default: usize,
    theme: &dyn Theme,
    tick: Duration,
    mut frame: F,
) -> dialoguer::Result<usize>
where
    F: FnMut(usize) -> Vec<String>,
{
    let term = Term::stderr();
    if !term.is_term() {
        return Err(io::Error::new(io::ErrorKind::NotConnected, "not a terminal").into());
    }

    let labels = frame(default);
    if labels.is_empty() {
        return Err(io::Error::other("empty list of items given to `select_live`").into());
    }
    let sel = default.min(labels.len() - 1);

    term.hide_cursor()?;
    let mut live = Live {
        term,
        theme,
        prompt,
        labels,
        sel,
        offset: 0,
        drawn: 0,
    };
    let outcome = live.run(tick, &mut frame);
    live.finish();
    outcome
}

struct Live<'a> {
    term: Term,
    theme: &'a dyn Theme,
    prompt: &'a str,
    labels: Vec<String>,
    sel: usize,
    /// First visible item. Sticky, so the list does not slide while the selection
    /// is already on screen.
    offset: usize,
    /// Lines currently on screen, so a repaint knows what to take back.
    drawn: usize,
}

impl Live<'_> {
    fn run<F>(&mut self, tick: Duration, frame: &mut F) -> dialoguer::Result<usize>
    where
        F: FnMut(usize) -> Vec<String>,
    {
        self.render()?;
        loop {
            let key = self.read_key(tick, frame)?;
            let before = self.sel;
            match key {
                Key::ArrowDown | Key::Tab | Key::Char('j') => self.step(1),
                Key::ArrowUp | Key::BackTab | Key::Char('k') => self.step(-1),
                Key::Enter | Key::Char(' ') => return Ok(self.sel),
                // Everything else is ignored, exactly as `Select::interact` does
                // — Esc and 'q' included, since only the `_opt` variants quit.
                _ => {}
            }
            let next = frame(self.sel);
            if next != self.labels || self.sel != before {
                self.labels = next;
                self.render()?;
            }
        }
    }

    /// Wait for one key, repainting on the way whenever the rows change.
    fn read_key<F>(&mut self, tick: Duration, frame: &mut F) -> dialoguer::Result<Key>
    where
        F: FnMut(usize) -> Vec<String>,
    {
        let (tx, rx) = mpsc::channel();
        // One key per thread: by the time the value arrives the thread is done,
        // so nothing is left reading the terminal when this picker hands it back
        // to `dialoguer`. The single exception is the interrupt path below, which
        // ends the process anyway.
        std::thread::spawn(move || {
            let _ = tx.send(Term::stderr().read_key());
        });

        loop {
            match rx.recv_timeout(tick) {
                Ok(key) => return Ok(key?),
                Err(RecvTimeoutError::Timeout) => {
                    // A Ctrl-C typed at the list normally returns through the
                    // reader: raw mode delivers it as a byte, which `console`
                    // turns back into SIGINT. This catches one that landed in the
                    // gap between reads, where the tty still handles it itself.
                    if crate::util::interrupt::is_set() {
                        return Err(
                            io::Error::new(io::ErrorKind::Interrupted, "interrupted").into()
                        );
                    }
                    let next = frame(self.sel);
                    if next != self.labels {
                        self.labels = next;
                        self.render()?;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "terminal input closed",
                    )
                    .into());
                }
            }
        }
    }

    fn render(&mut self) -> io::Result<()> {
        if self.labels.is_empty() {
            return Err(io::Error::other("live select lost its items"));
        }
        self.sel = self.sel.min(self.labels.len() - 1);
        if self.drawn > 0 {
            self.term.clear_last_lines(self.drawn)?;
            self.drawn = 0;
        }

        let (rows, _columns) = self.term.size();
        let capacity = viewport_capacity(self.labels.len(), rows as usize);
        self.offset = viewport_offset(self.offset, self.sel, self.labels.len(), capacity);

        let mut prompt = self.prompt.to_string();
        if let Some(hint) = viewport_hint(self.offset, capacity, self.labels.len()) {
            prompt.push_str(&hint);
        }

        // Formatted up front so the whole block is written in one pass, with no
        // window in which another thread could see a half-drawn list.
        let mut lines = Vec::with_capacity(capacity + 1);
        let mut line = String::new();
        self.theme
            .format_select_prompt(&mut line, &prompt)
            .map_err(io::Error::other)?;
        lines.push(line);
        for (idx, item) in self
            .labels
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(capacity)
        {
            let mut line = String::new();
            self.theme
                .format_select_prompt_item(&mut line, item, idx == self.sel)
                .map_err(io::Error::other)?;
            lines.push(line);
        }

        for line in &lines {
            self.term.write_line(line)?;
        }
        self.drawn = lines.len();
        self.term.flush()
    }

    fn step(&mut self, delta: isize) {
        let len = self.labels.len() as isize;
        if len <= 0 {
            return;
        }
        // Wrapping in both directions, as `Select` does.
        self.sel = (self.sel as isize + delta).rem_euclid(len) as usize;
    }

    /// Take the block back and restore the cursor, on every exit path.
    ///
    /// `dialoguer` balances hide/show only on its success path, which is how a
    /// failed prompt used to leave a shell with no cursor. Only reachable on a
    /// real terminal (`select_live` refuses anything else), so `show_cursor`
    /// cannot leak an escape into a pipe.
    fn finish(&mut self) {
        if self.drawn > 0 {
            let _ = self.term.clear_last_lines(self.drawn);
            self.drawn = 0;
        }
        let _ = self.term.show_cursor();
        let _ = self.term.flush();
    }
}

/// Item rows the terminal can hold: everything but the prompt line and one row of
/// headroom. The headroom matters — a block that exactly fills the screen scrolls
/// as it is written, and a scrolled block is one `clear_last_lines` cannot take
/// back.
fn viewport_capacity(items: usize, terminal_rows: usize) -> usize {
    items.min(terminal_rows.saturating_sub(2)).max(1)
}

/// Where the visible window starts. Sticky: it moves only when the selection
/// would otherwise be off screen, so arrowing inside the window does not make the
/// whole list slide.
fn viewport_offset(previous: usize, sel: usize, items: usize, capacity: usize) -> usize {
    let max = items.saturating_sub(capacity);
    let mut offset = previous.min(max);
    if sel < offset {
        offset = sel;
    } else if sel >= offset + capacity {
        offset = sel + 1 - capacity;
    }
    offset.min(max)
}

/// Says there is more than what fits, replacing `dialoguer`'s `[Page x/y]` inner
/// paging (which this picker does not use — the browser is already paged, and a
/// second, hidden pagination inside one page reads as a bug).
fn viewport_hint(offset: usize, capacity: usize, items: usize) -> Option<String> {
    if capacity >= items {
        return None;
    }
    Some(format!(
        "  (rows {}–{} of {})",
        offset + 1,
        (offset + capacity).min(items),
        items
    ))
}

#[cfg(test)]
mod tests {
    use super::{viewport_capacity, viewport_hint, viewport_offset};

    #[test]
    fn capacity_leaves_room_for_the_prompt_and_headroom() {
        assert_eq!(viewport_capacity(5, 24), 5, "a short list fits whole");
        assert_eq!(
            viewport_capacity(30, 24),
            22,
            "prompt plus one row reserved"
        );
        assert_eq!(viewport_capacity(30, 3), 1);
        // A terminal that reports nothing must still leave one usable row.
        assert_eq!(viewport_capacity(30, 0), 1);
        assert_eq!(viewport_capacity(1, 24), 1);
    }

    #[test]
    fn the_window_does_not_move_while_the_selection_is_visible() {
        // Window [0,10) — moving between rows 0 and 9 must not scroll.
        assert_eq!(viewport_offset(0, 0, 30, 10), 0);
        assert_eq!(viewport_offset(0, 9, 30, 10), 0);
        // Row 10 is one past the window, so it scrolls by exactly one.
        assert_eq!(viewport_offset(0, 10, 30, 10), 1);
        // Coming back up, likewise.
        assert_eq!(viewport_offset(5, 4, 30, 10), 4);
        assert_eq!(viewport_offset(5, 5, 30, 10), 5);
    }

    #[test]
    fn the_window_clamps_to_the_ends_including_a_wrap() {
        // Wrapping from the first row to the last jumps the window to the end.
        assert_eq!(viewport_offset(0, 29, 30, 10), 20);
        // And back to the top.
        assert_eq!(viewport_offset(20, 0, 30, 10), 0);
        // A list that shrank under a stale offset must not leave a gap.
        assert_eq!(viewport_offset(20, 2, 5, 10), 0);
        assert_eq!(viewport_offset(0, 0, 1, 1), 0);
    }

    #[test]
    fn the_hint_appears_only_when_rows_are_hidden() {
        assert_eq!(viewport_hint(0, 10, 10), None);
        assert_eq!(viewport_hint(0, 22, 10), None);
        assert_eq!(
            viewport_hint(0, 10, 30).unwrap(),
            "  (rows 1–10 of 30)".to_string()
        );
        assert_eq!(
            viewport_hint(20, 10, 30).unwrap(),
            "  (rows 21–30 of 30)".to_string()
        );
    }
}
