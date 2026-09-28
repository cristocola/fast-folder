//! The registry itself: every command, declared once, in the order the help
//! and the palette list them.

use super::*;

const G: &[Context] = &[Context::Global];
/// The library's own screen: the table and its pane. The templates
/// tab is **not** in it, so `n` means one thing per hint bar: "new project"
/// here, "new template" on the tab.
const LISTS: &[Context] = &[Context::Projects, Context::Detail];
const PD: &[Context] = &[Context::Projects, Context::Detail];
const ACTIONS: &[Context] = &[Context::Projects, Context::Detail, Context::Actions];
const TEMPLATES: &[Context] = &[Context::Templates];
/// Both tabs: the switch itself, the search bar, and the app-wide verbs that
/// mean the same thing wherever you are.
const TABS: &[Context] = &[Context::Projects, Context::Detail, Context::Templates];
const BACKSTEP: &[Context] = TABS;
/// Every list and every scrollable dialog: where the arrow keys go — and,
/// since one grammar is the whole point, where the page keys and the jumps to
/// the ends go too. Declared over anything narrower, they skip some list, and
/// a list that cannot be searched — the action menu, the builder — is then
/// walked a row at a time.
const SCROLLERS: &[Context] = &[
    Context::Projects,
    Context::Detail,
    Context::Templates,
    Context::Actions,
    Context::Builder,
    Context::Settings,
    Context::Guide,
    // The search bar is on this list because the arrows there move the
    // library under the query, which is the same command they are everywhere
    // else. `keys_in` takes `j`, `g`, `Ctrl-u` and the rest back out of what
    // that context advertises, because the field claims them first.
    Context::SearchEdit,
    Context::Modal,
];
/// Every dialog that closes with Esc — the guide included.
const DIALOGS: &[Context] = &[
    Context::Actions,
    Context::Builder,
    Context::Settings,
    Context::Guide,
    Context::Modal,
];
/// Where the horizontal axis moves focus: both tabs, each a list with a pane
/// of its own. Nowhere else — a dialog has no second pane, and a text field
/// owns its own arrows.
const PANED: &[Context] = &[Context::Projects, Context::Detail, Context::Templates];
const STUDIO: &[Context] = &[Context::Templates];
/// The guide overlay itself — the reader, not the key that opens it.
const READER: &[Context] = &[Context::Guide];
/// The palette itself. Everything printable there is the query, so these are
/// the only keys it can declare.
const IN_PALETTE: &[Context] = &[Context::Palette];
const IN_PROMPT: &[Context] = &[Context::Prompt];
const IN_PANE_EDIT: &[Context] = &[Context::PaneEdit];
const IN_PICK: &[Context] = &[Context::Pick];
/// Everywhere the palette can be *opened* from — which is everywhere except
/// the palette, so `Ctrl-p` inside it is free to mean the previous entry.
const OPENS_PALETTE: &[Context] = &[
    Context::Projects,
    Context::Detail,
    Context::Templates,
    Context::Actions,
    Context::Builder,
    Context::Settings,
    Context::SearchEdit,
    Context::Guide,
    Context::Prompt,
    Context::Pick,
    Context::Modal,
];

