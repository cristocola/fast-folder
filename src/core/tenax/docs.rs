//! fastf's own guides, compiled in, read by heading for Tenax's `fastf_docs`.
//!
//! The guides are what `CLAUDE.md` requires to change with every behaviour, so
//! a Tenax that reads them knows the commands of the build he is in — and a
//! guide read a section at a time costs a request only what it asked for.

use super::tools::GUIDES;

fn guide(name: &str) -> Option<&'static str> {
    Some(match name {
        "app" => include_str!("../../../docs/app.md"),
        "cli" => include_str!("../../../docs/cli.md"),
        "projects" => include_str!("../../../docs/projects.md"),
        "templates" => include_str!("../../../docs/templates.md"),
        "config" => include_str!("../../../docs/config.md"),
        "windows" => include_str!("../../../docs/windows.md"),
        _ => return None,
    })
}

/// A section's text is cut here: one section of `cli.md` is the longest any
/// answer needs, and a request should not carry a whole guide by accident.
const SECTION_MAX: usize = 12_000;

/// With no heading, the guide's headings, one a line; with one, that section —
/// from its heading to the next of the same or a higher level — matched
/// ignoring case and leading `#`s, and then by the first heading that holds
/// the words. `Err` is the sentence that goes back to the model.
pub fn lookup(name: &str, heading: Option<&str>) -> Result<String, String> {
    let Some(text) = guide(name) else {
        return Err(format!(
            "there is no guide called {name}; the guides are {}",
            GUIDES.join(", ")
        ));
    };
    lookup_in(name, text, heading)
}

/// [`lookup`] over the text of the guide called `name`.
fn lookup_in(name: &str, text: &str, heading: Option<&str>) -> Result<String, String> {
    // Offsets count the raw lines, endings and all: a Windows checkout gives
    // the guides `\r\n`, and `lines()` would hide a byte a line, so a section
    // would start in the wrong place or inside a character. A `#` line inside
    // a fenced block is a shell comment, and would end the section it is in.
    let mut headings: Vec<(usize, usize, &str)> = Vec::new();
    let mut at = 0usize;
    let mut fenced = false;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            let level = line.bytes().take_while(|b| *b == b'#').count();
            if level > 0 && line[level..].starts_with(' ') {
                headings.push((at, level, line[level..].trim()));
            }
        }
        at += raw.len();
    }
    let Some(wanted) = heading.map(str::trim).filter(|h| !h.is_empty()) else {
        let list: Vec<String> = headings
            .iter()
            .map(|(_, level, title)| format!("{}{title}", "  ".repeat(level.saturating_sub(1))))
            .collect();
        return Ok(list.join("\n"));
    };
    let wanted = wanted.trim_start_matches('#').trim().to_lowercase();
    let found = headings
        .iter()
        .position(|(_, _, title)| title.to_lowercase() == wanted)
        .or_else(|| {
            headings
                .iter()
                .position(|(_, _, title)| title.to_lowercase().contains(&wanted))
        });
    let Some(index) = found else {
        return Err(format!(
            "no heading in {name} reads {wanted:?}; ask with heading null for the list"
        ));
    };
    let (start, level, _) = headings[index];
    let end = headings[index + 1..]
        .iter()
        .find(|(_, other, _)| *other <= level)
        .map_or(text.len(), |(at, _, _)| *at);
    let section = text[start..end].replace("\r\n", "\n");
    Ok(cut(&section, SECTION_MAX))
}

/// `text` up to `max` bytes, cut at a line, saying so.
fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.trim_end().to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').unwrap_or(end);
    format!(
        "{}\n[the section goes on; ask for a narrower heading]",
        &text[..end]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_guide_the_tool_offers_is_compiled_in() {
        for name in GUIDES {
            assert!(
                guide(name).is_some_and(|text| text.starts_with('#')),
                "{name}"
            );
        }
    }

    #[test]
    fn a_guide_lists_its_headings_and_reads_one_section() {
        let list = lookup("app", None).unwrap();
        assert!(list.lines().any(|line| line.trim() == "Keys"));
        let keys = lookup("app", Some("## keys")).unwrap();
        assert!(keys.starts_with("## Keys"));
        // The section stops at the next heading of its level.
        assert!(!keys.contains("\n## Searching"));
        // Words inside a heading find it.
        assert!(
            lookup("app", Some("bases the list"))
                .unwrap()
                .starts_with("## Which bases")
        );
    }

    /// A Windows checkout gives the guides `\r\n` endings, and the build
    /// embeds them so: every section reads the same as from an `\n` checkout.
    /// Both spellings are made here, since the compiled-in one is whichever
    /// this checkout has.
    #[test]
    fn a_guide_checked_out_with_crlf_reads_the_same_sections() {
        for name in GUIDES {
            let lf = guide(name).unwrap().replace("\r\n", "\n");
            let crlf = lf.replace('\n', "\r\n");
            let list = lookup_in(name, &lf, None).unwrap();
            assert_eq!(lookup_in(name, &crlf, None).unwrap(), list, "{name}");
            for heading in list.lines() {
                assert_eq!(
                    lookup_in(name, &crlf, Some(heading)),
                    lookup_in(name, &lf, Some(heading)),
                    "{name}: {heading}"
                );
            }
        }
    }

    /// A `#` comment in a fenced block is a line of the example, not a
    /// heading: the settings section holds every `config set` under it.
    #[test]
    fn a_comment_in_a_code_block_is_no_heading() {
        let list = lookup("config", None).unwrap();
        assert!(!list.contains("Terminal to open"), "{list}");
        let settings = lookup("config", Some("Settings")).unwrap();
        assert!(
            settings.contains("fastf config set tenax off"),
            "{settings}"
        );
        assert!(settings.contains("post_create.print_path"), "{settings}");
    }

    #[test]
    fn a_wrong_guide_or_heading_says_what_there_is() {
        assert!(lookup("nope", None).unwrap_err().contains("app, cli"));
        assert!(
            lookup("cli", Some("no such heading at all"))
                .unwrap_err()
                .contains("heading null")
        );
    }

    #[test]
    fn a_long_section_is_cut_at_a_line() {
        let text = "a\n".repeat(10);
        let cut = cut(&text, 7);
        assert!(cut.ends_with("narrower heading]"));
        assert!(cut.starts_with("a\na\na"));
    }
}
