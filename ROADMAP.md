# fastf robustness roadmap

The release train, the gates every release must pass, and what is not built
yet. Each release's detailed guarantees live in `CLAUDE.md` (current design) and
the test suite (enforced); this file tracks what shipped when, what to run
before tagging the next one, and what is still open.

## Product contract

fastf is a local, single-user project scaffolder for self-contained trees of
ordinary directories and regular files. It has two surfaces, the CLI and the
guided TUI, and **no network surface at all**; fastf commands may wait behind one
coarse mutation lock. It does not coordinate simultaneous writers on multiple
computers.

A move first asks the operating system to rename the directory. Only a standard
cross-device failure switches to fastf's internal Rust copy path. That path
copies the directory topology and every regular file, checks source and staged
file paths and byte lengths, publishes the complete staging directory, and only
then attempts to remove the source. The project must remain untouched by other
programs while it is moving.

### What fastf may start

fastf spawns programs on the user's behalf in five places, all of them
configuration rather than input: a template's `post_create` commands, the
editor, the file manager for Reveal, a clipboard tool
(`wl-copy`/`xclip`/`xsel`/`clip`/`pbcopy`), and — new in v2.1.0, unix only — a
terminal emulator plus `notify-send`. The emulator is named by the `terminal`
config key, else `$TERMINAL`, else `xdg-terminal-exec`, else the first known
emulator on `PATH`; it is started only when fastf has been asked for something
interactive and can prove nothing can read its output, and it is given the
process's own argv as argv, never through a shell.

### What fastf trusts

One OS account. Bases, templates, `config.toml`, the counters and the caches are
the user's own files and are trusted as content — a template's `post_create`
commands are executable configuration and run with the user's privileges, which
is the feature. There is **no network surface** of any kind.

Two things fastf enforces anyway, because they are the routes by which a file
that travels can start naming somewhere it should not:

- **A cache entry can only ever point at a direct child of its own base.**
  `.fastf-index.json` travels with the projects by design (that is what makes it
  portable across operating systems), so a synced folder or an unpacked archive
  can deliver one. An entry naming anything else is rejected, the cache is
  abandoned, and the base is rescanned from the folders. `fastf open` and the
  TUI's Reveal check the folder is a real direct child holding a
  `PROJECT_INFO.md` before handing the path to the system file manager.
- **A write never follows a link.** See below.

A write fastf performs beneath a root it controls — a new project, an apply
target, a template's `files/` — never follows a link, junction or reparse point
that is already there. Both layers are enforced: the path text cannot escape its
root, and the filesystem beneath it is checked component by component
immediately before each write. This is not a defence against another process
rewriting the tree concurrently; it is one user's own filesystem.

The contract deliberately does not include hashes, ACLs, extended attributes,
sparse-file layout, hard-link relationships, symlink/junction reproduction, or
storage-level durability. Links and special entries are rejected when copying
would be required. Process-crash recovery is in scope for the v2 journals below;
hardware failure, power loss, bit rot, and storage corruption remain the
responsibility of the filesystem and backups.

## Current phase

- Released in v3.6.1: **the app never takes the mouse.** v3.6.0 made
  capture a setting, off by default; the click handling is gone now, and with
  it the setting. No mouse mode is switched on, so text selects as in any
  program, and the wheel is the terminal's arrow keys on the alternate screen.
  `mouse` is a retired `config set` key, accepted and ignored.
- Released in v3.6.0: **the pane reads the file, and motion guides the eye.**
  **Notes are the journal**: a note is a dated entry under `## Notes` with
  every further line indented under its first, so a message from stdin, the
  editor or the quick note keeps all of its lines — the old writer put the
  whole message on one line and the reader dropped everything after it. The
  reader is lenient (a column-0 list item starting with a date is an entry
  with or without the `—`, every line up to the next belongs to it, text
  above the first entry is one undated note, a heading is matched in any
  case), a file from before 3.6.0 keeps its `## Journal`, and `core/body.rs`
  is the one grammar for the body's sections. **Todos** live under `## Todo`,
  toggled by rewriting the one character inside the brackets. **The pane
  shows a note as its own rows** and the todos as a list Enter toggles, every
  section ending in an add row; **a cached detail carries a stamp** and is
  checked against the disk on every visit, on F5 and once a second, so a file
  edited in another window shows unasked. **The mouse is a setting, off by
  default**, so text selects as in any program. **Motion fades rather than
  flashes**: a pulse mixes toward the palette's ground, the focus eases
  between its rest states, a message arrives under a wash, and a sort pulses
  the row that kept the selection.
