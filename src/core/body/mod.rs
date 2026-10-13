//! The markdown body of `PROJECT_INFO.md`: its sections, the notes, the todos.
//!
//! Below the frontmatter the file is the user's own, and fastf reads and
//! writes exactly three things in it, each under a `##` heading it finds by
//! one rule: the **notes** — dated entries, one `- <timestamp> — <text>` line
//! and as many indented lines under it as the note has — the **todos** —
//! `- [ ] text` and `- [x] text` — and, in a file written before v3.6.0, a
//! `## Journal` section that held the notes then and keeps holding them now.
//!
//! **One grammar, read by the writer and the reader alike.** `section_span`
//! is where any section is, for every writer and reader here; `notes_span`
//! is where `append_journal_entry` puts a note, always inside a section
//! `notes_in` reads. With a definition each, a note the writer puts past the
//! point the reader stops at is written, confirmed, and never seen again.
//!
//! **The reader never fails and never drops a line it could show.** A
//! heading is matched wherever it starts a line, in any case, with or without
//! a trailing colon; a line under the notes heading that does not start an
//! entry belongs to the entry above it (or, before the first entry, to one
//! undated note); a line under the todo heading that is not a task is simply
//! not a task — except a `###` line, which labels the **phase** every task
//! below it belongs to, until the next one. A file somebody hand-edited into a
//! shape fastf never wrote is shown as far as it can be read, which is as far
//! as it goes.
//!
//! **The writer changes the bytes it is about and no others.** An append
//! lands at the end of its section; an edit splices over the note's own lines;
//! a toggle rewrites the one character inside the brackets; a reworded todo
//! keeps everything up to and including its brackets, and its line ending; a
//! removal takes the item's own lines, and a blank line only where it would
//! otherwise be left doubled. A single-line note writes the bytes every
//! earlier version wrote, so a diff over an old file shows only the line that
//! was added.

use std::fs;
use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::core::project_info::split_frontmatter_body;

/// The `## Notes` heading, spelled once: what a new section is opened with.
pub const NOTES_HEADING: &str = "## Notes";

/// The `## Todo` heading, spelled once: what a new section is opened with.
pub const TODO_HEADING: &str = "## Todo";

/// One note: when it was written, and what it says — which may be several
/// lines. `timestamp` is `None` for the one undated note a section can hold:
/// free text above the first entry, which is what the `## Notes` section of
/// a file written before v3.6.0 was for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub timestamp: Option<String>,
    pub text: String,
}

impl Note {
    /// The day the note was written, for a column ten wide: the first ten
    /// characters of its timestamp — by `get`, never a byte slice, because a
    /// hand-edited file can put anything there — or `None` for an undated
    /// note.
    pub fn day(&self) -> Option<&str> {
        self.timestamp
            .as_deref()
            .map(|ts| ts.get(..10).unwrap_or(ts))
    }

    /// Whether the timestamp is a date fastf can order: it begins
    /// `YYYY-MM-DD`. The reader takes any word before a ` — ` as a timestamp,
    /// so a hand-written `- Remember — this` has one, and sorted as text it
    /// would sit above every real date; the journal and "the last note" keep
    /// to the dated ones.
    pub fn is_dated(&self) -> bool {
        self.timestamp.as_deref().is_some_and(starts_with_a_day)
    }
}

/// One task under `## Todo`.
///
/// The text is the task alone: what the line says after its brackets and
/// before its **trailer**, the run of `@owner`, `added:<stamp>` and
/// `done:<stamp>` tokens at its end (`split_trailer`). A line a hand wrote
/// without one is a todo with no owner and no dates, and nothing here
/// invents them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Todo {
    pub done: bool,
    pub text: String,
    /// The `###` label above it, if the list is grouped into phases — the
    /// nearest one, so a task before the first label has none. It is read,
    /// never written by a toggle: a phase is a line of the user's own file.
    pub phase: Option<String>,
    /// Whose court it is in: the `@owner` at the end of its line. `None`
    /// for a todo nobody was named for.
    pub owner: Option<String>,
    /// When it was added, as the `added:` token says: the stamp notes
    /// carry, or a day, whichever was written. `None` without one.
    pub added: Option<String>,
    /// When it was ticked, as the `done:` token says. A tick writes it and
    /// an untick removes it; a hand may leave either.
    pub done_at: Option<String>,
}

impl Todo {
    /// The day the todo was added, for a column ten wide — by `get`, never
    /// a byte slice, since a hand can put anything after `added:`.
    pub fn added_day(&self) -> Option<&str> {
        self.added.as_deref().map(day_of)
    }

    /// The day the todo was ticked, the same way.
    pub fn done_day(&self) -> Option<&str> {
        self.done_at.as_deref().map(day_of)
    }
}

/// The first ten characters of a stamp — the day — or the stamp whole when
/// it is shorter.
fn day_of(stamp: &str) -> &str {
    stamp.get(..10).unwrap_or(stamp)
}

/// What a todo line's trailer says. Each token is optional and a hand may
/// write them in any order; `render_trailer` writes them in one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Trailer {
    owner: Option<String>,
    added: Option<String>,
    done: Option<String>,
}

