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

A finished verb patches its row by **id** (`ListChange::Patched`) because a
rename or a move changed the path; the pty suite traces that the list is not
rescanned. A typed confirmation that does not match keeps the text in the
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

## The template studio and the builder

`T` opens `Modal::Studio`: every template on disk with the selected one's
details beside it, read on a worker (`loaders::template_view`, which renders
`cli::template::describe` — the same lines `template show` prints, so the two
cannot drift). Its verbs are `n`, Enter, `g` and `D`.

**The builder is a list of a template's five parts, not a sequence of steps.**
The old one walked six steps and *then* offered a review menu to go back into
any of them, which is to say the review menu was the interface and the walk was
a tax. `app/studio.rs` holds the scratch `Template` and the section the list has
open; every row says what that part currently holds, so the list is the summary
the old builder printed after each step. Nothing is written until Save, and Save
says `Cannot save:` with `Template::validate`'s own words rather than writing
something that will not load.

`fastf template new` and `fastf template edit <slug>` open the app at
`Entry::Studio`, so the command line and `T` are one editor.

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
`settings_rows`, `studio_rows`. The table/pane split favours the table
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
