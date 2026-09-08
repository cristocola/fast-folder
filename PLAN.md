# PLAN.md — v3.3.0, the hardening and polish pass

Working file for the release. One phase per session, each landing as its own PR
into `main`. Delete this file when v3.3.0 is tagged, as `ROADMAP.md` is then the
record.

Every gate was green when this started: fmt, clippy `--all-targets` in debug and
release, `cargo check --target x86_64-pc-windows-gnu`, `cargo test
--all-targets`, `cargo test --release`. No `TODO`/`FIXME`/`HACK`/`XXX` anywhere.
So nothing here is a failing gate; it is what a green gate cannot see — a guard
that is written down and dead, an error read as a default, a panic one row below
the size anyone tests at, and a message that is never printed.

**The theme is silence.** Every fix below is a place fastf did the wrong thing,
or nothing, and said so nowhere.

## Phase 1 — Nothing vanishes  ·  status: **done**

The library never silently loses a project, and the counter never hands out a
number twice.

- [x] **1.1** `read_project_meta` warns when a `PROJECT_INFO.md` has frontmatter
      that will not parse, instead of dropping the project in silence; and
      `Metadata`'s non-identity fields (`template_name`, `created`, `folder`,
      `path`) take `#[serde(default)]` so a hand-edit stops being fatal at all.
      `tests/hostile_fs.rs:96` currently pins the wrong behaviour — amend it.
- [x] **1.2** `Counters::floor` warns and skips on an unreadable data-dir
      counter rather than reading it as `0`, the way `Counters::propagate`
      already does.
- [x] **1.3** `read_base_readonly` abandons a cache holding a rejected entry and
      rescans, the way `discover_base` does. It feeds `max_id` → the ID floor.
- [x] **1.4** `append_journal_entry` writes at the end of the **journal
      section**, not the end of the file, so a note after a user's own `##`
      heading is still readable.
- [x] **1.5** `reconcile` sees a `.<name>.fastf-case` staging folder left by a
      hard kill mid case-only rename. It was the one multi-step mutation with no
      recovery story.
- [x] **1.6** Tests for two invariants that had none: `backfill_id_numbers` and
      `on_name_collision = "error"`.

Two more turned up while fixing 1.2 and both landed with it: `cli::id`'s
`print_counter` rendered "the maximum, 999999999999, is reached" whenever the
counter file could not be *read* — three facts sharing two branches — and the
read failure was reported once per call rather than once per process, so one
broken file said so three times in one command.

Phase 2 begins at `src/tui/view/builder.rs:617`. The repro is in the plan's
verification section and takes one command.

## Phase 2 — Nothing crashes, nothing corners you  ·  status: not started

- [ ] **2.1** `view/builder.rs` Bases editor: `clamp(4, height - row)` panics
      (`min > max`) on any window 16–23 rows tall. Reproduced at 80×18/80×20.
- [ ] **2.2** Three dead depth guards — `tree_size`, `transactions::scan_at`,
      `template_import::scan_dir_at` all recurse through the zero-initialising
      wrapper. Plus a real stack for the `size_scan` workers.
- [ ] **2.3** Bound the `structure:` recursions on `MAX_WALK_DEPTH`.
- [ ] **2.4** `TextArea` windows its line and its caret from two cursors.
- [ ] **2.5** `Msg::TemplateSourceLoaded` lands on whatever builder is on top.
- [ ] **2.6** Quit from the palette bypasses the dirty-builder question.
- [ ] **2.7** Three scrolls that cannot reach the end.
- [ ] **2.8** Dialogs measured at a width they may not get.
- [ ] **2.9** Threads that die quietly; screen taken before the input thread.
- [ ] **2.10** `u16` overflow in layout arithmetic.

## Phase 3 — Every surface says one thing  ·  status: not started

CLI: the `recent_limit` key the file does not hold; Esc at the template picker
as an error; `register --recursive` reporting success over total failure; two
answers for one flag mistake; `--limit 0` above the launcher hand-off; the
editor's discarded exit status; four sentences for "you cancelled"; two dead
ends and a `✓` over an empty reconcile.

App: eight places drifted out of the one registry; keys that do nothing and say
nothing; bytes measured where columns are meant; a template description that
cannot be read in the app; twelve hardcoded `✓`.

## Phase 4 — The record  ·  status: not started

`ROADMAP.md`'s Current phase (two releases stale); doc drift; the release
archive layout nothing asserts; bootstrap's half-populated data dir and its
banner on stdout; the browser-UI doc comments and dead `pub` API left from
v2.0.0; three tests that cannot fail.

## Phase 5 — Release  ·  status: not started

The `release` skill: version, tag on a green PR commit, Release workflow, both
AUR packages, `.github/release-notes/v3.3.0.md`. A minor — no flag, key spelling
or schema changes.
