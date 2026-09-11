# CLAUDE.md — `src/tui/`

Every interactive terminal surface. The guided app is how the tool is used day
to day, so its polish is the product's polish: the list draws before a single
folder has been walked, cancel is always possible, typed input is never thrown
away by a later validation failure, and a network-share stall is never a frozen
screen. The root `CLAUDE.md` has the layering rule and the module list,
`src/core/CLAUDE.md` the engine, `tests/CLAUDE.md` the suites.

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

**The theme is a pure function of an `Env`** (`theme::choose`): `FASTF_THEME`,
then `NO_COLOR`/`TERM=dumb`, then the config's `theme`, then what the terminal
announces (`COLORTERM`, a truecolor `TERM`/`TERM_PROGRAM`, Windows Terminal),
else ANSI. On Windows a host that announces no emulator gets the ASCII alphabet;
`FASTF_ASCII=0` forces Unicode. A theme set on the settings screen lands through
`Effect::Retheme` → `Msg::Themed`, so `update` reads no environment.

**Glyphs come from `Glyphs`** wherever a theme is in reach, so the ASCII alphabet
holds (the ROADMAP lists four screens that still spell characters out). Success
wears the theme's tick through `App::good`, because `runtime::run_action` runs on
a worker with no theme; the spinner is `Glyphs::spin(elapsed_ms)`; the progress
bar is `view::modals::bar`, drawn from `Glyphs::bar_full`/`bar_empty` rather than
ratatui's `Gauge`, and a `total` of zero draws an empty track.

Look at every screen you build with the screenshot tool
(`tests/tui_pty/screenshot.rs`) before you write its snapshot.

# The guided app

`tui::run(Entry)` is the door: `fastf` (`Entry::Menu`), `fastf recent`
(`Entry::Recent`, its flags as a `Preset` chip) and `fastf search`
(`Entry::Search`). `require_tty` runs **before** the screen is taken, so a
terminal nobody is holding is never switched to the alternate screen.

## The shape: model, message, effect, view

`app::update(&mut App, Msg) -> Vec<Effect>` is the one state transition and
**performs no I/O**: everything it wants done is an `Effect` (`effect.rs`) that
`runtime.rs` carries out, and `view::view(&App, &mut Frame)` takes the app by
shared reference. So `tests/tui_update/` drives the state machine with no
terminal, `tests/tui_snapshots.rs` renders any state a test can build, and a slow
filesystem can never reach the key handler.

The viewport is the app's: `LibraryState.offset` is kept by `clamp_viewport`
(every list stops at its ends and the window moves by the minimum —
`widgets::nav`), and each frame builds a `TableState` from it, since one kept in
the view would need `&mut App`.

## The runtime owns the screen

`runtime.rs` takes raw mode, the alternate screen and bracketed paste **on
stderr**, so `fastf > log` still opens the app and stdout keeps choosing output
format. `Runtime::init` calls `tty::mark_interactive_surface` and installs a
screen-restoring panic hook for the **main thread only**; `spawn_worker` turns a
worker's panic into a warning rather than tearing down a live session.

