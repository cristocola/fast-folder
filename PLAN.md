# PLAN.md — v2.1.0: fastf answers the launcher

> Written 2026-08-31. Worked one phase per session: read PLAN.md, do phase N,
> run the gates, tick only the boxes whose named verification actually ran.
> Each phase lands in its own PR with `ROADMAP.md` and the matching `docs/`
> page updated in the same commit. Findings outside the phase go to the
> Parking lot, not into the diff. This file replaces the v2.0.0 record, which
> lives on in git history.

## Why this plan exists

fastf is launched two ways the current binary only half-serves: a shell, and a
desktop launcher (KDE's krunner and friends), where the process has **no
terminal at all** — stdin is `/dev/null`, stdout and stderr are journald
sockets, and nothing a command prints is ever seen. From a launcher today:

- `fastf open <query>` with one match works (it spawns the file manager and
  needs no terminal). With several matches it prints the ambiguity list into
  the void and exits 1 — from the launcher's point of view, nothing happened.
- `fastf search <query>` starts, refuses or prints to nowhere, and exits.
  The launcher shows a bouncing cursor, then nothing.
- There is no command that puts a project's path on the clipboard. The TUI
  action menu has **Copy path**; the command line has nothing, so the
  launcher has nothing.

The v2.1.0 answer, in one sentence: **when fastf is asked something
unambiguous it acts silently; when it needs to show text or ask a question
and has no terminal, it opens one; and the clipboard becomes a first-class
verb.** Three product decisions behind that:

1. **The ambiguity picker serves the verb it interrupted.** A picker reached
   from `open` opens the chosen project; from `copy` it copies. It never
   detours into the full project action menu — that is what `fastf` (the
   menu) and `fastf recent` are for.
2. **Scripts must never notice any of this.** A pipe, a redirect, cron, CI:
   stdout that is a regular file or FIFO means somebody is reading it, and
   fastf keeps its exact current behavior — plain output, plain errors.
   The relaunch fires only where output provably has no reader.
3. **`fastf path` is for scripts, `fastf copy` is for hands.** `path` prints
   the bare path to stdout (`cd "$(fastf path api)"`); `copy` puts it on the
   clipboard and says so. In a headless GUI session — where stdout has no
   reader — `path` degrades to copy-plus-notification rather than printing
   into the void, so both verbs do the useful thing when typed into a
   launcher.

## Gates — every phase, before its PR

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings` — and `--release` (debug-only
  code is dead in release, live in debug)
- `cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
  (local early warning; CI's Windows runner is the authority)
- `cargo test --all-targets` and `cargo test --release`
- `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked`
- `cargo test --test repo_hygiene`
- `ROADMAP.md` updated in the same commit; the matching `docs/` page when
  behaviour changed; the relevant `CLAUDE.md` only once the decision landed.

---

## Phase 1 — structured resolution and numeric IDs

**Goal.** `library::resolve` currently flattens ambiguity into an error
string, so no caller can offer the candidates to a picker. After this phase
the library answers with data — `Resolution` — and the old `resolve` is a
thin wrapper producing byte-identical errors. Every resolver caller also
gains a numeric tier: an all-digits query matches the project whose ID
*number* equals it, so `fastf open 37` finds ID0037 instead of falling
through to a name-substring search.

**Evidence.** `src/core/library/resolve.rs:19-58`: tiers are exact ID → ID
prefix → case-insensitive name substring; the ambiguous arm formats a string
(capped at `take(10)`) and `bail!`s, so the candidate set dies there. Callers
(`src/cli/recent.rs:155`, `src/cli/move_project.rs:36`, `src/cli/note.rs:40`
and `:86`, `src/cli/tag.rs:22,39,67,107`) all use `library::resolve(&cfg,
query)?`. `src/core/library/tests.rs` asserts on the error text
(`resolve_distinguishes_no_match_exact_and_ambiguous`,
`resolve_ambiguous_errors_with_candidates`) — those stay untouched and green.
`naming::id_value` (`src/core/naming.rs:220`) already reads an ID's trailing
digit run as `Option<u64>`.

**Decisions.**
- `pub enum Resolution { NoProjects, NoMatch, One(Box<Project>), Many(Vec<Project>) }`
  in `resolve.rs`. **Box the `One` variant**: `Project` is large and the
  Windows clippy leg fires `large_enum_variant` where Linux does not
  (`ActionLoop` precedent, root `CLAUDE.md`).
- `pub fn resolve_matches(cfg: &Config, query: &str) -> Resolution`. Tiers:
  exact ID → **numeric** → ID prefix → name substring. Numeric fires only
  when the query is all ASCII digits and parses to `u64`; it matches
  `naming::id_value(&p.id) == Some(n)`. Exact ID stays first because a
  template may declare a digits-only ID prefix, making an all-digits string a
  legal complete ID.
- The three error messages move into `pub(crate)` builders in `resolve.rs`
  (`no_projects_error()`, `no_match_error(query)`,
  `ambiguous_error(query, &[Project])` — the last keeps the `take(10)` cap in
  the *text* while `Many` carries the full list). `resolve()` becomes
  `resolve_matches` + those builders, so the strings exist exactly once.
- `move`, `tag`, `note` gain the numeric tier implicitly (desired) and do
  **not** gain a picker in this plan (Parking lot).

**Steps.**
1. Add `Resolution` and `resolve_matches`; rewrite `resolve` as the wrapper.
2. Tests in `src/core/library/tests.rs`, written against the broken build
   first where the behaviour changes (the numeric-tier test must fail before
   the tier exists): `an_all_digit_query_matches_the_id_number_exactly`
   (library holding ID0004 plus ID0040–ID0049; `resolve(cfg, "4")` must
   return ID0004 — today it is ambiguous), `the_numeric_tier_sits_between_exact_id_and_prefix`,
   `a_numeric_query_too_large_for_u64_falls_through`,
   `resolve_matches_reports_many_without_erroring`.
3. Confirm `resolve_by_id_prefix_and_name` and both existing
   ambiguity-text tests pass unchanged (the wrapper pin).
4. Docs: the shared query paragraph in `docs/cli.md` (how a query resolves)
   gains the numeric tier; `Open`'s `after_help` in `src/main.rs` gains a
   `fastf open 37` example line.

**Acceptance.**
- [x] All gates pass.
- [x] The numeric-tier test was observed failing on the pre-phase build
      (`an_all_digit_query_matches_the_id_number_exactly` and
      `the_numeric_tier_sits_between_exact_id_and_prefix`, both FAILED before
      the tier existed).
- [x] `resolve_ambiguous_errors_with_candidates` and
      `resolve_distinguishes_no_match_exact_and_ambiguous` pass **unmodified**.
- [x] `grep -rn "is ambiguous" src/` shows the string in exactly one place.

---

## Phase 2 — `fastf copy` and `fastf path`

**Goal.** Two new plain subcommands sharing `open`'s resolve→revalidate
spine. `fastf copy <query>` puts the project path on the clipboard and says
what it did; `fastf path <query>` prints the bare path to stdout and nothing
else. The clipboard spawn is hardened so the copied text survives a launcher
cleaning up its children. Ambiguity still errors in this phase (the picker is
Phase 3) — ship value early, keep the diff reviewable.

**Evidence.** The only clipboard caller today is the TUI's private
`copy_path` (`src/tui/actions.rs:316-327`), wording documented at
`docs/cli.md:189` ("It always says what it did"). `util::clipboard::copy`
(`src/util/clipboard.rs:29`) probes `wl-copy`/`xclip`/`xsel`/`clip.exe`/
`clip`/`pbcopy`; the Wayland/X11 tools survive process exit by forking
themselves — but the fork inherits fastf's process group, and a launcher
that kills the group on exit kills the clipboard owner with it.
`ROADMAP.md` already lists `fastf path <query>` in the Unscheduled backlog
under Scriptability. `fastf open`'s revalidate-then-act shape is
`src/cli/recent.rs:152-175`. Note the TUI's Copy path validates nothing —
the CLI commands should mirror `open`'s `revalidate_for_read` instead.

**Decisions.**
- New modules `src/cli/copy.rs` and `src/cli/path_cmd.rs` (`paths_cmd.rs`
  naming precedent — `path` is not a keyword but symmetry helps), plus two
  `Commands` variants and dispatch arms in `src/main.rs`. Plain positionals,
  no `trailing_var_arg`, so `classify_extra` and the exhaustiveness test in
  `main.rs` need no changes.
- `copy`: `Config::load()?` → `library::resolve` → `revalidate_for_read`
  (same `.with_context` shape as `recent::open`) → `clipboard::copy` →
  print `✓  Copied with <tool>` or the documented fallback (print the path
  on its own line) — wording matches `tui::actions::copy_path` exactly.
- `path`: same spine, then one `println!` of
  `util::paths::display_path(&project.path)` with **no `colored` call at
  all** — the bare line is the script contract.
- `fastf path` vs the existing `fastf paths` (data-dir info): a
  cross-reference sentence in **both** commands' `after_help` and both
  `docs/cli.md` entries.
- `clipboard::feed` gains `process_group(0)` on unix
  (`std::os::unix::process::CommandExt`, std since 1.64) with a comment
  naming the reason: launchers signal the process group; the forked
  `wl-copy`/`xclip` that owns the selection must not die with it. Looks
  removable otherwise.

**Steps.**
1. `clipboard.rs` hardening (one `#[cfg(unix)]` block in `feed`).
2. The two command modules + clap variants + `src/cli/mod.rs`.
3. Tests in `tests/cli_output.rs`:
   `path_prints_the_bare_path_and_nothing_else` (stdout is exactly the path
   + newline), `copy_without_any_clipboard_tool_prints_the_path_instead`
   (`PATH` pointed at an empty dir), `path_and_copy_refuse_a_stale_project`
   (plant a project, delete its `PROJECT_INFO.md` after the cache exists —
   revalidate must refuse), `an_ambiguous_copy_errors_with_candidates_when_piped`
   (pins the Phase 3 piped contract early).
4. Docs: `docs/cli.md` command-overview rows + a short section for each;
   `ROADMAP.md`: tick/remove the backlog `fastf path <query>` item.

**Acceptance.**
- [ ] All gates pass.
- [ ] `fastf path <id> | cat` output is byte-exactly the path plus `\n`.
- [ ] With an empty `PATH`, `fastf copy <id>` exits 0 and prints the path.
- [ ] The clipboard comment in `clipboard.rs` names the process-group reason.

---

## Phase 3 — the ambiguity picker serves the verb

**Goal.** In an interactive terminal, an ambiguous `open`/`copy`/`path`
query shows a project picker; Enter performs the invoked verb on the chosen
project and nothing else; Esc cancels politely with exit 0. Piped and
scripted invocations keep the exact Phase 1 error text.

**Evidence.** There is no project picker — `src/tui/pickers.rs` holds only
`pick_template` and `pick_base`; project selection today means the full
browser (`src/tui/browser.rs`), whose Enter opens the whole action menu
(`src/tui/actions.rs:51`), which is precisely what the launcher flow must
not do. The interactivity gate convention is `src/cli/recent.rs:54-55` /
`src/cli/search.rs:83-84`: `stdout().is_terminal() &&
tty::prompt_available()`. Cancel convention: `prompt::report_cancelled`
(`src/tui/prompt.rs:347`) + `Ok(())`, as in `cli/new.rs`, `cli/apply.rs`,
`cli/register.rs`.

**Decisions.**
- `tui::pickers::pick_project(prompt: &str, candidates: &[Project], how: &str)
  -> Result<Option<Project>>`, mirroring `pick_template`:
  `tty::require_tty("pick a project", how)?`, rows via
  `RowWidths::measure` + `rows::project_row(p, &widths, None, true)` +
  `clamp_label(_, terminal_columns())`, selection via
  `prompt::select_with_theme(..., &ProjectRowTheme::new(terminal_columns()))`.
  **Not** `live_select`: the candidate list is static (no sizes land later),
  already narrowed by the query, and `live_select` carries three
  load-bearing caller obligations this list does not need.
- One shared flow, used by all three verbs — new `src/cli/target.rs`:
  `pub fn one_project(cfg, query, prompt, how) -> Result<Option<Project>>`
  matching on `Resolution`: `One` → `Ok(Some)`, `Many` + interactive gate →
  `pick_project`, `Many` otherwise → `Err(ambiguous_error(..))`, `NoMatch`/
  `NoProjects` → their errors. `Ok(None)` means cancelled; the caller prints
  `report_cancelled("nothing was opened" / "…copied" / "no path printed")`
  and returns `Ok(())` — exit 0.
- `cli::recent::open` is rewired through `one_project` (behaviour change for
  terminals: picker instead of error — the point of the phase).
- The `how` string names the escape: `` "give a full ID, e.g. `fastf copy ID0037`" ``.

**Steps.**
1. `pick_project` in `src/tui/pickers.rs`.
2. `src/cli/target.rs` + rewire `open`, `copy`, `path`.
3. Pty tests in `tests/tui_pty/flows.rs` (drive **`fastf path`**, never
   `open` — `open` would spawn the real file manager on CI):
   `an_ambiguous_path_query_opens_a_picker_and_prints_the_choice`
   (`pty::run_stdout_to` captures the chosen path),
   `esc_on_the_ambiguity_picker_cancels_with_exit_0_and_says_so`.
4. `tests/cli_output.rs`: `an_ambiguous_open_still_errors_when_piped`
   (headless run, exact `Specify a full ID` text).
5. Docs: `open`/`copy`/`path` sections in `docs/cli.md` describe the picker
   and the piped behaviour.

**Acceptance.**
- [ ] All gates pass (layering enforces the picker's placement by itself).
- [ ] The pty picker test was observed failing on the pre-phase build.
- [ ] Piped ambiguity output is byte-identical to Phase 1's.
- [ ] Esc exits 0 and prints a `Cancelled —` line.

---

## Phase 4 — a terminal when there is no terminal

**Goal.** Launched fully headless inside a GUI session, fastf re-executes
itself inside a terminal emulator instead of printing to nowhere: bare
`fastf` (the menu), `fastf recent`, `fastf search`, and the ambiguous branch
of `open`/`copy`/`path`. Headless single matches stay direct: `copy` copies
and raises a desktop notification; `path` copies, notifies, **and** still
prints (a journal trace costs nothing). A relaunched window that only showed
text waits for Enter before closing; one that ran a picker or menu closes
immediately. A new `terminal` config key names the emulator.

**Evidence.** Nothing in the crate re-executes itself or spawns a terminal
(`current_exe` appears once, in the portable-mode probe). The no-TTY
convention is "refuse and name the escape hatch"
(`tests/cli_output.rs::refuses_without_a_terminal`) — this phase is a
deliberate, documented departure that must not leak into piped/scripted
contexts. Under a launcher on a systemd-managed desktop: stdin `/dev/null`
(chardev), stdout/stderr journald **stream sockets**, `WAYLAND_DISPLAY`/
`DISPLAY` set; `INVOCATION_ID`/`JOURNAL_STREAM` are useless discriminators
(the launcher's own service sets them for everything it spawns). Cron hands
its children **pipes**; `nohup` a regular file; the test harness pipes.
dialoguer prompts probe **stderr** (`src/util/tty.rs`), stdout decides
format.

**Decisions.**
- **The rule** (all must hold): (1) stdin, stdout, stderr all non-TTY;
  (2) `fstat` classifies both fd 1 and fd 2 as socket, character device, or
  closed (`EBADF` = provably no reader) — never a regular file or FIFO;
  (3) `WAYLAND_DISPLAY` or `DISPLAY` set; (4) `SSH_CONNECTION` unset;
  (5) `FASTF_RELAUNCHED` and `FASTF_NO_RELAUNCH` unset; (6) `--plain` not
  passed and the terminal preference is not `none`; (7) the command is one
  of the listed surfaces. Accepted misfires, documented with their escape
  hatches (`--plain`, `FASTF_NO_RELAUNCH=1`, `terminal = "none"`): a systemd
  user service running an interactive fastf command with the session display
  imported (byte-for-byte the launcher environment), and cron with
  `>/dev/null 2>&1` **plus** an exported display.
- **`src/util/relaunch.rs`**, `#[cfg(unix)]` (precedent: `shell_open.rs` is
  `cfg(windows)`). No printing — spawning from util is the `clipboard.rs`
  precedent. API: `pub fn headless_gui_session() -> bool`;
  `pub fn respawn_in_terminal(preference: Option<&str>) -> Result<()>`
  (Ok = a terminal owns the rerun; caller exits 0). Private, pure,
  unit-tested: `candidate_commands(pref, exe, args) -> Vec<Vec<OsString>>`,
  `stream_has_no_reader(fd) -> bool` (raw `libc::fstat` + `S_IFMT` — POSIX,
  allocation-free; `libc` is already a unix dependency).
- **Emulator table** (program, leading args; argv appended after):
  `konsole -e` · `gnome-terminal --` (its `-e` is deprecated and takes one
  string) · `xfce4-terminal -x` (its `-e` shell-parses one string — never
  use) · `alacritty -e` · `kitty` (trailing argv) · `foot` (trailing argv) ·
  `wezterm start --` · `xterm -e` (must be last option). Resolution order:
  config `terminal` → `$TERMINAL` → `xdg-terminal-exec` (the XDG default-
  terminal resolver, trailing argv) → table probe. A configured/`$TERMINAL`
  name is matched by basename against the table for its arg style; unknown
  names get the xterm-compatible `-e` convention. A value with embedded
  arguments is not supported — it names a program.
- **Spawn**: `current_exe()` (fallback `args_os().next()`) +
  `args_os().skip(1)` as `OsString`s — argv is passed as argv, never through
  a shell. `.process_group(0)`, all stdio `Stdio::null()`,
  `.env("FASTF_RELAUNCHED", "1")`, `spawn()`, drop the child, return. Every
  candidate is tried in order; total failure → `Err`, caller falls through
  to today's plain behaviour and best-effort-notifies. Inside the relaunch,
  rule (5) makes the guard self-limiting: still no TTY → plain behaviour.
- **Config**: `Config` gains `#[serde(default)] pub terminal: String` and
  `resolve_terminal() -> TerminalPreference { Disabled, Named(String), Probe }`
  (`"none"` → Disabled; empty → `$TERMINAL` else Probe) — never read the
  field raw. Wire `terminal` into `cli::config::set`'s match + valid-keys
  string, `config show`, and Settings → Basics via `set_from_prompt`
  (`src/tui/menu.rs` pattern). The CLI passes the resolved preference into
  `util::relaunch` — util does not read `Config` (layering).
- **`src/util/notify.rs`** (`cfg(unix)`): `pub fn notify(summary, body) ->
  bool` — spawns `notify-send -a fastf` when present, `Stdio::null()`,
  best-effort. Promote `clipboard::which` to
  `util::paths::find_on_path(name)` — clipboard, relaunch and notify all
  need it.
- **Pause-before-close**: `static SURFACE_RAN: AtomicBool` in
  `src/util/tty.rs` + `mark_interactive_surface()` /
  `interactive_surface_ran()`. Set in exactly two choke points:
  `require_tty`'s success path (covers every dialoguer prompt via
  `prompt::ready()` and every picker) and `live_select::select_live` after
  its `is_term()` check (the browser bypasses `require_tty`). Read only in
  `main.rs`: after the run (success or printed error), if
  `FASTF_RELAUNCHED` is set, no interactive surface ran, interrupt is not
  set, and stdin+stderr are TTYs → `press Enter to close…` + `read_line`.
  Cooked mode, after `restore_terminal`, so no ordering hazard with the
  interrupt module. Accepted tradeoff: a picker-driven `copy`'s ✓ line
  flashes — the clipboard already has the payload.
- **Wiring**: `recent`/`search` — where the existing `interactive` gate is
  false, before discovery: if `!plain && headless_gui_session()` →
  `respawn_in_terminal` and return. `target::one_project` — the `Many` arm
  gains the same branch ahead of the error. Bare `fastf` — in `main.rs`
  before the menu's `require_tty`. `copy`/`path` single match —
  `headless_gui_session()` → degrade as in the Goal.
- **Windows**: none of this module exists (`cfg(unix)` declarations);
  launching a console exe from the Win+R/launcher surface already allocates
  a console. Windows clippy `-D warnings` must stay clean.

**Steps.**
1. `util::relaunch` + `util::notify` + `find_on_path` promotion + unit tests
   (`candidate_commands` exact argv per emulator; `stream_has_no_reader`
   against a pipe = false, a tempfile = false, `/dev/null` = true).
2. Config key end to end (struct, resolver, `config set`/`show`, Settings
   menu, `ConfigAction::Set` after_help).
3. The pause mark + `main.rs` hook.
4. Wire the five surfaces.
5. Harness: `Sandbox::run_null(args, env)` in `tests/common/mod.rs` —
   `Stdio::null()` on all three streams, caller injects a fake
   `DISPLAY=:99`; plus a recorder fixture (`#!/bin/sh` script writing `$@`
   to a file, `chmod 755`). **Harness rule, added to `tests/CLAUDE.md`:
   relaunch tests always pin `config set terminal <recorder>` so CI can
   never spawn a real emulator.**
6. New `tests/relaunch.rs` (`#![cfg(unix)]`), each written to fail first
   where behaviour changes:
   `a_piped_stdout_never_relaunches_even_with_a_display`,
   `a_null_stdio_gui_session_relaunches_through_the_configured_terminal`
   (recorder file holds the exe + original argv; parent exited 0, printed
   nothing), `no_relaunch_env_and_terminal_none_both_suppress_it`,
   `ssh_connection_suppresses_it`,
   `a_relaunched_child_with_no_tty_falls_through_to_plain_output` (loop
   guard: `FASTF_RELAUNCHED=1` preset, recorder untouched),
   `path_headless_gui_copies_and_notifies_but_still_prints` (recorder
   shadowing `notify-send` on `PATH`).
7. Pty tests (`tests/tui_pty/flows.rs`):
   `a_relaunched_run_with_nothing_interactive_waits_for_enter`
   (`FASTF_RELAUNCHED=1`, empty library, `fastf recent` → expect the pause
   line, send Enter, exit 0) and
   `a_relaunched_run_that_showed_a_picker_does_not_wait`.
8. Docs, same commit: `docs/cli.md` — the "Prompts and terminals" section
   gains the headless-GUI carve-out; the line claiming `recent`/`search`
   "retain their existing command-line output" is amended with the exact
   suppression conditions; the config-keys table gains `terminal`; an
   environment-variables note gains `FASTF_NO_RELAUNCH` (public) and
   `FASTF_RELAUNCHED` (internal). `ROADMAP.md` — the product-contract
   section gains a line for what fastf may now spawn (a terminal emulator
   named by config/`$TERMINAL`/`xdg-terminal-exec`/probe; `notify-send`).
   `docs/windows.md` — one sentence: no relaunch machinery on Windows.

**Acceptance.**
- [ ] All gates pass.
- [ ] The recorder test proves the exact re-exec argv and that the parent
      printed nothing and exited 0.
- [ ] The loop-guard test proves a relaunched no-TTY child behaves like
      today's build.
- [ ] `fastf search <term> | cat` and `fastf path <id> > f` behave
      byte-identically to v2.0.1.
- [ ] Windows clippy leg clean with the `cfg(unix)` modules absent.
- [ ] **Needs the maintainer:** a real launcher smoke test — Alt+Space:
      `fastf search <term>`, an ambiguous `fastf open <term>`, a
      single-match `fastf copy <term>`, bare `fastf`. CI cannot do this.

---

## Phase 5 — sweep and release prep

**Goal.** The cross-cutting reading pass no single phase owns, then the
release is ready to cut.

**Steps.**
1. Read `docs/cli.md`, `docs/projects.md`, `docs/windows.md`, `README.md`
   top to bottom against the shipped behaviour; fix drift. README mentions
   `open`/`search` in prose only — keep the command reference in `docs/`.
2. `ROADMAP.md`: release-train entry for v2.1.0; confirm the backlog items
   this plan absorbed are gone.
3. Confirm completions and man pages need no hand edits (both derive from
   clap).
4. Root `CLAUDE.md` + `src/core/CLAUDE.md`/`src/tui/CLAUDE.md`: record the
   landed decisions (Resolution enum, the relaunch rule and its accepted
   misfires, the picker-serves-the-verb rule) where they now belong.
5. Cut v2.1.0 with the `release` skill (version bump, tag, Release
   workflow, MSI, AUR bumps).

**Acceptance.**
- [ ] All gates pass on the release commit.
- [ ] **Needs the maintainer:** approve the release notes and run the
      launcher smoke test from Phase 4 on the installed build.

---

## Release — v2.1.0

Minor version: new commands (`copy`, `path`), new resolver tier, new
interactive behaviour, one new config key; no breaking changes (`--plain`
and piped output are contractually unchanged). Notes must name: the two new
commands and the `path`/`paths` distinction; numeric ID queries; the
ambiguity picker; the launcher behaviour and all three ways to turn it off
(`--plain`, `FASTF_NO_RELAUNCH=1`, `terminal = "none"`); the clipboard
process-group hardening.

## Parking lot

- A picker for `move`/`tag`/`note` ambiguity (they gained the numeric tier
  only).
- A native KRunner DBus-runner plugin — search-as-you-type from Alt+Space
  without spawning fastf per keystroke. Separate deliverable, likely its own
  repo.
- `--json` output and the rest of the Scriptability backlog.
- `reveal_folder` on unix blocks on `.status()` — a foreground file-manager
  handler would hang a headless `open`. Consider detaching like the
  relaunch spawn.
- `ptyxis` (GNOME 47+ default) in the emulator table if anyone asks.
- A watchdog for a clipboard tool that does not fork (`wl-copy --foreground`
  shape): `feed`'s `wait()` currently has no timeout.

## Phase log

(One line per finished phase: date, PR, what differed from the plan.)
