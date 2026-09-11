//! The markdown body of `PROJECT_INFO.md`: its sections, the notes, the todos.
//!
//! Below the frontmatter the file is the user's own, and fastf reads and
//! writes exactly three things in it, each under a `##` heading it finds by
//! one rule: the **notes** — dated entries, one `- <timestamp> — <text>` line
//! and as many indented lines under it as the note has — the **todos** —
//! `- [ ] text` and `- [x] text` — and, in a file written before v3.6.0, a
//! `## Journal` section that held the notes then and keeps holding them now.
//!
//! **One grammar, read by the writer and the reader alike.** `notes_span`
//! is where the notes are, for `append_journal_entry` and `notes_in` both;
//! `section_span` is where any section is. The two halves had a definition
//! each once, and a note the writer put past the point the reader stopped at
//! was written, confirmed, and never seen again.
//!
//! **The reader never fails and never drops a line it could show.** A
//! heading is matched wherever it starts a line, in any case, with or without
//! a trailing colon; a line under the notes heading that does not start an
//! entry belongs to the entry above it (or, before the first entry, to one
//! undated note); a line under the todo heading that is not a task is simply
//! not a task. A file somebody hand-edited into a shape fastf never wrote is
//! shown as far as it can be read, which is as far as it goes.
//!
//! **The writer changes the bytes it is about and no others.** An append
//! lands at the end of its section; an edit splices over the note's own lines;
//! a toggle rewrites the one character inside the brackets. A single-line
//! note writes the bytes every earlier version wrote, so a diff over an old
//! file shows only the line that was added.

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

/// One task under `## Todo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Todo {
    pub done: bool,
    pub text: String,
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