/// A trailer token, by shape: `@word`, `added:<stamp>`, `done:<stamp>`. A
/// stamp is anything that starts with a day, so `added:2026-10-12` written
/// by hand reads as well as the full stamp fastf writes; the prefixes are
/// read in any case. Anything else is text, which is what ends the trailer.
fn trailer_token(token: &str) -> Option<TrailerToken<'_>> {
    if let Some(name) = token.strip_prefix('@') {
        let name = name.trim_end_matches([',', '.', ';', ':', '!', '?']);
        return (!name.is_empty()).then_some(TrailerToken::Owner(name));
    }
    let keyed = |key: &str| -> Option<&str> {
        let (head, rest) = token.split_at_checked(key.len())?;
        (head.eq_ignore_ascii_case(key) && starts_with_a_day(rest)).then_some(rest)
    };
    if let Some(stamp) = keyed("added:") {
        return Some(TrailerToken::Added(stamp));
    }
    if let Some(stamp) = keyed("done:") {
        return Some(TrailerToken::Done(stamp));
    }
    None
}

enum TrailerToken<'a> {
    Owner(&'a str),
    Added(&'a str),
    Done(&'a str),
}

/// Where `after` — a task line's text after its `]` — ends and its trailer
/// begins: the byte where the text's last word ends (the text is
/// `after[..text_end].trim()`), the byte where the trailer's first token
/// starts — `None` when the line has no trailer — and what the trailer
/// says. **The trailer is the maximal run of trailer tokens at the end of
/// the line**, so a `@word` in the middle of the text is text, and the
/// first owner of the run is the owner.
fn split_trailer(after: &str) -> (usize, Option<usize>, Trailer) {
    // Every whitespace-separated word, with where it sits.
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (at, c) in after.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(at),
            (true, Some(from)) => {
                words.push((from, at));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        words.push((from, after.len()));
    }
    let mut trailer = Trailer::default();
    let mut first = words.len();
    for (index, (from, to)) in words.iter().enumerate().rev() {
        match trailer_token(&after[*from..*to]) {
            // The leftmost of each wins: it is the first a reader meets.
            Some(TrailerToken::Owner(name)) => trailer.owner = Some(name.to_string()),
            Some(TrailerToken::Added(stamp)) => trailer.added = Some(stamp.to_string()),
            Some(TrailerToken::Done(stamp)) => trailer.done = Some(stamp.to_string()),
            None => break,
        }
        first = index;
    }
    let text_end = match first {
        0 => 0,
        n => words[n - 1].1,
    };
    let trailer_start = words.get(first).map(|(from, _)| *from);
    (text_end, trailer_start, trailer)
}

/// The trailer as fastf writes it: `@owner added:<stamp> done:<stamp>`, each
/// only when there is one, one space apart — empty with none. **The one
/// writer of the order**, so a line reassigned, ticked and unticked comes
/// out as it went in.
fn render_trailer(owner: Option<&str>, added: Option<&str>, done: Option<&str>) -> String {
    let mut out = String::new();
    let mut push = |token: String| {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&token);
    };
    if let Some(owner) = owner {
        push(format!("@{owner}"));
    }
    if let Some(added) = added {
        push(format!("added:{added}"));
    }
    if let Some(done) = done {
        push(format!("done:{done}"));
    }
    out
}

/// A todo as somebody typed it: the text, and the owner and the added stamp
/// a trailer in it names. `done:` typed into a new todo is dropped, since a
/// todo is added open; a text that is nothing but a trailer is empty, and
/// skipped as an empty one is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewTodo {
    pub text: String,
    pub owner: Option<String>,
    pub added: Option<String>,
}

impl NewTodo {
    /// Read `typed` by the trailer's own grammar: `cut the first minute
    /// @claude` is the text and its owner, as the reader will read it back.
    pub fn typed(typed: &str) -> Self {
        let (text_end, _, trailer) = split_trailer(typed);
        Self {
            text: typed[..text_end].trim().to_string(),
            owner: trailer.owner,
            added: trailer.added,
        }
    }

    /// [`typed`](Self::typed), with `owner` over whatever the text named.
    pub fn owned(typed: &str, owner: Option<&str>) -> Self {
        let mut new = Self::typed(typed);
        if let Some(owner) = owner {
            new.owner = Some(owner.to_string());
        }
        new
    }
}

/// A section fastf knows how to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Notes,
    Journal,
    Todo,
}

impl Section {
    /// Whether a heading's text — trimmed, lower-cased, colon stripped — is
    /// this section. Lenient on purpose: the file is hand-edited.
    fn accepts(self, name: &str) -> bool {
        let names: &[&str] = match self {
            Section::Notes => &["notes"],
            Section::Journal => &["journal"],
            Section::Todo => &["todo", "todos", "to do", "to-do", "tasks"],
        };
        names.iter().any(|n| n.eq_ignore_ascii_case(name))
    }
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/// The text of a `##` heading line, or `None` when the line is not one. A
/// heading starts a line with exactly two `#` — `###` is a sub-heading and
/// neither starts nor ends a section — and its name is what follows, trimmed,
/// without a trailing colon.
fn heading_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("##")?;
    if rest.starts_with('#') {
        return None;
    }
    let name = rest.trim().trim_end_matches(':').trim();
    Some(name)
}

/// The phase a `###` line names, if it names one. Three hashes or more, so a
/// deeper label groups the tasks under it the same way; an empty label is not
/// a phase and leaves the one above it standing. `##` is a section and never
/// reaches here, because a section ends where the next one starts.
fn phase_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("###")?;
    let name = rest
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .trim();
    (!name.is_empty()).then_some(name)
}

