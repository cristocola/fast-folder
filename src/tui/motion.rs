//! Movement that guides the eye, and nothing that does not.
//!
//! The app draws a command centre, so motion here is held to the same rule as
//! colour: it appears where it *means* something and never as decoration.
//! Every movement points at the one thing on screen that just changed, and
//! then lets go:
//!
//! - a **pulse** on a row a verb just changed — *which* rows did that batch
//!   touch, when the cursor is somewhere else — and on a pane row an edit
//!   just landed on;
//! - a **pulse** on a size cell whose number *changed* — is the figure the one
//!   that was there a moment ago (never on a first fill: a page of sizes
//!   arrives at once, and lighting every visible row together is a flash);
//! - a **pulse** on the selected row after a sort or a filter reordered the
//!   list — the row kept the selection, so where did it go;
//! - the **focus easing** from one pane to the other — border and title move
//!   between their two rest colours rather than swapping, so the move is seen
//!   where it landed and not only inferred from a line that changed colour
//!   while nobody was reading it;
//! - a **wash** under a status message as it arrives — that line is where
//!   what just happened is said — and a **fade** as it goes, so it reads as
//!   going and can still be read;
//! - one **activity indicator** wherever something is pending — is it working,
//!   or is it stuck (that one is `Glyphs::spin`, and older than this module).
//!
//! **A pulse fades; it does not flash.** It was one step — the wash held for
//! 450 ms and then gone — and a background that snaps on and off is what a
//! terminal looks like when it glitches, which is how it was described. A
//! terminal cell has no alpha, so a fade needs something to fade *toward*:
//! the rich palette declares its `ground`, the dark it is drawn on, and a
//! pulse mixes from the wash to the ground with an ease-out, spending most
//! of its time near the ground so the last step to nothing is not seen. The
//! sixteen ANSI colours have no ramp, so there the wash is held and let go;
//! mono has no colour at all, and never moves.
//!
//! Deliberately not built: eased scrolling, dialog transitions, cursor trails,
//! a reveal sweep as a pane fills in. They answer nothing — and the sweep
//! fights a held arrow key.
//!
//! **This module is pure.** No clock, no environment, no I/O: every function
//! takes the elapsed milliseconds it should reason about, which is what lets
//! `update` start a pulse and a test assert on the frame it produces at a
//! millisecond it chooses. The clock itself is `App.elapsed_ms`, stamped on
//! every message by the one place that owns a clock, `runtime`.

use std::path::PathBuf;

use ratatui::style::{Color, Modifier, Style};

use crate::tui::theme::{Theme, ThemeKind};

/// How long a pulse takes to fade out. Long enough to catch an eye that was
/// looking elsewhere, short enough that a batch of ten does not leave the
/// table shimmering.
pub const PULSE_MS: u64 = 600;

/// How long the focus takes to ease from one pane to the other.
pub const FOCUS_MS: u64 = 250;

/// How long the wash under an arriving status message takes to settle.
pub const ARRIVE_MS: u64 = 500;

/// How long before a status message expires it begins to dim.
pub const FADE_MS: u64 = 500;

/// How often a live animation wants the screen redrawn. Not a frame rate the
/// terminal has to keep up with — the app draws when it has something to say —
/// but the interval at which a fade has visibly moved.
pub const FRAME_MS: u64 = 50;

/// How much of a pulse the sixteen-colour wash is held for before it is let
/// go: there is no ramp to fade along, so it holds while an eye would still
/// be finding the row, and lets go before it reads as a state.
const ANSI_HOLD: f32 = 0.6;

/// Whether the app moves at all.
///
/// A pulse is a colour wash, so on a theme with no colour it is a flicker
/// rather than a cue: `Mono` is `Off` whatever the setting says, for the same
/// reason `NO_COLOR` chose `Mono` in the first place.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Motion {
    #[default]
    On,
    Off,
}

