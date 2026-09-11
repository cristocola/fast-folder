# CLAUDE.md — `src/tui/`

Every interactive terminal surface. The guided app is how the tool is used day
to day, so its polish is the product's polish: the list draws before a single
folder has been walked, cancel is always possible, typed input is never thrown
away by a later validation failure, and a network-share stall is never a frozen
screen.

The root `CLAUDE.md` has the layering rule and the module list; `src/core/CLAUDE.md`
has the engine underneath. The ratatui rebuild landed as v3.0.0 (PRs #35–#42
and the consolidation pass, #43); what follows is the design as it stands.

# The look

A command centre, not a demo. **Muted and cool, minimal and sophisticated,
robust as a rock.** The rules, in the order they matter:

- The terminal's own text colour carries the content. Slate grey recedes. One
  steel-blue accent says what has focus. Green, amber and red appear only where
  they *mean* success, a warning, a failure — never as decoration. In truecolor
  (`Theme::rich`) every colour is desaturated; in ANSI the same roles map to
  the plain sixteen, used sparingly. No magenta, no rainbow tags.
- Bold is rare — the app's name, the selected row — so it keeps its weight.
- Glyphs are few and each has one job (`▸` the cursor, `✓` a mark, `●` a tag,
  `⚠` a warning, `⌕` search). No decorative symbols in titles or counters.
- Whitespace and alignment do the structuring: three spaces between facts on
  a line, right-aligned figures, plain-word panel titles.
- Every state is visible and quiet: loading (`(from index)` and a spinner, a
  dialog that says `reading…` the moment its key is pressed), empty (one
  sentence saying what to do, inside the box), an error (one line, or a
  dialog when it has more to say), disabled (dimmed, with the reason on the
  key — `Move` with one base says which base is missing, `o` and `t` without
  a display say so).
- Columns are measured from their content, never fixed: a title can never run
  into its description. A count reads `1 base`. A confirmation is sized to its
  question and names every folder it is about.
- Depth is in what it can do, not in what it shows at once. A screen shows what
  is needed to act; the palette and help hold the rest.

Look at every screen you build with the screenshot tool
(`tests/tui_pty/screenshot.rs`) before you write its snapshot.

# The guided app

`tui::run(Entry)` is the door: `fastf` (`Entry::Menu`), `fastf recent`
(`Entry::Recent`, the flags as a `Preset` chip and the rows already read) and
`fastf search` (`Entry::Search`, the terms in the bar). `require_tty` runs
**before** the screen is taken, with the same message the menu used, so an app
that cannot be driven never switches a terminal nobody is holding to the
alternate screen.

## The shape: model, message, effect, view

`app::App` is the model; `Msg` (`msg.rs`) is everything that can happen to it;
`app::update(&mut App, Msg) -> Vec<Effect>` is the one state transition, and
**it performs no I/O** — everything it wants done comes back as an `Effect`
(`effect.rs`) that `runtime.rs` carries out. `view::view(&App, &mut Frame)`
takes the app by shared reference, so a frame cannot change state and any state
a test can construct can be rendered.

That split is load-bearing twice over. `tests/tui_update.rs` drives the state
machine with no terminal at all — a fixture `App`, messages in, effects out —
and `tests/tui_snapshots.rs` renders frames through ratatui's `TestBackend`.
And it is what keeps a slow filesystem out of the key handler: nothing in
`update` blocks, because nothing in `update` reads a disk.

The viewport is the app's, not ratatui's. `LibraryState.offset` is kept by
`clamp_viewport` (arrows wrap, page keys clamp, the window moves by the minimum
— `widgets::nav`), and the frame builds a `TableState` from it each time. A
`TableState` that lived in the view would need `&mut App` to keep its offset,
and a `TableState` rebuilt from scratch each frame — what the prototype did —
throws the offset away and re-derives the window every draw.

## One registry

**Every quit goes through `App::quit`.** There are three gestures — `q`, the
palette's own entry, and the too-small-window guard — and `CommandId::Quit` ran
`Effect::Quit` on the spot, so a template worked on for ten minutes went with
one keystroke while Esc on the same screen asked first. `close_top` owns the
question; `quit` owns who has to ask it, and `ConfirmThen::DiscardTemplate`
carries `then_quit` so answering it does what was asked rather than stopping one
level short.

**A dialog carries its target, and never re-reads the selection at submit.**
`TextThen::Rename`/`Delete` and `ConfirmThen::Unregister` hold the project's
path. The prompt text is built once from the row under the cursor and the action
used to be built again from whatever was selected when Enter landed — so a
discovery arriving underneath, which moves the cursor when the named row has
left the snapshot, pointed a destructive verb at a *different* project from the
one on screen. `App::project_at` resolves the path in the current snapshot and a
target that is gone is a refusal, never a neighbour.

**A worker that answers must say which question it is answering.**
`Msg::TemplateSourceLoaded` replaced whatever builder was on top with whatever
landed; `Builder::pending` carries the slug now. `on_template_loaded` and
`TemplateViewLoaded` had always checked — this was the one that did not, and on
a slow disk Enter, Esc, Enter on a second template let the first read arrive and
become the second's contents.

**Every command is declared once, in `command.rs`**, with its title, its
description, the contexts it fires in, its default keys, its category, and
whether the palette and the hint bar show it. The keymap (`lookup`), the fuzzy
palette (`palette_entries`), the help overlay (`help_lines`), the hint bar
(`hints`) and every dialog's own key line all read that list; the prototype
carried four copies of its key table and they had already drifted.
`tests/tui_commands.rs` holds the invariants: one key means one thing per
context (global bindings count everywhere), every id is declared exactly
once, every bound command is in its context's help.

The dialogs are contexts too — `Actions`, `Studio`, `Builder`, `Settings` —
and their handlers end in `lookup_and_run`: a key a dialog does not consume
itself (a text field's, `y`/`n`'s) is whatever the registry binds there.
That is how `?` opens the help for wherever the keys go right now, and how a
verb's own letter runs it from inside the action menu. `Close` (Esc, `q`) is
one command for every dialog, one level at a time; `Quit` and `Back` are the
dashboard's own, because the one-key-one-meaning invariant counts global
bindings in every context — it is what caught `g` meaning both "first row"
and "template from a folder" in the studio, which is the collision the
horizontal axis and the movement grammar finally took apart. The keys a text widget consumes
(Ctrl-S in a text area, Tab in a form) are the one honest exception: the
widget's key line names them.

**A sentence that names a key reads it from here.** `command::key_of(id)` and
`command::NO_TEMPLATES` exist because eight did not: three different sentences
for "no templates yet", one of them naming `T` — the tab switch — where the
registry says `n`, and seven keys spelled into status lines and empty states.
They were all correct on the day they were written, which is the drift the one
registry exists to prevent. A verb with no key of its own shows `Enter` in the
action menu, because that is what runs the row under the cursor; an empty column
reads as a row that cannot be run.

**A key line is cut at a whole pair, and the way out comes before the extras.**
`view::builder::key_line` takes the width it is drawn in and drops entries from
the end — it used to take every pair it was handed and let the terminal cut the
last one wherever it landed, so a narrow settings dialog advertised
`Esc leave i`. Half an entry is a key line saying something that is not a key.

**Success wears the theme's tick, and `App::good` is where it is put on.**
Twelve of `runtime::run_action`'s messages carried a literal `✓`, which
`Glyphs::ascii` maps to `+` — so on a legacy Windows console they drew a
replacement box beside the app's own correctly-themed messages. `run_action`
runs on a worker with no theme to ask.

An `Availability` is a function of the app: `Disabled(reason)` is listed dimmed
and pressing its key shows the reason; `Hidden` is not bound at all (Move with
no other mounted base, Clear-filter with no filter).

**A command may be palette-only.** `BackToLibrary` lost its `←` when the
horizontal axis became focus and `T` and Esc already do the job; it stays
declared with `palette = true` and no keys, the `ReautoTags` precedent, so the
palette can still name it and the help does not list a key that is not there.

## The movement grammar, and the horizontal axis

**One set of movement keys, and every list has all of it.** `SCROLLERS` is the
single context list the arrows, the page keys, the halves and the jumps to the
ends are all declared over, because three narrower lists is how they drifted:
`PgUp`/`PgDn` stopped short of the action menu and the builder and `Home`/`End`
stopped short of the templates tab as well — so the two lists that cannot be
searched were the two that could only be walked a row at a time, and
`page_top_modal`'s `Modal::Actions` arm sat there as dead code proving it.
`tests/tui_commands::every_list_binds_the_whole_movement_grammar` is the guard:
a context that binds one of the eight binds all eight.

**An arrow and its vim letter are one key**, never bound apart
(`an_arrow_and_its_vim_letter_are_bound_together`). The exception is a
text-entry context, where every printable character is the text: the palette
binds `↓` and cannot bind `j`.

**`g` and `G` are first row and last row, with no exception to remember.** They
were the templates tab's "from a folder" and "the guide", which is why that tab
had no jump keys at all; the two verbs are `I` and `H` now. `H` rather than
`Ctrl-g` because a hint bar is a fixed width and five extra columns pushed
`c commands` off the end of it — the key that costs the least is the one that
reads the same length as what it replaced.

**The horizontal axis is focus: `→`/`l` go into the pane beside the list,
`←`/`h` come back to the list. They never run anything.** For one release
`→` was `Descend` — "whatever Enter does here", so it opened the action menu,
edited a template, ran a verb, changed a setting — and `←` was `Ascend`,
closing a dialog. An arrow that executes is an arrow you cannot lean on to
look around, and it made the pane's own Enter impossible: the pane is an
editor now, and Enter there has to mean *edit this row*. Both are gone;
Enter and Esc are the confirm and the back, and the arrows only ever move the
cursor. `FocusList` and `FocusDetail` are declared over `PANED` (both tabs)
and dispatch on `screen`; each is **hidden rather than a no-op** where it has
nowhere to go (`pane_has_focus`, `pane_can_take_focus`), so the help never
lists a key that does nothing. **The axis never quits**: leaving a tab is
Esc's ladder and `T`; `BackToLibrary` is palette-only. The templates tab's
pane takes focus too, at any width — its split lives in
`layout::templates_panes` so `studio_scroll_max` and the view measure the
same box; it measured the *library's* pane before, which is closed under a
hundred columns while the template pane is always drawn, and Tab could not
reach a pane that was right there. `tests/tui_commands::
the_horizontal_axis_only_moves_focus_or_turns_a_page` holds all of it.

Two surfaces own their own left and right, and both are the same exception a
text area's `Ctrl-S` is:

- **A field.** `LineEdit`, `TextArea` and a `Form`'s choices take the arrows as
  a caret or an option.
- **A reader.** The guide has `Context::Guide` for exactly this — with the
  pages declared (`GuideNext`, `GuidePrevious`) rather than hand-written in
  `on_guide_key`, `←` turns a page there and the help can say so. A page is
  horizontal; that is the one place the axis means something other than
  focus. Forward off the last page leaves the guide, whichever key is being
  pressed; the alternative is a reader pressing a key against the end of a
  document.

**Ctrl-C is a command** (`CommandId::Interrupt`), so the key that cancels a
running job is in the help — it was in no help, no hint bar and no palette. It
is still dispatched from `on_key` directly rather than through `lookup`: it has
to answer from inside a text field and from under a modal that consumes every
key, and no availability state may swallow it.

**In a text-entry context, everything printable is the text, and only a key a
field cannot hold reaches the registry.** That one rule is what gave
`Context::SearchEdit` and `Context::Palette` a help that is true — they had no
commands at all, so `?` there described a screen you were not on — and it is
why `Ctrl-p` opens the palette from the search bar while `c` types a `c`.
Inside the palette `Ctrl-p` is the previous entry, which is possible because
the opener is declared over every context **except** `Palette`: a palette that
opens the palette is the one context where that key had something better to do.
Esc is the way out of every context, whichever command owns it there
(`every_context_has_help_and_a_way_out` checks the key, not the id), and the
search bar is a rung of Esc's own ladder — an empty bar is left, a bar with
something in it is cleared and stays open to be retyped.

**The hint bar's order is stated on the category, not on "is it global".** The
verbs you can use here come first and the ways to ask — `? help`, `c commands`
— come last, because they are the same everywhere and are what a narrow window
can afford to lose. It read "own commands, then global ones" until the palette
stopped being global, at which point `c commands` led every bar on every
screen.

## Every key line is read, and the field goes first

**Seven surfaces used to write `↑↓` into a key line by hand** — the palette's,
the picker's, the pager's, the search bar's, the preview's, the guide's and the
one every list on a dialog shares. `command::movement_pair(ctx)` is the one
place the arrows are spelled now; it reads the labels off whichever command
binds them in that context, so a rebinding reaches every line, and it carries
the surface's own verb — you *choose* from a list of things to do, *move*
through a list of things to pick, *scroll* a body of text.

`Down` and `Up` stay `hint = false` and the pair is prepended only where
`Context::hints_movement()` says so: **a dialog says how to move in it and the
dashboard does not**, because a table with a highlighted row and a scrollbar
beside it already says which way the arrows go, and the bar's width is better
spent on the verbs. The search bar is the exception that proves it — there the
arrows move the list *underneath* what is being typed, which nothing on screen
says.

**The hint bar spells nothing at all.** It hand-wrote six pairs, which is
exactly why four dialogs had a `Context` with no commands in it: nothing needed
them, because the bar already knew. `Context::Prompt` (a one-line prompt, a
quick note, the first-run question) and `Context::Pick` (a picker of one or of
several) exist so those keys can be declared, and `Availability` does the rest —
`PromptNewline` is `Hidden` outside a note, `PickToggle` outside a multi-pick.
The bar reads `command::hints` for every dialog that does not draw its own key
line inside its frame.

**A field has first refusal, and `keys_in` is the other half of that rule.**
In a text-entry context every printable key is a letter of what is being typed
and the caret's chords are the field's (`LineEdit::CLAIMED`, which is where
they are listed once). Neither ever reaches the registry — so
`command::keys_in(ctx, command)` takes them back out of what that context
advertises, and `? help` no longer appears over a rename prompt where `?` types
a question mark. What is left is true: `F1 help`, `Ctrl-p commands`.

That rule is what lets the search bar be on `SCROLLERS` like every other list.
Its arrows are `CommandId::Down` and `Up` themselves; `Ctrl-u` there is
`LineEdit`'s kill-to-start because the field claims it, and `keys_in` keeps
`HalfUp` off that context's help for the same reason. `on_search_key` is three
lines now: the field, then the registry, then the field again for the caret
keys nothing binds.

**`tests/layering.rs::no_key_line_is_written_by_hand`** holds it — a source
scan, because a string literal is not something a runtime test can see. No
module under `src/tui/view/` may write an arrow as a key, and
`view/dashboard.rs` may write no key label at all. The key lines a widget draws
inside its own frame keep their literals: `Ctrl-S` in a text area and `Tab` in
a form are the widget's, not the registry's.

**The activity indicator is the theme's.** `const SPINNER` sat in
`view/dashboard.rs`, the one glyph in the app outside `Glyphs` and therefore
outside the ASCII alphabet everything else answers to. It is `Glyphs::spinner`
with `spin(ticks)` as its one expression, and `dashboard_ascii_80x24` checks
its frames alongside the other ten.

**What to type is not a key.** The palette's `#` prefix moved off the hint bar
— where every pair is a command and `#` is nothing the registry knows — onto
the palette's own blank row, under the query, while the query is empty.

`ClearSearch` has no key. It gave `Ctrl-u` up to `HalfUp`, and it is the one
command that could afford to: Esc's first rung already clears the query, and
inside the bar `Ctrl-u` has always been `LineEdit`'s kill-to-start — the same
physical key meaning two things one keystroke apart.

Keys are normalised into `Key` (`Char` with the Ctrl and Alt flags; shift
folded into the character; Ctrl-letters lower-cased) by the input thread, which
also drops `KeyEventKind::Release` — Windows delivers one for every press.

**Naming trap, now historical.** While the CLI's prompts were dialoguer's,
`tests/layering.rs` grepped `src/tui` for `Input::`, `Confirm::`, `Select::`,
`Sort::` and `MultiSelect::` outside `prompt.rs`, and a type whose name merely
*ended* in one tripped it — `LineInput::new()` contains `Input::`. That is why
the types are called `LineEdit`, `Order` and `PickState`. The rule is gone with
dialoguer; the names stay, because renaming them now would be churn for nothing.

## The runtime owns the screen

`runtime.rs` is the one module that takes the terminal: raw mode, the alternate
screen and bracketed paste, **on stderr** — the stream fastf has always drawn
prompts on, so `fastf > log` still opens the app and stdout keeps deciding
output format. `Runtime::init` is the choke point that calls
`tty::mark_interactive_surface` (the one `live_select` used to be), and it
installs the panic hook that restores the screen — for a panic **on the main
thread only**. A worker's panic is caught by `spawn_worker` and becomes a
warning; restoring the screen for it would tear the frame down under a session
that is still running.

**The terminal is always given back.** The second signal from outside (`kill
-INT` twice, a terminal that sends one on close, SIGHUP) exits from the
handler, where nothing of ratatui may run: `Runtime::init` registers
`restore_on_signal` with `interrupt::set_restore`, which writes the escapes
that undo the mouse, the paste reports and the alternate screen with raw
system calls and puts back the terminal settings `tty::remember_cooked_mode`
captured before raw mode was ever enabled. `inline.rs` registers its own for
its rows, and installs a panic hook of its own. Ctrl-Z is a command
(`Suspend`): the input thread is paused, the screen released, `SIGTSTP`
raised, and `fg` retakes the screen at whatever size the window has — every
suspend ends with a `Resize` message for that reason.

**Pasted text goes into a field, never to the keys.** The input thread hands
a run of printable keys that arrives faster than a hand can type over as one
`Msg::Paste` (`collect_burst`) — what a terminal without bracketed paste
delivers — and `on_paste` gives a single-line field the first line (and says
how many it dropped), a text area every line, and the dashboard nothing but a
status line. A pasted note once ran as a dozen commands.

**The loop idles.** The input thread polls at 50 ms for two seconds after a
key and once a second after that; `poll` returns the moment a key comes, so
the cost is wakeups, not latency. The main loop's idle wake is a second.

**Never call `Terminal::clear`.** In ratatui 0.30 it asks the terminal where
its cursor is and waits up to two seconds for the answer, which a pty under
test never sends. A fresh `Terminal` draws its first frame against an empty
back buffer and the alternate screen starts blank, so there is nothing to clear.

**The loop blocks.** `recv_timeout` on the one channel; a wake without a
message is a `Tick` only while `App::needs_tick` says something is moving — a
job, a toast about to expire, a size cell still pending — and otherwise just a
look at `interrupt::is_set`. A burst of messages (a paste, a batch of sizes) is
drained and drawn once. On each tick the runtime diffs `SizeScanner::cells_for`
against what it last reported and hands the app only the news.

**A thread that dies says so.** `spawn_worker` catches a panic and warns, and
the input thread and the detail reader now do the same in their own shapes —
they are loops that outlive the requests they serve, so `spawn_worker` cannot be
reused for them. It matters because the runtime holds its own `Sender`:
`recv_timeout` never sees a disconnect, so a dead input thread left the main
loop drawing a live-looking frame that answered nothing, with an external signal
the only way out and nothing on screen to say why. `InputEnd` distinguishes
"stop() was called" from "the terminal stopped answering", and only the second
is reported. The input thread is also spawned **before** the screen is taken,
which its own doc comment always claimed: on a spawn failure `init` returned
without reaching `shutdown`, so `SCREEN_OWNED` stayed set and the error printed
onto an alternate screen nobody would see again.

**Where work runs.** Discovery, the header's summary (probes, indexes,
templates, `list_incomplete`), on-demand metadata and every `operations::*`
call go to a worker (`spawn_worker`, a 4 MiB stack because a Windows thread
gets 1 MiB and discovery walks under `MAX_WALK_DEPTH`). The detail pane has one
worker with a latest-wins slot, which is the debounce for a held arrow key.
Reveal, terminal and clipboard spawns run on a worker too — `reveal_folder`
blocks on `.status()` and `wl-copy` can hang. The scanner's `request`/`forget`
are inline: they only take a mutex.

