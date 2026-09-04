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
//! everything else gets the sixteen ANSI colours used sparingly. The choice is
//! a pure function of an [`Env`] (`choose`), so it is tested without touching
//! the process environment; `FASTF_THEME` and the `theme` config key override
//! it for the terminals that announce nothing — an ssh session forwards no
//! `COLORTERM`, and an old machine may be lying either way. Snapshot tests use
//! `Theme::mono` so their frames never depend on the machine that rendered
//! them.

use ratatui::style::{Color, Modifier, Style};

/// Which palette was chosen, mostly for `fastf paths`-style diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    Mono,
    Ansi,
    Rich,
}

impl ThemeKind {
    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Mono => "mono",
            ThemeKind::Ansi => "ansi",
            ThemeKind::Rich => "rich",
        }
    }
}

/// A theme preference, as the `theme` config key or `FASTF_THEME` spell it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Follow the terminal.
    #[default]
    Auto,
    Kind(ThemeKind),
}

impl ThemeChoice {
    /// The spellings `config set theme` accepts, in the order the settings
    /// screen cycles them.
    pub const NAMES: [&'static str; 4] = ["auto", "mono", "ansi", "rich"];

    /// `None` for anything that is not one of the four; the empty string is
    /// `auto`, which is what an unset config key reads as.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Some(ThemeChoice::Auto),
            "mono" => Some(ThemeChoice::Kind(ThemeKind::Mono)),
            "ansi" => Some(ThemeChoice::Kind(ThemeKind::Ansi)),
            "rich" => Some(ThemeChoice::Kind(ThemeKind::Rich)),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ThemeChoice::Auto => "auto",
            ThemeChoice::Kind(kind) => kind.name(),
        }
    }
}

/// What the environment says about the terminal, read once and then reasoned
/// about without it — so the reasoning has tests and the reading is one place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// `NO_COLOR` set to anything but the empty string.
    pub no_color: bool,
    pub term: String,
    pub colorterm: String,
    pub term_program: String,
    /// Windows Terminal announces itself with `WT_SESSION`.
    pub wt_session: bool,
    pub fastf_theme: Option<String>,
    pub fastf_ascii: Option<String>,
    /// Alacritty, WezTerm and ConEmu each leave a variable of their own.
    pub alacritty: bool,
    pub wezterm: bool,
    pub conemu: bool,
}

impl Env {
    pub fn read() -> Self {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        let set = |name: &str| std::env::var_os(name).is_some();
        Self {
            no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
            term: var("TERM"),
            colorterm: var("COLORTERM"),
            term_program: var("TERM_PROGRAM"),
            wt_session: set("WT_SESSION"),
            fastf_theme: std::env::var("FASTF_THEME").ok(),
            fastf_ascii: std::env::var("FASTF_ASCII").ok(),
            alacritty: set("ALACRITTY_WINDOW_ID") || set("ALACRITTY_SOCKET"),
            wezterm: set("WEZTERM_PANE") || set("WEZTERM_EXECUTABLE"),
            conemu: set("ConEmuANSI"),
        }
    }
}

/// Pick the palette and the alphabet. `preference` is the config's `theme`.
///
/// Precedence, highest first: `FASTF_THEME` (the per-session escape hatch);
/// `NO_COLOR` and `TERM=dumb` (a promise the user made to every program);
/// the config key; then what the terminal announces — `COLORTERM`, a `TERM`
/// that names a truecolor emulator, a `TERM_PROGRAM` known to be one, or
/// Windows Terminal — and the sixteen colours for everything else.
pub fn choose(env: &Env, preference: Option<&str>) -> (ThemeKind, Glyphs) {
    let glyphs = if ascii_wanted(env) {
        Glyphs::ascii()
    } else {
        Glyphs::unicode()
    };
    let forced = env
        .fastf_theme
        .as_deref()
        .and_then(ThemeChoice::parse)
        .unwrap_or_default();
    if let ThemeChoice::Kind(kind) = forced {
        return (kind, glyphs);
    }
    if env.no_color || env.term.trim().eq_ignore_ascii_case("dumb") {
        return (ThemeKind::Mono, glyphs);
    }
    if let Some(ThemeChoice::Kind(kind)) = preference.and_then(ThemeChoice::parse) {
        return (kind, glyphs);
    }
    if announces_truecolor(env) {
        (ThemeKind::Rich, glyphs)
    } else {
        (ThemeKind::Ansi, glyphs)
    }
}

