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

## Phase 2 — Nothing crashes, nothing corners you  ·  status: **done**

- [x] **2.1** `view/builder.rs` Bases editor: `clamp(4, height - row)` panics
      (`min > max`) on any window 16–23 rows tall. Reproduced at 80×18/80×20.
- [x] **2.2** Three dead depth guards — `tree_size`, `transactions::scan_at`,
      `template_import::scan_dir_at` all recurse through the zero-initialising
      wrapper. Plus a real stack for the `size_scan` workers.
- [x] **2.3** Bound the `structure:` recursions on `MAX_WALK_DEPTH`.
- [x] **2.4** `TextArea` windows its line and its caret from two cursors.
- [x] **2.5** `Msg::TemplateSourceLoaded` lands on whatever builder is on top.
- [x] **2.6** Quit from the palette bypasses the dirty-builder question.
- [x] **2.7** Three scrolls that cannot reach the end.
- [x] **2.8** Dialogs measured at a width they may not get.
- [x] **2.9** Threads that die quietly; screen taken before the input thread.
- [x] **2.10** `u16` overflow in layout arithmetic.

Three more landed with them, all the same shape as something already on the
list: a click on the table's bottom border selected an undrawn row;
`Then::MoveToBase` asked `!marks.is_empty()` where every other verb asks
`batching()`; and the rename, delete and unregister dialogs re-read the
selection at submit time instead of carrying the project they named — which,
with a discovery landing underneath, deleted the neighbour of the project the
question was about. That one has a test that fails loudly on the old code.

`tests/tui_snapshots.rs`'s `settings_bases_as_text` had never opened the bases
editor: ten rows down is **Theme**, whose Enter cycles the value where it
stands. It asserts the editor is on screen now, so the snapshot named after a
screen is a snapshot of that screen.

## Phase 3 — Every surface says one thing  ·  status: **done**

CLI: the `recent_limit` key the file did not hold; Esc at the template picker as
an error; `register --recursive` reporting success over total failure;
`--limit 0` above the launcher hand-off; the editor's discarded exit status;
four sentences for "you cancelled"; two dead ends and a `✓` over a report that
could not inspect anything; a dead branch and a redundant config load.

App: `command::key_of` and `command::NO_TEMPLATES`, so a sentence that names a
key reads it from the registry; three dialogs that printed their key line twice;
a key line cut mid-word; Enter on a yes/no; the builder's lists clamping where
the registry says lists wrap; `Ctrl-K` missing from the base editor's line;
bytes and characters measured where columns were meant, `visible_window`
included; the template description that could not be read in the app; the
settings screen's fixed 26-column label; twelve hardcoded `✓`; an unheaded
warnings list.

**Not done, deliberately:** `?` still does nothing inside a create/apply/register
*form*. The form is a place you type into — on a choice field every letter is
`Ignored`, so falling through to the registry would make `q` (`Close`
everywhere) throw away a filled-in form with no question. The preview step has
nothing to type into, and `?` and `q` work there. The reason is a comment at
`on_flow_key`.

The two-layer flag validation (finding 3.4 — `register <path> --dry-run` is
clap's exit 2 while `register <path> --var=x --dry-run` is `validate`'s exit 1)
is deferred to Phase 4's documentation pass: the *code* is right in both cases,
and what is wrong is `CLAUDE.md`'s model of what `trailing_var_arg` does in
clap 4.6.

## Phase 4 — The record  ·  status: **done**

`ROADMAP.md`'s Current phase, which described v3.0.0 as unreleased while v3.2.0
was tagged. Doc drift: `preview-lines` documented nowhere, `copy-to` in neither
of the picker lists, a `from-folder` skip list of nine where the code has twelve
(read from the list itself now, so it cannot drift again), a README pinned to
`v3.2.0` and claiming a size the binary passed. The release archive's layout,
written in one place and read by two consumers and asserted by nobody — the
smoke job checks every path now. Bootstrap: a first run that failed between the
two bundled templates left the data dir permanently half-populated, and the
banner went to stdout, so it landed inside `$(fastf path …)`. The browser-UI
doc comments and the `swept` field left over from v2.0.0, two dead `pub`
functions, a comment pasted twice, `fs_retry`'s 256 where the constant is 64,
and two doc comments attached to the wrong item. `Cargo.toml` had no `exclude`,
so `cargo package` shipped the whole repository.

Three tests that could not fail now assert what they are named for, and
`tests/CLAUDE.md` has the three shapes written down. The `recent`/`tag reauto`
and unix-symlink coverage gaps are closed.

**Deferred, with the reasoning written down rather than the code changed:** the
AUR *source* package pins a checksum over GitHub's auto-generated tarball, which
is not byte-stable. `update.sh` re-checksums on every bump, so it can only ever
affect an already-published version; `packaging/aur/PUBLISHING.md` says what to
do when it happens. Restructuring the release to upload a stable source asset is
a bigger change than the exposure warrants.

## Phase 5 — Release  ·  status: not started

The `release` skill: version, tag on a green PR commit, Release workflow, both
AUR packages, `.github/release-notes/v3.3.0.md`. A minor — no flag, key spelling
or schema changes.
