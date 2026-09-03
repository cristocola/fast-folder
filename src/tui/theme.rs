//! Colours and glyphs, chosen once from the environment.
//!
//! The look is a command centre, not a demo: muted and cool, minimal and
//! sophisticated. The terminal's own text colour carries the content, slate
//! grey recedes, one steel-blue accent says what has focus, and the three
//! meaning colours — good, warning, bad — appear only where they mean
//! something. Bold is rare (the app's name, the selected row) so it keeps its
//! weight; glyphs are few and each has one job.
//!
//! The palette is semantic — *accent*, *dim*, *good*, *bad* — and the
//! environment decides what each one is: `NO_COLOR` or a dumb terminal gets
//! none, a terminal that announces truecolor gets the muted RGB version,
//! everything else gets the sixteen ANSI colours used sparingly. Snapshot
//! tests use `Theme::mono` so their frames never depend on the machine that
//! rendered them.

use ratatui::style::{Color, Modifier, Style};

/// Which palette was chosen, mostly for `fastf paths`-style diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    Mono,
    Ansi,
    Rich,
}

/// The characters the frames are drawn with. Few, and each with one job:
/// the cursor, a mark, a tag dot, the search prefix, a warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub cursor: &'static str,
    pub mark: &'static str,
    pub dot: &'static str,
    pub search: &'static str,
    pub warn: &'static str,
    pub ellipsis: &'static str,
    pub sep: &'static str,
    pub arrow: &'static str,
    pub folder: &'static str,
    pub rule: &'static str,
    pub check: &'static str,
    pub cross: &'static str,
    pub pending: &'static str,
}

impl Glyphs {
    pub const fn unicode() -> Self {
        Self {
            cursor: "▸",
            mark: "✓",
            dot: "●",
            search: "⌕",
            warn: "⚠",
            ellipsis: "…",
            sep: "·",
            arrow: "→",
            folder: "▸",
            rule: "─",
            check: "✓",
            cross: "✗",
            pending: "scanning…",
        }
    }

