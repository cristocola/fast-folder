//! The help overlay and the palette, read from the registry, and the keys a
//! sentence names.

use super::*;

/// The help overlay: every command that fires in `ctx` (plus the global ones),
/// grouped by category in `Category::ALL` order.
pub fn help_sections(ctx: Context) -> Vec<(Category, Vec<&'static Command>)> {
    Category::ALL
        .iter()
        .map(|category| {
            let commands: Vec<&'static Command> = COMMANDS
                .iter()
                .filter(|c| c.category == *category)
                .filter(|c| c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global))
                .filter(|c| exists_here(c.id))
                .collect();
            (*category, commands)
        })
        .filter(|(_, commands)| !commands.is_empty())
        .collect()
}

/// One line of the help overlay's body, as text; the view styles it. Built
/// here so `update` can count the lines with the same arithmetic the view
/// draws them by, and clamp the scroll to what there is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelpLine {
    Heading(&'static str),
    /// A command: its keys, its title, and the first line of its description.
    Command {
        keys: String,
        title: &'static str,
        description: String,
    },
    /// The rest of a description that did not fit its line, drawn `indent`
    /// columns in.
    Continuation {
        indent: usize,
        text: String,
    },
    Blank,
}

/// Below this many columns for a description, the help puts descriptions on
/// their own line under the keys and the title rather than beside them.
const NARROW_DESCRIPTION: usize = 28;

/// The three column widths the help overlay lays out in: keys, title, and
/// what is left for the description — measured from the commands themselves,
/// so a long title can never run into its description.
pub fn help_columns(
    ctx: Context,
    inner_width: usize,
    g: &crate::tui::theme::Glyphs,
) -> (usize, usize, usize) {
    let commands: Vec<&Command> = help_sections(ctx)
        .into_iter()
        .flat_map(|(_, commands)| commands)
        .collect();
    let keys_width = commands
        .iter()
        .map(|c| key_labels(ctx, c, g).chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 18);
    let title_width = commands
        .iter()
        .map(|c| c.title.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 36);
    // Three columns of indent, a space after the keys, a space after the title.
    let description_width = inner_width.saturating_sub(3 + keys_width + 1 + title_width + 1);
    (keys_width, title_width, description_width)
}

/// `? / F1`, `c / : / Ctrl-p`: a command's keys as the help prints them.
/// The key a command is bound to, for a sentence that has to name one.
///
/// **Read the registry; never spell a key in prose.** Eight sentences did —
/// three of them said "no templates yet" three different ways and one named `T`,
/// the tab switch, where the registry says `n`. They all happened to be right
/// the day they were written, which is exactly the drift the one registry
/// exists to prevent: `command.rs` carried four copies of its key table in the
/// prototype and they had already disagreed.
///
/// Empty when a command has no key of its own, which is a sentence that should
/// not have been written.
/// The one sentence for "there are no templates on disk", and the key it names
/// is the key that makes one.
pub const NO_TEMPLATES: &str = "no templates yet — n makes one";

pub fn key_of(id: CommandId) -> String {
    find(id)
        .keys
        .first()
        .map(|key| key.label())
        .unwrap_or_default()
}

/// The key a command is bound to, in `g`'s alphabet — for a key line drawn on
/// screen, where an arrow has to be one the terminal can draw.
pub fn key_of_in(id: CommandId, g: &crate::tui::theme::Glyphs) -> String {
    find(id)
        .keys
        .first()
        .map(|key| key.label_in(g))
        .unwrap_or_default()
}

pub fn key_labels(ctx: Context, command: &Command, g: &crate::tui::theme::Glyphs) -> String {
    keys_in(ctx, command)
        .iter()
        .map(|k| k.label_in(g))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The help overlay's body for `ctx`, laid out for `inner_width` columns: a
/// description that does not fit its line continues under itself.
pub fn help_lines(
    ctx: Context,
    inner_width: usize,
    g: &crate::tui::theme::Glyphs,
) -> Vec<HelpLine> {
    let (keys_width, title_width, description_width) = help_columns(ctx, inner_width, g);
    // Wide enough: three columns. Narrow: the description on its own line
    // under the title, indented past the keys, so it reads as prose rather
    // than a ladder of three-word lines.
    let beside = description_width >= NARROW_DESCRIPTION;
    let (indent, width) = if beside {
        (3 + keys_width + 1 + title_width + 1, description_width)
    } else {
        let indent = 3 + keys_width + 1;
        (indent, inner_width.saturating_sub(indent + 1).max(12))
    };
    let mut lines = Vec::new();
    for (category, commands) in help_sections(ctx) {
        lines.push(HelpLine::Heading(category.label()));
        for c in commands {
            let mut parts = wrap_words(c.description, width).into_iter();
            lines.push(HelpLine::Command {
                keys: key_labels(ctx, c, g),
                title: c.title,
                description: if beside {
                    parts.next().unwrap_or_default()
                } else {
                    String::new()
                },
            });
            lines.extend(parts.map(|text| HelpLine::Continuation { indent, text }));
        }
        lines.push(HelpLine::Blank);
    }
    lines
}

/// Greedy word wrap into lines of at most `width` characters; a word longer
/// than a line is broken where it must be.
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let head: String = word.chars().take(width).collect();
            word = word.chars().skip(width).collect();
            lines.push(head);
        }
        let needed = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if needed > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(&word);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// How many lines the help overlay draws for `ctx` at `inner_width`: the
/// body, and the five lines of footer under it. The view draws exactly
/// this, and `update` clamps the scroll with it.
pub fn help_line_count(ctx: Context, inner_width: usize, g: &crate::tui::theme::Glyphs) -> usize {
    help_lines(ctx, inner_width, g).len() + 5
}

/// The palette's command entries: everything listed and not hidden, the
/// current context's commands first, then the global ones, then the rest.
pub fn palette_entries(ctx: Context, app: &App) -> Vec<(&'static Command, Availability)> {
    let rank = |c: &Command| {
        if c.contexts.contains(&ctx) {
            0
        } else if c.contexts.contains(&Context::Global) {
            1
        } else {
            2
        }
    };
    let mut entries: Vec<(&'static Command, Availability)> = COMMANDS
        .iter()
        .filter(|c| c.palette)
        .map(|c| (c, (c.available)(app)))
        .filter(|(_, availability)| *availability != Availability::Hidden)
        .collect();
    entries.sort_by_key(|(c, _)| rank(c));
    entries
}