impl Motion {
    /// What the config key spells.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "on" | "true" | "1" => Some(Motion::On),
            "off" | "false" | "0" => Some(Motion::Off),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Motion::On => "on",
            Motion::Off => "off",
        }
    }

    pub fn is_on(self) -> bool {
        self == Motion::On
    }
}

/// How far through an animation `now` is: `0.0` at the start, `1.0` at the
/// end, and `None` once it is over — which is what lets a caller drop it.
///
/// `started` in the future answers `Some(0.0)` rather than panicking: the
/// clock is monotone, but a fixture's is whatever the test set it to.
pub fn phase(started: u64, now: u64, duration: u64) -> Option<f32> {
    if duration == 0 {
        return None;
    }
    let elapsed = now.saturating_sub(started);
    if elapsed >= duration {
        return None;
    }
    Some(elapsed as f32 / duration as f32)
}

/// Fast out of the gate, slow to settle: `1 - (1 - t)³`. What every fade
/// here follows, so a change is seen at once and lets go gently.
pub fn ease_out(t: f32) -> f32 {
    let left = 1.0 - t.clamp(0.0, 1.0);
    1.0 - left * left * left
}

/// The colour `t` of the way from `from` to `to`, for two RGB colours. Any
/// other pair — a named ANSI colour, `Reset` — has no ramp between them, and
/// answers `from` unchanged.
pub fn mix(from: Color, to: Color, t: f32) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let t = t.clamp(0.0, 1.0);
            let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            Color::Rgb(lerp(r1, r2), lerp(g1, g2), lerp(b1, b2))
        }
        _ => from,
    }
}

/// The wash a pulsing row wears `phase` of the way through its pulse.
///
/// **A background, and one that fades.** Every cell in a row sets its own
/// foreground, so a foreground set on the row loses to all of them; a
/// background is the one thing the cells leave alone. On the rich palette it
/// mixes from `theme.pulse` toward `theme.ground` with an ease-out; on the
/// sixteen colours it is held for `ANSI_HOLD` of the pulse and then let go,
/// because there is nothing to fade along; under `Motion::Off` and in mono
/// there is no wash at all.
pub fn wash(theme: &Theme, phase: f32, motion: Motion) -> Option<Style> {
    if !motion.is_on() || theme.kind == ThemeKind::Mono {
        return None;
    }
    match theme.kind {
        ThemeKind::Rich => {
            Some(Style::default().bg(mix(theme.pulse, theme.ground, ease_out(phase))))
        }
        _ => (phase < ANSI_HOLD).then(|| Style::default().bg(theme.pulse)),
    }
}

/// One thing that changed, and when. `K` is whatever names it — a row's path
/// in the table, a row's index in the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pulse<K = PathBuf> {
    pub key: K,
    pub started: u64,
}

/// Every pulse in flight. A `Vec` because there are as many as a batch just
/// touched — five, ten — and never enough to want a map.
///
/// Generic over the key because the table's rows are named by path and the
/// pane's by index, and the arithmetic — start or restart, retire, the wash —
/// is the same for both. Two structs would have been two copies of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pulses<K = PathBuf> {
    live: Vec<Pulse<K>>,
}

impl<K> Default for Pulses<K> {
    fn default() -> Self {
        Self { live: Vec::new() }
    }
}

impl<K: PartialEq> Pulses<K> {
    /// Start one, or restart the one that is already on this row: a row
    /// touched twice in a batch should pulse from now, not carry on fading.
    pub fn start(&mut self, key: K, now: u64) {
        if let Some(pulse) = self.live.iter_mut().find(|p| p.key == key) {
            pulse.started = now;
            return;
        }
        self.live.push(Pulse { key, started: now });
    }

