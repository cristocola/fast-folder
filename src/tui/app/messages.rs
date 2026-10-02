//! `App::handle`: one message in, effects out. A worker's answer, a paste, a
//! tick.

use super::*;

impl App {
    pub(super) fn handle(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Key(key) => self.on_key(key),
            Msg::Paste(text) => self.on_paste(&text),
            Msg::Resize(width, height) => self.on_resize(width, height),
            Msg::Tick => self.on_tick(),
            Msg::Sizes(cells) => self.on_sizes(cells),
            Msg::Summary(summary) => {
                self.summary = Some(*summary);
                self.summary_error = None;
                // A template written or deleted is a change to the templates
                // tab's list, whether or not that tab is the one on screen.
                let mut effects = self.refresh_templates();
                effects.extend(self.maybe_finish_leftovers());
                effects
            }
            Msg::SummaryPart { generation, part } => self.install_summary_part(generation, *part),
            Msg::SummaryFailed(error) => {
                self.summary_error = Some(error.clone());
                self.error(format!("the library summary could not be read: {error}"));
                Vec::new()
            }
            Msg::Discovered {
                generation,
                projects,
            } => {
                if !self.library.install(generation, projects) {
                    return Vec::new();
                }
                let mut effects = self.rows_arrived();
                effects.extend(self.discovery_over());
                effects
            }
            Msg::DiscoveryPlanned { generation, bases } => {
                if self.library.plan(generation, bases) {
                    self.rows_arrived()
                } else {
                    Vec::new()
                }
            }
            Msg::DiscoveredBase {
                generation,
                base,
                projects,
            } => {
                if self.library.install_base(generation, base, projects) {
                    self.rows_arrived()
                } else {
                    Vec::new()
                }
            }
            Msg::DiscoverySettled { generation, silent } => {
                if !self.library.settle(generation, silent) {
                    return Vec::new();
                }
                let mut effects = self.rows_arrived();
                effects.extend(self.discovery_over());
                effects
            }
            Msg::DiscoverFailed { generation, error } => self.on_discover_failed(generation, error),
            Msg::Detail { path, detail } => self.on_detail(path, detail),
            Msg::MetaLoaded(loaded) => {
                self.library.absorb_meta(loaded);
                self.recompute();
                self.after_rows_changed()
            }
            Msg::Jobs(jobs) => self.on_jobs(jobs),
            Msg::JobStarted(started) => self.on_job_started(started),
            Msg::AutoReconcileStarted(started) => self.on_auto_reconcile_started(started),
            Msg::TemplateLoaded { slug, result } => self.on_template_loaded(&slug, result),
            Msg::TemplateSourceLoaded { slug, result } => {
                self.on_template_source_loaded(slug, result)
            }
            Msg::TemplateViewLoaded { slug, lines } => {
                if self.studio.selected_slug().as_deref() == Some(slug.as_str()) {
                    self.studio.shown = Some(slug);
                    self.studio.lines = lines;
                    self.studio.scroll = 0;
                }
                Vec::new()
            }
            Msg::SettingsLoaded(loaded) => self.on_settings_loaded(loaded),
            Msg::Themed { theme, motion } => {
                self.theme = *theme;
                self.motion = motion;
                Vec::new()
            }

            Msg::SettingsFailed(error) => {
                if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                    state.pending = false;
                }
                self.error(format!("the settings could not be read: {error}"));
                Vec::new()
            }
            Msg::Previewed(preview) => self.on_previewed(*preview),
            Msg::PreviewFailed { field, error } => {
                let Some(Modal::Flow(flow)) = self.modals.top_mut() else {
                    return Vec::new();
                };
                flow.pending = false;
                flow.step = Step::Form;
                flow.form.fail(field.as_deref(), error);
                Vec::new()
            }
            Msg::ActivityLoaded { messages, log } => {
                let g = self.theme.glyphs;
                if let Some(Modal::Activity(activity)) = self.modals.top_mut() {
                    activity.messages = crate::tui::app::modal::message_rows(&messages, &g);
                    activity.log = crate::tui::app::modal::log_rows(&log);
                    activity.loaded = true;
                }
                // A page that shrank must not leave its scroll past the end.
                self.scroll_top_modal(0)
            }
            Msg::ViewLoaded {
                request,
                title,
                lines,
            } => self.on_view_loaded(request, title, lines),
            Msg::ActionDone { id, outcome } => self.on_action_done(id, outcome),
            Msg::Spawned { what, outcome } => self.on_spawned(what, outcome),
            Msg::Resumed(Resumed::PostCreate) => {
                self.session = crate::tui::frame::recent_actions();
                Vec::new()
            }
            Msg::Resumed(Resumed::Shell) => Vec::new(),

