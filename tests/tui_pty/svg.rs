//! The screenshot tool's SVG output: a `vt100` frame as a picture, cell by
//! cell, in the app's own truecolor palette — the README's screenshot, taken
//! from the real binary rather than drawn by hand.
//!
//! Only the sandbox library is ever rendered (`FASTF_SHOT_REAL=1` is refused
//! with `FASTF_SHOT_SVG`): the repository is public, and `repo_hygiene.rs`
//! does not read images.

use fastf::tui::theme::Theme;
use ratatui::style::Color as RColor;

const CELL_W: f32 = 8.4;
const CELL_H: f32 = 18.0;
const FONT_SIZE: u32 = 14;
const FG: &str = "#c9ced4";
const BG: &str = "#14181e";

/// One run of same-styled cells on a row.
struct Run {
    col: u16,
    text: String,
    fg: String,
    bg: Option<String>,
    bold: bool,
    underline: bool,
}

pub(crate) fn render(screen: &vt100::Screen, theme: &Theme) -> String {
    let (rows, cols) = screen.size();
    let width = f32::from(cols) * CELL_W + 16.0;
    let height = f32::from(rows) * CELL_H + 16.0;
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"ui-monospace, SFMono-Regular, Menlo, Consolas, 'DejaVu Sans Mono', monospace\" font-size=\"{FONT_SIZE}\">\n"
    ));
    out.push_str(&format!(
        "<rect width=\"100%\" height=\"100%\" rx=\"6\" fill=\"{BG}\"/>\n"
    ));
    for row in 0..rows {
        let mut runs: Vec<Run> = Vec::new();
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let text = cell.contents().to_string();
            let mut fg = color(cell.fgcolor(), theme, true).unwrap_or_else(|| FG.to_string());
            let mut bg = color(cell.bgcolor(), theme, false);
            if cell.inverse() {
                let swapped_fg = bg.clone().unwrap_or_else(|| BG.to_string());
                bg = Some(fg.clone());
                fg = swapped_fg;
            }
            let bold = cell.bold();
            let underline = cell.underline();
            match runs.last_mut() {
                Some(run)
                    if run.fg == fg
                        && run.bg == bg
                        && run.bold == bold
                        && run.underline == underline
                        && run.col + run.text.chars().count() as u16 == col =>
                {
                    run.text.push_str(if text.is_empty() { " " } else { &text });
                }
                _ => runs.push(Run {
                    col,
                    text: if text.is_empty() {
                        " ".to_string()
                    } else {
                        text
                    },
                    fg,
                    bg,
                    bold,
                    underline,
                }),
            }
        }
        let y = 8.0 + f32::from(row) * CELL_H;
        for run in &runs {
            let x = 8.0 + f32::from(run.col) * CELL_W;
            let w = run.text.chars().count() as f32 * CELL_W;
            if let Some(bg) = &run.bg {
                out.push_str(&format!(
                    "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{CELL_H:.1}\" fill=\"{bg}\"/>\n"
                ));
            }
            if run.text.trim().is_empty() {
                continue;
            }
            // Box-drawing characters as lines: a glyph does not span the row,
            // so a border drawn as text comes out dashed.
            if run.text.chars().all(|c| is_box_drawing(c) || c == ' ') {
                for (i, c) in run.text.chars().enumerate() {
                    let cx = x + i as f32 * CELL_W;
                    out.push_str(&box_lines(c, cx, y, &run.fg));
                }
                continue;
            }

            let mut attrs = format!("fill=\"{}\"", run.fg);
            if run.bold {
                attrs.push_str(" font-weight=\"bold\"");
            }
            if run.underline {
                attrs.push_str(" text-decoration=\"underline\"");
            }
            out.push_str(&format!(
                "<text x=\"{x:.1}\" y=\"{:.1}\" xml:space=\"preserve\" {attrs}>{}</text>\n",
                y + 13.5,
                escape(&run.text)
            ));
        }
    }
    out.push_str("</svg>\n");
    out
}

/// A terminal colour as SVG paint. `Rgb` is what the rich theme emits; the
/// sixteen indexed colours are mapped onto the theme's roles so a frame taken
/// under `FASTF_THEME=ansi` still looks like the app. `None` for a default
/// background, which the page already has.
fn color(color: vt100::Color, theme: &Theme, foreground: bool) -> Option<String> {
    let role = match color {
        vt100::Color::Rgb(r, g, b) => return Some(format!("#{r:02x}{g:02x}{b:02x}")),
        vt100::Color::Default => return foreground.then(|| FG.to_string()),
        vt100::Color::Idx(0) => return foreground.then(|| BG.to_string()),
        vt100::Color::Idx(1 | 9) => theme.bad,
        vt100::Color::Idx(2 | 10) => theme.good,
        vt100::Color::Idx(3 | 11) => theme.warn,
        vt100::Color::Idx(4 | 12) => theme.accent,
        vt100::Color::Idx(5 | 13) => theme.tags[3],
        vt100::Color::Idx(6 | 14) => theme.accent_alt,
        vt100::Color::Idx(8) => theme.dim,
        vt100::Color::Idx(_) => return foreground.then(|| FG.to_string()),
    };
    match role {
        RColor::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => foreground.then(|| FG.to_string()),
    }
}

fn is_box_drawing(c: char) -> bool {
    matches!(
        c,
        '─' | '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼'
    )
}

/// The lines one box-drawing character is made of, in its cell.
fn box_lines(c: char, x: f32, y: f32, stroke: &str) -> String {
    let (mx, my) = (x + CELL_W / 2.0, y + CELL_H / 2.0);
    let (x2, y2) = (x + CELL_W, y + CELL_H);
    let mut segments: Vec<(f32, f32, f32, f32)> = Vec::new();
    let up = (mx, y, mx, my);
    let down = (mx, my, mx, y2);
    let left = (x, my, mx, my);
    let right = (mx, my, x2, my);
    match c {
        '─' => segments.extend([left, right]),
        '│' => segments.extend([up, down]),
        '┌' => segments.extend([right, down]),
        '┐' => segments.extend([left, down]),
        '└' => segments.extend([right, up]),
        '┘' => segments.extend([left, up]),
        '├' => segments.extend([up, down, right]),
        '┤' => segments.extend([up, down, left]),
        '┬' => segments.extend([left, right, down]),
        '┴' => segments.extend([left, right, up]),
        '┼' => segments.extend([up, down, left, right]),
        _ => return String::new(),
    }
    segments
        .into_iter()
        .map(|(x1, y1, x2, y2)| {
            format!(
                "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{stroke}\" stroke-width=\"1\"/>\n"
            )
        })
        .collect()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
