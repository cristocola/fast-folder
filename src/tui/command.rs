//! The one list of everything the guided app can do.
//!
//! A command is declared once — its title, its description, the contexts it
//! fires in, its default keys, whether the palette and the hint bar show it —
//! and every surface that names a command reads it from here: the keymap
//! (`lookup`), the fuzzy palette (`palette_entries`), the help overlay
//! (`help_sections`) and the hint bar (`hints`). The prototype this replaces
//! carried four copies of its key table, and they had already drifted.
//!
//! `tests/tui_commands.rs` holds the invariants: no two commands share a key in
//! one context, every command has a title and a description, every id is
//! declared exactly once.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;

/// One keystroke, normalised: shift is folded into the character, Ctrl and Alt
/// are flags. `KeyCode::Char('c')` with the control flag is Ctrl-C.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl Key {
    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
        }
    }

    pub const fn ch(c: char) -> Self {
        Self::plain(KeyCode::Char(c))
    }

    pub const fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            ctrl: true,
            alt: false,
        }
    }

    /// The text the hint bar and the help overlay print for this key.
    pub fn label(&self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "Shift-Tab".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Del".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        };
        match (self.ctrl, self.alt) {
            (true, true) => format!("Ctrl-Alt-{base}"),
            (true, false) => format!("Ctrl-{base}"),
            (false, true) => format!("Alt-{base}"),
            (false, false) => base,
        }
    }

    /// A printable character with no modifier — what a text field inserts.
    pub fn typed(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt && !c.is_control() => Some(c),
            _ => None,
        }
    }
}

impl From<KeyEvent> for Key {
    fn from(event: KeyEvent) -> Self {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        let code = match event.code {
            // Ctrl-Shift-P and Ctrl-p are the same chord; a terminal reports
            // either casing depending on the protocol it speaks.
            KeyCode::Char(c) if ctrl => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        Self { code, ctrl, alt }
    }
}

/// Where a key was pressed. A command lists the contexts it answers in;
/// `Global` commands answer everywhere a text field is not capturing input.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Context {
    Global,
    /// The project table has focus.
    Projects,
    /// The detail pane has focus.
    Detail,
    /// The template strip has focus.
    Templates,
    /// The selected project's action menu is open.
    Actions,
    /// The template studio: the list of templates with the selected one's
    /// details beside it.
    Studio,
    /// The template builder, on its section list or on the variables or
    /// files list — never while a form or a text area has the keys.
    Builder,
    /// The settings screen, on its list — not while a value is being edited.
    Settings,
    /// The search bar is being edited.
    SearchEdit,
    /// The command palette is open.
    Palette,
    /// Any other dialog: a confirmation, a picker, help, a message.
    Modal,
}

impl Context {
    pub fn label(self) -> &'static str {
        match self {
            Context::Global => "everywhere",
            Context::Projects => "project list",
            Context::Detail => "detail pane",
            Context::Templates => "template strip",
            Context::Actions => "project actions",
            Context::Studio => "template studio",
            Context::Builder => "template builder",
            Context::Settings => "settings",
            Context::SearchEdit => "search bar",
            Context::Palette => "command palette",
            Context::Modal => "dialogs",
        }
    }

    /// Every context, for the invariants and the help.
    pub const ALL: [Context; 11] = [
        Context::Global,
        Context::Projects,
        Context::Detail,
        Context::Templates,
        Context::Actions,
        Context::Studio,
        Context::Builder,
        Context::Settings,
        Context::SearchEdit,
        Context::Palette,
        Context::Modal,
    ];
}

/// How the help overlay groups commands.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Category {
    Navigate,
    Search,
    Project,
    Library,
    Templates,
    Settings,
    Help,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Navigate => "Navigate",
            Category::Search => "Search and filter",
            Category::Project => "Project",
            Category::Library => "Library",
            Category::Templates => "Templates",
            Category::Settings => "Settings",
            Category::Help => "Help",
        }
    }

    pub const ALL: [Category; 7] = [
        Category::Navigate,
        Category::Search,
        Category::Project,
        Category::Library,
        Category::Templates,
        Category::Settings,
        Category::Help,
    ];
}