**Ctrl-C is a key.** In raw mode it never becomes SIGINT. The app closes a
dialog with it, else quits with `Exit::Interrupted`; `tui::run` then calls
`interrupt::raise()` and returns an error so `main` prints `aborted.` and exits
130 exactly as a signal would have. An external SIGINT is seen on the idle
wake.

**`diag` goes through the channel.** `Runtime::init` installs a `diag` sink
that turns `warn`/`note` into `Msg::Diag`; a worker's `eprintln!` would land on
the alternate screen mid-frame and be scrolled away on exit.

## Discovery, patches and generations

The first frame's counts come from `library::index_summary` — the index and
nothing else, labelled `(from index)` — while `library::discover` runs on a
worker; a pty test asserts opening the app over a fresh index performs one
`discover` and zero `scan_base`.

**A content mutation patches its row; only a structural change reloads.**
`ListChange` (`effect.rs`) is how a finished action reaches the list: `Patched {
project, stale }` replaces the row **by id** (a rename or a move changes the
path), drops the size snapshots in `stale`, and lets `recompute` decide whether
the row still satisfies the query; `Removed { path }` drops it; `Reload`
discovers again. Adding one tag must never re-read every `PROJECT_INFO.md` in
the library, and the pty suite traces that it does not.

`LibraryState.generation`/`inflight`: a discovery answers with the generation it
was sent with and is installed only if it is the one in flight. A patch or
removal while one is in flight sets `dirty`, and the landing answer triggers
one more discovery, because it may predate the change. Selection survives a
re-filter, a re-sort and a reload by **path**; snapshot indices do not.