**Two modules take the terminal** (`tests/layering.rs`): `runtime.rs` the
alternate screen, `inline.rs` a few rows at the cursor for a command-line prompt.
A third owner would be two unsynchronised writers on one tty. `Suspended` has two
variants, both because the *terminal* is needed: `Note` (the `$EDITOR` flow) and
`PostCreate` (`git init`, the editor, a template's commands).

**Never ask the terminal where its cursor is** (`ESC [ 6 n`): it waits up to two
seconds for an answer a pty under test, and some real terminals, never send. So
never call `Terminal::clear` — a fresh `Terminal` and a fresh alternate screen are
already blank — and never use `Viewport::Inline`. `inline` reserves its rows by
printing newlines and repaints as *move up n, draw*, colouring with SGR from the
theme's own `Style` (`inline::paint_span`), so `NO_COLOR` and ANSI behave as in the
app.

**The terminal is always given back.** A second external signal (`kill -INT`
twice, SIGHUP) exits from the handler, where nothing of ratatui may run:
`restore_on_signal`, registered with `interrupt::set_restore`, writes the undo
escapes with raw system calls and restores the terminal settings
`tty::remember_cooked_mode` captured before raw mode. `inline.rs` registers its
own, and its own panic hook. Ctrl-Z is `Suspend`: the input thread pauses, the
screen is released, `SIGTSTP` is raised, and `fg` retakes the screen and sends a
`Resize`, because the window may have changed.

**Pasted text goes into a field, never to the keys.** A run of printable keys
faster than a hand types becomes one `Msg::Paste` (`collect_burst`), for
terminals without bracketed paste; `on_paste` gives a line field the first line
(saying how many it dropped), a text area every line, and the dashboard only a
status line.

**The app never takes the mouse**, because a terminal that reports the mouse takes
every drag too, and selecting text would need a modifier nobody finds.
`take_screen` enables no mouse mode; with none requested, kitty, Konsole, VTE,
WezTerm, Alacritty and Windows Terminal send the wheel on the alternate screen as
arrow keys, which the app already answers. No tracking mode reports only the
wheel, and `DECSET 1007` is not written — no new escape writes for one terminal.
`mouse` is a retired key, and `tests/tui_pty/app.rs` asserts that no tracking mode
is ever enabled, whatever an older `config.toml` says.

**The loop.** The input thread polls at 50 ms for two seconds after a key, then
once a second. The main loop blocks in `recv_timeout`, drains a burst and draws
once. **A tick is due at a moment** (`Runtime.next_tick`), so a stream of messages
cannot starve the spinner exactly when there is most to wait for.
`App::tick_interval` says how soon — five a second for a spinner or countdown,
twenty only while a fade can be seen — and `App::needs_tick` whether at all; with
nothing moving the wake is `IDLE_WAKE`, a look at `interrupt::is_set` and
`Runtime::watch_detail`. Each tick diffs `SizeScanner::cells_for` and hands the
app only the news. `the_faster_wake_ends_with_the_pulse` holds the promise that
idle costs nothing.

**The clock is stamped on every message**: `Runtime::dispatch` sets
`App.elapsed_ms` before `update` sees any message, because a clock advanced only
by ticks is stale whenever nothing moves, and a message stamped with it would
expire in the past.

**A thread that dies says so.** `spawn_worker` catches panics, and the input
thread and the detail reader — loops that outlive their requests — report in their
own shapes, because the runtime holds its own `Sender` and a dead input thread
would otherwise leave a live-looking frame that answers nothing. `InputEnd`
reports "the terminal stopped answering", never "stop() was called". The input
thread starts **before** the screen is taken, so a spawn failure cannot strand
`SCREEN_OWNED` and an error on an alternate screen.

**Where work runs.** Discovery, the header's summary, on-demand metadata and every
`operations::*` call run on `spawn_worker` threads with `WORKER_STACK` (walk depth
is in `src/core/CLAUDE.md`). The detail pane has one latest-wins worker, the
debounce for a held arrow. Reveal, terminal and clipboard spawns run on workers
too — `reveal_folder` waits on `.status()` and `wl-copy` can hang. The size
scanner's `request`/`forget` only take a mutex, so they run inline.

**Ctrl-C is a key**, not SIGINT, in raw mode: it closes a dialog, else quits with
`Exit::Interrupted`, and `tui::run` then calls `interrupt::raise()` so `main`
prints `aborted.` and exits 130 as a signal would. An external SIGINT is seen on
the idle wake. **`diag` goes through the channel** as `Msg::Diag`, because a
worker's `eprintln!` would land mid-frame on the alternate screen.

**`session.rs`** keeps five things in `state.toml` beside `config.toml`: the sort,
whether the pane is open, the selected row's id, `guide_seen` and `explain_open`.
It is read before the first frame, written after the screen is given back, and
applied once on the first discovery — a reload is not a restart. `fastf recent`
and `search` own their order and take only the pane's state.

**`run_action` refuses while one is already running**, and `on_action_done` drops
an answer whose `ActionId` is not the one in flight, or a second action would
erase the first's outcome. The registry's `not_busy` guards the keys; this guards
the screens whose rows are not commands.

## The command line's own prompts

`prompt.rs` is the contract — the `require_tty` guard, `Ok(None)` for cancelled —
over `inline.rs`'s drawing. `pickers::pick_project` is the ambiguity picker
`open`/`copy`/`path`/`term` share, never the app; `vars` holds the variable
prompts a scripted `fastf new` falls back to. The picker is **not filterable**:
the query already narrowed the list, and it should be answered in a keystroke or
two; Esc and `q` cancel. Every prompt leaves **one line of transcript** — the
question and its answer, or `cancelled` — so a run's history shows what was asked.
The session ring (`frame.rs`) holds three entries per process for the header;
anything durable belongs in the journal.

## Layout

**Geometry lives in `layout.rs`**, read by `update` and `view` alike, so a cursor
never leaves the drawn window: `actions_box`, `pick_box`, `help_box`,
`message_box`, `sized_dialog`, `settings_rows`, `templates_panes`,
`panel_fits_width`. Three helpers replace spellings that panic or wrap, and may not
be written out by hand again:

- `fit_between(wanted, min, max)` — `Ord::clamp` asserts `min <= max`, and a `max`
  computed from a window somebody can drag may be the smaller; the room wins.
- `percent_of(whole, share)` — `width * 76 / 100` overflows a `u16` above 862
  columns: a panic in debug, a wrap in release.
- `box_at_row(body, row, wanted, min)` — an editor opening over its row slides up
  when there is no room below.

**A dialog is measured at the width it is drawn at**
(`view::modals::question_size`), with the screen as the only ceiling, because a
destructive confirmation must never hide part of what it is about.

**A scroll counts drawn rows, and `update` clamps it**: `view::modals::message_rows`
for the wrapped views (as `command::help_line_count` does for help) and
`view::modals::preview_max_scroll`, from `flow_rect`, for the flow preview.
Clamped only at draw time, a scroll runs past the end and the dialog seems frozen.

`layout::panel_fits_width` settles whether a panel is coming before a dialog is
sized, because the panel changes the height the dialog wants. The table/pane split
favours the table (`regions`), and the pane closes under `DETAIL_PANE_MIN`. The
templates tab splits through `layout::templates_panes`, so `studio_scroll_max` and
the view measure one box.

## One registry

**Every command is declared once, in `command.rs`** — title, description, contexts,
default keys, category, palette and hint visibility — and the keymap (`lookup`),
the palette (`palette_entries`), help (`help_lines`), the hint bar (`hints`) and
every dialog's key line read it, so none can drift. `tests/tui_commands.rs` holds
the invariants: one key means one thing per context (global bindings count
everywhere), every id is declared once, every bound command is in its context's
help.

Dialogs are contexts too, and their handlers end in `lookup_and_run`: a key the
dialog does not consume is whatever the registry binds there, which is how `?` and
a verb's own letter work inside the action menu. `Close` (Esc, `q`) is one command
for every dialog, one level at a time; `Quit` and `Back` are the dashboard's. A
text widget's own keys (Ctrl-S in a text area, Tab in a form) are the one
exception, and its key line names them.

**A sentence that names a key reads it from here** (`command::key_of`,
`command::NO_TEMPLATES`), so no prompt or empty state outlives a rebinding. A verb
with no key of its own shows `Enter` in the action menu, since Enter runs the row.

An `Availability` is a function of the app: `Disabled(reason)` is listed dimmed
and its key shows the reason; `Hidden` is not bound at all (Move with no other
base, Clear-filter with no filter). **A command may be palette-only**
(`palette = true` with no keys — `BackToLibrary`, `ReautoTags`, the tag filter),
so the help never lists a key that is not there.

**Keys named nowhere else in this file**: `R` reindexes from anywhere (`Reindex`);
`C` copies to a folder outside the bases (`CopyTo`) and `p` shows the full path
(`ShowPath`), both action-menu verbs; `i` toggles the detail pane on the lists
(`ToggleDetail`) and the explanation panel in the builder (`BuilderExplain`); `*`
marks every row in view (`MarkAll`) and `-` clears the marks (`MarkNone`).

**Each fact is stated once**, because one count in three formats reads as three
facts: the search bar reports the list (counts, the `(from index)` spinner, sort,
filters, marks), the hint bar advertises keys, and the status line says what a
batch verb would act on. `MarkToggle` is `hint = true, palette = true` for that
reason. The hint bar orders by category — this context's verbs first, then `?
help` and `c commands`, which are the same everywhere and what a narrow window can
afford to lose.

## Every key line is read, and the field goes first

**No key line spells an arrow by hand.** `command::movement_pair(ctx)` reads the
arrow labels from whatever binds them in that context and carries the surface's
verb (*choose* an action, *move* through picks, *scroll* text). It is prepended
only where `Context::hints_movement()` says — the dialogs, and the search bar,
whose arrows move the list under the text — never on the dashboard, where a
highlighted row and a scrollbar already say it.
`tests/layering.rs::no_key_line_is_written_by_hand` scans the source: nothing
under `src/tui/view/` writes an arrow as a key, and `view/dashboard.rs` writes no
key label; a widget's in-frame key line keeps its literals.

**The hint bar spells nothing.** `Context::Prompt` (a one-line prompt, a quick
note, the first-run question) and `Context::Pick` exist so those keys are
declared, and `Availability` hides the rest (`PromptNewline` outside a note,
`PickToggle` outside a multi-pick). **A key line is cut at a whole pair, the way
out before the extras** (`view::builder::key_line`), because half an entry
advertises a key that is not one.

**In a text-entry context everything printable is text**, and only a key a field
cannot hold reaches the registry. The caret's chords are `LineEdit::CLAIMED`, and
`command::keys_in(ctx, command)` takes them back out of what the context
advertises, so a rename prompt shows `F1 help`, never `? help`. `Ctrl-p` opens the
palette from the search bar while `c` types a `c`; inside the palette `Ctrl-p` is
the previous entry, because the opener is declared over every context except
`Palette`. Esc leaves every context (`every_context_has_help_and_a_way_out` checks
the key, not the id), and the search bar is a rung of its ladder: an empty bar
closes, a non-empty one clears and stays. The search bar is on `SCROLLERS`; its
`Ctrl-u` is the field's kill-to-start, so `keys_in` keeps `HalfUp` off its help and
`ClearSearch` needs no key. `on_search_key` is the field, the registry, then the
field again.

The palette's `#` prefix is shown on its own blank row while the query is empty —
never on the hint bar, where every pair is a command. Keys are normalised into
`Key` by the input thread (Ctrl and Alt flags, shift folded in, Ctrl-letters
lower-cased), which drops `KeyEventKind::Release` — Windows sends one per press.

## The movement grammar, and the horizontal axis

**One movement grammar over every list.** The arrows, pages, halves and ends are
declared over one context list, `SCROLLERS`, because narrower lists drift apart;
`every_list_binds_the_whole_movement_grammar` requires a context that binds one of
the eight to bind all eight. **An arrow and its vim letter are one key**
(`an_arrow_and_its_vim_letter_are_bound_together`), except in a text-entry
context: the palette binds `↓` and cannot bind `j`. **`g`/`G` are the first and
last row everywhere**; the templates tab's verbs are `I` (from a folder) and `H`
(the guide — shorter than `Ctrl-g` in a fixed-width hint bar).

**The horizontal axis is focus, and never runs anything**: `→`/`l` into the pane,
`←`/`h` back to the list, because an arrow that executes cannot be leaned on to
look around, and Enter in the pane means *edit this row*. `FocusList` and
`FocusDetail` are declared over `PANED` and dispatch on `screen`; each is
**hidden, not a no-op**, where it has nowhere to go (`pane_has_focus`,
`pane_can_take_focus`). The axis never quits — leaving a tab is Esc's ladder and
`T` — and `the_horizontal_axis_only_moves_focus_or_turns_a_page` holds it. Two
surfaces own their left and right: a **field** (`LineEdit`, `TextArea`, a `Form`
choice) for its caret or option, and the guide, a **reader**, whose
`Context::Guide` declares `GuideNext`/`GuidePrevious`; forward off the last page
leaves the guide.

**Ctrl-C is a command** (`CommandId::Interrupt`) so it is in the help, but
`on_key` dispatches it directly: it must work inside a text field and under a
modal that consumes every key, and no availability may swallow it.

## Modals and the palette

`ModalStack`: Esc pops one; a picker's answer is data (`Then`), not a closure, so
`update` stays inspectable. The palette ranks a title hit above a description hit,
Enter dispatches exactly the `CommandId` a key would, and `#`/`@` restricts it to
projects.

**Every quit goes through `App::quit`** — `q`, the palette entry, the too-small
guard — so each asks what Esc would ask before a worked-on template is lost.
`close_top` owns the question, `quit` who must ask it, and
`ConfirmThen::DiscardTemplate` carries `then_quit` so the answer finishes the quit.

**A dialog carries its target and never re-reads the selection at submit.**
`TextThen::Rename`/`Delete` and `ConfirmThen::Unregister` hold the path, because a
discovery underneath can move the cursor; `App::project_at` resolves it in the
current snapshot, and a target that is gone is a refusal, never a neighbour.

**A worker's answer names its question.** `Builder::pending` carries the slug a
template read was for, as `on_template_loaded` and `TemplateViewLoaded` check
theirs, so a slow read never lands as another template's contents.

## Discovery, patches and generations

The first frame's counts come from `library::index_summary`, labelled `(from
index)`, while `library::discover` runs on a worker; a pty test asserts one
`discover` and zero `scan_base` over a fresh index.

**A content mutation patches its row; only a structural change reloads.**
`ListChange::Patched { project, stale }` replaces the row, drops the `stale` size
snapshots and lets `recompute` re-test the query; `Removed { path }` drops it;
`Reload` discovers again; `DetailOnly { path }` re-reads only the pane (notes and
todos live nowhere else). Adding a tag must never re-read every
`PROJECT_INFO.md`, and the pty suite traces that it does not.

**The pane reads the file, and keeps reading it**, because a cache only in-app
verbs invalidate lies after an outside edit. Each `ProjectDetail` carries a
`Stamp` — metadata mtime and length, folder mtime — taken **before** the read, so
a racing write is caught next time. `Effect::RefreshDetail { path, stamp }` stats
on the worker and reads only when the disk disagrees; `selection_effects` asks on
each visit to a cached row, `Reload` (F5) asks for the selected one, and
`Runtime::watch_detail` asks once a second (`WATCH_EVERY`). `DetailWorker` serves
`wanted` before `check`, latest wins on both. A `Msg::Detail` that disagrees with
its row patches the row and pushes `Effect::RefreshCache`
(`library::refresh_cache`: lock-free, atomic, disposable — last writer wins, as a
hand edit already allows).

`LibraryState.generation`/`inflight`: a discovery is installed only if its
generation is the one in flight; a patch or removal meanwhile sets `dirty`, and
the landing answer triggers one more discovery. Selection survives a re-filter, a
re-sort and a reload by **path**.

**A batch item's effects are the app's**: `on_job_item_done` returns
`apply_change`'s effects with the next item's `Run`. `App::discover` sets
`inflight` before returning its effect, so a dropped `Reload` would leave every
later patch merely `dirty` and the list frozen for the session; the helpers in
`tests/tui_update/harness.rs` find the `Effect::Run` among the effects for that
reason.

## The table

**The folder name is never cut**; a row is eaten from the right.
`view::projects::choose_columns` adds optional columns while the widest name
still fits, in the order they are missed — size, date, base, template, tags — and
**stops at the first that does not fit**, so a narrow later column never jumps a
wider earlier one. Widths come from the rows, never the sizes, so a landing size
cannot reflow the table; the size cell is `rows::SIZE_CELL`, right-aligned, header
included.

**The base is promoted above the date when the visible rows span more than one
base** — a question about the rows on screen, not the configuration.
`LibraryState.many_bases` and `base_width` are measured in `recompute`, so
`App::table_min_width` can claim that column before long names take its room.

**Width is display columns, never bytes or characters** (`Проекты` is seven
columns and fourteen bytes): `view::fit`, `view::pad`, and
`widgets::input::visible_window` for a caret. **Every column is measured**, tags
included (`tag_cell_width`, and no column when no visible row has a tag), because
a remainder column cuts a tag mid-word into a different tag. **One column of right
gutter is always reserved**, or a right-aligned size touches the border and the
scrollbar lands on its digits; reserving it only with a scrollbar would reflow
widths as rows arrive.

**Nothing blocks on a size.** A cell shows `scanning…` first; `util::size_scan`
has two workers, and `request` **replaces** the queue with what is on screen,
selected row first. Snapshots last the session, and `forget` takes a mutation's
`stale`. **Bases are probed, never `is_dir`-ed** (`paths::probe_dirs` on the
summary worker), because `is_dir()` on a dead SMB mount blocks for the OS timeout.

## Search, sorting, filtering

`app::search::Query` splits the bar: what `core::query` parses with an operator is
`structured` and evaluated by `core::query::evaluate` exactly as `fastf search`
would; bare words are `free`. Rows answer `tag:`, `template=` and `created>` from a
`Metadata` synthesised by `row_meta`; a template-variable predicate emits
`LoadMeta` for the rows that lack it. Relevance sorts while there are bare words,
unless `s` chose an order. A malformed query is named while it is typed
(`core::query::diagnose`).

**Fuzzy is deliberately not very fuzzy** (`fuzzy.rs`, `library::match_fields`),
because a subsequence over every field joined together says yes to almost
anything: a word matches **inside one field**, **substring first**, and a fuzzy
hit counts only when its span is at most the word's length plus a third. Substring
hits outscore fuzzy ones; `Fuzzy::match_all` also ranks the palette and pickers.

**Every one-way order runs both ways**: `Sort { order, reversed }` rather than
more `Order` variants, and `Order::reversible()` keeps `newest`/`oldest` from
offering a reversed twin. **The tie-break (the date) does not reverse**, or groups
of equals would shuffle. `s` cycles the orders; a direction is the picker's to
choose. `Sort::from_label` reads labels with and without a direction.

**The tag filter writes `tag:x` into the search bar** (`Then::TagFilter`,
palette-only) instead of adding a filter field, so it clears with the same Esc
rung and the bar keeps saying what filters the list.

## The single-project actions, marks and batches

The verbs on a project are native modals (`app/actions.rs`: `ActionsState`,
`TextPrompt`, `Confirm`, `MultiPick`). `command.rs` binds `Enter`/`a` to the
action menu, `A`/`Ctrl-T` to add/remove tags, `N`/`Ctrl-N` to the editor and
inline notes, `r m u D` to rename/move/unregister/delete, and `M`/`J` to the
read-only metadata and notes views. Menu rows come from the registry in display
order (`action_entries`), and one that cannot run is dimmed with its reason.
Prompt texts and validators live in `validators.rs`.

**Every verb but rename batches over the marks**, with `jobs::JobKind` carrying
the answer asked once (the tag, the note, the base). Delete asks for the word
`delete` and names every folder; a mismatch keeps the text and says
`validators::DELETE_MISMATCH`, "type delete to confirm — nothing deleted". The
quick note is a text area (Enter saves, Alt-Enter breaks a line).

**`command::batch_target` is what a batching verb is available on.** Marks are
kept by path and survive a filter, so `targets()` intersects them with the view,
and an empty intersection must say so rather than do nothing. It is **not** part of
`needs_selection`: `o`, `t` and `y` act on the cursor's row only.

**A mark is the retry list**: an item that succeeds loses its mark as its outcome
lands (`take_inflight`), so a failed batch leaves exactly the rows to retry.
**`v` marks from the last mark to the cursor**, in **view order**;
`LibraryState.last_mark` is the row Space last acted on (mark or unmark), kept by
path and dropped when the row leaves; with no anchor the key is `Disabled` with a
sentence saying what to press first.

A finished verb patches its row by the **path it had** (`ListChange::Patched`
carries `was`) before the id, because `copy-to` can put one id in two bases.

**A move is a job** on a worker with a shared `Progress` and a cancel flag, so
Ctrl-C during a move cancels it instead of quitting. `MoveOutcome::staged` and
`copied` tell both surfaces whether it was an instant rename or a verified copy of
so many files, and `JobStatus` reaches `Done` on both paths, so `Runtime.moving`
clears and a later `Effect::CancelMove` touches nothing.

The `$EDITOR` note suspends into `Suspended::Note` and the CLI's own
`cli::note::note_from_editor`. The metadata and notes views load through
`loaders.rs` and render read-only, notes in file order. **The message log**
(`App.log`, `L`) keeps every status line and `diag` warning, stamped by
`App.clock` (the wall clock in the runtime, a fixed string in fixtures), with a
count of the warnings that arrived under a dialog.

## The flows that build something

Create (`n`), apply (`E`) and register (`e`) are one shape in `app/wizard.rs`:
**a form, then a preview, then Enter** — one `Modal::Flow(Flow)` with a `Step`.

**Every question is on screen at once** (`widgets/form.rs`: Tab and the arrows
move, `←`/`→` change a choice, Space opens a fuzzy picker over its options through
`Then::FormField`, Enter submits, Esc abandons), because prompts in sequence hide
earlier answers and a late rejection loses all of them.

**A refusal names its field.** What `update` can check without I/O — a required
variable — it checks first (`Flow::missing_required`); the rest the preview worker
refuses with `loaders::PreviewRefusal { field, error }`, and `Form::fail` puts the
message on that field with the text intact.

**The preview is built by the code that commits**: `Effect::Preview(Request)` and
`Action::{Create,Apply,Register}` take the same `Request`. A preview's ID is
advisory — `operations::create` re-plans under the lock, since reusing a previewed
ID is how duplicates are minted. `confirm_create = false` sets
`Flow::auto_commit`: the plan is still built, so every refusal still applies. Esc
at the preview goes back to the answers.

**Post-create runs on the main screen** (`Suspended::PostCreate` through
`ActionOutcome::follow_up`), because `git init`, the editor and a template's
commands want a terminal. `ActionOutcome::select` then selects the new project by
path once `Msg::Discovered` contains it.

Register differs only in which questions apply: the scope field hides the three
that `RegisterFlags::validate` refuses for bulk registration.
`cli::register::{plan_rename, recursive_targets, recursive_id_note}` are the
print-free halves both surfaces preview from.

## Two tabs

`App.screen` is `Screen::{Library, Templates}`, switched with `T`. **Templates are
a tab, not a dialog**, because a tab keeps its place; `Studio` is state on the
`App`. The tabs share every band but the middle one, and each owns its search: the
library's is the query grammar, the templates tab's a plain substring over slug
and name (`Studio::rows`), because fuzzy over tens of rows says yes to nearly all
of them; switching clears it. `Context::Templates` carries the tab's verbs;
`LISTS` is the library's screen, so `n` means one thing per hint bar. `f` filters
the library by the selected template **and goes back to it**.

The tab lists real templates **alphabetically by display name** — a stable order,
so creating a project never moves a row — then orphan slugs (named by projects,
held by no folder), dimmed and never opened on. `TemplatesState::rebuild` builds
the list, counts and orphans both, and `App::refresh_templates` hands it over; the
header counts `TemplateCard::on_disk`. `Studio::install` keeps the selection by
slug **only once a real template has been listed**, since discovery lands before
the summary and would park the cursor on `(registered)`. A template action reports
**`ListChange::SummaryOnly`**, re-reading no base, and the landing summary
refreshes the tab with its selection kept.

## The builder

Enter or `e` on the tab opens it, with the template's details read on a worker
(`loaders::template_view`, rendering `cli::template::describe`, the lines
`template show` prints). The tab's verbs are `n`, Enter, `I` and `D`.

**The builder is a list of a template's five parts, not a sequence of steps**
(`app/studio.rs` holds the scratch `Template` and the open section), and every row
summarises what its part holds. Nothing is written until Save, which answers
`Cannot save:` in `Template::validate`'s words rather than writing something that
will not load.

**The builder stays up until the write has landed** (`Builder::saving`), so a
refusal from under the lock — an occupied slug, a held lock, a full disk — lands
on the list with every answer intact; `on_action_done` pops only on success, like
the `Settings` and `Onboarding` arms. While saving it takes no keys, Esc included,
because a sent write cannot be cancelled.

**Leaving asks only when something would be lost**: `is_dirty` compares against
`Builder::original` whole, so an undone typo is not "worked on". Esc, `q` and
Ctrl-C all reach `close_top`'s `ConfirmThen::DiscardTemplate`.

**A new template may not land on an occupied slug**; `operations::save_template`
is the authority (see `src/core/CLAUDE.md`), and the app asks the in-memory cards
first so the refusal lands on the list. **On an edit the slug stops following the
name**: `metadata_form` marks an existing slug `touched`, so `suggest_slug` never
retypes it and fixing a title never renames the directory.

**The naming pattern is checked where it is written** (`studio::pattern_warning`):
a `{token}` no variable answers, and a declared variable the pattern never uses,
both load and save fine and then name every project wrongly. The Metadata row
wears `⚠`, the footer says which, and the form's hint updates as you type
(`sync_metadata_form`); neither refuses a save.

**The footer says what the highlighted row is for**, and `s` saves from the
section list — the one face with nothing to type into, like `a`, `d`, `K` and `J`
on its inner lists (`command::builder_list_closed`). `fastf template new` and
`template edit <slug>` open the app at `Entry::Studio`, so the command line and
`T` share one editor.

**Structure** is a `widgets::text_area::TextArea`, one folder per line with the
tree redrawn beside it; Enter is a newline, so **Ctrl-S commits**. **Files** is a
path line over a text area, listing the `{tokens}` the template understands and
naming the ones the text uses as it is typed; an empty body is a `.gitkeep` marker.
**`widgets/text_area.rs` is ours** because `tui-textarea` pins an older ratatui
that does not resolve against ours. Like `LineEdit`, its cursor is a char index on
both axes, and its viewport is a `Cell`, because `view` holds `&App` and a scroll
re-derived every frame jumps.

## The panel, the guide, and where the words live

**`guide.rs` is to explanations what `command.rs` is to keys**: the builder's
panel, the seven-page guide and the coach all read it, so each explanation exists
once. It is pure — no I/O, clock or `Config` — and takes only the scratch
`Template`, so `update` can call it. It holds itself to two rules: **no key is
spelled in prose** (`{key:BuilderSave}` resolves through `command::key_of`;
`every_key_placeholder_names_a_command`), and **no character the theme owns a
glyph for is written into prose**, which has no theme to ask for an ASCII spelling
(`no_glyph_the_theme_owns_is_written_into_the_prose`; the live tree asks
`Glyphs::is_ascii()`). An em dash is punctuation, not a glyph.

**The panel explains; the footer refuses and warns**, because the footer is a
fixed row a warning cannot be pushed off. So `pattern_warning` is in the footer
alone, and the panel shows the sample folder name it is about. **The panel never
repeats the editor beside it**: from the section list it shows the live half
(sample name, the next two IDs, the tree, the files); with a section open it is
`explain_section`, prose only.

**`studio::sample_folder_name` is pure and not "now"**: a `naming::RenderContext`
literal dated 31 January, the day `{YYYY}`, `{MM}` and `{DD}` are visibly
different numbers, so `update` and snapshots can both call it.

**The guide offers itself once, through one flag**: `Session::guide_seen`, set in
`open_guide` by every route, so whoever found it is never offered it and nobody
sees it twice. It opens on top of whatever asked, so Esc goes back there. **The
coach advises and never refuses**: `guide::gaps` counts what is worth a look for
the Save row, while `Template::validate` and `operations::save_template` keep the
authority.

**Every suite that drives the templates tab starts past the offer** —
`Sandbox::guide_seen()` in the pty tests, `App.guide_seen` in
`tui::testing::fixture` — so a test about the editor is about one thing.
`tui::testing::guide_fixture` and
`flows::the_guide_offers_itself_once_and_leaves_the_editor_underneath` meet the
offer on purpose.

## Motion that guides the eye, and only where it answers a question

**`src/tui/motion.rs` is pure** — no clock, environment or I/O, and every function
takes the milliseconds it reasons about — so `update` starts a pulse and a test
asserts the frame it makes at a chosen millisecond.

**A pulse is a background**, because every cell sets its own foreground, which
would override a row's. **It fades rather than flashes**, because a background
snapping on and off looks like a broken terminal: `motion::wash` eases
(`ease_out`, `mix`) from `pulse` toward `Theme.ground` over `PULSE_MS`. The sixteen
ANSI colours have no ramp, so there the wash holds for `ANSI_HOLD` and lets go;
mono never moves. One `wash` serves the table's rows, the pane's rows and the
status line.

**Focus eases between rest states**: `motion::border_style`/`title_style` take
`focus_moved_at` and ease the gaining pane `border → border_focus` and
`dim → accent` over `FOCUS_MS`, and the losing pane back (`eased`), so there is no
end step to snap through. `App::set_focus` is the one mover, so every change
stamps `focus_moved_at`; `view::projects::title_style` and `border_style` read it
for the table, the pane and both templates-tab panes. Dialogs keep
`Theme::border`.

**A message arrives, and goes**: `Status.shown_at` (an `Option`, so a
`Status::default()` in a test does not read as arriving) drives
`motion::arriving_style`'s wash for `ARRIVE_MS`, and `expiring_style` dims the last
`FADE_MS`.

**Find my row**: `App::reordered` recomputes and pulses the selected row for
every reorder someone asked for — `s`, `S`, `f`, `b`, `F`, the tag filter — and is
deliberately not in `after_rows_changed`, which discovery, sizes and every search
keystroke run through.

**What moves, in full**: a row a verb changed (by path in the table, by index in
the pane — hence `Pulses<K>`); a size cell whose number *changed*; the selected
row after a reorder; the focus between panes; a status message arriving and
going; one activity indicator where something is pending. **Not built**: eased
scrolling, dialog transitions, cursor trails, a reveal sweep (it fights a held
arrow). Motion, like colour, appears only where it means something.

**A page filling in is not a change**: `Msg::Sizes` pulses a number that replaced
a different one, or a size a verb discarded coming back (`App.rescanning`, set by
`ListChange::Patched`) — never a first fill, which lights every visible row at
once and again on every scroll.

**Off is a first-class state**: `theme::choose_motion` resolves it beside the
palette from an `Env` and one config key (so `Effect::Retheme` and `Msg::Themed`
carry both), and `Mono` is always off. Off, every frame is the rest state and no
fast wake is asked for.

**The pane's cursor is the mono-visible focus cue**: focus is otherwise colour,
which `Theme::mono` lacks, so the cursor is drawn only while the pane has focus
(`view/projects.rs::detail`), in the selection style mono draws reversed — which
also keeps a lit row out of a pane the keys are not going to.

**A snapshot cannot see motion**: `TestBackend` records symbols, and snapshots
render mono. `testing::render_to_buffer` is the one place a frame's *colours* are
asserted.

## The pane is an editor you enter on purpose

**`pane::pane_rows` is the one answer to "what is in the pane"**, read by the view,
the cursor arithmetic and the scroll ceiling, so which rows exist and which are
`PaneRow::selectable` is never counted twice. One line per row and no `Wrap`,
because the cursor and `detail_scroll` count rows.

**A note is several rows, and so is a long line**: `App::pane_rows` hands
`pane_rows` the pane's inside width, and every line of a note is wrapped to the
columns after its date (`NOTE_INDENT`) by `wrap_columns`, in display columns.
`PaneRow::Note` is the first row with the date in a ten-wide column (blank when
undated), then `NoteLine`s up to `NOTE_LINES_SHOWN` rows, then `NoteMore`; only the
first is selectable, and Enter edits the whole note. The latest `NOTES_SHOWN` notes
show under `EarlierNotes(n)`, whose Enter is `ShowJournal`. A todo is a
`PaneRow::Todo`, drawn `[x]`/`[ ]`, with the rest of a long one in `TodoLine`s.
**A row holds only what fits, so an edit reads its text from the detail**: the
note editor opens on `detail.notes[ordinal]`, and a toggle names
`detail.todos[ordinal]` — never a row's text, or a wrapped todo would be refused
as changed. **Rules are
never selectable, and every section that can grow ends in an add row** (`AddTag`,
`AddNote`, `AddTodo`), so the three sections behave alike. `Figures` counts the
notes and the todos done.

**Nothing edits until Enter, and Esc leaves the row as it was.** The cursor walks
the selectable rows (`pane::step_cursor`), and `CommandId::PaneEdit` dispatches on
the row: the name opens the rename prompt; a tag opens on its line (emptied, it is
removed through `operations::replace_tag`); "add a tag" is `open_add_tag`; a text
variable opens on its line; a `select` variable opens `Modal::Pick` with
`Then::PaneVariable`, which cannot yield a value outside the options; a note opens
a `TextArea` over its rows (`PaneEdit::Note`, with the ordinal and the old text —
`Ctrl-S` saves through `operations::replace_note`, empty removes, unchanged
cancels, and `PaneEditConfirm` is hidden so Enter is a newline); "add a note" is
`NoteInline`; "add a todo" is a `TextPrompt` with `TextThen::AddTodo`. **A todo
toggles at once**: Enter sends `Action::ToggleTodo` and records `App.pane_pending`
rather than opening a `PaneEdit`, because `App::context()` answers
`Context::PaneEdit` whenever `pane_edit` is set, and a toggle has no field. Adds
set `pane_pending` to the new item's ordinal, so the cursor follows it. The hint
bar's Enter names what it will do (`edit`, `toggle`, `add`, `show`) through
`command::hint_title`. The edit lives in `App.pane_edit` beside the rows, so what
is being changed stays in view.

**`Context::PaneEdit` is a text-entry context**: the field has first refusal, and
the registry answers Enter, Esc and `Ctrl-S` (`on_pane_edit_key`). So Enter on the
list is its own id, `ActionsEnter` over `[Projects]`, hidden from the bar and the
palette, because one id cannot bind different keys in different contexts; `a`
still opens the menu from the pane.

**An edit stays open until the worker answers**: `send_pane_edit` marks it
pending, and `on_action_done` closes it and pulses the row on `Ok`, or puts the
refusal on it with the text intact on `Err` (`PaneEdit::fail`). Moving the focus
or the selection drops an open edit untouched.

**The cursor follows the thing, not its index.** A tag or variable edit returns
`ListChange::Patched`, which drops the cached detail and rebuilds the rows; a note
or todo returns `DetailOnly`, which keeps the pane until the re-read lands, so no
`reading…` frame flickers. `PaneEdit::target` (`PaneTarget`: tag text, variable
slug or ordinal) lets `settle_pane_cursor` find the row after `apply_change`, and
`App.pane_return` finds it again when `Msg::Detail` lands, which also re-anchors
an open edit (`PaneEdit::set_row`) and re-clamps the cursor.

**What the pane admits is what the file can hold**, ruled once in `core`:
`validated::Tag` at `operations::add_tags`, `vars::rendered_values` in
`set_variable`, and `body`'s grammar under `replace_note`, `toggle_todo` and
`add_todo`, which refuse a note or todo changed on disk since it was read.
`validators::tag` is the same rule for the prompts that refuse before a worker is
asked.

## Settings, the counter, maintenance, the first run

`,` opens `Modal::Settings`: every setting on one screen, grouped, each value
beside its label. **A row's key is the configuration key**: every write runs
`cli::config::apply` — the print-free half of `fastf config set` — on a worker, so
a refusal is the command line's, word for word, and `app/settings.rs` knows
nothing about what is legal. A yes/no or a two-way choice toggles in place;
anything else edits **on its own line**, pre-filled, with the refusal under it and
the text kept. The **library bases** are one `TextArea`, a folder per line, Ctrl-S
to keep. The **ID counter** and the maintenance verbs (reindex, check and recover,
data locations) are rows too; `!` is `CommandId::Reconcile` from anywhere, for the
header's `⚠ n needs attention`. `ActionOutcome::settings()` re-reads the screen
after a write, so a normalised value shows as stored.

**`/` narrows the settings** (`Editing::Filter`, the value editors' own machinery,
so `Modal::context()` needs no new answer): a case-insensitive substring over the
label, the value and the **configuration key**. A heading survives only with
something under it. The filter shows in the **title** and is edited on the
**footer**, so it costs no list row and cannot be pushed off; Esc gives the whole
screen back, since a forgotten filter hides rows for no visible reason.

**The first run is a dialog.** `tui::run` loads the `Config` before taking the
screen, so a corrupt one stops where the error can be read, and passes a folder to
suggest when no base exists anywhere; `App::request_onboarding` shows the question
before the first frame and keeps it up until the folder exists, with the text kept
on a refusal.

## Testing

`tests/CLAUDE.md` has the three layers — `tui_update` for the state machine,
`tui_snapshots` for the frames, `tui_pty` for the runtime through a real terminal
— and the rules that keep each one honest.