    pub const fn ascii() -> Self {
        Self {
            cursor: ">",
            mark: "*",
            dot: "*",
            search: "/",
            warn: "!",
            ellipsis: "...",
            sep: "-",
            arrow: "->",
            folder: ">",
            rule: "-",
            check: "+",
            cross: "x",
            pending: "scanning...",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub kind: ThemeKind,
    pub glyphs: Glyphs,
    pub accent: Color,
    pub accent_alt: Color,
    pub text: Color,
    pub dim: Color,
    pub good: Color,
    pub bad: Color,
    pub warn: Color,
    pub border: Color,
    pub border_focus: Color,
    pub mark: Color,
    /// The highlighted row.
    pub selection: Style,
    /// Colours a tag hashes onto.
    pub tags: [Color; 6],
}

impl Theme {
    /// Read the environment and pick a palette.
    pub fn detect() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let term = std::env::var("TERM").unwrap_or_default();
        let glyphs = if ascii_wanted() {
            Glyphs::ascii()
        } else {
            Glyphs::unicode()
        };
        if no_color || term == "dumb" {
            return Self::mono().with_glyphs(glyphs);
        }
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        if colorterm == "truecolor" || colorterm == "24bit" {
            Self::rich().with_glyphs(glyphs)
        } else {
            Self::ansi().with_glyphs(glyphs)
        }
    }

    fn with_glyphs(mut self, glyphs: Glyphs) -> Self {
        self.glyphs = glyphs;
        self
    }

    /// No colour at all: bold and reverse video carry the structure. What the
    /// snapshot tests render with.
    pub fn mono() -> Self {
        Self {
            kind: ThemeKind::Mono,
            glyphs: Glyphs::unicode(),
            accent: Color::Reset,
            accent_alt: Color::Reset,
            text: Color::Reset,
            dim: Color::Reset,
            good: Color::Reset,
            bad: Color::Reset,
            warn: Color::Reset,
            border: Color::Reset,
            border_focus: Color::Reset,
            mark: Color::Reset,
            selection: Style::default().add_modifier(Modifier::REVERSED),
            tags: [Color::Reset; 6],
        }
    }

    /// The sixteen ANSI colours, used sparingly: the terminal's own text
    /// colour for text, dark grey for what recedes, one accent for what has
    /// focus, and the three meaning colours only where they mean something.
    pub fn ansi() -> Self {
        Self {
            kind: ThemeKind::Ansi,
            glyphs: Glyphs::unicode(),
            accent: Color::Blue,
            accent_alt: Color::Cyan,
            text: Color::Reset,
            dim: Color::DarkGray,
            good: Color::Green,
            bad: Color::Red,
            warn: Color::Yellow,
            border: Color::DarkGray,
            border_focus: Color::Blue,
            mark: Color::Yellow,
            selection: Style::default().add_modifier(Modifier::REVERSED),
            tags: [
                Color::Blue,
                Color::Cyan,
                Color::Green,
                Color::Magenta,
                Color::Yellow,
                Color::White,
            ],
        }
    }

    /// Truecolor: the same restraint with a muted, cool palette. Slate greys
    /// for what recedes, steel blue for focus, amber for a warning, and a set
    /// of desaturated tag colours that sit beside each other without shouting.
    pub fn rich() -> Self {
        Self {
            kind: ThemeKind::Rich,
            accent: Color::Rgb(122, 162, 196),
            accent_alt: Color::Rgb(150, 172, 192),
            dim: Color::Rgb(118, 126, 136),
            good: Color::Rgb(128, 168, 140),
            bad: Color::Rgb(196, 118, 110),
            warn: Color::Rgb(204, 168, 108),
            border: Color::Rgb(70, 76, 84),
            border_focus: Color::Rgb(122, 162, 196),
            mark: Color::Rgb(204, 168, 108),
            selection: Style::default()
                .bg(Color::Rgb(44, 52, 62))
                .add_modifier(Modifier::BOLD),
            tags: [
                Color::Rgb(122, 162, 196),
                Color::Rgb(120, 162, 152),
                Color::Rgb(160, 160, 122),
                Color::Rgb(160, 140, 162),
                Color::Rgb(182, 160, 130),
                Color::Rgb(140, 150, 170),
            ],
            ..Self::ansi()
        }
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.dim)
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.text)
    }

    pub fn accent(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn accent_alt(&self) -> Style {
        Style::default()
            .fg(self.accent_alt)
            .add_modifier(Modifier::BOLD)
    }

    pub fn good(&self) -> Style {
        Style::default().fg(self.good).add_modifier(Modifier::BOLD)
    }

    pub fn bad(&self) -> Style {
        Style::default().fg(self.bad).add_modifier(Modifier::BOLD)
    }

    pub fn warn(&self) -> Style {
        Style::default().fg(self.warn).add_modifier(Modifier::BOLD)
    }

    pub fn bold(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    pub fn key(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn border(&self, focused: bool) -> Style {
        Style::default().fg(if focused {
            self.border_focus
        } else {
            self.border
        })
    }

    /// The characters a search word hit, inside a row: underlined, in the
    /// secondary accent — visible, not loud.
    pub fn hit(&self) -> Style {
        Style::default()
            .fg(self.accent_alt)
            .add_modifier(Modifier::UNDERLINED)
    }

    /// A stable colour for a tag, so `draft` is the same colour on every row.
    pub fn tag_color(&self, tag: &str) -> Color {
        // FNV-1a rather than a byte sum: a sum gives every anagram the same
        // colour, which is how `draft` and `tfard` came to match.
        let mut hash: u32 = 0x811c_9dc5;
        for byte in tag.bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        self.tags[(hash % self.tags.len() as u32) as usize]
    }
}

/// ASCII glyphs when the terminal is unlikely to have the Unicode ones: the
/// legacy Windows console, or an explicit request.
fn ascii_wanted() -> bool {
    if std::env::var_os("FASTF_ASCII").is_some_and(|v| v == "1") {
        return true;
    }
    cfg!(windows) && std::env::var_os("WT_SESSION").is_none()
}

#[cfg(test)]
mod tests {
    use super::Theme;

    #[test]
    fn a_tag_colour_is_stable_and_anagrams_differ() {
        let theme = Theme::ansi();
        assert_eq!(theme.tag_color("draft"), theme.tag_color("draft"));
        // Not a guarantee in general — six colours cannot separate every pair —
        // but the one pair a byte sum gets wrong by construction.
        assert_ne!(theme.tag_color("draft"), theme.tag_color("tfard"));
    }
}
