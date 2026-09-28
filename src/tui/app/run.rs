//! `App::run`: what each command does.

use super::*;

impl App {
    /// Run a command as its key or its palette entry would.
    pub fn run(&mut self, id: CommandId) -> Vec<Effect> {
        let command = command::find(id);
        match (command.available)(self) {
            Availability::Enabled => {}
            Availability::Disabled(reason) => {
                self.warn(format!("{}: {reason}", command.title));
                return Vec::new();
            }
            Availability::Hidden => return Vec::new(),
        }
        match id {
            CommandId::Quit => self.quit(Exit::Normal),
            CommandId::Back => {
                // A running batch is cancelled first, before a keystroke can
                // clear something the user was looking at.
                if self.job.is_some() {
                    return self.request_cancel();
                }
                // The progress dialog is the first rung: Esc hides it, and the
                // job goes on — the chip in the header keeps saying so.
                if self.job_dialog_up() {
                    return self.hide_job_dialog();
                }
                // The pane is a level, like a tab: Esc leaves it for the list
                // before it clears anything the list shows — and long before
                // the ladder runs out and quits.
                if self.focus == Focus::Detail {
                    self.set_focus(Focus::Projects);
                    return Vec::new();
                }
                // On the templates tab, the first step back is to the library:
                // Esc is "one level out", and a tab is a level.
                if self.screen == Screen::Templates {
                    if !self.search.input.is_empty() {
                        self.search.input.clear();
                        return Vec::new();
                    }
                    return self.toggle_templates();
                }
                if !self.search.input.is_empty() {
                    self.search.input.clear();
                    return self.after_query_change();
                }
                if self.library.template_filter.is_some() {
                    return self.set_template_filter(None);
                }
                if !self.library.marks.is_empty() {
                    // Marks can hide nothing, but clearing them first matches
                    // the Esc ladder: one keystroke at a time, nothing lost.
                    self.library.marks.clear();
                    self.info("marks cleared");
                    return Vec::new();
                }
                if self.library.preset.is_some() {
                    // `fastf recent --tag draft` opened the app already
                    // narrowed; the chip is a filter like any other, and Esc
                    // takes it off rather than leaving the app inside it.
                    self.library.preset = None;
                    self.recompute();
                    self.info("showing every project");
                    return self.after_rows_changed();
                }
                vec![Effect::Quit(Exit::Normal)]
            }
            CommandId::Help => {
                // The help for where the keys go right now — a dialog's own
                // context when one is open, else the focused pane's.
                let ctx = self.context();
                self.modals.push(Modal::Help { ctx, scroll: 0 });
                Vec::new()
            }
            CommandId::Close => self.close_top(),
            CommandId::Interrupt => {
                // A batch is running: Ctrl-C cancels it rather than quitting
                // under a worker that is still mutating the filesystem.
                if self.job.is_some() {
                    return self.request_cancel();
                }
                if self.job_dialog_up() {
                    return self.cancel_followed_job();
                }
                // Here Ctrl-C is a close, and a close may not throw away a
                // template that has been worked on: it asks what Esc and `q`
                // ask.
                //
                // **A save in flight is deliberately not part of this.** Esc
                // and `q` are ignored while one runs, because it is about to
                // land and its refusal needs the list to land on. Ctrl-C is
                // the opposite case: `DataLock::acquire` waits up to thirty
                // seconds when another fastf holds it, and Ctrl-C is the only
                // way out of that wait. It keeps its ordinary meaning — the
                // write is a single atomic publish on a worker, and
                // interrupting the app over it is exactly what the interrupt
                // key is for.
                if matches!(
                    self.modals.top(),
                    Some(Modal::Builder(builder)) if builder.is_dirty() && !builder.saving
                ) {
                    return self.close_top();
                }
                if self.modals.pop().is_some() {
                    Vec::new()
                } else {
                    vec![Effect::Quit(Exit::Interrupted)]
                }
            }
            CommandId::GuideNext => self.turn_guide(1),
            CommandId::GuidePrevious => self.turn_guide(-1),
            CommandId::FocusList => {
                self.set_focus(Focus::Projects);
                Vec::new()
            }
            CommandId::FocusDetail => {
                self.set_focus(Focus::Detail);
                Vec::new()
            }
            CommandId::PanePreviousProject | CommandId::PaneNextProject => self
                .step_project_from_pane(if id == CommandId::PaneNextProject {
                    1
                } else {
                    -1
                }),
            CommandId::BackToLibrary => self.toggle_templates(),
            CommandId::ShowLog => self.open_log(),
            CommandId::Suspend => vec![Effect::Suspend(Suspended::Shell)],

            CommandId::ActionsRun => {
                let chosen = match self.modals.top() {
                    Some(Modal::Actions(actions)) => crate::tui::app::actions::action_entries(self)
                        .get(actions.selected)
                        .map(|(id, _)| *id),
                    _ => None,
                };
                let Some(id) = chosen else {
                    return Vec::new();
                };
                self.modals.pop();
                self.run(id)
            }
            CommandId::Guide => {
                // The page for what is under the cursor: opened from a part of
                // the editor it lands on that part, opened from the tab it
                // starts at the beginning. A guide that always opens at page
                // one is a guide nobody opens twice.
                let page = match self.modals.top() {
                    Some(Modal::Builder(builder)) if builder.pending.is_none() => {
                        crate::tui::guide::page_for_row(builder.row())
                    }
                    _ => 0,
                };
                self.open_guide(page)
            }
            CommandId::BuilderExplain => {
                self.explain_open = !self.explain_open;
                self.info(if self.explain_open {
                    "explanation panel shown"
                } else {
                    "explanation panel hidden"
                });
                Vec::new()
            }
            CommandId::StudioNew => self.open_builder(None),
            CommandId::StudioEdit => match self.studio.selected_slug() {
                Some(slug) => self.open_builder(Some(slug)),
                None => Vec::new(),
            },
            CommandId::StudioFromFolder => {
                self.modals.push(Modal::Flow(Box::new(Flow::new(
                    FlowKind::FromFolder,
                    wizard::from_folder_form(),
                ))));
                Vec::new()
            }
            CommandId::StudioDelete => {
                let Some(slug) = self.studio.selected_slug() else {
                    return Vec::new();
                };
                self.modals.push(Modal::Confirm(Confirm {
                    prompt: format!("Delete template '{slug}' and its bundled files?"),
                    then: ConfirmThen::DeleteTemplate(slug),
                }));
                Vec::new()
            }
            CommandId::BuilderOpen | CommandId::BuilderEditText => self.builder_open(),
            CommandId::BuilderAdd => self.builder_add(),
            CommandId::BuilderRemove => self.builder_remove(),
            CommandId::BuilderMoveUp | CommandId::BuilderMoveDown => {
                self.builder_move(id == CommandId::BuilderMoveUp)
            }
            CommandId::BuilderSave => self.save_template(),
            CommandId::SettingsChange | CommandId::SettingsEditText => self.settings_change(),
            CommandId::SettingsFilter => {
                if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                    state.begin_filter();
                }
                Vec::new()
            }
            CommandId::Palette => {
                self.open_palette();
                Vec::new()
            }
            CommandId::Reload => {
                let mut effects = vec![self.discover(), Effect::LoadSummary];
                // And the pane: F5 is the key a person presses after editing
                // the file in another window.
                if self.pane_live()
                    && let Some(project) = self.library.selected()
                {
                    effects.push(self.detail_effect(&project.path));
                }
                effects
            }
            CommandId::Reindex => self.run_action("reindexing…", Action::Reindex),
            CommandId::FocusNext | CommandId::FocusPrevious => {
                let forward = id == CommandId::FocusNext;
                // On the activity screen the next pane is the next page.
                if let Some(Modal::Activity(activity)) = self.modals.top_mut() {
                    activity.turn(if forward { 1 } else { -1 });
                    return Vec::new();
                }
                let next = self.next_focus(forward);
                self.set_focus(next);
                Vec::new()
            }
            CommandId::Down | CommandId::Up => {
                let delta = if id == CommandId::Down { 1 } else { -1 };
                if !self.modals.is_empty() {
                    return self.step_top_modal(delta);
                }
                if self.screen == Screen::Templates {
                    return self.step_templates_or_pane(delta);
                }
                match self.focus {
                    Focus::Projects => {
                        self.library.step(delta);
                        self.after_selection_change()
                    }
                    Focus::Detail => {
                        self.move_pane_cursor(delta);
                        Vec::new()
                    }
                }
            }
            CommandId::PageDown | CommandId::PageUp | CommandId::HalfDown | CommandId::HalfUp => {
                let rows = self.page_rows() as isize;
                // Half a page is at least one row: on a window short enough
                // for `page_rows` to be 1, a half of it rounds to nothing and
                // the key would do nothing at all.
                let step = match id {
                    CommandId::HalfDown | CommandId::HalfUp => (rows / 2).max(1),
                    _ => rows,
                };
                let delta = if matches!(id, CommandId::PageDown | CommandId::HalfDown) {
                    step
                } else {
                    -step
                };
                if !self.modals.is_empty() {
                    return self.page_top_modal(delta);
                }
                if self.screen == Screen::Templates {
                    return self.step_templates_or_pane(delta);
                }
                match self.focus {
                    Focus::Detail => {
                        self.page_pane_cursor(delta);
                        Vec::new()
                    }
                    _ => {
                        self.library.jump(delta);
                        self.after_selection_change()
                    }
                }
            }
            CommandId::First | CommandId::Last => {
                let first = id == CommandId::First;
                if !self.modals.is_empty() {
                    return self.page_top_modal(if first { isize::MIN } else { isize::MAX });
                }
                if self.screen == Screen::Templates {
                    if self.focus == Focus::Detail {
                        self.studio.scroll = if first { 0 } else { self.studio_scroll_max() };
                        return Vec::new();
                    }
                    return self.jump_templates(first);
                }
                match self.focus {
                    Focus::Detail => {
                        self.move_pane_cursor(if first { isize::MIN } else { isize::MAX });
                        Vec::new()
                    }
                    Focus::Projects => {
                        if first {
                            self.library.select_first();
                        } else {
                            self.library.select_last();
                        }
                        self.after_selection_change()
                    }
                }
            }

            // Each of these dispatches on which dialog is on top, the way
            // `Close` always has: one key, one meaning, three shapes of
            // prompt under it.
            CommandId::PromptConfirm => match self.modals.top() {
                Some(Modal::TextPrompt(_)) => self.submit_text_prompt(),
                Some(Modal::Note(_)) => self.save_note(),
                Some(Modal::Onboarding(_)) => self.submit_onboarding(),
                _ => Vec::new(),
            },
            CommandId::PromptNewline => {
                if let Some(Modal::Note(note)) = self.modals.top_mut() {
                    note.area.apply(&Key::plain(KeyCode::Enter));
                }
                Vec::new()
            }
            CommandId::PromptCancel => {
                let skipped = matches!(self.modals.top(), Some(Modal::Onboarding(_)));
                self.modals.pop();
                if skipped {
                    self.info(validators::ONBOARDING_SKIPPED);
                }
                Vec::new()
            }
            CommandId::PickChoose => match self.modals.top() {
                Some(Modal::MultiPick(_)) => self.submit_multi_pick(),
                _ => self.choose_pick(),
            },
            CommandId::PickToggle => self.toggle_multi_pick(),
            CommandId::PickNext => self.step_pick(1),
            CommandId::PickPrevious => self.step_pick(-1),
            CommandId::PickCancel => {
                self.modals.pop();
                Vec::new()
            }
            CommandId::PaletteRun => self.run_palette_entry(),
            CommandId::PaletteNext => self.step_palette(1),
            CommandId::PalettePrevious => self.step_palette(-1),
            CommandId::PaletteClose => {
                self.modals.pop();
                Vec::new()
            }
            CommandId::SearchAccept => {
                self.search.editing = false;
                Vec::new()
            }
            // The bar's own Esc, rather than a rung bolted onto `Back`'s
            // ladder: it is the same family as `PickCancel` and
            // `PromptCancel`, each naming what it leaves.
            CommandId::SearchCancel => {
                if self.search.input.is_empty() {
                    self.search.editing = false;
                    return Vec::new();
                }
                self.search.input.clear();
                self.after_query_change()
            }
            CommandId::Search => {
                self.search.editing = true;
                self.set_focus(Focus::Projects);
                Vec::new()
            }
            CommandId::ClearSearch => {
                self.search.input.clear();
                self.after_query_change()
            }
            CommandId::SortCycle => {
                // The cycle walks the orders forwards. A direction is the
                // picker's to choose, so `s` from a reversed order lands on
                // the next one the right way up rather than staying upside
                // down for the rest of the session.
                let current = self.library.effective_sort(&self.search.query);
                self.library.explicit_sort = Some(Sort::new(current.order.next()));
                let sort = self.library.effective_sort(&self.search.query);
                self.info(format!("sorted by {}", sort.label()));
                self.reordered()
            }
            CommandId::SortPick => {
                // Both directions of every order that has two, each beside the
                // other: a person looking for the biggest folder and a person
                // looking for the smallest are reading the same list.
                let items = Order::CYCLE
                    .iter()
                    .flat_map(|order| {
                        let forwards = Sort::new(*order);
                        let backwards = Sort {
                            order: *order,
                            reversed: true,
                        };
                        [Some(forwards), order.reversible().then_some(backwards)]
                    })
                    .flatten()
                    .map(|sort| PickItem {
                        label: sort.label(),
                        detail: if sort.reversed {
                            sort.order.reversed_detail().to_string()
                        } else {
                            String::new()
                        },
                        value: sort.label(),
                    })
                    .collect();
                self.modals.push(Modal::Pick(PickState::new(
                    "Sort by",
                    items,
                    Then::SortPick,
                )));
                Vec::new()
            }
            CommandId::FilterTemplate => {
                let slug = self.library.selected().map(|p| p.template.clone());
                self.set_template_filter(slug)
            }
            CommandId::FilterBase => self.open_base_filter(),
            CommandId::FilterTag => {
                let items = self
                    .library
                    .known_tags
                    .iter()
                    .map(|tag| PickItem {
                        label: tag.clone(),
                        detail: String::new(),
                        value: tag.clone(),
                    })
                    .collect();
                self.modals.push(Modal::Pick(PickState::new(
                    "Filter by tag",
                    items,
                    Then::TagFilter,
                )));
                Vec::new()
            }
            CommandId::ClearFilters => self.clear_filters(),
            CommandId::Actions | CommandId::ActionsEnter => {
                self.modals.push(Modal::Actions(
                    crate::tui::app::actions::ActionsState::default(),
                ));
                Vec::new()
            }
            CommandId::PaneEdit => self.pane_edit_start(),
            CommandId::PaneEditConfirm => self.pane_edit_confirm(),
            CommandId::PaneEditSave => self.pane_edit_save(),
            CommandId::PaneEditCancel => {
                // Esc on the add line is "done": now, or once what it sent
                // has landed. Anywhere else it leaves the row as it was.
                if let Some(pane::PaneEdit::Line {
                    input,
                    target:
                        pane::EditTarget::NewTodo {
                            sending,
                            queued,
                            closing,
                            ..
                        },
                    ..
                }) = &mut self.pane_edit
                    && !(sending.is_empty() && queued.is_empty())
                {
                    input.set_text("");
                    *closing = true;
                    return Vec::new();
                }
                self.close_pane_edit();
                Vec::new()
            }
            CommandId::OpenFolder => self.spawn_for_selection(SpawnKind::Reveal),
            CommandId::OpenTerminal => self.spawn_for_selection(SpawnKind::Terminal),
            CommandId::CopyPath => match self.library.selected() {
                Some(project) => vec![Effect::Spawn(SpawnKind::Clipboard(
                    crate::util::paths::display_path(&project.path),
                ))],
                None => Vec::new(),
            },
            CommandId::ShowPath => {
                if let Some(project) = self.library.selected() {
                    let path = crate::util::paths::display_path(&project.path);
                    self.info(path);
                }
                Vec::new()
            }
            CommandId::ToggleDetail => {
                // Show or hide, literally. A pane on screen is hidden: closed
                // where it shares the body, left for the list where it took
                // the list's place. A pane not on screen is shown: switched
                // on, and gone into where it can only be seen from inside.
                if self.detail_visible() {
                    if self.regions().placement != Some(layout::Placement::Over) {
                        self.detail_open = false;
                    }
                    self.set_focus(Focus::Projects);
                } else {
                    self.detail_open = true;
                    if self.regions().placement == Some(layout::Placement::Over) {
                        self.set_focus(Focus::Detail);
                    }
                }
                self.after_selection_change()
            }
            CommandId::MarkToggle => {
                // Toggle the selected row and move on: marking a run is one
                // keystroke per row, the same shape every mark-and-act list has.
                let Some(path) = self.library.selected().map(|p| p.path.clone()) else {
                    return Vec::new();
                };
                if !self.library.marks.remove(&path) {
                    self.library.marks.insert(path.clone());
                }
                // Where `v` reaches from. Set on a mark *and* on an unmark:
                // the anchor is "the row Space last acted on", so changing
                // your mind about a row does not leave the anchor on it.
                self.library.last_mark = Some(path);
                // In the pane the mark is all: stepping would swap the project
                // under the rows being read.
                if self.focus == Focus::Detail {
                    return Vec::new();
                }
                self.library.step(1);
                self.after_selection_change()
            }
            CommandId::MarkToHere => {
                let added = self.library.mark_to_here();
                let total = self.library.marks.len();
                self.info(format!(
                    "{added} more marked {} {total} in all",
                    self.theme.glyphs.sep
                ));
                Vec::new()
            }
            CommandId::MarkAll => {
                let before = self.library.marks.len();
                for row in 0..self.library.len() {
                    if let Some(project) = self.library.row(row) {
                        self.library.marks.insert(project.path.clone());
                    }
                }
                let now = self.library.marks.len();
                if now == before && before > 0 {
                    self.info(format!("{before} already marked"));
                } else {
                    self.info(format!("{now} marked"));
                }
                Vec::new()
            }
            CommandId::MarkNone => {
                let cleared = self.library.marks.len();
                self.library.marks.clear();
                self.info(format!("{cleared} marks cleared"));
                Vec::new()
            }
            CommandId::AddTag => self.open_add_tag(),
            CommandId::RemoveTags => {
                // Over marks the list is every tag any of them has; a project
                // that lacks one of the picked tags is simply left as it is.
                let targets = self.library.targets();
                let mut tags: Vec<String> = targets
                    .iter()
                    .flat_map(|project| project.tags.iter().cloned())
                    .collect();
                tags.sort();
                tags.dedup();
                if tags.is_empty() {
                    self.warn("no tags to remove");
                    return Vec::new();
                }
                let title = if targets.len() > 1 {
                    format!("Remove tags from {} projects", targets.len())
                } else {
                    "Remove tags".to_string()
                };
                self.modals.push(Modal::MultiPick(MultiPick::new(
                    title,
                    tags,
                    MultiThen::RemoveTags,
                )));
                Vec::new()
            }
            CommandId::ReautoTags => {
                if self.batching() {
                    return self.start_job(jobs::JobKind::ReautoTags, None);
                }
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                self.run_action("re-deriving tags…", Action::ReautoTags(Box::new(project)))
            }
            CommandId::AddNote => {
                // The editor opens once; over marks the text it comes back
                // with goes to every one of them (`Msg::Resumed`).
                let Some(project) = self.library.targets().into_iter().next() else {
                    return Vec::new();
                };
                vec![Effect::Suspend(Suspended::Note(Box::new(project)))]
            }
            CommandId::NoteInline => {
                let count = self.library.targets().len();
                self.modals.push(Modal::Note(NoteState::new(count)));
                Vec::new()
            }
            // Where the pane can show the list, a todo is typed into it,
            // at the end, on a line that opens the next once it lands; where
            // it cannot, the prompt.
            CommandId::AddTodo | CommandId::ListAddTodo => {
                self.start_adding(crate::core::body::TodoPlace::End)
            }
            CommandId::AddPhase => self.start_phase(),
            CommandId::PaneAdd => self.pane_add(),
            CommandId::PaneEditText => self.pane_edit_text(),
            CommandId::ListRename => self.run(CommandId::Rename),
            CommandId::Rename => {
                let Some(project) = self.library.selected().cloned() else {
                    return Vec::new();
                };
                let mut prompt = TextPrompt::new(
                    validators::RENAME_PROMPT,
                    TextThen::Rename(project.path.clone()),
                );
                prompt.input =
                    crate::tui::widgets::input::LineEdit::with_text(project.name.clone());
                self.modals.push(Modal::TextPrompt(prompt));
                Vec::new()
            }
            CommandId::Move => self.open_move_picker(),
            CommandId::CopyTo => {
                let mut prompt = TextPrompt::new(validators::COPY_TO_PROMPT, TextThen::CopyTo);
                prompt.input = crate::tui::widgets::input::LineEdit::default();
                self.modals.push(Modal::TextPrompt(prompt));
                Vec::new()
            }
            CommandId::Unregister => {
                // Nothing is lost by unregistering, so a yes/no is enough —
                // but the question names the folders it is about.
                let names = self.target_names();
                if names.is_empty() {
                    return Vec::new();
                }
                let then = match self.library.selected() {
                    _ if self.batching() => ConfirmThen::UnregisterBatch,
                    Some(project) => ConfirmThen::Unregister(project.path.clone()),
                    None => return Vec::new(),
                };
                self.modals.push(Modal::Confirm(Confirm {
                    prompt: validators::unregister_prompt(&names),
                    then,
                }));
                Vec::new()
            }
            CommandId::Delete => {
                // One word confirms a delete, single or batch, and the prompt
                // names every folder it is about.
                let names = self.target_names();
                if names.is_empty() {
                    return Vec::new();
                }
                let then = match self.library.selected() {
                    Some(project) => TextThen::Delete(project.path.clone()),
                    // A batch still needs a variant; the marks are the target
                    // and the path is ignored.
                    None => TextThen::Delete(std::path::PathBuf::new()),
                };
                self.modals.push(Modal::TextPrompt(TextPrompt::new(
                    validators::delete_prompt(&names),
                    then,
                )));
                Vec::new()
            }

            CommandId::ShowMetadata | CommandId::ShowJournal => self.open_view(id),
            CommandId::NewProject => self.open_create(),
            CommandId::Register => self.open_register(),
            CommandId::ApplyTemplate => self.open_apply(),
            CommandId::Templates => self.toggle_templates(),
            CommandId::Settings => self.open_settings(),
            CommandId::Reconcile => self.run_job(settings::Job::Reconcile),
            CommandId::Attention => self.open_attention(),
            // `f` on the templates tab: filter the library by this template
            // **and go back to it**, since the tab is the one place the answer
            // is not.
            CommandId::StripFilter => {
                let slug = self.studio.selected_slug();
                let next = if slug == self.library.template_filter {
                    None
                } else {
                    slug
                };
                let mut effects = self.toggle_templates();
                effects.extend(self.set_template_filter(next));
                effects
            }
        }
    }

    /// Leave, from whichever gesture asked to.
    ///
    /// **Every quit goes through here** — `q`, the palette's entry and the
    /// too-small-window guard — so each asks what Esc on the same screen
    /// asks before a worked-on template is lost. `close_top` owns the
    /// question; this owns who has to ask it.
    pub(super) fn quit(&mut self, exit: Exit) -> Vec<Effect> {
        // Quitting under a running batch would abandon it between items.
        // A move, a copy, a delete or a reconcile is a job of its own and
        // goes on without the app.
        if self.job.is_some() {
            return self.request_cancel();
        }
        let dirty_builder = matches!(
            self.modals.top(),
            Some(Modal::Builder(builder)) if !builder.saving && builder.is_dirty()
        );
        if dirty_builder {
            let Some(Modal::Builder(builder)) = self.modals.top() else {
                return vec![Effect::Quit(exit)];
            };
            let prompt = validators::discard_template_prompt(builder.original_slug.is_some());
            self.modals.push(Modal::Confirm(Confirm {
                prompt,
                then: ConfirmThen::DiscardTemplate {
                    then_quit: Some(exit),
                },
            }));
            return Vec::new();
        }
        vec![Effect::Quit(exit)]
    }

    /// Esc on a dialog: one level at a time. A builder section goes back to
    /// the section list; the section list closes the builder, asking first
    /// when the template has been worked on; everything else simply closes.
    pub(super) fn close_top(&mut self) -> Vec<Effect> {
        match self.modals.top_mut() {
            Some(Modal::Builder(builder)) => {
                // A save is in flight and the answer is nearly here; closing
                // now would leave the outcome nothing to land on.
                if builder.saving {
                    return Vec::new();
                }
                if builder.open.is_some() {
                    builder.open = None;
                } else if builder.is_dirty() {
                    // **A template that has been worked on is not thrown away
                    // on one keystroke**, and `q` is the key this app teaches
                    // you to close things with everywhere else.
                    let prompt =
                        validators::discard_template_prompt(builder.original_slug.is_some());
                    self.modals.push(Modal::Confirm(Confirm {
                        prompt,
                        then: ConfirmThen::DiscardTemplate { then_quit: None },
                    }));
                } else {
                    self.modals.pop();
                    self.info("Closed — nothing was written.");
                }
            }
            Some(_) => {
                self.modals.pop();
            }
            None => {}
        }
        Vec::new()
    }

    /// Whether there is a pane to put the focus in: the library's whenever it
    /// is switched on — beside the list, under it, or in its place — and the
    /// templates tab's always.
    pub fn pane_present(&self) -> bool {
        self.screen == Screen::Templates || self.pane_live()
    }

    /// The one way focus moves, so every mover leaves the same trace: the
    /// pane it moved to pulses, once, and only when it really moved.
    pub fn set_focus(&mut self, focus: Focus) {
        if self.focus != focus {
            self.focus_moved_at = Some(self.elapsed_ms);
            // An edit belongs to the row it was opened on; leaving the pane
            // leaves the row as it was.
            self.pane_pending = None;
            self.close_pane_edit();
        }
        self.focus = focus;
    }

    fn next_focus(&self, forward: bool) -> Focus {
        let mut ring = vec![Focus::Projects];
        if self.pane_present() {
            ring.push(Focus::Detail);
        }
        let at = ring.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = if forward {
            (at + 1) % ring.len()
        } else {
            (at + ring.len() - 1) % ring.len()
        };
        ring[next]
    }

    /// Start one mutation on a worker.
    ///
    /// **Refused while one is already running.** The runtime answers with the
    /// `ActionId` it was given and `on_action_done` drops anything that is not
    /// the one in flight, so a second action started over the first would make
    /// the first's outcome vanish — the row unpatched, the message never shown.
    /// The command registry's `not_busy` guards the keys; this guards the
    /// screens whose rows are not commands.
    pub(super) fn run_action(&mut self, what: &'static str, action: Action) -> Vec<Effect> {
        if let Some(running) = self.busy {
            self.warn(format!("still {running}"));
            return Vec::new();
        }
        self.next_action += 1;
        let id = ActionId(self.next_action);
        self.busy = Some(what);
        self.busy_id = Some(id);
        vec![Effect::Run(id, Box::new(action))]
    }

    fn spawn_for_selection(&self, kind: fn(Box<Project>) -> SpawnKind) -> Vec<Effect> {
        match self.library.selected() {
            Some(project) => vec![Effect::Spawn(kind(Box::new(project.clone())))],
            None => Vec::new(),
        }
    }

    /// `b`: pick the base to restrict the list to. Every configured base is
    /// offered, mounted or not — an unmounted one showing `0` is the answer to
    /// "where did those projects go", where hiding it is not.
    fn open_base_filter(&mut self) -> Vec<Effect> {
        let Some(bases) = self.summary.as_ref().and_then(Summary::bases_known) else {
            return Vec::new();
        };
        let mut items: Vec<PickItem> = bases
            .iter()
            .map(|base| PickItem {
                label: base.label.clone(),
                detail: base.note(),
                value: base.path.display().to_string(),
            })
            .collect();
        if items.is_empty() {
            return Vec::new();
        }
        // The way out is in the list, not only on a second key: a picker whose
        // only escape is Esc cannot say that "every base" is a choice.
        items.insert(
            0,
            PickItem {
                label: "every base".to_string(),
                detail: String::new(),
                value: String::new(),
            },
        );
        self.modals.push(Modal::Pick(PickState::new(
            "Show which base?",
            items,
            Then::BaseFilter,
        )));
        Vec::new()
    }
}
