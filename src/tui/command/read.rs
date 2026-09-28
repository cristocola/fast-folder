//! Reading the registry: the keymap, the hint bar, and the words a key line is
//! built from.

use super::*;

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
    // What the bar is for comes first: the verbs you can use where you are.
    // Then the ways to ask — help, the palette — which are the same everywhere
    // and are therefore the ones a narrow window can afford to lose.
    //
    // The rule is stated on the category rather than on "is it global",
    // because the palette stopped being a global command the day it stopped
    // opening itself, and a bar that led with `c commands` on every screen was
    // the whole of that change showing through.
    //
    // **The pane's bar is what the pane does.** The verbs it shares with the
    // list — open, terminal, copy the path, mark, new, the tab switch — are on
    // the list's bar and in the action menu, and in the pane they crowded out
    // the pane's own: its row actions, the way back, and help.
    let pane_keeps = |c: &Command| {
        ctx != Context::Detail
            || !c.contexts.contains(&Context::Projects)
            || matches!(
                c.id,
                CommandId::Search | CommandId::Actions | CommandId::FocusList
            )
    };
    let mut ranked: Vec<&Command> = COMMANDS
        .iter()
        .filter(|c| c.hint && (c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global)))
        .filter(|c| pane_keeps(c))
        .filter(|c| (c.available)(app) != Availability::Hidden)
        .collect();
    // A stable sort, so declaration order decides within each group — which
    // is why `? help` still comes before `c commands`, as it always has.
    //
    // **Except the doors.** Where the pane takes the list's place, whichever
    // of the two is out of sight is one key away and nothing on screen says
    // so: that key leads, so a narrow bar never cuts the way in or out.
    let door = if app.pane_behind_list() {
        Some(CommandId::FocusDetail)
    } else if app.pane_over_list() {
        Some(CommandId::FocusList)
    } else {
        None
    };
    ranked.sort_by_key(|c| {
        let asking = c.category == Category::Help;
        (
            Some(c.id) != door,
            asking,
            !asking && !c.contexts.contains(&ctx),
        )
    });
    for c in ranked {
        let Some(key) = keys_in(ctx, c).first().copied() else {
            continue;
        };
        let label = key.label_in(&app.theme.glyphs);
        let title = hint_title(c.id, c.title, app);
        // **A bar never says one verb twice.** On a pane row Enter already
        // edits, F2's `edit` would repeat it; on a todo, where Enter ticks,
        // F2 is the one that says it.
        if out.iter().any(|(_, said): &(String, &str)| *said == title) {
            continue;
        }
        let cost = label.chars().count() + 1 + title.chars().count() + 2;
        if used + cost > width && !out.is_empty() {
            break;
        }
        used += cost;
        out.push((label, title));
    }
    out
}

/// Whether the edit open in the pane is the line a new todo is typed on.
fn adding(app: &App) -> bool {
    app.pane_edit
        .as_ref()
        .is_some_and(crate::tui::app::pane::PaneEdit::is_adding)
}

/// Whether the edit open in the pane is the line a new phase is named on.
fn naming(app: &App) -> bool {
    matches!(
        app.pane_edit,
        Some(crate::tui::app::pane::PaneEdit::Line {
            target: crate::tui::app::pane::EditTarget::NewPhase { .. },
            ..
        })
    )
}

/// What Enter does on the pane row under the cursor, in one word.
fn pane_verb(app: &App) -> &'static str {
    use crate::tui::app::pane::PaneRow;
    match app.pane_rows().get(app.pane_cursor) {
        Some(PaneRow::Todo { .. }) => "toggle",
        Some(
            PaneRow::AddTag
            | PaneRow::AddNote
            | PaneRow::AddTodo
            | PaneRow::AddPhase
            | PaneRow::Adding,
        ) => "add",
        Some(PaneRow::EarlierNotes(_)) => "show",
        _ => "edit",
    }
}

/// **The keys of `command` that actually fire in `ctx`.** In a text-entry
/// context a field has first refusal: every printable key is a letter of what
/// is being typed, and the caret's chords are the field's
/// (`LineEdit::CLAIMED`). Neither ever reaches the registry, so neither may
/// appear in that context's help or on its hint bar — `? help` over a rename
/// prompt where `?` types a question mark is the registry telling a lie about
/// itself, which is the one thing it exists not to do.
pub fn keys_in(ctx: Context, command: &Command) -> Vec<Key> {
    if !ctx.is_text_entry() {
        return command.keys.to_vec();
    }
    command
        .keys
        .iter()
        .copied()
        .filter(|k| {
            k.typed().is_none() && !crate::tui::widgets::input::LineEdit::CLAIMED.contains(k)
        })
        .collect()
}