- Released in v3.6.0: **the app feels deliberate.** Four things, each about the cursor
  going where you meant and nothing happening that you did not ask for. **A
  list stops at its ends** — every list wrapped through one shared helper, and
  one `j` too many at the bottom of a long table put the cursor back at the
  top with nothing to say why. **The horizontal axis is focus** — `→` was
  "whatever Enter does here" and `←` "whatever Esc does", two arrows that ran
  verbs and closed dialogs; they only move the cursor between the list and
  the pane now, on both tabs, hidden where there is nowhere to go, and the
  pane the focus arrives in pulses once. **The detail pane is an editor you
  enter on purpose** — a cursor over the rows Enter can act on; Enter on the
  name renames, on a tag edits it (emptied, removes it), on a variable edits
  the value in place or picks from a `select`'s options and nothing else, on
  the notes opens the section as a text area with `Ctrl-S` to save; Esc leaves
  the row as it was; a refusal lands under the field with the text still there.
  **What the pane admits is what the file can hold**: a tag is one word
  (`validated::Tag`, at the one door every tag comes through), a variable
  lands through the template's own validation and transform, a note may not
  begin a line with `##`; setting a variable rewrites the frontmatter, its
  derived tag and the body's table in one atomic write, and the table only
  while it is still the one fastf wrote.
- Released, unversioned: **two things that looked like faults.** The size pulse fired on
  arrival, and every visible row's size arrives at once — so the first
  screenful of a run washed twenty rows together, and so did every scroll after
  it. A pulse is a cue; a page lighting up is a flash. It answers nothing there
  either, because the table is measured from the rows and never from the sizes,
  so a landing number cannot reflow anything: it now pulses on a number that
  replaced a *different* number, and on a size a verb threw away coming back.
  And `tag reauto` removed every tag under a `tag_from` slug's namespace, which
  is wider than the set it derived — a template's own `tags: ["tier/legacy"]`
  and a `tier/manual` somebody typed both matched, and a command whose job is to
  refresh the derived tags deleted them. `PROJECT_INFO.md` records which tags
  were derived (`auto_tags`), so re-deriving replaces exactly those; a project
  written before the record reconstructs what it can from its own variables,
  and nothing is migrated.
