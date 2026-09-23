# PLAN — the pane goes where the room is

Ten phases on one branch, `pane/room`, one commit per phase, one PR at the end. The gate
list is the `release` skill's; run all of it between phases. Publication — the version
bump, the tag, the AUR — waits for an explicit "release".

## Why

The detail pane is drawn only beside the table, only at 100 columns or more, and only
when the table leaves it 26 (`layout::regions`, `src/tui/layout.rs:47`). The table keeps
every folder name whole and measures its claim from the longest name, so:

| Window | Showcase names (~40 chars) | A library of ~90-char names (`FASTF_SHOT_LONG=1`) |
|---|---|---|
| 170×30 | pane beside | pane ~38 wide; name, facts and figures cut mid-word |
| 120×40 | pane 48 wide | **no pane**, the table claims ~118 |
| 100×30 | pane 28 wide; name cut; figures cut after "2 notes ·" | no pane |
| 99×30, 80×24 | **no pane, `→` unbound** | no pane; names cut by the border |
| 60×45 | no pane under 8 rows and ~30 empty ones | no pane |

Notes and todos — the parts of a project that change every day — are hidden at the
sizes people actually use. The goal: the pane is one key away at every window shape
down to a tmux quarter or a phone, laid out to read well at that size; todos read as a
calm, finished work list that is quick to add to and to edit; nothing that works today
stops working.

**Decided by the maintainer (2026-09-23):** Enter ticks a todo and **F2 edits** its text,
and that pairing is **the same throughout the app**; **adding todos must be easy**; the
pane's order is header, tags, todos, notes, then reference (variables, contents).

### Defects this plan fixes

| # | Defect | Where |
|---|---|---|
| 1 | A resize leaves focus on a pane that is no longer drawn | `Msg::Resize` → `after_selection_change`, `app/mod.rs:849`, never repairs focus |
| 2 | A resize drops an open pane edit, typed note included — also on return from `$EDITOR`/Ctrl-Z, since `announce_size` resends an unchanged size | `mod.rs:666`, `runtime.rs:584` |
| 3 | **Esc in the pane quits the app** when the Esc ladder is empty | `CommandId::Back`, `mod.rs:1558-1597` |
| 4 | No caret while editing in the pane | `view/mod.rs:55-62` places it only while searching |
| 5 | PgUp/PgDn in the pane move by the table's height and skip wrapped rows | `page_rows`, `mod.rs:1539`; `mod.rs:1791` |
| 6 | The note editor draws nothing on the pane's last row | `view/projects.rs:677-686` (`top + 2 > bottom`) |
| 7 | `width * 60 / 100` — the banned u16-overflow spelling | `layout.rs:64` |
| 8 | The pane's text width is computed twice | `app/pane.rs:733-751` vs the view's `block.inner` |
| 9 | Below the minimum, `q` over an edited template pushes an invisible confirm; the second `q` discards unseen | `App::quit`, `mod.rs:2196` |
| 10 | `FASTF_SHOT_REAL=1` wrote the real `state.toml` | fixed in Phase 0 |
| 11 | Space in the pane swaps the project without reading it (`reading…` forever) | `MarkToggle`, `mod.rs:1997-2012` |
| 12 | A paste into a pane edit is dropped | `on_paste`, `mod.rs:1280` |
| 13 | Every background reload resets the pane cursor to the top | `after_rows_changed`, `mod.rs:639-658` |
| 14 | A detail refresh during a tag edit moves the editor to "add a tag" | `Msg::Detail` re-anchors with the *typed* text, `mod.rs:990-995` |
| 15 | The table's scrollbar ignores the ASCII alphabet | ratatui's default `║`/`█`, `view/projects.rs:332` |

## The design in one page

**Three placements, one pure decision.** `layout::place(body, needs) -> (table, pane,
Placement)`; no focus goes in — whether the pane is *drawn* is the app's answer.

