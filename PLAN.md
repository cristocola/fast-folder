# The 3.7 freeze

One cleanup pass, five phases, one PR each. v3.7.0 changes no flag, no config
key and no file format; it tidies the record of the work that led here. The
full plan with the audit findings behind each item is the maintainer's; this
file is the hand-off between sessions.

Principles for every rewrite:

- A user doc describes the tool as it is today: no release numbers, no "used
  to". The only history a user needs is a compatibility note about a file they
  may still have, one sentence each.
- An agent doc keeps every rule and its one-sentence why; the story of how it
  was found, the PR numbers and the before-state go. Source comments that
  explain a past defect stay — that is the house style in code.
- Shipped work is recorded in `.github/release-notes/` and the releases page,
  nowhere else.
- Every phase ends with the full gate set green.

## Phase 1 — user documentation ✔ (this PR)

`docs/` is six guides, one subject each: `app.md` (the guided app, split out
of `cli.md`), `cli.md` (commands only), `config.md` (settings, environment,
data locations, the ID counter — new), `templates.md`, `projects.md` (gained
"What fastf promises", from the ROADMAP's product contract), `windows.md`
(lost the developer-only live-test section, which moves to `tests/CLAUDE.md`
in phase 3). README trimmed to a summary that links out. Corrections: F5 /
Ctrl-R reload versus `R` reindex; five remembered things in `state.toml`;
`structure[].children` documented; `motion` added to `config set --help`
(`src/main.rs`). Release archaeology purged from every user doc.

## Phase 2 — the release record and the release routine

- `ROADMAP.md` → only what is open: the outstanding manual passes, the one
  unchecked smoke item, the backlog. Delete "Current phase", the release
  train, both regression checklists and the gates list.
- One release routine: `.claude/skills/release/SKILL.md` is the routine,
  `packaging/aur/PUBLISHING.md` the AUR mechanics only (one-time setup, the
  commands, the tarball-checksum-drift note). Remove from PUBLISHING.md what
  the skill also says; use `$FASTF_AUR_DIR` like `update.sh`. In the skill fix
  the "as executed for 1.1.1" heading, "five patterns" → six, "~150 lines",
  the docs list (six files). One line on the release-notes convention: a file
  per v3+ tag, none earlier, a missing file is fine.
- Both `packaging/aur/*/PKGBUILD` `# Maintainer:` lines → `hello@argyrolabs.com`
  (`.SRCINFO` does not carry the comment; confirm with a diff). Add to the
  hygiene rule in the root `CLAUDE.md`: no personal email in a tracked file.
- Verify: `cargo fmt --check`, `cargo test --test repo_hygiene`.

## Phase 3 — the four CLAUDE.md files

Rewrite each as current state. Targets: root 503→~330, core 748→~480, tui
1321→~650, tests 180→~140.

- Root: module lists gain `app/pane.rs`, `tui/motion.rs`, `tui/guide.rs`,
  `util/term_open.rs`; "three CLAUDE.md files" → four; retired keys are three
  (`mouse`); the test-harness paragraphs (processes not threads, one env
  guard, lock order, HOME) become one pointer to `tests/CLAUDE.md`; history
  blocks go.
- Core: `ActionLoop` no longer exists; the builder's reserved-name refusal
  does not offer `NOTES.md`; collapse each history passage into the rule it
  guards; the Windows-stack / `MAX_WALK_DEPTH` note lives here only.
- TUI: dissolve "What the consolidation pass added" into the topic sections;
  fold "Every flow is native, and dialoguer is gone" into the runtime section;
  one cursor-query paragraph, not two; no "strip" (it is gone — orphan slugs
  are on the templates tab); `needs_tick` and `tick_interval` both exist;
  `Glyphs::spin(elapsed_ms)`; `DELETE_MISMATCH` reads "type delete to confirm
  — nothing deleted"; drop the PR numbers; the testing section is a pointer;
  name the six keys the file never mentions (`R`, `C`, `p`, `i`, `*`, `-`).
- Tests: add `term_cmd.rs` and its recorder rule; add the `windows_live.rs`
  invocation (from the old `docs/windows.md`); keep "Three ways a test passes
  over the thing it is for", two sentences per example.
- Verify: `cargo test --test layering --test repo_hygiene`, then grep every
  backticked identifier in the four files against `src/` + `tests/`.

## Phase 4 — source: dead weight and the two oversized files

- Delete `Metadata::from_plan` (`src/core/project_info.rs`),
  `layout::studio_rows`, `App::selected_detail` — no callers.
- Remove the 13 `#[allow(dead_code)]` in `tests/tui_pty/harness.rs`.
- `AssetJob::last_progress_at` stays (its journal is `deny_unknown_fields`);
  cut its comment to two lines.
- Trim the UUID / "interim build" prose in `src/core/naming.rs`; fix the
  dangling `crate::tui::browser` example in `tests/layering.rs`; halve the
  `//!` header of `src/tui/motion.rs`.
- Move `config_ignores_removed_project_info_keys` and
  `config_defaults_are_backwards_compatible` from `tests/create.rs` to
  `tests/data_dir.rs`.
- Split `tests/tui_update.rs` into `tests/tui_update/` along its existing
  inline modules and banner comments; shared helpers in `harness.rs`; one
  binary as before.
- Split `src/tui/app/mod.rs` by moving `impl App` blocks into the flow
  modules that already exist (`wizard`, `register`, `studio`, `settings`,
  `pane`, `jobs`, `palette`); pure moves, `pub(super)` where a call crosses a
  file; `mod.rs` keeps the types, `new`/`start`, dispatch, status, discovery.
- Verify: fmt, clippy debug + release, clippy on `x86_64-pc-windows-gnu`,
  `cargo test` debug + release, snapshots unchanged, two frames screenshotted.

## Phase 5 — v3.7.0

`Cargo.toml` → 3.7.0; `.github/release-notes/v3.7.0.md`; green PR run on
both platforms; tag on `main`; `packaging/aur/update.sh 3.7.0`; `makepkg -f`
both; push both AUR clones; commit the packaging bump; delete this file.

## Not doing

The screenshot tool stays in the pty binary; ~40 over-exposed `pub` items
stay `pub`; no `cargo` ecosystem in dependabot; no release notes for pre-v3
tags; the git history and the repository's visibility are unchanged.