/// Whether a command can run right now, and if not, why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Availability {
    Enabled,
    /// Listed, dimmed, with the reason. Pressing its key shows the reason.
    Disabled(&'static str),
    /// Not listed and not bound: the command makes no sense in this state.
    Hidden,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CommandId {
    // Global
    Quit,
    Back,
    Close,
    Help,
    Palette,
    Reload,
    Reindex,
    FocusNext,
    FocusPrevious,
    // Navigation in the focused list
    Down,
    Up,
    PageDown,
    PageUp,
    First,
    Last,
    // Search and filters
    Search,
    ClearSearch,
    SortCycle,
    SortPick,
    FilterTemplate,
    FilterBase,
    ClearFilters,
    // The selected project
    Actions,
    OpenFolder,
    OpenTerminal,
    CopyPath,
    ShowPath,
    ToggleDetail,
    // Single-project actions
    AddTag,
    RemoveTags,
    ReautoTags,
    AddNote,
    NoteInline,
    Rename,
    Move,
    Unregister,
    Delete,
    ShowMetadata,
    ShowJournal,
    // Marks, batch targets
    MarkToggle,
    MarkAll,
    MarkNone,
    // Flows that open their own screen
    NewProject,
    Register,
    ApplyTemplate,
    Templates,
    Settings,
    Reconcile,
    // The template strip
    StripFilter,
    // The action menu
    ActionsRun,
    // The template studio
    StudioNew,
    StudioEdit,
    StudioFromFolder,
    StudioDelete,
    // The template builder's lists
    BuilderOpen,
    BuilderAdd,
    BuilderRemove,
    BuilderMoveUp,
    BuilderMoveDown,
    // The settings list
    SettingsChange,
    // The message log
    ShowLog,
    // Ctrl-Z
    Suspend,
}

impl CommandId {
    pub const ALL: [CommandId; 62] = [
        CommandId::Quit,
        CommandId::Back,
        CommandId::Close,
        CommandId::Help,
        CommandId::Palette,
        CommandId::Reload,
        CommandId::Reindex,
        CommandId::FocusNext,
        CommandId::FocusPrevious,
        CommandId::Down,
        CommandId::Up,
        CommandId::PageDown,
        CommandId::PageUp,
        CommandId::First,
        CommandId::Last,
        CommandId::Search,
        CommandId::ClearSearch,
        CommandId::SortCycle,
        CommandId::SortPick,
        CommandId::FilterTemplate,
        CommandId::FilterBase,
        CommandId::ClearFilters,
        CommandId::Actions,
        CommandId::OpenFolder,
        CommandId::OpenTerminal,
        CommandId::CopyPath,
        CommandId::ShowPath,
        CommandId::ToggleDetail,
        CommandId::AddTag,
        CommandId::RemoveTags,
        CommandId::ReautoTags,
        CommandId::AddNote,
        CommandId::NoteInline,
        CommandId::Rename,
        CommandId::Move,
        CommandId::Unregister,
        CommandId::Delete,
        CommandId::ShowMetadata,
        CommandId::ShowJournal,
        CommandId::MarkToggle,
        CommandId::MarkAll,
        CommandId::MarkNone,
        CommandId::NewProject,
        CommandId::Register,
        CommandId::ApplyTemplate,
        CommandId::Templates,
        CommandId::Settings,
        CommandId::Reconcile,
        CommandId::StripFilter,
        CommandId::ActionsRun,
        CommandId::StudioNew,
        CommandId::StudioEdit,
        CommandId::StudioFromFolder,
        CommandId::StudioDelete,
        CommandId::BuilderOpen,
        CommandId::BuilderAdd,
        CommandId::BuilderRemove,
        CommandId::BuilderMoveUp,
        CommandId::BuilderMoveDown,
        CommandId::SettingsChange,
        CommandId::ShowLog,
        CommandId::Suspend,
    ];
}

pub struct Command {
    pub id: CommandId,
    /// What the palette, the action menu and the help overlay call it.
    pub title: &'static str,
    /// One clause: what happens when it runs.
    pub description: &'static str,
    /// Where its keys fire. `Global` fires everywhere a text field is not
    /// capturing input.
    pub contexts: &'static [Context],
    /// Default bindings. The first is what the hint bar prints.
    pub keys: &'static [Key],
    pub category: Category,
    /// Listed in the command palette.
    pub palette: bool,
    /// Shown in the hint bar at the bottom of the screen.
    pub hint: bool,
    pub available: fn(&App) -> Availability,
}

fn always(_: &App) -> Availability {
    Availability::Enabled
}

/// Job control is a unix thing; on Windows the key is not bound at all.
fn unix_only(_: &App) -> Availability {
    if cfg!(unix) {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// A verb that starts a window — the file manager, a terminal — needs a
/// desktop to start it on. Over ssh or on a console there is none, and the
/// key says so instead of pretending.
fn needs_selection_and_display(app: &App) -> Availability {
    if !app.has_display {
        return Availability::Disabled("no display — needs a desktop session; y copies the path");
    }
    needs_selection(app)
}

fn needs_selection(app: &App) -> Availability {
    if app.library.selected().is_some() {
        Availability::Enabled
    } else if app.library.loaded {
        Availability::Disabled("no project selected")
    } else {
        Availability::Disabled("still loading the library")
    }
}

fn not_busy(app: &App) -> Availability {
    if app.busy.is_some() || app.job.is_some() {
        Availability::Disabled("working…")
    } else {
        Availability::Enabled
    }
}

fn selection_and_not_busy(app: &App) -> Availability {
    match not_busy(app) {
        Availability::Enabled => needs_selection(app),
        other => other,
    }
}

/// A verb that cannot batch: rename, where every row would need its own name.
fn single_and_not_busy(app: &App) -> Availability {
    if !app.library.marks.is_empty() {
        return Availability::Disabled("one folder at a time — clear the marks (-) to rename");
    }
    selection_and_not_busy(app)
}

fn has_search(app: &App) -> Availability {
    if app.search.input.is_empty() {
        Availability::Hidden
    } else {
        Availability::Enabled
    }
}

fn has_row_filter(app: &App) -> Availability {
    if app.library.template_filter.is_some() || app.library.base_filter.is_some() {
        Availability::Enabled
    } else {
        Availability::Hidden
    }
}

/// A base filter is worth offering only where there is more than one base to
/// choose between — with one, every row answers it already.
fn many_bases(app: &App) -> Availability {
    match app.summary.as_ref() {
        Some(summary) if summary.bases.len() > 1 => Availability::Enabled,
        Some(_) => Availability::Hidden,
        // The summary is still being read; the key is bound, and pressing it
        // before the bases are known says so rather than doing nothing.
        None => Availability::Disabled("still reading the bases"),
    }
}

fn has_strip_selection(app: &App) -> Availability {
    if app.templates.cards.is_empty() {
        Availability::Disabled("no templates")
    } else {
        Availability::Enabled
    }
}

fn has_any_rows(app: &App) -> Availability {
    if app.library.is_empty() {
        Availability::Disabled("no projects")
    } else {
        Availability::Enabled
    }
}

fn has_marks(app: &App) -> Availability {
    if app.library.marks.is_empty() {
        Availability::Hidden
    } else {
        Availability::Enabled
    }
}

/// The studio's verbs on a template need one to be selected.
fn has_studio_selection(app: &App) -> Availability {
    match app.modals.top() {
        Some(crate::tui::app::modal::Modal::Studio(studio)) if studio.selected_slug().is_some() => {
            Availability::Enabled
        }
        _ => Availability::Disabled("no templates yet — n makes one"),
    }
}

/// `a` and `d` belong to the builder's variables and files lists; `K`/`J`
/// reorder the variables only. On the section list they are not bound.
fn builder_list_open(app: &App) -> Availability {
    use crate::tui::app::studio::Open;
    match app.modals.top() {
        Some(crate::tui::app::modal::Modal::Builder(builder))
            if matches!(
                builder.open,
                Some(Open::Variables(_)) | Some(Open::Files(_))
            ) =>
        {
            Availability::Enabled
        }
        _ => Availability::Hidden,
    }
}

fn builder_variables_open(app: &App) -> Availability {
    use crate::tui::app::studio::Open;
    match app.modals.top() {
        Some(crate::tui::app::modal::Modal::Builder(builder))
            if matches!(builder.open, Some(Open::Variables(_))) =>
        {
            Availability::Enabled
        }
        _ => Availability::Hidden,
    }
}

/// Move needs a mounted base to move to that is not the one the project is
/// in. With one base it is listed dimmed with the reason rather than hidden:
/// a person with one drive should still learn that a second one is a key
/// away, and pressing `m` should say why nothing happened.
fn can_move(app: &App) -> Availability {
    let Some(project) = app.library.selected() else {
        return Availability::Hidden;
    };
    let Some(summary) = &app.summary else {
        return Availability::Disabled("still probing the bases");
    };
    if summary
        .bases
        .iter()
        .any(|base| base.probe.usable() && base.path != project.base)
    {
        not_busy(app)
    } else if summary.bases.len() > 1 {
        Availability::Disabled("no other base is mounted right now")
    } else {
        Availability::Disabled("only one base is configured — add another under Settings")
    }
}

const G: &[Context] = &[Context::Global];
const LISTS: &[Context] = &[Context::Projects, Context::Detail, Context::Templates];
const PD: &[Context] = &[Context::Projects, Context::Detail];
const ACTIONS: &[Context] = &[Context::Projects, Context::Detail, Context::Actions];
const T: &[Context] = &[Context::Templates];
/// Every list and every scrollable dialog: where the arrow keys go.
const SCROLLERS: &[Context] = &[
    Context::Projects,
    Context::Detail,
    Context::Templates,
    Context::Actions,
    Context::Studio,
    Context::Builder,
    Context::Settings,
    Context::Modal,
];
/// The pages: the project list, and the dialogs with a body to scroll.
const PAGERS: &[Context] = &[
    Context::Projects,
    Context::Detail,
    Context::Studio,
    Context::Settings,
    Context::Modal,
];
/// The jumps to the ends. Not the studio: `g` is its "from a folder" there,
/// and its list is a handful of rows.
const JUMPERS: &[Context] = &[
    Context::Projects,
    Context::Detail,
    Context::Settings,
    Context::Modal,
];
/// Every dialog that closes with Esc.
const DIALOGS: &[Context] = &[
    Context::Actions,
    Context::Studio,
    Context::Builder,
    Context::Settings,
    Context::Modal,
];
const STUDIO: &[Context] = &[Context::Studio];
const BUILDER: &[Context] = &[Context::Builder];
const SETTINGS: &[Context] = &[Context::Settings];

macro_rules! cmd {
    ($id:ident, $title:expr, $desc:expr, $ctx:expr, [$($key:expr),* $(,)?], $cat:ident, palette = $pal:expr, hint = $hint:expr, $avail:expr) => {
        Command {
            id: CommandId::$id,
            title: $title,
            description: $desc,
            contexts: $ctx,
            keys: &[$($key),*],
            category: Category::$cat,
            palette: $pal,
            hint: $hint,
            available: $avail,
        }
    };
}

/// Every command, in the order the help overlay and the palette list them.
pub static COMMANDS: &[Command] = &[
    // --- global ---------------------------------------------------------
    cmd!(
        Help,
        "Help",
        "every key for where you are, and how to reach the rest",
        G,
        [Key::ch('?'), Key::plain(KeyCode::F(1))],
        Help,
        palette = true,
        hint = true,
        always
    ),
    cmd!(
        ShowLog,
        "Show messages",
        "every status line and warning this session, newest first, with the time it arrived",
        G,
        [Key::ch('L')],
        Help,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        Suspend,
        "Suspend",
        "give the terminal back to the shell, as Ctrl-Z does in any program — `fg` brings fastf back",
        G,
        [Key::ctrl('z')],
        Navigate,
        palette = true,
        hint = false,
        unix_only
    ),
    cmd!(
        Palette,
        "Command palette",
        "type to find any command, project or template",
        G,
        [Key::ch('c'), Key::ch(':'), Key::ctrl('p')],
        Help,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        ActionsRun,
        "Run the highlighted action",
        "the verb under the cursor — or press its own key",
        &[Context::Actions],
        [Key::plain(KeyCode::Enter)],
        Navigate,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        Reload,
        "Reload the library",
        "read every base again",
        G,
        [Key::plain(KeyCode::F(5)), Key::ctrl('r')],
        Library,
        palette = true,
        hint = false,
        not_busy
    ),
    cmd!(
        Reindex,
        "Reindex",
        "rescan every base from its folders and rebuild the caches",
        G,
        [Key::ch('R')],
        Library,
        palette = true,
        hint = false,
        not_busy
    ),
    cmd!(
        FocusNext,
        "Next pane",
        "move focus: projects → detail → templates",
        G,
        [Key::plain(KeyCode::Tab)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        FocusPrevious,
        "Previous pane",
        "move focus the other way",
        G,
        [Key::plain(KeyCode::BackTab)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    // --- lists and scrollable dialogs --------------------------------------
    cmd!(
        Down,
        "Down",
        "next row, or scroll down (a list wraps at the end)",
        SCROLLERS,
        [Key::plain(KeyCode::Down), Key::ch('j')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        Up,
        "Up",
        "previous row, or scroll up (a list wraps at the top)",
        SCROLLERS,
        [Key::plain(KeyCode::Up), Key::ch('k')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        PageDown,
        "Page down",
        "a screenful down (stops at the end)",
        PAGERS,
        [Key::plain(KeyCode::PageDown)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        PageUp,
        "Page up",
        "a screenful up (stops at the top)",
        PAGERS,
        [Key::plain(KeyCode::PageUp)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        First,
        "First row",
        "jump to the top",
        JUMPERS,
        [Key::plain(KeyCode::Home), Key::ch('g')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        Last,
        "Last row",
        "jump to the bottom",
        JUMPERS,
        [Key::plain(KeyCode::End), Key::ch('G')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    // --- search and filters ----------------------------------------------
    cmd!(
        Search,
        "Search",
        "type a query: words match a name, id, template or tag; tag:x template=y created>date match exactly",
        LISTS,
        [Key::ch('/')],
        Search,
        palette = true,
        hint = true,
        always
    ),
    cmd!(
        ClearSearch,
        "Clear the search",
        "show every project again",
        LISTS,
        [Key::ctrl('u')],
        Search,
        palette = true,
        hint = false,
        has_search
    ),
    cmd!(
        SortCycle,
        "Sort: next order",
        "newest → oldest → name → id → template → base → size",
        PD,
        [Key::ch('s')],
        Search,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        SortPick,
        "Sort by…",
        "pick the order from a list",
        PD,
        [Key::ch('S')],
        Search,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        FilterTemplate,
        "Filter by this project's template",
        "show only projects made from the same template",
        PD,
        [Key::ch('f')],
        Search,
        palette = true,
        hint = false,
        needs_selection
    ),
    cmd!(
        FilterBase,
        "Filter by base",
        "show only the projects in one base",
        LISTS,
        [Key::ch('b')],
        Search,
        palette = true,
        hint = false,
        many_bases
    ),
    cmd!(
        ClearFilters,
        "Clear the filters",
        "show every template's and every base's projects again",
        LISTS,
        [Key::ch('F')],
        Search,
        palette = true,
        hint = false,
        has_row_filter
    ),
    // --- the selected project --------------------------------------------
    cmd!(
        Actions,
        "Project actions",
        "open the action menu for the selected project",
        PD,
        [Key::ch('a'), Key::plain(KeyCode::Enter)],
        Project,
        palette = true,
        hint = true,
        selection_and_not_busy
    ),
    cmd!(
        OpenFolder,
        "Open project folder",
        "reveal it in the file manager",
        ACTIONS,
        [Key::ch('o')],
        Project,
        palette = true,
        hint = true,
        needs_selection_and_display
    ),
    cmd!(
        OpenTerminal,
        "Open terminal here",
        "a new terminal window in the project folder",
        ACTIONS,
        [Key::ch('t')],
        Project,
        palette = true,
        hint = true,
        needs_selection_and_display
    ),
    cmd!(
        CopyPath,
        "Copy path",
        "put the project's folder path on the clipboard",
        ACTIONS,
        [Key::ch('y')],
        Project,
        palette = true,
        hint = true,
        needs_selection
    ),
    cmd!(
        ShowPath,
        "Show path",
        "print the full path in the status line",
        ACTIONS,
        [Key::ch('p')],
        Project,
        palette = true,
        hint = false,
        needs_selection
    ),
    cmd!(
        ToggleDetail,
        "Toggle the detail pane",
        "show or hide the pane beside the list",
        LISTS,
        [Key::ch('i')],
        Navigate,
        palette = true,
        hint = false,
        always
    ),
    // --- single-project actions -------------------------------------------
    cmd!(
        AddTag,
        "Add a tag",
        "pick one the library already uses, or type a new one — on every marked project, if any",
        ACTIONS,
        [Key::ch('A')],
        Project,
        palette = true,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        RemoveTags,
        "Remove tags",
        "tick the tags to take off this project, or off every marked one",
        ACTIONS,
        [Key::ctrl('t')],
        Project,
        palette = true,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        ReautoTags,
        "Re-derive tags",
        "recompute the template's automatic tags from the variables — for every mark, if any",
        ACTIONS,
        [],
        Project,
        palette = true,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        AddNote,
        "New note",
        "write a journal note in your editor — the same note on every marked project, if any",
        ACTIONS,
        [Key::ch('N')],
        Project,
        palette = true,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        NoteInline,
        "Quick note",
        "type a short journal note where you are (Alt-Enter for a new line) — on every mark, if any",
        ACTIONS,
        [Key::ctrl('n')],
        Project,
        palette = true,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        Rename,
        "Rename folder",
        "change the folder's name on disk",
        ACTIONS,
        [Key::ch('r')],
        Project,
        palette = true,
        hint = false,
        single_and_not_busy
    ),
    cmd!(
        Move,
        "Move to another base",
        "move this project — or every marked one — into a different mounted base",
        ACTIONS,
        [Key::ch('m')],
        Project,
        palette = true,
        hint = false,
        can_move
    ),
    cmd!(
        Unregister,
        "Unregister (keep files)",
        "remove its PROJECT_INFO.md; the files stay on disk — every marked one, if any",
        ACTIONS,
        [Key::ch('u')],
        Project,
        palette = true,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        Delete,
        "Delete folder permanently",
        "delete the project and everything inside it — every marked one, if any; it asks for the word delete",
        ACTIONS,
        [Key::ch('D')],
        Project,
        palette = true,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        ShowMetadata,
        "Show metadata",
        "the project's frontmatter and variables, read-only",
        ACTIONS,
        [Key::ch('M')],
        Project,
        palette = true,
        hint = false,
        needs_selection
    ),
    cmd!(
        ShowJournal,
        "Show journal",
        "every note ever added to this project",
        ACTIONS,
        [Key::ch('J')],
        Project,
        palette = true,
        hint = false,
        needs_selection
    ),
    // --- marks (what a batch verb will act on) ----------------------------
    // Marking is how every batch verb is aimed, so it belongs in the hint bar
    // and the palette like any other verb. It was advertised only by a
    // hand-written sentence on the status line, which is exactly the drift the
    // one registry exists to prevent.
    cmd!(
        MarkToggle,
        "Mark / unmark",
        "mark the selected project as a batch target; Space moves to the next row",
        PD,
        [Key::ch(' ')],
        Project,
        palette = true,
        hint = true,
        needs_selection
    ),
    cmd!(
        MarkAll,
        "Mark all",
        "mark every project the current view shows",
        PD,
        [Key::ch('*')],
        Project,
        palette = true,
        hint = false,
        has_any_rows
    ),
    cmd!(
        MarkNone,
        "Clear marks",
        "unmark every project",
        PD,
        [Key::ch('-')],
        Project,
        palette = true,
        hint = false,
        has_marks
    ),
    // --- flows ------------------------------------------------------------
    cmd!(
        NewProject,
        "Create new project",
        "pick a template, answer its questions, preview, create",
        LISTS,
        [Key::ch('n')],
        Library,
        palette = true,
        hint = true,
        not_busy
    ),
    cmd!(
        Register,
        "Register existing folder",
        "adopt a folder fastf did not create — one, or every unregistered folder in a base",
        LISTS,
        [Key::ch('e')],
        Library,
        palette = true,
        hint = false,
        not_busy
    ),
    cmd!(
        ApplyTemplate,
        "Apply a template to a folder",
        "fill in a folder's missing folders and files from a template — never overwrites",
        LISTS,
        [Key::ch('E')],
        Templates,
        palette = true,
        hint = false,
        not_busy
    ),
    cmd!(
        Templates,
        "Manage templates",
        "create, edit, generate from a folder, apply, show, delete",
        LISTS,
        [Key::ch('T')],
        Templates,
        palette = true,
        hint = false,
        not_busy
    ),
    cmd!(
        Settings,
        "Settings",
        "bases, workflow prompts, post-create actions, the ID counter, maintenance",
        LISTS,
        [Key::ch(',')],
        Settings,
        palette = true,
        hint = false,
        not_busy
    ),
    cmd!(
        Reconcile,
        "Check and recover",
        "finish or roll back work a crash left half-done — what ⚠ needs attention means",
        LISTS,
        [Key::ch('!')],
        Library,
        palette = true,
        hint = false,
        not_busy
    ),
    // --- the template strip -----------------------------------------------
    cmd!(
        StripFilter,
        "Filter by this template",
        "show only this template's projects (again to clear)",
        T,
        [Key::plain(KeyCode::Enter)],
        Templates,
        palette = false,
        hint = true,
        has_strip_selection
    ),
    // --- the template studio ----------------------------------------------
    cmd!(
        StudioEdit,
        "Edit this template",
        "open the selected template in the builder",
        STUDIO,
        [Key::plain(KeyCode::Enter), Key::ch('e')],
        Templates,
        palette = false,
        hint = true,
        has_studio_selection
    ),
    cmd!(
        StudioNew,
        "New template",
        "build a template from scratch: metadata, variables, folders, files",
        STUDIO,
        [Key::ch('n')],
        Templates,
        palette = true,
        hint = true,
        not_busy
    ),
    cmd!(
        StudioFromFolder,
        "Template from a folder",
        "generate a template out of a folder that already has the shape you want",
        STUDIO,
        [Key::ch('g')],
        Templates,
        palette = true,
        hint = true,
        not_busy
    ),
    cmd!(
        StudioDelete,
        "Delete this template",
        "delete the selected template and its bundled files — it asks first",
        STUDIO,
        [Key::ch('D')],
        Templates,
        palette = false,
        hint = true,
        has_studio_selection
    ),
    // --- the template builder ---------------------------------------------
    cmd!(
        BuilderOpen,
        "Open",
        "open the highlighted section, or save or discard from the section list; edit the highlighted variable or file",
        BUILDER,
        [Key::plain(KeyCode::Enter)],
        Templates,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        BuilderAdd,
        "Add",
        "a new variable or file at the end of the list",
        BUILDER,
        [Key::ch('a')],
        Templates,
        palette = false,
        hint = true,
        builder_list_open
    ),
    cmd!(
        BuilderRemove,
        "Remove",
        "take the highlighted variable or file out of the template",
        BUILDER,
        [Key::ch('d')],
        Templates,
        palette = false,
        hint = true,
        builder_list_open
    ),
    cmd!(
        BuilderMoveUp,
        "Move up",
        "ask for this variable earlier",
        BUILDER,
        [Key::ch('K')],
        Templates,
        palette = false,
        hint = true,
        builder_variables_open
    ),
    cmd!(
        BuilderMoveDown,
        "Move down",
        "ask for this variable later",
        BUILDER,
        [Key::ch('J')],
        Templates,
        palette = false,
        hint = true,
        builder_variables_open
    ),
    // --- the settings list -------------------------------------------------
    cmd!(
        SettingsChange,
        "Change / run",
        "flip a yes/no or cycle a choice where it stands, open a value on its line, or run the maintenance verb",
        SETTINGS,
        [Key::plain(KeyCode::Enter)],
        Settings,
        palette = false,
        hint = true,
        always
    ),
    // --- leaving: declared last so their hints come last --------------------
    cmd!(
        Quit,
        "Quit",
        "leave fastf",
        LISTS,
        [Key::ch('q')],
        Navigate,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        Back,
        "Back",
        "one step back: cancel a running job, clear the search, the filter, the marks — then quit",
        LISTS,
        [Key::plain(KeyCode::Esc)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    // --- closing a dialog ---------------------------------------------------
    cmd!(
        Close,
        "Close",
        "close this dialog — one level at a time, nothing already answered is lost",
        DIALOGS,
        [Key::plain(KeyCode::Esc), Key::ch('q')],
        Navigate,
        palette = false,
        hint = true,
        always
    ),
];

/// The command declared for `id`.
pub fn find(id: CommandId) -> &'static Command {
    COMMANDS
        .iter()
        .find(|c| c.id == id)
        .expect("every CommandId is declared in COMMANDS")
}

/// The command `key` runs in `ctx`, if any. The context's own bindings win
/// over the global ones, so a modal can take `q` for itself.
///
/// A `Disabled` command is still returned — the caller shows the reason — but
/// a `Hidden` one is not bound at all.
pub fn lookup(ctx: Context, key: Key, app: &App) -> Option<CommandId> {
    let in_ctx = |c: &&Command, wanted: Context| {
        c.contexts.contains(&wanted)
            && c.keys.contains(&key)
            && (c.available)(app) != Availability::Hidden
    };
    COMMANDS
        .iter()
        .find(|c| in_ctx(c, ctx))
        .or_else(|| COMMANDS.iter().find(|c| in_ctx(c, Context::Global)))
        .map(|c| c.id)
}

/// The hint bar: `(key label, title)` pairs for the commands that fire in
/// `ctx`, in declaration order, as many as fit in `width` columns.
pub fn hints(ctx: Context, app: &App, width: usize) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let mut used = 0usize;
    // The context's own commands first — they are what the bar is for — and
    // the global ones (help, the palette, quit) after them.
    let own = COMMANDS
        .iter()
        .filter(|c| c.hint && c.contexts.contains(&ctx));
    let global = COMMANDS
        .iter()
        .filter(|c| c.hint && !c.contexts.contains(&ctx) && c.contexts.contains(&Context::Global));
    for c in own
        .chain(global)
        .filter(|c| (c.available)(app) != Availability::Hidden)
    {
        let Some(key) = c.keys.first() else {
            continue;
        };
        let label = key.label();
        let title = hint_title(c.id, c.title);
        let cost = label.chars().count() + 1 + title.chars().count() + 2;
        if used + cost > width && !out.is_empty() {
            break;
        }
        used += cost;
        out.push((label, title));
    }
    out
}

/// The hint bar has one line, so a few titles get a shorter form there.
pub fn hint_title(id: CommandId, title: &'static str) -> &'static str {
    match id {
        CommandId::Palette => "commands",
        CommandId::Actions => "actions",
        CommandId::OpenFolder => "open",
        CommandId::OpenTerminal => "terminal",
        CommandId::CopyPath => "copy path",
        CommandId::NewProject => "new",
        CommandId::Search => "search",
        CommandId::Help => "help",
        CommandId::MarkToggle => "mark",
        CommandId::ShowLog => "messages",
        CommandId::Quit => "quit",
        CommandId::Close => "close",
        CommandId::StripFilter => "filter",
        CommandId::ActionsRun => "run",
        CommandId::StudioEdit => "edit",
        CommandId::StudioNew => "new",
        CommandId::StudioFromFolder => "from a folder",
        CommandId::StudioDelete => "delete",
        CommandId::BuilderOpen => "open",
        CommandId::BuilderAdd => "add",
        CommandId::BuilderRemove => "remove",
        CommandId::BuilderMoveUp => "up",
        CommandId::BuilderMoveDown => "down",
        CommandId::SettingsChange => "change / run",
        _ => title,
    }
}

/// The help overlay: every command that fires in `ctx` (plus the global ones),
/// grouped by category in `Category::ALL` order.
pub fn help_sections(ctx: Context) -> Vec<(Category, Vec<&'static Command>)> {
    Category::ALL
        .iter()
        .map(|category| {
            let commands: Vec<&'static Command> = COMMANDS
                .iter()
                .filter(|c| c.category == *category)
                .filter(|c| c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global))
                .collect();
            (*category, commands)
        })
        .filter(|(_, commands)| !commands.is_empty())
        .collect()
}

/// One line of the help overlay's body, as text; the view styles it. Built
/// here so `update` can count the lines with the same arithmetic the view
/// draws them by, and clamp the scroll to what there is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelpLine {
    Heading(&'static str),
    /// A command: its keys, its title, and the first line of its description.
    Command {
        keys: String,
        title: &'static str,
        description: String,
    },
    /// The rest of a description that did not fit its line, drawn `indent`
    /// columns in.
    Continuation {
        indent: usize,
        text: String,
    },
    Blank,
}

/// Below this many columns for a description, the help puts descriptions on
/// their own line under the keys and the title rather than beside them.
const NARROW_DESCRIPTION: usize = 28;

/// The three column widths the help overlay lays out in: keys, title, and
/// what is left for the description — measured from the commands themselves,
/// so a long title can never run into its description.
pub fn help_columns(ctx: Context, inner_width: usize) -> (usize, usize, usize) {
    let commands: Vec<&Command> = help_sections(ctx)
        .into_iter()
        .flat_map(|(_, commands)| commands)
        .collect();
    let keys_width = commands
        .iter()
        .map(|c| key_labels(c).chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 18);
    let title_width = commands
        .iter()
        .map(|c| c.title.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 36);
    // Three columns of indent, a space after the keys, a space after the title.
    let description_width = inner_width.saturating_sub(3 + keys_width + 1 + title_width + 1);
    (keys_width, title_width, description_width)
}

/// `? / F1`, `c / : / Ctrl-p`: a command's keys as the help prints them.
pub fn key_labels(command: &Command) -> String {
    command
        .keys
        .iter()
        .map(|k| k.label())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The help overlay's body for `ctx`, laid out for `inner_width` columns: a
/// description that does not fit its line continues under itself.
pub fn help_lines(ctx: Context, inner_width: usize) -> Vec<HelpLine> {
    let (keys_width, title_width, description_width) = help_columns(ctx, inner_width);
    // Wide enough: three columns. Narrow: the description on its own line
    // under the title, indented past the keys, so it reads as prose rather
    // than a ladder of three-word lines.
    let beside = description_width >= NARROW_DESCRIPTION;
    let (indent, width) = if beside {
        (3 + keys_width + 1 + title_width + 1, description_width)
    } else {
        let indent = 3 + keys_width + 1;
        (indent, inner_width.saturating_sub(indent + 1).max(12))
    };
    let mut lines = Vec::new();
    for (category, commands) in help_sections(ctx) {
        lines.push(HelpLine::Heading(category.label()));
        for c in commands {
            let mut parts = wrap_words(c.description, width).into_iter();
            lines.push(HelpLine::Command {
                keys: key_labels(c),
                title: c.title,
                description: if beside {
                    parts.next().unwrap_or_default()
                } else {
                    String::new()
                },
            });
            lines.extend(parts.map(|text| HelpLine::Continuation { indent, text }));
        }
        lines.push(HelpLine::Blank);
    }
    lines
}

/// Greedy word wrap into lines of at most `width` characters; a word longer
/// than a line is broken where it must be.
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let head: String = word.chars().take(width).collect();
            word = word.chars().skip(width).collect();
            lines.push(head);
        }
        let needed = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if needed > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(&word);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// How many lines the help overlay draws for `ctx` at `inner_width`: the
/// body, and the five lines of footer under it. The view draws exactly
/// this, and `update` clamps the scroll with it.
pub fn help_line_count(ctx: Context, inner_width: usize) -> usize {
    help_lines(ctx, inner_width).len() + 5
}

/// The palette's command entries: everything listed and not hidden, the
/// current context's commands first, then the global ones, then the rest.
pub fn palette_entries(ctx: Context, app: &App) -> Vec<(&'static Command, Availability)> {
    let rank = |c: &Command| {
        if c.contexts.contains(&ctx) {
            0
        } else if c.contexts.contains(&Context::Global) {
            1
        } else {
            2
        }
    };
    let mut entries: Vec<(&'static Command, Availability)> = COMMANDS
        .iter()
        .filter(|c| c.palette)
        .map(|c| (c, (c.available)(app)))
        .filter(|(_, availability)| *availability != Availability::Hidden)
        .collect();
    entries.sort_by_key(|(c, _)| rank(c));
    entries
}