- Released: **v3.5.0 — the app answers the keys you try.** The terminal app had
  a key for everything and a grammar for nothing: some lists paged and some did
  not, `g` meant "first row" on one screen and "template from a folder" on the
  next, `←`/`→` did nothing anywhere, and a batch that changed ten rows left a
  frame identical to the one before it. Five phases, each its own PR (#60–#64):
  - **One movement grammar** over every list — the arrows, the page keys, half
    a page (`Ctrl-D`/`Ctrl-U`) and the jumps to the ends, declared over one
    context set instead of three that had drifted. `g`/`G` are the ends
    everywhere; the templates tab's two verbs moved to `I` and `H`.
  - **A horizontal axis that means one thing**: `→`/`l` goes into whatever is
    under the cursor, `←`/`h` comes back out one level, and it never quits.
  - **Every key line read from the registry.** Seven surfaces wrote `↑↓` by
    hand; `Context::Prompt` and `Context::Pick` exist so the hint bar can read
    a prompt's and a picker's keys, and `command::keys_in` takes back out what
    a text field swallows — `? help` over a rename prompt was the registry
    telling a lie about itself. Ctrl-C is a declared command now.
  - **Four things the app could not do**: `v` marks to the cursor, every
    one-way sort runs both ways, `FilterTag` writes the grammar's own clause,
    and `/` narrows the settings screen.
  - **Motion, only where it answers a question**: a row a verb changed, a size
    cell as it lands, one activity indicator, a message on its way out.
    Nothing else moves. It also fixed a real defect — a tick was a quiet
    interval rather than a deadline, so a burst of work stopped the spinner
    exactly when there was most to wait for.

  `.github/release-notes/v3.5.0.md` is the user-facing account. No flag, config
  format or template format changed; `state.toml` records a sort direction and
  older files still parse, and there is one new setting, `motion`.
- Released: **v3.4.0 — the template editor explains itself.** The one surface
  that asked people to learn a vocabulary before they could use it: five nouns
  in the manifest's own words, and a single footer line cut with an ellipsis as
  the whole teaching budget behind them. Three things, in one PR (#57):
  - **An explanation panel** beside the builder's list and every form in it:
    what the highlighted part is, and what this template would produce *right
    now* — the folder name a project would get, the first two IDs, the tree its
    folders make. On by default, `i` hides it and the choice is remembered, and
    a window too narrow for both keeps the list and today's footer exactly.
  - **A seven-page guide** (`G`, the palette, or offered once unasked the first
    time templates come up at all), written for somebody who has never opened
    the TUI and ending in a walkthrough that builds a real template from
    nothing. Opened from a part of the editor it lands on that part's page.
  - **A coach**: the Save row counts what is still worth a look and the panel
    names it. Advice and never a refusal — `Template::validate` and
    `operations::save_template` keep all of the authority, and every gap it
    names is a template that loads and saves.

  `tui::guide` is to explanations what `command.rs` is to keys: the one place
  any of them are written, with a test that every key in its prose comes from
  the registry and another that no character the theme owns an ASCII spelling
  for is written into it. `.github/release-notes/v3.4.0.md` is the user-facing
  account. Nothing changed about a flag, a config key or a file format;
  `state.toml` gains two remembered preferences and older files still parse.
- Released: **v3.3.0 — the hardening and polish pass.** Feature work reached
  a wall at v3.2.0 with every gate green and no `TODO` anywhere, so this release
  spends itself on what a green gate cannot see: a guard that is written down
  and dead, an error read as a default, a panic one row below the size anyone
  tests at, and a message that is never printed. The theme is **silence** —
  every place fastf did the wrong thing, or nothing, and said so nowhere. Four
  work phases, each its own PR:
  - **Nothing vanishes** (#51): a folder holding a `PROJECT_INFO.md` fastf
    cannot read is named instead of dropped, and a hand-edit that removes a
    field which is not the project's identity no longer removes the project;
    the counter floor stops reading an unreadable file as zero;
    `read_base_readonly` abandons a rejected cache the way discovery does; a
    note written after a heading the user added stays readable; and `reconcile`
    finishes a case-only rename killed between its two renames — the one
    multi-step mutation in the crate that had no recovery story.
  - **Nothing crashes, nothing corners you** (#52): the settings screen's base
    editor panicked on any window 16–23 rows tall; three depth guards were
    written down and unreachable, one of them on the walk running on the
    smallest stack; a destructive verb could act on a project nobody had named;
    and a worked-on template could be thrown away by a quit with no question.
  - **Every surface says one thing** (#53): the `recent_limit` key the file did
    not hold; Esc as an error at one prompt and a cancel at the next; a
    recursive register reporting success over total failure; an editor's
    discarded exit status; eight sentences that spelled a key into prose; and
    widths measured in bytes and characters where columns were meant.
  - **The record** (#54): this file, the docs, the packaging, and the dead
    weight v2.0.0 left behind — plus three tests that could not fail.

  `.github/release-notes/v3.3.0.md` is the user-facing account. Nothing changed
  about a flag, a config key or a file format: every `config.toml` and
  `PROJECT_INFO.md` an earlier fastf wrote still works.
- Released: **v3.2.0, published 2026-09-08** — what a preview promises is what a
  create writes, and a template is addressed by the folder it lives in: eight
  findings a Windows pass reproduced, none of them Windows-specific.
- Released: **v3.1.4, v3.1.3, v3.1.2** — the relaunch flag off every surface a
  user reads, "I am the rerun" as a flag on argv rather than an inherited
  variable, and a fastf-opened terminal carrying none of fastf's own
  bookkeeping.
- Released: **v3.1.1** — one command installs fastf on any Linux, checksum
  verified, and puts it on PATH.
- Released: **v3.1.0** — the dashboard says each thing once, templates are a
  tab, batch verbs land, and `copy-to` puts a project on a backup drive keeping
  its ID.
- Released: **v3.0.0, published 2026-09-04** — the guided app on ratatui: one
  dashboard over the whole library, every flow native, the command line's
  prompts in the same palette, and `dialoguer` gone. Delivered in nine PRs
  (#35–#43); `.github/release-notes/v3.0.0.md` is the user-facing account.
  **Breaking:** `show-banner` and `show-frame` are gone (accepted and ignored,
  so nothing that sets them starts failing); `recent-default-limit` is now
  `recent-limit`. Search stopped guessing at two kinds of word: a number means
  an ID, not the digits scattered through a date, and a word containing `/` is a
  literal tag path.
- The earlier releases are in the train below; the current design is
  `CLAUDE.md`, `src/core/CLAUDE.md` and `src/tui/CLAUDE.md`.
- Verified by hand, 2026-08-31: the launcher smoke test on a desktop session,
  plus a Windows pass. Neither is reachable from CI.
- Outstanding manual passes, needing the maintainer (none is reachable from
  CI; the pty suite covers each on a sandbox):
  - `fastf` in an 80×24 and a 120×40 window; `fastf search tag:x`;
    `fastf </dev/null`; `NO_COLOR=1 fastf`; a launcher-started `fastf` still
    opens a window running the app.
  - A real move between two mounted bases with the progress modal, and a
    cancel mid-batch-move on a real second volume; the `$EDITOR` note flow in
    a real terminal.
  - A marked batch over the real library — a tag, a note, a delete.
  - A real create with post-create actions (`git init` / `$EDITOR`) on a real
    template, and a register of a folder that already holds a
    `PROJECT_INFO.md`.
  - Build a real template end to end and create a project from it; edit one
    of the gallery templates — following the guide's own walkthrough, which is
    the one test of it that matters.
  - The legacy Windows console pass for the ASCII alphabet, and the wheel on
    a Windows console's alternate screen.
  - Ctrl-Z and `fg`; `kill -INT` twice against the app leaves the shell
    cooked; `ssh localhost -t fastf` picks a theme and `o` says "no display".
- Last reviewed: **2026-09-11** (v3.6.1)

## Release train

| Release | What it delivered | Evidence |
|---|---|---|
| v1.4.0 | — | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.4.0) |
| v1.4.1 + v1.5.0 | containment and path safety, plus move/create recovery v2 (shipped inside v1.5.1, no separate tag) | [implementation commit](https://github.com/cristocola/fast-folder/commit/f4f7d40) · [Windows portability fix](https://github.com/cristocola/fast-folder/commit/78a2e1d) |
| v1.5.1 | every mutation shares validation, locking, authoritative reload, and cache refresh | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.5.1) |
| v1.6.0 | the guided project browser never waits on a folder size | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.6.0) |
| v1.6.1 | what fastf says happened is what happened, and every file it rewrites stays readable | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.6.1) |
| v1.7.0 | the guided menu: one way out, nothing lost, nothing rescanned (superseded — Windows CI failed) | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.7.0) |
| v1.7.1 | the same, with the recursion bound safe for a Windows thread stack | [release](https://github.com/cristocola/fast-folder/releases/tag/v1.7.1) |
| v2.0.0 | two surfaces, one engine, nothing trusted by accident: the browser UI removed, and every boundary that took a path on trust made to check it | [release](https://github.com/cristocola/fast-folder/releases/tag/v2.0.0) |
| v2.0.1 | the Windows binary carries its own C runtime, so the exe and the MSI start on a clean install with no Visual C++ Redistributable | [release](https://github.com/cristocola/fast-folder/releases/tag/v2.0.1) |
| v2.1.0 | fastf answers the launcher: `copy`/`path`, numeric ID queries, an ambiguity picker that serves the verb it interrupted, and a terminal opened for itself when it was launched without one | [release](https://github.com/cristocola/fast-folder/releases/tag/v2.1.0) |
| v2.1.1 | the folder name leads the project row, so the column that tells two projects apart survives a narrow relaunched window | [release](https://github.com/cristocola/fast-folder/releases/tag/v2.1.1) |
| v2.2.0 | `fastf term` opens a terminal at a project's folder | [release](https://github.com/cristocola/fast-folder/releases/tag/v2.2.0) |
| v2.2.1 | a text prompt shows its caret in the line being edited, so a rename has a visible insertion point | [release](https://github.com/cristocola/fast-folder/releases/tag/v2.2.1) |
| v3.0.0 | the guided app on ratatui: one dashboard over the whole library, every flow native, the command line's prompts in the same palette, and dialoguer gone | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.0.0) |
| v3.1.0 | the dashboard says each thing once, templates are a tab, batch verbs land, and `copy-to` puts a project on a backup drive keeping its ID | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.1.0) |
| v3.1.1 | one command installs fastf on any Linux, checksum verified, and puts it on PATH | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.1.1) |
| v3.1.2 | the terminal fastf opens is the user's: it carries none of fastf's own bookkeeping, so nothing started from that window behaves differently | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.1.2) |
| v3.1.3 | "I am the rerun" is a flag on the rerun's own command line, so nothing a fastf window starts can inherit the claim — a package build no longer stops for a keypress | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.1.3) |
| v3.1.4 | that flag is off every surface a user reads: `hide` never kept it out of the generated shell completions | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.1.4) |
| v3.2.0 | what a preview promises is what a create writes, and a template is addressed by the folder it lives in: eight findings the Windows pass reproduced, none of them Windows-specific | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.2.0) |
| v3.3.0 | nothing fails quietly: a project fastf cannot read is named rather than dropped, the app cannot be crashed or made to act on the wrong project, guards that were written down are enforced, and every surface says one thing | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.3.0) |
| v3.4.0 | the template editor explains itself: a panel that says what each part is and what the template would produce, a seven-page guide with a walkthrough that builds one, and a Save row that counts what is still worth a look | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.4.0) |
| v3.5.0 | the app answers the keys you try: one movement grammar in every list, `→`/`←` to go in and come back, every key line read from the registry, four things it could not do, and motion only where it answers a question | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.5.0) |
| v3.6.0 | the pane reads the file: notes are the journal and keep every line, todos, a detail cache that checks the disk, the mouse as a setting, and motion that fades and eases instead of flashing | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.6.0) |
| v3.6.1 | the mouse is the terminal's: no clicks, text selects as in any program, the wheel still scrolls, and `mouse` a retired key | [release](https://github.com/cristocola/fast-folder/releases/tag/v3.6.1) |