- **Beside** when the names fit whole plus a pane of `PANE_BESIDE_MIN` (36): table
  `fit_between(percent_of(w, 60), table_min, w - 36)`. The 100×30 fixture keeps its split.
- **Below** otherwise, when the body has `TABLE_BELOW_MIN` (9) + `PANE_BELOW_MIN` (14)
  rows: full-width table (names whole), pane under it. Table height
  `fit_between(min(library_len + 3, percent_of(h, 45)), 9, h - 14)`.
- **Over** otherwise: the list has the body; focusing the pane (`→ l Tab i`) draws it in
  the list's place; `← h Tab Esc` return. Same rects either way, so nothing re-wraps.
- The table's claim is measured over the **whole library**, not the filtered rows, so
  typing a search never moves the pane (it can today).
- Over, list focused: a dim peek of the row's notes/todos on the table's bottom border,
  and `→ details` leads the hint bar. The content is read whenever the pane is **on**, so
  `→` is instant.
- `i` shows or hides, literally. **Esc from the pane goes to the list**, both tabs.
  `<`/`>` in the pane: previous/next project, same section, no pulse (`[`/`]` need AltGr
  on several European layouts). The cursor follows the item across resizes, reloads and
  re-wraps; open edits survive; a same-size resize is a no-op.

**One item grammar, declared once in `command.rs`:**