## The table

**The folder name is never cut.** It is the one column that tells projects
apart, and a row is eaten from the right. `view::projects::choose_columns`
measures the widest name and adds the optional columns only while it still fits
whole, in the order a person misses them: the size, the date, the base, the
template, the tags. (The old row put base and template before the date; in a
table with a detail pane beside it the size is what the row is for — the name
carries the date already — and the pane shows the rest.) Widths are measured from the rows, never
from the sizes, so a landing snapshot cannot reflow the table; the size cell is
`rows::SIZE_CELL` wide and right-aligned, header included.

**Election stops at the first column that does not fit.** The greedy version
kept trying, so a narrower later column slipped in past a wider earlier one and
a 60-column window drew a BASE column with no SIZE — which reads as a bug, not
as a priority.

**The base is promoted above the date when the rows come from more than one
base**, which is a question about the rows on screen and not about the
configuration: two bases with one unmounted shows one base's projects, and a
column repeating one word earns nothing. `LibraryState.many_bases` and
`base_width` are measured in `recompute` beside `widths`, because
`App::table_min_width` has to claim the column the table is about to elect — a
library of ninety-character names left the split with room for the size and
nothing else, and the one column saying which drive a project is on never
appeared on the machine that had four of them.

**Everything is measured in display columns, never in bytes or characters.**
`RowWidths` measured three of its four in bytes while the comment on the fourth
explained why that is wrong — a base folder called `Проекты` is seven columns
and fourteen bytes — and `widgets::input::visible_window` built its window from
`chars()` and returned a char index that `render_line` then added to
`prefix.width()`, so a CJK folder name in a rename prompt put the caret at
roughly half the column it belonged in. `view::fit` and `view::pad` were always
right; everything else has caught up.

**Every column is measured, the tags included.** The tags cell was a `Fill(1)`
remainder sharing the slack with the name, so one column of gutter cut the first
tag's last letter — and a tag cut mid-word names a different tag. It is
`tag_cell_width` now, and absent entirely when no row on screen carries one.

