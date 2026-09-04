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

- Prepared: **v3.0.0 — the guided app on ratatui**, tagged on `main` when the
  consolidation PR lands. One full-screen dashboard replaced the
  menu-of-prompts: the library on screen and acted on, fuzzy search and a
  command palette, sizes filling in without input, every mutation patching its
  row rather than rescanning. Delivered in eight PRs (#35–#42) — the runtime,
  the one command registry, the dashboard, search, sort and filters; the action
  menu and every single-project verb as a native modal, with a move as a
  cancellable job; marks, and the destructive verbs over them as jobs with a
  failure report that leaves the unrun rows marked; the create, register and
  apply flows as a form, a preview built by the code that commits it, and
  Enter; the template studio and a builder that is a list of a template's
  parts with a live folder tree; every setting on one screen, the ID counter,
  the maintenance verbs and a first-run dialog; the command line's own
  prompts drawn by the same ratatui, `dialoguer` removed; the mouse and the
  ASCII alphabet for the legacy Windows console — and a ninth, the
  consolidation pass (#43): every verb over the marks, delete by the word,
  the message log, session memory, the theme key, the terminal always given
  back, `rename`/`unregister`/`delete` on the command line, and the docs
  brought into line. `.github/release-notes/v3.0.0.md` is the user-facing
  account.

  **Breaking:** `show-banner` and `show-frame` are gone (accepted and ignored,
  so nothing that sets them starts failing); `recent-default-limit` is now
  `recent-limit`, with the old key still parsing. Search stopped guessing at
  two kinds of word: a number means an ID, not the digits scattered through a
  date, and a word containing `/` is a literal tag path.
- Released: **v2.2.1, published 2026-09-03** — the text prompts show where you
  are typing. `prompt::text` draws its line with `write_line`, which ends the
  block a row *below* the text, and it hid the caret for the repaint and never
  showed it again, so **Rename folder** — and every other typed field — offered
  no insertion point at all. The line editor now parks the caret in the line it
  is editing and shows it there, at the cursor's offset **within the visible
  window** rather than its index into the whole string, which are different
  numbers once a long line has scrolled.
- Released: **v2.2.0, published 2026-09-01** — `fastf term` opens a terminal at
  a project's folder, the fourth verb (with `open`, `copy`, `path`) that
  resolves a query and hands the result to another program.
- The v2.1.x guarantees are in the release train below; the current design is
  `CLAUDE.md`.
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
    of the gallery templates.
  - The legacy Windows console pass for the ASCII alphabet, and the mouse in a
    terminal that reports it.
  - Ctrl-Z and `fg`; `kill -INT` twice against the app leaves the shell
    cooked; `ssh localhost -t fastf` picks a theme and `o` says "no display".
- Last reviewed: **2026-09-04** (v3.0.0 consolidated)

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
- [x] `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --locked` (CI's "docs build
  clean"; a `pub` item's docs may not link to a `pub(crate)` one)
- [x] Existing Linux CI target ([main run 32631534113](https://github.com/cristocola/fast-folder/actions/runs/32631534113))
- [x] Existing Windows CI targets, debug and release ([main run 32631534113](https://github.com/cristocola/fast-folder/actions/runs/32631534113))
- [x] GitHub Release workflow built Linux GNU/musl archives, the Windows ZIP,
  and the MSI; every asset matched `SHA256SUMS` ([run 32631862162](https://github.com/cristocola/fast-folder/actions/runs/32631862162)).
- [x] `makepkg -f` completed for `fast-folder` and `fast-folder-bin`; the source
  package's release test suite passed before both AUR repositories were pushed.

Regression coverage grows with the relevant release:

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
- [ ] Windows same-drive rename and ordinary move to another mounted drive/share
  using the published MSI or ZIP; plus, new in v2.0.0, "Reveal" from the app's
  action menu and `fastf open` (the `ShellExecuteW` path — CI compiles and lints
  it, but only a real desktop session opens a window). This remains the sole post-release validation
  item, still outstanding across every release since v1.5.1.

Behavior changes and user documentation land together. CLI flags and the
template and cache schemas remain compatible within a major version; rejecting
previously accepted unsafe input is intentional, and v2.0.0 is the major that
removes `fastf ui`. Update this roadmap in every
implementation PR/commit. Update `CLAUDE.md` only after a decision has landed.
This work does not use GitHub issues, a separate ADR system, or a changelog.

## Unscheduled backlog

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
  mode, the mouse and the ASCII alphabet are untested there.
- An input thread that truly blocks: it polls once a second when idle because
  crossterm's read cannot be cancelled for the suspend handshake.