            Msg::Resumed(Resumed::Note { project, text }) => {
                self.on_note_from_editor(project, text)
            }

            Msg::Diag(level, text) => {
                match level {
                    Level::Warn => self.warn(format!("warning: {text}")),
                    Level::Note => self.info(format!("note: {text}")),
                }
                Vec::new()
            }
            Msg::Interrupted => vec![Effect::Quit(Exit::Interrupted)],
        }
    }

    fn on_tick(&mut self) -> Vec<Effect> {
        if self
            .status
            .expires_at
            .is_some_and(|at| at <= self.elapsed_ms)
        {
            self.status = Status::default();
        }
        self.pulses.retire(self.elapsed_ms);
        self.pane_pulses.retire(self.elapsed_ms);
        if !motion::focus_easing(self.focus_moved_at, self.elapsed_ms) {
            self.focus_moved_at = None;
        }
        self.look_again_while_waiting()
    }

    fn on_sizes(&mut self, cells: Vec<(PathBuf, Option<u64>)>) -> Vec<Effect> {
        for (path, size) in cells {
            // **A number that changed, not a number that arrived.** The
            // table is measured from the rows, never from the sizes, so
            // a landing number cannot reflow anything — the only
            // question a pulse answers here is whether the figure you
            // are looking at is the one that was there a moment ago.
            // The first fill of a row cannot be that, and every visible
            // row fills at once on the first screenful and on every
            // scroll: `rescanning` is the one arrival that *is* a
            // change, a size a verb threw away coming back.
            let previous = self.library.sizes.insert(path.clone(), size);
            let rescanned = self.rescanning.remove(&path);
            if rescanned || previous.is_some_and(|had| had != size) {
                self.pulses.start(path, self.elapsed_ms);
            }
        }
        if self.library.effective_sort(&self.search.query).order == Order::Size {
            self.recompute();
            let rows = self.rows_on_screen();
            self.library.clamp_viewport(rows);
        }
        Vec::new()
    }

    fn on_discover_failed(&mut self, generation: u64, error: String) -> Vec<Effect> {
        if self.library.inflight == Some(generation) {
            self.library.inflight = None;
            self.library.loaded = true;
            self.library.error = Some(error.clone());
            self.modals.push(Modal::message(
                "the library could not be read",
                format!(
                    "{error}\n\nfix the configuration (`fastf config show`), then reload with {}.",
                    command::key_of(CommandId::Reload)
                ),
                MessageLevel::Error,
            ));
        }
        Vec::new()
    }

    fn on_detail(&mut self, path: PathBuf, detail: Box<ProjectDetail>) -> Vec<Effect> {
        if self.details.len() >= DETAIL_CACHE {
            self.details.clear();
        }
        let mut effects = Vec::new();
        // The file is the truth, and the pane just read it: a row
        // whose tags or names disagree with the metadata — edited
        // outside the app, or an index that went stale — takes the
        // file's, and the base's index entry is written to match.
        if let Some(meta) = &detail.meta
            && let Some(mut row) = self.project_at(&path)
        {
            let mut changed = false;
            if row.tags != meta.tags {
                row.tags = meta.tags.clone();
                changed = true;
            }
            if !meta.template_name.is_empty() && row.template_name != meta.template_name {
                row.template_name = meta.template_name.clone();
                changed = true;
            }
            if !meta.created.is_empty() && row.created != meta.created {
                row.created = meta.created.clone();
                changed = true;
            }
            if changed && self.library.patch(&path, row) {
                self.recompute();
                let rows = self.rows_on_screen();
                self.library.clamp_viewport(rows);
                effects.push(Effect::RefreshCache(path.clone()));
            }
        }
        let selected = self.library.selected().is_some_and(|p| p.path == path);
        self.details.insert(path, *detail);
        if selected {
            // A landed write settles on its row and pulses there;
            // anything else — a re-read after an outside edit, the
            // first read — finds the cursor and an open edit again
            // by what they are on, and moves nothing.
            if let Some(target) = self.pane_return.take() {
                self.settle_pane_cursor(&target);
            }
            self.refind_pane();
            self.land_pane_seek();
        }
        effects
    }

    fn on_template_source_loaded(
        &mut self,
        slug: String,
        result: Result<Box<crate::core::template::Template>, String>,
    ) -> Vec<Effect> {
        let Some(Modal::Builder(builder)) = self.modals.top_mut() else {
            return Vec::new();
        };
        // A read for a template this builder is no longer waiting on is
        // an answer to a question nobody is asking any more — the same
        // guard `on_template_loaded` and `TemplateViewLoaded` make. Esc
        // out of one pending builder and open another, and on a slow
        // disk the first read would arrive and become the second's
        // contents, wiping anything typed meanwhile.
        if builder.pending.as_deref() != Some(slug.as_str()) {
            return Vec::new();
        }
        builder.pending = None;
        match result {
            Ok(template) => **builder = Builder::new(Some(*template)),
            Err(error) => {
                self.modals.pop();
                self.error(format!("template '{slug}' could not be read: {error}"));
            }
        }
        Vec::new()
    }

    fn on_settings_loaded(&mut self, loaded: Box<data::Settings>) -> Vec<Effect> {
        // The screen went up when `,` was pressed, saying it was
        // reading; a read that lands after it was closed has nothing
        // to fill in and is dropped.
        let (theme, motion) = (loaded.theme.clone(), loaded.motion.clone());
        let rows = layout::settings_rows(self.area());
        if let Some(Modal::Settings(state)) = self.modals.top_mut() {
            state.refresh(*loaded);
            // A screen opened on one row lands with that row in view.
            state.clamp_viewport(rows);
        }
        // A theme — or a motion setting — written on this screen takes
        // effect on the frame that shows it was written.
        vec![Effect::Retheme { theme, motion }]
    }

    fn on_view_loaded(&mut self, request: u64, title: String, lines: Vec<String>) -> Vec<Effect> {
        // The dialog went up when the key was pressed, saying it was
        // reading; fill it in if it is still the one on top and this is
        // the read it asked for, else the user has moved on and the read
        // is dropped.
        if request != self.view_request {
            return Vec::new();
        }
        if let Some(Modal::Message {
            title: shown,
            lines: body,
            ..
        }) = self.modals.top_mut()
            && *shown == title
        {
            *body = lines;
        }
        Vec::new()
    }

    fn on_note_from_editor(&mut self, project: Box<Project>, text: Option<String>) -> Vec<Effect> {
        self.session = crate::tui::frame::recent_actions();
        // The editor ran once; the note goes to what it was opened about.
        let targets = self
            .editor_note_for
            .take()
            .unwrap_or_else(|| crate::tui::app::actions::Targets::one(project.path.clone()));
        match text {
            Some(text) if !text.trim().is_empty() => self.add_note(text, &targets),
            _ => {
                self.info("no note written");
                Vec::new()
            }
        }
    }

    fn on_action_done(
        &mut self,
        id: ActionId,
        outcome: Result<Box<ActionOutcome>, String>,
    ) -> Vec<Effect> {
        if self.busy_id != Some(id) {
            return Vec::new();
        }
        self.busy = None;
        self.busy_id = None;
        if self.job.is_some() {
            return self.on_job_item_done(outcome);
        }
        match outcome {
            Ok(outcome) => self.on_action_succeeded(outcome),
            Err(error) => self.on_action_failed(error),
        }
    }

    fn on_action_succeeded(&mut self, outcome: Box<ActionOutcome>) -> Vec<Effect> {
        let outcome = *outcome;
        if let Some(entry) = outcome.session {
            crate::tui::frame::record(entry);
            self.session = crate::tui::frame::recent_actions();
        }
        self.report_outcome(outcome.message, outcome.warning);
        if let Some(path) = outcome.select {
            self.select_when_found = Some(path);
        }
        let reload_settings = outcome.reload_settings;
        // The first-run question is answered once the folder exists.
        if matches!(self.modals.top(), Some(Modal::Onboarding(_))) {
            self.modals.pop();
        }
        // A template that saved is on disk; the builder holding it has
        // nothing left to hold. Only now, and only here — see
        // `save_template`.
        if matches!(self.modals.top(), Some(Modal::Builder(builder)) if builder.saving) {
            self.modals.pop();
        }
        // A pane edit that landed: the edit closes, the row it was on
        // pulses, and the cursor stays where the edit was made —
        // `apply_change` would otherwise put it back on the name,
        // which is the one row nobody who just changed a variable is
        // looking at.
        //
        // **The add line stays open** — it takes keys all along — and
        // the todo it sent pulses where the writer says it put it
        // (`todo_ordinal`), once the re-read shows it; a guess at that
        // place is wrong wherever a phase's name repeats.
        let landed = self.take_landed_pane_edit(outcome.todo_ordinal);
        let mut effects = self.apply_change(outcome.change);
        if let Some(target) = landed {
            // Settled now where its row is already there; the new
            // todo's is not, until the re-read lands.
            self.settle_pane_cursor(&target);
            // The detail was just dropped and will be read again;
            // the cursor finds the row once more when it lands.
            self.pane_return = Some(target);
        }
        // What was entered on the add line while this was written
        // goes next, or the line closes as it was asked to.
        effects.extend(self.flush_adds());
        if let Some(FollowUp::PostCreate {
            root,
            template_slug,
        }) = outcome.follow_up
        {
            effects.push(Effect::Suspend(Suspended::PostCreate {
                root,
                template_slug,
            }));
        }
        if reload_settings && matches!(self.modals.top(), Some(Modal::Settings(_))) {
            if let Some(Modal::Settings(state)) = self.modals.top_mut() {
                state.editing = None;
                state.pending = true;
            }
            effects.push(Effect::LoadSettings);
        }
        effects
    }

    /// The status line for an action that went through; a dialog if its warning needs one.
    fn report_outcome(&mut self, message: String, warning: Option<String>) {
        match warning {
            Some(warning) if needs_a_dialog(&warning) => {
                self.warn(format!("{}  —  see the report", message));
                self.modals
                    .push(Modal::message("needs a look", warning, MessageLevel::Warn));
            }
            Some(warning) => self.warn(format!("{}  —  warning: {warning}", message)),
            None => self.good(message),
        }
    }

    /// The row a landed pane write settles on; the add line itself stays open.
    fn take_landed_pane_edit(&mut self, todo_ordinal: Option<usize>) -> Option<pane::PaneTarget> {
        if self
            .pane_edit
            .as_ref()
            .is_some_and(pane::PaneEdit::adding_in_flight)
        {
            self.adds_landed(todo_ordinal);
            todo_ordinal.map(pane::PaneTarget::Todo)
        } else {
            self.pane_edit
                .take_if(|edit| edit.pending())
                .map(|edit| edit.target())
                .or_else(|| self.pane_pending.take())
                .map(|target| match todo_ordinal {
                    Some(ordinal) => pane::PaneTarget::Todo(ordinal),
                    None => target,
                })
        }
    }

    fn on_action_failed(&mut self, error: String) -> Vec<Effect> {
        // A refusal belongs on the field that earned it, wherever one
        // is open: `config set`'s own message, under the value that is
        // still there to be corrected.
        if self
            .pane_edit
            .as_ref()
            .is_some_and(pane::PaneEdit::adding_in_flight)
        {
            self.adds_refused(error);
            return Vec::new();
        }
        if let Some(edit) = &mut self.pane_edit
            && edit.pending()
        {
            edit.fail(error);
            return Vec::new();
        }
        match self.modals.top_mut() {
            Some(Modal::Settings(state)) if state.editing.is_some() => {
                state.pending = false;
                state.fail(error);
            }
            Some(Modal::Onboarding(state)) => {
                state.pending = false;
                state.error = Some(error);
            }
            // The refusal goes under the list the template is still
            // sitting in, in the same place `Cannot save:` already
            // appears, with every answer untouched.
            Some(Modal::Builder(builder)) if builder.saving => {
                builder.saving = false;
                builder.error = Some(format!("Cannot save: {error}"));
            }
            _ if needs_a_dialog(&error) => {
                let first = error.lines().next().unwrap_or_default().to_string();
                self.error(format!("error: {first}"));
                self.modals
                    .push(Modal::message("error", error, MessageLevel::Error));
            }
            _ => self.error(format!("error: {error}")),
        }
        Vec::new()
    }

    fn on_spawned(&mut self, what: SpawnKind, outcome: Result<String, String>) -> Vec<Effect> {
        match (what, outcome) {
            (SpawnKind::Reveal(project), Ok(_)) => {
                self.good(format!("Opened {} in the file manager", project.name));
            }
            (SpawnKind::Reveal(_), Err(error)) => {
                self.error(format!("could not open the folder: {error}"))
            }
            (SpawnKind::Terminal(_), Ok(_)) => {
                self.good("Terminal opened");
            }
            (SpawnKind::Terminal(_), Err(error)) => {
                self.error(format!("could not open a terminal: {error}"));
            }
            (SpawnKind::Clipboard(_), Ok(tool)) => {
                self.good(format!("Copied with {tool}"));
            }
            (SpawnKind::Clipboard(text), Err(_)) => {
                self.modals.push(Modal::message(
                    "no clipboard tool found — here is the path:",
                    format!("{text}\n\ninstall wl-copy, xclip or xsel to copy from here."),
                    MessageLevel::Warn,
                ));
            }
        }
        Vec::new()
    }

    /// Pasted text goes into whichever field has the caret, and nowhere
    /// else. A single-line field takes the first line and says how many it
    /// dropped; a text area takes them all; with no field open the paste is
    /// ignored and said so — it is never read as keystrokes, or a pasted
    /// paragraph would run a dozen commands.
    fn on_paste(&mut self, text: &str) -> Vec<Effect> {
        // **A line ends however the terminal ends it.** A bracketed paste
        // carries the clipboard's line breaks as the terminal sends them, and
        // many send a bare carriage return; `str::lines` splits on `\n`
        // alone, so without this a multi-line paste arrives as one line with
        // its breaks dropped — a note run together, a list as one todo.
        let text = &text.replace("\r\n", "\n").replace('\r', "\n");
        // A list pasted onto the add line is that many todos, at once.
        if self.modals.is_empty()
            && !self.search.editing
            && self
                .pane_edit
                .as_ref()
                .is_some_and(|edit| edit.is_adding() && !edit.pending())
            && let Some(effects) = self.paste_todos(text)
        {
            return effects;
        }
        let lines = text.lines().count();
        let dropped = lines.saturating_sub(1);
        let (effects, kept_first) = self.paste_into_field(text);
        if kept_first && dropped > 0 {
            self.warn(format!(
                "pasted {lines} lines — kept the first, this field takes one"
            ));
        }
        effects
    }

    /// The paste given to the field with the caret; `true` beside the effects
    /// when that field took only the first line.
    fn paste_into_field(&mut self, text: &str) -> (Vec<Effect>, bool) {
        let first = text.lines().next().unwrap_or_default().to_string();
        let mut kept_first = false;
        let effects = match self.modals.top_mut() {
            Some(Modal::Palette(palette)) => {
                palette.input.paste(&first);
                kept_first = true;
                self.refresh_palette();
                Vec::new()
            }
            Some(Modal::Pick(pick)) => {
                pick.query.paste(&first);
                kept_first = true;
                pick.rank(&mut self.fuzzy);
                Vec::new()
            }
            Some(Modal::TextPrompt(prompt)) => {
                prompt.input.paste(&first);
                prompt.error = None;
                kept_first = true;
                Vec::new()
            }
            Some(Modal::Note(note)) => {
                note.area.paste(text);
                Vec::new()
            }
            Some(Modal::Flow(flow)) => {
                if let Some(field) = flow.form.focused_mut() {
                    field.paste(&first);
                    kept_first = true;
                }
                Vec::new()
            }
            Some(Modal::Builder(builder)) => {
                kept_first = paste_into_builder(builder, text, &first);
                Vec::new()
            }
            Some(Modal::Settings(state)) => {
                kept_first = paste_into_settings(state, text, &first);
                Vec::new()
            }
            Some(Modal::Onboarding(state)) => {
                state.input.paste(&first);
                kept_first = true;
                Vec::new()
            }
            Some(_) => {
                self.info("pasted text ignored — nothing here takes typing");
                Vec::new()
            }
            None if self.search.editing => {
                self.search.input.paste(&first);
                kept_first = true;
                self.after_query_change()
            }
            // An edit open in the pane: a line takes the first line, a note
            // every line — and nothing while its write is on its way.
            None if self.pane_edit.as_ref().is_some_and(|edit| !edit.pending()) => {
                kept_first = self.paste_into_pane_edit(text, &first);
                Vec::new()
            }
            None => {
                self.info(format!(
                    "pasted text ignored — press {} to search, or open a field first",
                    command::key_of(CommandId::Search)
                ));
                Vec::new()
            }
        };
        (effects, kept_first)
    }

    /// `true` when the pane's edit took only the first line.
    fn paste_into_pane_edit(&mut self, text: &str, first: &str) -> bool {
        let mut kept_first = false;
        if let Some(edit) = &mut self.pane_edit {
            edit.clear_error();
            match edit {
                pane::PaneEdit::Line { input, .. } => {
                    input.paste(first);
                    kept_first = true;
                }
                pane::PaneEdit::Note { area, .. } => area.paste(text),
            }
        }
        kept_first
    }
}