**The table reserves one column of right gutter, always.** The last cell is
right-aligned, so without it a size sits against the border glyph and reads as
cut off, with the scrollbar — drawn over the border column — landing on the
digits. It is reserved whether or not a scrollbar is showing: taking it back
when the list gets short would reflow every width as rows arrive, which is the
one thing measured columns exist to prevent.

**Nothing blocks on a size.** The list draws first with `scanning…` in the
cell; `util::size_scan` owns two workers, `request` **replaces** the queue with
what is on screen, selected row first. Snapshots last for the session; a
mutation's `stale` list is what `forget` is called with.

**Bases are probed, never `is_dir`-ed** — `paths::probe_dirs` on the summary
worker, because `is_dir()` on a dead SMB mount blocks for the operating
system's timeout.

## Search

`app::search::Query` splits the bar: anything `core::query` parses with an
operator is `structured` and evaluated by `core::query::evaluate` exactly as
`fastf search` would; the bare words are `free`. A row answers `tag:`,
`template=`, `created>` from a `Metadata` synthesised from the `Project`
(`row_meta`); a predicate on a template variable emits `LoadMeta` for the rows
that lack it and they fill in as chunks land. Relevance is the sort while there
are bare words, unless `s` chose one.

**Fuzzy is deliberately not very fuzzy.** The first build matched a word as a
subsequence of one string made of id, name, template, template name and tags
joined together, and that says yes to almost everything — `lrmx` found a dozen
rows. Two rules fixed it, both in `fuzzy.rs` and `library::match_fields`: a
word matches **inside one field** (name, id, template slug, template name, a
tag, a variable value), never across two; and it is a **substring first**, with
a fuzzy hit accepted only when its characters span at most the word's length
plus a third — a dropped or doubled letter, not letters picked from across the
name. Substring hits outscore fuzzy ones. The same `Fuzzy::match_all` ranks the
palette and the pickers.

## Modals and the palette

`ModalStack`: Esc pops one; what a picker's answer means is data (`Then`), not
a closure, so `update` stays inspectable. The palette ranks a title hit above a
description hit — `open` is *Open project folder* before it is "open the
action menu" — and Enter dispatches exactly the `CommandId` a key would.
`#`/`@` restricts it to projects.

## The single-project actions

The verbs on a selected project are native modals, not bridges.
`app/actions.rs` holds their states (`ActionsState`, `TextPrompt`, `Confirm`,
`MultiPick`); `command.rs` binds `Enter` and `a` to the action menu, `A` /
`Ctrl-T` to add / remove tags, `N` / `Ctrl-N` to the editor and inline notes,
`r m u D` to rename / move / unregister / delete, and `M` / `J` to the
read-only metadata and journal views. The action menu's rows come from the one
registry, ordered by display (`action_entries`); an entry that cannot run right
now is listed dimmed with the reason on the key, not hidden — pressing it says
why. Prompt texts and validators live in `validators.rs`, byte-identical to
the prompt-at-a-time flows they replaced.

A finished verb patches its row by the **path it had**, and only then by id
(`ListChange::Patched` carries `was`); the pty suite traces that the list is not
rescanned. The id alone was the key, because it survives a rename and a move —
but `copy-to` can put a second project with the same id on a backup drive, and
once that drive is a base, patching by id tags one row and shows it on the
other. The old path is unique whatever else is true. A typed confirmation that does not match keeps the text in the
prompt and says `name did not match — nothing deleted`. A move is a one-item
job on a worker (`spawn_worker`) with a `Progress` shared with the runtime and
a cancel flag: Ctrl-C during a move cancels the job instead of quitting. The
`$EDITOR` note flow suspends the screen (`Suspended::Note`) into the same
scratch-file flow as the CLI (`cli::note::note_from_editor`, made public for
it). Metadata and journal views load on a worker (`loaders.rs`) and render
read-only, the journal in the order the file holds it.

## The flows that build something

Create (`n`), apply (`E`) and register (`e`) are one shape, and `app/wizard.rs`
holds it: **a form, then a preview, then Enter**. All three answer a few
questions, show what answering them would do, and commit — so they are one
`Modal::Flow(Flow)` with a `Step`, not three screens.

**Every question is on screen at once.** `widgets/form.rs` is the form: Tab and
the arrows move, typing edits the field with the cursor, `←`/`→` change a
choice, Space opens a fuzzy picker over that choice's options (`Then::FormField`
— which is what makes twenty templates usable), Enter submits the whole form and
Esc abandons it. A sequence of prompts could only ask one thing at a time, so an
answer given three questions ago was invisible and a rejection at the end took
every earlier answer with it. Both defects are structurally gone.

**A refusal names its field.** `update` performs no I/O, so a path that must
exist cannot be checked there: the worker that builds the preview refuses with
`loaders::PreviewRefusal { field, error }`, and `Form::fail` puts the message on
that field and moves the cursor to it, with the typed text untouched. What
`update` *can* answer — a required variable left empty — it answers before any
worker is asked (`Flow::missing_required`).

**The preview is built by the code that commits.** `Effect::Preview(Request)`
and `Action::{Create,Apply,Register}` take the *same* `Request`, so the screen
cannot promise one thing and do another — which happened twice in this
codebase's history (a rename prompt offering `ID0001` while the commit wrote
`ID0011`; a preview header saying nothing would be created immediately before
creating it). The ID a create preview shows is still advisory: `operations::create`
recomputes the plan under the data lock, because reusing a previewed value is
how duplicate IDs were minted.

`confirm_create = false` sets `Flow::auto_commit`: the plan is still built, by
the same path, and then committed unasked. Skipping the *build* would skip every
refusal with it.

Esc at the preview goes back to the answers, and Esc again abandons the flow —
the app's Esc ladder, one step at a time. (A run of prompts cancelled
everything from anywhere; a form has somewhere to go back to.)

**Post-create runs on the main screen.** `git init`, the user's editor and a
template's own `commands` all want a terminal and print to it, so a finished
create asks for `Suspended::PostCreate` through `ActionOutcome::follow_up`
rather than running them on the worker. `ActionOutcome::select` then puts the
cursor on the new project once discovery has seen it — a create makes a row no
snapshot holds yet, so the selection is asked for by path and applied in
`Msg::Discovered`.

Register's shape is its own (`app/register.rs`) only in which questions apply:
the scope field hides the three that bulk registration cannot answer, because
`RegisterFlags::validate` refuses them on the command line for the same reason.
`cli::register::{plan_rename, recursive_targets, recursive_id_note}` are the
print-free halves both surfaces preview from.

## Two tabs

`App.screen` is `Screen::{Library, Templates}` and `T` switches between them.
**A tab, not a dialog.** Templates were an 84 %-wide modal over the library
*plus* a three-row strip along the bottom that showed the same counts and could
be filtered by pressing Enter on a card and nothing else — two halves of one
subject, neither of them a place you could work, and between them three rows off
every table. The strip is gone (`Focus` is `Projects | Detail` now, and the
focus ring with it), and `Studio` is state on the `App` rather than a modal,
because a tab you leave and come back to keeps its place and a modal cannot be
a tab.

The two tabs share every band but the middle one — the name, the tabs, the
bases, the status line, the keys stay where they are and only the work changes.
The search bar belongs to whichever tab is showing: the library's is the query
grammar, the templates tab's is a plain case-insensitive substring over the slug
and the name (`Studio::rows`), because a template list is tens of rows and a
fuzzy hit there says yes to almost every slug. Switching clears it, since the
two are searches over different things.

`Context::Studio` is gone; `Context::Templates` — which was the strip's — is the
tab's, and carries the verbs the studio had. `LISTS` shrank to the library's own
screen for the same reason: while the templates were a strip *on* that screen,
`n` meant both "new project" and "new template" in one hint bar.

`f` on the templates tab filters the library by the selected template **and
goes back to it**. The strip set the filter and left you looking at the strip,
which is the one place the answer is not.