Each release's guarantees live in `CLAUDE.md` (the current design) and the test
suite (enforced), not here — this table is what shipped when and where to find
the evidence. `git log`/the PR history has the phase-by-phase detail for any
release that wants it.

## Release and documentation gates

**The Release workflow runs all of this itself** — `release.yml`'s `gates` job
calls `ci.yml` in full, and `build` needs it. A tag can no longer publish
something CI has never seen, so this list is what to expect green rather than a
checklist to work through by hand.

**Which is exactly why the tag goes on a commit whose PR run was already
green on both platforms.** Every release failure this project has had was a
test that passes on the maintainer's Arch desktop and fails on a Windows runner
or a headless two-core Linux one; because `gates` is the whole of CI, each one
was a failed *release*. The `release` skill lists the five patterns and how to
recognise them. Push the branch, open the PR, wait for the matrix, then tag.

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo clippy --all-targets --release -- -D warnings` (debug-only code is
  absent from a release build, so an item used only from it is dead there and
  nowhere else)
- [x] `cargo test --all-targets`
- [x] `cargo test --release`
- [x] Windows cfg compile: `cargo check --all-targets --target
  x86_64-pc-windows-{gnu,msvc}`
- [x] Windows clippy: `cargo clippy --all-targets -- -D warnings` on a Windows
  runner (CI's "fmt + clippy (windows-latest)" leg), so `#[cfg(windows)]` code is
  linted rather than merely compiled
