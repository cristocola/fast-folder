//! Where everything goes on the screen, as a function of its size.
//!
//! Pure geometry, shared by the frames (which draw into the regions) and the
//! app (which needs to know how many rows the table has to keep its viewport).

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Below this the frame is one paragraph saying so.
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;

/// A terminal tall enough for the header's blank third line.
pub const TALL_MIN_HEIGHT: u16 = 30;

pub fn too_small(area: Rect) -> bool {
    area.width < MIN_WIDTH || area.height < MIN_HEIGHT
}

/// Where the detail pane goes: whichever the window has room for. See
/// [`place`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Beside the table, sharing the body's width.
    Beside,
    /// Under the table, at the body's full width.
    Below,
    /// In the table's place, taking the whole body while it has the focus.
    Over,
}

/// The least a pane beside the table is worth drawing at: the facts and
/// figures wrap whole into it, a note keeps two dozen columns after its date.
pub const PANE_BESIDE_MIN: u16 = 36;
/// The least a table above the pane shows: six projects under its header.
pub const TABLE_BELOW_MIN: u16 = 9;
/// The least a pane under the table is worth: a dozen rows inside its border.
pub const PANE_BELOW_MIN: u16 = 14;
/// The table's share of the width when the pane sits beside it.
const TABLE_SHARE: u32 = 60;
/// The table's share of the height, at most, when the pane sits under it.
const TABLE_BELOW_SHARE: u32 = 45;
/// Under this width the help overlay takes most of the window.
const HELP_NARROW_BELOW: u16 = 100;

