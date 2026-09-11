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

## Phase 1 — user documentation ✔ (#75)

`docs/` is six guides, one subject each: `app.md` (the guided app, split out
of `cli.md`), `cli.md` (commands only), `config.md` (settings, environment,
data locations, the ID counter — new), `templates.md`, `projects.md` (gained
"What fastf promises", from the ROADMAP's product contract), `windows.md`
(lost the developer-only live-test section, which moves to `tests/CLAUDE.md`
in phase 3). README trimmed to a summary that links out. Corrections: F5 /
Ctrl-R reload versus `R` reindex; five remembered things in `state.toml`;
`structure[].children` documented; `motion` added to `config set --help`
(`src/main.rs`). Release archaeology purged from every user doc.

## Phase 2 — the release record and the release routine ✔ (#76)

`ROADMAP.md` is the manual passes (the Windows Reveal/open/term item folded
in), the backlog and its two sub-lists, and a pointer to the release notes;
the product contract's one unported sentence (compatibility within a major)
went to `docs/projects.md`. The skill is the routine — bump + `Cargo.lock` +
release notes, green PR, merge, tag, AUR — and now carries the gates list
and all six failure patterns in one table; `PUBLISHING.md` is setup, the
commands over `$FASTF_AUR_DIR`, and checksum drift. Both PKGBUILDs say
`hello@argyrolabs.com` (`.SRCINFO` diffed unchanged). The email rule is
enforced, not just written: `tests/repo_hygiene.rs` flags any address but
that one and the reserved example domains, with a unit test for the matcher.
Pointers to deleted ROADMAP sections were fixed in root `CLAUDE.md` (gates →
skill, and the hygiene line), `src/core/CLAUDE.md` (threat model →
`docs/projects.md`), `tests/CLAUDE.md` and `tests/windows_live.rs`.

## Phase 3 — the four CLAUDE.md files ✔ (#77)

All four rewritten as current state, 2 750 → 1 779 lines: root 385, core
493, tui 761, tests 140. Every rule kept with its one-line why; PR numbers,
release numbers and before-states gone. Root: the module lists carry
`copy_engine.rs`, `body.rs`, `term_open`, `guide.rs`, `motion.rs`,
`app/pane`; four CLAUDE.md files; three retired keys; the harness rules are
one pointer. Core: no `ActionLoop`; the builder refuses the reserved name
without offering an example; walk depth and the Windows stack live here.
TUI: the consolidation section dissolved into Layout, the runtime, marks and
batches, two tabs; one cursor-query rule; no strip; both tick functions;
`spin(elapsed_ms)`; the real `DELETE_MISMATCH`; the six keys named. Tests:
`term_cmd.rs`, the `windows_live.rs` invocation (without `--test-threads=1`,
which the harness rule forbids and the per-case `live-…` folders make
unnecessary), the three ways at two sentences each. Verified: layering and
repo_hygiene pass, and a script extracting every backticked identifier finds
all of them in `src/`/`tests/` except `IndexMap`, `RUSTFLAGS`, `_unlocked`
and serde's `ContentDeserializer`, which are external or a suffix.

## Phase 4 — source: dead weight and the two oversized files ✔ (#78)

`Metadata::from_plan`, `layout::studio_rows`, `App::selected_detail` and the
thirteen `#[allow(dead_code)]` in the pty harness are gone. `last_progress_at`
stays with a two-line comment, but not for the reason written here: it is a
field of `Progress`, which is `Serialize` only, so no journal reads it — it is
kept as the field a "no progress" note would read. The UUID prose, the
`crate::tui::browser` example and half of `motion.rs`'s header went; the two
config-compatibility tests live in `data_dir.rs`. `tests/tui_update.rs` is one
binary with twenty modules under `tests/tui_update/` and a `harness.rs`
prelude (170 tests before and after). `src/tui/app/mod.rs` is 4 681 → 2 347
lines: 92 methods moved by script, byte for byte, into `pane`, `palette`,
`wizard`, `register`, `studio`, `settings`, `jobs`, `actions` and `modal` (the
guide's methods beside `GuideState`), 54 of them now `pub(super)` because a
call crosses a file. Verified: fmt; clippy debug, release and
`x86_64-pc-windows-gnu`; `cargo test` debug and release; the doc gate from a
clean `target/doc`; no snapshot changed; the dashboard and builder frames from
the screenshot tool identical to the pre-split binary's.

## Phase 5 — v3.7.0

`Cargo.toml` → 3.7.0; `.github/release-notes/v3.7.0.md`; green PR run on
both platforms; tag on `main`; `packaging/aur/update.sh 3.7.0`; `makepkg -f`
both; push both AUR clones; commit the packaging bump; delete this file.

Added at the maintainer's request during the pass, and shipped in this
release as its one user-visible change: the detail pane wraps a note or a
todo too wide for it (`pane::wrap_columns`, in display columns) instead of
cutting it with `…`, and a todo toggle names the todo's text from the detail
rather than from its row. One new snapshot
(`detail_pane_wraps_a_long_note_and_todo_120x40`); every other frame is
unchanged. The release PR carries it with the version bump and the notes.

## Not doing

The screenshot tool stays in the pty binary; ~40 over-exposed `pub` items
stay `pub`; no `cargo` ecosystem in dependabot; no release notes for pre-v3
tags; the git history and the repository's visibility are unchanged.
