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
- The pane in every shape of a real terminal: a 40×12 corner, a tall narrow
  tmux split (the pane under the list), a wide short drop-down (beside it),
  and dragging a corner across all three with a note open in the pane.
- The todo keys on a real keyboard: F2 on a laptop (with and without Fn),
  `+` on a keyboard where it needs Shift, `<` and `>` on a German layout, and a
  list pasted from a browser, an editor and a chat window.
- A real move between two mounted bases with the job dialog: Esc to hide it,
  the header's chip, the terminal window closed mid-move, and a cancel
  mid-batch-move on a real second volume; the `$EDITOR` note flow in a real
  terminal.
- A marked batch over the real library — a tag, a note, a delete.
- A real create with post-create actions (`git init` / `$EDITOR`) on a real
  template, and a register of a folder that already holds a `PROJECT_INFO.md`.
- Build a real template end to end and create a project from it; edit one of
  the gallery templates, following the guide's own walkthrough, which is the one
  test of it that matters.
- The legacy Windows console pass for the ASCII alphabet — the hint bar's
  `Right details` and `Up/Down` included — and the wheel on a Windows console's
  alternate screen; F2, `+`, `<` and `>` there.
- Ctrl-Z and `fg`; `kill -INT` twice against the app leaves the shell cooked;
  `ssh localhost -t fastf` picks a theme and `o` says "no display".
- On Windows, Reveal from the app's action menu and `fastf open` (the
  `ShellExecuteW` path, which CI compiles and lints but cannot watch open a
  window), plus `fastf term`. A person at a desktop has to say whether the right
  window appeared.

Last reviewed: 2026-09-23.

## Backlog

Unscheduled; nothing here is promised.

- Editing a template's starter todos in the guided builder: the manifest field
  round-trips untouched, but only `template.yaml` and an editor can change it.
- Portable project packages.
- Template upgrades.
- Template diagnostics and language-server support.
- Project lifecycle states.
- Declarative post-create workflows.
- Scriptability, what is left of it: `search --limit/--template/--since/--tag`,
  `print_path` as a `new` flag rather than only a config toggle,
  `--color=auto|always|never` (`colored` gates on stdout only, so stderr gets
  ANSI when redirected), documented exit codes, and `completions <shell>` as a
  typed `clap_complete::Shell` rather than a bare `String`.
- An ambiguity picker for `move`, `tag`, `note` and `todo`. They resolve and then act on
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

### Known weaknesses in the code, most serious first

None is a wrong result today; each is where the next one would come from.

**The engine.**

2. Three reads ask one base after another with no deadline: `library::max_id`
   (every create and every preview), `Counters::base_floor` and `fastf
   reindex`. A mount that stopped answering holds `fastf new` for the kernel's
   own timeout, against the rule in `CLAUDE.md` › Configuration.
3. Reconcile makes destructive filesystem calls that skip `util::fs_retry`.
4. `transactions::target_ignores_case` ignores whether its probe file could be
   removed; one left behind is in the copy, and verification refuses the move.
5. A record's removal gives up after about four minutes on a mount that
   answers `EIO` for good, because waits nest (`src/core/CLAUDE.md`). The
   schedules are as they were; whether four minutes is wanted is undecided.
6. `provisioning::finish_record` files a *successful* sweep of strays under
   `unrecoverable`, which counts as needing a look.
7. `jobs::claim` discards the result of the write that makes the claim.
8. The case-only rename's search for a free staging name has no upper bound
   (`library::lifecycle`), and `removal::empty_again` pauses 1.2 s after its
   last listing.

**What a person can meet.**

9. The palette: Enter on a project the base filter or a `recent` preset hides
   does not bring it into view; the cursor stays where it was.
10. The pane's folder listing takes 200 entries before it sorts, so a folder
    with more shows an arbitrary 200 (`tui::loaders`).
11. On Windows, `fastf move`'s list of bases and `print_path` print the
    `\\?\` form of a path, where everything else prints `display_path`.
12. The picker pads its id, base and template columns by character count and
    measures them in display columns: a base label in double-width characters
    misaligns the rows (`tui::rows`).
13. A hand-written `on_name_collision = "Error"` means `suffix`, because the
    value is matched case for case; `config set` lowercases first.
14. A hand-written `recent_limit = 0` is read as 1; the flag and `config set`
    refuse 0.
15. `fastf apply` mentions `{id}` when only an `exclude`d file holds the token.
16. `register --recursive` refuses a date flag saying bulk registration "takes
    each folder's own date", while `--use-today` is accepted with it.
17. Search forgives a letter left out (`lulaby`) and not one typed too often.
18. On Windows the cursor is put back after a signal by an escape sequence
    through the ordinary handles; whether a legacy console prints it as text
    has not been looked at.

**Structure.**

19. A cancel travels as message text, in three spellings, and is recognised by
    matching it.
20. `Result<_, String>` is the error channel in `merge`, `move_preflight` and
    the app's messages; `theme`, `motion` and `log_level` are strings in
    `Config`; a record's `kind` is a string with named constants.
21. Four dispatch functions run 300 to 700 lines: `App::run`, `App::handle`,
    `runtime::run_action`, `main::run`.
22. Eight files are between 1,500 and 1,950 lines: `core/body.rs`,
    `tui/app/studio.rs`, `tui/runtime.rs`, `tui/view/modals.rs`, `src/main.rs`
    and three test suites. `tests/layering.rs` finds `runtime.rs` by its place
    and the view files by their folder, so splitting either means moving a
    guard with it.
23. Five sentences in `src/tui/view/` spell a key by hand where the rule is to
    read it from the registry; `view::modals::render_move_progress` draws every
    kind of job.
24. About seven loops claim a free name each in their own way; about 45
    plurals are written out by hand; `cli::job_worker` has three batch loops of
    one shape.
25. Kept and unused: `Job.warnings` is never filled, `rows::project_row`'s
    size branch is reached only by its tests, `library::now_iso8601` is a
    compatibility re-export nothing imports, and a few `#[cfg(test)]` branches
    sit inside production functions. `IncompleteKind`'s doc says its names are
    in journals on disk, and nothing in the tree writes them.

**Tests and docs.**

26. The pty suite waits by fixed pauses; `tests/windows_live.rs` skips in
    silence where its drives are missing.
27. `assets::plan_entries`' size limit has no test at the limit.
28. The scans in `tests/layering.rs` stop at a file's first `mod tests` and do
    not start again, so code below an inline test module is not read.
29. `tests/properties.rs` names the YAML crate; `docs/projects.md` narrates
    release numbers.

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
`Glyphs`. One change, with the guard test `guide.rs` already has extended over
`src/tui/`.

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