    /// Drop what has finished. Called on the tick, so the list cannot grow
    /// with a session.
    pub fn retire(&mut self, now: u64) {
        self.live
            .retain(|pulse| phase(pulse.started, now, PULSE_MS).is_some());
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    pub fn clear(&mut self) {
        self.live.clear();
    }

    /// How far through its pulse this row is, if it is in one.
    fn at<Q>(&self, key: &Q, now: u64) -> Option<f32>
    where
        K: PartialEq<Q>,
        Q: ?Sized,
    {
        let pulse = self.live.iter().find(|p| p.key == *key)?;
        phase(pulse.started, now, PULSE_MS)
    }

    /// The wash this row wears at `now`, if it is pulsing — see [`wash`].
    pub fn style_for<Q>(&self, key: &Q, now: u64, theme: &Theme, motion: Motion) -> Option<Style>
    where
        K: PartialEq<Q>,
        Q: ?Sized,
    {
        wash(theme, self.at(key, now)?, motion)
    }
}

/// A colour easing between its two rest states as the focus moves: `lit` for
/// the pane that has the focus, `rest` for the one that does not. From
/// `moved_at` the newly focused pane goes `rest → lit` over `FOCUS_MS` and
/// the pane it left goes the other way at the same moment; at rest, and on a
/// palette with no ramp or with motion off, the colour is simply the one the
/// state calls for — a transition between two rest states has no end step
/// to snap through.
fn eased(
    theme: &Theme,
    rest: Color,
    lit: Color,
    focused: bool,
    moved_at: Option<u64>,
    now: u64,
    motion: Motion,
) -> Color {
    let (from, to) = if focused { (rest, lit) } else { (lit, rest) };
    if !motion.is_on() || theme.kind != ThemeKind::Rich {
        return to;
    }
    match moved_at.and_then(|at| phase(at, now, FOCUS_MS)) {
        Some(phase) => mix(from, to, ease_out(phase)),
        None => to,
    }
}

/// A pane's border: `border_focus` while it has the focus, `border` while it
/// does not, easing between the two for `FOCUS_MS` after the focus moved.
pub fn border_style(
    theme: &Theme,
    focused: bool,
    moved_at: Option<u64>,
    now: u64,
    motion: Motion,
) -> Style {
    Style::default().fg(eased(
        theme,
        theme.border,
        theme.border_focus,
        focused,
        moved_at,
        now,
        motion,
    ))
}

/// A pane's title: accent and bold while it has the focus, dim while it does
/// not, the colour easing between the two as the border's does.
pub fn title_style(
    theme: &Theme,
    focused: bool,
    moved_at: Option<u64>,
    now: u64,
    motion: Motion,
) -> Style {
    let style = Style::default().fg(eased(
        theme,
        theme.dim,
        theme.accent,
        focused,
        moved_at,
        now,
        motion,
    ));
    if focused {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// Whether the focus that moved at `moved_at` is still easing at `now`.
pub fn focus_easing(moved_at: Option<u64>, now: u64) -> bool {
    moved_at.is_some_and(|at| phase(at, now, FOCUS_MS).is_some())
}

/// The wash under a status message that arrived at `shown_at`: the same
/// pulse a row wears, over `ARRIVE_MS`, so the eye is drawn to the line where
/// what just happened is said. `None` once it has settled.
pub fn arriving_style(
    shown_at: Option<u64>,
    now: u64,
    theme: &Theme,
    motion: Motion,
) -> Option<Style> {
    wash(theme, phase(shown_at?, now, ARRIVE_MS)?, motion)
}

/// Whether a message shown at `shown_at` is still arriving at `now`.
pub fn arriving(shown_at: Option<u64>, now: u64) -> bool {
    shown_at.is_some_and(|at| phase(at, now, ARRIVE_MS).is_some())
}

/// How dim a status message is on its way out: `None` while it is simply
/// there, `Some(style)` once it has begun to go.
pub fn expiring_style(
    expires_at: Option<u64>,
    now: u64,
    theme: &Theme,
    motion: Motion,
) -> Option<Style> {
    if !motion.is_on() || theme.kind == ThemeKind::Mono {
        return None;
    }
    let left = expires_at?.saturating_sub(now);
    (left < FADE_MS).then(|| theme.dim().add_modifier(Modifier::DIM))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_runs_from_nothing_to_gone() {
        assert_eq!(phase(0, 0, 100), Some(0.0));
        assert_eq!(phase(0, 50, 100), Some(0.5));
        assert_eq!(phase(0, 100, 100), None, "over at its duration, not after");
        assert_eq!(phase(0, 999, 100), None);
        assert_eq!(phase(0, 0, 0), None, "an animation of no length is over");
        // A fixture's clock is whatever the test set it to.
        assert_eq!(phase(500, 0, 100), Some(0.0));
    }

    #[test]
    fn an_ease_out_starts_fast_and_settles_slowly() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.8, "most of the way there by the middle");
        assert!(ease_out(0.25) > 0.5);
        assert_eq!(ease_out(2.0), 1.0, "clamped");
    }

    #[test]
    fn a_mix_walks_between_two_rgb_colours_and_leaves_the_rest_alone() {
        let from = Color::Rgb(0, 100, 200);
        let to = Color::Rgb(100, 0, 0);
        assert_eq!(mix(from, to, 0.0), from);
        assert_eq!(mix(from, to, 1.0), to);
        assert_eq!(mix(from, to, 0.5), Color::Rgb(50, 50, 100));
        assert_eq!(
            mix(Color::Blue, to, 0.5),
            Color::Blue,
            "no ramp from a named colour"
        );
        assert_eq!(mix(from, Color::Reset, 0.5), from, "nor toward one");
    }

    #[test]
    fn a_pulse_fades_from_the_wash_to_the_ground_and_lets_go() {
        let rich = Theme::rich();
        let at = |phase: f32| wash(&rich, phase, Motion::On).and_then(|s| s.bg);
        assert_eq!(at(0.0), Some(rich.pulse), "lit at once");
        let mid = at(0.5).unwrap();
        assert_ne!(mid, rich.pulse);
        assert_ne!(mid, rich.ground);
        assert_eq!(
            wash(&rich, 0.0, Motion::On).unwrap().fg,
            None,
            "the row's own colours are left alone"
        );
        // Most of the way to the ground by the middle: the last step to
        // nothing is small.
        let Color::Rgb(_, _, b) = mid else {
            panic!("rgb")
        };
        let (Color::Rgb(_, _, b_pulse), Color::Rgb(_, _, b_ground)) = (rich.pulse, rich.ground)
        else {
            panic!("rgb")
        };
        assert!((b as i32 - b_ground as i32).abs() < (b as i32 - b_pulse as i32).abs());
    }

    #[test]
    fn the_sixteen_colours_hold_the_wash_and_let_go() {
        let ansi = Theme::ansi();
        assert_eq!(
            wash(&ansi, 0.0, Motion::On).and_then(|s| s.bg),
            Some(ansi.pulse)
        );
        assert!(wash(&ansi, ANSI_HOLD - 0.01, Motion::On).is_some());
        assert!(
            wash(&ansi, ANSI_HOLD, Motion::On).is_none(),
            "let go before it reads as a state"
        );
    }

    #[test]
    fn nothing_moves_without_colour_or_with_motion_off() {
        let mut pulses = Pulses::default();
        let row = PathBuf::from("/mnt/projects/one");
        pulses.start(row.clone(), 0);
        let rich = Theme::rich();
        assert!(pulses.style_for(&row, 0, &rich, Motion::On).is_some());
        assert!(
            pulses
                .style_for(&row, PULSE_MS, &rich, Motion::On)
                .is_none(),
            "over at its duration"
        );
        assert!(
            pulses.style_for(&row, 0, &rich, Motion::Off).is_none(),
            "the setting is the setting"
        );
        assert!(
            pulses
                .style_for(&row, 0, &Theme::mono(), Motion::On)
                .is_none(),
            "a colour wash on a theme with no colour is a flicker, not a cue"
        );
        assert!(wash(&Theme::mono(), 0.0, Motion::On).is_none());
    }

    #[test]
    fn a_pulse_lights_up_retires_and_does_not_stack() {
        let mut pulses = Pulses::default();
        let row = PathBuf::from("/mnt/projects/one");
        pulses.start(row.clone(), 1_000);
        assert!(pulses.at(&row, 1_000).is_some());
        assert!(pulses.at(&row, 1_000 + PULSE_MS).is_none());

        // Touched again: it pulses from now rather than carrying on fading.
        pulses.start(row.clone(), 1_400);
        assert_eq!(pulses.live.len(), 1);
        assert!(pulses.at(&row, 1_500).is_some());

        pulses.retire(9_999);
        assert!(pulses.is_empty(), "nothing in flight outlives its duration");
    }

    #[test]
    fn the_focus_eases_between_its_rest_states_and_snaps_where_it_cannot() {
        let rich = Theme::rich();
        let fg = |focused, moved_at, now| {
            border_style(&rich, focused, moved_at, now, Motion::On)
                .fg
                .unwrap()
        };
        // At rest: the colour the state calls for.
        assert_eq!(fg(true, None, 0), rich.border_focus);
        assert_eq!(fg(false, None, 0), rich.border);
        // The moment of the move: still the colour it had.
        assert_eq!(fg(true, Some(100), 100), rich.border);
        assert_eq!(fg(false, Some(100), 100), rich.border_focus);
        // Half way: between; done: at rest.
        let mid = fg(true, Some(100), 100 + FOCUS_MS / 2);
        assert_ne!(mid, rich.border);
        assert_ne!(mid, rich.border_focus);
        assert_eq!(fg(true, Some(100), 100 + FOCUS_MS), rich.border_focus);
        assert!(focus_easing(Some(100), 100 + FOCUS_MS - 1));
        assert!(!focus_easing(Some(100), 100 + FOCUS_MS));
        // The title: dim to accent, bold only once focused.
        let title = title_style(&rich, true, Some(100), 100, Motion::On);
        assert_eq!(title.fg, Some(rich.dim));
        assert!(title.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            title_style(&rich, true, Some(100), 100 + FOCUS_MS, Motion::On).fg,
            Some(rich.accent)
        );
        // Off, and the sixteen colours: the rest state at once — a
        // transition has no end step to snap through.
        assert_eq!(
            border_style(&rich, true, Some(100), 100, Motion::Off).fg,
            Some(rich.border_focus)
        );
        let ansi = Theme::ansi();
        assert_eq!(
            border_style(&ansi, true, Some(100), 100, Motion::On).fg,
            Some(ansi.border_focus)
        );
    }

    #[test]
    fn a_message_arrives_with_a_wash_and_dims_only_on_its_way_out() {
        let rich = Theme::rich();
        assert_eq!(
            arriving_style(Some(1_000), 1_000, &rich, Motion::On).and_then(|s| s.bg),
            Some(rich.pulse)
        );
        assert!(arriving_style(Some(1_000), 1_000 + ARRIVE_MS, &rich, Motion::On).is_none());
        assert!(arriving_style(None, 1_000, &rich, Motion::On).is_none());
        assert!(arriving_style(Some(1_000), 1_000, &rich, Motion::Off).is_none());
        assert!(arriving(Some(1_000), 1_000 + ARRIVE_MS - 1));
        assert!(!arriving(Some(1_000), 1_000 + ARRIVE_MS));

        assert!(expiring_style(Some(10_000), 0, &rich, Motion::On).is_none());
        assert!(expiring_style(Some(10_000), 9_600, &rich, Motion::On).is_some());
        assert!(expiring_style(None, 9_600, &rich, Motion::On).is_none());
        assert!(expiring_style(Some(10_000), 9_600, &rich, Motion::Off).is_none());
    }

    #[test]
    fn the_setting_reads_what_it_writes() {
        assert_eq!(Motion::parse("on"), Some(Motion::On));
        assert_eq!(Motion::parse(" off "), Some(Motion::Off));
        assert_eq!(Motion::parse("sideways"), None);
        assert_eq!(Motion::parse(Motion::On.name()), Some(Motion::On));
        assert_eq!(Motion::parse(Motion::Off.name()), Some(Motion::Off));
    }
}
