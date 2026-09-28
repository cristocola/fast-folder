//! `update`'s side of the templates tab: opening it, moving in it, and the
//! room its panes ask for.

use super::{App, Focus, Screen};
use crate::tui::effect::Effect;
use crate::tui::layout;

impl App {
    /// `T`: to the templates tab, and `T` back. The search bar belongs to
    /// whichever tab is showing, so switching drops the query with it — the
    /// two searches are over different things and carrying one across would
    /// hide most of the other list for no reason anyone typed.
    pub(super) fn toggle_templates(&mut self) -> Vec<Effect> {
        self.screen = match self.screen {
            Screen::Library => Screen::Templates,
            Screen::Templates => Screen::Library,
        };
        self.search.editing = false;
        self.search.input.clear();
        self.search.query = Default::default();
        self.recompute();
        let mut effects = self.after_rows_changed();
        if self.screen == Screen::Templates {
            let rows = self.studio.rows("");
            self.studio.reselect(&rows);
            self.studio.clamp_viewport(&rows, self.template_rows());
            if self.studio.shown.is_none()
                && let Some(slug) = self.studio.selected_slug()
            {
                effects.push(Effect::LoadTemplateView { slug });
            }
            self.offer_guide_once();
        }
        effects
    }

    /// The ends of the templates list. `Studio::jump` keeps the selection on
    /// a row the current filter shows, exactly as `step` does.
    pub(super) fn jump_templates(&mut self, first: bool) -> Vec<Effect> {
        let rows = self.studio.rows(self.search.input.text());
        self.studio.jump(first, &rows);
        self.studio.clamp_viewport(&rows, self.template_rows());
        self.studio
            .selected_slug()
            .map(|slug| vec![Effect::LoadTemplateView { slug }])
            .unwrap_or_default()
    }

    /// The templates tab's arrows, over the rows its own query keeps.
    fn step_templates(&mut self, delta: isize) -> Vec<Effect> {
        let rows = self.studio.rows(self.search.input.text());
        self.studio.step(delta, &rows);
        self.studio.clamp_viewport(&rows, self.template_rows());
        self.studio
            .selected_slug()
            .map(|slug| vec![Effect::LoadTemplateView { slug }])
            .unwrap_or_default()
    }

    /// Up/Down and the page keys on the templates tab: the card list, or the
    /// pane beside it when that is what Tab has the focus on.
    pub(super) fn step_templates_or_pane(&mut self, delta: isize) -> Vec<Effect> {
        if self.focus == Focus::Detail {
            self.studio.scroll = self
                .studio
                .scroll
                .saturating_add_signed(delta)
                .min(self.studio_scroll_max());
            return Vec::new();
        }
        self.step_templates(delta)
    }

    /// The last row the pane can be scrolled to, from the geometry `view`
    /// draws with — so the cursor cannot leave the drawn window.
    ///
    /// **The templates tab's own split**, not the library's: `regions().detail`
    /// is the library pane — somewhere else entirely, or closed, while the
    /// template pane is always drawn.
    pub(super) fn studio_scroll_max(&self) -> usize {
        let (_, pane, _) = self.template_panes();
        let rows = pane.height.saturating_sub(2) as usize;
        self.studio.lines.len().saturating_sub(rows)
    }

    /// What the templates list asks of the body: every card's name whole with
    /// its count, and how many cards there are — measured over every card, so
    /// a search on the tab never moves its pane.
    pub fn templates_needs(&self) -> layout::TableNeeds {
        let name = self
            .studio
            .cards
            .iter()
            .map(|card| {
                unicode_width::UnicodeWidthStr::width(super::TemplatesState::display_name(card))
            })
            .max()
            .unwrap_or(8)
            .clamp(8, crate::tui::view::templates::SLUG_MAX);
        layout::TableNeeds {
            // The borders, the marker and its space, the name, the count.
            min_width: (2 + 2 + name + 5 + 1) as u16,
            rows: self.studio.cards.len(),
        }
    }

    /// The templates tab's list and pane, and where the pane is.
    pub fn template_panes(
        &self,
    ) -> (
        ratatui::layout::Rect,
        ratatui::layout::Rect,
        layout::Placement,
    ) {
        layout::templates_panes(self.regions().body, self.templates_needs())
    }

    /// How many cards the list shows at once: its height inside the border.
    pub fn template_rows(&self) -> usize {
        self.template_panes().0.height.saturating_sub(2) as usize
    }
}
