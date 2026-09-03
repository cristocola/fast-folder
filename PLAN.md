# PLAN.md — v3.0.0: the guided app on ratatui

> **Complete. All eight phases are done (PRs #35–#42), and v3.0.0 is prepared:
> the version is bumped and the release row is written, so the tag is the one
> step left and it belongs on `main`.** This file was worked one phase per
> session: read it, do phase N, run the gates, tick only the boxes whose named
> verification actually ran, each phase in its own PR into `main` with
> `ROADMAP.md` and the matching `docs/` page in the same commit, and findings
> outside the phase sent to the Parking lot rather than fixed in passing. The
> **Phase log** at the bottom says what differed and why, phase by phase — read
> it before changing anything it explains.

## How to work a phase — read this first

1. `git checkout main && git pull`, then a branch `ratatui/phase-N-<slug>`.
   The phase's section below is the scope; its checkboxes are the exit
   criteria. Do not widen it — a finding outside it goes to the Parking lot.
2. Read `src/tui/CLAUDE.md`: the architecture (`App` + `Msg` + `update` with
   no I/O; `view(&App)`; one command registry; the runtime owns the screen)
   and **the look** (muted, minimal, robust). `tests/CLAUDE.md` has the
   harness rules.
3. Build the screen, then **look at it yourself** the way a person will:
   `FASTF_SHOT_KEYS="down enter" cargo test --test tui_pty screenshot --
   --ignored --nocapture` drives the real binary with those keys in a planted
   sandbox and prints the frame (`FASTF_SHOT_PROJECTS=n` for more rows,
   `FASTF_SHOT_REAL=1` for the maintainer's own library, read-only keys only).
   Do it for every new screen and every state of it — empty, loading, error,
   too small — and read what it shows. Only then write its snapshot.
4. Tests in all three layers for what the phase adds: `tests/tui_update.rs`
   (the state machine, no terminal), `tests/tui_snapshots.rs` (the frames,
   reviewed by eye, `INSTA_UPDATE=always` then commit), `tests/tui_pty/`
   (the runtime; assert on `app_screen`, never the raw transcript). Keep the
   traced guarantees: a mutation patches its row, opening does not rescan.
5. The gates below, `docs/cli.md` for anything a user sees, `ROADMAP.md`'s
   current-phase bullet, the checkboxes, a Phase-log entry for what differed,
   then the PR into `main`.

**Dependencies.** Widget crates are welcome when they earn their place — a
multi-line editor (`tui-textarea`) for notes and template files, a tree
(`tui-tree-widget`) for template structure — with two conditions: the crate
must build against the `ratatui` version in `Cargo.toml` (a widget from a
different ratatui does not implement our `Widget` trait at all; if it lags,
write the piece ourselves), and the version is pinned and named in the Phase
log. Do not add a crate for what is a few lines on top of ratatui: spinners,
popups, a line editor (`widgets::input::LineEdit` exists and is tested).

## Why this plan exists

fastf's daily surface was a guided *menu*: a sequence of dialoguer prompts — a
main menu, a paged list, an action menu, wizards. It could not show the library
and act on it at the same time, had no fuzzy anything, no multi-select, and
every screen was one prompt wide. A prototype on ratatui showed what one
full-screen dashboard over the real core feels like; it was prototype-grade
underneath (blocking discovery on the render thread, an 8 fps busy loop,
substring "fuzzy", no tests), so it became the visual spec and nothing else.

**Goal:** rebuild `src/tui/` on ratatui as one full-screen app that reaches
everything fastf can do — every menu item, every command-line-only capability,
and what the tool lacks: batch move/delete/tag with marks, a fuzzy command
palette, jump-to-project, live previews — with the engineering rules kept
(layering, locking, probes-never-`is_dir`, patch-vs-reload, the launcher
rules, the Windows clippy leg) and a test suite of the same seriousness as the
rest of the repo.

## Decisions (final)

1. **In place, one crate, one binary.** End state: ratatui only. `browser.rs`
   and `util::live_select` went in Phase 0; dialoguer stays for the CLI's
   inline prompts until Phase 6, when `prompt.rs`/`pickers.rs`/`vars.rs` are
   reimplemented on `Viewport::Inline` and dialoguer leaves `Cargo.toml`.
2. **`fastf` opens the new app from Phase 0.** Flows not yet native (create,
   register, templates, settings, the action menu) are reached through a
   *suspend bridge* — the screen is released, the dialoguer flow runs on the
   main screen, the app comes back — and each phase makes flows native and
   deletes their bridge variant. `fastf recent`/`search` open the same app.
3. **Elm-style.** `App` + `Msg` + `update(&mut App, Msg) -> Vec<Effect>`
   (no I/O) + `view(&App, &mut Frame)`. A `Runtime` owns the terminal, an
   input thread and worker threads (std, no tokio); the loop blocks on one
   channel and ticks only while something on screen is moving. A modal stack.
   **One command registry** drives keys, palette, help and hints.
4. **Fuzzy with `nucleo-matcher`**; the search bar keeps `core::query` for
   operators and scores bare words fuzzily, highlighted.
5. **Marks and batch jobs** (Phase 2): every verb acts on the marks if any.
6. **Reuse, never reimplement:** `SizeScanner`, `probe_dirs`, `index_summary`,
   `operations::*`, `plan_report`, `clipboard`, `reveal_folder`,
   `open_terminal_at`, the `LineEditor` (ported as `widgets::input::LineEdit`).
7. **The screen is stderr**; unix builds enable crossterm's `use-dev-tty`.
   `require_tty` runs before the screen is taken; `Runtime::init` is the
   `mark_interactive_surface` choke point that replaced `live_select`'s.
8. **The look:** a command centre — muted and cool, minimal and sophisticated
   (`src/tui/CLAUDE.md`, "The look"). The theme comes from the environment:
   `NO_COLOR`/dumb → mono, truecolor → the muted RGB palette, else ANSI-16
   used sparingly; ASCII glyphs on conhost or `FASTF_ASCII=1`.
9. **Tests:** pure `update` tests, `TestBackend`+`insta` snapshots, registry
   invariants, layering additions, the pty suite on a `vt100` replay.
10. **Delivery:** eight phases, one per session, each its own PR into `main`.

## Gates — every phase, before its PR

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings` — and `--release`
- `cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
- `cargo test --all-targets` (includes `tui_update`, `tui_commands`,
  `tui_snapshots` with committed snapshots, `tui_pty`, `layering`,
  `repo_hygiene`) and `cargo test --release`
- `cargo check --all-targets --target x86_64-pc-windows-msvc`
- `cargo +1.88.0 check --locked --all-targets` (MSRV)
- `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked`
- `ROADMAP.md` updated in the same commit; the matching `docs/` page when
  behaviour changed; the relevant `CLAUDE.md` only once the decision landed.

---

## Phase 0 — foundation ✔ (this branch)

**Goal.** The app exists, is the daily surface, and every old flow is still
reachable. `runtime.rs`, `command.rs`, `theme.rs`, `fuzzy.rs`, `App`/`update`
/`view`, the dashboard (header from the indexes, search bar with structured +
fuzzy matching, the table with a sticky viewport and live sizes, the detail
pane, the read-only template strip, status and hints), the palette, help, the
too-small guard; `o t y p i s S f F / R F5` native; `n e T , Enter` bridged.

- [x] `Cargo.toml`: `ratatui 0.30` (no default features), `nucleo-matcher`,
  `unicode-width`, unix `crossterm` with `use-dev-tty`; dev `insta`, `vt100`
- [x] `util::diag::set_sink` (a worker's warning reaches the app, not the
  alternate screen); `util::interrupt::raise`; `live_select.rs` and
  `browser.rs` deleted; `frame.rs` is the session ring only
- [x] `main.rs` → `tui::run(Entry::Menu)`; `cli::recent`/`cli::search` →
  `Entry::Recent`/`Entry::Search`
- [x] `tests/tui_update.rs` (27), `tests/tui_commands.rs` (7),
  `tests/tui_snapshots.rs` (9, snapshots committed), `tests/layering.rs`
  gains `ratatui_and_crossterm_stay_under_tui`
- [x] `tests/tui_pty/*` rewritten for the app (39): the traced assertions
  kept — one `discover` and zero `scan_base` on open over a fresh index, a
  tag patches its row, a delete drops its row, sizes land with no input
- [x] Gates: fmt, clippy ×3, `cargo test --all-targets`, `cargo test
  --release`, MSVC check, MSRV check, docs — all run on 2026-09-03
- [x] `docs/cli.md` "The guided app"; `ROADMAP.md`; `src/tui/CLAUDE.md`
  rewritten; root `CLAUDE.md` and `tests/CLAUDE.md` pointers
- [ ] Manual, needs the maintainer: run `fastf` in an 80×24 and a 120×40
  window; `fastf search tag:x`; `fastf </dev/null`; `NO_COLOR=1 fastf`; a
  launcher-started `fastf` still opens a window running the app
- Measured: see the Phase log.

## Phase 1 — single-project actions, metadata, journal

`app/actions.rs` with `ListChange`; the action-menu modal from the registry;
`a A Ctrl-T N Ctrl-N r m u D M J`; a single move as a one-item job with the
progress modal (`Arc<Mutex<Progress>>` + cancel flag from the runtime,
snapshotted per tick); `$EDITOR` under `Suspend`; the sort picker; delete
`src/tui/actions.rs` and the `ActionMenu` bridge; `validators.rs` for the
prompt texts and messages, kept verbatim.

- [x] update tests: each verb → `Action`; `Patched` + `ForgetSizes(stale)`;
  `Removed` clamps; typed-confirm mismatch → `name did not match — nothing
  deleted`; `y`/`n` answer a confirm without Enter
- [x] snapshots: `action_menu`, `delete_typed_confirm`, `metadata_view`,
  `journal_view`, `move_progress`
- [x] pty: the tag and delete tests native (`discover == 1`);
  `a_note_added_in_the_editor_is_appended` with a recorder `EDITOR`
- [x] docs/cli.md keys table; ROADMAP
- [x] Gates: fmt, clippy ×3 (debug, release, windows-gnu), `cargo test
  --all-targets`, `cargo test --release`, MSVC check, MSRV check, docs — all
  run on 2026-09-03
- [ ] Manual, needs the maintainer: a real move between two mounted bases with
  the progress modal, and the `$EDITOR` note flow in a real terminal
- Measured: see the Phase log.

## Phase 2 — marks and batch jobs ✔ (this branch)

`app/jobs.rs`, the job runner, `Space * -`, `targets()`, batch
confirm/progress/report modals, cancel mid-move, the debug-only
`move:force-staged` failpoint and `FASTF_FAULT` as a comma list.

- [x] update tests: marks, job construction in display order, per-item
  patching, cancel keeps failed items marked
- [x] snapshots: `batch_delete_confirm`, `job_progress`, `job_report_with_failures`
- [x] pty: `a_batch_move_reports_each_item`;
  `a_failed_move_surfaces_in_the_ui_and_leaves_the_list_consistent`
  (`FASTF_FAULT=move:force-staged,move:after-staging`)
- [x] docs/cli.md marks and batch paragraph; ROADMAP
- [x] Gates: fmt, clippy ×3 (debug, release, windows-gnu), `cargo test
  --all-targets`, `cargo test --release`, MSVC check, MSRV check, docs — all
  run on 2026-09-03
- [ ] Manual, needs the maintainer: a marked batch over a real library, and a
  cancel mid-batch-move on a real second volume
- Measured: see the Phase log.

## Phase 3 — new-project wizard, register, apply ✔ (this branch)

`app/wizard.rs`, `app/register.rs`, `widgets/{form,tree}.rs`; preview from
`project::plan_report`; post-create under `Suspend`; print-free
`cli::register::{plan_rename, recursive_targets}` extracted; delete
`menu_create/menu_register*/menu_apply` and their bridge variants.

- [x] update tests: step transitions; Esc at the answers → `Cancelled —
  nothing was created.` (Esc at the preview steps back — see the Phase log);
  `confirm_create=false` skips; validator messages on their own field
- [x] snapshots: `wizard_variables`, `wizard_preview`, `register_form`,
  `register_recursive_preview`, `apply_preview`
- [x] pty: the create/register/apply tests native, plus a create end to end
  and a real apply
- [x] docs/cli.md "The flows that build something" and the keys table;
  ROADMAP; `src/tui/CLAUDE.md`
- [x] Gates: fmt, clippy ×3 (debug, release, windows-gnu), `cargo test
  --all-targets`, `cargo test --release`, MSVC check, MSRV check, docs — all
  run on 2026-09-03
- [ ] Manual, needs the maintainer: a real create with post-create actions
  (`git init` / `$EDITOR`) on a real template, and a register of a folder that
  already holds a `PROJECT_INFO.md`

## Phase 4 — template studio, builder, from-folder ✔ (this branch)

`app/studio.rs`, `view/builder.rs`; the builder as sections with a live tree;
`cli::template::scan_for_preview` and `describe` extracted print-free; delete
`template_builder.rs`, `menu_templates`, `template_from_folder_flow`.

- [x] update tests: section state; `Cannot save:` on an invalid template; the
  studio's stale-read guard; from-folder previews before it writes
- [x] snapshots: `builder_review`, `builder_variable_form`,
  `builder_structure_tree`, `template_show`, `from_folder_preview`
- [x] pty: the builder tests native; `deleting_a_template_asks_first`; the
  from-folder slug refused on its own line; the caret test reads the app's own
  caret out of the frame
- [x] docs/templates.md "The builder"; docs/cli.md; ROADMAP;
  `src/tui/CLAUDE.md`
- [x] Gates: fmt, clippy ×3 (debug, release, windows-gnu), `cargo test
  --all-targets`, `cargo test --release`, MSVC check, MSRV check, docs — all
  run on 2026-09-03
- [ ] Manual, needs the maintainer: build a real template end to end and create
  a project from it; edit one of the gallery templates

## Phase 5 — settings, ID counter, maintenance, onboarding, needs-attention

`app/settings.rs`; a print-free `cli::config::apply` split from `set`; the
onboarding modal; `⚠ n needs attention` opens the reconcile report; delete
`menu.rs`.

- [ ] update tests: every field → key + message; bases by text; onboarding
  iff both base fields empty
- [ ] snapshots: `settings_basics`, `settings_bases_with_probe_notes`,
  `id_counter`, `onboarding`, `reconcile_report`
- [ ] pty: the settings/maintenance tests native;
  `first_run_asks_for_a_base_and_creates_it`

## Phase 6 — retire dialoguer

`inline.rs` on `Viewport::Inline` (stderr, `insert_before` for the transcript
line); `prompt.rs`/`pickers.rs`/`vars.rs` over it; `rows.rs` on
`unicode-width`; dialoguer removed; layering: delete the three dialoguer
rules, add `only_the_runtime_touches_the_terminal` and `dialoguer_is_gone`.

- [ ] the `LineEdit` tests cover the CLI prompts; inline select cancels on
  Esc/`q`; confirm answers bare `y`/`n`
- [ ] pty: `a_text_prompt_parks_a_visible_caret_after_the_text` rewritten;
  the ambiguity-picker and relaunch tests still green

## Phase 7 — polish and release ✔ (this branch)

Mouse (click row, wheel, click a palette entry); the ASCII alphabet pinned by a
snapshot; the two parking-lot settings retired; `docs/windows.md`, README hero,
`ROADMAP.md` release row; `Cargo.toml` at 3.0.0.

- [x] update tests: a click selects the row under it and moves focus to the
  pane it landed in; the wheel is `↑`/`↓` wherever they go; a click in the
  palette runs the entry under it; a click on nothing does nothing
- [x] snapshot: `dashboard_ascii_80x24`, asserting no decorative glyph survives
- [x] `show-banner`/`show-frame` retired (accepted and ignored);
  `recent-default-limit` renamed `recent-limit`, the old key still parsing
- [x] README hero, `docs/windows.md` (the old console, the mouse),
  `docs/cli.md`, root `CLAUDE.md`, `ROADMAP.md` release row and train
- [x] Gates: fmt, clippy ×3 (debug, release, windows-gnu), `cargo test
  --all-targets`, `cargo test --release`, MSVC check, MSRV check, docs — all
  run on 2026-09-03
- [ ] **The maintainer's**: merge the stack into `main`, then tag `v3.0.0`
  there (the Release workflow runs the whole of `ci.yml` before it builds), and
  bump both AUR packages on an Arch machine with the AUR key — see the
  `release` skill.
- [ ] Manual, needs the maintainer: the legacy Windows console pass for the
  ASCII alphabet, and the mouse in a terminal that reports it.

---

## Parking lot

- `reveal_folder` on unix blocks on `.status()` (already in ROADMAP); it runs
  on a worker now, so the app does not freeze, but the spawn still waits.
- The header's `newest` comes from the first base with any projects, as the
  old frame did; with the live snapshot it could be the true newest.
- ~~Mouse events are read and dropped until Phase 7.~~ Done in Phase 7.
- ~~`show-banner`/`show-frame` are inert; remove at v3.0.0.~~ Done in Phase 7.
- ~~`recent-default-limit` no longer sizes a page; rename to `recent-limit` at
  the next major, keeping the old key parsing.~~ Done in Phase 7.

## Phase log

- **Phase 0 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **`Terminal::clear` is never called.** In ratatui 0.30 it queries the
    cursor position and waits two seconds for an answer a pty under test
    never sends; a fresh `Terminal` needs no clear. `Terminal::resize` uses
    `clear_viewport`, which does not query, so a real resize is fine.
  - **The pty suite reads frames through `vt100`.** ratatui redraws only the
    cells that changed, so `1 of 1 projects` never appears contiguously in the
    transcript; `harness::app_screen` replays it into a 120×40 virtual
    terminal. `pty::run` now gives the child a real window size — a sizeless
    pty reports 0×0 and ratatui draws nothing.
  - **Column priority changed.** The old row put base and template before the
    date; the table adds size, date, base, template, tags in that order and
    never cuts the folder name (`view::projects::choose_columns`). With a
    detail pane beside the list, size and date are what the row is for.
  - **Names that trip `only_tui_prompt_prompts`.** `Sort::` and `LineInput::`
    matched the dialoguer-prompt grep; hence `Order` and `LineEdit`, and a
    check in `tui_commands.rs` that says so before CI.
  - **Onboarding** (`menu::onboard_first_run`) runs before the screen is
    taken, on the main screen, where the answer stays visible.
  - The palette ranks a title hit above a description hit; the hint bar leads
    with the context's own commands.
  - `validators.rs` was deferred to Phase 1: the app has no native text
    prompt yet.
  - **Fuzzy was too fuzzy** (review feedback on the first build): a word was
    matched as a subsequence of one string made of id, name, template, template
    name and tags, so `lrmx` found most rows. Now a word matches inside one
    field, substring first, and a fuzzy hit is accepted only when its
    characters span at most the word's length plus a third
    (`fuzzy.rs`, `library::match_fields`). Docs, help and placeholder reworded.
  - **The index could be a clock tick older than its base.** `write_cache`
    publishes by rename, which stamps the directory after the file; the coarse
    file clock made `cache_is_stale` read the base as newer every so often and
    the next discovery rescan for nothing — which the new pty test caught as a
    flake. `write_cache` now re-stamps the index after the rename.
  - **The header was too busy** (review feedback): the pulse chart, the
    `◈ ⬡ ⌂ ▲` counters, the `newest` line and the long search placeholder
    went. Two lines now — name and counts with the highest id on the right;
    the bases with `⚠ n needs attention` or the session ring on the right —
    and the table, strip and detail titles are plain words.
  - **The palette went muted** (review feedback: "a robust command center,
    not fancy tech colors"): steel blue for focus, slate grey for what
    recedes, amber/green/red only where they mean something, desaturated
    tag colours, bold kept for the name and the selected row. Recorded as
    "The look" in `src/tui/CLAUDE.md`.
  - **A screenshot tool** for the next phases: `tests/tui_pty/screenshot.rs`
    drives the real binary with `FASTF_SHOT_KEYS` and prints the frame, via
    a timestamped pty transcript (`pty::run_chunked`, `harness::screen_at`).
  - Measured on the maintainer's machine, 2026-09-03: the release binary is
    3.87 MB (4 056 544 bytes; v2.2.1 shipped under 4 MB, so the README's
    claim still holds) and a cold `cargo build --release` — every
    dependency from scratch, LTO, one codegen unit — took 26 s.
- **Phase 1 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **The caret test found a new home.** `prompt::text`'s only pre-filled
    (`initial`) instance reachable from the app was the rename prompt, which
    Phase 1 made native — so
    `a_text_prompt_parks_a_visible_caret_after_the_text` now drives the
    template builder's *Folder path* edit, the remaining dialoguer prompt that
    opens with editable text. It stays put until Phase 6 rewrites it for
    `Viewport::Inline` as planned.
  - **`y`/`n` and the modal keys.** The confirm answers a bare `y` or `n`
    without Enter, matching the old `confirm` prompt; Esc still cancels. The
    action menu opens on `Enter` and `a`; the hint bar shows the key that
    *starts* the gesture (`a actions`), not the older Enter.
  - **A running move turns every quit gesture into a cancel.** Ctrl-C did, by
    plan; it turned out `q` and Esc at the root would have quit under the
    worker mid-write. They now cancel too (`run`, after Esc has closed
    whatever dialog was open) — the move engine is crash-recoverable, but
    abandoning a live job on purpose is worse than letting it finish or stop.
  - **`patch` matches by id and `replace` is gone.** Phase 0 kept two row
    updaters — `patch` (same path) and `replace` (path changed) — and a verb
    that guessed wrong silently fell through to a rescan. Phase 1 unified them:
    `patch` finds the row by the frontmatter `id`, which is the one identity a
    rename or a move does not change, and clears the old path's bookkeeping
    (marks, metadata, sizes) when the row moved. `tests/tui_update.rs` pins a
    rename patching without a reload.
  - **The action menu title is the project's id**, not its name: the row the
    verbs act on is the folder *identity* the frontmatter carries, which a
    rename changes. The screenshot tool showed the name and it read wrong.
  - **The multi-pick's first row must be selectable.** `MultiPick::new`
    started `selected` at 1 when items existed, which put the highlight past
    the last row for a one-tag project and panicked the update test; the
    initial index is 0 like every other picker.
  - **The delete confirm keeps its text on a mismatch.** The typed name stays
    in the prompt and the error line says `name did not match — nothing
    deleted`, so one Backspace can fix a typo instead of retyping the whole
    name.
  - **A move's progress modal shows human bytes** (`1.1 MB of 3.3 MB`), not
    raw counts — the raw snapshot read like a bug.
  - Measured on the maintainer's machine, 2026-09-03: the pty tag/delete
    tests still trace exactly one `discover`, and the editor test's recorder
    `EDITOR` proves the scratch file is handed to the configured editor.
- **Phase 2 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **Marks and row bookkeeping were already seeded.** Phase 0 drew the mark
    glyph and the search bar's mark count, and `LibraryState` carried
    `marks` + `targets()` — there were simply no commands behind them.
    Phase 2 added `Space`/`*`/`-`, then found that a verb's "act on the
    marks" contract was already the natural reading of `targets()`: the
    single-project flows became the one-target case of the batch flows, and
    delete/unregister/move are the verbs that batch. Rename does not —
    every row would need its own name.
  - **A job is app-side sequencing over the single-action machinery.** The
    runtime already ran one `Action` per worker with a `busy`/`ActionDone`
    handshake, so a batch is just "send the next item when the last one's
    outcome lands", with per-item `ListChange` application. No new runtime
    path, no job worker — which is why `cancel mid-move` stayed the same
    `Effect::CancelMove` for the item in flight.
  - **A stale progress modal can trap the app.** The job's final advance
    armed `move_progress` for an item that never began, so after the job
    finished every quit gesture read "a move is running" and cancelled
    instead of quitting — the pty suite's deadline caught it as a hang.
    The modal is armed only when an item actually starts, and `job_finish`
    clears it defensively. This is the second trap of that shape (Phase 1's
    quit-keys fix); the two belong together under "quitting is a decision
    the running state can veto".
  - **The failure report doubles as the retry list.** Failed and unrun rows
    keep their marks because success drops a mark through the ordinary
    row-change path (delete removes the row, a move patches its path away).
    Closing the report therefore lands on exactly the rows that still need
    the verb, which the pty test asserts on disk and on screen.
  - **`move:force-staged` is a decision, not a crash** — the first failpoint
    answered by `is_armed` rather than `check`, and the invariant test now
    collects `is_armed` call sites too. A same-volume test reaches the
    staged path by arming it together with `move:after-staging`, one comma
    list, one run.
  - Measured on the maintainer's machine, 2026-09-03: the two new pty tests
    move real folders between sandbox bases — the batch one traces a single
    `discover`, and the forced-failure one leaves the source folder and the
    row's mark exactly where they started.
- **Phase 3 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **One shape, not three.** Create, apply and register are the same thing —
    answer a few questions, look at what that would do, say yes — so they are
    one `Flow` with a `Step`, one `Modal::Flow`, one `Effect::Preview` and one
    `Request` shared by the preview and the commit. `wizard.rs` holds the
    shape and the create/apply halves; `register.rs` holds the questions only
    register asks.
  - **The template is a field, not a picker that runs first.** `pick_template`
    then `pick_base` then the variables was three screens deep before anything
    was visible; the form opens on the configured default template (or the
    first) with every question already on it, and changing the template
    rebuilds the variable fields while keeping any answer whose variable the
    new template also has. The base is a field too, present only when more
    than one base is mounted — which is the early return
    `pick_base_interactively` used to make.
  - **Esc at the preview goes back to the answers**, not out of the flow.
    The plan's exit criterion said Esc at every step cancels, which is what a
    sequence of prompts had to do because there was nothing to go back *to*.
    A form has: one Esc returns to it with everything still typed, the second
    abandons the flow and says `Cancelled — nothing was created.` Nothing is
    created either way, which is what the criterion was protecting.
  - **A refusal is a message on a field.** `update` performs no I/O, so a path
    that must exist cannot be checked there. The preview worker refuses with
    `PreviewRefusal { field, error }`, `Form::fail` puts it on that field and
    moves the cursor to it, and the typed text stays. `apply` got register's
    wording for a missing folder (`no such folder: …`) rather than
    `require_real_directory`'s error chain ending in `os error 2`.
  - **`confirm_create = false` still builds the plan.** It commits without
    showing it (`Flow::auto_commit`), because skipping the build would skip
    every refusal the build produces.
  - **`ActionOutcome` gained `select` and `follow_up`**, and a builder, so
    adding a field stops touching every verb that never sets it. `select` is
    what puts the cursor on a project that did not exist when the action
    started; `follow_up` is how a finished create asks for its post-create
    actions on the main screen, since they run `git init`, the editor and the
    template's own commands.
  - **`E` applies a template to a folder.** It was a row in the bridged
    templates menu, which Phase 3 deletes; it is a command in the registry now,
    with the same fuzzy palette entry as everything else.
  - **Search stopped guessing at numbers and paths** (asked for alongside the
    phase): a word of digits is matched literally, because every folder name
    carries a date and `45` was finding the `4` and the `5` of `2026-04-15`; a
    word containing `/` is matched literally, because `c/A` fuzzed its way to
    every hierarchical tag. `fastf search` already matched bare terms as
    substrings, so the two surfaces now agree.
  - Measured on the maintainer's machine, 2026-09-03: the new pty cases create
    a real project from the wizard (and find it selected in the list it comes
    back to), register a whole base after previewing it, and apply a template
    to an existing folder.
- **Phase 4 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **`tui-textarea` did not earn its place, and could not.** Its current
    release (0.7.0) requires `ratatui 0.29`; adding it does not merely pull a
    second ratatui into the tree, it fails to resolve at all against ours.
    That is exactly the condition the plan set for a widget crate, so
    `widgets/text_area.rs` is ours: `LineEdit` with a second dimension, the
    cursor a char index on both axes, ~250 lines with its own tests.
  - **The builder is a list of parts, not a sequence of steps.** The old one
    walked six steps and then offered a review menu to go back into any of
    them — which is to say the review menu was the real interface and the
    walk was a tax. The list *is* the summary: every row says what that part
    holds, Enter opens it, Esc comes back, and Save says `Cannot save:` with
    `Template::validate`'s own words when the template is not yet loadable.
  - **`fastf template new`/`edit` open the app.** Deleting `template_builder.rs`
    would have left those two commands without an implementation. They open
    `Entry::Studio` instead, so the command line and `T` are the same editor.
  - **The structure section is where the text area earns itself**: one folder
    path per line with the tree drawn beside it, redrawn on every keystroke.
    Enter is a newline there, so Ctrl-S commits and the key line says so.
  - **A select's options are one comma-separated line**, not the old
    one-option-per-line loop. On a form the whole answer has to be visible and
    correctable, and three words are a line.
  - **`ListChange::SummaryOnly`.** Writing or deleting a template changes what
    the header and the strip say and moves no folder at all; `Reload` would
    have walked every base to answer a question none of the folders were
    asked. The studio's own list is refreshed from the landing summary, keeping
    its selection by slug.
  - **The caret test reads the caret, not the escape that moved it.** Phase 1
    left `a_text_prompt_parks_a_visible_caret_after_the_text` driving the
    dialoguer builder's *Folder path* edit, which this phase deletes. It now
    types into the wizard's own field and asserts the terminal's cursor
    position out of the `vt100` replay (`harness::app_cursor`) — the same
    guarantee, read the way a person reads it.
  - **A form field remembers whether it was touched**, which is how the slug
    keeps following the name until somebody types a slug of their own. The old
    builder could only offer the suggestion once, as a prompt default.
  - Measured on the maintainer's machine, 2026-09-03: the pty suite builds and
    saves a real template section by section, declares an empty `.gitkeep`,
    refuses an invalid one, deletes one after asking, and generates one from a
    folder after correcting its slug in place.
- **Phase 5 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **One screen, not seven submenus.** The plan's snapshot names imply groups;
    the groups are headings on one scrolling list rather than screens you enter.
    Every setting is visible with its value beside it, which is the thing the
    old menu could never do.
  - **`cli::config::apply` is the whole validator.** Splitting the print-free
    half out of `set` was the plan; what it bought is that `app/settings.rs`
    knows nothing about what is legal, so there is no second validator to
    drift, and the app's refusals are `config set`'s own words.
  - **A yes/no is answered where it stands**, and so is a two-way choice. The
    plan's "every field → key + message" reads as a dialog per field; opening
    one to answer a question with two answers spends a keystroke on nothing.
  - **`ActionOutcome::settings()`** re-reads the screen after a write, so it
    shows what is on disk rather than what was typed — a `~/Projects` that the
    config normalised to an absolute path shows as it was stored.
  - **`run_action` refuses while one is already running.** Found while writing
    the maintenance test: two Enters in a row started two actions, and because
    `on_action_done` drops anything that is not the `ActionId` in flight, the
    first outcome vanished silently — the row unpatched, the message never
    shown. The registry's `not_busy` guards the keys; this guards the screens
    whose rows are not commands.
  - **The first-run dialog stays up until the folder exists.** Popping it on
    Enter and reporting a failure as a status line would drop a first-time user
    onto an empty dashboard with an error and no question.
  - **`snapshots: settings_bases_with_probe_notes` became `settings_bases_as_text`.**
    The probe notes belong to the header, which already draws them per base; the
    thing worth pinning here is that the list is a text area.
  - **The pty settings tests move by row count.** They used to walk named
    submenus; a flat list is navigated by `down(n)`, so each script says which
    row it lands on and the maintenance one says how many selectable rows there
    are. Brittle in the same way the old `down(3)` was, and no more so.
  - Measured on the maintainer's machine, 2026-09-03: the pty suite runs a
    first run end to end (the folder is created and recorded), refuses a
    counter below the floor and then raises it, and edits the base list as
    text under a concurrent `config set`.
- **Phase 6 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **`Viewport::Inline` is the wrong tool, and the suite said so at once.**
    ratatui's inline viewport asks the terminal where the cursor is
    (`ESC [ 6 n`) and waits up to two seconds for an answer. A pty under test
    never sends one, so `cli_flags` failed on the first run with *the cursor
    position could not be read within a normal duration* — and a terminal that
    does not answer would put that stall in front of `fastf copy`, the command
    that exists to be instant. It is the same trap that already cost this
    codebase `Terminal::clear`. `inline.rs` reserves its rows by printing
    newlines and repaints with *move up n, draw*: every movement relative,
    nothing ever asked of the terminal.
  - **Colour is written as SGR from the theme's own `Style`**
    (`inline::paint_span`), so the command line's prompts use the app's palette
    and `NO_COLOR`, the ANSI sixteen and truecolor all behave identically on
    both surfaces. Twenty lines, and it is what "unify the design" actually
    required — a second theme would have drifted by the next phase.
  - **The picker stays unfilterable.** It is the picker a verb interrupted, over
    a list a query already narrowed, and its job is to be answered in one or two
    keystrokes. Typing to filter would take `q` away as a cancel and put a
    decision where there was a reflex. Fuzzy search lives in the app.
  - **Every prompt leaves one line of transcript** — the question and its
    answer, or the question and `cancelled`. dialoguer did this for a `Select`
    and not for the hand-rolled `confirm`; now it is one function.
  - **`prompt.rs` kept the contract and gave up the drawing.** It is the
    `require_tty` guard and `Ok(None)`-is-cancelled, over `inline`. The layering
    rule that used to grep for dialoguer type names is now
    `only_the_runtime_touches_the_terminal`: two modules take the terminal, and
    a third owner is two unsynchronised writers on one tty.
  - **`ProjectRowTheme` is gone.** It existed to reverse-video a whole row
    through dialoguer's theme trait; a list widget highlights a row by itself.
    `rows.rs` measures and pads with `unicode-width` and `view::fit` instead of
    `console`, and `cli::move_project`'s progress line asks crossterm for the
    width (the one thing `cli` may still ask a terminal — layering says so).
  - **`FASTF_SHOT_ARGS`** was added to the screenshot tool so the command
    line's prompts can be looked at the same way the app's screens are.
  - Measured on the maintainer's machine, 2026-09-03: the release binary is
    3.97 MB (4 161 720 bytes) with dialoguer and `console` gone and ratatui
    doing both surfaces — still under the 4 MB the README claims.
- **Phase 7 (2026-09-03).** Decisions taken while building, beyond the plan:
  - **The wheel needs no geometry.** It is `↑`/`↓`, three at a time, wherever
    the keys already go, so it is right in every list, every scrollable dialog
    and the detail pane without a second copy of the layout to drift from the
    first. A *click* does need to know what is under it, so it is answered only
    where `layout` already owns the geometry — the dashboard's regions, and the
    palette's centred box, which `palette_rows` computes either way. Anywhere
    else a click does nothing, which is better than a click that guesses.
  - **Mouse capture takes plain drag-to-select away**, which is the cost of the
    wheel working. Shift-drag still selects in every terminal that matters, and
    both `docs/cli.md` and `docs/windows.md` say so.
  - **The ASCII snapshot pins the alphabet, not the borders.** Box-drawing
    characters are ratatui's and are the one non-ASCII set the legacy console
    has always had; what has to go there is the decorative alphabet, and the
    test asserts each of those glyphs is absent rather than that the frame is
    ASCII.
  - **`show-banner`/`show-frame` are accepted and ignored** rather than
    refused. A config file or a script that still sets one must not start
    failing at a major version; `Config` has no `deny_unknown_fields`, so the
    file already parses, and `config set` now says the key is no longer used.
  - **`recent-default-limit` → `recent-limit`,** with the old key still
    parsing, as the Parking lot asked. The field on disk keeps its name: it is
    a serialized document, and renaming it would break every config that has
    one for no gain the user can see.
  - The Parking-lot items that stay: `reveal_folder` still blocks on `.status()`
    (on a worker, so nothing freezes); the header's `newest` still comes from
    the first base with any projects.
  - Measured on the maintainer's machine, 2026-09-03: the release binary is
    3.97 MB with dialoguer gone and both surfaces on ratatui, and the whole
    suite — 54 pty cases among them — is green in debug and release.