/// What `text` names as a phase — the label a writer puts after `### ` so
/// that the reader reads back exactly that: `"## Grade:"` is `Grade`.
/// Every hash and space is taken off the front and every colon and space
/// off the end, all at once, since a name that still ended in `:` would be
/// written as one label and read as another, and each later todo would open
/// it again. `None` when it names nothing; a line break is not a phase either.
pub fn phase_label(text: &str) -> Option<String> {
    if text.contains(['\n', '\r']) {
        return None;
    }
    let name = text
        .trim_start_matches(|c: char| c == '#' || c.is_whitespace())
        .trim_end_matches(|c: char| c == ':' || c.is_whitespace());
    (!name.is_empty()).then(|| name.to_string())
}

/// One `###` label of the todo list: its name, and how many tasks sit above
/// it — so a label with nothing under it, which no task's `phase` names, is
/// still somewhere a reader can draw and a writer will find.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseLabel {
    pub name: String,
    pub before: usize,
}

/// Every `###` label in `content`'s todo list, in file order, empty ones
/// included.
pub fn phase_labels_in(content: &str) -> Vec<PhaseLabel> {
    let Some(span) = section_span(content, Section::Todo) else {
        return Vec::new();
    };
    let mut labels = Vec::new();
    let mut tasks = 0;
    for (_, line) in section_lines(content, &span) {
        if let Some(name) = phase_name(line) {
            labels.push(PhaseLabel {
                name: name.to_string(),
                before: tasks,
            });
        } else if task_line(line).is_some() {
            tasks += 1;
        }
    }
    labels
}

/// Whether `line` starts a section — any `##` heading, whatever it says.
fn is_heading(line: &str) -> bool {
    heading_name(line).is_some()
}

/// Where `section` is in the file: the byte range from its heading line to
/// the `\n` before the next `##` line, or to the end. **The one rule for every
/// section fastf reads or writes.** Offsets are into the whole file, so a
/// writer can splice with them; the search itself runs over the body, so a
/// frontmatter value can never be taken for a heading.
///
/// `None` when the heading is not there. The end is always a `char` boundary:
/// the length, or the offset of a `\n`.
pub(crate) fn section_span(content: &str, section: Section) -> Option<Range<usize>> {
    let body = split_frontmatter_body(content)
        .map(|(_, body)| body)
        .unwrap_or(content);
    // The body is a suffix of the content — with or without a BOM in front.
    let base = content.len() - body.len();

    let mut start = None;
    let mut at = 0;
    for line in body.split_inclusive('\n') {
        let here = at;
        at += line.len();
        let text = line.trim_end_matches(['\n', '\r']);
        let Some(name) = heading_name(text) else {
            continue;
        };
        match start {
            None if section.accepts(name) => start = Some(here),
            None => {}
            // The `\n` that ends the line before this heading.
            Some(from) => return Some(base + from..base + here.saturating_sub(1)),
        }
    }
    start.map(|from| base + from..content.len())
}

/// Where a new note goes: the `## Journal` section if the file has one — a
/// file written before v3.6.0 keeps its shape, byte for byte — else the
/// `## Notes` section. `notes_in` reads both, so what is written here is
/// read back.
fn notes_span(content: &str) -> Option<Range<usize>> {
    section_span(content, Section::Journal).or_else(|| section_span(content, Section::Notes))
}

/// The lines of a section after its heading line, each with its range in the
/// file. The line text is without its line ending.
fn section_lines<'a>(content: &'a str, span: &Range<usize>) -> Vec<(Range<usize>, &'a str)> {
    let section = &content[span.clone()];
    let mut lines = Vec::new();
    let mut at = span.start;
    for (index, line) in section.split_inclusive('\n').enumerate() {
        let range = at..at + line.len();
        at += line.len();
        if index == 0 {
            continue; // the heading
        }
        lines.push((range, line.trim_end_matches(['\n', '\r'])));
    }
    lines
}

/// Whether the ten characters at the front of `token` spell `YYYY-MM-DD`.
fn starts_with_a_day(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// What starts a dated entry: a column-0 list item whose rest is
/// `<timestamp> — <text>` — the bytes fastf writes — or whose first word is
/// a date, with or without a separator after it. `None` for any other line.
fn entry_start(line: &str) -> Option<(String, String)> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))?;
    if let Some((ts, text)) = rest.split_once(" — ") {
        let ts = ts.trim();
        if !ts.is_empty() {
            return Some((ts.to_string(), text.trim().to_string()));
        }
    }
    let token = rest.split_whitespace().next()?;
    let timestamp = token.trim_end_matches(':');
    if !starts_with_a_day(timestamp) {
        return None;
    }
    let after = rest[rest.find(token).unwrap_or(0) + token.len()..].trim_start();
    let text = after
        .strip_prefix('—')
        .or_else(|| after.strip_prefix('-'))
        .or_else(|| after.strip_prefix(':'))
        .unwrap_or(after)
        .trim();
    Some((timestamp.to_string(), text.to_string()))
}

/// A continuation line as the note holds it: the writer's two-space indent
/// taken back off, a tab likewise, anything deeper kept.
fn continuation(line: &str) -> &str {
    line.strip_prefix("  ")
        .or_else(|| line.strip_prefix('\t'))
        .or_else(|| line.strip_prefix(' '))
        .unwrap_or(line)
        .trim_end()
}

