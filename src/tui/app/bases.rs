//! The bases panel (`b`): which bases the list shows.
//!
//! Every base is marked active or not, and the list shows every base or only
//! the active ones (`library::BasesView`), remembered between runs. Enter on a
//! base opens its menu — every verb with its key, the shape of the project
//! action menu — and the same keys answer in the panel itself.
//!
//! The panel holds only its cursor. Its rows are the summary's bases, read
//! each frame, so a base added in the settings is in the panel the moment the
//! summary that names it lands.

use std::path::PathBuf;

use super::App;
use crate::tui::app::data::{BaseInfo, Summary};
use crate::tui::app::library::BasesView;
use crate::tui::app::modal::Modal;
use crate::tui::app::settings::SettingsState;
use crate::tui::command::{self, Availability, Category, CommandId, Context};
use crate::tui::effect::Effect;

/// The panel's cursor, and the first row its window shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BasesPanel {
    pub selected: usize,
    pub offset: usize,
}

/// One base's menu. It carries the base it was opened on, as the
/// configuration spells it, so a summary landing underneath cannot point its
/// verbs at a neighbour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseMenu {
    pub base: PathBuf,
    pub selected: usize,
}

/// The verbs a base's menu lists, in the registry's order, each with whether
/// it can run now.
pub fn base_menu_entries(app: &App) -> Vec<(CommandId, Availability)> {
    command::COMMANDS
        .iter()
        .filter(|command| {
            command.contexts.contains(&Context::BaseMenu) && command.category == Category::Search
        })
        .map(|command| (command.id, (command.available)(app)))
        .filter(|(_, availability)| *availability != Availability::Hidden)
        .collect()
}

/// What a base verb says it will do, for the base it is about and the view on
/// now: a menu row names the change, not the toggle.
pub fn base_verb_title(app: &App, id: CommandId) -> &'static str {
    match id {
        CommandId::BaseToggleActive if app.base_in_hand_active() => "Mark inactive",
        CommandId::BaseToggleActive => "Mark active",
        CommandId::BaseShowOnly if app.base_in_hand_alone() => "Show every base again",
        CommandId::BaseShowOnly => "Show only this base",
        CommandId::BasesView | CommandId::ListBasesView
            if app.library.view == BasesView::Active =>
        {
            "Show every base"
        }
        CommandId::BasesView | CommandId::ListBasesView => "Show active bases only",
        _ => command::find(id).title,
    }
}

impl App {
    /// The bases as the summary has them; `None` while they are probed.
    pub fn known_bases(&self) -> Option<&[BaseInfo]> {
        self.summary.as_ref().and_then(Summary::bases_known)
    }

    /// The base a base verb acts on: the menu's own, or the one under the
    /// panel's cursor.
    pub fn base_in_hand(&self) -> Option<&BaseInfo> {
        let bases = self.known_bases()?;
        match self.modals.top()? {
            Modal::BaseMenu(menu) => bases.iter().find(|base| base.configured == menu.base),
            Modal::Bases(panel) => bases.get(panel.selected),
            _ => None,
        }
    }

    /// Whether the base in hand is active: what Space would untick.
    pub fn base_in_hand_active(&self) -> bool {
        self.base_in_hand()
            .is_none_or(|base| self.base_is_active(base))
    }

    /// Whether the base in hand is the one the list shows alone.
    pub fn base_in_hand_alone(&self) -> bool {
        self.base_in_hand()
            .is_some_and(|base| self.shown_alone(base))
    }

    /// Marked by either of its names: the configured one is what a tick
    /// writes, the real one what `state.toml` keeps beside it.
    pub fn base_is_active(&self, base: &BaseInfo) -> bool {
        !self.library.inactive.contains(&base.configured)
            && !self.library.inactive.contains(&base.path)
    }

    /// Whether the list shows this base and no other.
    pub fn shown_alone(&self, base: &BaseInfo) -> bool {
        self.library.base_filter.as_ref() == Some(&base.path)
    }

    /// Whether a configured base is marked inactive — the question both views
    /// are the same list without.
    pub fn some_base_inactive(&self) -> bool {
        match self.known_bases() {
            Some(bases) => bases.iter().any(|base| !self.base_is_active(base)),
            None => !self.library.inactive.is_empty(),
        }
    }