/// Where the notes are: the `## Journal` section if the file has one — a
/// file written before v3.6.0 keeps its shape, byte for byte — else the
/// `## Notes` section. **Read by the writer and the reader alike.**
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
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if split_frontmatter_body(&content).is_none() {
        bail!("{} has no YAML frontmatter — cannot {verb}", path.display());
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
fn append_in_section(content: &str, span: &Range<usize>, has_items: bool, line: &str) -> String {
    let mut out = String::with_capacity(content.len() + line.len() + 2);
    out.push_str(&content[..span.end]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !has_items && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(line);
    out.push_str(&content[span.end..]);
    out
}

/// A new section at the end of the file: a blank line, the heading, a blank
/// line, the first line.
fn open_section(content: &str, heading: &str, line: &str) -> String {
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
    out.push_str(line);
    out
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
    let new_content = match notes_span(&content) {
        Some(span) => {
            let dated = notes_in_section(&content, &span)
                .iter()
                .any(|placed| placed.note.timestamp.is_some());
            append_in_section(&content, &span, dated, &entry)
        }
        None => open_section(&content, NOTES_HEADING, &entry),
    };
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
    let new_content = if text.is_empty() {
        remove_lines(&content, current.span.clone())
    } else {
        let rendered = match &current.note.timestamp {
            Some(timestamp) => render_entry(timestamp, text),
            None => render_preamble(text)?,
        };
        splice(&content, current.span.clone(), &rendered)
    };
    crate::util::atomic::write(path, new_content.as_bytes())
}

/// `content` without the lines in `span`, and without the blank line that
/// removing them would leave doubled.
fn remove_lines(content: &str, span: Range<usize>) -> String {
    let mut out = splice(content, span.clone(), "");
    let at = span.start;
    if out[..at].ends_with("\n\n") && out[at..].starts_with('\n') {
        out.remove(at);
    }
    out
}

/// Set the undated note — the free text above the first entry of the notes
/// section — to `text`: written where it was, or under the heading when
/// there was none, or taken out when `text` is empty. Every other byte stays.
/// A file with no notes section gets one at the end. What the pane's notes
/// editor wrote before notes had dates, kept for the file shape it produces.
pub fn set_preamble(path: &Path, text: &str) -> Result<()> {
    let content = read_document(path, "write the notes")?;
    let text = text.trim_matches(['\n', '\r']);
    // The `## Notes` section and not the journal's: in a file with both, the
    // free text was always the notes' and the entries the journal's.
    let new_content = match section_span(&content, Section::Notes) {
        Some(span) => {
            let lines = section_lines(&content, &span);
            let preamble = notes_in_section(&content, &span)
                .into_iter()
                .find(|placed| placed.note.timestamp.is_none());
            // The end of the heading line: where the section's own text
            // begins.
            let after_heading = lines
                .first()
                .map(|(range, _)| range.start)
                .unwrap_or(span.end);
            match (preamble, text.is_empty()) {
                (Some(current), true) => {
                    // Out with the blank line above it too, then a blank
                    // line back unless one is already there.
                    let mut out = splice(&content, after_heading..current.span.end, "");
                    if !out[after_heading..].starts_with('\n') {
                        out.insert(after_heading, '\n');
                    }
                    out
                }
                (Some(current), false) => {
                    splice(&content, current.span.clone(), &render_preamble(text)?)
                }
                (None, true) => content.clone(),
                (None, false) => {
                    let mut block = String::from("\n");
                    block.push_str(&render_preamble(text)?);
                    let mut out = splice(&content, after_heading..after_heading, &block);
                    let resume = after_heading + block.len();
                    if !out[resume..].starts_with('\n') {
                        out.insert(resume, '\n');
                    }
                    out
                }
            }
        }
        None if text.is_empty() => content.clone(),
        // A file that lost its notes section gets one back where it belongs:
        // before the journal, with the blank line the journal expects under
        // it, else at the end.
        None => match section_span(&content, Section::Journal) {
            Some(journal) => {
                let block = format!("{NOTES_HEADING}\n\n{}\n", render_preamble(text)?);
                splice(&content, journal.start..journal.start, &block)
            }
            None => open_section(&content, NOTES_HEADING, &render_preamble(text)?),
        },
    };
    crate::util::atomic::write(path, new_content.as_bytes())
}

// ---------------------------------------------------------------------------
// Todos
// ---------------------------------------------------------------------------

/// A task as it sits in the file: the task, and where the character inside
/// its brackets is (an empty range for `[]`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlacedTodo {
    todo: Todo,
    marker: Range<usize>,
}

/// What a task line is: a list item, any indent, then `[ ]`, `[x]`, `[X]` or
/// `[]`, then the text. Anything else under the heading is not a task.
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
    let text = rest[inside + 1..].trim();
    Some((done, marker_at, inside, text))
}

fn place_todos(content: &str) -> Vec<PlacedTodo> {
    let Some(span) = section_span(content, Section::Todo) else {
        return Vec::new();
    };
    section_lines(content, &span)
        .into_iter()
        .filter_map(|(range, line)| {
            let (done, marker_at, inside, text) = task_line(line)?;
            Some(PlacedTodo {
                todo: Todo {
                    done,
                    text: text.to_string(),
                },
                marker: range.start + marker_at..range.start + marker_at + inside,
            })
        })
        .collect()
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

/// Flip task `ordinal` between open and done by rewriting the one character
/// inside its brackets. Returns whether it is done now. `expected` is the
/// text the caller last read — see [`replace_note`].
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
    let new_content = splice(
        &content,
        current.marker.clone(),
        if done { "x" } else { " " },
    );
    crate::util::atomic::write(path, new_content.as_bytes())?;
    Ok(done)
}