The tab is ordered real templates first, then **alphabetically by display
name** — not busiest first, which is what a horizontal ribbon wanted. A list
you scan and search should be in the same order tomorrow, and creating one
project should not move a row. `TemplatesState::rebuild` is the one place the
list is built (the counts and the orphan slugs both), and `App::refresh_templates`
hands it to the tab: the two used to be built separately and drift, the strip
from the summary *and* the per-template counts, the studio from the summary
alone. `Studio::install` keeps the selection by slug, but **only once a real
template has been on the list to choose from** — discovery lands before the
summary, so the first list is nothing but orphan slugs, and keeping that
parked the cursor on `(registered)` for the rest of the run.

## The builder

Enter or `e` on the tab opens it: every template on disk with the selected
one's details beside it, read on a worker (`loaders::template_view`, which
renders `cli::template::describe` — the same lines `template show` prints, so
the two cannot drift). Its verbs are `n`, Enter, `I` and `D`.

**The builder is a list of a template's five parts, not a sequence of steps.**
The old one walked six steps and *then* offered a review menu to go back into
any of them, which is to say the review menu was the interface and the walk was
a tax. `app/studio.rs` holds the scratch `Template` and the section the list has
open; every row says what that part currently holds, so the list is the summary
the old builder printed after each step. Nothing is written until Save, and Save
says `Cannot save:` with `Template::validate`'s own words rather than writing
something that will not load.

**The builder stays up until the write has landed.** `save_template` used to
pop the modal in the same breath as handing the effect over, so a refusal from
*under* the data lock — an occupied slug, a lock held by another terminal, a
full disk — arrived with nothing to land on: the template and every answer in
it were gone, and one red line on the status bar was all that was left.
`Builder::saving` is the flag; `on_action_done` pops the modal on the success
path only, and puts the refusal on the list otherwise, beside the two arms that
already do this for `Settings` and `Onboarding`. While it is set the builder
takes no keys at all, Esc included — a write already sent cannot be cancelled,
and pretending otherwise is worse than waiting.

**Leaving asks when there is something to lose.** `Builder::original` is the
template as the builder opened it, and `is_dirty` is a whole-document
comparison, so correcting a typo and correcting it back is not "worked on" — a
question nobody needs is the fastest way to teach people to answer it without
reading. Esc, `q` and Ctrl-C all reach it: `close_top` asks through
`ConfirmThen::DiscardTemplate`, and the Ctrl-C branch in `on_key` defers to
`close_top` for a dirty builder rather than popping the modal itself, because
it was the one gesture that reached past the question and it did not even
leave a status line behind.

**A new template may not land on an occupied slug.** The rename guard in
`operations::save_template` lived inside `if let Some(original)`, and a new
template carries `None` — so typing `general` as the slug of a new template
overwrote the bundled one and said `✓ Saved`. The authority is core, keyed on
the **manifest** rather than the directory (`load_all` reads only
subdirectories holding a `template.yaml`, so a bare directory is a leftover and
not a template); the app asks the same question of the cards already in memory
first, so the refusal lands on the list instead of arriving from a worker.

**On an edit the slug stops following the name.** `suggest_slug` rewrites any
slug field nobody has typed into, and a form built from a loaded template has
touched nothing — so correcting a typo in the *title* of `music-video` retyped
the slug, and Save renamed the template's directory to match. `metadata_form`
marks the field `touched` when the template already has a slug, which is what
"this value was chosen" means for one that is already on disk.

**The naming pattern is checked where it is written.** Two mistakes that
`Template::validate` cannot refuse, because both produce a template that loads
and saves perfectly and then names every project wrongly: a `{token}` no
variable answers (`{clientname}` for `client_name`, left in the folder name
verbatim), and a declared variable the pattern never uses — which is the one
that costs a first template, since the questions are asked, the answers are
recorded, and every folder name comes out identical. `studio::pattern_warning`
says which; the Metadata row wears `⚠` and the footer carries the sentence, and
the metadata form updates its own hint on the keystroke that caused it
(`sync_metadata_form`). Neither refuses a save: both are legal.

**The footer says what the highlighted row is for.** It was empty until a save
was refused, over a list of five nouns in the manifest's vocabulary. `s` saves
from the section list — the one face of the builder with nothing to type into,
which is the same bargain `a`, `d`, `K` and `J` already make on the lists
inside it (`command::builder_list_closed`).

`fastf template new` and `fastf template edit <slug>` open the app at
`Entry::Studio` — the templates tab, or the builder straight away — so the
command line and `T` are one editor.

## The panel, the guide, and where the words live

**`guide.rs` is to explanations what `command.rs` is to keys: the one place any
of them are written.** The builder's explanation panel, the seven-page guide
overlay and the coach all read it, so the sentence about what a naming pattern
is exists once and cannot drift from the sentence two columns away. It is pure —
no I/O, no clock, no `Config` — and its one dynamic input is the scratch
`Template` the builder is already holding, which is what lets `update` call into
it.

Two rules that module enforces on itself, both because the first draft broke
them:

- **A key is never spelled in its prose.** A block writes `{key:BuilderSave}`
  and `resolve` substitutes `command::key_of`.
  `every_key_placeholder_names_a_command` walks every block and proves it — and
  it is what caught the two commands this feature adds before they existed.
- **A character the theme owns a glyph for is never written into its prose.**
  The first draft drew a folder tree out of `├──` and headed its walkthrough
  steps with `·`; both render as a replacement box in the ASCII alphabet, and
  prose has no theme to ask. `no_glyph_the_theme_owns_is_written_into_the_prose`
  is the guard, and `Glyphs::is_ascii()` is how the live tree in the panel asks.
  Prose punctuation — an em dash — is not a glyph and is fine.

**The panel explains; the footer refuses and warns.** Each has one job, which is
this app's "say each thing once" applied to a box that now has two places to put
a sentence. It is also the only split that cannot lose: the footer is a fixed row
of the dialog, so a warning can never be pushed off the end of it the way it can
off the bottom of a panel that ran out of rows. `pattern_warning` is therefore in
the footer and deliberately *not* repeated in the panel — the panel shows the
sample folder name the warning is about, which is the same fact from the other
side.

**The panel never repeats what the editor beside it is already showing.** From
the section list it carries the live half — the sample name, the next two IDs,
the tree, the file list; with a section *open*, its own editor is drawing that,
so the panel is `explain_section` (prose only).

**Whether the panel is coming is settled before the dialog is sized.** A panel
wants sixteen rows where the list wants seven, and `sized_dialog`'s width does
not depend on its height — so `layout::panel_fits_width` answers the width half
first. Asking afterwards grew the box on every window too narrow to draw one.

**`studio::sample_folder_name` is pure and deliberately not "now".**
`naming::RenderContext`'s four fields are the whole of its state, so a sample
context is a struct literal — no clock, which is what lets `update` and a
snapshot test both call it. The date is a fixed 31 January: the one day where
`{YYYY}`, `{MM}` and `{DD}` are three visibly different numbers, so a reader can
tell which token produced which digits.

**The guide offers itself once, and both doors share one flag.**
`Session::guide_seen`, set in `open_guide` — so every route sets it, including
the key and the palette, and somebody who found it themselves is never offered
it. Two flags would show it twice in one afternoon to the person who looked at
the tab and then pressed `n`, which is exactly the reader it is for. It goes **on
top of** whatever asked for it, so Esc leaves you where you were going.

**The coach is advice and never a refusal.** `guide::gaps` counts what is still
worth a look and the Save row says how many; `Template::validate` and
`operations::save_template` keep every bit of the authority. Two of the gaps are
templates that load and save perfectly well.

**Every suite that drives the templates tab starts past the offer** —
`Sandbox::guide_seen()` in the pty tests, `App.guide_seen` in
`tui::testing::fixture` — for the reason `relaunch.rs` pins its terminal: a test
about the editor that has to dismiss a welcome first is a test about two things.
`tui::testing::guide_fixture` and
`flows::the_guide_offers_itself_once_and_leaves_the_editor_underneath` are the
two that meet it on purpose.