/// What the table asks of the body: the width that shows every folder name
/// whole with the id and the size beside it, and how many projects there are.
///
/// Both are measured over the **whole library**, never the rows a search
/// leaves, so typing into the search bar cannot move the pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableNeeds {
    pub min_width: u16,
    pub rows: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regions {
    pub header: Rect,
    pub search: Rect,
    /// The whole middle band. The table and the pane share it; the templates
    /// tab takes it all.
    pub body: Rect,
    pub table: Rect,
    /// Where the pane is drawn when it is: `None` with the pane closed. In
    /// `Placement::Over` it is the body, the same box as the table, and the
    /// app draws one of the two.
    pub detail: Option<Rect>,
    pub placement: Option<Placement>,
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

/// Split the body between the table and the pane, as the window allows.
///
/// - **Beside** when the names fit whole and a pane of `PANE_BESIDE_MIN` still
///   fits next to them. The table takes `TABLE_SHARE` of the width, or what its
///   names need, and never so much the pane drops under its minimum.
/// - **Below** otherwise, when the body is tall enough for a list worth
///   scrolling and a pane worth reading. The table keeps the full width, so the
///   names stay whole, and the height its projects need up to
///   `TABLE_BELOW_SHARE` — a library of eight hands the pane its spare rows.
/// - **Over** otherwise: the list has the whole body, and the pane takes the
///   list's place when it has the focus. The same box either way, so nothing
///   re-wraps going in and out.
///
/// A pure function of the body and the table's needs, never of the focus:
/// the pane's width is what its rows are wrapped to, and a focus that changed
/// it would re-wrap them under the cursor.
pub fn place(body: Rect, needs: TableNeeds) -> (Rect, Rect, Placement) {
    let (w, h) = (body.width, body.height);
    // In u32: a claim near the top of a u16 plus the pane's minimum would
    // saturate to a width that seems to fit and does not.
    if w as u32 >= needs.min_width as u32 + PANE_BESIDE_MIN as u32 {
        let table = fit_between(
            percent_of(w, TABLE_SHARE),
            needs.min_width,
            w - PANE_BESIDE_MIN,
        );
        return (
            Rect::new(body.x, body.y, table, h),
            Rect::new(body.x + table, body.y, w - table, h),
            Placement::Beside,
        );
    }
    if h >= TABLE_BELOW_MIN + PANE_BELOW_MIN {
        let wanted = (needs.rows.saturating_add(3).min(u16::MAX as usize) as u16)
            .min(percent_of(h, TABLE_BELOW_SHARE));
        let table = fit_between(wanted, TABLE_BELOW_MIN, h - PANE_BELOW_MIN);
        return (
            Rect::new(body.x, body.y, w, table),
            Rect::new(body.x, body.y + table, w, h - table),
            Placement::Below,
        );
    }
    (body, body, Placement::Over)
}

/// The bands of the screen, top to bottom, and the table and the pane in the
/// middle one (`place`). `pane_open` is whether the pane is switched on at
/// all; `needs` is what the table asks of the body.
pub fn regions(area: Rect, pane_open: bool, needs: TableNeeds) -> Regions {
    let tall = area.height >= TALL_MIN_HEIGHT;
    // Two lines — the tabs and the bases — and a blank one under them where
    // there is room to breathe. The templates strip that used to sit above the
    // status line is a tab of its own now, which gave the table three rows back.
    let header_height = if tall { 3 } else { 2 };
    let bands = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let body = bands[2];

    let (table, detail, placement) = if pane_open {
        let (table, pane, placement) = place(body, needs);
        (table, Some(pane), Some(placement))
    } else {
        (body, None, None)
    };

    Regions {
        header: bands[0],
        search: bands[1],
        body,
        table,
        detail,
        placement,
        status: bands[3],
        hints: bands[4],
    }
}

/// The templates tab's split: the card list, and the pane beside it. Read by
/// the view that draws it and by `update` when it clamps the pane's scroll,
/// so the cursor cannot leave the drawn window.
pub fn templates_panes(body: Rect) -> (Rect, Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(body);
    (panes[0], panes[1])
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

/// The templates tab's list: the body band's rows, inside its border.
pub fn template_rows(area: Rect) -> usize {
    regions(area, false, TableNeeds::default())
        .body
        .height
        .saturating_sub(2) as usize
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

/// Where the help overlay is drawn: most of a narrow window, 84 % of a wide
/// one. The app clamps its scroll with the same box the view draws it in.
pub fn help_box(area: Rect) -> Rect {
    if area.width < HELP_NARROW_BELOW {
        centered_fixed(
            area,
            area.width.saturating_sub(4),
            area.height.saturating_sub(3),
        )
    } else {
        centered(area, 84, 84)
    }
}

/// Where the template guide is drawn: the same room the help overlay takes,
/// and for the same reason — it is a document, and a document cut into a
/// quarter of the window is a document nobody finishes.
pub fn guide_box(area: Rect) -> Rect {
    help_box(area)
}

/// The builder's body, split into the list and the panel that explains it —
/// or `None` when there is not enough width for both, where the list keeps the
/// whole body and the footer carries the one-line hint it always did.
///
/// `fit_between` and `percent_of`, never `Ord::clamp` and never `w * n / 100`:
/// both of those are documented crashes in this file, and every `max` here is
/// computed from a window somebody can drag.
pub fn builder_panel(body: Rect) -> Option<(Rect, Rect)> {
    if body.width < PANEL_MIN_TOTAL || body.height < PANEL_MIN_HEIGHT {
        return None;
    }
    let panel = fit_between(
        percent_of(body.width, 46),
        PANEL_MIN_WIDTH,
        body.width.saturating_sub(LIST_MIN_WIDTH),
    );
    let list_width = body.width.saturating_sub(panel);
    Some((
        Rect::new(body.x, body.y, list_width, body.height),
        Rect::new(body.x + list_width, body.y, panel, body.height),
    ))
}

/// Whether a body this wide would be split into a list and a panel.
///
/// The dialog's *height* is chosen before its body exists — a panel wants more
/// rows than a seven-row list — and its width does not depend on its height, so
/// this is the half of `builder_panel`'s question that can be asked first. Ask
/// it with anything else and the box grows for a panel that is never drawn.
pub fn panel_fits_width(body_width: u16) -> bool {
    body_width >= PANEL_MIN_TOTAL
}

/// The panel is worth its width only when both halves are still usable. Below
/// this the list wins: a column of two-word lines explains nothing, and the
/// summary beside each row is the part you cannot do without.
const PANEL_MIN_TOTAL: u16 = 84;
/// Its own minimum, and the list's. `LIST_MIN_WIDTH` is what a row needs: the
/// cursor, a twelve-column label and enough summary to be worth reading.
const PANEL_MIN_WIDTH: u16 = 34;
const LIST_MIN_WIDTH: u16 = 40;
/// Below this the panel would show two lines of a paragraph, which reads as a
/// sentence that has been cut rather than as an explanation.
const PANEL_MIN_HEIGHT: u16 = 8;

/// Where a read-only message (metadata, a journal, a report) is drawn.
pub fn message_box(area: Rect) -> Rect {
    centered(area, 70, 50)
}

/// A rectangle of at most `width` × `height` cells, centred, never larger than
/// `area`.
/// `wanted`, kept inside `min..=max` **even when the room is smaller than the
/// minimum**.
///
/// `Ord::clamp` asserts `min <= max` and panics otherwise, and every `max` in a
/// terminal layout is computed from a window somebody can drag. The settings
/// screen's Bases editor did exactly that — `clamp(4, body.height - row)`, with
/// `row` walking down the body — and pressing Enter on that row in a window
/// between 16 and 23 rows tall took the whole app down with `min > max`. It
/// survived 80×24 by one row, which is why the manual pass at that size never
/// found it.
///
/// The available room wins over the wanted minimum: `max` is a hard limit and
/// `min` is only a preference, so a box in a two-row hole is two rows rather
/// than a panic.
pub fn fit_between(wanted: u16, min: u16, max: u16) -> u16 {
    wanted.max(min).min(max)
}

/// `share` percent of `whole`, computed in `u32`.
///
/// `area.width * 76 / 100` is the obvious spelling and overflows a `u16` above
/// 862 columns — which release builds, with no overflow checks, wrap instead of
/// reporting: a 900-column terminal drew a 46-column dialog. A debug build
/// panics there instead. Neither is a size.
pub fn percent_of(whole: u16, share: u32) -> u16 {
    ((whole as u32 * share) / 100).min(u16::MAX as u32) as u16
}

/// A box `wanted` rows tall opening at `row` within `body`.
///
/// It sits on its row when there is room below and slides up when there is not,
/// so an editor that opens over the row it belongs to never has to choose
/// between panicking and drawing a sliver. Never taller than `body`.
pub fn box_at_row(body: Rect, row: u16, wanted: u16, min: u16) -> Rect {
    let height = fit_between(wanted, min, body.height);
    // `body.height - height` cannot underflow: `fit_between`'s `max` is that
    // height.
    let y = body.y + row.min(body.height - height);
    Rect::new(body.x, y, body.width, height)
}

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
    fn a_size_survives_a_range_with_no_room_in_it() {
        assert_eq!(fit_between(6, 4, 10), 6, "inside the range, unchanged");
        assert_eq!(fit_between(2, 4, 10), 4, "below the minimum, raised");
        assert_eq!(fit_between(20, 4, 10), 10, "above the maximum, cut");
        // The case that panicked: less room than the minimum is worth.
        assert_eq!(fit_between(6, 4, 1), 1, "the room wins over the preference");
        assert_eq!(fit_between(6, 4, 0), 0);
    }

    #[test]
    fn a_percentage_of_a_very_wide_terminal_is_still_a_percentage() {
        assert_eq!(percent_of(120, 60), 72);
        // `900 * 76` wraps a u16; in release that made a 46-column dialog on a
        // 900-column screen, and in debug it panicked.
        assert_eq!(percent_of(900, 76), 684);
        assert_eq!(percent_of(u16::MAX, 88), 57670);
    }

    #[test]
    fn a_row_editor_slides_up_rather_than_off_the_bottom() {
        let body = Rect::new(0, 5, 40, 10);
        // Room below: it opens on its row.
        assert_eq!(box_at_row(body, 2, 4, 4), Rect::new(0, 7, 40, 4));
        // No room below: it slides up so its bottom is the body's.
        assert_eq!(box_at_row(body, 9, 4, 4), Rect::new(0, 11, 40, 4));
        // A body shorter than the minimum is the whole body, not a panic.
        let squeezed = Rect::new(0, 0, 40, 2);
        assert_eq!(box_at_row(squeezed, 1, 4, 4), Rect::new(0, 0, 40, 2));
    }

    fn inside(outer: Rect, inner: Rect) -> bool {
        let end = |start: u16, len: u16| start as u32 + len as u32;
        inner.x >= outer.x
            && inner.y >= outer.y
            && end(inner.x, inner.width) <= end(outer.x, outer.width)
            && end(inner.y, inner.height) <= end(outer.y, outer.height)
    }

    fn needs(min_width: u16, rows: usize) -> TableNeeds {
        TableNeeds { min_width, rows }
    }

    /// Beside when the names fit whole with a pane beside them, below when the
    /// window is tall enough for both, and in the list's place otherwise.
    #[test]
    fn the_pane_goes_where_the_room_is() {
        let wide = Rect::new(0, 0, 120, 40);
        // The names fit the 60 %: the usual split.
        let r = regions(wide, true, needs(54, 8));
        assert_eq!(r.placement, Some(Placement::Beside));
        assert_eq!((r.table.width, r.detail.map(|d| d.width)), (72, Some(48)));
        // Long names: the table takes what they need, the pane the rest.
        let r = regions(wide, true, needs(80, 8));
        assert_eq!(r.placement, Some(Placement::Beside));
        assert_eq!((r.table.width, r.detail.map(|d| d.width)), (80, Some(40)));
        // Names so long a pane beside them would be a sliver: it goes under
        // the table, which keeps the whole width and the names whole.
        let r = regions(wide, true, needs(95, 8));
        assert_eq!(r.placement, Some(Placement::Below));
        let pane = r.detail.unwrap();
        assert_eq!((r.table.width, pane.width), (120, 120));
        assert_eq!(r.table.height, 11, "eight projects, a header and borders");
        assert_eq!(pane.y, r.table.y + r.table.height);
        assert_eq!(r.table.height + pane.height, r.body.height);

        // A standard terminal has neither room: the pane takes the list's place.
        let r = regions(Rect::new(0, 0, 80, 24), true, needs(64, 8));
        assert_eq!(r.placement, Some(Placement::Over));
        assert_eq!(r.table, r.body);
        assert_eq!(r.detail, Some(r.body));

        // Wide and short: beside, whatever the height.
        let r = regions(Rect::new(0, 0, 200, 15), true, needs(64, 8));
        assert_eq!(r.placement, Some(Placement::Beside));

        // Tall and narrow: below, with the table as tall as its projects.
        let r = regions(Rect::new(0, 0, 60, 45), true, needs(52, 8));
        assert_eq!(r.placement, Some(Placement::Below));
        assert_eq!(r.table.height, 11);
        assert_eq!(r.detail.unwrap().height, r.body.height - 11);
        // A library too big to show whole takes its share and no more.
        let r = regions(Rect::new(0, 0, 60, 45), true, needs(52, 500));
        assert_eq!(r.table.height, percent_of(r.body.height, TABLE_BELOW_SHARE));
    }

    #[test]
    fn a_standard_terminal_gets_the_compact_layout() {
        let r = regions(Rect::new(0, 0, 80, 24), true, needs(64, 12));
        assert_eq!(r.header.height, 2);
        assert_eq!(
            r.placement,
            Some(Placement::Over),
            "the pane in the list's place"
        );
        assert_eq!(r.table_rows(), 24 - 2 - 1 - 1 - 1 - 3);
        assert!(!too_small(Rect::new(0, 0, 80, 24)));
    }

    /// The templates strip is a tab now, so the three rows it used to take
    /// along the bottom belong to the table.
    #[test]
    fn a_large_terminal_gets_the_pane_and_the_rows_the_strip_used_to_take() {
        let r = regions(Rect::new(0, 0, 120, 40), true, TableNeeds::default());
        assert_eq!(r.header.height, 3);
        assert_eq!(r.placement, Some(Placement::Beside));
        assert_eq!(r.table.height, 40 - 3 - 1 - 1 - 1);
        let r = regions(Rect::new(0, 0, 120, 40), false, TableNeeds::default());
        assert!(r.detail.is_none(), "the pane can be closed");
        assert_eq!(r.table, r.body, "and the table has the body");
    }

    /// **Every window tiles.** Whatever the size — one column, the widest a
    /// u16 holds — the bands cover the screen in order, the table and the pane
    /// sit inside the body and never overlap unless the pane is `Over`, and a
    /// placement is only chosen with the room it promises.
    #[test]
    fn regions_tile_every_window() {
        let mut sizes: Vec<(u16, u16)> = Vec::new();
        for w in (1..=300).step_by(7).chain([36, 100, 862, 900, u16::MAX]) {
            for h in (1..=100).step_by(3).chain([23, 24, 30, u16::MAX]) {
                sizes.push((w, h));
            }
        }
        for (w, h) in sizes {
            let area = Rect::new(0, 0, w, h);
            for open in [false, true] {
                for need in [
                    needs(0, 0),
                    needs(64, 8),
                    needs(118, 86),
                    needs(u16::MAX, 9999),
                ] {
                    let r = regions(area, open, need);
                    let bands = [r.header, r.search, r.body, r.status, r.hints];
                    let covered: u32 = bands.iter().map(|b| b.height as u32).sum();
                    assert!(covered <= h as u32, "{w}x{h}: the bands overflow");
                    assert!(
                        bands.windows(2).all(|p| p[0].y + p[0].height <= p[1].y),
                        "{w}x{h}: the bands are in order"
                    );
                    assert!(inside(r.body, r.table), "{w}x{h}: {r:?}");
                    let Some(pane) = r.detail else {
                        assert!(!open && r.placement.is_none());
                        assert_eq!(r.table, r.body);
                        continue;
                    };
                    assert!(inside(r.body, pane), "{w}x{h}: {r:?}");
                    match r.placement.expect("a pane has a placement") {
                        Placement::Beside => {
                            assert!(pane.width >= PANE_BESIDE_MIN);
                            assert!(r.table.width >= need.min_width);
                            assert_eq!(r.table.width + pane.width, r.body.width);
                            assert_eq!(pane.x, r.table.x + r.table.width);
                        }
                        Placement::Below => {
                            assert!(pane.height >= PANE_BELOW_MIN);
                            assert!(r.table.height >= TABLE_BELOW_MIN);
                            assert_eq!(r.table.height + pane.height, r.body.height);
                            assert_eq!(r.table.width, r.body.width);
                        }
                        Placement::Over => {
                            assert_eq!(r.table, r.body);
                            assert_eq!(pane, r.body);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_tiny_terminal_is_too_small() {
        assert!(too_small(Rect::new(0, 0, 40, 10)));
        assert!(!too_small(Rect::new(0, 0, 60, 16)));
    }
}
