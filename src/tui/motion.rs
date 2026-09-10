//! Movement that answers a question, and nothing that does not.
//!
//! The app draws a command centre, so motion here is held to the same rule as
//! colour: it appears where it *means* something and never as decoration.
//! Four things move, and each one answers a question a still frame could not:
//!
//! - a **pulse** on a row a verb just changed — *which* rows did that batch
//!   touch, when the cursor is somewhere else;
//! - a **pulse** on a size cell as its number lands — is that number new, or
//!   did the table reflow;
//! - one **activity indicator** wherever something is pending — is it working,
//!   or is it stuck (that one is `Glyphs::spin`, and older than this module);
//! - a **fade** on a status message about to expire — it is going, and you can
//!   still read it.
//!
//! Deliberately not built: eased scrolling, dialog transitions, cursor trails.
//! They answer nothing.
//!
//! **This module is pure.** No clock, no environment, no I/O: every function
//! takes the elapsed milliseconds it should reason about, which is what lets
//! `update` start a pulse and a test assert on the frame it produces at a
//! millisecond it chooses. The clock itself is `App.elapsed_ms`, fed by
//! `Msg::Tick` from the one place that owns a clock, `runtime`.

use std::path::{Path, PathBuf};

use ratatui::style::{Modifier, Style};

use crate::tui::theme::{Theme, ThemeKind};

/// How long a pulse takes to fade out. Long enough to catch an eye that was
/// looking elsewhere, short enough that a batch of ten does not leave the
/// table shimmering.
pub const PULSE_MS: u64 = 450;

/// How long before a status message expires it begins to dim.
pub const FADE_MS: u64 = 500;

/// How often a live animation wants the screen redrawn. Not a frame rate the
/// terminal has to keep up with — the app draws when it has something to say —
/// but the interval at which a fade has visibly moved.
pub const FRAME_MS: u64 = 50;

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

/// One row that changed, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pulse {
    pub path: PathBuf,
    pub started: u64,
}

/// Every pulse in flight. A `Vec` because there are as many as a batch just
/// touched — five, ten — and never enough to want a map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pulses {
    live: Vec<Pulse>,
}

impl Pulses {
    /// Start one, or restart the one that is already on this row: a row
    /// touched twice in a batch should pulse from now, not carry on fading.
    pub fn start(&mut self, path: PathBuf, now: u64) {
        if let Some(pulse) = self.live.iter_mut().find(|p| p.path == path) {
            pulse.started = now;
            return;
        }
        self.live.push(Pulse { path, started: now });
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
    fn at(&self, path: &Path, now: u64) -> Option<f32> {
        let pulse = self.live.iter().find(|p| p.path == path)?;
        phase(pulse.started, now, PULSE_MS)
    }

    /// The wash a pulsing row wears.
    ///
    /// **A background, and one step.** Every cell in a row sets its own
    /// foreground, so a foreground set on the row loses to all of them; a
    /// background is the one thing the cells leave alone. And one step,
    /// because a terminal cell has no alpha and this theme defines no page
    /// background — `Color::Reset` has no RGB — so there is nothing to
    /// interpolate *towards*. What a terminal can do honestly is hold the row
    /// lit for as long as an eye needs to find it and then let go, which at
    /// `PULSE_MS` reads as a pulse rather than a state.
    pub fn style_for(&self, path: &Path, now: u64, theme: &Theme, motion: Motion) -> Option<Style> {
        if !motion.is_on() || theme.kind == ThemeKind::Mono {
            return None;
        }
        self.at(path, now)?;
        Some(Style::default().bg(theme.pulse))
    }
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
    fn nothing_moves_without_colour_or_with_motion_off() {
        let mut pulses = Pulses::default();
        let row = PathBuf::from("/mnt/projects/one");
        pulses.start(row.clone(), 0);

        let rich = Theme::rich();
        // A background, because the cells own their foregrounds.
        let lit = pulses.style_for(&row, 0, &rich, Motion::On).unwrap();
        assert_eq!(lit.bg, Some(rich.pulse));
        assert_eq!(lit.fg, None, "the row's own colours are left alone");
        assert!(
            pulses
                .style_for(&row, PULSE_MS / 2, &rich, Motion::On)
                .is_some(),
            "held for the whole of its duration"
        );
        assert!(
            pulses
                .style_for(&row, PULSE_MS, &rich, Motion::On)
                .is_none()
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
    }

    #[test]
    fn a_message_dims_only_on_its_way_out() {
        let rich = Theme::rich();
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