- [x] `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked` (CI's "docs build
  clean"; a `pub` item's docs may not link to a `pub(crate)` one). **Locally,
  `rm -rf target/doc` first**: `cargo doc` is incremental and reports a clean
  run without rebuilding, so a fresh `private_intra_doc_links` error can pass on
  a developer's machine and fail on the runner, which starts from nothing.
- [x] Existing Linux CI target ([main run 32631534113](https://github.com/cristocola/fast-folder/actions/runs/32631534113))
- [x] Existing Windows CI targets, debug and release ([main run 32631534113](https://github.com/cristocola/fast-folder/actions/runs/32631534113))
- [x] GitHub Release workflow built Linux GNU/musl archives, the Windows ZIP,
  and the MSI; every asset matched `SHA256SUMS` ([run 32631862162](https://github.com/cristocola/fast-folder/actions/runs/32631862162)).
- [x] `makepkg -f` completed for `fast-folder` and `fast-folder-bin`; the source
  package's release test suite passed before both AUR repositories were pushed.

Regression coverage grows with the relevant release:

- [x] The app never switches a mouse tracking mode on, whatever an older
  `config.toml` says, and `config set mouse` is accepted and says it is no
  longer used (v3.6.1).

- [x] A note of several lines round-trips through `append_journal_entry`,
  `note add -`, the editor and the quick note; the reader takes every entry
  shape (` — `, a bare date, a colon, a star), every line up to the next
  entry, the undated preamble, both sections of a legacy file, a heading in
  any case with or without a colon, and never a `###`; a legacy `## Journal`
  receives the append byte for byte; `replace_note` and `toggle_todo` refuse
  a note or task that changed meanwhile and write nothing; `add_todo` opens
  `## Todo` after the notes; `notes --since` refuses what `recent --since`
  refuses (v3.6.0).

- [x] A note is several pane rows with only its first selectable, the latest
  five under `… n earlier`, a long one cut with `… n more lines`; Enter on a
  todo sends the toggle at once with no edit open and the answer lands on its
  row; a `DetailOnly` change keeps the detail on screen, starts no table
  pulse and keeps the cursor; a cached detail is checked with its stamp on
  every visit and on F5, never read again unasked; a detail whose metadata
  disagrees with its row patches the row and refreshes the index; a note
  appended from outside the app is on the pane within a second under a pty;
  the mouse setting is flipped live from the palette and written through
  `config set` (v3.6.0).

- [x] A pulse is the wash at its start, between the wash and the ground at
  its middle and gone at its end; the sixteen colours hold and let go; the
  focus borders and titles ease between their rest states with no wash and no
  snap, and land at once with motion off; a status line arrives under a wash
  across the whole line and settles; a sort or a filter pulses the selected
  row and a recompute alone does not; mono never asks for the fast wake
  (v3.6.0).

- [x] Every list stops at its ends and the extreme jump deltas do not
  overflow; every command bound to `←`/`→`/`h`/`l` is a focus move or a page
  turn; Tab reaches the template pane on a narrow window; a focus move pulses
  the pane it landed in and asks for the fast wake only while it does; the
  pane's cursor walks selectable rows, stops, keeps itself in view and is drawn
  only with the focus; `set_variable` rewrites the frontmatter, the derived
  tag and the body table and nothing else, refuses a `select` value outside
  its options and leaves a reshaped table alone; `replace_tag` renames in
  place and empties to a removal; `set_notes` keeps every other byte, creates
  the section before the journal, and refuses a `##` line; `add_tags` refuses
  what is not a tag; in the pane nothing edits before Enter, Esc restores, a
  refusal stays open with its message, a select offers only its options, the
  notes save with `Ctrl-S`, and a landed edit's cursor follows the thing it
  edited across the rows that changed.

- [x] A page of sizes filling in for the first time does not pulse, a size that
  changed does, and a size a verb threw away pulses once when it comes back;
  `tag reauto` keeps every tag it did not derive — the template's own literal
  tags and anyone's hand-typed `slug/value` alike — on a project with the
  record and on one written before it existed.

- [x] The app moves only where a still frame could not answer a question, and
  stops the moment it has: the clock is stamped on every message rather than
  counted on the tick, a tick is due at a moment so a burst cannot starve the
  spinner, the faster wake ends with the pulse, and `motion = off` or a palette
  with no colour draws the frame that was drawn before (v3.5.0).

- [x] `v` reaches from the last mark to the cursor in view order and refuses
  without an anchor; every one-way sort runs both ways with its tie-break the
  right way up, and a sort label written before there was a direction still
  names an order; `/` narrows the settings and Esc gives the screen back
  (v3.5.0).

- [x] One movement grammar reaches every list — the action menu and the builder
  page and jump like the rest — an arrow and its vim letter are never bound
  apart, `→`/`←` go in and back out without ever quitting, every context has a
  help that is true and an Esc that leaves it, and Ctrl-C is a declared command
  (v3.5.0).

- [x] Every explanatory sentence in the template editor is declared once, in
  `tui::guide`, names its keys through `command::key_of`, and writes no
  character the theme owns an ASCII spelling for; the sample folder name is a
  pure function of the scratch template with no clock in it; the panel falls
  back to today's footer on a window too narrow for it, and the ASCII alphabet
  reaches its live folder tree. The guide offers itself once across **both**
  automatic doors, lands on the page for the row it was opened from, stops at
  both ends, and clamps its scroll at the width the view draws it (v3.4.0).

- [x] A first run that failed between the two bundled templates is finished by
  the next one, and the first-run banner is on stderr so it cannot land inside
  `$(fastf path …)` (v3.3.0).
- [x] `tag reauto` re-derives the template's tags and keeps the free-form ones;
  a same-filesystem move preserves a symlink inside the project, on unix as well
  as on the two Windows suites CI never runs (v3.3.0).
- [x] The key `config set` takes is the key `config.toml` holds is the key
  `config show` prints; Esc at the template picker is a cancel and not an error;
  a recursive register that onboarded nothing exits non-zero and says what it
  skipped; an editor that failed is not reported as having opened anything; and
  every refusal `recent` makes is below the launcher hand-off (v3.3.0).
- [x] A sentence that names a key reads it from the command registry, a key line
  is cut at a whole pair, and a success wears the theme's own tick — so the
  ASCII alphabet is right everywhere (v3.3.0).
- [x] Widths are display columns everywhere, the line editor's window included,
  so a CJK or Cyrillic name puts the caret where it belongs (v3.3.0).
- [x] The settings screen's base editor opens on a short window instead of
  panicking, a confirmation names every folder it is about at 60 columns, and a
  text area's drawn line and its caret agree about where the cursor is (v3.3.0).
- [x] The depth limit is enforced on every recursive walk that declares one —
  the size scan, a move manifest, and reading a template out of a folder — and a
  template's `structure:` is bounded where it loads (v3.3.0).
- [x] A destructive verb runs on the project its dialog named or on nothing, a
  worked-on template is not thrown away by any quit gesture, and a template read
  that answers for something else is dropped (v3.3.0).
- [x] A folder holding a `PROJECT_INFO.md` fastf cannot read is named rather
  than dropped in silence, and a hand-edit that removes a field which is not the
  project's identity does not remove the project; an ordinary folder with no
  metadata still says nothing (v3.3.0).
- [x] An unreadable data-directory counter is reported once per process instead
  of read as zero, a create refuses while it cannot be read, and `id show` says
  the next ID is unknown rather than claiming the maximum is reached; the
  counter floor abandons a forged cache the way discovery does, without writing
  (v3.3.0).
- [x] A journal note written after a heading the user added is still readable,
  and their section stays where they put it (v3.3.0).
- [x] `reconcile` finishes a case-only rename that was killed between its two
  renames, refuses when the target is taken, and leaves a lookalike dot-folder
  alone (v3.3.0).
- [x] `reindex` writes down the number behind an id it can resolve and leaves
  alone one it cannot; `on_name_collision = "error"` refuses instead of
  suffixing (v3.3.0).

- [x] A dry run's file list and its previews come from one walk and one
  classification: an excluded file appears in neither, a verbatim one is
  previewed with its `{braces}` intact and marked, and every previewed path is
  a path the create writes (v3.2.0).
- [x] A template is addressed by the folder it lives in: a manifest whose
  `slug:` disagrees still loads under the folder's name, two folders cannot
  answer to one slug, a folder name that is not a valid slug is skipped, and a
  save repairs the manifest. Every template `template list` names can be shown
  and created from (v3.2.0).
- [x] `search` refuses a clause it cannot read and `recent` refuses a filter
  that can only match nothing, both naming the real answers; a `*` in a value
  may lead, trail or do both (v3.2.0).
- [x] `id show` prints the counter and its successor as the same kind of
  number, including under a digits-only `id.prefix` (v3.2.0).

- [x] A copy lands verified with the original untouched and its ID kept; a
  destination inside a configured base is refused by name; two bases holding one
  ID list as two rows, resolve as "in 2 bases", and mutate independently
  (v3.1.0).
- [x] A batch item's effects reach the runtime, a batch aimed at marks a filter
  hides says so, and a move reports rename versus copy (v3.1.0).
- [x] Real `.tmp`/`.part`, zero-byte, binary, and empty-directory move payloads.
- [x] Missing source, occupied target/staging, cancellation, marker failure, and
  cleanup-pending behavior.
- [x] Template slug, structure, rendered-path, and base-override containment.
- [x] Obsolete/malformed markers remain byte-identical and cannot affect outside
  sentinels; reconciliation is idempotent.
- [x] Hard-abort subprocess cases at transaction creation, mid-copy,
  post-verification, post-publication, and before/after source cleanup (v1.5.0).
- [x] Cross-interface mutation-loss and registration partial-outcome cases
  (v1.5.1).
- [x] The project list draws before measuring, fills in without input, and never
  reflows a row as a size lands (v1.6.0).
- [x] Names that sanitize away or start with `.` are refused before any folder is
  created; the counter's maximum is enforced at `id set` and at create; an
  unreadable template manifest is never overwritten (v2.0.0).
- [x] A folder name full of shell metacharacters runs no command of its own and
  the post-create shell's cwd is the project (v2.0.0, unix); the Windows
  expansion is the quoted variable (v2.0.0, windows).
- [x] `template delete` waits for a held `DataLock` and leaves the template on
  disk until it gets it; a slug rename moves the directory; a linked template
  directory is refused (v2.0.0).
- [x] Apply through a link in the target is refused and the outside directory
  stays empty (v2.0.0, unix symlink + windows junction); template ingestion
  refuses a pre-planted link before writing a byte.
- [x] A planted cache naming `/etc`, `..` or an absolute path outside the base
  lists nothing and opens nothing; a project directory replaced by a link is
  refused by `open` (v2.0.0).
- [x] A text prompt parks a visible caret after the text it is editing, driven
  through a real pty and matched against the cursor escapes themselves — the one
  place in that suite where the cursor is the behaviour rather than noise
  (v2.2.1).

Manual move smoke and follow-up:

- [x] Linux same-filesystem direct rename and genuine cross-filesystem staged
  move (`/tmp` to `/dev/shm`) using the release binary.
- [x] Windows same-drive rename and ordinary move to another mounted
  drive/share, on a real NTFS volume and a real SMB share. Automated rather
  than left to a hand-run smoke: `tests/windows_live.rs` is opt-in on two
  environment-supplied bases, and covers the same-volume rename, the
  cross-device staged copy end to end (verified, published, no transaction
  left behind), a junction refused by the staged path, the counter converging
  across the share, and discovery, tagging and notes over SMB. It found two
  Windows-only defects on its first run — `atomic::write`'s publish blocked by
  a read-only destination, and a non-ASCII case-only rename refused as its own
  target — both fixed with regression tests that run in CI.
- [ ] "Reveal" from the app's action menu and `fastf open` (the
  `ShellExecuteW` path — CI compiles and lints it, but only a real desktop
  session opens a window), plus `fastf term`. Outstanding since v1.5.1: these
  need a person at a desktop to say whether the right window appeared, so no
  suite can close them.

Behavior changes and user documentation land together. CLI flags and the
template and cache schemas remain compatible within a major version; rejecting
previously accepted unsafe input is intentional, and v2.0.0 is the major that
removes `fastf ui`. Update this roadmap in every
implementation PR/commit. Update `CLAUDE.md` only after a decision has landed.
This work does not use GitHub issues, a separate ADR system, or a changelog.

## Unscheduled backlog

- A `fastf todo` verb — list, add, toggle — so the command line has what the
  pane has; and removing or rewording a todo from the pane, which today means
  editing the file (the pane follows within a second).
- Portable project packages.
- Template upgrades.
- Template diagnostics and language-server support.
- Project lifecycle states.
- Declarative post-create workflows.
- Scriptability: `--json`/`--format` output, `search --limit/--template/--since/--tag`,
  `print_path` as a `new` flag rather than only a config toggle,
  `--color=auto|always|never` (`colored` currently gates on stdout only, so
  stderr gets ANSI when redirected), documented exit codes, and
  `completions <shell>` as a typed `clap_complete::Shell` rather than a bare
  `String`.

Carried over from the v2.1.0 plan's parking lot, so they do not stay buried in a
closed plan file:

- An ambiguity picker for `move`, `tag` and `note`. They gained v2.1.0's numeric
  tier, but not the picker — they resolve and then act on the one project, so
  offering a choice there is a larger change than it looks.
- A native KRunner DBus runner: search-as-you-type from Alt+Space without
  spawning fastf per keystroke. Its own deliverable, probably its own repository.
- `reveal_folder` on unix still waits on `.status()` (the app runs it on a
  worker, and since v3.0.0 it checks for a display, gives the handler no
  terminal and reads the exit status). A file-manager handler that runs in the
  foreground would still hold a `fastf open` that has no terminal to show the
  wait in; detaching it the way the relaunch spawn does is the remaining step.
- `ptyxis` (the GNOME 47+ default) in the emulator table, if anyone asks.
- A watchdog for a clipboard tool that does not fork — the `wl-copy --foreground`
  shape. `clipboard::feed`'s `wait()` has no timeout.

The ASCII alphabet, finished. v3.3.0 rescued the theme's tick from twelve
literal `✓`s and the template editor's own separators followed with the guide;
the same defect is still spelled out on four other screens, where a console with
no `·`, `…` or `→` draws a replacement box:

- `app/jobs.rs` — `busy()`'s eight `…` labels and the report's `·` separator.
- `runtime.rs` — the session lines (`renamed X → Y`, `moved`, `applied`) and
  `run_action`'s `·`-joined warning.
- `app/actions.rs` — `NEW_TAG` (`"New tag…"`), a picker row.
- `rows.rs` — `PENDING_LABEL`, which duplicates `Glyphs::pending` rather than
  reading it; `view::projects` already asks the theme, so the two can disagree.

Each is a function that builds a display string with no theme in reach, so the
fix is the one `Builder::summary` and `transform_example` just took: hand it the
`Glyphs`. Worth one phase, with the guard test `guide.rs` already has extended
over `src/tui/`.

Smaller findings from the v1.7.1 audit, not worth a phase on their own:

- `query::resolve_field` clones per field access and `Predicate::Free`
  lowercases per comparison (`src/core/query.rs`) — fine at current scale, would
  matter at a much larger library.
- `size_scan::request`'s queue dedup is an O(n²) `contains` scan
  (`src/util/size_scan.rs`) — bounded by page size today.
- The action menu only offers "Move to another base" when one is already
  mounted; an `Unresponsive` base could offer a "retry probe" item instead of
  just being left out.
- Clipboard via OSC 52 for ssh sessions, where no clipboard tool exists: a
  new escape-sequence write to the terminal, deliberately left out of v3.0.0;
  the "here is the path" dialog is the answer until then.
- Delete to the system trash instead of permanently (a dependency and a core
  change).
- A `base=` search operator, or a base filter key, for a library on several
  drives.
- "Open in `$EDITOR`" as a project verb; the journal's `--since` in the app;
  `fastf new --no-post` parity in the wizard.
- Windows terminal-layer tests: the pty suite is unix by construction, so raw
  mode, the wheel and the ASCII alphabet are untested there.
- An input thread that truly blocks: it polls once a second when idle because
  crossterm's read cannot be cancelled for the suspend handshake.