    /// The inactive bases, for `state.toml`: as configured, and by their
    /// real path too where the two differ (a base reached through a link), so
    /// the next run can match rows handed in whole — `fastf recent` — before
    /// any summary has said which path is which base.
    pub fn inactive_to_remember(&self) -> Vec<String> {
        let mut out: std::collections::BTreeSet<PathBuf> = self.library.inactive.clone();
        for base in self.known_bases().unwrap_or(&[]) {
            if !self.base_is_active(base) {
                out.insert(base.configured.clone());
                out.insert(base.path.clone());
            }
        }
        out.iter()
            .map(|base| base.to_string_lossy().into_owned())
            .collect()
    }

    /// `b`: the panel, its cursor on the base the list shows alone if it shows
    /// one, else on the first.
    pub(super) fn open_bases_panel(&mut self) -> Vec<Effect> {
        let selected = self
            .known_bases()
            .and_then(|bases| bases.iter().position(|base| self.shown_alone(base)))
            .unwrap_or(0);
        self.modals.push(Modal::Bases(BasesPanel {
            selected,
            offset: 0,
        }));
        self.keep_bases_cursor_in_view();
        Vec::new()
    }

    /// Enter in the panel: the menu of the base under the cursor.
    pub(super) fn open_base_menu(&mut self) -> Vec<Effect> {
        let Some(base) = self.base_in_hand().map(|base| base.configured.clone()) else {
            return Vec::new();
        };
        self.modals
            .push(Modal::BaseMenu(BaseMenu { base, selected: 0 }));
        Vec::new()
    }

    /// Enter in a base's menu: the verb under its cursor.
    pub(super) fn run_chosen_base_verb(&mut self) -> Vec<Effect> {
        let chosen = match self.modals.top() {
            Some(Modal::BaseMenu(menu)) => base_menu_entries(self)
                .get(menu.selected)
                .map(|(id, _)| *id),
            _ => None,
        };
        match chosen {
            Some(id) => self.run(id),
            None => Vec::new(),
        }
    }

    /// A verb about one base, from the panel or from that base's menu. The
    /// base is taken before the menu closes; the panel stays up under a
    /// change of state, so the change is seen where it was made.
    pub(super) fn base_verb(&mut self, id: CommandId) -> Vec<Effect> {
        let Some(base) = self.base_in_hand().cloned() else {
            return Vec::new();
        };
        if matches!(self.modals.top(), Some(Modal::BaseMenu(_))) {
            self.modals.pop();
        }
        match id {
            CommandId::BaseToggleActive => self.toggle_base(&base),
            CommandId::BaseShowOnly => self.show_base_alone(&base),
            _ => Vec::new(),
        }
    }

    /// Space: a base in or out of the active ones.
    ///
    /// **Unticking a base is asking not to see it**, so in the every-base
    /// view — where it would change nothing on screen — the view becomes the
    /// active bases. Ticking the last inactive one again makes the two views
    /// one list, and the one shown is the one that says nothing is left out.
    fn toggle_base(&mut self, base: &BaseInfo) -> Vec<Effect> {
        let was_active = self.base_is_active(base);
        let switched = if was_active {
            self.library.inactive.insert(base.configured.clone());
            std::mem::replace(&mut self.library.view, BasesView::Active) == BasesView::Every
        } else {
            self.library.inactive.remove(&base.configured);
            self.library.inactive.remove(&base.path);
            if !self.some_base_inactive() {
                self.library.view = BasesView::Every;
            }
            false
        };
        let view_key = command::key_of(CommandId::ListBasesView);
        match (was_active, switched) {
            // Shown alone, it stays on screen: a base named beats the view.
            (true, _) if self.shown_alone(base) => self.info(format!(
                "{} is inactive — shown alone until {} clears it",
                base.label,
                command::key_of(CommandId::ClearFilters)
            )),
            (true, true) => self.info(format!(
                "{} is inactive — the list shows the active bases now, {view_key} shows every base",
                base.label
            )),
            (true, false) => self.info(format!("{} is inactive — out of the list", base.label)),
            (false, _) => self.info(format!("{} is active", base.label)),
        }
        let mut effects = self.reordered();
        effects.push(Effect::SaveSession);
        effects
    }

