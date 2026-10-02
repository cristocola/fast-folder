//! What is drawn over the dashboard: the palette, help, a picker, a message.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::App;
use crate::tui::app::actions::{ActionsState, Confirm, MultiPick, TextPrompt};
use crate::tui::app::modal::{MessageLevel, Modal, PickState};
use crate::tui::app::palette::PaletteState;
use crate::tui::app::wizard::{Flow, Preview, Step};
use crate::tui::command::{self, Availability, CommandId, Context};
use crate::tui::layout::{centered, centered_fixed};
// `clear` by this path too: the files below call it as `super::clear`.
use super::clear;
use crate::tui::view::{fit, fit_spans, highlighted, pad, split_line};

mod bases;
mod flow;
mod jobs;
mod lists;
mod prompts;
mod reading;

pub(crate) use bases::base_note_text;
use bases::*;
pub(crate) use flow::*;
pub use jobs::*;
use lists::*;
use prompts::*;
pub(crate) use reading::*;

/// Draw the top modal, if any. Returns where its caret is.
pub fn render(app: &App, frame: &mut Frame, area: Rect) -> Option<Position> {
    match app.modals.top()? {
        Modal::Palette(palette) => Some(render_palette(app, palette, frame, area)),
        Modal::Help { ctx, scroll } => {
            render_help(app, *ctx, *scroll, frame, area);
            None
        }
        Modal::Pick(pick) => Some(render_pick(app, pick, frame, area)),
        Modal::Actions(actions) => render_actions(app, actions, frame, area),
        Modal::Bases(panel) => {
            render_bases(app, panel, frame, area);
            None
        }
        Modal::BaseMenu(menu) => render_base_menu(app, menu, frame, area),
        Modal::TextPrompt(prompt) => Some(render_text_prompt(app, prompt, frame, area)),
        Modal::Note(note) => Some(render_note(app, note, frame, area)),
        Modal::Confirm(confirm) => render_confirm(app, confirm, frame, area),
        Modal::MultiPick(pick) => render_multi_pick(app, pick, frame, area),
        Modal::Flow(flow) => render_flow(app, flow, frame, area),
        Modal::Builder(builder) => {
            crate::tui::view::builder::render_builder(app, builder, frame, area)
        }
        Modal::Settings(state) => {
            crate::tui::view::builder::render_settings(app, state, frame, area)
        }
        Modal::Onboarding(state) => {
            crate::tui::view::builder::render_onboarding(app, state, frame, area)
        }
        Modal::Guide(state) => {
            render_guide(app, state, frame, area);
            None
        }
        Modal::Activity(activity) => {
            render_activity(app, activity, frame, area);
            None
        }
        Modal::Message {
            title,
            lines,
            level,
            scroll,
        } => {
            render_message(app, title, lines, *level, *scroll, frame, area);
            None
        }
    }
}

/// A progress bar, drawn from the theme's own two glyphs.
///
/// `width` cells, `done` of `total` filled, with the percentage after it. A
/// `total` of zero draws an empty track rather than a full one — nothing
/// measured is not everything done, and a move that has not scanned its
/// manifest yet would otherwise open at 100 %.
fn bar<'a>(app: &App, width: usize, done: u64, total: u64) -> Line<'a> {
    let g = app.theme.glyphs;
    let width = width.max(4);
    let filled = if total == 0 {
        0
    } else {
        // Saturating, because `done` is read from a live mutex and a manifest
        // can be re-totalled between two frames.
        ((done.min(total) as u128 * width as u128) / total as u128) as usize
    };
    let percent = if total == 0 {
        0
    } else {
        (done.min(total) as u128 * 100 / total as u128) as u64
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(g.bar_full.repeat(filled), app.theme.accent()),
        Span::styled(g.bar_empty.repeat(width - filled), app.theme.dim()),
        Span::styled(format!(" {percent:>3}%"), app.theme.dim()),
    ])
}

fn frame_block<'a>(app: &App, title: String, focused: bool) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, app.theme.accent()))
        .border_style(app.theme.border(focused))
}