Two sections are more than a form. **Structure** is `widgets::text_area::TextArea`
— one folder path per line, with the tree they make drawn beside them and
redrawn on every keystroke; Enter is a newline there, so **Ctrl-S commits** and
the key line says so. **Files** is a path line over a text area, with the
`{tokens}` the template understands above it and the ones the text actually uses
named as they are typed — the check that catches `{clientname}` typed for a
variable called `client_name`. An empty body is a marker file (`.gitkeep`),
which the old content loop could not declare at all.

**`widgets/text_area.rs` is ours on purpose.** `tui-textarea`'s current release
pins `ratatui 0.29`: it does not merely pull a second ratatui into the tree, it
fails to resolve against ours. A widget crate has to build against the
ratatui in `Cargo.toml`; this one does not, so the piece is ours. It is
`LineEdit` with a second dimension and the same rule — the cursor is a char
index, never a byte offset, on both axes — and its viewport is a `Cell`,
because `view` takes the app by shared reference and a scroll re-derived from
scratch every frame is a scroll that jumps.

**`ListChange::SummaryOnly`** is what a template action reports: the header and
the strip change, and not one folder moved, so re-reading every base would be a
walk to answer a question none of them were asked. The landing summary also
refreshes an open studio's list, keeping its selection by slug.

## Four things the app could not do

**`v` marks from the last mark to the cursor.** `LibraryState.last_mark` is set
by Space on a mark *and* on an unmark — the anchor is "the row Space last acted
on", so changing your mind about a row does not leave the anchor on it — and it
is kept by path, so it survives a re-filter and a re-sort and is dropped when
that row leaves, exactly as a mark is. The range is taken **in view order**:
the rows between two rows are the rows a person can see between them, whatever
order they were discovered in. With no anchor the key is `Disabled` with the
sentence that says what to press first, rather than silently doing nothing —
which is what a `return` on an empty anchor would have been.

**Every one-way order runs both ways.** `Sort { order, reversed }`, because the
direction is not a second `Order` variant: every order that has one has the
*same* one, and five more variants is five more rows in a picker, five more
labels to persist and five more arms in `compare`. `newest`/`oldest` are
already the two directions of one order and `Order::reversible()` says so, so
the picker never offers "newest reversed" beside "oldest". **The tie-break does
not turn round with the order** — two rows the order cannot tell apart are
settled by date either way, and reversing that as well would shuffle every
group of equals. `s` cycles the orders the right way up; a direction is the
picker's to choose. `Sort::from_label` reads `"size reversed"` **and every
label written before there was a direction to write**, so a `state.toml` from
an earlier version still names an order.

**The tag filter writes the query the grammar already had.** `Then::TagFilter`
puts `tag:x` in the search bar rather than adding a fourth filter field beside
the template and the base: one mechanism, so clearing it is the same Esc rung
as clearing any other query and the bar goes on reporting what is filtering the
list. It is palette-only and takes no key — the search bar could always do it;
what was missing was a way to find it without knowing the grammar.

**`/` narrows the settings.** `Editing::Filter` rather than a mode of its own,
so it is the machinery the value editors already use and `Modal::context()`
needs no new answer. It is a plain case-insensitive substring over the label,
the value and the **configuration key** — the same argument the templates tab
makes: tens of rows with known names, where a fuzzy hit says yes to almost all
of them. A heading survives only if something under it did; a screen of
headings with nothing beneath them is a list that looks broken. The filter is
drawn in the **title** while it is set, because a filter that costs a row shows
you less of what you were looking for, and edited on the **footer**, which is a
fixed row nothing can push off the end and leaves the list whole underneath so
you can watch it narrow. Esc gives the whole screen back: a filter left behind
is a screen missing rows for a reason nobody can see.

## Motion, and only where it answers a question

**`src/tui/motion.rs` is pure.** No clock, no environment, no I/O: every
function takes the milliseconds it should reason about, which is what lets
`update` start a pulse and a test assert on the frame that pulse produces at a
millisecond it chose. It is the same bargain `guide.rs` makes with prose and
`layout.rs` with geometry.

**The clock is stamped on every message, not counted on the tick.** `App.ticks`
was a counter incremented by `Msg::Tick`, which made every duration a multiple
of whatever the wake interval happened to be — and the interval is not one
number any more. `App.elapsed_ms` is set by `Runtime::dispatch` before `update`
sees *any* message, because a clock that only advanced on a tick was stale the
moment nothing was moving: a status message set against a stale clock has an
expiry already in the past, which is exactly what the pty suite caught.

**A tick is due at a moment, not after a quiet interval.** `wait` kept only a
`recv_timeout`, so every message restarted it and a stream of them — a batch of
sizes, a paste, a run of `MetaLoaded` chunks — starved the tick entirely. The
spinner stopped turning exactly when there was most to wait for, which is the
one moment it exists for. `Runtime.next_tick` is the deadline; a burst is still
drained and drawn once, and the tick that came due during it is delivered after.

**`App::tick_interval` replaces `needs_tick`**, because two kinds of thing move
at two speeds: a spinner and a countdown want five frames a second, a pulse
fading wants twenty. Asking for the faster one **only while a pulse is in
flight** is what keeps the documented claim true — the app costs nothing while
idle — and `the_faster_wake_ends_with_the_pulse` holds it to that.

**The pulse is a background, and it has to be.** Every cell in a row sets its
own foreground — the id is accent, the size is dim, a tag is its own colour — so
a foreground set on the `Row` loses to all of them and shows almost nowhere.
That was the first draft, and it was invisible in a real frame while passing a
test that asked the wrong question. A background is the one thing the cells
leave alone.

**One step, not a fade.** A terminal cell has no alpha and this theme defines no
page background — `Color::Reset` has no RGB — so there is nothing to interpolate
*towards*. What a terminal can do honestly is hold the row lit for as long as an
eye needs to find it and then let go, which at 450 ms reads as a pulse rather
than a state. The status line is the one thing that really does fade, because
`DIM` is a modifier every terminal honours.

**Five things move, and nothing else.** A row a verb changed (*which* rows did
that batch touch, when the cursor is elsewhere) — in the table by path and in
the pane by row index, which is why `Pulses<K>` is generic over its key rather
than being two structs; a size cell whose number *changed* (is the figure the
one that was there a moment ago); one activity indicator wherever something is
pending (is it working, or stuck); a message on its way out (it is going, and
you can still read it); and the title of the pane the focus just moved to
(which pane will the next key go to — the border says so at rest, but a colour
changing on a line nobody was reading is not seen, and the pulse is the moment
of the move). `App::set_focus` is the one way focus moves, so every mover
stamps `focus_moved_at`; `motion::focus_style` reads it.

**A page filling in is not a change.** The size pulse fired on arrival at
first, which is every visible row at once on the first screenful and again on
every scroll — twenty rows washing together several times in the opening
seconds of a run, which reads as a fault and was reported as one. There is
nothing for it to answer either: the table is measured from the rows and never
from the sizes (`view/projects.rs`), so a landing number cannot reflow
anything. `Msg::Sizes` now pulses on a number that replaced a *different*
number, and on the one arrival that is a change rather than a first fill — a
size a verb threw away coming back, which `ListChange::Patched` records in
`App.rescanning` on its way past and the answering size spends. Deliberately **not** built: eased scrolling,
dialog transitions, cursor trails. They answer nothing, and this app's rule for
motion is the rule it already had for colour — it appears where it *means*
something and never as decoration.

**Off is a first-class state.** `theme::choose_motion` resolves it beside the
palette, from an `Env` and one config key, so `update` still reads no
environment and a setting written on the settings screen takes effect on the
frame that shows it was written — which is why `Effect::Retheme` and
`Msg::Themed` carry both. `Mono` is always off: a colour wash with no colour is
a flicker rather than a cue.

**The pane's cursor is the mono-visible focus cue.** The focused pane's border
and title are colour, and in `Theme::mono` colour is `Reset` — so before the
pane had a cursor, focus was invisible there. The cursor is drawn only while
the pane has the focus (`view/projects.rs::detail`), as the selection style,
which mono draws reversed; and the same rule keeps a lit row out of a pane you
are not in, where it would say the next key goes there when it does not.

