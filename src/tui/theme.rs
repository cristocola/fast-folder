//! Colours and glyphs, chosen once from the environment.
//!
//! The prototype hard-coded one truecolor selection blue and a handful of
//! Unicode glyphs; on a 16-colour terminal the blue rendered as nothing and on
//! a legacy Windows console the glyphs rendered as boxes. Here the palette is
//! semantic — *accent*, *dim*, *good*, *bad* — and the environment decides what
//! each one is: `NO_COLOR` or a dumb terminal gets none, a terminal that
//! announces truecolor gets the rich version, everything else gets the sixteen
//! ANSI colours. Snapshot tests use `Theme::mono` so their frames never depend
//! on the machine that rendered them.

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

    /// The sixteen ANSI colours.
    pub fn ansi() -> Self {
        Self {
            kind: ThemeKind::Ansi,
            glyphs: Glyphs::unicode(),
            accent: Color::Cyan,
            accent_alt: Color::Magenta,
            text: Color::Reset,
            dim: Color::DarkGray,
            good: Color::Green,
            bad: Color::Red,
            warn: Color::Yellow,
            border: Color::DarkGray,
            border_focus: Color::Cyan,
            mark: Color::Yellow,
            selection: Style::default()
                .add_modifier(Modifier::REVERSED)
                .add_modifier(Modifier::BOLD),
            tags: [
                Color::Green,
                Color::Yellow,
                Color::Magenta,
                Color::Cyan,
                Color::Blue,
                Color::LightRed,
            ],
        }
    }

    /// Truecolor accents on top of the ANSI palette — the prototype's look.
    pub fn rich() -> Self {
        Self {
            kind: ThemeKind::Rich,
            selection: Style::default()
                .bg(Color::Rgb(24, 52, 88))
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
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

    /// The fuzzy-match highlight inside a row.
    pub fn hit(&self) -> Style {
        Style::default()
            .fg(self.accent_alt)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
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
