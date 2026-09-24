//! A theme that paints: Doom One's canvas reaches every cell of the frame, a
//! dialog sits on its own surface, and nothing is left on the terminal's own
//! background — the `Clear` under a dialog included.

use crate::harness::*;
use fastf::tui::motion::{self, Motion};
use fastf::tui::testing::render_to_buffer;
use ratatui::style::Color;

#[test]
fn doom_one_paints_every_cell_and_a_dialog_wears_its_surface() {
    let mut app = fixture(12, 100, 30);
    app.theme = Theme::doom_one();
    let _ = update(&mut app, Msg::Key(Key::ch('c')));
    let frame = render_to_buffer(&app, 100, 30);

    let unpainted: Vec<(u16, u16)> = frame
        .content
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.bg == Color::Reset || cell.fg == Color::Reset)
        .map(|(i, _)| ((i % 100) as u16, (i / 100) as u16))
        .collect();
    assert!(
        unpainted.is_empty(),
        "cells left on the terminal's own colours: {unpainted:?}"
    );
    // The corner is the canvas; the palette's middle is the dialog's surface.
    assert_eq!(frame[(0, 0)].bg, app.theme.canvas);
    assert_eq!(frame[(50, 15)].bg, app.theme.surface);
}

#[test]
fn a_palette_without_a_canvas_leaves_the_terminal_alone() {
    let mut app = fixture(12, 100, 30);
    let _ = update(&mut app, Msg::Key(Key::ch('c')));
    for theme in [Theme::mono(), Theme::ansi(), Theme::rich()] {
        app.theme = theme;
        let frame = render_to_buffer(&app, 100, 30);
        assert_eq!(frame[(0, 0)].bg, Color::Reset, "{:?}", app.theme.kind);
        assert_eq!(frame[(50, 15)].bg, Color::Reset, "{:?}", app.theme.kind);
    }
}

/// Doom One is a 24-bit palette, so a wash fades along a ramp to the canvas
/// rather than being held and let go as on the sixteen colours.
#[test]
fn a_doom_one_wash_fades_to_its_canvas() {
    let theme = Theme::doom_one();
    let bg = |phase: f32| motion::wash(&theme, phase, Motion::On).and_then(|style| style.bg);
    assert_eq!(bg(0.0), Some(theme.pulse));
    let halfway = bg(0.5).expect("still washing halfway");
    assert!(
        halfway != theme.pulse && halfway != theme.ground,
        "{halfway:?}"
    );
    assert!(
        matches!(bg(0.99), Some(Color::Rgb(..))),
        "a fade, not a held wash"
    );
}
