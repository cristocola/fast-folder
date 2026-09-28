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

use crate::tui::app::{App, Focus};

mod available;
mod context;
mod help;
mod key;
mod read;
mod table;

pub use available::*;
pub use context::*;
pub use help::*;
pub use key::*;
pub use read::*;
pub use table::*;

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
    PaletteRun,
    PaletteNext,
    PalettePrevious,
    PaletteClose,
    PromptConfirm,
    PromptNewline,
    PromptCancel,
    PickChoose,
    PickToggle,
    PickNext,
    PickPrevious,
    PickCancel,
    SearchAccept,
    SearchCancel,
    Reload,
    Reindex,
    FocusNext,
    FocusPrevious,
    /// Ctrl-C, declared so the key that cancels a running job is in the help.
    Interrupt,
    // Navigation in the focused list
    Down,
    Up,
    PageDown,
    PageUp,
    HalfDown,
    HalfUp,
    First,
    Last,
    /// The horizontal axis, leftwards: from the pane back to the list.
    FocusList,
    /// The horizontal axis, rightwards: from the list into the pane.
    FocusDetail,
    /// From the pane, the project above the one it shows — staying in the
    /// pane, in the same section.
    PanePreviousProject,
    /// From the pane, the project below the one it shows.
    PaneNextProject,
    /// F2 in the pane: the text of the row under the cursor, opened in
    /// place — a todo reworded, a tag, a variable, a note, the name.
    PaneEditText,
    /// `+` in the pane: one more of what the cursor is among.
    PaneAdd,
    /// F2 on the list: the folder's name — rename, as everywhere F2 edits.
    ListRename,
    /// `+` on the list: a todo for the project under the cursor.
    ListAddTodo,
    /// F2 in the builder: the highlighted part, variable or file, opened.
    BuilderEditText,
    /// F2 in the settings: a value, opened on its line.
    SettingsEditText,
    /// Back to the library from the templates tab — palette only; `T` and
    /// Esc are the keys.
    BackToLibrary,
    // Search and filters
    Search,
    ClearSearch,
    SortCycle,
    SortPick,
    FilterTemplate,
    FilterBase,
    FilterTag,
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
    /// One task onto the list — what the pane's add row does, findable.
    AddTodo,
    /// A new `###` phase on the list, written with its first todo.
    AddPhase,
    Rename,
    /// Enter on the project list: the action menu, as `a` opens it. Its own
    /// id because the pane's Enter means something else.
    ActionsEnter,
    /// Enter on a row of the detail pane: edit what is under the cursor.
    PaneEdit,
    /// The pane's line editor: keep what was typed.
    PaneEditConfirm,
    /// The pane's editor: leave the row as it was.
    PaneEditCancel,
    /// The pane's notes editor: save the text.
    PaneEditSave,
    Move,
    CopyTo,
    Unregister,
    Delete,
    ShowMetadata,
    ShowJournal,
    // Marks, batch targets
    MarkToggle,
    MarkToHere,
    MarkAll,
    MarkNone,
    // Flows that open their own screen
    NewProject,
    Register,
    ApplyTemplate,
    Templates,
    Settings,
    Reconcile,
    Attention,
    // The action menu
    ActionsRun,
    // The templates tab
    StripFilter,
    StudioNew,
    StudioEdit,
    StudioFromFolder,
    StudioDelete,
    /// The template guide — the one surface that teaches rather than does.
    Guide,
    GuideNext,
    GuidePrevious,
    // The template builder's lists
    BuilderOpen,
    BuilderAdd,
    BuilderRemove,
    BuilderMoveUp,
    BuilderMoveDown,
    BuilderSave,
    BuilderExplain,
    // The settings list
    SettingsChange,
    SettingsFilter,
    // The message log
    ShowLog,
    // Ctrl-Z
    Suspend,
}

impl CommandId {
    pub const ALL: [CommandId; 107] = [
        CommandId::Quit,
        CommandId::Back,
        CommandId::Close,
        CommandId::Help,
        CommandId::Palette,
        CommandId::PaletteRun,
        CommandId::PaletteNext,
        CommandId::PalettePrevious,
        CommandId::PaletteClose,
        CommandId::PromptConfirm,
        CommandId::PromptNewline,
        CommandId::PromptCancel,
        CommandId::PickChoose,
        CommandId::PickToggle,
        CommandId::PickNext,
        CommandId::PickPrevious,
        CommandId::PickCancel,
        CommandId::SearchAccept,
        CommandId::SearchCancel,
        CommandId::Reload,
        CommandId::Reindex,
        CommandId::FocusNext,
        CommandId::FocusPrevious,
        CommandId::Interrupt,
        CommandId::Down,
        CommandId::Up,
        CommandId::PageDown,
        CommandId::PageUp,
        CommandId::HalfDown,
        CommandId::HalfUp,
        CommandId::First,
        CommandId::Last,
        CommandId::FocusList,
        CommandId::FocusDetail,
        CommandId::PanePreviousProject,
        CommandId::PaneNextProject,
        CommandId::PaneEditText,
        CommandId::PaneAdd,
        CommandId::ListRename,
        CommandId::ListAddTodo,
        CommandId::BuilderEditText,
        CommandId::SettingsEditText,
        CommandId::BackToLibrary,
        CommandId::Search,
        CommandId::ClearSearch,
        CommandId::SortCycle,
        CommandId::SortPick,
        CommandId::FilterTemplate,
        CommandId::FilterBase,
        CommandId::FilterTag,
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
        CommandId::AddTodo,
        CommandId::AddPhase,
        CommandId::Rename,
        CommandId::ActionsEnter,
        CommandId::PaneEdit,
        CommandId::PaneEditConfirm,
        CommandId::PaneEditCancel,
        CommandId::PaneEditSave,
        CommandId::Move,
        CommandId::CopyTo,
        CommandId::Unregister,
        CommandId::Delete,
        CommandId::ShowMetadata,
        CommandId::ShowJournal,
        CommandId::MarkToggle,
        CommandId::MarkToHere,
        CommandId::MarkAll,
        CommandId::MarkNone,
        CommandId::NewProject,
        CommandId::Register,
        CommandId::ApplyTemplate,
        CommandId::Templates,
        CommandId::Settings,
        CommandId::Reconcile,
        CommandId::Attention,
        CommandId::StripFilter,
        CommandId::ActionsRun,
        CommandId::StudioNew,
        CommandId::StudioEdit,
        CommandId::StudioFromFolder,
        CommandId::StudioDelete,
        CommandId::Guide,
        CommandId::GuideNext,
        CommandId::GuidePrevious,
        CommandId::BuilderOpen,
        CommandId::BuilderAdd,
        CommandId::BuilderRemove,
        CommandId::BuilderMoveUp,
        CommandId::BuilderMoveDown,
        CommandId::BuilderSave,
        CommandId::BuilderExplain,
        CommandId::SettingsChange,
        CommandId::SettingsFilter,
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