/// Whether a field takes this key before the registry ever sees it.
pub fn field_claims(key: &Key) -> bool {
    crate::tui::widgets::input::LineEdit::CLAIMED.contains(key)
}

/// **The one place the arrows are spelled.** Seven surfaces used to write
/// `↑↓` into a key line by hand — the palette's, the picker's, the pager's,
/// the search bar's, the preview's, the guide's and the one every list on a
/// dialog shares — which is six copies more than a registry exists to allow,
/// and the reason `Down` and `Up` are `hint = false`: a bar that led with the
/// arrows on every screen would spend its width saying what a highlighted row
/// already says.
///
/// The labels come from whichever command binds the arrows in `ctx`, so a
/// rebinding reaches every line. The verb is the surface's own: you *choose*
/// from a list of things to do, you *move* through a list of things to pick,
/// and you *scroll* a body of text.
pub fn movement_pair(
    ctx: Context,
    g: &crate::tui::theme::Glyphs,
) -> Option<(String, &'static str)> {
    let by = |code: KeyCode| {
        COMMANDS
            .iter()
            .filter(|c| c.contexts.contains(&ctx) || c.contexts.contains(&Context::Global))
            .find_map(|c| keys_in(ctx, c).into_iter().find(|k| *k == Key::plain(code)))
    };
    let (up, down) = (by(KeyCode::Up)?, by(KeyCode::Down)?);
    let what = match ctx {
        Context::Modal | Context::Guide => "scroll",
        Context::Palette | Context::Pick | Context::SearchEdit => "move",
        _ => "choose",
    };
    // Two glyphs read as one pair; two words need a stroke between them.
    let joint = if g.is_ascii() { "/" } else { "" };
    Some((
        format!("{}{joint}{}", up.label_in(g), down.label_in(g)),
        what,
    ))
}

/// The hint bar has one line, so a few titles get a shorter form there —
/// and Enter in the pane says what it will do to the row under the cursor.
pub fn hint_title(id: CommandId, title: &'static str, app: &App) -> &'static str {
    match id {
        CommandId::Palette => "commands",
        CommandId::Actions => "actions",
        CommandId::FocusList => "list",
        // Where the pane is out of sight, the key is the way to it, and says
        // what is there rather than where.
        CommandId::FocusDetail if app.pane_behind_list() => "details",
        CommandId::FocusDetail => "pane",
        CommandId::PaneEditText => "edit",
        CommandId::PaneAdd => "add",
        CommandId::PaneEdit => pane_verb(app),
        // On the add line Enter writes one more and opens the next, and Esc
        // is the end of the run, not the loss of anything.
        CommandId::PaneEditConfirm if adding(app) => "add",
        // Naming a phase, Enter goes on to its first todo.
        CommandId::PaneEditConfirm if naming(app) => "next",
        CommandId::PaneEditConfirm => "keep",
        CommandId::PaneEditSave => "save",
        CommandId::PaneEditCancel if adding(app) => "done",
        CommandId::PaneEditCancel => "cancel",
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
        CommandId::Templates => "templates",
        CommandId::StripFilter => "its projects",
        CommandId::ActionsRun => "run",
        CommandId::StudioEdit => "edit",
        CommandId::StudioNew => "new",
        CommandId::StudioFromFolder => "from a folder",
        CommandId::StudioDelete => "delete",
        CommandId::Guide => "guide",
        CommandId::BuilderOpen => "open",
        CommandId::BuilderAdd => "add",
        CommandId::BuilderRemove => "remove",
        CommandId::BuilderMoveUp => "up",
        CommandId::BuilderMoveDown => "down",
        CommandId::BuilderSave => "save",
        CommandId::BuilderExplain => "explain",
        CommandId::SettingsChange => "change / run",
        CommandId::SettingsFilter => "filter",
        CommandId::MarkToHere => "mark to here",
        CommandId::PaletteRun => "run",
        CommandId::PaletteClose => "close",
        CommandId::PromptConfirm => "confirm",
        CommandId::PromptNewline => "new line",
        CommandId::PromptCancel => "cancel",
        CommandId::PickChoose => "choose",
        CommandId::PickToggle => "toggle",
        CommandId::PickCancel => "cancel",
        CommandId::SearchAccept => "keep",
        CommandId::SearchCancel => "clear / leave",
        _ => title,
    }
}