const BUILDER: &[Context] = &[Context::Builder];
/// The key that opens the guide answers wherever templates are the subject:
/// the tab and the editor. Deliberately not `Global` — the guide is about one
/// thing, and a key that opens it from the project list would say otherwise.
const GUIDE: &[Context] = &[Context::Templates, Context::Builder];
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
        "Messages and log",
        "what fastf said and what it did, from every session, newest first — the log has every step of every move",
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
        OPENS_PALETTE,
        [Key::ch('c'), Key::ch(':'), Key::ctrl('p')],
        Help,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        SearchAccept,
        "Keep the query",
        "leave the search bar with what is typed still filtering the list",
        &[Context::SearchEdit],
        [Key::plain(KeyCode::Enter)],
        Search,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        PaletteRun,
        "Run",
        "the command, project or template under the cursor",
        IN_PALETTE,
        [Key::plain(KeyCode::Enter)],
        Navigate,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        PaletteNext,
        "Next entry",
        "down the palette's list",
        IN_PALETTE,
        [Key::plain(KeyCode::Down), Key::ctrl('n')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        PalettePrevious,
        "Previous entry",
        "up the palette's list",
        IN_PALETTE,
        [Key::plain(KeyCode::Up), Key::ctrl('p')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        PaletteClose,
        "Close the palette",
        "leave it, with the list exactly as it was",
        IN_PALETTE,
        [Key::plain(KeyCode::Esc)],
        Navigate,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        SearchCancel,
        "Clear, then leave",
        "the first Esc clears the query, the second leaves the bar",
        &[Context::SearchEdit],
        [Key::plain(KeyCode::Esc)],
        Search,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        PromptConfirm,
        "Confirm",
        "take what is typed and act on it",
        IN_PROMPT,
        [Key::plain(KeyCode::Enter)],
        Navigate,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        PromptNewline,
        "New line",
        "break the line without saving the note",
        IN_PROMPT,
        [Key {
            code: KeyCode::Enter,
            ctrl: false,
            alt: true,
        }],
        Navigate,
        palette = false,
        hint = true,
        in_a_note
    ),
    cmd!(
        PromptCancel,
        "Cancel",
        "leave it, with nothing changed",
        IN_PROMPT,
        [Key::plain(KeyCode::Esc)],
        Navigate,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        PickChoose,
        "Choose",
        "take the row under the cursor — or every row ticked",
        IN_PICK,
        [Key::plain(KeyCode::Enter)],
        Navigate,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        PickToggle,
        "Tick",
        "put the row under the cursor in the set, or take it out",
        IN_PICK,
        [Key::ch(' ')],
        Navigate,
        palette = false,
        hint = true,
        in_a_multi_pick
    ),
    cmd!(
        PickNext,
        "Next row",
        "down the picker's list",
        IN_PICK,
        [Key::plain(KeyCode::Down)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        PickPrevious,
        "Previous row",
        "up the picker's list",
        IN_PICK,
        [Key::plain(KeyCode::Up)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        PickCancel,
        "Cancel",
        "leave the picker, with nothing chosen",
        IN_PICK,
        [Key::plain(KeyCode::Esc)],
        Navigate,
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
        "move focus between the list and its pane — on the messages and log screen, turn the page",
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
        "next row, or scroll down (stops at the end)",
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
        "previous row, or scroll up (stops at the top)",
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
        SCROLLERS,
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
        SCROLLERS,
        [Key::plain(KeyCode::PageUp)],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        HalfDown,
        "Half a page down",
        "half a screenful down (stops at the end)",
        SCROLLERS,
        [Key::ctrl('d')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        HalfUp,
        "Half a page up",
        "half a screenful up (stops at the top)",
        SCROLLERS,
        [Key::ctrl('u')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        First,
        "First row",
        "jump to the top",
        SCROLLERS,
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
        SCROLLERS,
        [Key::plain(KeyCode::End), Key::ch('G')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    // --- search and filters ----------------------------------------------
    cmd!(
        BackToLibrary,
        "Back to the library",
        "leave the templates tab for the projects you came from",
        TEMPLATES,
        [],
        Navigate,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        Search,
        "Search",
        "type a query: words match a name, id, template or tag; tag:x template=y created>date match exactly",
        TABS,
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
        TABS,
        [],
        Search,
        palette = true,
        hint = false,
        has_search
    ),
    cmd!(
        SortCycle,
        "Sort: next order",
        "in turn: newest, oldest, name, id, template, base, size",
        PD,
        [Key::ch('s')],
        Search,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        SortPick,
        "Sort: pick an order",
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
        FilterTag,
        "Filter by tag",
        "show only the projects carrying one tag — the search bar's `tag:` in a list",
        PD,
        [],
        Search,
        palette = true,
        hint = false,
        has_any_tags
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
        [Key::ch('a')],
        Project,
        palette = true,
        hint = true,
        selection_and_not_busy
    ),
    // Enter is `a` on the list and `edit` in the pane. One id cannot carry
    // two keys in two contexts, so the list's Enter is its own id with the
    // same handler, hidden from the bar and the palette — `a actions` is
    // the pair that names the verb.
    cmd!(
        ActionsEnter,
        "Project actions",
        "the action menu — what Enter does on the list",
        &[Context::Projects],
        [Key::plain(KeyCode::Enter)],
        Project,
        palette = false,
        hint = false,
        selection_and_not_busy
    ),
    cmd!(
        PaneEdit,
        "Edit",
        "act on what is under the cursor: edit the name, a tag, a variable or a note, toggle a todo, or add one",
        &[Context::Detail],
        [Key::plain(KeyCode::Enter)],
        Project,
        palette = false,
        hint = true,
        pane_row_and_not_busy
    ),
    // **Enter acts, F2 edits, `+` adds** — the same three wherever there is a
    // row to act on, text to edit or a list to add to. Enter on a todo ticks
    // it, so rewording one needs a key of its own, and F2 is the edit key a
    // file manager has always had.
    cmd!(
        PaneEditText,
        "Edit the text",
        "open the row under the cursor in place: reword a todo (Enter ticks it), a tag, a variable, a note, the name; emptied, a todo, a tag or a note is removed",
        &[Context::Detail],
        [Key::plain(KeyCode::F(2))],
        Project,
        palette = false,
        hint = true,
        pane_text_row
    ),
    cmd!(
        PaneAdd,
        "Add here",
        "one more of what the cursor is among: a todo, typed where it will land (in the cursor's phase) with the next line opening under it; a tag; a note",
        &[Context::Detail],
        [Key::ch('+')],
        Project,
        palette = false,
        hint = true,
        selection_and_not_busy
    ),
    cmd!(
        ListRename,
        "Rename folder",
        "F2 edits wherever it is pressed: on the list, the folder's name",
        &[Context::Projects],
        [Key::plain(KeyCode::F(2))],
        Project,
        palette = false,
        hint = false,
        single_and_not_busy
    ),
    cmd!(
        ListAddTodo,
        "Add a todo",
        "a todo for the project under the cursor, typed into its list in the pane",
        &[Context::Projects],
        [Key::ch('+')],
        Project,
        palette = false,
        hint = false,
        one_project
    ),
    cmd!(
        PaneEditConfirm,
        "Keep",
        "keep what was typed and write it to the project",
        IN_PANE_EDIT,
        [Key::plain(KeyCode::Enter)],
        Navigate,
        palette = false,
        hint = true,
        pane_line_editing
    ),
    cmd!(
        PaneEditSave,
        "Save the note",
        "write the note to the project (Enter is a new line here); emptied, the note is removed",
        IN_PANE_EDIT,
        [Key::ctrl('s')],
        Navigate,
        palette = false,
        hint = true,
        pane_note_editing
    ),
    cmd!(
        PaneEditCancel,
        "Cancel",
        "leave the row as it was",
        IN_PANE_EDIT,
        [Key::plain(KeyCode::Esc)],
        Navigate,
        palette = false,
        hint = true,
        always
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
        "show or hide the detail pane",
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
        batch_target
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
        batch_target
    ),
    cmd!(
        ReautoTags,
        "Re-derive tags",
        "recompute the template's automatic tags from the variables — for every mark, if any",
        ACTIONS,
        // Deliberately keyless: every mnemonic near it is taken (`R` is
        // Reindex, `r` is rename), and inventing a chord for a verb this rare
        // costs more than it is worth. The action menu shows `Enter` for a row
        // with no key of its own rather than an empty column — see
        // `view::modals::render_actions`.
        [],
        Project,
        palette = true,
        hint = false,
        batch_target
    ),
    cmd!(
        AddNote,
        "New note",
        "write a note in your editor — the same note on every marked project, if any",
        ACTIONS,
        [Key::ch('N')],
        Project,
        palette = true,
        hint = false,
        batch_target
    ),
    cmd!(
        NoteInline,
        "Quick note",
        "type a note where you are (Alt-Enter for a new line) — on every mark, if any",
        ACTIONS,
        [Key::ctrl('n')],
        Project,
        palette = true,
        hint = false,
        batch_target
    ),
    cmd!(
        AddTodo,
        "Add a todo",
        "type one task onto this project's list, as the pane's add row does",
        ACTIONS,
        [],
        Project,
        palette = true,
        hint = false,
        one_project
    ),
    cmd!(
        AddPhase,
        "Add a phase",
        "name a new phase for this project's todos and type them under it; the phase is written with its first todo",
        ACTIONS,
        [Key::ch('P')],
        Project,
        palette = true,
        hint = false,
        one_project
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
        CopyTo,
        "Copy to a folder",
        "copy this project — or every marked one — to a folder outside your bases, keeping its ID",
        ACTIONS,
        [Key::ch('C')],
        Project,
        palette = true,
        hint = false,
        batch_target
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
        batch_target
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
        batch_target
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
        "Show notes",
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
    // and the palette like any other verb, never in a hand-written sentence on
    // the status line.
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
        MarkToHere,
        "Mark to here",
        "mark every row between the last one you marked and the cursor",
        PD,
        [Key::ch('v')],
        Library,
        palette = true,
        hint = false,
        has_anchor
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
    // The tab switch, from either tab: `T` goes to the templates and `T`
    // comes back. One key for one place, in both directions.
    cmd!(
        Templates,
        "Templates",
        "the templates tab: every template, what it makes, and its verbs",
        TABS,
        [Key::ch('T')],
        Templates,
        palette = true,
        hint = true,
        not_busy
    ),
    // --- the horizontal axis: focus ------------------------------------------
    // Declared after the tab switch so the bar reads verbs first, then the
    // ways to look around, then the ways to ask: `→ pane` ahead of the verbs
    // pushes `? help` off an 80-column bar.
    cmd!(
        FocusList,
        "Back to the list",
        "put the cursor back on the list",
        PANED,
        [Key::plain(KeyCode::Left), Key::ch('h')],
        Navigate,
        palette = false,
        hint = true,
        pane_has_focus
    ),
    cmd!(
        FocusDetail,
        "Into the pane",
        "put the cursor in the pane — the project's detail, or the template's",
        PANED,
        [Key::plain(KeyCode::Right), Key::ch('l')],
        Navigate,
        palette = false,
        hint = true,
        pane_can_take_focus
    ),
    // Walking the projects without leaving the pane: reviewing the todos of
    // one project after another would otherwise cost ← ↓ → each, three keys
    // where the pane takes the list's place. `<` and `>`, not `[` and `]`,
    // which need AltGr on German, French and Nordic keyboards. Off the hint
    // bar: its last pair is `? help`, which says them, and a bar that spent
    // that room on a walk would lose the one pair that explains the rest.
    cmd!(
        PanePreviousProject,
        "Previous project",
        "show the project above in the pane, staying in the pane and in the same section",
        &[Context::Detail],
        [Key::ch('<')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        PaneNextProject,
        "Next project",
        "show the project below in the pane, staying in the pane and in the same section",
        &[Context::Detail],
        [Key::ch('>')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        Settings,
        "Settings",
        "bases, workflow prompts, post-create actions, the ID counter, maintenance",
        TABS,
        [Key::ch(',')],
        Settings,
        palette = true,
        hint = false,
        not_busy
    ),
    cmd!(
        Attention,
        "Unfinished work",
        "what fastf left unfinished — what it is finishing by itself, what waits for a base, and what needs you, each with what settles it",
        TABS,
        [Key::ch('!')],
        Library,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        Reconcile,
        "Reconcile now",
        "finish now what fastf can: interrupted moves and copies, old copies, deleted projects' folders — rather than wait for the app to start it",
        TABS,
        [],
        Library,
        palette = true,
        hint = false,
        no_reconcile_running
    ),
    // --- the templates tab ------------------------------------------------
    cmd!(
        StripFilter,
        "Show this template's projects",
        "filter the library by the selected template and go back to it",
        TEMPLATES,
        [Key::ch('f')],
        Templates,
        palette = true,
        hint = true,
        has_studio_selection
    ),
    cmd!(
        StudioEdit,
        "Edit this template",
        "open the selected template in the builder",
        STUDIO,
        [
            Key::plain(KeyCode::Enter),
            Key::ch('e'),
            Key::plain(KeyCode::F(2))
        ],
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
        [Key::ch('n'), Key::ch('+')],
        Templates,
        palette = true,
        hint = true,
        not_busy
    ),
    cmd!(
        Guide,
        "Template guide",
        "how templates work, and a walkthrough that builds your first one",
        GUIDE,
        [Key::ch('H')],
        Templates,
        palette = true,
        hint = true,
        always
    ),
    cmd!(
        GuideNext,
        "Next page",
        "forward through the guide — and out of it at the last page",
        READER,
        [
            Key::plain(KeyCode::Right),
            Key::ch('l'),
            Key::plain(KeyCode::Enter)
        ],
        Templates,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        GuidePrevious,
        "Previous page",
        "back one page",
        READER,
        [Key::plain(KeyCode::Left), Key::ch('h')],
        Templates,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        StudioFromFolder,
        "Template from a folder",
        "generate a template out of a folder that already has the shape you want",
        STUDIO,
        [Key::ch('I')],
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
        BuilderEditText,
        "Edit",
        "F2 edits wherever it is pressed: here, the highlighted part, variable or file",
        BUILDER,
        [Key::plain(KeyCode::F(2))],
        Templates,
        palette = false,
        hint = false,
        builder_text_row
    ),
    cmd!(
        BuilderAdd,
        "Add",
        "a new variable or file at the end of the list",
        BUILDER,
        [Key::ch('a'), Key::ch('+')],
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
    cmd!(
        BuilderSave,
        "Save the template",
        "write it to the templates folder, from anywhere on the section list",
        BUILDER,
        [Key::ch('s'), Key::ctrl('s')],
        Templates,
        palette = false,
        hint = true,
        builder_list_closed
    ),
    cmd!(
        BuilderExplain,
        "Toggle the explanation panel",
        "show or hide the panel that explains the highlighted part and shows what it would produce",
        BUILDER,
        [Key::ch('i')],
        Navigate,
        palette = true,
        hint = false,
        always
    ),
    // --- the settings list -------------------------------------------------
    cmd!(
        SettingsFilter,
        "Filter the settings",
        "type to narrow this screen to the settings you are looking for",
        SETTINGS,
        [Key::ch('/')],
        Settings,
        palette = false,
        hint = true,
        always
    ),
    cmd!(
        SettingsEditText,
        "Edit the value",
        "F2 edits wherever it is pressed: here, a value on its line; a yes/no or a choice is Enter's",
        SETTINGS,
        [Key::plain(KeyCode::F(2))],
        Settings,
        palette = false,
        hint = false,
        settings_text_row
    ),
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
        Interrupt,
        "Interrupt",
        "cancel a running job, else close what is open, else leave at once",
        G,
        [Key::ctrl('c')],
        Navigate,
        palette = false,
        hint = false,
        always
    ),
    cmd!(
        Quit,
        "Quit",
        "leave fastf",
        TABS,
        [Key::ch('q')],
        Navigate,
        palette = true,
        hint = false,
        always
    ),
    cmd!(
        Back,
        "Back",
        "one step back: cancel a running job, leave the pane, clear the search, the filter, the marks — then quit",
        BACKSTEP,
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