| Key | Means | Where |
|---|---|---|
| Enter | the row's action — **tick a todo**, flip a yes/no, open what is under the cursor | unchanged |
| **F2** | **edit this row's text in place**; emptied, removed where removal exists | pane (todo, tag, variable, note, name), project list (rename), templates tab, builder rows, settings values; hidden where there is nothing to type |
| **+** | **add one here** | pane (to the section under the cursor — a todo into the cursor's phase, a tag, a note), project list (a todo), builder lists, templates tab |

**Adding todos is fast:** `+` (or Enter on "· add a todo", or *Add a todo*) opens an
empty line in the list where the todo will land; Enter writes it and opens the next;
an empty Enter or Esc ends. A pasted list becomes one todo per line in one write
(`- [ ]`, `-`, `*`, `1.` stripped). Pane off → today's prompt. Command line:
`fastf todo edit <query> <n> <text>`, `fastf todo remove <query> <n>`.

**The pane reads well:** one text rect (borders + one column of padding); the name wraps
after `_ - .` or a space; facts and figures flow as whole facts with the size slot fixed
at `rows::SIZE_CELL`; order header, tags, todo, notes, variables, inside; a phase is a
text-colour sub-heading with a dim right-aligned count, tasks indented two, a finished
phase dim; `[ ]` no longer bold accent, `[x]` and its text dim; a themed scrollbar.

**Small windows:** minimum 40×12; header, search bar and status line keep what matters
and cut with the ellipsis; a name wider than the table is fitted, never cut by the
border; dialogs audited at 40×12; the templates tab placed by the same rule; a frame
sweep renders every buildable state across a grid of sizes.

## How to work a phase

Scope is that phase only; anything else found goes to the Parking lot. Gates between
phases: `cargo fmt --all -- --check`; `cargo clippy --all-targets -- -D warnings` in debug,
`--release`, and `--target x86_64-pc-windows-gnu`; `cargo test` and `cargo test
--release` (the pty suite included); `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`.
Look at every changed frame with `tests/tui_pty/screenshot.rs` **before** blessing its
snapshot (`INSTA_UPDATE=always cargo test --test tui_snapshots`). Update `docs/` and the
CLAUDE.md beside the code in the same commit. Tick a box only when it is verified; record
what happened in the Phase log.

---

## Phase 0 — groundwork

- [x] The screenshot tool's `FASTF_SHOT_REAL=1` reads a private copy of the data
  directory (`config.toml`, `state.toml`, `counters.toml`, `templates/`) through
  `FASTF_INSTALL_DIR`, keeping the real `HOME` (defect 10).
- [x] `FASTF_SHOT_LONG=1`: every other showcase folder gets a ~90-character name.
- [x] The showcase's first project's todos sit under `### Shoot` / `### Deliver`.
- [x] An `f2` token (`+`, `<`, `>` already pass as themselves).
- [x] The tool's doc comment and `tests/CLAUDE.md`; this file.

## Phase 1 — pane fixes that need no layout change

Defects 3, 4, 5, 6, 9, 11, 12.

- [x] Esc rung: right after the job-cancel check (`mod.rs:1562`), `focus == Detail →
  set_focus(Projects)`. `Back` is over `BACKSTEP = TABS`, so the template pane gets it too.
  Back's description (`command.rs:2007`) adds "leave the pane".
- [x] Caret: keep the search caret and the pane caret apart in `view/mod.rs:33-62`; set the
  pane's when `modals.is_empty() && pane_edit.is_some()`. Add
  `testing::render_with_caret(app, w, h) -> (Buffer, Position)`.
- [x] Paging: `page_rows()` follows the focus (table, pane, templates list, template pane);
  the `Focus::Detail` branch (`mod.rs:1791`) calls a new pure `pane::page_cursor(rows,
  from, delta_rows)` that pages by screen rows.
- [x] Note editor: `layout::box_at_row(inner, row, rest, 4)` replaces the early return at
  `view/projects.rs:677-686`; the key line stays on the box's last row.
- [x] Paste: an `on_paste` arm for `pane_edit` — a line takes the first line, a note every
  line, nothing while pending; clears the error.
- [x] Space: on the list, step and `after_selection_change`; in the pane, mark and stay.
- [x] Too-small: `render_too_small` says when a discard question is waiting ("a template
  has unsaved changes — make the window bigger to keep it, or q again discards it").
- [x] Tests (`tui_update`): `esc_in_the_pane_goes_back_to_the_list_and_never_quits`,
  `esc_from_the_template_pane_goes_to_its_list_first`,
  `the_caret_sits_in_the_field_being_edited`,
  `a_note_opened_on_the_last_visible_row_slides_up`, `a_paste_lands_in_the_pane_field`,
  `page_down_in_the_pane_moves_by_the_panes_height`,
  `space_on_the_list_steps_and_reads_the_next_row`, `space_in_the_pane_marks_and_stays`,
  `the_too_small_guard_names_the_template_it_would_discard`; unit tests for
  `page_cursor`. Snapshot `too_small_40x10` changes.
- [x] Docs: `docs/app.md` (Esc, Space); the Esc ladder in `src/tui/CLAUDE.md`.

## Phase 2 — the cursor follows the item

Defects 1, 2, 13, 14.

- [x] `app/pane.rs`: `PaneTarget` gains `Name`, `AddTag`, `EarlierNotes`; `find_row`
  arms; `pane::target_at(rows, i)`; `PaneEdit::anchor()` — the row the edit opened on,
  never the typed text. (`PaneSection` waits for phase 5, the first thing that needs
  it; `App.pane_anchor` is new — see the log.)
- [x] `Msg::Resize` → `on_resize`: take the anchors, resize, repair focus only if no pane
  exists, clear the index-keyed `pane_pulses`, clamp the library and studio viewports,
  re-find the cursor and the edit row, `viewport_offset`, never drop the edit. A same-size
  resize is a no-op for the pane.
- [x] `App.pane_for: Option<PathBuf>`: the resets in `after_rows_changed` and
  `after_selection_change` (`mod.rs:647-648, 664-668`) run only when the selected path
  changes (and clear `pane_pulses` then).
- [x] `Msg::Detail` (`mod.rs:982-1004`): take the anchor before `insert`; `pane_return`
  with a pulse, else the anchor without; the edit re-finds its row by `anchor()`.
- [x] Tests: `a_resize_keeps_the_panes_cursor_and_an_open_note_with_its_text` (with a
  same-size resize), `a_resize_rewraps_and_the_cursor_stays_on_its_todo`,
  `a_detail_refresh_keeps_an_open_tag_edit_on_its_tag`,
  `a_reload_landing_keeps_the_pane_cursor`.
- [x] Docs: "The cursor follows the thing" in `src/tui/CLAUDE.md` — anchor vs target,
  `pane_for`, resize.

## Phase 3 — core and command line: reword, remove, add several

- [ ] `core/body.rs`: `PlacedTodo` gains `line` and `text` ranges; `replace_todo(path,
  ordinal, expected, text)` beside `toggle_todo` (`:583`) — mirrors `replace_note`: empty
  removes the line with `remove_lines`, otherwise splices the text keeping indent, list
  marker, bracket state and `\r`; refuses two lines and a todo changed meanwhile, worded
  like `toggle_todo`'s. `add_todos_in(path, texts, phase)` — one atomic write.
- [ ] `core/operations.rs`: `replace_todo`, `add_todos_in` — one-line check before the
  lock, then `DataLock`, `revalidate_project`, body.
- [ ] CLI: `TodoAction::Edit { query, number, text }` and `Remove { query, number }`
  (`main.rs:709-738`, help `:518`, dispatch `:1169`); `cli/todo.rs` pulls the numbering
  refusal out of `done` (`:144-158`) into one helper; each verb prints one ✓ line.
- [ ] Tests: `body.rs` — keeps indent/marker/state, removes without a doubled blank,
  refuses a changed todo, keeps CRLF, leaves phases and every other byte, refuses two
  lines; `add_todos_in` into a phase and at the end. `tests/cli_output.rs`:
  `todo_edit_rewords_and_remove_removes_by_number` (0 and out-of-range refused).
- [ ] Docs: `docs/cli.md` (table and Todos section), `docs/projects.md` (what each writer
  touches), `src/core/CLAUDE.md` "Notes and todos". No `pub` doc may link the private
  `PlacedTodo`.

## Phase 4 — placement

- [ ] `layout.rs`: `Placement`, `TableNeeds { min_width, rows }`, `place`,
  `PANE_BESIDE_MIN`, `TABLE_BELOW_MIN`, `PANE_BELOW_MIN`; `Regions` gains `body` and
  `placement`; `regions(area, pane_open, needs)` on `place` with `percent_of` (defect 7);
  `help_box` gets `HELP_NARROW_BELOW`; delete `DETAIL_MIN_WIDTH`, `DETAIL_PANE_MIN`,
  `templates_body` — the templates tab draws in `regions.body` (`view/mod.rs:44`,
  `studio.rs:1508`); `template_rows` reads `.body`.
- [ ] `app/library.rs:436-456`: `library_widths` over the whole snapshot; `table_needs()`
  and `table_min_width()` read it. `choose_columns` keeps measuring what is on screen.
- [ ] `app/mod.rs`: `pane_live()` (on), `detail_visible()` (drawn this frame),
  `pane_behind_list()` (Over, list focused, library tab); `selection_effects` (`:680`),
  `DetailOnly` (`:822`), F5 (`:1738`) and `runtime::watch_detail` (`runtime.rs:278`) use
  `pane_live()`; `pane_present()` is `Templates || pane_live()`; `ToggleDetail`
  (`:1990`) — visible: close (Beside/Below) or go to the list (Over); hidden: open, and in
  Over go into it; then `selection_effects` only.
- [ ] `view/mod.rs:27-41`: Over with the pane focused draws the pane in the body;
  otherwise the table, plus the pane Beside/Below.
- [ ] `command.rs`: reword "beside" at `:1160, :1486, :1746, :1757` and the comments at
  `:298, :300, :513-533, :825, :867`; `mod.rs:2256-2258`; `studio.rs:1503-1506`
  (`guide.rs:222` is about the builder panel and stays).
- [ ] Tests: layout unit tests `the_pane_goes_where_the_room_is` (120×40/54 → Beside
  72+48; /80 → Beside 80+40; /95 → Below; 80×24 → Over; 200×15 → Beside; 60×45 with 8
  rows → Below, table 11), `a_standard_terminal_gets_the_compact_layout` → Over,
  `regions_tile_every_window` (pure sweep, w 1..300 × h 1..100, open/closed, several
  needs, and `u16::MAX`: bands tile, table and pane inside the body, minima hold, no
  panic). `movement.rs:117-137` → `on_a_small_window_the_pane_takes_the_lists_place` +
  `the_arrows_are_unbound_with_the_pane_closed`; `i_shows_or_hides_the_pane_where_it_is`;
  `library.rs:688-695` → no read only with the pane off, plus
  `a_small_window_still_reads_the_detail_so_the_pane_opens_at_once`;
  `typing_a_search_never_moves_the_pane`;
  `a_window_that_shrinks_keeps_the_pane_focused_in_its_new_place`;
  `the_templates_tab_takes_the_whole_body_whatever_the_library_pane_does`. Narrow
  `tui_update` fixtures now see `LoadDetail`: fix exact-effect assertions.
- [ ] Frame sweep in `tests/tui_snapshots.rs`, `every_state_draws_at_every_size`: build
  each state once (dashboard, pane focused, a line edit, a note edit, help, actions,
  palette, a query, wizard, settings, templates tab, builder, confirm, delete prompt, the
  journal) and move it through ~12×10 sizes with `Msg::Resize` + `render_to_buffer` at the
  same size. Assert no panic, the pane cursor inside the pane, the caret inside the text
  rect. Snapshots: `dashboard_over_80x24_pane_focused`, `dashboard_below_60x45`;
  `wide_names_120x40` becomes Below.
- [ ] Docs: `docs/app.md:39-54` and the →, ←, Tab, `i` rows; `README.md:57`;
  `src/tui/CLAUDE.md` Layout, The table (the library-wide claim), the horizontal axis.
- [ ] Look: 120×40 `G right`, 100×30, 80×24 `G` and `G right`, 60×45, 200×15,
  `FASTF_SHOT_LONG=1` at 120×40.

## Phase 5 — Over, polished

- [ ] `PanePreviousProject` (`<`) and `PaneNextProject` (`>`) over `[Context::Detail]`,
  after `FocusDetail` (`command.rs:1754`), hint off, palette off. Step the library,
  `after_selection_change`, `App.pane_seek = Some(section)`; land on the first selectable
  row of that section when the detail is there (fallback: the section's add row, then
  Name). No pulse.
- [ ] The door: in `command::hints` (`:2075-2078`) `FocusDetail` leads while
  `pane_behind_list()`; `hint_title` says `details` there, `pane` elsewhere.
- [ ] The peek: `view/projects.rs::table` — a dim right-aligned `title_bottom` from
  `App::pane_counts()` while `pane_behind_list()`; nothing when there are no notes or todos.
- [ ] Tests: `angle_brackets_walk_the_projects_from_the_pane_and_keep_the_section` (no
  pulse), `the_hint_bar_leads_with_the_door_when_the_pane_is_hidden`,
  `the_peek_says_what_the_hidden_pane_holds`; registry invariants.
- [ ] Docs: the keys table; `src/tui/CLAUDE.md` "Keys named nowhere else", the hint-bar
  ordering exception.

## Phase 6 — the pane's content

- [ ] `layout::pane_text(pane) -> Rect` (borders + one column each side), read by
  `App::pane_rows`, `pane_rows_on_screen` (`pane.rs:733-751`) and the view (defect 8). The
  highlight and the wash cover the full inner row.
- [ ] A shared themed scrollbar (`view::scrollbar(theme)`, ASCII symbols when
  `g.is_ascii()`) for the table and the pane (defect 15).
- [ ] Rows (`pane.rs:161-277`): `Name(String)` + `NameLine` via `wrap_name`; `Facts` and
  `Figures` flowed whole, the size measured as `SIZE_CELL`; order header, tags, warning,
  todo, notes, variables, inside; `Rule(PaneSection)`; `reading` with `g.ellipsis`.
- [ ] Tests: `a_long_name_wraps_after_its_separators`,
  `facts_flow_whole_and_wrap_between_them`,
  `the_figures_row_count_does_not_depend_on_the_size`, `the_living_sections_come_first`,
  `a_size_landing_never_moves_the_pane_cursor`,
  `the_pane_text_is_where_pane_rows_measured_it`,
  `a_pane_taller_than_its_box_has_a_scrollbar`; pane snapshots re-blessed after a look.
- [ ] Docs: `docs/app.md` pane paragraph; `src/tui/CLAUDE.md` "The pane is an editor".

## Phase 7 — todos, finished

- [ ] Rows: `Phase { name, done, total }`; `Todo`/`TodoLine` gain `phased`; wrap width
  minus the indent.
- [ ] View (`view/projects.rs:568-600`): phase heading in the text colour, dim right-aligned
  count, dim when finished; `[ ]` and open text in the text colour (no accent, no bold);
  `[x]` and done text dim; continuation aligned under the text.
- [ ] F2 on a todo: `PaneEdit::Line` with `EditTarget::Todo { ordinal, was }`, the checkbox
  prefix, continuation rows hidden while open; unchanged cancels; `Action::ReplaceTodo`
  (boxed `Project`, like its siblings) → `DetailOnly`, "Todo reworded." / "Todo removed.";
  a refusal under the line with the text kept.
- [ ] `+` in the pane: `pane_rows` takes the open add and inserts an `Adding` row at the
  insertion point (end of the cursor's phase, else the end); Enter → `add_todo_in` with
  that phase, then the line reopens after the new todo once the re-read lands; empty
  Enter or Esc ends; a multi-line paste → `add_todos_in`. `+` on tags →
  `open_add_tag`; on notes → `NoteInline`. "· add a todo" and *Add a todo* open the same
  line when the pane can show; pane off → today's prompt. Hint on a todo:
  `Enter toggle  F2 edit  + add`.
- [ ] Tests: `f2_rewords_a_todo_on_its_line_and_emptied_removes_it`,
  `a_todo_changed_on_disk_meanwhile_is_refused_under_the_line`,
  `plus_adds_into_the_cursors_phase_and_keeps_the_line_open_for_the_next`,
  `a_pasted_list_becomes_one_todo_per_line`,
  `a_phase_heading_counts_its_tasks_and_indents_them`, rich-theme
  `an_open_todo_is_plain_text_and_a_finished_phase_recedes`; snapshots
  `detail_pane_groups_todos_by_phase_120x40`, `…wraps_a_long_note_and_todo…`, new
  `detail_pane_editing_a_todo_120x40`.
- [ ] Docs: `docs/app.md`; delete the ROADMAP backlog item "Removing or rewording a todo".

## Phase 8 — the item grammar everywhere else

- [ ] `CommandId::EditItem` (F2) declared once over Projects, Detail, Templates, Builder,
  Settings, dispatching on the context: rename; the pane edit; `StudioEdit`; the builder
  row's edit; a settings value (hidden on a yes/no or a verb).
- [ ] `CommandId::AddItem` (`+`) over Projects, Detail, Templates, Builder: a todo; the
  section's add; a new template; `BuilderAdd`.
- [ ] Both palette-off (their verbs are there), hint-on where they apply; `CommandId::ALL`
  and the `CONTEXTS` array in `tests/tui_commands.rs` follow.
- [ ] Tests: registry invariants; `f2_means_edit_and_plus_means_add_wherever_they_are_bound`.
- [ ] Docs: an "Enter, F2, +" subsection in `docs/app.md` Keys; "One registry" in
  `src/tui/CLAUDE.md`.

## Phase 9 — small and odd-shaped windows

- [ ] `MIN_WIDTH`/`MIN_HEIGHT` 40×12; the too-small screen centred, naming the short side.
- [ ] `dashboard.rs`: `split_line` gains a winning side and a style-keeping fit; line 1
  drops "highest" then the base count, keeping the tabs; line 2 keeps an attention
  warning over "this session"; the search bar's right side by priority (count, marks,
  filters, `(from index)`, sort); status strings (`:264-319`) fitted.
- [ ] Table: the name fitted with the ellipsis when wider than the room, highlights kept.
- [ ] Dialogs: note and confirm key lines through `builder::key_line` (`modals.rs:555`,
  `613`); the action menu drops its description column first; onboarding prose wraps
  (`builder.rs:866`); template list rows fitted.
- [ ] Templates tab placed by `layout::place` (its own list minimum, share 38);
  `App::template_rows()` replaces `layout::template_rows` (`studio.rs:1051/1069/1080`,
  `mod.rs:710`); `studio_scroll_max` reads the split.
- [ ] The frame sweep extended down to 40×12. Snapshots `dashboard_40x12`,
  `dashboard_60x20`, `templates_tab_60x20`, `dashboard_200x15`.
- [ ] Docs: `README.md`, `docs/app.md` (40×12); ROADMAP manual passes gain 40×12, a
  tall-narrow split and a wide-short window; regenerate `docs/img/dashboard.svg`.

## Verification (by eye, after phases 4, 6, 7, 9)

`FASTF_SHOT_SIZE=WxH FASTF_SHOT_KEYS="…" cargo test --test tui_pty screenshot -- --ignored --nocapture`

| Case | Size | Keys |
|---|---|---|
| Beside | 120×40 | `G right` |
| Below | 100×30 | — |
| Over | 80×24 | `G`, then `G right` |
| Below, sized table | 60×45 | — |
| Beside, short | 200×15 | — |
| Minimum / too small | 40×12 / 39×12 | — |
| Reword a todo | 80×24 | `G right G f2 type:x enter` |
| Rapid add | 80×24 | `G right + type:a enter type:b enter esc` |
| Long names | 120×40, `FASTF_SHOT_LONG=1` | — |
| A real library | 120×40, 80×24, `FASTF_SHOT_REAL=1` | read-only keys |

Manual, added to ROADMAP: a real terminal at everyday sizes, a tmux split, and a Windows
console for F2, `+`, `<` and `>`.

## Parking lot

(nothing yet)

## Phase log

- **Phase 0** (2026-09-23): the screenshot tool's real mode runs on a copy of the data
  directory — verified by a run whose `G` moved the cursor in the app while the real
  `state.toml` kept its timestamp and row. `FASTF_SHOT_LONG=1` reproduces the no-pane
  frame at 120×40; the showcase's todos carry two phases. crossterm decodes the Linux
  console's F2 (`ESC [ [ B`), so F2 is safe there.
- **Phase 1** (2026-09-23): Esc leaves the pane on both tabs; the caret has one owner
  at a time; the pane pages by drawn rows; the note editor slides up; paste reaches a
  pane edit; Space in the pane marks and stays, on the list reads the next row; the
  too-small guard says in words what `q` would discard. The caret is read back by
  parking the test backend's cursor off-screen before the draw, since ratatui keeps
  the frame's own cursor private. No snapshot moved.
- **Phase 2** (2026-09-23): the cursor's identity is state, `App.pane_anchor`, set by
  every cursor move — a target taken *after* the rows were rebuilt is already the
  wrong one, so it cannot be computed at the moment a list change lands. The four
  new tests fail on the phase-1 build and pass on this one.
