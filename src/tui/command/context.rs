//! Where keys fire: the contexts a command is declared over.

/// Where a key was pressed. A command lists the contexts it answers in;
/// `Global` commands answer everywhere a text field is not capturing input.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Context {
    Global,
    /// The project table has focus.
    Projects,
    /// The detail pane has focus.
    Detail,
    /// The templates tab: every template, with the selected one's details
    /// beside it and the verbs on it.
    Templates,
    /// The selected project's action menu is open.
    Actions,
    /// The bases panel: which bases the list shows.
    Bases,
    /// One base's menu, its verbs listed with their keys.
    BaseMenu,
    /// The template builder, on its section list or on the variables or
    /// files list — never while a form or a text area has the keys.
    Builder,
    /// The settings screen, on its list — not while a value is being edited.
    Settings,
    /// The search bar is being edited.
    SearchEdit,
    /// The command palette is open.
    Palette,
    /// The template guide: a reader of seven pages, which owns its own
    /// left and right the way a text field owns its caret.
    Guide,
    /// A one-line prompt, a quick note, the first-run question: a field with
    /// Enter under it. Everything printable is the text.
    Prompt,
    /// A picker — one choice or several. `Pick` carries a query, so it is a
    /// text-entry context too; `MultiPick` does not, and Space ticks a row.
    Pick,
    /// A row of the detail pane is being edited in place — a line for a
    /// variable or a tag, a text area for the notes. The field has the keys.
    PaneEdit,
    /// Any other dialog: a confirmation, a picker, help, a message.
    Modal,
}

impl Context {
    pub fn label(self) -> &'static str {
        match self {
            Context::Global => "everywhere",
            Context::Projects => "project list",
            Context::Detail => "detail pane",
            Context::Templates => "templates tab",
            Context::Actions => "project actions",
            Context::Bases => "bases",
            Context::BaseMenu => "a base's actions",
            Context::Builder => "template builder",
            Context::Settings => "settings",
            Context::SearchEdit => "search bar",
            Context::Palette => "command palette",
            Context::Guide => "template guide",
            Context::Prompt => "a prompt",
            Context::Pick => "a picker",
            Context::PaneEdit => "editing in the detail pane",
            Context::Modal => "dialogs",
        }
    }

    /// A context where a field has the keys: everything printable is the
    /// text, and the caret's own chords are the field's.
    pub fn is_text_entry(self) -> bool {
        matches!(
            self,
            Context::SearchEdit
                | Context::Palette
                | Context::Prompt
                | Context::Pick
                | Context::PaneEdit
        )
    }

    /// Whether the hint bar should say how to move here.
    ///
    /// Not on the dashboard: a table with a highlighted row and a scrollbar
    /// beside it already says which way the arrows go, and the bar's width is
    /// better spent on the verbs. In a dialog that has just opened over it,
    /// and in the search bar — where the arrows move the list *underneath*
    /// what is being typed, which nothing on screen says — they are worth the
    /// eight columns.
    pub fn hints_movement(self) -> bool {
        matches!(
            self,
            Context::SearchEdit
                | Context::Actions
                | Context::Bases
                | Context::BaseMenu
                | Context::Builder
                | Context::Settings
                | Context::Palette
                | Context::Guide
                | Context::Prompt
                | Context::Pick
                | Context::Modal
        )
    }

    /// Every context, for the invariants and the help.
    pub const ALL: [Context; 16] = [
        Context::Global,
        Context::Projects,
        Context::Detail,
        Context::Templates,
        Context::Actions,
        Context::Bases,
        Context::BaseMenu,
        Context::Builder,
        Context::Settings,
        Context::SearchEdit,
        Context::Palette,
        Context::Guide,
        Context::Prompt,
        Context::Pick,
        Context::PaneEdit,
        Context::Modal,
    ];
}