/// `true` when the builder's open field took only the first line.
fn paste_into_builder(builder: &mut Builder, text: &str, first: &str) -> bool {
    let mut kept_first = false;
    match &mut builder.open {
        Some(Open::Metadata(form)) | Some(Open::Id(form)) => {
            if let Some(field) = form.focused_mut() {
                field.paste(first);
                kept_first = true;
            }
        }
        Some(Open::Variables(list)) => {
            if let Some((_, form)) = &mut list.editing
                && let Some(field) = form.focused_mut()
            {
                field.paste(first);
                kept_first = true;
            }
        }
        Some(Open::Structure(area)) => area.paste(text),
        Some(Open::Files(list)) => {
            if let Some(edit) = &mut list.editing {
                if edit.in_body {
                    edit.body.paste(text);
                } else {
                    edit.path.paste(first);
                    kept_first = true;
                }
            }
        }
        None => {}
    }
    kept_first
}

/// `true` when the settings field being edited took only the first line.
fn paste_into_settings(state: &mut settings::SettingsState, text: &str, first: &str) -> bool {
    let mut kept_first = false;
    match &mut state.editing {
        Some(Editing::Value { input, error, .. }) => {
            input.paste(first);
            *error = None;
            kept_first = true;
        }
        Some(Editing::Bases { area, .. }) => area.paste(text),
        Some(Editing::Filter) => {
            state.filter.paste(first);
            state.apply_filter();
            kept_first = true;
        }
        None => {}
    }
    kept_first
}
