//! A key, from the top modal down to the registry, and the rows a verb is
//! about.

use super::*;

impl App {
    pub(super) fn on_key(&mut self, key: Key) -> Vec<Effect> {
        if layout::too_small(self.area()) {
            // The guard takes only the two quit gestures — and a batch that is
            // running still turns them into a cancel, exactly as it does on
            // a screen big enough to show it.
            if key != Key::ch('q') && key != Key::ctrl('c') {
                return Vec::new();
            }
            return self.quit(if key == Key::ctrl('c') {
                Exit::Interrupted
            } else {
                Exit::Normal
            });
        }
        if key == Key::ctrl('c') {
            // Declared in the registry so it is in the help, but dispatched
            // here rather than through `lookup`: the interrupt key answers
            // from inside a text field and from under a modal that consumes
            // every key, and no availability state may swallow it.
            return self.run(CommandId::Interrupt);
        }
        // A batch that is running turns the other quit gestures — `q`, and Esc
        // once it has closed whatever was open — into cancels too (`run`); see
        // the Ctrl-C case above.
        if !self.modals.is_empty() {
            return self.on_modal_key(key);
        }
        if self.search.editing {
            return self.on_search_key(key);
        }
        if self.pane_edit.is_some() {
            return self.on_pane_edit_key(key);
        }
        match command::lookup(self.context(), key, self) {
            Some(id) => self.run(id),
            None => Vec::new(),
        }
    }

    /// **The field has first refusal; the registry answers the rest.** Every
    /// printable key is a letter of the query and every caret chord is the
    /// field's — `Ctrl-u` here is kill-to-start, whatever it means on a list —
    /// and what is left is exactly what `Context::SearchEdit` declares. That
    /// is why the arrows can be `CommandId::Down` and `Up` themselves, moving
    /// the library under the query, rather than a second pair written out
    /// here; and it is what lets `Ctrl-p` open the palette from the bar while
    /// `c` types a `c`.
    ///
    /// `command::keys_in` is the other half of the same rule: it takes what
    /// the field claims back out of what this context advertises, so the help
    /// never names a key the bar will swallow.
    fn on_search_key(&mut self, key: Key) -> Vec<Effect> {
        if key.typed().is_some() || command::field_claims(&key) {
            return if self.search.input.apply(&key) {
                self.after_query_change()
            } else {
                Vec::new()
            };
        }
        match command::lookup(self.context(), key, self) {
            Some(id) => self.run(id),
            None => {
                if self.search.input.apply(&key) {
                    self.after_query_change()
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn on_modal_key(&mut self, key: Key) -> Vec<Effect> {
        match self.modals.top() {
            Some(Modal::Palette(_)) => self.on_palette_key(key),
            Some(Modal::Pick(_)) => self.on_pick_key(key),
            Some(Modal::Actions(_)) => self.on_actions_key(key),
            Some(Modal::TextPrompt(_)) => self.on_text_prompt_key(key),
            Some(Modal::Note(_)) => self.on_note_key(key),
            Some(Modal::Confirm(_)) => self.on_confirm_key(key),
            Some(Modal::MultiPick(_)) => self.on_multi_pick_key(key),
            Some(Modal::Flow(_)) => self.on_flow_key(key),
            Some(Modal::Builder(_)) => self.on_builder_key(key),
            Some(Modal::Settings(_)) => self.on_settings_key(key),
            Some(Modal::Onboarding(_)) => self.on_onboarding_key(key),
            Some(Modal::Guide(_)) => self.on_guide_key(key),
            Some(Modal::Help { .. }) | Some(Modal::Message { .. }) => self.on_scroll_modal_key(key),
            Some(Modal::Activity(_)) => self.on_activity_key(key),
            None => Vec::new(),
        }
    }

    /// The key a dialog did not take itself: whatever the registry binds in
    /// the dialog's context, or nothing. This is how every list on a dialog
    /// answers the same keys the help overlay lists for it.
    pub(super) fn lookup_and_run(&mut self, key: Key) -> Vec<Effect> {
        match command::lookup(self.context(), key, self) {
            Some(id) => self.run(id),
            None => Vec::new(),
        }
    }

    /// The project at `path` in the list as it stands now.
    ///
    /// A dialog carries the path it was opened for, and this is how it gets
    /// back to a project at submit time — by the one thing that is unique
    /// whatever else has happened, which is what `ListChange::Patched` also
    /// keys on.
    pub(super) fn project_at(&self, path: &Path) -> Option<Project> {
        self.library
            .snapshot
            .iter()
            .find(|project| project.path == path)
            .cloned()
    }

    /// The named project is not in the library any more, so the verb does not
    /// run — on a neighbour least of all.
    pub(super) fn gone_from_the_library(&mut self) -> Vec<Effect> {
        self.warn("That project is no longer in the library — nothing was done.");
        Vec::new()
    }

    /// Whether a verb acts on the marks rather than the selection.
    ///
    /// **Asked of `targets()`, not of the mark set.** Marks are kept by path
    /// and survive a filter change; `targets()` intersects them with the rows
    /// on screen. Asked of the marks alone, `batching()` says yes while
    /// `targets()` is empty, and every batch verb returns at its
    /// `if targets.is_empty() { return Vec::new(); }` — no picker, no dialog,
    /// no message.
    pub(super) fn batching(&self) -> bool {
        !self.library.targets().is_empty() && !self.library.marks.is_empty()
    }

    /// The folder names a confirmation is about: the marks, or the selection.
    pub(super) fn target_names(&self) -> Vec<String> {
        self.library
            .targets()
            .iter()
            .map(|project| project.name.clone())
            .collect()
    }

    /// A screenful for the list or pane the keys go to: the pane pages by its
    /// own height, never the table's.
    pub(super) fn page_rows(&self) -> usize {
        let rows = match (self.screen, self.focus) {
            (Screen::Library, Focus::Detail) => self.pane_rows_on_screen(),
            (Screen::Library, Focus::Projects) => self.rows_on_screen(),
            (Screen::Templates, Focus::Projects) => self.template_rows(),
            (Screen::Templates, Focus::Detail) => {
                self.template_panes().1.height.saturating_sub(2) as usize
            }
        };
        rows.max(1)
    }
}