/// Add an open task at the end of `## Todo`, opening the section at the end
/// of the file when there is none. One line.
pub fn add_todo(path: &Path, text: &str) -> Result<()> {
    if text.contains(['\n', '\r']) {
        bail!("a todo is one line");
    }
    let text = text.trim();
    if text.is_empty() {
        bail!("the todo is empty — nothing written");
    }
    let content = read_document(path, "add a todo")?;
    let line = format!("- [ ] {text}\n");
    let new_content = match section_span(&content, Section::Todo) {
        Some(span) => {
            let has_items = !place_todos(&content).is_empty();
            append_in_section(&content, &span, has_items, &line)
        }
        None => open_section(&content, TODO_HEADING, &line),
    };
    crate::util::atomic::write(path, new_content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "---\nid: ID0001\ntemplate: t\n---\n\n# Project Info\n\n";

    fn doc(body: &str) -> String {
        format!("{HEAD}{body}")
    }

    fn dated(ts: &str, text: &str) -> Note {
        Note {
            timestamp: Some(ts.to_string()),
            text: text.to_string(),
        }
    }

    fn undated(text: &str) -> Note {
        Note {
            timestamp: None,
            text: text.to_string(),
        }
    }

    /// A temp file holding `content`, for the writers.
    fn file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::core::project_info::pinfo_path(dir.path());
        fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn a_heading_is_matched_where_it_starts_a_line_in_any_spelling() {
        for heading in [
            "## Notes",
            "## notes",
            "## NOTES:",
            "##Notes",
            "##  Notes  ",
        ] {
            let content = doc(&format!("{heading}\n\ntext\n"));
            let span = section_span(&content, Section::Notes).unwrap();
            assert_eq!(&content[span], &format!("{heading}\n\ntext\n"), "{heading}");
        }
        for not in [
            "### Notes",
            "# Notes",
            "Notes",
            "  ## Notes",
            "see ## Notes below",
        ] {
            let content = doc(&format!("{not}\n\ntext\n"));
            assert_eq!(section_span(&content, Section::Notes), None, "{not}");
        }
        for todo in [
            "## Todo", "## TODO:", "## Todos", "## To do", "## to-do", "## Tasks",
        ] {
            assert!(section_span(&doc(&format!("{todo}\n")), Section::Todo).is_some());
        }
    }

    #[test]
    fn a_section_ends_at_the_next_heading_or_the_end_of_the_file() {
        let content = doc("## Notes\n\nfree\n\n## Archive\n\nkept\n");
        let span = section_span(&content, Section::Notes).unwrap();
        assert_eq!(&content[span.clone()], "## Notes\n\nfree\n");
        assert_eq!(&content[span.end..], "\n## Archive\n\nkept\n");
        assert_eq!(section_span(&content, Section::Todo), None);

        let content = doc("## Notes\n\nfree");
        let span = section_span(&content, Section::Notes).unwrap();
        assert_eq!(span.end, content.len());

        // A `###` inside the section is part of it.
        let content = doc("## Notes\n\n### day one\n\nfree\n");
        let span = section_span(&content, Section::Notes).unwrap();
        assert_eq!(&content[span], "## Notes\n\n### day one\n\nfree\n");

        // A heading in the frontmatter is a value, not a section.
        let content = "---\nid: ID0001\ntitle: '## Notes'\n---\n\nbody\n";
        assert_eq!(section_span(content, Section::Notes), None);
        // And a BOM does not shift the offsets.
        let bom = format!("\u{feff}{}", doc("## Notes\n\nfree\n"));
        let span = section_span(&bom, Section::Notes).unwrap();
        assert_eq!(&bom[span], "## Notes\n\nfree\n");
    }

    #[test]
    fn every_shape_of_entry_is_read_and_every_line_under_it_belongs_to_it() {
        let content = doc("## Notes\n\n\
             - 2026-01-01T00:00:00Z — the bytes fastf writes\n  \
               with a second line\n\n  \
               and a third after a blank\n\
             - 2026-01-02 no separator at all\n\
             - 2026-01-03: a colon\n\
             * 2026-01-04 - a hyphen, on a star\n\
             - 2026-01-05T10:00:00Z — corrupted by the old writer\n\
             line two at column zero\n\
             - [ ] not an entry, so still line three\n\
             oh you — who edit my videos\n\n\n");
        let notes = notes_in(&content);
        assert_eq!(
            notes,
            vec![
                dated(
                    "2026-01-01T00:00:00Z",
                    "the bytes fastf writes\nwith a second line\n\nand a third after a blank"
                ),
                dated("2026-01-02", "no separator at all"),
                dated("2026-01-03", "a colon"),
                dated("2026-01-04", "a hyphen, on a star"),
                dated(
                    "2026-01-05T10:00:00Z",
                    "corrupted by the old writer\nline two at column zero\n- [ ] not an entry, so still line three\noh you — who edit my videos"
                ),
            ]
        );
    }

    #[test]
    fn text_above_the_first_entry_is_one_undated_note_and_both_sections_are_read() {
        let content = doc(
            "## Notes\n\nremember the invoice\n- and a list\n\n- 2026-02-01 — under notes\n\n\
             ## Journal\n\n- 2026-01-01T00:00:00Z — under the journal\n",
        );
        assert_eq!(
            notes_in(&content),
            vec![
                undated("remember the invoice\n- and a list"),
                dated("2026-02-01", "under notes"),
                dated("2026-01-01T00:00:00Z", "under the journal"),
            ]
        );
        assert!(notes_in(&doc("## Notes\n\n")).is_empty());
        assert!(notes_in(&doc("# nothing\n")).is_empty());
        assert!(notes_in(&doc("## Notes\n\n\n\n")).is_empty());
        // A hand-edited timestamp is whatever was typed; nothing panics.
        let odd = doc("## Journal\n\n- 日本語のタイムスタンプ — hand-edited entry\n");
        assert_eq!(
            notes_in(&odd),
            vec![dated("日本語のタイムスタンプ", "hand-edited entry")]
        );
    }

    #[test]
    fn a_dated_note_is_written_one_line_as_before_and_its_other_lines_indented() {
        assert_eq!(
            render_entry("2026-01-01T00:00:00Z", "one line"),
            "- 2026-01-01T00:00:00Z — one line\n"
        );
        assert_eq!(
            render_entry(
                "T",
                "first\nsecond  \n\n## not a heading\n- not an entry — really"
            ),
            "- T — first\n  second\n\n  ## not a heading\n  - not an entry — really\n"
        );
        // And what was written is what is read.
        let content = doc(&format!(
            "## Notes\n\n{}",
            render_entry(
                "T",
                "first\nsecond\n\n## not a heading\n- not an entry — really"
            )
        ));
        assert_eq!(
            notes_in(&content),
            vec![dated(
                "T",
                "first\nsecond\n\n## not a heading\n- not an entry — really"
            )]
        );
        assert_eq!(
            section_span(&content, Section::Notes).unwrap().end,
            content.len()
        );
    }

    #[test]
    fn an_append_lands_at_the_end_of_the_section_under_a_blank_line() {
        // A fresh file: the shape `render_at` writes.
        let (_dir, path) = file(&doc("## Notes\n\n"));
        append_journal_entry(&path, "first").unwrap();
        append_journal_entry(&path, "second\nline two").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        let body = &after[HEAD.len()..];
        assert!(body.starts_with("## Notes\n\n- "), "{body:?}");
        assert!(body.contains(" — first\n- "), "{body:?}");
        assert!(body.ends_with(" — second\n  line two\n"), "{body:?}");
        assert!(!after.contains("## Journal"));
        assert_eq!(
            notes_in(&after)
                .iter()
                .map(|n| n.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second\nline two"]
        );

        // A legacy file: the journal keeps the notes, byte for byte around
        // the new line, and the user's own section stays below.
        let legacy =
            doc("## Notes\n\n## Journal\n\n- 2026-01-01T00:00:00Z — a\n\n## Archive\n\nmine\n");
        let (_dir, path) = file(&legacy);
        append_journal_entry(&path, "b").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        let ts = notes_in(&after)[1].timestamp.clone().unwrap();
        assert_eq!(
            after,
            legacy.replace(
                "— a\n\n## Archive",
                &format!("— a\n- {ts} — b\n\n## Archive")
            )
        );

        // A section with prose and no entry yet gets a blank line first; a
        // heading with nothing under it likewise; a file with no section
        // gets one at the end.
        let (_dir, path) = file(&doc("## Notes\n\nremember\n"));
        append_journal_entry(&path, "c").unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("remember\n\n- ")
        );
        let (_dir, path) = file(&doc("## Notes\n## Archive\n"));
        append_journal_entry(&path, "d").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("## Notes\n\n- "), "{after:?}");
        assert!(after.contains(" — d\n\n## Archive\n"), "{after:?}");
        let (_dir, path) = file(&doc("# Project Info\n\nprose"));
        append_journal_entry(&path, "e").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("prose\n\n## Notes\n\n- "), "{after:?}");
        assert!(after.ends_with(" — e\n"), "{after:?}");
    }

    #[test]
    fn a_note_is_rewritten_over_its_own_lines_or_taken_out() {
        let before = doc(
            "## Notes\n\nfree text\n\n- 2026-01-01 — one\n  more\n- 2026-01-02 — two\n\n## Archive\n\nmine\n",
        );
        let (_dir, path) = file(&before);
        replace_note(&path, 1, "one\nmore", "changed\nand longer\n\nstill").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            before.replace(
                "- 2026-01-01 — one\n  more\n",
                "- 2026-01-01 — changed\n  and longer\n\n  still\n"
            )
        );
        replace_note(&path, 0, "free text", "other text\nsecond").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("## Notes\n\nother text\nsecond\n\n- 2026-01-01"),
            "{after:?}"
        );
        // Removal takes the lines and the blank line it would have doubled.
        replace_note(&path, 1, "changed\nand longer\n\nstill", "").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("second\n\n- 2026-01-02 — two\n\n## Archive"),
            "{after:?}"
        );
        // The wrong ordinal, or a note that moved on, changes nothing.
        let err = replace_note(&path, 0, "free text", "x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("changed meanwhile"), "{err}");
        let err = replace_note(&path, 9, "", "x").unwrap_err().to_string();
        assert!(err.contains("no longer there"), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        // The undated note may not hold a heading.
        let err = replace_note(&path, 0, "other text\nsecond", "fine\n## Journal\nnot")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("## Journal") && err.contains("end the notes"),
            "{err}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }

    #[test]
    fn the_preamble_is_set_where_it_was_or_under_the_heading_and_emptied_away() {
        // The legacy shape: an empty notes section over a journal.
        let before = doc("## Notes\n\n## Journal\n\n- 2026-01-01T00:00:00Z — began\n");
        let (_dir, path) = file(&before);
        set_preamble(&path, "first cut due Friday\nthen colour").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before.replace(
                "## Notes\n\n## Journal\n",
                "## Notes\n\nfirst cut due Friday\nthen colour\n\n## Journal\n"
            )
        );
        set_preamble(&path, "").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "emptied reads as never written"
        );

        // A fresh file, then entries under the preamble, then emptied again.
        let (_dir, path) = file(&doc("## Notes\n\n"));
        set_preamble(&path, "remember the invoice").unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .ends_with("## Notes\n\nremember the invoice\n\n")
        );
        append_journal_entry(&path, "sent").unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("remember the invoice\n\n- ")
        );
        set_preamble(&path, "").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("## Notes\n\n- "), "{after:?}");
        assert!(!after.contains("invoice"));

        // Entries with no blank line under the heading: the preamble goes
        // between, with a blank line on each side.
        let (_dir, path) = file(&doc("## Notes\n- 2026-01-01 — a\n"));
        set_preamble(&path, "above").unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("## Notes\n\nabove\n\n- 2026-01-01 — a\n")
        );

        // A file that lost its notes section gets one back before the
        // journal; with no journal either, at the end. A heading is refused.
        let (_dir, path) = file(&doc("## Journal\n\n- 2026-01-01T00:00:00Z — began\n"));
        set_preamble(&path, "back").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("# Project Info\n\n## Notes\n\nback\n\n## Journal\n\n- "),
            "{after:?}"
        );
        assert_eq!(
            notes_in(&after),
            vec![undated("back"), dated("2026-01-01T00:00:00Z", "began")]
        );
        let (_dir, path) = file(&doc("# Project Info\n"));
        set_preamble(&path, "back").unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .ends_with("# Project Info\n\n## Notes\n\nback\n")
        );
        let err = set_preamble(&path, "fine\n## Journal\nnot")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("## Journal") && err.contains("end the notes"),
            "{err}"
        );
        set_preamble(&path, "# a title is fine\n- and a list").unwrap();
    }

    #[test]
    fn a_task_is_read_in_any_indent_and_toggled_by_one_byte() {
        let before = doc(
            "## Notes\n\n- 2026-01-01 — a\n\n## TODO:\n\nsome prose\n- [x] ingested\n  * [ ] edited\n- [] delivered\n- not a task\n- [y] nor this\n",
        );
        assert_eq!(
            todos_in(&before),
            vec![
                Todo {
                    done: true,
                    text: "ingested".to_string()
                },
                Todo {
                    done: false,
                    text: "edited".to_string()
                },
                Todo {
                    done: false,
                    text: "delivered".to_string()
                },
            ]
        );
        let (_dir, path) = file(&before);
        assert!(!toggle_todo(&path, 0, "ingested").unwrap());
        assert!(toggle_todo(&path, 1, "edited").unwrap());
        assert!(toggle_todo(&path, 2, "delivered").unwrap());
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            before
                .replace("[x] ingested", "[ ] ingested")
                .replace("[ ] edited", "[x] edited")
                .replace("[] delivered", "[x] delivered")
        );
        let err = toggle_todo(&path, 0, "edited").unwrap_err().to_string();
        assert!(err.contains("changed meanwhile"), "{err}");
        assert!(toggle_todo(&path, 9, "").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }

    #[test]
    fn a_todo_is_added_at_the_end_of_its_section_or_the_section_is_opened() {
        let (_dir, path) = file(&doc("## Notes\n\n- 2026-01-01 — a\n"));
        add_todo(&path, "  deliver the video ").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.ends_with("— a\n\n## Todo\n\n- [ ] deliver the video\n"),
            "{after:?}"
        );
        add_todo(&path, "invoice").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.ends_with("- [ ] deliver the video\n- [ ] invoice\n"),
            "{after:?}"
        );
        assert_eq!(todos_in(&after).len(), 2);

        // Mid-file, with the user's own section under it; and an empty
        // heading gets its blank line.
        let (_dir, path) = file(&doc("## Todo\n## Archive\n"));
        add_todo(&path, "one").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("## Todo\n\n- [ ] one\n\n## Archive\n"),
            "{after:?}"
        );

        for bad in ["", "  ", "two\nlines"] {
            assert!(add_todo(&path, bad).is_err(), "{bad:?}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }

    #[test]
    fn a_file_without_frontmatter_is_refused_by_every_writer() {
        let (_dir, path) = file("# no frontmatter\n\n## Notes\n\n- 2026-01-01 — a\n");
        for err in [
            append_journal_entry(&path, "x").unwrap_err(),
            replace_note(&path, 0, "a", "b").unwrap_err(),
            set_preamble(&path, "x").unwrap_err(),
            toggle_todo(&path, 0, "").unwrap_err(),
            add_todo(&path, "x").unwrap_err(),
        ] {
            assert!(err.to_string().contains("no YAML frontmatter"), "{err}");
        }
    }
}