/// A note as it sits in the file: the note, and the range of its lines —
/// from the start of its first line to the end of its last non-blank one,
/// that line's `\n` included.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Placed {
    note: Note,
    span: Range<usize>,
}

/// Every note in `content`, in file order: the `## Notes` section's, then
/// the `## Journal` section's. The dated ones, and before them the undated
/// one, if the section has text above its first entry.
fn place_notes(content: &str) -> Vec<Placed> {
    let mut placed = Vec::new();
    for section in [Section::Notes, Section::Journal] {
        let Some(span) = section_span(content, section) else {
            continue;
        };
        placed.extend(notes_in_section(content, &span));
    }
    placed
}

fn notes_in_section(content: &str, span: &Range<usize>) -> Vec<Placed> {
    // The note being built: its timestamp, its lines so far, where its first
    // line starts and where its last non-blank line ends.
    struct Building {
        timestamp: Option<String>,
        lines: Vec<String>,
        start: usize,
        end: usize,
    }
    fn finish(building: Building, out: &mut Vec<Placed>) {
        let mut lines = building.lines;
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        if lines.is_empty() {
            return;
        }
        out.push(Placed {
            note: Note {
                timestamp: building.timestamp,
                text: lines.join("\n"),
            },
            span: building.start..building.end,
        });
    }

    let mut out = Vec::new();
    let mut current: Option<Building> = None;
    for (range, line) in section_lines(content, span) {
        if let Some((timestamp, text)) = entry_start(line) {
            if let Some(done) = current.take() {
                finish(done, &mut out);
            }
            current = Some(Building {
                timestamp: Some(timestamp),
                lines: vec![text],
                start: range.start,
                end: range.end,
            });
            continue;
        }
        match &mut current {
            Some(building) => {
                building.lines.push(continuation(line).to_string());
                if !line.trim().is_empty() {
                    building.end = range.end;
                }
            }
            None => {
                // Text above the first entry: the undated note. Its framing
                // blank lines are not part of it.
                if line.trim().is_empty() {
                    continue;
                }
                current = Some(Building {
                    timestamp: None,
                    lines: vec![line.trim_end().to_string()],
                    start: range.start,
                    end: range.end,
                });
            }
        }
    }
    if let Some(done) = current.take() {
        finish(done, &mut out);
    }
    out
}

/// Every note in `content`, in file order — see `place_notes`.
pub fn notes_in(content: &str) -> Vec<Note> {
    place_notes(content)
        .into_iter()
        .map(|placed| placed.note)
        .collect()
}

/// Every note of the project at `project_root`, oldest first. Empty when
/// there is no notes section at all.
pub fn read_journal_entries(project_root: &Path) -> Result<Vec<Note>> {
    let content = crate::core::project_info::read(project_root)?;
    Ok(notes_in(&content))
}

