# fastf roadmap

What is still open, and nothing else. What shipped is in
[`.github/release-notes/`](.github/release-notes/) and on the
[releases page](https://github.com/cristocola/fast-folder/releases); the
current design is the `CLAUDE.md` files, and the gates a release passes are in
the `release` skill. Close an item by deleting it in the PR that does the work.

## Manual passes

None of these is reachable from CI. The pty suite covers each one on a sandbox;
what it cannot say is whether a real terminal, desktop or drive agrees.

- `fastf` in an 80×24 and a 120×40 window; `fastf search tag:x`;
  `fastf </dev/null`; `NO_COLOR=1 fastf`; a launcher-started `fastf` still
  opens a window running the app.
- A real move between two mounted bases with the progress modal, and a cancel
  mid-batch-move on a real second volume; the `$EDITOR` note flow in a real
  terminal.
- A marked batch over the real library — a tag, a note, a delete.
- A real create with post-create actions (`git init` / `$EDITOR`) on a real
  template, and a register of a folder that already holds a `PROJECT_INFO.md`.
- Build a real template end to end and create a project from it; edit one of
  the gallery templates, following the guide's own walkthrough, which is the one
  test of it that matters.
- The legacy Windows console pass for the ASCII alphabet, and the wheel on a
  Windows console's alternate screen.
- Ctrl-Z and `fg`; `kill -INT` twice against the app leaves the shell cooked;
  `ssh localhost -t fastf` picks a theme and `o` says "no display".
- On Windows, Reveal from the app's action menu and `fastf open` (the
  `ShellExecuteW` path, which CI compiles and lints but cannot watch open a
  window), plus `fastf term`. A person at a desktop has to say whether the right
  window appeared.

Last reviewed: 2026-09-11.

## Backlog

Unscheduled; nothing here is promised.

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
  `--color=auto|always|never` (`colored` gates on stdout only, so stderr gets
  ANSI when redirected), documented exit codes, and `completions <shell>` as a
  typed `clap_complete::Shell` rather than a bare `String`.
- An ambiguity picker for `move`, `tag` and `note`. They resolve and then act on
  the one project, so offering a choice there is a larger change than it looks.
- A native KRunner DBus runner: search-as-you-type from Alt+Space without
  spawning fastf per keystroke. Its own deliverable, probably its own repository.
- `reveal_folder` on unix waits on `.status()` (the app runs it on a worker,
  checks for a display, gives the handler no terminal and reads the exit
  status). A file-manager handler that runs in the foreground would still hold a
  `fastf open` that has no terminal to show the wait in; detaching it the way the
  relaunch spawn does is the remaining step.
- `ptyxis` (the GNOME 47+ default) in the emulator table, if anyone asks.
- A watchdog for a clipboard tool that does not fork — the `wl-copy --foreground`
  shape. `clipboard::feed`'s `wait()` has no timeout.

### The ASCII alphabet on four more screens

A console with no `·`, `…` or `→` draws a replacement box. The theme's glyphs
already answer for the tick, the template editor and the guide; four screens
still spell the characters out:

- `app/jobs.rs` — `busy()`'s eight `…` labels and the report's `·` separator.
- `runtime.rs` — the session lines (`renamed X → Y`, `moved`, `applied`) and
  `run_action`'s `·`-joined warning.
- `app/actions.rs` — `NEW_TAG` (`"New tag…"`), a picker row.
- `rows.rs` — `PENDING_LABEL`, which duplicates `Glyphs::pending` rather than
  reading it; `view::projects` already asks the theme, so the two can disagree.

Each is a function that builds a display string with no theme in reach, so the
fix is the one `Builder::summary` and `transform_example` took: hand it the
`Glyphs`. Worth one phase, with the guard test `guide.rs` already has extended
over `src/tui/`.

### Smaller findings

- `query::resolve_field` clones per field access and `Predicate::Free`
  lowercases per comparison (`src/core/query.rs`) — fine at current scale, would
  matter at a much larger library.
- `size_scan::request`'s queue dedup is an O(n²) `contains` scan
  (`src/util/size_scan.rs`) — bounded by page size today.
- The action menu only offers "Move to another base" when another base is
  usable; an unresponsive one could offer a "retry probe" item instead of just
  being left out.
- Clipboard via OSC 52 for ssh sessions, where no clipboard tool exists: a new
  escape-sequence write to the terminal, left out on purpose until it is asked
  for. The "here is the path" dialog is the answer until then.
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
