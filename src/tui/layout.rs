//! Where everything goes on the screen, as a function of its size.
//!
//! Pure geometry, shared by the frames (which draw into the regions) and the
//! app (which needs to know how many rows the table has to keep its viewport).

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Below this the frame is one paragraph saying so.
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;

/// A terminal wide enough to keep the detail pane beside the list.
pub const DETAIL_MIN_WIDTH: u16 = 100;
/// A terminal tall enough for the tall header and the template strip.
pub const TALL_MIN_HEIGHT: u16 = 30;

pub fn too_small(area: Rect) -> bool {
    area.width < MIN_WIDTH || area.height < MIN_HEIGHT
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regions {
    pub header: Rect,
    pub search: Rect,
    pub table: Rect,
    pub detail: Option<Rect>,
    pub strip: Option<Rect>,
    pub status: Rect,
    pub hints: Rect,
}

impl Regions {
    /// How many project rows the table shows: its height minus the borders and
    /// the header row.
    pub fn table_rows(&self) -> usize {
        self.table.height.saturating_sub(3) as usize
    }
}

pub fn regions(area: Rect, detail_open: bool) -> Regions {
    let tall = area.height >= TALL_MIN_HEIGHT;
    // Two lines, and a blank one under them where there is room to breathe.
    let header_height = if tall { 3 } else { 2 };
    let strip_height = if tall { 3 } else { 0 };
    let bands = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(strip_height),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let (table, detail) = if detail_open && area.width >= DETAIL_MIN_WIDTH {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(bands[2]);
        (panes[0], Some(panes[1]))
    } else {
        (bands[2], None)
    };

    Regions {
        header: bands[0],
        search: bands[1],
        table,
        detail,
        strip: (strip_height > 0).then_some(bands[3]),
        status: bands[4],
        hints: bands[5],
    }
}

/// A rectangle of `percent_x` × `percent_y` of `area`, centred.
pub fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical)[1]
}

/// A dialog sized to what it holds — never taller than most of the screen,
/// never so short that its footer and key line crowd the content. The studio,
/// the builder and the settings are drawn in one of these.
pub fn sized_dialog(area: Rect, body: u16) -> Rect {
    let full = centered(area, 84, 96);
    let height = (body + 4).clamp(8.min(full.height), full.height);
    Rect::new(
        full.x,
        full.y + (full.height - height) / 2,
        full.width,
        height,
    )
}

/// How many rows a list drawn inside `dialog` shows, under `above` rows of
/// chrome besides the borders (a footer and a key line, a query line…).
pub fn list_rows(dialog: Rect, above: u16) -> usize {
    dialog.height.saturating_sub(2 + above) as usize
}

/// The settings list: `sized_dialog` at its full body, minus the footer and
/// the key line.
pub fn settings_rows(area: Rect) -> usize {
    list_rows(sized_dialog(area, 22), 2)
}

/// The studio's list, beside a detail `lines` long.
pub fn studio_rows(area: Rect, cards: usize, lines: usize) -> usize {
    list_rows(sized_dialog(area, cards.max(lines).max(4) as u16), 2)
}

/// The action menu's box: as tall as its verbs, within reason.
pub fn actions_box(area: Rect, entries: usize) -> Rect {
    let height = (entries as u16 + 4).clamp(8, 30);
    centered_fixed(area, 64, height)
}

/// A fuzzy picker's box: a query line, a blank, then the ranked rows.
pub fn pick_box(area: Rect, items: usize) -> Rect {
    let height = (items as u16 + 4).clamp(6, 16);
    centered_fixed(area, 50, height)
}

/// Where the help overlay is drawn. The app clamps its scroll with the same
/// box the view draws it in.
pub fn help_box(area: Rect) -> Rect {
    centered(area, 84, 84)
}

/// Where a read-only message (metadata, a journal, a report) is drawn.
pub fn message_box(area: Rect) -> Rect {
    centered(area, 70, 50)
}

/// A rectangle of at most `width` × `height` cells, centred, never larger than
/// `area`.
pub fn centered_fixed(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standard_terminal_gets_the_compact_layout() {
        let r = regions(Rect::new(0, 0, 80, 24), true);
        assert_eq!(r.header.height, 2);
        assert!(r.detail.is_none(), "80 columns is too narrow for the pane");
        assert!(r.strip.is_none());
        assert_eq!(r.table_rows(), 24 - 2 - 1 - 1 - 1 - 3);
        assert!(!too_small(Rect::new(0, 0, 80, 24)));
    }

    #[test]
    fn a_large_terminal_gets_the_pane_and_the_strip() {
        let r = regions(Rect::new(0, 0, 120, 40), true);
        assert_eq!(r.header.height, 3);
        assert!(r.detail.is_some());
        assert!(r.strip.is_some());
        let r = regions(Rect::new(0, 0, 120, 40), false);
        assert!(r.detail.is_none(), "the pane can be closed");
    }

    #[test]
    fn a_tiny_terminal_is_too_small() {
        assert!(too_small(Rect::new(0, 0, 40, 10)));
        assert!(!too_small(Rect::new(0, 0, 60, 16)));
    }
}