/// The lines a dated note is written as: `- <ts> — <first line>`, then each
/// further line under two spaces so that nothing in it can start an entry
/// or end the section. A blank line stays blank. One line writes the bytes
/// every earlier version wrote.
fn render_entry(timestamp: &str, text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim();
    let mut out = format!("- {timestamp} — {first}\n");
    for line in lines {
        let line = line.trim_end();
        if !line.is_empty() {
            out.push_str("  ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// The lines the undated note is written as: as typed, each ended. A line
/// beginning `##` would end the section there, so it is refused, by name.
fn render_preamble(text: &str) -> Result<String> {
    if let Some(line) = text.lines().find(|line| is_heading(line.trim_start())) {
        bail!(
            "a line beginning with ## would end the notes section there (\"{}\") — use a single # or plain text",
            line.trim()
        );
    }
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    Ok(out)
}

/// Read the file and require the frontmatter — every writer here does, since
/// this is a structured project file.
fn read_document(path: &Path, verb: &str) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading {}", crate::util::paths::display_path(path)))?;
    if split_frontmatter_body(&content).is_none() {
        bail!(
            "{} has no YAML frontmatter — cannot {verb}",
            crate::util::paths::display_path(path)
        );
    }
    Ok(content)
}

/// `content` with `[span]` replaced by `with`.
fn splice(content: &str, span: Range<usize>, with: &str) -> String {
    let mut out = String::with_capacity(content.len() + with.len());
    out.push_str(&content[..span.start]);
    out.push_str(with);
    out.push_str(&content[span.end..]);
    out
}

/// Where the next line appended to a section lands, and what has to come
/// before it: the end of the section, with `\n\n` before the new line when
/// the section holds no item yet (only its heading, or only prose), so the
/// shape stays `## Notes`, a blank line, the list — and a single `\n` when a
/// list is already there to continue.
fn append_in_section(
    content: &str,
    span: &Range<usize>,
    has_items: bool,
    line: &str,
) -> (String, usize) {
    let mut out = String::with_capacity(content.len() + line.len() + 2);
    out.push_str(&content[..span.end]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !has_items && !out.ends_with("\n\n") {
        out.push('\n');
    }
    let at = out.len();
    out.push_str(line);
    out.push_str(&content[span.end..]);
    (out, at)
}

/// A new section at the end of the file: a blank line, the heading, a blank
/// line, the first line.
fn open_section(content: &str, heading: &str, line: &str) -> (String, usize) {
    let mut out = String::with_capacity(content.len() + heading.len() + line.len() + 4);
    out.push_str(content);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(heading);
    out.push_str("\n\n");
    let at = out.len();
    out.push_str(line);
    (out, at)
}

// ---------------------------------------------------------------------------
// Notes: append, edit
// ---------------------------------------------------------------------------

/// Append a note, dated now, to the notes section — at the end of **the
/// section**, wherever that is, which is what the reader reads back. A file
/// with no notes section (and no journal) gets a `## Notes` at its end.
///
/// The write is atomic: unique temp file + rename.
pub fn append_journal_entry(path: &Path, message: &str) -> Result<()> {
    let content = read_document(path, "append a note")?;
    let entry = render_entry(&crate::util::time::now_iso8601(), message.trim());
    let new_content = with_line_endings_of(&content, |content| match notes_span(content) {
        Some(span) => {
            let dated = notes_in_section(content, &span)
                .iter()
                .any(|placed| placed.note.timestamp.is_some());
            append_in_section(content, &span, dated, &entry).0
        }
        None => open_section(content, NOTES_HEADING, &entry).0,
    });
    crate::util::atomic::write(path, new_content.as_bytes())
}

/// Rewrite note `ordinal` — its index in [`notes_in`]'s order — as `text`,
/// or remove it when `text` is empty. Touches the note's own lines and no
/// others.
///
/// `expected` is the text the caller last read: a note whose text has moved
/// on since is refused rather than overwritten, because the ordinal alone
/// cannot tell an edit of *this* note from an edit of whatever now sits
/// where it was.
pub fn replace_note(path: &Path, ordinal: usize, expected: &str, text: &str) -> Result<()> {
    let content = read_document(path, "edit a note")?;
    let placed = place_notes(&content);
    let Some(current) = placed.get(ordinal) else {
        bail!("the note is no longer there — reload and edit it again");
    };
    if current.note.text != expected {
        bail!("the note changed meanwhile — reload and edit it again");
    }
    let text = text.trim();
    let rendered = if text.is_empty() {
        None
    } else {
        Some(match &current.note.timestamp {
            Some(timestamp) => render_entry(timestamp, text),
            None => render_preamble(text)?,
        })
    };
    // The note is found again in the text the edit is made on — the file's
    // own, or its `\n` reading when it was saved with `\r\n` — where line
    // endings do not change which note is which.
    let new_content = with_line_endings_of(&content, |content| {
        let Some(span) = place_notes(content).get(ordinal).map(|p| p.span.clone()) else {
            return content.to_string();
        };
        match &rendered {
            None => remove_lines(content, span),
            Some(rendered) => splice(content, span, rendered),
        }
    });
    crate::util::atomic::write(path, new_content.as_bytes())
}

/// `content` without the lines in `span`, and without the blank line that
/// removing them would leave doubled — in either line ending, since a file
/// saved on Windows has its blank lines as `\r\n`.
fn remove_lines(content: &str, span: Range<usize>) -> String {
    let mut out = splice(content, span.clone(), "");
    let at = span.start;
    let blank_above = out[..at].ends_with("\n\n") || out[..at].ends_with("\n\r\n");
    let blank_below = ["\n", "\r\n"]
        .into_iter()
        .find(|ending| out[at..].starts_with(ending));
    if let (true, Some(ending)) = (blank_above, blank_below) {
        out.replace_range(at..at + ending.len(), "");
    }
    out
}

// ---------------------------------------------------------------------------
// Todos
// ---------------------------------------------------------------------------

/// A task as it sits in the file: the task; its whole line, the line ending
/// included when there is one, which is what a removal takes; its text, from
/// just after the `]` to the end of its last word, which is what a rewording
/// replaces; its tail, from there to before the line ending — the space and
/// the trailer, or nothing — which is what a tick, an untick and a new owner
/// rewrite; and the character inside its brackets (an empty range for `[]`),
/// which is what a toggle rewrites.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlacedTodo {
    todo: Todo,
    line: Range<usize>,
    text: Range<usize>,
    tail: Range<usize>,
    marker: Range<usize>,
}

impl PlacedTodo {
    /// The tail a line with this trailer gets: a space and the tokens, or
    /// nothing at all, so a line with nothing to say ends at its last word.
    fn tail_for(owner: Option<&str>, added: Option<&str>, done: Option<&str>) -> String {
        let rendered = render_trailer(owner, added, done);
        if rendered.is_empty() {
            rendered
        } else {
            format!(" {rendered}")
        }
    }
}

/// What a task line is: a list item, any indent, then `[ ]`, `[x]`, `[X]` or
/// `[]`, then the text and its trailer, handed over untrimmed. Anything else
/// under the heading is not a task.
fn task_line(line: &str) -> Option<(bool, usize, usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let rest = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))?;
    let at = indent + (line.len() - indent - rest.len());
    let rest = rest.strip_prefix('[')?;
    let (done, inside) = match rest.as_bytes().first() {
        Some(b']') => (false, 0),
        Some(b' ') if rest.as_bytes().get(1) == Some(&b']') => (false, 1),
        Some(b'x' | b'X') if rest.as_bytes().get(1) == Some(&b']') => (true, 1),
        _ => return None,
    };
    let marker_at = at + 1;
    let after = &rest[inside + 1..];
    Some((done, marker_at, inside, after))
}

fn place_todos(content: &str) -> Vec<PlacedTodo> {
    let Some(span) = section_span(content, Section::Todo) else {
        return Vec::new();
    };
    let mut phase: Option<String> = None;
    let mut placed = Vec::new();
    for (range, line) in section_lines(content, &span) {
        if let Some(name) = phase_name(line) {
            phase = Some(name.to_string());
            continue;
        }
        let Some((done, marker_at, inside, after)) = task_line(line) else {
            continue;
        };
        // A section stops short of the `\n` before the next heading, so its
        // last line is handed over without one; the line is still the line.
        let mut whole = range.clone();
        if content[whole.end..].starts_with('\n') && !content[whole.clone()].ends_with('\n') {
            whole.end += 1;
        }
        let text_at = range.start + marker_at + inside + 1;
        let (text_end, trailer_start, trailer) = split_trailer(after);
        let line_end = range.start + line.len();
        // With a trailer the text stops at its last word and the tail is
        // the rest; without one the text runs to the line's end, as it
        // always did, so a rewording takes the trailing blanks with it and
        // a tick lands after them.
        let (text, tail) = if trailer_start.is_some() {
            (text_at..text_at + text_end, text_at + text_end..line_end)
        } else {
            (text_at..line_end, line_end..line_end)
        };
        placed.push(PlacedTodo {
            todo: Todo {
                done,
                text: after[..text_end].trim().to_string(),
                phase: phase.clone(),
                owner: trailer.owner,
                added: trailer.added,
                done_at: trailer.done,
            },
            line: whole,
            text,
            tail,
            marker: range.start + marker_at..range.start + marker_at + inside,
        });
    }
    placed
}

/// Every task in `content`, in file order.
pub fn todos_in(content: &str) -> Vec<Todo> {
    place_todos(content)
        .into_iter()
        .map(|placed| placed.todo)
        .collect()
}

/// Every task of the project at `project_root`, in file order.
pub fn read_todos(project_root: &Path) -> Result<Vec<Todo>> {
    let content = crate::core::project_info::read(project_root)?;
    Ok(todos_in(&content))
}

/// Every task of the project at `project_root` and every phase label, from
/// one read, so the two agree.
pub fn read_todo_list(project_root: &Path) -> Result<(Vec<Todo>, Vec<PhaseLabel>)> {
    let content = crate::core::project_info::read(project_root)?;
    Ok((todos_in(&content), phase_labels_in(&content)))
}

/// Flip task `ordinal` between open and done: the one character inside its
/// brackets, and its tail — a tick writes `done:<now>` there, an untick
/// takes the `done:` token away, and the owner and `added:` stay as they
/// were. One write. Returns whether it is done now. `expected` is the text
/// the caller last read — see [`replace_note`].
pub fn toggle_todo(path: &Path, ordinal: usize, expected: &str) -> Result<bool> {
    let content = read_document(path, "toggle a todo")?;
    let placed = place_todos(&content);
    let Some(current) = placed.get(ordinal) else {
        bail!("the todo is no longer there — reload and try again");
    };
    if current.todo.text != expected {
        bail!("the todo changed meanwhile — reload and try again");
    }
    let done = !current.todo.done;
    let now = crate::util::time::now_iso8601();
    let tail = PlacedTodo::tail_for(
        current.todo.owner.as_deref(),
        current.todo.added.as_deref(),
        done.then_some(now.as_str()),
    );
    // The tail sits after the marker, so splicing it first leaves the
    // marker's offsets where they were.
    let new_content = splice(&content, current.tail.clone(), &tail);
    let new_content = splice(
        &new_content,
        current.marker.clone(),
        if done { "x" } else { " " },
    );
    crate::util::atomic::write(path, new_content.as_bytes())?;
    Ok(done)
}

/// Reword task `ordinal` — its index in [`todos_in`]'s order — as `text`, or
/// remove its line when `text` is empty. `expected` is the text the caller
/// last read — see [`replace_note`].
///
/// A rewording replaces the text after the brackets and nothing else: the
/// indent, the list marker, what is inside the brackets, the trailer and the
/// line ending (a `\r\n` included) stay as they were. A text that ends in an
/// owner of its own — `… @sam` — names the new owner, and the trailer is
/// written again with it in the trailer's own place. A removal takes the
/// whole line, and the blank line it would leave doubled. A `###` phase
/// label is never touched, even when the last task under it goes.
pub fn replace_todo(path: &Path, ordinal: usize, expected: &str, text: &str) -> Result<()> {
    if text.contains(['\n', '\r']) {
        bail!("a todo is one line");
    }
    let content = read_document(path, "edit a todo")?;
    let placed = place_todos(&content);
    let Some(current) = placed.get(ordinal) else {
        bail!("the todo is no longer there — reload and try again");
    };
    if current.todo.text != expected {
        bail!("the todo changed meanwhile — reload and try again");
    }
    let typed = NewTodo::typed(text);
    let new_content = if typed.text.is_empty() && typed.owner.is_none() {
        remove_lines(&content, current.line.clone())
    } else if typed.text.is_empty() {
        bail!("a todo is more than its owner — give it some text");
    } else if let Some(owner) = &typed.owner {
        let tail = PlacedTodo::tail_for(
            Some(owner),
            current.todo.added.as_deref(),
            current.todo.done_at.as_deref(),
        );
        splice(
            &content,
            current.text.start..current.tail.end,
            &format!(" {}{tail}", typed.text),
        )
    } else {
        splice(&content, current.text.clone(), &format!(" {}", typed.text))
    };
    crate::util::atomic::write(path, new_content.as_bytes())
}

/// Give task `ordinal` to `owner`, or to nobody with `None`: its tail is
/// written again with the new `@owner` and the dates it had, and nothing
/// else on the line moves. `expected` is the text the caller last read —
/// see [`replace_note`].
pub fn set_owner(path: &Path, ordinal: usize, expected: &str, owner: Option<&str>) -> Result<()> {
    if owner.is_some_and(|owner| owner.is_empty() || owner.chars().any(char::is_whitespace)) {
        bail!("an owner is one word");
    }
    let content = read_document(path, "set a todo's owner")?;
    let placed = place_todos(&content);
    let Some(current) = placed.get(ordinal) else {
        bail!("the todo is no longer there — reload and try again");
    };
    if current.todo.text != expected {
        bail!("the todo changed meanwhile — reload and try again");
    }
    let tail = PlacedTodo::tail_for(
        owner,
        current.todo.added.as_deref(),
        current.todo.done_at.as_deref(),
    );
    let new_content = splice(&content, current.tail.clone(), &tail);
    crate::util::atomic::write(path, new_content.as_bytes())
}

/// Add an open task at the end of `## Todo`, opening the section at the end
/// of the file when there is none. One line.
pub fn add_todo(path: &Path, text: &str) -> Result<()> {
    add_todo_in(path, text, None)
}

/// Add an open todo under the `### phase` label, creating the label when the
/// list has none by that name — before an `### Other`, which is where a list
/// keeps what belongs to no phase, and at the end otherwise. `None` for the
/// phase is the plain append: the end of the section, whatever labels it has.
///
/// Matching a label is as lenient as reading one: the name, trimmed, ignoring
/// case. A list whose labels repeat takes the task into the last block of that
/// name, because that is where the eye goes.
pub fn add_todo_in(path: &Path, text: &str, phase: Option<&str>) -> Result<()> {
    add_todos_in(path, &[text.to_string()], phase)
}

/// Add several open todos, in the order given, where [`add_todo_in`] would
/// put one — together, as one run — in **one** atomic write, so a list
/// pasted in is in the file whole or not at all. Each text is trimmed and an
/// empty one skipped; a text with a line break in it refuses the lot, since a
/// second line would not be a todo.
pub fn add_todos_in(path: &Path, texts: &[String], phase: Option<&str>) -> Result<()> {
    let place = match phase.map(str::trim).filter(|p| !p.is_empty()) {
        Some(name) => TodoPlace::Phase(name.to_string()),
        None => TodoPlace::End,
    };
    add_todos_at(path, texts, &place).map(|_| ())
}

/// Where new todos go in a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoPlace {
    /// The end of the section: the plain append. A list whose last run sits
    /// under a `###` label takes them into that run.
    End,
    /// Under this `###` label — the last block of that name, ignoring case —
    /// opening the label when the list has none by that name.
    Phase(String),
    /// With the tasks that belong to no label: after the last of them, or
    /// above the first label when there are none.
    Loose,
}