    /// `f`: the list shows this base and no other — the old base filter,
    /// which beats the view, so an inactive base can be looked into without
    /// being made active. On the base already shown alone, every base in view
    /// again. Either way the list is what was asked for, so the panel closes.
    fn show_base_alone(&mut self, base: &BaseInfo) -> Vec<Effect> {
        while matches!(
            self.modals.top(),
            Some(Modal::Bases(_) | Modal::BaseMenu(_))
        ) {
            self.modals.pop();
        }
        if self.shown_alone(base) {
            self.info("showing every base in view again");
            return self.set_base_filter(None);
        }
        self.info(format!("showing only {}", base.label));
        self.set_base_filter(Some(base.path.clone()))
    }

    /// `B`: every base, or only the active ones.
    pub(super) fn switch_bases_view(&mut self) -> Vec<Effect> {
        if matches!(self.modals.top(), Some(Modal::BaseMenu(_))) {
            self.modals.pop();
        }
        self.library.view = match self.library.view {
            BasesView::Every => BasesView::Active,
            BasesView::Active => BasesView::Every,
        };
        let out = self
            .known_bases()
            .map(|bases| bases.iter().filter(|b| !self.base_is_active(b)).count())
            .unwrap_or(self.library.inactive.len());
        match self.library.view {
            BasesView::Active => self.info(format!(
                "showing the active bases — {out} inactive base{} out of the list",
                crate::util::plural::s(out)
            )),
            BasesView::Every => self.info("showing every base"),
        }
        let mut effects = self.reordered();
        effects.push(Effect::SaveSession);
        effects
    }

    /// "Add or remove bases": the settings, on the row that holds the list,
    /// over the panel — so leaving them comes back to it, showing what
    /// changed.
    pub(super) fn edit_base_list(&mut self) -> Vec<Effect> {
        if matches!(self.modals.top(), Some(Modal::BaseMenu(_))) {
            self.modals.pop();
        }
        self.modals
            .push(Modal::Settings(Box::new(SettingsState::pending_at(
                "bases",
            ))));
        vec![Effect::LoadSettings]
    }

    /// A project the view leaves out, asked for by name — a palette jump, a
    /// project just made in an inactive base: its base is shown alone, which
    /// the view gives way to, rather than the project not being found.
    /// `false` when it is not in the library, or out of view for another
    /// reason.
    pub(super) fn show_its_base_alone(&mut self, path: &std::path::Path) -> bool {
        let Some(project) = self.project_at(path) else {
            return false;
        };
        if !self.library.out_of_view(&project) {
            return false;
        }
        let was = self.library.base_filter.replace(project.base.clone());
        self.recompute();
        if !self.library.select_path(path) {
            // Something else — a template filter, a query — leaves it out
            // too: the list is put back as it was, not left on a base nobody
            // asked to see.
            self.library.base_filter = was;
            self.recompute();
            return false;
        }
        self.info(format!(
            "{} is in {}, an inactive base — shown alone, {} goes back to the active bases",
            project.id,
            crate::core::library::base_label(&project.base),
            command::key_of(CommandId::ClearFilters)
        ));
        true
    }

    /// Keep the panel's cursor inside its window.
    pub(super) fn keep_bases_cursor_in_view(&mut self) {
        let rows = self.known_bases().map_or(0, <[BaseInfo]>::len);
        let visible = crate::tui::layout::bases_list_rows(self.area(), rows);
        if let Some(Modal::Bases(panel)) = self.modals.top_mut() {
            panel.offset = crate::tui::widgets::nav::viewport_offset(
                panel.offset,
                Some(panel.selected),
                rows,
                visible,
            );
        }
    }

    /// The arrows, a page or an end in the panel or a base's menu.
    pub(super) fn step_bases(&mut self, delta: isize) -> Vec<Effect> {
        let rows = self.known_bases().map_or(0, <[BaseInfo]>::len);
        let verbs = base_menu_entries(self).len();
        match self.modals.top_mut() {
            Some(Modal::Bases(panel)) => {
                panel.selected =
                    crate::tui::widgets::nav::step(Some(panel.selected), rows, delta).unwrap_or(0);
                self.keep_bases_cursor_in_view();
            }
            Some(Modal::BaseMenu(menu)) => {
                menu.selected =
                    crate::tui::widgets::nav::step(Some(menu.selected), verbs, delta).unwrap_or(0);
            }
            _ => {}
        }
        Vec::new()
    }
}
