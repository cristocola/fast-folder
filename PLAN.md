# v3.5.0 — the app answers the keys you try

Five phases, one per session. Each is its own PR into `main`, green on both
platforms before the next starts. Delete this file when the release ships.

The full plan, with the audit that produced it, is the session plan; what follows
is the executable list and what each phase leaves behind.

## Phase 1 — one movement grammar, and a horizontal axis  ☑

- `PAGERS` gains `Actions` and `Builder`; `JUMPERS` gains `Templates`, `Actions`
  and `Builder`. Every list pages and jumps.
- `HalfDown`/`HalfUp` (`Ctrl-d`/`Ctrl-u`) over `SCROLLERS`. `ClearSearch` gives up
  `Ctrl-u` and becomes palette-only.
- `StudioFromFolder` `g` → `I`; `Guide` `G` → `Ctrl-g`. `g`/`G` are first row and
  last row everywhere, with no exception to remember.
- **The horizontal axis is depth.** `→`/`l` enters what is under the cursor;
  `←`/`h` leaves one level. `←` never quits: it is unbound on the library, which
  is home. New `FocusTable` (`Detail`) and `BackToLibrary` (`Templates`).
  A text field and a paged reader own their own arrows — the one exception.
- `Context::Guide` with declared `GuideNext`/`GuidePrev`.
- `Ctrl-C` becomes `CommandId::Interrupt` so the key that cancels a job is in the
  help; the body moves into `run`.
- Pulled forward from phase 2, because `every_context_has_help_and_a_way_out`
  found it red: `Context::SearchEdit` and `Context::Palette` get declared
  commands, and their handlers offer the registry every key that is not
  printable text. The palette opener is bound in every context but the palette,
  so `Ctrl-p` inside it is the previous entry.

What changed against the plan, and why:

- `Guide` went to `H`, not `Ctrl-g`: five extra columns in the hint bar pushed
  `c commands` off the end of the templates tab.
- `→`/`←` are one `Descend` and a small `Ascend`/`FocusTable`/`BackToLibrary`
  family rather than extra keys on five openers — four keys on one row sized the
  help's key column to `a / Enter / → / l`.
- The hint bar's ordering rule moved onto `Category::Help`; it had been "own
  commands, then global ones", and the palette had just stopped being global.

Left behind: `every_list_binds_the_whole_movement_grammar`,
`an_arrow_and_its_vim_letter_are_bound_together`,
`every_context_has_help_and_a_way_out`, a `movement` module in
`tests/tui_update.rs`, and the grammar written down in `src/tui/CLAUDE.md`.

## Phase 2 — every key line comes from the registry  ☑

- A movement summary in `command::hints`; the six hand-written `↑↓` copies go.
- `Context::SearchEdit` and `Context::Palette` get declared commands, and their
  handlers consult the registry for every key that is not printable text.
- The literal key pairs in `view/builder.rs` and `view/modals.rs` read the
  registry. `SPINNER` moves into `Theme::glyphs` with an ASCII twin.
- `inline.rs`'s picker speaks the same vocabulary.

Beyond the plan: `Context::Prompt` and `Context::Pick` had to exist before the
bar could read anything for a prompt or a picker, and `command::keys_in` had to
exist before it could read anything true — a bar that offered `? help` over a
rename prompt, where `?` types a question mark, is worse than one that spells
its keys. The search bar joined `SCROLLERS` on the back of it.

Left behind: `no_key_line_is_written_by_hand` in `tests/layering.rs` (proven
against a planted literal), `a_text_entry_context_advertises_only_the_keys_that_fire_there`
and `the_arrows_are_spelled_once_and_read_everywhere` in `tests/tui_commands.rs`,
and a `key_lines` module in `tests/tui_update.rs`.

## Phase 3 — four things the app could not do  ☑

- `v` marks from the last mark to the cursor.
- `/` filters the settings list.
- Every sort runs both ways.
- `FilterTag`, palette-only, writes `tag:<x>` into the bar.

Left behind: a `more_options` module in `tests/tui_update.rs`, and a
`session.rs` unit test that a sort label written before there was a direction
still names an order.

## Phase 4 — motion, and only where it answers a question  ☑

- `Msg::Tick(u64)` carries milliseconds; `App.ticks` becomes `App.elapsed_ms`.
- `Runtime::wait` keeps a deadline that survives a message burst — today the
  spinner freezes exactly when the app is busiest.
- `App::tick_interval()` replaces `needs_tick`.
- `src/tui/motion.rs`: the change pulse, the resolve pulse, one activity
  indicator, the toast fade. Nothing else moves.
- `motion` config key, a settings row, `FASTF_MOTION=0`; off under mono.

What the plan did not know: the clock has to be stamped on **every** message,
not carried by the tick — a clock that only advances on a tick is stale the
moment nothing is moving, and a status message set against a stale one expires
instantly. And the pulse has to be a **background**: every cell in a row sets
its own foreground, so a foreground set on the row is invisible in a real frame
while passing a test that asked the wrong question.

Left behind: `testing::render_to_buffer` — the one place a frame's colours are
asserted, since snapshots record symbols and render in `mono` where motion is
off by rule — plus a `motion` module in `tests/tui_update.rs` and six unit
tests in `src/tui/motion.rs`. The settings-navigating tests in the pty and
snapshot suites now walk to a row **by name**, because counting `Down`s meant
one new setting broke six assertions about something else.

## Phase 5 — the record, and the release  ☐

`docs/cli.md`, `src/tui/CLAUDE.md`, `ROADMAP.md`, screenshots of every touched
screen, then the `release` skill.