/// [`add_todos_in`], at `place`, answering the ordinal the first of them got
/// — where the writer put it, so a caller that shows the list need not
/// predict it. Each text is read as somebody typed it ([`NewTodo::typed`]):
/// a trailing `@owner` is the owner. A file saved with `\r\n` line endings
/// keeps them.
pub fn add_todos_at(path: &Path, texts: &[String], place: &TodoPlace) -> Result<usize> {
    if texts.iter().any(|text| text.contains(['\n', '\r'])) {
        bail!("a todo is one line");
    }
    let todos: Vec<NewTodo> = texts.iter().map(|text| NewTodo::typed(text)).collect();
    add_todos_with(path, &todos, place)
}

/// [`add_todos_at`] over todos already read: each is written as
/// `- [ ] text @owner added:<now>` — the owner only when it has one, and
/// one `added:` stamp for the whole run, since it is one write at one
/// moment; a todo typed with an `added:` of its own keeps it, because it
/// says when it was added, which now is not.
pub fn add_todos_with(path: &Path, todos: &[NewTodo], place: &TodoPlace) -> Result<usize> {
    if todos.iter().any(|todo| {
        todo.text.contains(['\n', '\r'])
            || todo
                .owner
                .as_deref()
                .is_some_and(|o| o.contains(['\n', '\r']))
    }) {
        bail!("a todo is one line");
    }
    if todos.iter().any(|todo| {
        todo.owner
            .as_deref()
            .is_some_and(|owner| owner.is_empty() || owner.chars().any(char::is_whitespace))
    }) {
        bail!("an owner is one word");
    }
    let now = crate::util::time::now_iso8601();
    let lines: Vec<String> = todos
        .iter()
        .map(|todo| (todo.text.trim(), todo))
        .filter(|(text, _)| !text.is_empty())
        .map(|(text, todo)| {
            format!(
                "{text}{}",
                PlacedTodo::tail_for(
                    todo.owner.as_deref(),
                    Some(todo.added.as_deref().unwrap_or(now.as_str())),
                    None,
                )
            )
        })
        .collect();
    if lines.is_empty() {
        bail!("the todo is empty — nothing written");
    }
    let place = match place {
        TodoPlace::Phase(name) if name.contains(['\n', '\r']) => bail!("a phase is one line"),
        TodoPlace::Phase(name) => match phase_label(name) {
            Some(name) => TodoPlace::Phase(name),
            None => TodoPlace::End,
        },
        other => other.clone(),
    };
    let content = read_document(
        path,
        crate::util::plural::of(lines.len(), "add a todo", "add todos"),
    )?;
    let block: String = lines.iter().map(|text| format!("- [ ] {text}\n")).collect();
    let mut first = 0;
    let new_content = with_line_endings_of(&content, |content| {
        let (written, at) = place_block(content, &block, &place);
        // The block's first line starts at `at` in what was written: every
        // task above it keeps its ordinal.
        first = place_todos(&written)
            .iter()
            .take_while(|placed| placed.line.start < at)
            .count();
        written
    });
    crate::util::atomic::write(path, new_content.as_bytes())?;
    Ok(first)
}

