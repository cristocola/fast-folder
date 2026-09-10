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
and "template from a folder" in the studio. The keys a text widget consumes
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
the two cannot drift). Its verbs are `n`, Enter, `g` and `D`.

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
