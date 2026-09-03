# PLAN.md — v3.0.0: the guided app on ratatui

> **In progress.** Phase 0 landed on `ratatui/phase-0-foundation`. This file
> is worked one phase per session: read it, do phase N, run the gates, tick
> only the boxes whose named verification actually ran, each phase in its own
> PR into `main` with `ROADMAP.md` and the matching `docs/` page in the same
> commit, and findings outside the phase sent to the Parking lot rather than
> fixed in passing. The **Phase log** at the bottom says what differed and why.

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
8. **Theme** from the environment: `NO_COLOR`/dumb → mono, truecolor →
   rich, else ANSI-16; ASCII glyphs on conhost or `FASTF_ASCII=1`.
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

- [ ] update tests: each verb → `Action`; `Patched` + `ForgetSizes(stale)`;
  `Removed` clamps; typed-confirm mismatch → `name did not match — nothing
  deleted`; `y`/`n` answer a confirm without Enter
- [ ] snapshots: `action_menu`, `delete_typed_confirm`, `metadata_view`,
  `journal_view`, `move_progress`
- [ ] pty: the tag and delete tests native (`discover == 1`);
  `a_note_added_in_the_editor_is_appended` with a recorder `EDITOR`
- [ ] docs/cli.md keys table; ROADMAP

## Phase 2 — marks and batch jobs

`app/jobs.rs`, the job runner, `Space * -`, `targets()`, batch
confirm/progress/report modals, cancel mid-move, the debug-only
`move:force-staged` failpoint and `FASTF_FAULT` as a comma list.

- [ ] update tests: marks, job construction in display order, per-item
  patching, cancel keeps failed items marked
- [ ] snapshots: `batch_delete_confirm`, `job_progress`, `job_report_with_failures`
- [ ] pty: `a_batch_move_reports_each_item`;
  `a_failed_move_surfaces_in_the_ui_and_leaves_the_list_consistent`
  (`FASTF_FAULT=move:force-staged,move:after-staging`)

## Phase 3 — new-project wizard, register, apply

`app/wizard.rs`, `app/register.rs`, `widgets/{form,tree}.rs`; preview from
`project::plan_report`; post-create under `Suspend`; print-free
`cli::register::{preview_rename, recursive_targets}` extracted; delete
`menu_create/menu_register*/menu_apply` and their bridge variants.

- [ ] update tests: step transitions; Esc at every step → `Cancelled —
  nothing was created.`; `confirm_create=false` skips; validator messages
- [ ] snapshots: `wizard_variables`, `wizard_preview`, `register_form`,
  `register_recursive_preview`, `apply_preview`
- [ ] pty: the create/register/apply tests native

## Phase 4 — template studio, builder, from-folder

`app/studio.rs`, `view/{templates,builder}.rs`; the builder as sections with
a live tree; `cli::template::scan_source` `pub(crate)`; delete
`template_builder.rs`, `menu_templates`, `template_from_folder_flow`.

- [ ] update tests: section state; `Cannot save:` on an invalid template;
  bundle confirm gating
- [ ] snapshots: `builder_review`, `builder_variable_form`, `template_show`,
  `from_folder_preview`
- [ ] pty: the builder tests native; `deleting_a_template_asks_first`

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

## Phase 7 — polish and release

Mouse (click row, wheel, click a palette entry); ASCII glyphs checked on
conhost; `docs/windows.md`, README hero, `ROADMAP.md` release row; v3.0.0 via
the `release` skill.

---

## Parking lot

- `reveal_folder` on unix blocks on `.status()` (already in ROADMAP); it runs
  on a worker now, so the app does not freeze, but the spawn still waits.
- `show-banner`/`show-frame` are inert; remove at v3.0.0.
- `recent-default-limit` no longer sizes a page; rename to `recent-limit` at
  the next major, keeping the old key parsing.
- The header's `newest` comes from the first base with any projects, as the
  old frame did; with the live snapshot it could be the true newest.
- Mouse events are read and dropped until Phase 7.

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
    date; the table adds date, size, base, template, tags in that order and
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
  - Measured on the maintainer's machine, 2026-09-03: the release binary is
    3.87 MB (4 056 544 bytes; v2.2.1 shipped under 4 MB, so the README's
    claim still holds) and a cold `cargo build --release` — every
    dependency from scratch, LTO, one codegen unit — took 26 s.