/// Whether the terminal said, one way or another, that it draws 24-bit colour.
/// `COLORTERM` is the convention, but ssh does not forward it, so the emulators
/// that name themselves in `TERM` or `TERM_PROGRAM` are recognised too.
fn announces_truecolor(env: &Env) -> bool {
    let colorterm = env.colorterm.trim().to_ascii_lowercase();
    if colorterm == "truecolor" || colorterm == "24bit" {
        return true;
    }
    let term = env.term.to_ascii_lowercase();
    const TERMS: [&str; 7] = [
        "direct",
        "truecolor",
        "kitty",
        "alacritty",
        "wezterm",
        "foot",
        "ghostty",
    ];
    if TERMS.iter().any(|name| term.contains(name)) {
        return true;
    }
    let program = env.term_program.trim().to_ascii_lowercase();
    const PROGRAMS: [&str; 6] = [
        "wezterm",
        "iterm.app",
        "vscode",
        "ghostty",
        "hyper",
        "tabby",
    ];
    if PROGRAMS.contains(&program.as_str()) {
        return true;
    }
    env.wt_session || env.alacritty || env.wezterm
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
    /// A progress bar's filled and empty cell. Ours rather than ratatui's
    /// `Gauge`, whose glyphs and colour model are its own: the palette here is
    /// a pure function of an `Env`, and the ASCII path has to stay right on a
    /// terminal that draws no block elements.
    pub bar_full: &'static str,
    pub bar_empty: &'static str,
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
            bar_full: "█",
            bar_empty: "░",
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
            bar_full: "#",
            bar_empty: "-",
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
    /// Read the environment and pick a palette, with no configured preference.
    pub fn detect() -> Self {
        Self::detect_with(None)
    }

    /// Read the environment and pick a palette, honouring the config's
    /// `theme` key where the environment leaves the choice open — see
    /// [`choose`] for the precedence.
    pub fn detect_with(preference: Option<&str>) -> Self {
        let env = Env::read();
        let (kind, glyphs) = choose(&env, preference);
        Self::from_kind(kind).with_glyphs(glyphs)
    }

    pub fn from_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Mono => Self::mono(),
            ThemeKind::Ansi => Self::ansi(),
            ThemeKind::Rich => Self::rich(),
        }
    }

    /// The same palette with a different alphabet — what the conhost check and
    /// the ASCII snapshot both go through.
    pub fn with_glyphs(mut self, glyphs: Glyphs) -> Self {
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
/// legacy Windows console, or an explicit request. `FASTF_ASCII=1` asks for
/// them anywhere and `FASTF_ASCII=0` refuses them anywhere; without it, only a
/// Windows host that is none of the emulators known to draw Unicode — Windows
/// Terminal, Alacritty, WezTerm, ConEmu, anything that sets `TERM_PROGRAM` or
/// `TERM` — is taken for the legacy console.
pub fn ascii_wanted(env: &Env) -> bool {
    match env
        .fastf_ascii
        .as_deref()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("1" | "true" | "yes" | "on") => return true,
        Some("0" | "false" | "no" | "off") => return false,
        _ => {}
    }
    cfg!(windows)
        && !env.wt_session
        && env.term_program.trim().is_empty()
        && env.term.trim().is_empty()
        && !env.alacritty
        && !env.wezterm
        && !env.conemu
}

#[cfg(test)]
mod tests {
    use super::{Env, Glyphs, Theme, ThemeChoice, ThemeKind, ascii_wanted, choose};

    #[test]
    fn a_tag_colour_is_stable_and_anagrams_differ() {
        let theme = Theme::ansi();
        assert_eq!(theme.tag_color("draft"), theme.tag_color("draft"));
        // Not a guarantee in general — six colours cannot separate every pair —
        // but the one pair a byte sum gets wrong by construction.
        assert_ne!(theme.tag_color("draft"), theme.tag_color("tfard"));
    }

    fn env(term: &str, colorterm: &str) -> Env {
        Env {
            term: term.to_string(),
            colorterm: colorterm.to_string(),
            ..Env::default()
        }
    }

    #[test]
    fn colorterm_is_the_convention_and_the_default_is_ansi() {
        assert_eq!(choose(&env("xterm-256color", ""), None).0, ThemeKind::Ansi);
        assert_eq!(
            choose(&env("xterm-256color", "truecolor"), None).0,
            ThemeKind::Rich
        );
        assert_eq!(
            choose(&env("xterm-256color", "24bit"), None).0,
            ThemeKind::Rich
        );
    }

    #[test]
    fn an_emulator_that_names_itself_is_truecolor_without_colorterm() {
        // What an ssh session looks like: TERM forwarded, COLORTERM not.
        assert_eq!(choose(&env("xterm-kitty", ""), None).0, ThemeKind::Rich);
        assert_eq!(choose(&env("foot", ""), None).0, ThemeKind::Rich);
        assert_eq!(choose(&env("xterm-direct", ""), None).0, ThemeKind::Rich);
        let vscode = Env {
            term_program: "vscode".to_string(),
            ..env("xterm-256color", "")
        };
        assert_eq!(choose(&vscode, None).0, ThemeKind::Rich);
        let windows_terminal = Env {
            wt_session: true,
            ..Env::default()
        };
        assert_eq!(choose(&windows_terminal, None).0, ThemeKind::Rich);
    }