**A snapshot cannot see any of this** — `TestBackend` records symbols, and the
snapshots render in `Theme::mono` where motion is off by rule. That is a feature
(the layout snapshots do not churn) and it is why
`testing::render_to_buffer` exists: the one place a frame's *colours* are
asserted, for the one thing `render_to_string` cannot show.

## The pane is an editor you enter on purpose

**`pane::pane_rows` is the one answer to "what is in the pane"**, read by the
view that draws it, by the cursor arithmetic in `update`, and by the scroll
ceiling. The pane was one `Paragraph` the view built as it went, with
`detail_scroll_max` hand-counting the same lines a second time; once some rows
became things you can change, which rows exist and which the cursor may rest
on (`PaneRow::selectable`) had to be one pure function. One line per row and
no `Wrap`: the cursor is an index into the rows and `detail_scroll` counts
rows, so a row that took two lines would put both off by one from there down.

**Nothing edits until Enter, and Esc leaves the row as it was.** The cursor
walks the selectable rows (`pane::step_cursor`, clamped like every list) and
Enter on one is `CommandId::PaneEdit`, which dispatches on the row: the name
is the rename prompt, a tag opens on its own line (emptied, it is removed —
`operations::replace_tag`), "add a tag" is `open_add_tag`, a text variable
opens on its line, a `select` variable opens `Modal::Pick` over its options
with `Then::PaneVariable` — the picker is the one shape that cannot hold a
value outside the options — the notes rule opens a `TextArea` over the section
(`Ctrl-S` saves; `PaneEditConfirm` is *hidden* there so Enter reaches the
widget as a new line), the journal rule is `NoteInline`. The edit lives in
`App.pane_edit` beside the rows rather than in a dialog over them, so what is
being changed stays in view with everything around it.

**`Context::PaneEdit` is a text-entry context**, so the field has first refusal
on every printable key and the registry answers Enter, Esc and `Ctrl-S`
(`on_pane_edit_key`, the `on_text_prompt_key` shape). It is why Enter on the
list had to become its own id: `Actions` carried `[a, Enter]` over both the
list and the pane, one id cannot bind different keys in different contexts,
and the pane's Enter now means *edit*. `ActionsEnter` is Enter over
`[Projects]` with the same handler, hidden from the bar and the palette; `a`
still opens the menu from the pane.

**An edit stays open, pending, until the worker answers.** `send_pane_edit`
marks it and `on_action_done` finishes it: an `Ok` closes the edit and the row
pulses; an `Err` lands on it (`PaneEdit::fail`) with the text still there to
correct — the builder's `saving` and a settings row's edit already worked this
way, and a refusal in a dialog over a field you can no longer see is worse
than none. Moving the focus or the selection drops an open edit untouched.

**The cursor follows the thing, not its index.** A landed edit returns
`ListChange::Patched`, which patches the row and drops the cached detail, so
the pane's rows are rebuilt — with a tag more or less above the variable that
changed, and with the variables gone until the re-read lands. `PaneEdit::target`
says what the edit was about (`PaneTarget`), `settle_pane_cursor` finds that
row after `apply_change`, and `App.pane_return` keeps the target so
`Msg::Detail` finds it again once the detail is back. Keeping the old index
put the cursor one row off the moment a tag arrived.

**What the pane admits is what the file can hold**, and the rule lives in
`core`, once: `validated::Tag` at `operations::add_tags`, `vars::rendered_values`
inside `set_variable`, the `##` refusal in `set_notes`. `validators::tag` is
the same rule for the prompts that want to refuse under the line before a
worker is asked. See `src/core/CLAUDE.md`, "The pane's edits".

## Settings, the counter, maintenance, the first run

`,` opens `Modal::Settings`: every setting fastf has on one screen, grouped by
heading, with what it is set to beside it. The menu this replaces was seven
submenus deep, so seeing what fastf was configured to do meant walking the tree
and remembering.

**A row's key is the configuration key.** `cli::config::apply` — the print-free
half `fastf config set` now calls too — performs every write on a worker, so a
refusal here is the refusal the command line has always made, in the same words,
and there is no second validator to drift. `app/settings.rs` builds the rows and
knows nothing about what is legal. A yes/no and a two-way choice are written
where they stand: opening a dialog to answer a question with two answers spends
a keystroke on nothing. Everything else opens **on its own line**, pre-filled,
with the refusal under it and the text still there.

The **library bases** are one `TextArea` — one folder per line, Ctrl-S to keep —
because that is what the list is. The old menu added and removed them one prompt
at a time and could not show you the set you were building.

