# CLAUDE.md — `src/tui/`

Every interactive terminal surface. The guided app is how the tool is used day
to day, so its polish is the product's polish: the list draws before a single
folder has been walked, cancel is always possible, typed input is never thrown
away by a later validation failure, and a network-share stall is never a frozen
screen.

The root `CLAUDE.md` has the layering rule and the module list; `src/core/CLAUDE.md`
has the engine underneath. `PLAN.md` is the phase-by-phase record of the
ratatui rebuild; what follows is the design as it stands.

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
- Every state is visible and quiet: loading (`(from index)` and a spinner),
  empty (one sentence saying what to do), an error (one line, or a dialog when
  it has more to say), disabled (dimmed, with the reason on the key).
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
palette (`palette_entries`), the help overlay (`help_sections`) and the hint bar
(`hints`) all read that list; the prototype carried four copies of its key
table and they had already drifted. `tests/tui_commands.rs` holds the
invariants: one key means one thing per context (global bindings count
everywhere), every id is declared exactly once, every bound command is in its
context's help.

An `Availability` is a function of the app: `Disabled(reason)` is listed dimmed
and pressing its key shows the reason; `Hidden` is not bound at all (Move with
no other mounted base, Clear-filter with no filter).

Keys are normalised into `Key` (`Char` with the Ctrl and Alt flags; shift
folded into the character; Ctrl-letters lower-cased) by the input thread, which
also drops `KeyEventKind::Release` — Windows delivers one for every press.

**Naming trap.** Until the CLI's prompts move off dialoguer, `tests/layering.rs`
greps `src/tui` for `Input::`, `Confirm::`, `Select::`, `Sort::` and
`MultiSelect::` outside `prompt.rs`. A type called any of those in the app trips
it, and so does a type whose name *ends* in one — `LineInput::new()` contains
`Input::`. Hence `LineEdit`, `Order`, `PickState`. `tests/tui_commands.rs` says
so before CI does.

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
`rows::SIZE_CELL` wide and right-aligned.

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

Since Phase 1 the verbs on a selected project are native modals, not bridges.
`app/actions.rs` holds their states (`ActionsState`, `TextPrompt`, `Confirm`,
`MultiPick`); `command.rs` binds `Enter` and `a` to the action menu, `A` /
`Ctrl-T` to add / remove tags, `N` / `Ctrl-N` to the editor and inline notes,
`r m u D` to rename / move / unregister / delete, and `M` / `J` to the
read-only metadata and journal views. The action menu's rows come from the one
registry, ordered by display (`action_entries`); an entry that cannot run right
now is listed dimmed with the reason on the key, not hidden — pressing it says
why. Prompt texts and validators live in `validators.rs`, byte-identical to
the dialoguer flows they replaced.

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
the app's Esc ladder, one step at a time. (The dialoguer flows cancelled
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
fails to resolve against ours. That is the plan's own condition for a widget
crate. It is `LineEdit` with a second dimension and the same rule — the cursor
is a char index, never a byte offset, on both axes.

**`ListChange::SummaryOnly`** is what a template action reports: the header and
the strip change, and not one folder moved, so re-reading every base would be a
walk to answer a question none of them were asked. The landing summary also
refreshes an open studio's list, keeping its selection by slug.

## The bridged flow

Settings is still the dialoguer flow in `menu.rs`, with `pickers.rs` and
`vars.rs` behind it, reached through `Effect::Suspend(Suspended::Legacy(..))`:
the input thread is parked (a `Condvar` handshake — two readers on one tty is
how keys go missing), the screen is released, the flow runs on the main screen
in cooked mode, a flow that prints a result waits for `press Enter to return to
fastf…`, and the screen is taken again. A recoverable error is reported the way
`menu::contain` reports it; `menu::is_fatal` (the prompt itself failing, an
interrupt) ends the app. Each phase of `PLAN.md` makes flows native and deletes
their bridge variant.

While it exists, the rules below still hold for it.

**Every prompt goes through `tui::prompt`**, and `tests/layering.rs` fails the
build if any other module under `src/tui` or `src/cli` names a dialoguer prompt
type. `Ok(None)` is a cancelled prompt and is never an error. `select`,
`multi_select` and `sort` reuse dialoguer's `interact_opt` (Esc or `q` cancels
in one keystroke); `confirm` and `text` are hand-rolled — `text` is the same
line editor as `widgets::input::LineEdit` (char-index cursor, windowed not
wrapped) and parks the terminal's caret in the line it is editing, which
`a_text_prompt_parks_a_visible_caret_after_the_text` pins. `TextOpts` has both
`initial` (editable starting text) and `default_value` (`prompt [default]:`);
they are different gestures.

**Menus match on labels, not indices.** Vocabulary: **Back** to a parent menu,
**Cancel** to abandon an action. **A value with a local validity rule is
checked at the prompt that collected it**, and dependent questions come after
the value they depend on. `tui::pickers` holds all three pickers; `pick_project`
is the **ambiguity** picker for `open`/`copy`/`path`/`term` and deliberately not
the app.

The session ring (`frame.rs`) is a `Mutex<Vec<String>>`, three entries, per
process; the header reads it after every bridged flow. Anything durable belongs
in the project's journal.

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
  saw; `pty::plain` (escapes stripped) is for what a bridged flow printed in
  cooked mode. Match on the screen, never on the stream.
