//! Where things are on the screen, asked of the app: the regions, what the
//! table needs, where the pane goes, and which context the keys are in.

use super::*;

impl App {
    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.size.0, self.size.1)
    }

    pub fn regions(&self) -> layout::Regions {
        layout::regions(self.area(), self.pane_live(), self.table_needs())
    }

    /// What the table asks of the body (`layout::TableNeeds`): measured over
    /// the whole library, so a search never moves the pane.
    pub fn table_needs(&self) -> layout::TableNeeds {
        layout::TableNeeds {
            min_width: self.table_min_width(),
            rows: self.library.snapshot.len(),
        }
    }

    /// The width the table needs to show every folder name whole with the id
    /// and the size beside it: the cursor cell, the id, the name and the size
    /// cell, each followed by a space, inside the borders, plus the right
    /// gutter `view::projects::table` always reserves.
    pub fn table_min_width(&self) -> u16 {
        let (id_w, name_w) = self.library.widths;
        // The base column joins the claim once the rows come from more than one
        // base. It is elected right after the size there, and a column the
        // table does not claim never gets its width: long folder names leave
        // the split room for the size and nothing else, and the one column
        // saying which drive a project is on never appears.
        let base = if self.library.many_bases {
            self.library.base_width.min(BASE_CLAIM_MAX) + 1
        } else {
            0
        };
        // `choose_columns` adds a column only while it fits with its spacing.
        (2 + 1 + id_w + 1 + name_w + 1 + crate::tui::rows::SIZE_CELL + 1 + base + 1)
            .min(u16::MAX as usize) as u16
    }

    pub fn rows_on_screen(&self) -> usize {
        self.regions().table_rows()
    }

    /// Whether the pane is switched on: its content is read and kept read
    /// whatever its placement, so going into a pane drawn in the list's place
    /// is instant.
    ///
    /// A library with nothing in it has nothing for a pane to show, and one
    /// still being discovered does not know yet where the pane will go: the
    /// pane arrives with the first project, in its place, rather than sitting
    /// beside an empty list and then moving when the names come in.
    pub fn pane_live(&self) -> bool {
        self.detail_open && !self.library.snapshot.is_empty()
    }

    /// Whether the library's pane is drawn this frame: beside or under the
    /// table, or in the list's place while it has the focus.
    pub fn detail_visible(&self) -> bool {
        self.screen == Screen::Library
            && match self.regions().placement {
                Some(layout::Placement::Beside | layout::Placement::Below) => true,
                Some(layout::Placement::Over) => self.focus == Focus::Detail,
                None => false,
            }
    }

    /// What the selected project's record holds, once read: its notes, and
    /// its todos done out of how many. The pane's figures say it; so does the
    /// list's peek while the pane is out of sight.
    pub fn pane_counts(&self) -> Option<(usize, usize, usize)> {
        let project = self.library.selected()?;
        let detail = self.details.get(&project.path)?;
        let done = detail.todos.iter().filter(|todo| todo.done).count();
        Some((detail.notes.len(), done, detail.todos.len()))
    }

    /// The pane is drawn in the list's place and has the focus: the list is
    /// out of sight, one key away.
    pub fn pane_over_list(&self) -> bool {
        self.focus == Focus::Detail && self.placement_here() == Some(layout::Placement::Over)
    }

    /// Where this tab's pane is: the library's, or the templates tab's.
    pub fn placement_here(&self) -> Option<layout::Placement> {
        match self.screen {
            Screen::Library => self.regions().placement,
            Screen::Templates => Some(self.template_panes().2),
        }
    }

    /// The pane is on but drawn in the list's place, and the list has the
    /// focus: the pane is one key away and nothing on screen shows it.
    pub fn pane_behind_list(&self) -> bool {
        self.focus == Focus::Projects && self.placement_here() == Some(layout::Placement::Over)
    }

    /// Where a key goes right now.
    pub fn context(&self) -> Context {
        if let Some(modal) = self.modals.top() {
            return modal.context();
        }
        if self.search.editing {
            return Context::SearchEdit;
        }
        if self.pane_edit.is_some() {
            return Context::PaneEdit;
        }
        self.focus_context()
    }

    pub(super) fn focus_context(&self) -> Context {
        if self.screen == Screen::Templates {
            return Context::Templates;
        }
        match self.focus {
            Focus::Projects => Context::Projects,
            Focus::Detail => Context::Detail,
        }
    }

    /// How soon the runtime should wake the app with nothing to say, or
    /// `None` while nothing on screen is moving at all.
    ///
    /// Two speeds, because two kinds of thing move. A spinner turning and a
    /// toast counting down want five frames a second — five is enough to read
    /// as motion, and the wake is not free on a laptop. A pulse fading out
    /// wants twenty, because a fade drawn five times is a flicker. Asking for
    /// the faster one only while a pulse is in flight is what keeps the
    /// documented claim true: **the app costs nothing while idle.**
    pub fn tick_interval(&self) -> Option<std::time::Duration> {
        // Only where a frame could show it: off, or with no colour to show
        // it in, nothing moves and nothing asks to be redrawn for it.
        let visible = self.motion.is_on() && self.theme.kind != crate::tui::theme::ThemeKind::Mono;
        if visible
            && (!self.pulses.is_empty()
                || !self.pane_pulses.is_empty()
                || motion::focus_easing(self.focus_moved_at, self.elapsed_ms)
                || motion::arriving(self.status.shown_at, self.elapsed_ms))
        {
            return Some(std::time::Duration::from_millis(motion::FRAME_MS));
        }
        // A message about to go dims on its way out, which is a fade too — but
        // only for its last half second, so the slow wake is asked to cover it
        // rather than the fast one being held for six seconds.
        // A job the app follows, or any that is running: the dialog's count
        // and the chip's spinner move with it.
        let slow = self.busy.is_some()
            || self.background.watching()
            || self.status.expires_at.is_some()
            || (self.library.loaded && self.library.sizes_pending(self.rows_on_screen()));
        slow.then(|| std::time::Duration::from_millis(SLOW_FRAME_MS))
    }

    /// Whether anything on screen is moving at all.
    pub fn needs_tick(&self) -> bool {
        self.tick_interval().is_some()
    }

    /// The size cell for `path`, as the table draws it.
    pub fn size_cell(&self, path: &std::path::Path) -> SizeCell {
        match self.library.sizes.get(path) {
            Some(size) => SizeCell::Known(*size),
            None => SizeCell::Pending,
        }
    }
}
