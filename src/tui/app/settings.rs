//! Settings, the ID counter and maintenance, as one screen.
//!
//! The menu this replaces was seven submenus deep: every value was one item of
//! one list of one submenu, so seeing what fastf was configured to do meant
//! walking the whole tree and remembering. Here every setting is a row with its
//! current value beside it, and the rows are grouped by heading rather than
//! hidden behind one.
//!
//! **A row's key is the configuration key**, so a row is written by the same
//! `cli::config::apply` the command line calls, and a refusal is the refusal
//! `config set` has always made. Nothing here validates a value itself.

use crate::tui::app::data::Settings;
use crate::tui::widgets::input::LineEdit;
use crate::tui::widgets::nav;
use crate::tui::widgets::text_area::TextArea;

/// What a row does when Enter reaches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Not a row you can land on: the name of the group under it.
    Heading,
    /// A configuration key edited as text.
    Text(&'static str),
    /// A configuration key toggled in place — no dialog for a yes/no.
    Bool(&'static str),
    /// A configuration key cycled through a fixed set.
    Choice(&'static str, &'static [&'static str]),
    /// The library bases, edited as lines of text: one base per line, which is
    /// what the list *is*. The old menu added and removed them one prompt at a
    /// time and could not show you the set you were building.
    Bases,
    /// Something that runs rather than something that is set.
    Run(Job),
}

/// The maintenance verbs, which the command line had and the menu reached only
/// by leaving it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// Raise the global counter (it never goes down).
    RaiseCounter,
    /// Make every mounted base agree on the highest ID seen anywhere.
    SyncCounters,
    /// Rescan every base from its folders.
    Reindex,
    /// Finish or roll back interrupted work.
    Reconcile,
    /// Where fastf keeps its things.
    DataLocations,
}

impl Job {
    pub fn busy(self) -> &'static str {
        match self {
            Job::RaiseCounter => "raising the counter…",
            Job::SyncCounters => "syncing the counters…",
            Job::Reindex => "reindexing…",
            Job::Reconcile => "checking and recovering…",
            Job::DataLocations => "reading…",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
    /// The dimmed line the footer shows while this row has the cursor.
    pub hint: &'static str,
    pub kind: Kind,
}

impl Row {
    pub fn selectable(&self) -> bool {
        self.kind != Kind::Heading
    }
}

/// The first-run question: where projects should live, and what creating it
/// there was refused with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Onboarding {
    pub input: LineEdit,
    pub error: Option<String>,
    /// The folder is being created.
    pub pending: bool,
}

impl Onboarding {
    pub fn new(suggested: String) -> Self {
        Self {
            input: LineEdit::with_text(suggested),
            error: None,
            pending: false,
        }
    }
}

/// What is being edited over the list, if anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Editing {
    /// One value on a line, with the refusal it earned.
    Value {
        key: &'static str,
        label: &'static str,
        input: LineEdit,
        error: Option<String>,
    },
    /// The library bases, one per line.
    Bases {
        area: TextArea,
        error: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsState {
    pub settings: Settings,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub offset: usize,
    pub editing: Option<Editing>,
    /// A worker is reading the settings back.
    pub pending: bool,
}

impl SettingsState {
    pub fn new(settings: Settings) -> Self {
        let rows = rows(&settings);
        let selected = rows.iter().position(Row::selectable).unwrap_or(0);
        Self {
            settings,
            rows,
            selected,
            offset: 0,
            editing: None,
            pending: false,
        }
    }

    /// Rebuild after a write, keeping the cursor on the row it was on.
    pub fn refresh(&mut self, settings: Settings) {
        let keep = self.selected;
        self.settings = settings;
        self.rows = rows(&self.settings);
        self.selected = keep.min(self.rows.len().saturating_sub(1));
        if !self.rows.get(self.selected).is_some_and(Row::selectable) {
            self.step(1);
        }
        self.pending = false;
    }

    pub fn row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Move to the next selectable row, skipping the headings.
    pub fn step(&mut self, delta: isize) {
        let selectable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.selectable())
            .map(|(index, _)| index)
            .collect();
        if selectable.is_empty() {
            return;
        }
        let at = selectable
            .iter()
            .position(|index| *index >= self.selected)
            .unwrap_or(0);
        let next = nav::wrap_step(Some(at), selectable.len(), delta).unwrap_or(0);
        self.selected = selectable[next];
    }

    /// A page or an end: `delta` rows among the selectable ones, clamped —
    /// `isize::MIN` is the first, `isize::MAX` the last.
    pub fn jump(&mut self, delta: isize) {
        let selectable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.selectable())
            .map(|(index, _)| index)
            .collect();
        if selectable.is_empty() {
            return;
        }
        let at = selectable
            .iter()
            .position(|index| *index >= self.selected)
            .unwrap_or(0);
        let next = nav::clamp_jump(Some(at), selectable.len(), delta).unwrap_or(0);
        self.selected = selectable[next];
    }

    pub fn clamp_viewport(&mut self, rows: usize) {
        self.offset = nav::viewport_offset(self.offset, Some(self.selected), self.rows.len(), rows);
    }

    /// Open the editor the selected row wants, if it is edited rather than run.
    pub fn begin_edit(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        self.editing = match &row.kind {
            Kind::Text(key) => Some(Editing::Value {
                key,
                label: row.label,
                input: LineEdit::with_text(raw_value(&self.settings, key)),
                error: None,
            }),
            Kind::Bases => Some(Editing::Bases {
                area: TextArea::with_text(&self.settings.bases.join("\n")),
                error: None,
            }),
            _ => None,
        };
    }

    /// The `(key, value)` a toggle or a cycle writes, without opening anything.
    pub fn immediate_write(&self) -> Option<(&'static str, String)> {
        match &self.rows.get(self.selected)?.kind {
            Kind::Bool(key) => {
                let on = raw_value(&self.settings, key) == "true";
                Some((key, (!on).to_string()))
            }
            Kind::Choice(key, options) => {
                let current = raw_value(&self.settings, key);
                let at = options.iter().position(|o| *o == current).unwrap_or(0);
                Some((key, options[(at + 1) % options.len()].to_string()))
            }
            _ => None,
        }
    }

    /// What the open editor would write.
    pub fn pending_write(&self) -> Option<(&'static str, String)> {
        match self.editing.as_ref()? {
            Editing::Value { key, input, .. } => Some((key, input.text().to_string())),
            // `config set bases` takes the comma-separated list, which is what
            // the lines are once the blank ones are dropped.
            Editing::Bases { area, .. } => Some(("bases", area.entries().join(","))),
        }
    }

    /// Put a refusal on whatever is open, keeping the text that earned it.
    pub fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        match &mut self.editing {
            Some(Editing::Value { error, .. }) | Some(Editing::Bases { error, .. }) => {
                *error = Some(message);
            }
            None => {}
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self.editing.as_ref()? {
            Editing::Value { error, .. } | Editing::Bases { error, .. } => error.as_deref(),
        }
    }
}

/// The value as `config set` would take it, which is not always what the row
/// shows: a row says `(always ask)` where the setting is the empty string.
pub fn raw_value(settings: &Settings, key: &str) -> String {
    match key {
        "base-dir" => settings.base_dir.clone(),
        "editor" => settings.editor.clone(),
        "terminal" => settings.terminal.clone(),
        "theme" => or(&settings.theme, "auto"),
        "default-template" => settings.default_template.clone(),
        "date-format" => settings.date_format.clone(),
        "register-naming-pattern" => settings.register_naming_pattern.clone(),
        "preview-lines" => settings.preview_lines.to_string(),
        "recent-limit" => settings.recent_default_limit.to_string(),
        "prompt-open-after-create" => settings.prompt_open_after_create.to_string(),
        "confirm-create" => settings.confirm_create.to_string(),
        "on-name-collision" => settings.on_name_collision.clone(),
        "post_create.git_init" => settings.git_init.to_string(),
        "post_create.reveal" => settings.reveal.to_string(),
        "post_create.open_in_editor" => settings.open_in_editor.to_string(),
        "post_create.print_path" => settings.print_path.to_string(),
        _ => String::new(),
    }
}

fn or(value: &str, empty: &str) -> String {
    if value.trim().is_empty() {
        empty.to_string()
    } else {
        value.to_string()
    }
}

fn yes_no(on: bool) -> String {
    if on { "yes" } else { "no" }.to_string()
}

fn heading(label: &'static str) -> Row {
    Row {
        label,
        value: String::new(),
        hint: "",
        kind: Kind::Heading,
    }
}

fn text(label: &'static str, key: &'static str, value: String, hint: &'static str) -> Row {
    Row {
        label,
        value,
        hint,
        kind: Kind::Text(key),
    }
}

fn toggle(label: &'static str, key: &'static str, on: bool, hint: &'static str) -> Row {
    Row {
        label,
        value: yes_no(on),
        hint,
        kind: Kind::Bool(key),
    }
}

fn run(label: &'static str, job: Job, value: String, hint: &'static str) -> Row {
    Row {
        label,
        value,
        hint,
        kind: Kind::Run(job),
    }
}

const COLLISION: &[&str] = &["suffix", "error"];
const THEMES: &[&str] = &crate::tui::theme::ThemeChoice::NAMES;

/// Every setting fastf has, grouped, with what it is set to now.
pub fn rows(s: &Settings) -> Vec<Row> {
    vec![
        heading("Project basics"),
        text(
            "Base directory",
            "base-dir",
            or(&s.base_dir, "(your home directory)"),
            "where new projects are created — empty falls back to your home directory, never the current one",
        ),
        text(
            "Default template",
            "default-template",
            or(&s.default_template, "(always ask)"),
            "the template `fastf new` and the wizard open on; empty asks every time",
        ),
        text(
            "Date format",
            "date-format",
            format!("{}   (today: {})", s.date_format, s.date_preview),
            "strftime, e.g. %Y-%m-%d — it is what {date} renders as",
        ),
        text(
            "Editor",
            "editor",
            or(&s.editor, "($EDITOR)"),
            "the command a journal note opens in; empty uses $EDITOR",
        ),
        text(
            "Terminal",
            "terminal",
            or(&s.terminal, "(probe the known ones)"),
            "the emulator to open when fastf is started without one; \"none\" never opens a window",
        ),
        text(
            "Register pattern",
            "register-naming-pattern",
            s.register_naming_pattern.clone(),
            "what `register --rename` names a folder with no template; must contain {id}",
        ),
        heading("Creating a project"),
        toggle(
            "Confirm before creating",
            "confirm-create",
            s.confirm_create,
            "off commits the plan as soon as it is built, without showing it",
        ),
        toggle(
            "Ask to open after creating",
            "prompt-open-after-create",
            s.prompt_open_after_create,
            "`fastf new` offers to open the new folder in the file manager",
        ),
        text(
            "Preview lines",
            "preview-lines",
            s.preview_lines.to_string(),
            "how many lines of each template file a preview shows",
        ),
        Row {
            label: "Name collision",
            value: s.on_name_collision.clone(),
            hint: "suffix gives a taken folder name _2, _3, …; error refuses it",
            kind: Kind::Choice("on-name-collision", COLLISION),
        },
        heading("Appearance"),
        Row {
            label: "Theme",
            value: or(&s.theme, "auto (follows the terminal)"),
            hint: "auto follows what the terminal announces; mono, ansi or rich force a palette — FASTF_THEME overrides for one run",
            kind: Kind::Choice("theme", THEMES),
        },
        heading("Library bases"),
        Row {
            label: "Bases",

            value: match s.bases.len() {
                0 => "(only the base directory)".to_string(),
                n => format!("{n} extra"),
            },
            hint: "one folder per line — Enter opens the list, Ctrl-S keeps it",
            kind: Kind::Bases,
        },
        text(
            "Recent limit",
            "recent-limit",
            s.recent_default_limit.to_string(),
            "the default --limit for `fastf recent`",
        ),
        heading("After a project is created"),
        toggle(
            "git init",
            "post_create.git_init",
            s.git_init,
            "run `git init` in the new folder",
        ),
        toggle(
            "Reveal the folder",
            "post_create.reveal",
            s.reveal,
            "open it in the system file manager",
        ),
        toggle(
            "Open in the editor",
            "post_create.open_in_editor",
            s.open_in_editor,
            "spawn the configured editor on the new folder",
        ),
        toggle(
            "Print the path",
            "post_create.print_path",
            s.print_path,
            "print the absolute path on its own line, for `cd \\\"$(fastf new …)\\\"`",
        ),
        heading("ID counter"),
        run(
            "Counter",
            Job::RaiseCounter,
            format!("{}   next: {}", s.counter_floor, s.next_id),
            "Enter raises it — the counter is the highest ID seen anywhere and never goes down",
        ),
        run(
            "Sync every base",
            Job::SyncCounters,
            String::new(),
            "make every mounted base agree on that number — after copying projects in from elsewhere",
        ),
        heading("Maintenance"),
        run(
            "Reindex",
            Job::Reindex,
            String::new(),
            "rescan every base from its folders and rebuild the caches",
        ),
        run(
            "Check and recover",
            Job::Reconcile,
            match s.attention {
                0 => String::new(),
                1 => "1 needs attention".to_string(),
                n => format!("{n} need attention"),
            },
            "finish or roll back work a crash left half-done",
        ),
        run(
            "Data locations",
            Job::DataLocations,
            s.data_dir.clone(),
            "where the config, the counter and the templates live, and how that was decided",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        Settings {
            base_dir: "/mnt/projects".to_string(),
            bases: vec!["/media/usb/archive".to_string()],
            date_format: "%Y-%m-%d".to_string(),
            date_preview: "2026-09-03".to_string(),
            preview_lines: 20,
            confirm_create: true,
            recent_default_limit: 20,
            register_naming_pattern: "{date}_{name}_{id}".to_string(),
            on_name_collision: "suffix".to_string(),
            counter_floor: 248,
            next_id: "ID0249".to_string(),
            data_dir: "/home/user/.config/fastf".to_string(),
            ..Settings::default()
        }
    }

    #[test]
    fn a_heading_is_never_landed_on() {
        let mut state = SettingsState::new(sample());
        assert!(state.row().unwrap().selectable());
        for _ in 0..state.rows.len() * 2 {
            state.step(1);
            assert!(state.row().unwrap().selectable(), "{:?}", state.row());
        }
    }

    #[test]
    fn a_toggle_writes_the_other_value_with_no_dialog() {
        let mut state = SettingsState::new(sample());
        while state.row().unwrap().label != "Confirm before creating" {
            state.step(1);
        }
        assert_eq!(
            state.immediate_write(),
            Some(("confirm-create", "false".to_string()))
        );
        state.begin_edit();
        assert!(state.editing.is_none(), "a yes/no needs no editor");
    }

    #[test]
    fn a_choice_cycles_through_its_options() {
        let mut state = SettingsState::new(sample());
        while state.row().unwrap().label != "Name collision" {
            state.step(1);
        }
        assert_eq!(
            state.immediate_write(),
            Some(("on-name-collision", "error".to_string()))
        );
    }

    #[test]
    fn a_text_row_opens_with_the_value_config_set_would_take() {
        let mut state = SettingsState::new(sample());
        while state.row().unwrap().label != "Default template" {
            state.step(1);
        }
        // The row *shows* `(always ask)`; the editor opens on the empty string
        // the setting actually holds.
        assert_eq!(state.row().unwrap().value, "(always ask)");
        state.begin_edit();
        assert_eq!(
            state.pending_write(),
            Some(("default-template", String::new()))
        );
    }

    #[test]
    fn the_bases_are_edited_as_lines_and_written_as_a_list() {
        let mut state = SettingsState::new(sample());
        while state.row().unwrap().label != "Bases" {
            state.step(1);
        }
        state.begin_edit();
        let Some(Editing::Bases { area, .. }) = &mut state.editing else {
            panic!("expected the base list");
        };
        area.paste("\n/mnt/second");
        assert_eq!(
            state.pending_write(),
            Some(("bases", "/media/usb/archive,/mnt/second".to_string()))
        );
    }

    #[test]
    fn a_refusal_keeps_the_text_that_earned_it() {
        let mut state = SettingsState::new(sample());
        while state.row().unwrap().label != "Recent limit" {
            state.step(1);
        }
        state.begin_edit();
        state.fail("recent_default_limit must be at least 1");
        assert_eq!(
            state.error(),
            Some("recent_default_limit must be at least 1")
        );
        assert_eq!(
            state.pending_write(),
            Some(("recent-limit", "20".to_string())),
            "the value is still there to be corrected"
        );
    }
}