The **ID counter** and the three **maintenance** verbs (reindex, check and
recover, data locations) are rows on the same screen; `!` is
`CommandId::Reconcile` from anywhere, which is what the header's `⚠ n needs
attention` is about.

**`ActionOutcome::settings()`** asks the screen to re-read itself after a write.
It shows what is on disk, not what was typed, so a value the config normalised
(`~/Projects` → an absolute path) shows as it was stored.

**The first run is a dialog.** `tui::run` loads the `Config` before the screen —
which is also what makes a corrupt one stop the app where the error can be read
— and hands `runtime::run` the folder to suggest when no base is configured
anywhere. `App::request_onboarding` puts the question up before the first frame,
so the app never opens on an empty dashboard with no explanation. It stays up
until the folder exists: a path that cannot be created is refused with the text
still on the line.

**`run_action` refuses while one is already running.** The runtime answers with
the `ActionId` it was given and `on_action_done` drops anything that is not the
one in flight, so a second action started over the first would make the first's
outcome vanish — the row unpatched, the message never shown. The command
registry's `not_busy` guards the keys; this guards the screens whose rows are
not commands.

## Every flow is native, and dialoguer is gone

There is no suspend bridge left and no `LegacyFlow`. `Suspended` has two
variants, and both exist because the *terminal* is needed, not because a flow
was not rewritten: `Note` (the `$EDITOR` journal flow) and `PostCreate` (`git
init`, the editor, a template's own commands).

**Two modules take the terminal, and `tests/layering.rs` says so.**
`runtime.rs` owns the alternate screen for the guided app; `inline.rs` owns a
few rows at the cursor for a command-line prompt. A third owner is two
unsynchronised writers on one tty, which is how a frame comes back with
somebody else's line in the middle of it.

`prompt.rs` is now the *contract* — the `require_tty` guard, and `Ok(None)`
meaning cancelled — over `inline.rs`, which does the drawing. `pickers.rs` and
`vars.rs` sit on top and serve the command line: the ambiguity picker
`open`/`copy`/`path`/`term` share, and the variable prompts a scripted `fastf
new` falls back to.

**The cursor position is never queried.** ratatui's `Viewport::Inline` is the
obvious way to draw an inline prompt and the wrong one: it asks the terminal
where the cursor is (`ESC [ 6 n`) and waits up to two seconds for an answer. A
pty under test never sends one — the suite failed on it the first time — and
neither does every real terminal. It is the same trap that already cost this
codebase `Terminal::clear`, and it would be worse here, because a stall in front
of `fastf copy` is a stall in front of the command that exists to be instant.
So `inline` reserves its rows by printing newlines, and every repaint is *move
up n, draw*. Colour is written as SGR from the theme's own `Style`
(`inline::paint_span`), so `NO_COLOR` and the ANSI palette work exactly as they
do in the app.

The picker is deliberately **not filterable**. It is the picker a verb
interrupted — `fastf copy lullaby` matching three projects — and its job is to
be answered in one or two keystrokes over a list the query already narrowed.
Fuzzy search lives in the app, where there is a library to search. Esc *and* `q`
cancel, as they always did.

Every prompt leaves **one line of transcript**: the question and its answer, or
the question and `cancelled`. A prompt that vanishes makes a run's history read
as though it was never asked.

`tui::pickers` holds all three pickers; `pick_project` is the **ambiguity**
picker for `open`/`copy`/`path`/`term` and deliberately not the app.

The session ring (`frame.rs`) is a `Mutex<Vec<String>>`, three entries, per
process; the header reads it after every action. Anything durable belongs in the
project's journal.

## Testing the app

Three layers, each for what only it can see:

- `tests/tui_update.rs` — the state machine, no terminal. Build with
  `tui::testing::fixture`, send `Msg`s, assert on the `Effect`s.
- `tests/tui_snapshots.rs` — the frames. `Theme::mono`, Unicode glyphs, fixed
  dates, `/mnt/projects/…` paths (the hygiene test forbids real ones),
  `insta` snapshots under `tests/snapshots/`. A deliberate change is reviewed
  with `INSTA_UPDATE=always` and committed.
- `tests/tui_pty/` — the runtime through a real 120×40 pty. **ratatui redraws
  only the cells that changed**, so the raw transcript is fragments: `1 of 1
  projects` never appears contiguously, and a word can arrive one letter at a
  time. `harness::app_screen` replays the transcript (up to the last
  `LeaveAlternateScreen`) into a `vt100` terminal and returns the frame a person
  saw; `pty::plain` (escapes stripped) is for what a suspended flow or an
  inline prompt printed in cooked mode. Match on the screen, never on the
  stream.

## What the consolidation pass added

Everything below landed in one pass after the eight phases, from three audits
and forty frames of the screenshot tool.

**The theme is a pure function of an `Env`** (`theme::choose`): `FASTF_THEME`,
then `NO_COLOR`/`TERM=dumb`, then the config's `theme` key, then what the
terminal announces — `COLORTERM`, a `TERM`/`TERM_PROGRAM` naming a truecolor
emulator, Windows Terminal — else ANSI. A theme written on the settings
screen takes effect on the frame that shows it was written
(`Effect::Retheme` → `Msg::Themed`); `update` still reads no environment. The
Windows ASCII heuristic is "a host that announces no emulator", and
`FASTF_ASCII=0` forces Unicode.

**Session memory** (`session.rs`): `state.toml` beside `config.toml` keeps the
sort order, the pane and the id of the row the cursor was on; read before the
first frame, written after the screen is given back, applied once on the
first discovery — a reload is not a restart. `fastf recent`/`search` own
their order and take only the pane's state.

**Every verb but rename batches** (`jobs::JobKind` carries the answer — the
tag, the note, the base — asked once); delete asks for the word `delete`,
single or batch, and the prompt names every folder; the quick note is a text
area (Enter saves, Alt-Enter breaks a line).

**A batch item's effects are the app's.** `on_job_item_done` returns what
`apply_change` gave it, alongside the next item's `Run`. It used to drop them,
and `App::discover` sets `library.inflight` *before* returning the effect that
answers it — so one dropped `ListChange::Reload` left the app waiting on a
generation nothing would ever send, after which every `patch`/`remove` only set
`dirty` and **the list stopped changing for the rest of the session**. A batch
re-derive of tags rewrote every file and showed nothing at all. The test
helpers in `tests/tui_update.rs` look for the one `Effect::Run` among the
effects rather than requiring it to stand alone, which is the shape this fix
makes normal.

**A mark is the retry list.** An item that succeeded loses its mark when its
outcome lands (`take_inflight` hands the project back for its path);
`LibraryState::patch` only ever dropped one when the path moved, so a clean
batch reported "3 tagged" over three rows still wearing `✓`.

**`command::batch_target` is what a batching verb is available on.** Marks are
kept by path and survive a filter change, so a marked row can be off screen
while the verb is aimed at it — `targets()` intersects the two and comes back
empty, and every batch verb hit an early `return Vec::new()` with no picker, no
dialog and no message. That is what "batch tagging does nothing" was. It is
deliberately **not** part of `needs_selection`: `o`, `t` and `y` act on the row
under the cursor and are none of a hidden mark's business.

**A move says which kind it was.** `MoveOutcome::staged` and `copied` reach
both surfaces: `renamed on the same filesystem, nothing copied`, or `copied 412
files, 199.5 GB, verified`. A same-filesystem rename is instant however large
the folder is, and a message naming only the destination reads the same either
way. `JobStatus` is set to `Done` at the end of both paths — it was assigned
`Running` at construction and never changed anywhere in the crate, so the
runtime's "is it done yet" was always false, `Runtime.moving` was never
cleared, and a later `Effect::CancelMove` set the flag on a dead job's handle.

**The bar is ours** (`view::modals::bar`), drawn from `Glyphs::bar_full` /
`bar_empty`, not ratatui's `Gauge`: the palette is a pure function of an `Env`
and the ASCII path has to stay right on a terminal that draws no block
elements. A `total` of zero draws an empty track — nothing measured is not
everything done. Both progress dialogs are sized to the lines they hold.

**Geometry lives in `layout.rs`**, read by `update` and `view` alike, so a
cursor can never leave the drawn window and End lands on the last line:
`actions_box`, `pick_box`, `help_box`, `message_box`, `sized_dialog`,
`settings_rows`, `studio_rows`.

**Three of that module's functions exist because the obvious spelling panics or
wraps**, and none of them may be written out by hand again:

- `fit_between(wanted, min, max)` — `Ord::clamp` **asserts `min <= max`**, and
  every `max` in a terminal is computed from a window somebody can drag. The
  settings screen's Bases editor was `clamp(4, body.height - row)` with `row`
  walking down the body, so Enter on that row in any window 16–23 rows tall took
  the whole app down with `min > max`. It survived 80×24 by exactly one row,
  which is why the documented manual pass at that size never found it. The room
  wins over the wanted minimum: `max` is a hard limit, `min` only a preference.
- `percent_of(whole, share)` — `area.width * 76 / 100` overflows a `u16` above
  862 columns. Release has no overflow checks, so it wrapped: a 900-column
  terminal drew a 46-column create dialog. Debug panics instead. Neither is a
  size.
- `box_at_row(body, row, wanted, min)` — an editor that opens over its row
  slides up when there is no room below, rather than choosing between panicking
  and drawing a sliver.

**A dialog is measured at the width it will be drawn at.** The confirm and the
typed prompt asked `wrapped_rows` about a hardcoded 62 or 64 and then let
`centered_fixed` clamp the box to the screen, so at 60 columns the text wrapped
wider than had been reserved and the tail was cut — and a flat eight-row ceiling
cut it again. `view::modals::question_size` is the one answer, and the screen is
the only ceiling: a destructive confirmation that hides part of what it is about
is the one that must not.

**A message's scroll counts wrapped rows, not entries.** The journal and
metadata views are drawn with `Wrap`; `lines.len()` as the limit meant every
note longer than one line counted once and drew twice, so the end of a long
journal was unreachable. `Modal::Help` had always counted them properly
(`command::help_line_count`); `view::modals::message_rows` is the same sum for
the other one. The flow preview clamps in `update` too now
(`view::modals::preview_max_scroll`, from `flow_rect`, the geometry `view` draws
with) — clamping only at draw time let `scroll` run to 200 over a twelve-line
preview and then take twenty PgUps to come back, reading as a frozen dialog. The table/pane split favours the table
(`regions` takes the width the names need with the size beside them; the pane
takes the rest and closes under `DETAIL_PANE_MIN`).

**The message log** (`App.log`, `L`): every status line and `diag` warning,
stamped by `App.clock` — the wall clock in the runtime, a fixed string in a
fixture — and a count of the warnings that arrived under a dialog.

**A malformed query is named while it is typed** (`core::query::diagnose`,
additive; `parse` is unchanged for the command line).

**Honest counts.** The header counts templates on disk (`TemplateCard::on_disk`);
the strip lists orphan slugs dimmed after them and never opens on one.

**Each fact is stated once.** The project count lived in the header, in the
search bar and in the status line, in three formats, which reads as three
different facts; `?` was advertised by the hint bar *and* by a hand-written
sentence on the status line. The search bar is now the one place the list
reports itself — the counts, the `(from index)` spinner, the sort, the template
and base filters, the mark count — the hint bar is the one place a key is
advertised, and the status line is left for what neither can show: what a batch
verb would act on right now. `MarkToggle` is `hint = true, palette = true` for
the same reason: the sentence that used to advertise Space was exactly the
drift the one registry exists to prevent.