/// `block` — whole task lines — placed in `content` at `place`, and where in
/// the result its first line starts.
fn place_block(content: &str, block: &str, place: &TodoPlace) -> (String, usize) {
    match (section_span(content, Section::Todo), place) {
        (Some(span), TodoPlace::Phase(name)) => insert_under_phase(content, &span, name, block),
        (Some(span), TodoPlace::Loose) => insert_loose(content, &span, block),
        (Some(span), TodoPlace::End) => {
            let has_items = !place_todos(content).is_empty();
            append_in_section(content, &span, has_items, block)
        }
        (None, TodoPlace::Phase(name)) => {
            let label = format!("### {name}\n");
            let (written, at) = open_section(content, TODO_HEADING, &format!("{label}{block}"));
            (written, at + label.len())
        }
        (None, _) => open_section(content, TODO_HEADING, block),
    }
}

/// `block` with the tasks that belong to no label: after the last of them,
/// or, when there are none, above the first label with a blank line under
/// it. A list with no labels at all is the plain append.
fn insert_loose(content: &str, span: &Range<usize>, block: &str) -> (String, usize) {
    if let Some(last) = place_todos(content)
        .iter()
        .rfind(|placed| placed.todo.phase.is_none())
    {
        let at = last.line.end;
        let text = with_newline_before(content, at, block);
        let offset = at + text.len() - block.len();
        return (splice(content, at..at, &text), offset);
    }
    match section_lines(content, span)
        .into_iter()
        .find(|(_, line)| phase_name(line).is_some())
    {
        Some((label, _)) => (
            splice(content, label.start..label.start, &format!("{block}\n")),
            label.start,
        ),
        None => append_in_section(content, span, false, block),
    }
}