    #[test]
    fn no_color_and_a_dumb_terminal_are_mono_whatever_the_config_says() {
        let no_color = Env {
            no_color: true,
            ..env("xterm-kitty", "truecolor")
        };
        assert_eq!(choose(&no_color, Some("rich")).0, ThemeKind::Mono);
        assert_eq!(
            choose(&env("dumb", "truecolor"), Some("ansi")).0,
            ThemeKind::Mono
        );
    }

    #[test]
    fn the_config_decides_where_the_terminal_announces_nothing() {
        assert_eq!(
            choose(&env("xterm-256color", ""), Some("rich")).0,
            ThemeKind::Rich
        );
        assert_eq!(
            choose(&env("xterm-kitty", "truecolor"), Some("mono")).0,
            ThemeKind::Mono
        );
        // `auto`, the empty string and a typo all mean "follow the terminal".
        for pref in ["auto", "", "purple"] {
            assert_eq!(
                choose(&env("xterm-kitty", ""), Some(pref)).0,
                ThemeKind::Rich,
                "{pref:?}"
            );
        }
    }

    #[test]
    fn fastf_theme_is_the_escape_hatch_above_everything() {
        let forced = Env {
            no_color: true,
            fastf_theme: Some("rich".to_string()),
            ..env("dumb", "")
        };
        assert_eq!(choose(&forced, Some("mono")).0, ThemeKind::Rich);
        let typo = Env {
            fastf_theme: Some("purple".to_string()),
            ..env("xterm-256color", "")
        };
        assert_eq!(choose(&typo, Some("mono")).0, ThemeKind::Mono, "ignored");
    }

    #[test]
    fn the_four_names_parse_and_nothing_else_does() {
        assert_eq!(ThemeChoice::parse("Auto"), Some(ThemeChoice::Auto));
        assert_eq!(ThemeChoice::parse(""), Some(ThemeChoice::Auto));
        assert_eq!(
            ThemeChoice::parse(" rich "),
            Some(ThemeChoice::Kind(ThemeKind::Rich))
        );
        assert_eq!(ThemeChoice::parse("purple"), None);
        for name in ThemeChoice::NAMES {
            assert!(ThemeChoice::parse(name).is_some(), "{name}");
        }
    }

    #[test]
    fn the_alphabet_is_asked_for_or_refused_explicitly() {
        let ascii = Env {
            fastf_ascii: Some("1".to_string()),
            ..Env::default()
        };
        assert!(ascii_wanted(&ascii));
        assert_eq!(choose(&ascii, None).1, Glyphs::ascii());
        let unicode = Env {
            fastf_ascii: Some("0".to_string()),
            ..Env::default()
        };
        assert!(!ascii_wanted(&unicode));
        // Every emulator that announces itself draws Unicode, on any host.
        for known in [
            Env {
                wt_session: true,
                ..Env::default()
            },
            Env {
                term_program: "vscode".to_string(),
                ..Env::default()
            },
            Env {
                alacritty: true,
                ..Env::default()
            },
            Env {
                wezterm: true,
                ..Env::default()
            },
            Env {
                conemu: true,
                ..Env::default()
            },
            env("xterm-256color", ""),
        ] {
            assert!(!ascii_wanted(&known), "{known:?}");
        }
        // Nothing announced at all: the legacy console on Windows, an ordinary
        // terminal everywhere else.
        assert_eq!(ascii_wanted(&Env::default()), cfg!(windows));
    }

    #[test]
    fn detect_reads_the_process_environment() {
        let (mut guard, _dir) = crate::util::test_env::EnvGuard::sandbox();
        guard.also_remove("NO_COLOR");
        guard.also_set("TERM", "xterm-256color");
        guard.also_remove("COLORTERM");
        guard.also_remove("TERM_PROGRAM");
        guard.also_remove("FASTF_THEME");
        guard.also_remove("FASTF_ASCII");
        assert_eq!(Theme::detect().kind, ThemeKind::Ansi);
        assert_eq!(Theme::detect_with(Some("rich")).kind, ThemeKind::Rich);
        guard.also_set("FASTF_THEME", "mono");
        assert_eq!(Theme::detect_with(Some("rich")).kind, ThemeKind::Mono);
        guard.also_set("FASTF_ASCII", "1");
        assert_eq!(Theme::detect().glyphs, Glyphs::ascii());
    }
}