/// `edit` applied to `content` as if its lines ended in `\n`, and the result
/// given back in the file's own endings: a file saved with `\r\n` throughout
/// keeps them on every line a writer adds. A file that mixes the two is
/// handed over as it is, since there is no one ending to keep.
fn with_line_endings_of(content: &str, edit: impl FnOnce(&str) -> String) -> String {
    let lines = content.matches('\n').count();
    let crlf = lines > 0 && content.matches("\r\n").count() == lines;
    if !crlf {
        return edit(content);
    }
    edit(&content.replace("\r\n", "\n")).replace('\n', "\r\n")
}

/// `line`, placed under `phase` inside the todo section at `span`, and where
/// in the result it starts.
fn insert_under_phase(
    content: &str,
    span: &Range<usize>,
    phase: &str,
    line: &str,
) -> (String, usize) {
    let wanted = phase.to_lowercase();
    let mut in_wanted = false;
    let mut at: Option<usize> = None;
    let mut other_at: Option<usize> = None;
    let mut last_task_end: Option<usize> = None;
    for (range, text) in section_lines(content, span) {
        if let Some(name) = phase_name(text) {
            let name = name.to_lowercase();
            in_wanted = name == wanted;
            if in_wanted {
                at = Some(range.end);
            } else if name == "other" && other_at.is_none() {
                other_at = Some(range.start);
            }
            continue;
        }
        if task_line(text).is_some() {
            last_task_end = Some(range.end);
            if in_wanted {
                at = Some(range.end);
            }
        }
    }
    // A label of that name: the task joins the end of its run.
    if let Some(at) = at {
        let text = with_newline_before(content, at, line);
        let offset = at + text.len() - line.len();
        return (splice(content, at..at, &text), offset);
    }
    // No such label. Open one where a reader would look for it.
    let label = format!("### {phase}\n");
    let block = format!("{label}{line}");
    if let Some(start) = other_at {
        return (
            splice(content, start..start, &format!("{block}\n")),
            start + label.len(),
        );
    }
    let at = last_task_end.unwrap_or(span.end).max(span.start);
    let blank = if content[..at].ends_with("\n\n") {
        ""
    } else {
        "\n"
    };
    let text = with_newline_before(content, at, &format!("{blank}{block}"));
    let offset = at + text.len() - line.len();
    (splice(content, at..at, &text), offset)
}

/// `text`, with the newline the line before it is missing — a section whose
/// last line the file ended without one.
fn with_newline_before(content: &str, at: usize, text: &str) -> String {
    if at == 0 || content[..at].ends_with('\n') {
        text.to_string()
    } else {
        format!("\n{text}")
    }
}

#[cfg(test)]
mod tests;
