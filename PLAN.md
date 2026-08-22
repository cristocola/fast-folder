# fastf improvement plan

Seventeen phases in three tracks, each sized for one agent session. Track A ships as v1.6.1, Track B as v1.7.0, Track C as v1.7.1. The plan came out of a full audit on 2026-08-21 (baseline v1.6.0: all gates green, 355 tests). Every defect cited here was reproduced or read in the source at the stated line before it was written down; line numbers are as of `f52fcca` and will drift, so search for the symbol if a number misses.

## How to run a phase

You are one agent doing one phase in one session. Do not start the next phase.

1. Read this file top to bottom once (Ground rules, your phase, Parking lot). Then read `CLAUDE.md` fully, `tests/CLAUDE.md`, and `src/ui/CLAUDE.md` if your phase touches `src/ui/`.
2. `git checkout main && git pull`, then branch `phase-NN-<slug>`. Run the baseline gates (below) and confirm green before changing anything.
3. Write your own sub-plan before editing: which files, which tests first, which docs. The phase gives you the design decisions that are already made and the acceptance criteria; the sequence of edits is yours. Put the sub-plan in the PR description.
4. For every defect fix, write the failing test first and watch it fail on the unmodified build (`tests/CLAUDE.md` explains why: two past "regression" tests passed before the fix). Process-level behaviour gets a `tests/cli_surface.rs` case; interactive behaviour gets a `tests/tui_pty.rs` case.
5. Tick each checkbox in this file as you complete it, in the same commit as the work. If a box cannot be done, leave it unticked and say why under the phase in a `Notes` line. Never tick a box you did not verify.
6. Finish with the gates, the docs (`docs/` not README, per `CLAUDE.md`), a one-line entry for your phase under the current release in `ROADMAP.md`, and `CLAUDE.md` updates only for decisions that landed. Commit, push, open the PR, stop. No check-ins mid-phase; no questions that block. If you discover a defect outside your phase, add it to the Parking lot with `file:line` and move on.
7. Release checkpoints are separate sessions that use the `release` skill. A phase never bumps the version or tags.

Gates (all must pass; run them exactly like this):

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo test --release
node --check src/ui/web/app.js
RUSTDOCFLAGS=-D warnings cargo doc --no-deps --locked
cargo check --all-targets --target x86_64-pc-windows-gnu   # if the target is installed locally
```

## Ground rules (the missions, stated as constraints)

- **No data loss, ever.** A source is never removed before a verified destination exists. Every write of config, counter, metadata, template, or journal goes through `util::atomic`. Every read-modify-write of shared state holds `util::lockfile::DataLock`, and the lock is never held across a prompt, an editor, a reveal, or a post-create command. A cached `Project` is a hint; authority is re-read under the lock. Mutations go through `core::operations`.
- **Quick.** Nothing on the interactive path blocks on work the user did not ask for. Sizes stay off the critical path (`util::size_scan`). Discovery reads the cache, not every `PROJECT_INFO.md`, unless the base changed. Measure before claiming a speedup; debug-only counters exist from Phase 9 (`util::trace`).
- **TUI first.** The guided menu is the daily surface. Its polish matters more than the browser UI's. Cancel is always possible, typed input is never thrown away by a later validation failure, and a network-share stall is never a frozen screen.
- **Two operating systems, one library.** Linux and Windows mount the same bases. Keep the counter monotonic and converging (`Counters`), keep `.fastf-index.json` portable, and treat the Windows console as the constraint: picker labels single-line, ANSI-free, clamped (`clamp_label`); `\\?\` stays in paths and is stripped only for display; sharing violations are retried (`util::fs_retry`).
- **Compatible.** CLI flags, `template.yaml`, `.fastf-index.json`, `PROJECT_INFO.md`, and HTTP JSON shapes stay compatible. Rejecting input that was silently mishandled before is allowed and encouraged.
- **Honest output.** Headers and summaries say what happened. A preview is labelled a preview, a dry run a dry run, an error an error (exit 1 with a path and a next step). A warning is never the only signal that a request was ignored.
- **Docs with behaviour.** A behaviour change lands with its `docs/` update. README stays the compact front door. `CLAUDE.md` records decisions after they land, not plans. Minimal em dashes and comma chains in user-facing text.
- **Tests are processes when the bug lived between clap and the core, ptys when it lived in a prompt, and units when the logic is pure.** New harnesses redirect `FASTF_INSTALL_DIR` and `HOME`/`USERPROFILE` into the sandbox.
- **Release safety.** Never run `paru -S`, `pacman -S`, `makepkg -i/-s`, or anything that mutates installed packages. Build-only validation is fine.

## Phase 0 (Cristo, not an agent)

- [x] Commit the in-flight `CLAUDE.md` trim and the untracked `tests/CLAUDE.md` so Phase 1 starts from a clean tree. Phase 17 assumes the "How it got here" and "Testing" sections are already gone from `CLAUDE.md`.

---

# Track A: correctness and hygiene (release v1.6.1)

## Phase 1: Honest output and honest errors

**Goal.** Remove the three places where fastf's output contradicts what it did: a swallowed config error, a "dry run" header on real creates, and a settings menu that writes stale state.

**Evidence.**
- Twenty sites call `Config::load().unwrap_or_default()`: `src/cli/recent.rs:38,547,931`, `src/cli/search.rs:43`, `src/cli/tag.rs:21,38,66,106`, `src/cli/note.rs:39,85`, `src/tui/menu.rs:53,66,201,244,268,581,639,702,732`, `src/core/operations.rs:545`. `Config::load()` already returns `Ok(default)` when the file is absent (`src/core/config.rs:124-134`), so every `unwrap_or_default()` hides a real parse or I/O error. Reproduced: append `this is = not [valid toml` to `config.toml`; `fastf recent --plain` prints "No projects yet" and exits 0; `fastf tag list ID0001` says "no projects found"; the TUI header shows the home directory as the base. `fastf new` and `config show` report the error correctly.
- `src/core/project.rs:124-130` (`print_dry_run`) and `:775-781` (`print_apply_plan`) hard-code the header `Preview · dry run — nothing will be created`. `src/cli/new.rs:67` and `src/cli/apply.rs:98` call them on the real path, before the confirmation. `menu_apply` (`src/tui/menu.rs:289-311`) shows it twice, once truthfully.
- `src/tui/menu.rs:639-692` (`menu_settings_bases`) loads config, prompts, then writes the whole `bases` list from the stale copy and removes by index. This is the bug `edit_postcreate_commands` was fixed for ("Prompt first, then lock, then reload" in `CLAUDE.md`); a base added by the browser UI meanwhile is reverted.
- `src/util/interrupt.rs:71` exits 130 from the second Ctrl-C without restoring the cursor; `main.rs:657-666` (`restore_cursor`) is not reachable from there.

**Design decisions (made).**
- A config that exists but does not parse is an error everywhere, including the TUI, which prints it and exits 1. Falling back to defaults changes which directory is the library; that is a correctness failure, not resilience. Keep the existing context message ("parsing `<path>`: TOML parse error ...") and append one hint line at the `main.rs` error printer when the error chain contains "parsing" + `config.toml`: `hint: fix the file, or delete it to start over with defaults`.
- `print_dry_run` and `print_apply_plan` take a `PreviewKind { DryRun, BeforeCommit }` (or a `dry_run: bool`). `DryRun` keeps the current header verbatim. `BeforeCommit` prints `Preview` (the confirm prompt follows). The TUI two-pass apply prints the dry-run header on pass one and the plain header on pass two.
- `menu_settings_bases` prompts for the value first, then calls `core::operations::update_config` (`src/core/operations.rs:553`, which locks, reloads, mutates, saves) with a mutator that validates the entry via `config::expand_base_path` and pushes/removes by text. The remove item carries the base text, not an index.
- Move `restore_cursor` from `main.rs` into `util` (for example `util::interrupt::restore_terminal`) guarded by `is_terminal` (the v1.3 gotcha), and call it before `process::exit(130)` in the signal path as well as from `main.rs`.

**Out of scope.** Non-TTY prompt guards (Phase 2), anything in `core/project.rs` beyond the header parameter (Phase 12 moves rendering out of core).

**Steps.**
- [x] Add `tests/cli_surface.rs` cases first: with a corrupt `config.toml`, `recent --plain`, `search x --plain`, `tag list <id>`, `notes <id>`, and `reconcile` exit 1 with the config path on stderr and nothing on stdout. Add a `tests/tui_pty.rs` case: the TUI prints the parse error and exits 1 instead of showing a menu.
- [x] Replace every `Config::load().unwrap_or_default()` (and any `.ok()`/`.unwrap_or(...)` on `Config::load`) with `?`, including inside the browser loader closures in `menu.rs` and `recent.rs`. Audit `src/ui/mod.rs` for the same pattern and make it a 500 with the message, not defaults.
- [x] Add the one-line hint in `main.rs`'s error printer.
- [x] `tests/cli_surface.rs`: `new general --name=x --yes` output does not contain "nothing will be created"; `new ... --dry-run` does; same pair for `apply`. Then add `PreviewKind` to both printers and fix the callers (`cli/new.rs`, `cli/apply.rs`, `tui/menu.rs`).
- [x] Rewrite `menu_settings_bases` as prompt, then `operations::update_config`, with text-based removal. Add a pty test: add a base, and concurrently (between prompt and Enter) `fastf config set bases ...` from a second process; both entries survive.
- [x] Cursor restore on the second Ctrl-C path; keep the `is_terminal` guard (verify with `cat -v` of a piped failure that no `\x1b[?25h` leaks).
- [x] Docs: `docs/cli.md` gains one sentence under config: a config that fails to parse stops every command until fixed. `CLAUDE.md`: replace the "Prompt first, then lock, then reload" example list with both functions.

**Acceptance.** All gates green. The five corrupt-config cases fail loudly. No real create or apply prints "dry run". A concurrent config write during the bases prompt is not reverted.

**Notes.** The `src/ui` audit found no `unwrap_or_default` at all — all twelve loads were already `?`. What was wrong there was the status: a config parse failure was reported as 400. `ui::load_config` now tags it with a new `SERVER_ERROR_PREFIX` so `status_for` answers 500, asserted the way the suite already asserts a status (on the message prefix). The second-Ctrl-C path is driven by a template whose post-create command ignores SIGINT, which keeps fastf blocked long enough for the second signal to land there. Removal by text got its own pty case as well (`removing_a_base_leaves_the_rest_of_a_concurrent_edit_alone`), since "the item carries the text, not an index" is only observable once something else has edited the list; both concurrency cases were watched failing against the old code. `tui/menu.rs` needed no `PreviewKind` change: its two-pass apply calls `apply::run` with `dry_run` true and then false, so it already gets the dry-run header on pass one and the plain one on pass two — both are the paths `a_real_apply_is_not_labelled_a_dry_run` covers.

## Phase 2: Flags anywhere on the line, and prompts that know when there is no terminal

**Goal.** Make every declared flag work in every position for `new`/`apply`/`register`, and make every interactive prompt fail with an actionable message instead of a raw dialoguer error when there is no terminal.

**Evidence.**
- `src/main.rs:50-79, 231-306` declare `New`/`Apply`/`Register` with `trailing_var_arg` `extra`. Clap parses known flags normally until the first unknown `--var=value`; after that every token lands in `extra`. `classify_extra` (`src/cli/new.rs:206-279`) knows five flags; only the `New` arm (`main.rs:702-706`) OR-combines all five. `Apply` (`:823-824`) and `Register` (`:777,784`) keep only `dry_run` and `yes`. Register's own flags are not known to the recognizer at all. Reproduced: `fastf register <path> --template=general --name=Legacy --rename --yes` prints `warning: unrecognized flag '--rename' — ignored` and registers without renaming. `--base-dir /path` (space form) yields two nonsense warnings. `docs/cli.md:49` says flags work before or after the slug.
- Prompts without a non-TTY guard: `src/cli/apply.rs:103`, `src/cli/register.rs:213` (rename confirm), `src/cli/template.rs:258-274` (`from-folder --bundle-assets` confirm; no `--yes` exists), `src/cli/new.rs:182-186` (template picker). `src/cli/new.rs:68-73` shows the right message shape.
- Every prompt-availability guard tests `std::io::stdout().is_terminal()`; dialoguer prompts on stderr (`util/live_select.rs:54` uses `Term::stderr()`). `fastf new t > out.txt` refuses although the terminal is available; `fastf new t 2>/dev/null` passes the guard and hangs.

**Design decisions (made).**
- Keep the `--slug=value` variable syntax and the `extra` mechanism (clap cannot accept arbitrary unknown `--key=value`). Replace the hand-written recognizer with one derived from clap: `classify_extra(extra, &clap::Command)` reads the subcommand's declared arguments (long, short, takes-value) and produces `recognized: Vec<(String, Option<String>)>` plus `vars`. Rules: a declared flag in `--flag`, `--flag=value`, `--flag value`, `-s value` forms is recognized; `--key=value` with an undeclared key is a variable; `--key` with no `=` and not declared is an **error** ("unknown flag `--key`; variables are passed as `--key=value`"); a stray positional is an error ("unexpected argument"). Do not use `Command::ignore_errors` (documented as unstable).
- Each arm applies recognized flags through one `apply_extra(&mut XArgs, recognized)` per command whose `match` is exhaustive with a trailing `_ => bail!("flag {name} is declared but not handled after the positional")`. Add a unit test per command that iterates `Cli::command().find_subcommand(..).get_arguments()` and calls `apply_extra` for every long flag, so a flag added to clap but not to the arm fails the test instead of being dropped.
- Add one helper, for example `cli::prompting::require_tty(what: &str, how: &str) -> Result<()>`, that probes **stderr** for prompt availability and produces the `new.rs:68` message shape. Use it in every prompt site in `src/cli/` (template picker, apply confirm, register rename confirm, from-folder bundle confirm, move confirm/picker). Output-format decisions (`recent --plain`, `search --plain`) keep probing stdout.
- `template from-folder` gains `-y/--yes` (accept the bundle prompt) and `--dry-run` (print the `ScanResult` the command already computes at `template.rs:256`, write nothing).
- `warn_unknown` (`main.rs:868-876`) goes away; classify's errors replace it.

**Out of scope.** A new `--var k=v` syntax (breaking), JSON output, completions.

**Steps.**
- [x] `tests/cli_surface.rs` first: `register <path> --template=general --name=X --rename --yes` renames; `register <path> --name=X --recursive --dry-run` previews (with a base path); `apply <t> <dir> --name=x --no-post --yes` runs no post-create (use a `commands` entry that writes a sentinel file); `new general --base-dir <dir> --name=x --yes` (space form) lands in `<dir>`; `new general --nope --yes` exits 1 naming the flag; `new general --name x --yes` exits 1 suggesting `--name=x`.
- [x] Rewrite `classify_extra` as designed; delete `ExtraFlags`; add `apply_extra` per command with the exhaustiveness test.
- [x] `tests/cli_surface.rs`: with stdin and stderr redirected away from a TTY, `apply`, `register --rename`, `template from-folder --bundle-assets`, and `new` (no slug, no default template) exit 1 with the "no terminal" message and the flag that avoids the prompt. With a pty on stderr but stdout redirected, `new <slug> --name=x` prompts and completes.
- [x] Add `require_tty` and route every `src/cli/` prompt through it; switch prompt-availability probes to stderr.
- [x] `template from-folder --yes` and `--dry-run`; thread `yes` into `run_from_folder`.
- [x] Docs: fix `docs/cli.md:49` (variables go after the slug; declared flags work in any position); document `template delete --yes`, `from-folder --force/--yes/--dry-run`, `ui --address`; reconcile the two completion recipes (`docs/cli.md:292` vs `main.rs:389`) into one. `CLAUDE.md`: replace the "three coordinated edits" gotcha with the new rule (declare in clap, handle in `apply_extra`, the test catches the rest).

**Notes.** Two design decisions came out differently and one item grew. (1) The
apply case in step 1 was written as "`apply <t> <dir> --name=x --no-post --yes`
runs no post-create": `apply` never ran post-create actions and does not declare
`--no-post`, so that case could not fail against the old build. It became
`apply_refuses_a_flag_it_does_not_declare` (the same defect from the other side:
a flag that does nothing is not silently accepted) plus a `-y`-after-the-target
design guard. (2) `register --recursive` dropped the variables typed on the line,
so a template with required variables could not be used for bulk onboarding at
all; `RecursiveArgs` gained `vars`. (3) `require_tty` lives in `util::tty`, not
`cli::prompting`, because `core::vars::collect_vars` needs it and core cannot
import cli (Phase 6 moves `collect_vars` into `tui/`; the helper does not move
with it). (4) One prompt was not on the list and mattered most: `fastf move`
skipped its confirmation when stdout was not a terminal and moved the folder — it
now refuses without `--yes`. (5) `recent`/`search` keep their stdout format probe
but also fall back to the plain list when the picker could not be drawn, instead
of waiting for a key on an invisible list. `register_recursive_dry_run_after_the_path_previews_the_children`
passes pre-fix (clap parses declared flags normally until the first token it does
not know) and is labelled a design guard, per `tests/CLAUDE.md`.

**Acceptance.** Every declared flag works in any position for the three commands, proven by the exhaustiveness test plus the process cases. No `src/cli/` prompt can produce a raw `IO error: not a terminal`.

## Phase 3: Files fastf writes must stay readable

**Goal.** Close the paths where a mutation destroys data fastf did not own, or writes a file it will later refuse to read.

**Evidence.**
- `src/core/project_info.rs:70-94`: `Metadata` has no `#[serde(flatten)]` catch-all. `write_frontmatter` (`:288`) parses into `Metadata` and re-serializes it, so `tag add/remove/reauto`, `move`, `rename`, `register --created`, and `mark/clear_provisioning` delete every unknown frontmatter key. Realistic trigger: Linux fastf 1.7 writes a new key, Windows fastf 1.6 runs `tag add`. The body has a byte-identity test (`CLAUDE.md` v0.4); the frontmatter has none.
- `src/core/project_info.rs:142`: `serde_yaml::to_string(&meta).unwrap_or_else(|e| format!("# yaml-serialize-error: {e}\n"))` produces valid delimiters around a comment-only document; `read_project_meta` (`src/core/library.rs:277`) then errors and the project is invisible to discovery.
- `src/core/template.rs:239,256`: bare `fs::write` for `template.yaml` and each file under `files/`; `from-folder --force` clears `files/` first. `util::atomic` exists because the same pattern truncated config and counter files on crash.
- `src/core/counter.rs:129`: `let _ = local.save()` drops the data-dir counter write silently (the one that prevents an unplugged base restarting numbering); its per-base siblings at `:95,120` warn.
- `src/core/library.rs:1112`: `let _ = fs::rename(&staging, &project.path)` is the case-only-rename rollback; if it fails the project is stranded under a dot-prefixed name that discovery skips, silently.
- `src/core/provisioning.rs:874`: `faults::check("move:after-source-cleanup").ok()` discards the injected error, unlike every other `check` in the file; the failpoint cannot fire.
- `src/core/provisioning.rs:371`: `pub fn reconcile` mutates without `DataLock`; only `reconcile_locked` (`:392`) honours the documented invariant.

**Design decisions (made).**
- `Metadata` gains `#[serde(flatten)] pub extra: BTreeMap<String, serde_yaml::Value>`. `BTreeMap` keeps output stable. Verify the existing byte-identity tests still pass for files without extras (no field is emitted when the map is empty). Do the same evaluation for `Template` (`template.yaml` is user-owned and rewritten by the TUI/UI editors); add the flatten there too if the gallery and round-trip tests stay green.
- `render` returns `Result<String>`; callers already sit in `Result` contexts.
- `save_to_file` uses `util::atomic::write` for the manifest and for every file under `files/`.
- The counter data-dir save warns like its siblings. The case-only-rename rollback failure returns an error that names the stranded path and the original failure. The failpoint uses `if let Err`.
- `provisioning::reconcile` becomes `pub(crate)` (or `#[doc(hidden)]` with an `_unlocked` suffix if a test needs it; prefer switching the test to `reconcile_locked`).

**Steps.**
- [x] `tests/integration.rs` first: write a `PROJECT_INFO.md` with an extra top-level key and a nested extra map; run `tag add`, `rename`, and `move` through `core::operations`; the keys survive byte-for-byte in value and order. Confirm the existing no-op round-trip test is unchanged.
- [x] Add `extra` to `Metadata` (and `Template` if green). *(Done differently — see Notes. Both files preserve unknown keys; neither type gained a field.)*
- [x] `render -> Result`; a unit test feeding a value serde_yaml cannot emit (if one exists; otherwise a test that the error type propagates from `write`).
- [x] Atomic template writes; a `crash_recovery.rs` failpoint around `save_to_file` (new fault point `template:mid-save`) proving a kill leaves either the old or the new manifest, never a truncated one.
- [x] Counter save warning, rollback error, failpoint fix, `reconcile` visibility.
- [x] Docs: `docs/projects.md` states that unknown frontmatter keys are preserved across fastf mutations. `CLAUDE.md` "PROJECT_INFO.md" section gets one line on `extra`.

**Acceptance.** Unknown frontmatter keys survive every mutation. A serialize error fails the create. A killed template save never leaves a truncated manifest. No `pub` function under `core/` mutates shared state without `DataLock` unless its name says so.

**Notes.** The one design decision came out differently, and two tests are
labelled design guards rather than regressions.

(1) **`#[serde(flatten)]` was rejected after reading the vendored sources.**
`flatten` routes every field through serde's `Content` buffer
(`serde-1.0.229/src/private/de.rs:1255`), and serde_yaml resolves a plain
unquoted scalar to a typed value (`serde_yaml-0.9.34/src/de.rs:1472`), so
`year: 2026` in a hand-edited file would arrive as an integer and be rejected by
the `String` field it belongs to. `library::read_project_meta` drops that error,
so the project would vanish from discovery — this phase would have traded one
invisible-project bug for another. It also sorts unknown keys to the end of the
file, which step 1 explicitly forbids. Instead `util::yaml::to_string_preserving_unknown`
merges the fresh struct onto the parsed `serde_yaml::Mapping` (an `IndexMap`, so
positions hold), driven by an `OWNED_KEYS` const per type with an exhaustiveness
test. Parsing is untouched, so there is no new failure class, and neither
`Metadata` nor `Template` gained a field — no struct literals changed and no
HTTP JSON shape moved. `Template::OWNED_KEYS` must keep listing `files` and
`dir`: without them a pre-v0.8 flat `files:` block would start being *preserved*
instead of dropped.

(2) **`render`'s `Result` has no reachable serialize failure**, since every
`Metadata` field is a `String`/`Vec`/`BTreeMap`/`bool` that serde_yaml can always
emit. What the phase actually needed proving was the consequence the old
fallback had, so it got a proptest instead: arbitrary hostile variable values
render to frontmatter that reads back to the same values
(`rendered_metadata_always_reads_back`).

(3) **`template:mid-save` is a design guard, not a regression test.** A failpoint
can only be placed around `fs::write`, never inside the window where it has
truncated the file and not yet written the bytes, so the case cannot be made to
fail against the pre-fix build. It pins what is checkable: a hard `abort` at that
boundary leaves a loadable manifest and no `.tmp` scaffolding, and the child's
stderr is asserted so the test cannot pass vacuously. Same for
`a_case_only_rename_that_cannot_commit_restores_the_project` — a real occupied
target reaches the rollback deterministically, but making the *rollback itself*
fail needs both renames to fail at once, so the stranded-folder message is pinned
by a unit test on `stranded_rename_message` instead.

(4) **`reconcile` became `#[doc(hidden)] pub fn reconcile_unlocked` rather than
switching the tests to `reconcile_locked`**, which this phase preferred:
`reconcile_locked` reloads `Config` from disk, and the twelve test call sites
build their config in memory and never save it, so they would have reconciled the
wrong base. The rename satisfies the acceptance verbatim and matches the rule
Phase 5 states.

(5) Two items grew slightly. `ALL_FAULT_POINTS`'s doc comment claimed an
invariant test iterated it and asserted agreement with the call sites; nothing
referenced the list at all, so the source-scan test now exists
(`every_failpoint_in_the_source_is_declared_and_vice_versa`). And
`bootstrap.rs`'s two bundled-template `fs::write`s went atomic alongside
`template.rs`'s, since an interrupted first run leaving an unloadable bundled
template is the same defect.

(6) **One pre-existing flake was fixed rather than parked**, because a gate that
fails one run in twelve is not green. `tui::menu`'s two "recoverable" unit tests
read the process-global interrupt flag through `is_fatal`, which
`util::interrupt`'s own tests raise, and they did not take `interrupt::TEST_LOCK`
— the rule `tests/CLAUDE.md` states for exactly this. Measured at 1/12 failures
on the unmodified build and 0/15 after; the new lib tests in this phase only
changed the scheduling that exposed it.

(7) The counter-warning case is `#[cfg(unix)]`: a read-only data directory is
what makes only the *write* fail, and planting a directory in place of
`counters.toml` (the cross-platform trick) breaks the read first, which already
propagates correctly.

## Phase 4: Browser-server hardening, CI gates that match the docs, and the release procedure in git

**Goal.** Fix the two server findings, make CI run what four documents claim it runs, and stop the release procedure from living on one machine.

**Evidence.**
- `src/ui/mod.rs:2093,2109`: `header_end + content_length` with `content_length` unbounded; overflow panics in `read_request`, which runs before `route_request_caught`'s `catch_unwind` (`:393-405`).
- `src/ui/mod.rs:500-505` → `open_path` (`:1915-1931`): authorizes by `path.exists()` and spawns `xdg-open`/`open`/`cmd /c start`; every sibling route uses `find_project` (`:1419-1426`). No test mentions `api/open`. `GET /api/project?path=` (`:1363`) reads an arbitrary path.
- `write_response` (`:2140`) sets `Cache-Control` and `nosniff` but no `Content-Security-Policy`.
- `.github/workflows/ci.yml`: `lint` runs on ubuntu only, so `#[cfg(windows)]` code is never clippy-checked; `README.md:193` says CI runs both. `node --check src/ui/web/app.js` is mandated by `README.md:170`, `CLAUDE.md` (twice), `src/ui/CLAUDE.md:243` and run by no job. `release.yml` never checks that the tag matches `Cargo.toml`'s version.
- `.gitignore:3` is `.claude` (no leading slash); `git ls-files .claude` is empty; `.claude/skills/release/SKILL.md` holds the release routine, the AUR safety boundary, and the WiX rules, and is referenced from `CLAUDE.md:73`. It contains no secrets.
- README: the suite table (`:176-184`) lists 7 of 9 suites; "about 3 MB" is 3.7 MB.
- `Cargo.toml` `[profile.release]` does not say that `panic = "abort"` would break the UI's panic-to-500 handling.

**Design decisions (made).**
- `read_request`: `if content_length > MAX_REQUEST_SIZE { bail!("request is too large") }` before the loop. Also stop rescanning the whole buffer for `\r\n\r\n` after every read (search from `len - 3`).
- `/api/open` and `/api/project`: authorize the path through a shared `authorize_local_path(cfg, path)` that accepts a canonical path equal to a discovered project's path, or inside the data dir (templates) if the frontend opens those. Read `app.js` for every caller of `/api/open` before deciding the allow set; do not break a working button. Unauthorized paths get 403 with the existing JSON error shape.
- CSP on the HTML response only if `app.js`/`index.html` contain no inline `<script>`, no `on*=` attributes, and no `javascript:` URLs. Check first. If inline handlers exist, park the CSP item (Parking lot) rather than shipping `'unsafe-inline'` for scripts. `style-src 'self' 'unsafe-inline'` is acceptable (inline `style=` exists).
- CI: add `node --check` to the lint job; add a clippy step for the Windows target (`rustup target add x86_64-pc-windows-msvc` then `cargo clippy --all-targets --target x86_64-pc-windows-msvc -- -D warnings`, or a second lint leg on `windows-latest`; pick whichever is cheaper in wall time and keep `rust-cache`). `release.yml`: first step compares `${GITHUB_REF_NAME#v}` to `Cargo.toml`'s version and fails on mismatch.
- `.gitignore`: keep `.claude/settings.local.json` ignored, un-ignore `.claude/skills/` (negation lines), and track the skill. Track `tests/CLAUDE.md` if Phase 0 has not.
- README fixes limited to the drift items; `Cargo.toml` gets a two-line comment above `[profile.release]`.

**Steps.**
- [ ] `tests/ui_server.rs` first: a request with `Content-Length: 18446744073709551615` gets a 4xx response (connection not dropped); `POST /api/open` with a path outside every base gets 403 and spawns nothing; with a discovered project path the route passes authorization (stub or skip the spawn on CI; test the authorization function directly for the positive case).
- [ ] Bound `Content-Length`; authorize `/api/open` and `/api/project`; CSP if the precondition holds.
- [ ] CI: `node --check`; Windows clippy; tag/version guard. Make the Windows clippy step pass (fix any `#[cfg(windows)]` lint it finds; that is the point).
- [ ] `.gitignore` negation; `git add .claude/skills/release/SKILL.md`.
- [ ] README: suite table (9 suites), CI claim, size. `Cargo.toml` comment. `docs/UI.md`: note the 403 and the CSP if added.
- [ ] Double-check `ROADMAP.md`'s "Release and documentation gates" list includes `node --check` (it does) and add the Windows clippy line.

**Acceptance.** CI runs every gate the docs name. The server has no panic path outside `catch_unwind`. `/api/open` cannot open an arbitrary path. The release procedure is in `git log`.

## Phase 5: Dead code out, stale gotchas corrected

**Goal.** Delete the superseded move engine and the uncalled wrappers, de-duplicate helpers, narrow `pub` items that have no external consumer, and make `CLAUDE.md` describe the code that exists.

**Evidence (all verified with grep across `src/` and `tests/`).**
- `src/core/assets.rs`: `find_links:325`, `copy_tree:337`, `jobs_for_tree:380`, `verify_tree:417` have zero production callers (only their own tests and `tests/windows_semantics.rs:237` for `find_links`). Doc comments still say "used by `library::move_project`'s cross-device fallback". `CLAUDE.md` v1.1 gotchas describe `verify_tree` as the verifier and name `refuse_if_contains_links`, which exists nowhere; the invariants now live in `src/core/transactions.rs` (`MoveManifest::verify_destination`).
- `src/core/library.rs`: five public move entry points; production uses `move_project_configured_with_outcome` (`operations.rs:541`) only. `move_project_with:553` and `move_project_configured_outcome:533` have zero callers anywhere; `move_project_outcome:526` only via `move_project`. `CLAUDE.md` v1.3 still says the CLI runs `move_project_with` on a scoped thread (it runs `operations::move_project`). Duplicate occupancy check at `:682-684` and `:693-695`; dead `_folder_label` at `:765`; back-to-back `set_phase("finalizing"); set_phase("done")` at `:724-725`.
- `src/core/project.rs:891`: `copy_template_files(verbose: bool)` is `false` at both call sites; the `println!` blocks at `:936-945` and `:969-971` are unreachable; comments at `:748,760` still say "printed".
- `src/core/provisioning.rs`: `reconcile_locked(_cfg)` dead parameter; `ReconcileReport.swept` (`:350`) is never written, so `is_empty` (`:361`) and `src/cli/reconcile.rs:49-53` have a dead branch; `let _ = metadata;` (`:510`); `write_create_marker:158`, `write_move_marker:198`, `staging_path:188` and the three `Legacy*` structs are `pub` writers for a format `CLAUDE.md` says must never be resurrected, used only by tests (`tests/ui_server.rs:967`, `tests/hostile_fs.rs:194`, `library.rs:1742` test).
- Duplicates: `require_real_file` byte-identical at `transactions.rs:611` and `provisioning.rs:938`; `validate_native_relative` at `:253`/`:926`; alias wrappers `transactions::write_json:607` and `project_info::atomic_write:443`; `naming::ensure_relative_safe_path:235` has one caller (`template.rs:245`).
- `library::max_id_in_base:1226` documents a per-base comparison no code performs.
- `pub` with no consumer outside its file: `library::cache_remove`, `max_id_in_base`, `Counters::base_floor`, `Counters::save_base`, `counter::BASE_COUNTER_FILE`, `project::suffixed_name`, `project::MAX_NAME_ATTEMPTS`, `template::strip_reserved_files`, `atomic::is_temp_file`/`TMP_SUFFIX`, `interrupt::interrupted_error`, `lockfile::lock_path`/`LOCK_FILENAME`, `fs_retry::ERROR_DIR_NOT_EMPTY`/`ERROR_LOCK_VIOLATION`, `assets::CANCELLED_MSG`, `provisioning::MARKER_CREATE`/`MARKER_MOVE_PREFIX`, `transactions::MANIFEST_FILE`.

**Design decisions (made).**
- Delete the four `assets.rs` functions and their tests; `tests/windows_semantics.rs:237` gets the same guarantee from the transaction path (a staged move refuses a junction/symlink) or from `transactions`' scanner directly.
- Keep `library::move_project` (the documented compatibility shape) and `move_project_configured_with_outcome`; delete the other three and fold `move_project_outcome`'s body into `move_project`. `delete_project`, `rename_project`, `unregister_project` (unlocked, test-only callers): switch `tests/windows_semantics.rs` to the `_configured` variants where possible; whatever remains gets `#[doc(hidden)]` and an `_unlocked` suffix. Rule going forward: no `pub` function mutates without `DataLock` unless its name says `_unlocked`.
- Legacy marker writers move behind `#[cfg(any(test, feature = "test-fixtures"))]`, or the two integration tests plant the marker bytes themselves (preferred: the tests then prove fastf never needs a writer).
- Hoist the duplicated helpers into `util` (for example `util::paths::require_real_file`, `util::paths::validate_native_relative`); delete the alias wrappers; inline `SafeRelativePath::parse` at `template.rs:245` and delete `ensure_relative_safe_path`.
- Remove `verbose`, `_folder_label`, `_cfg`, the `swept` branch (keep the JSON field if the UI reads it; the agent checks `app.js`), the duplicate occupancy check, and the unobservable phase write. Narrow the listed `pub` items to `pub(crate)` or private; `cargo doc` must stay clean (a `pub` doc may not link to a `pub(crate)` item).
- `CLAUDE.md`: rewrite the v1.1 "verify_tree / refuse_if_contains_links" gotcha to point at `transactions::MoveManifest`; fix the v1.3 `move_project_with` sentence; fix the counter paragraph that claims a per-base comparison (`max_id_in_base`) or implement nothing and delete the claim; update the `assets.rs` description in the layout list.

**Steps.**
- [ ] Rewire or rewrite the affected tests first (`windows_semantics.rs`, `integration.rs:2410`, `ui_server.rs:967`, `hostile_fs.rs:194`), keeping the guarantees they express.
- [ ] Deletions and narrowings as listed; `cargo clippy` and `cargo doc` clean.
- [ ] `CLAUDE.md` corrections; `ROADMAP.md` phase line.

**Acceptance.** `grep -rn "copy_tree\|jobs_for_tree\|verify_tree\|find_links\|refuse_if_contains_links\|move_project_with\b" src tests CLAUDE.md` returns nothing. Test counts drop only by the deleted `assets.rs` unit tests; every integration guarantee is preserved.

## Release checkpoint: v1.6.1

A separate session with the `release` skill. Before tagging: all gates, `ROADMAP.md` gets a `v1.6.1` row and section listing Phases 1-5 with PR links, `docs/` reflect Phases 1-4, and the Windows manual smoke from `ROADMAP.md` (same-drive rename, cross-drive move with the MSI) is still the one post-release item.

---

# Track B: the guided TUI (release v1.7.0)

Cristo's verdict on PR #3: Esc-to-back and the main-menu frame were right, the implementation was not (Esc worked in some places and not others). Open-in-editor and default-template preselect are not wanted. The track below makes cancel a single mechanism enforced by a test, keeps typed input through validation, stops the browser from rescanning, and then adds the frame, the keys, and the parity items.

## Phase 6: Relocate the terminal picker library (pure move)

**Goal.** Put the interactive-terminal code where it belongs so the next four phases edit one module set. No behaviour change.

**Evidence.**
- `src/cli/recent.rs` (1094 lines) is the shared picker library: `run_paged_browser:314` (callers: only `tui/menu.rs`), `project_action_menu:507`, `ProjectRowTheme:176`, `clamp_label:157`, `paged_labels:429`, `size_label:483`, `show_metadata:823`, `show_journal:904`, `run_picker:224` (shared with `cli/search.rs`), `print_plain:112`. About 100 lines are `fastf recent`/`fastf open`.
- `tui/menu.rs:6-8` imports seven `cli::*` modules; the TUI is a client of the CLI layer.
- Duplicates: template picker ×2 (`cli/new.rs:165-189` returns `Template`, labels `name — description`; `tui/menu.rs:896-912` returns `String`, labels `name (slug)`, different "no templates" errors); base picker ×3 (`cli/move_project.rs:86-95`, `tui/menu.rs:200-233`, `cli/recent.rs:594-609`; only one clamps labels, only one marks `(default)`); byte formatter ×2 (`recent.rs:483-503` KB..TB with "unavailable", `cli/template.rs:347-361` KB..GB); row-width computation ×3 (`recent.rs:112`, `:238-272`, `:429`).
- `src/core/vars.rs:2` imports `dialoguer`; `collect_vars:70` prompts and `eprintln!`s; its callers are all `cli/` (`new.rs:53`, `apply.rs:35`, `register.rs:177`). `validated_raw_values:12` is the shared, noninteractive boundary and stays in core.

**Design decisions (made).**
- New modules: `src/tui/browser.rs` (paged browser, row theme, size cell, frame tick), `src/tui/actions.rs` (project action menu, metadata and journal display), `src/tui/rows.rs` (one row-width/label builder used by plain output, picker, and browser; `clamp_label` and its unit tests), `src/tui/pickers.rs` (one `pick_template(prompt) -> Result<Option<Template>>`, one `pick_base(bases, prompt) -> Result<Option<PathBuf>>` with clamped labels and a `(default)` marker), `src/tui/vars.rs` (`collect_vars`, moved from core; core keeps the types and `validated_raw_values`), `src/util/human_bytes.rs` (one formatter; KB..TB, "unavailable" handled by the caller).
- `src/cli/recent.rs` keeps `RecentArgs`, `run`, `filter_projects`, `print_plain`, `run_picker` (Phase 10 retires `run_picker`). `cli/search.rs` imports from the new modules.
- Layering rule from here on: `core` and `util` import nothing from `cli`, `tui`, `ui`. New shared interactive code goes in `src/tui/`. `cli` may call `tui` helpers; `tui/menu.rs` may still call `cli::*::run` (that re-layering is not in scope).
- Pure move: behaviour, strings, and test expectations stay identical, except the byte formatter unification may change template-size rounding in `template show` (acceptable; update its test if one exists).

**Steps.**
- [ ] Move the code; update imports; keep unit tests next to the code they test.
- [ ] Replace the two template pickers, three base pickers, and two byte formatters with the single versions. `fastf move`'s picker and the create-time base picker must both clamp and both mark the default.
- [ ] Add a unit test that `core/` has no `dialoguer` import (a source scan over `src/core/*.rs`; Phase 12 extends it to `colored` and `println!`).
- [ ] `CLAUDE.md`: update "Project layout" and the gotchas that name `cli::recent::run_picker`, `recent.rs` as the place for picker actions, and `collect_vars`' location. `tests/CLAUDE.md` unchanged unless a test moved.

**Acceptance.** All gates green with the same test outcomes. `grep -rn "use dialoguer" src/core` is empty. `wc -l src/cli/recent.rs` is under 250.

## Phase 7: One way out: a single cancel contract for every prompt

**Goal.** Esc (and `q` where dialoguer maps it) backs out of every menu, list, and confirmation; text prompts cancel the same way; the behaviour is identical at every level because every prompt goes through one module, and a test fails the build if any prompt bypasses it.

**Evidence.**
- Every picker uses `dialoguer::Select::interact()`, which ignores Esc; `util/live_select.rs:106-109` deliberately matches it. Dead ends: `pick_template` (`new.rs:182`), `pick_base` (`menu.rs:223`), `prompt_template_slug` (`menu.rs:906`, used by Edit/Show/Delete/Apply), the required-variable loop in `collect_vars` ("is required — please enter a value", forever). The only exit is Ctrl-C, which ends the session at exit 130.
- Empty-answer-cancels already exists for two text prompts (`menu_search` `menu.rs:257`, `menu_register` `:323`) and nowhere else.
- PR #3's attempt moved 29 prompts to `interact_opt` by hand and missed some; text prompts could not cancel at all because `dialoguer::Input` swallows Esc.
- Vocabulary today: Back, Quit, Back to list, `[Quit]`, `[Cancel]`, Done, Discard changes; the action menu's "Quit" returns to the main menu (`recent.rs:388`); "Projects" names both the browser and the page-size setting (`menu.rs:95,495,709`); every submenu matches on raw indices with `_ => unreachable!()` (`menu.rs:108-120, 409-419, 460-473, 503-512, 533-570, 606-628, 713-723, 751-759`); `move_idx` is hard-coded to 6 (`recent.rs:576-585`).

**Design decisions (made).**
- One module, `src/tui/prompt.rs`, is the only place that may name `dialoguer::Select`, `MultiSelect`, `Confirm`, `Input`, or `FuzzySelect` (the source-scan test enforces it over `src/tui` and `src/cli`; `util/live_select.rs` may keep `dialoguer::console`). API shape: `select(prompt, items, default) -> Result<Option<usize>>`, `multi_select(...) -> Result<Option<Vec<usize>>>`, `confirm(prompt, default) -> Result<Option<bool>>`, `text(prompt, TextOpts { initial, allow_empty, validator }) -> Result<Option<String>>`. `None` means cancelled and is never an error, so `contain`/`is_fatal` stay untouched.
- Selects and confirms use dialoguer's `interact_opt`, which in the vendored 0.11.0 cancels on `Key::Escape | Key::Char('q')` (`prompts/select.rs:244`, `prompts/confirm.rs:199`); `live_select` mirrors exactly those two keys. `text` cannot use `dialoguer::Input` (`prompts/input.rs` has no `Key::Escape` arm at all): implement a minimal single-line editor on `dialoguer::console::Term::stderr()` the way `live_select` replaced `Select`: printable insert at cursor, Backspace, Delete, Left/Right, Home/End, Enter submits (runs the validator; an error is shown on the line below and the text is kept), Esc cancels. Rendering is one line; when `prompt + text` exceeds the terminal width, show a window around the cursor rather than wrapping (conhost ghosting is caused by wrapped lines; see the v1.0.2 gotcha). `with_initial_text` semantics: the initial text is editable, not a hint. Non-TTY: every function bails through Phase 2's `require_tty`.
- Semantics table (document it in `docs/cli.md` under the interactive menu): Esc in a submenu → its parent; Esc in the main menu → quit (same as the Quit item); Esc in a confirmation → the action is not taken, back to the menu that offered it; Esc anywhere in the create wizard (template, base, variables, final confirm) → "Cancelled, nothing was created", main menu, counter untouched; Esc in the project list → main menu; Esc in the action menu → the list, same row selected; Esc in a settings field → the settings submenu, value unchanged; Esc in the template builder in edit mode → its section menu; in new mode → "Discard this template?" confirm (Phase 11 gives new mode the section menu; until then this is the rule). The visible Back/Cancel items stay: Esc is the shortcut, the item is the discoverable path.
- While migrating, apply one vocabulary: `Back` (to the parent menu), `Cancel` (abandon the current action), `Quit` (main menu only). The action menu's last items become `Back to list` and `Back to main menu`. Rename the settings entry "Projects" to "Project list (page size)". Replace index `match` arms with a `match items[choice]` over `&str` constants or a small enum per menu; `move_idx` is found by label. Every menu keeps `.default(0)` except where a destructive default would be wrong (keep `menu_id`'s Back default).
- Required variables: Esc cancels the whole create (the table above). An empty answer on a required variable re-prompts with the same text kept, as today, but the hint says "(Esc to cancel)".

**Steps.**
- [ ] Write `tui/prompt.rs` with unit tests for the line editor's pure parts (cursor moves, windowing math, validator retention) and the source-scan test (initially failing, listing every offending site).
- [ ] `tests/tui_pty.rs` first, one case per row of the semantics table: drive into the level, press Esc, assert the parent prompt string reappears, and for the create wizard assert no folder was created and the counter file is unchanged. Anchor assertions on prompt strings, not on timing (`tests/CLAUDE.md`).
- [ ] Migrate every prompt in `src/tui/` and `src/cli/` to `tui::prompt`; apply the vocabulary and label-based matching as you touch each menu; delete the per-site `is_terminal` checks that `prompt` now owns.
- [ ] `live_select`: Esc returns `None`; `q` matches whatever dialoguer does for `interact_opt` in the vendored version (check `~/.cargo/registry`); keep the existing key set otherwise (Phase 10 adds more).
- [ ] Docs: `docs/cli.md` interactive section gets the semantics table in prose. `CLAUDE.md` gets a gotcha: "every prompt goes through `tui::prompt`; the source-scan test is the enforcement; `None` is cancel, never an error".

**Acceptance.** The source-scan test passes with `tui/prompt.rs` as the only match. Every pty case in the table passes. Ctrl-C is no longer the only way out of any flow. `cargo test --release` passes on Windows CI (the line editor is exercised by unit tests there even though pty tests are Unix-only).

## Phase 8: Keep what you typed

**Goal.** No answer is lost because a later answer was wrong. Validation happens at the prompt that collected the value, and a rejected value comes back editable.

**Evidence.**
- `menu_register` (`menu.rs:319-384`) asks path, template, rename, apply, then `register::run` rejects the path (`register.rs:153-158`); all four answers are gone (`tests/tui_pty.rs:63` asserts containment, not retention).
- `template_from_folder_flow` (`menu.rs:426-437`): path, slug, force, then `validate_source`/`validate_slug` (`template.rs:281-299`) reject.
- `menu_apply` (`menu.rs:275-295`): template, target, dry-run, every variable, then `apply.rs:70-75` bails on the target.
- `menu_settings_basics` (`menu.rs:539-545`) re-prompts from empty; `menu_id` (`menu.rs:462-468`) "expected a number" → retype; `template_builder::edit_metadata` (`template_builder.rs:217-219`) `bail!("slug cannot be empty")` discards a whole in-memory template; `edit_id` (`:255`) swallows a bad digit count with `unwrap_or`.
- `menu_search` (`menu.rs:267-270`): no matches → main menu; the query must be retyped.

**Design decisions (made).**
- Use `tui::prompt::text`'s validator for every value that has a local validity rule: path exists and is a directory (use `config::resolve_base_dir_input`/`expand_base_path` where the value is a base, plain canonicalize where it is a target); slug via `validate_slug` plus "exists unless force"; numbers via parse with a range. The validator error is shown inline and the text stays in the field.
- Ask in dependency order: the path before anything that depends on it (register, from-folder, apply). After the local checks pass, the core operation can still fail (race, permissions); then `contain` reports it and the flow returns to the submenu, which is acceptable because the local-validity class is what lost answers in practice.
- Search: loop; no matches → "No projects match that query" and the same prompt with the previous query prefilled; Esc leaves.
- Template builder: slug and name validated at the prompt; `edit_id` reports "expected a number between 1 and 9" and keeps the old value only when the user cancels.

**Steps.**
- [ ] `tests/tui_pty.rs` first: register with a bad path, see the inline error and the bad text still in the field, fix it, complete; the earlier answers are used (assert on the written `PROJECT_INFO.md`). Same shape for from-folder (bad slug), apply (bad target after variables), settings base dir, ID counter, and search (retry with the query prefilled).
- [ ] Implement per flow; keep the prompts' wording stable where tests anchor on it.
- [ ] Docs: `docs/cli.md` interactive section, one paragraph.

**Acceptance.** All new pty cases pass; no existing case regresses; no flow in `src/tui/` asks a dependent question before validating what it depends on.

## Phase 9: The browser stops rescanning

**Goal.** Tagging or annotating a project in the guided browser updates that row and nothing else. Unmounted or unresponsive bases never freeze the screen.

**Evidence.**
- `run_paged_browser` (`tui/browser.rs`, formerly `recent.rs:314-405`) calls `load()` on entry and after every `ActionLoop::Changed`; the guided browser sets `reload_after_change = true` (`:378`), so add tag, remove tag, and add note (`:765-767, 787-789, 806-808`) each run a full `library::discover` across all bases. `menu_search`'s loader (`menu.rs:267-270` → `search.rs:96-106`) also re-reads every `PROJECT_INFO.md` in the library.
- `project_action_menu` calls `Config::load()` and `is_dir()` per base on entry (`:546-556`); `pick_base`, `id::mounted_bases`, and `move_project::run:41-48` do the same. `is_dir()` on a dead SMB mount blocks for the OS timeout with no message.

**Design decisions (made).**
- Replace the boolean with an outcome enum returned by the action menu: `Unchanged`, `Patched(Project)` (tag add/remove/reauto, note, rename, move: re-read that one project's metadata or use the `Project` the operation returns), `Removed` (delete, unregister), `Reload` (fallback on error). The browser patches or drops the row in memory and calls `size_scan.forget` only for the patched project (already the case). In search results, re-evaluate the query against the patched metadata (`core::query::evaluate`) and drop the row if it no longer matches.
- Mounted bases are probed once when the browser (or a picker) opens, via a new `util::paths::probe_dirs(&[PathBuf], Duration) -> Vec<(PathBuf, Probe)>` with `Probe::{Mounted, Absent, Unresponsive}` that runs `metadata()` on a helper thread and `recv_timeout`s (the `live_select` pattern). An `Unresponsive` base is shown as such wherever bases are listed and excluded from move targets for that session. Re-probe only on `Reload`.
- Add `util::trace` (debug builds only, env-gated by `FASTF_TRACE_FILE`): `trace::hit("discover")`, `trace::hit("template_load")`, `trace::hit("read_metadata")` append one line each. Tests count lines. Release builds compile it to nothing, like `util::faults`.

**Steps.**
- [ ] `util::trace` with a unit test; instrument `library::discover`, `library::scan_base`, `Template::load_from_file`, `project_info::read_metadata`.
- [ ] `tests/tui_pty.rs` first: open the browser with `FASTF_TRACE_FILE` set, tag a project, assert the row shows the tag and the `discover` count did not increase; delete a project, assert the row is gone and the list did not scan more than once.
- [ ] Outcome enum, in-memory patching, search re-evaluation.
- [ ] `probe_dirs`; use it in the browser, `pick_base`, `fastf move`, `id show`. Unit test with a directory that a helper thread holds open is not possible cross-platform; test the timeout path with a closure-injected prober.
- [ ] Docs: `docs/projects.md` gets a sentence on unresponsive bases. `CLAUDE.md`: the outcome enum rule ("a content mutation patches the row; only structural changes reload") and `util::trace`.

**Acceptance.** The trace-based pty tests pass. A deliberately unreachable base path (for example a non-existent host under `/run/user/.../gvfs` or a `probe_dirs` injected timeout) shows "unresponsive" within the timeout instead of hanging the menu.

## Phase 10: The main-menu frame, a better action menu, more keys, one browser

**Goal.** Give the main menu the context it lacks, make the most-used screen faster to operate, and make `fastf recent`/`fastf search` use the same browser the menu uses.

**Evidence and ideas (Cristo asked for mine; he will check them).**
- The main menu prints only `project base → …` (`menu.rs:84-90`). PR #3's frame was wanted.
- Action menu today: Open folder, Show metadata, Add tag (typed), Remove tag (typed), Add note, Show journal, Move (conditional), Rename, Unregister, Delete, Back, Quit. Missing: a way to get the path out (copy to clipboard, or print it), tag selection from known tags instead of retyping, `tag reauto`, and sizes in the `fastf recent`/`fastf search` pickers (`project_action_menu(p, None, false)` at `recent.rs:284`).
- `live_select` handles ↑/↓/j/k/Tab/Enter/Space (`live_select.rs:103-110`). No PageUp/PageDown, Home/End, or filter; with `recent-default-limit` 50 on a 30-row terminal you scroll one row at a time.
- `run_picker` (`recent.rs:224`) is a second, size-less browser used by `fastf recent` and `fastf search` on a TTY.

**Design decisions (made).**
- Frame: under the banner, a compact block that costs no scan. Lines: version; default base (display path) and each extra base with its `Probe` state from Phase 9; library counts from the cache files only (`library::load_cache` per mounted base, no staleness check, labelled "from index"); highest ID and newest project from the same caches; the last three actions of this session (an in-memory ring: "created ID0248", "tagged ID0100 urgent", "moved ID0017 → archive"). Plain printed lines, ASCII or single-width box characters, no ANSI inside `Select` items. Hidden when `show_banner` is off? No: the frame is information, the banner is decoration; add a separate `show_frame` config key defaulting to on, exposed in Settings → Workflow prompts.
- Action menu order (most used first): Open folder, Copy path, Show metadata, Tags (Add from known tags or new, Remove via multi-select of the project's tags, Re-derive), Journal (Add note, Show), Move, Rename, Unregister, Delete, Back to list, Back to main menu. "Copy path" tries, in order, `wl-copy`, `xclip -selection clipboard`, `xsel --clipboard`, `clip.exe`/`clip`, `pbcopy`, each only if found on PATH, and otherwise prints the path on its own line; always prints what it did. Known tags come from the discovered projects already in memory (no rescan). No open-in-editor.
- `live_select` keys: PageUp/PageDown (one viewport), Home/End, `/` enters filter mode (a one-line `filter: …` under the list, substring over the visible label: id, name, tags, base; while the filter has focus every printable key including `q`, `j`, `k` is a literal; Esc clears the filter first, then cancels; Enter selects). Rows stay single-line and clamped; the filter line counts in the block height (`clear_last_lines` takes the block back by line count).
- One browser: `fastf recent` and `fastf search` on a TTY call `tui::browser::run_paged_browser` with the CLI's filters applied to the loader; `run_picker` is deleted. "Back to main menu" reads "Quit" when the browser was started from the CLI (the caller passes the label). `--plain` output is unchanged.
- Also: the Size cell appears in all three entry points now; `SIZE_CELL` stays fixed width.

**Steps.**
- [ ] `tests/tui_pty.rs` first: frame shows "from index" counts that match a planted cache and a base marked unmounted; `/` filter narrows to one row and Enter opens its action menu; PageDown moves the highlight by the viewport; `fastf recent` on a pty shows the Size column and the action menu; Copy path with no clipboard tool on PATH prints the path.
- [ ] Frame, `show_frame` key (`config::set` accepts it, Settings shows it, `config show` lists it), session ring.
- [ ] Action menu restructure using `tui::prompt` (`multi_select` for tags), Copy path, Re-derive.
- [ ] `live_select` keys and filter; unit tests for viewport and filter math.
- [ ] Retire `run_picker`; route `recent`/`search` through the browser.
- [ ] Docs: `docs/cli.md` (keys table, Copy path, `show_frame`), `CLAUDE.md` (frame reads caches only; filter line is part of the block height).

**Acceptance.** All pty cases pass. Opening the main menu performs zero directory scans (trace count for `scan_base` is 0 on a warm cache). `grep -rn run_picker src` is empty.

## Phase 11: Parity with the CLI, and a template builder that lets you change your mind

**Goal.** Everything the CLI can do that a TUI user plausibly needs is reachable from the menu, and the template builder never throws work away.

**Evidence.**
- CLI-only today: `template from-folder --bundle-assets` (`menu.rs:437` hard-codes `false`), `register --recursive` (and `--use-today`/`--created`; `menu.rs:381-382` hard-code them), `new --dry-run` (`menu.rs:186`; with `confirm-create` off the TUI creates with no preview at all), `reindex`, `reconcile`, `paths`, `tag reauto` (Phase 10 adds it to the action menu), `config set register-naming-pattern` (the only key without a TUI editor).
- `template_builder.rs:29-47`: new mode is linear (Step 1/6 → 6/6 → "Save template?" → No prints "Discarded."). Edit mode (`:52-148`) has the section-review menu new mode needs. `edit_structure` (`:305-318`) and `edit_files` (`:329-335`) are all-or-nothing ("Replace folder structure?"); variables got Add/Edit/Remove/Reorder (`:356-438`). `collect_file` (`:596-605`) cannot create an empty file and has no abort. `template_builder.rs` has zero test coverage of any kind.

**Design decisions (made).**
- From-folder flow: after the pre-scan, if binary or large files exist, ask "Bundle N assets (X MB)?" with the default No; pass the answer through. Register flow: a first question "One folder, or every folder in a base?"; the recursive path shows the dry-run preview (the same renderer as `--recursive --dry-run`) and asks to proceed; a "Created date" question with Folder's date (default) / Today / Specific date. Create flow: always print the preview in the TUI; when `confirm_create` is on ask, when off proceed (never skip the preview in the TUI; the config key governs the question, not the information). Settings → new "Maintenance" submenu: Reindex (runs `reindex`, prints per-base counts), Check and recover (runs `reconcile_locked`, renders the report with the CLI's renderer), Show data locations (the `paths` output). Settings → Project basics gains "Register naming pattern" (validated: must contain `{id}`, as `config::set` enforces).
- Builder: new mode runs the six steps, then lands in the same section menu edit mode uses (Name and description, Variables, Folder structure, Files, ID format, Save, Discard). Structure and files get per-item Add/Edit/Remove like variables. `collect_file` allows an empty file (an explicit "Empty file" choice) and Esc aborts the current file without losing the others. Keep the reserved-name rejection and the `NOTES.md` example.
- Tests: `tui_pty.rs` gains builder coverage (create a template through the menu, change a folder after the review, save, load it with `Template::load_from_file`, assert structure and files). This is the first coverage of `template_builder.rs`.

**Steps.**
- [ ] pty tests first for: bundle prompt; recursive register preview then commit; created-date choice; create preview shown with `confirm_create=false`; Maintenance → Reindex and Check and recover outputs; register naming pattern rejected without `{id}`; builder new-mode review loop; per-item structure edit; empty file.
- [ ] Implement flows; reuse CLI renderers (`print_apply_plan`, the reconcile report renderer, `paths_cmd`'s printer) rather than duplicating text.
- [ ] Docs: `docs/cli.md` interactive section (Maintenance submenu, register choices), `docs/templates.md` (builder review menu, per-item editing).

**Acceptance.** Every CLI capability listed above has a pty test through the menu. `template_builder.rs` is covered by at least three pty cases. No TUI create ever commits without printing the preview.

## Release checkpoint: v1.7.0

Separate session with the `release` skill. `ROADMAP.md` gets a `v1.7.0` row ("the guided menu: one way out, nothing lost, nothing rescanned") with Phases 6-11. `docs/cli.md`'s interactive section is reviewed end to end against the real menu before tagging (drive it once by hand on Linux; note in ROADMAP that the Windows console pass is the post-release manual item, as before).

---

# Track C: core structure (release v1.7.1, no behaviour change)

Each phase here is behaviour-neutral. The proof is the unchanged test suite plus the specific guard each phase adds.

## Phase 12: Rendering out of core, and the module cycles broken

**Goal.** `core/` neither prints nor prompts; the five module cycles are gone; a test keeps it that way.

**Evidence.**
- `src/core/project.rs:124-332` and `:775-820`: `print_dry_run`, `print_resolved_values`, `print_file_previews`, `print_success`, `print_project_path`, `print_tree`, `print_apply_plan`: 255 lines of `colored` output with callers only in `cli/` and `tui/` (`cli/new.rs:62,67,102`, `cli/apply.rs:80,84`, `cli/template.rs:78`, `tui/template_builder.rs:764`). `print_dry_run` is not pure (`:142` walks `files/`).
- `src/core/post_create.rs` (224 lines): a serde struct (`:16-54`) plus process spawning, `colored` warnings, and a `dialoguer::Confirm` at `:153`. `project::run_post_create` calls it from core.
- Remaining `println!`/`eprintln!` in core after Phase 6: `library.rs` (12, best-effort warnings), `counter.rs:95,120`, `template.rs:342`, `project.rs:464,927`, `assets.rs:798,847`, `util/paths.rs:60`, `util/faults.rs:72`.
- Cycles: `library`↔`project_info` and `library`↔`provisioning` via `library::now_iso8601` (`library.rs:1266`); `project`↔`project_info` via `ProjectPlan` (`project.rs:34`) used by `project_info::from_plan`; `naming`↔`template` via `apply_transform` (`naming.rs:6`) and the Phase-5-deleted `ensure_relative_safe_path`; `config`↔`post_create` via the struct.
- `Metadata::from_plan` (`project_info.rs:104`) reads the clock at `:119`, which is why register does a two-step write (`operations.rs:405`).

**Design decisions (made).**
- Core produces data; `cli` renders. `project::plan_report(plan, template, config) -> DryRunReport` (folder name, tree, interpolated file list, resolved values with transforms, id and counter delta, date tokens, the first `preview_lines` of each templated file) and `project::apply_report(actions) -> ApplyReport`. New `src/cli/render.rs` owns `print_dry_run(&DryRunReport, PreviewKind)`, `print_apply_plan`, `print_success`, `print_project_path`, `print_tree` (one copy; `template show` and the builder import it from there, per the existing gotcha).
- `post_create`: the serde struct stays in core (`core/post_create.rs` shrinks to the struct and `resolve_post_create`); the runner moves to `core`-free `src/cli/post_create.rs`? No: the runner is noninteractive except `prompt_and_reveal`, and `ui/` must not depend on `cli/`. So: the runner stays in core but returns `Vec<PostCreateWarning>` instead of printing, and `prompt_and_reveal` (the only prompt) moves to `cli/`. `ui/` already calls the runner; it renders warnings its own way.
- Best-effort warnings deep in core (the `eprintln!` list) route through one `util::diag::warn(msg)` so there is exactly one allowed sink; the guard test allows `util::diag` and nothing else.
- Cycles: `now_iso8601` → `util::time`; `ProjectPlan` → `core/plan.rs`; `apply_transform` → `template.rs` (or `core/transform.rs`); `PostCreate` struct already separated above. Add `Metadata::from_plan_at(plan, tmpl, tags, created)` with `from_plan` delegating to `now`; register uses `from_plan_at` and drops the second write (keep the byte-identity test green).
- Guard: a unit test that scans `src/core/**` and `src/util/**` for `use colored`, `use dialoguer`, `println!`, `eprintln!`, `print!` and allows only `util/diag.rs` and `util/live_select.rs` (which renders by design). A second scan asserts no `crate::cli`, `crate::tui`, `crate::ui` under `core`/`util`.

**Steps.**
- [ ] Guard tests first (failing), then the moves.
- [ ] `DryRunReport`/`ApplyReport` with unit tests for the report content (this is the first direct test of the dry-run data).
- [ ] `from_plan_at`; remove register's second write; round-trip tests unchanged.
- [ ] `CLAUDE.md`: "Output display" section moves its function names to `cli/render.rs`; gotcha on `util::diag` as the one sink; cycles paragraph deleted if present.

**Acceptance.** Guard tests pass. `cargo tree`-independent check: `grep -rln "colored\|dialoguer" src/core src/util` lists only `live_select.rs`. All existing tests pass unchanged.

## Phase 13: Split `library.rs` (pure move)

**Goal.** `library.rs` (1268 production lines, nine responsibilities) becomes a directory of focused modules with the same public paths.

**Evidence (by line range).** Types `:43-135`; discovery `:137-312`; cache I/O `:314-406`; revalidation `:412-504`; move entry points `:506-651`; move engine `:652-975` (depends on `transactions`, `assets`, `fs_retry`, `faults`, `Progress`: a different dependency set from everything else); unregister/delete/rename `:977-1158`; resolve and counter floor `:1160-1246`; `reindex` `:1248`.

**Design decisions (made).**
- `src/core/library/mod.rs` re-exports everything under the current `library::` paths so the ~30 call sites and all tests compile unchanged. Submodules: `model.rs`, `discovery.rs`, `cache.rs`, `guard.rs` (revalidation), `lifecycle.rs` (unregister/delete/rename), `resolve.rs`. The move engine leaves the library entirely: `src/core/move_engine.rs` next to `transactions.rs`, with `library::move_project*` delegating.
- Unit tests move with their functions. No logic edits; if you find one needed, park it.
- Narrow visibility inside the new modules to `pub(super)` where only `library` siblings call a function.

**Steps.**
- [ ] Move; `pub use`; `cargo doc` clean (a `pub` item's docs may not link to a `pub(crate)` one).
- [ ] `CLAUDE.md` "Project layout": list the submodules in one line each; update every `library.rs:NNN`-style mention to the new file.

**Acceptance.** Identical test results; `wc -l` of no file under `src/core/library/` exceeds 500 production lines; `move_engine.rs` has no dependency on `discovery`/`cache` beyond `touch_cache`/`refresh_cache` calls through the public API.

## Phase 14: One clock, one load, lazy templates

**Goal.** A create samples the clock once, loads config/counters/template once, and listing templates does not read their file contents.

**Evidence.**
- `src/core/naming.rs:40-57`: `interpolate` calls `Local::now()` and formats four strings on every call; `interp_rel` (`assets.rs:486`) calls it per path segment; `copy_file` (`:573`) per file. Variables substitute in `HashMap` order, so a value containing another token expands nondeterministically; a create spanning midnight can name the folder with one date and the files with another; the previewed plan and the committed plan can disagree.
- Per `fastf new` (traced through `cli/new.rs:24` → `operations::create:56` → `project::plan:49` → `Counters::next_value:156` → `floor:217` → `library::max_id:1214`): `Config::load` ×2, `Counters::load` ≥4, full template parse plus a read of every text file under `files/` ×2, `library::max_id` ×2 (the double `plan()` inside and outside the lock is deliberate; keep it), `effective_bases()` with a `canonicalize()` per base ×6-8, `assets::walk` ×3. `fastf apply` walks the template four times (`operations::apply:111` → `apply_plan:663`, then `apply:739` → `apply_plan_resolved:674` again, then `copy_template_files:761`, plus `scan_files` on load) and computes `rendered_values` twice.
- `Template::load_from_file:172` → `scan_files:200` reads every UTF-8 file ≤ `TEXT_MAX_BYTES` into memory; `load_all:322` does it for every template; hit by `template list`, `id show` (`cli/id.rs:114`), the template picker, `new` without a slug, `/api/state` (`ui/mod.rs:638`).
- `cache_upsert:362` builds a throwaway `CacheEntry` per existing project inside `retain`.

**Design decisions (made).**
- `RenderContext { date, yyyy, mm, dd }` built once per operation (`plan` builds it and stores it in `ProjectPlan`; apply and register build one at entry) and passed to `interpolate`/`interpolate_name`/`interp_rel`/`copy_file`. Substitution becomes a single left-to-right scan: find `{token}`, look it up (built-ins, then variables), emit; unknown tokens pass through unchanged; a variable's value is never re-scanned. Property test: output is independent of `HashMap` order and of the number of variables; existing naming tests pin the behaviour for `__` collapse.
- `operations::create` takes the loaded `Config`, `Counters`, and `Template` from the caller for the pre-lock preview and reloads only what the lock requires (config and counters; the template is re-read under the lock only if its manifest mtime changed). `effective_bases()` memoizes the canonicalized list on the `Config` instance (a `OnceCell` field with `#[serde(skip)]`).
- `apply` computes one `ApplyPlan` under the lock and executes it; the dry-run path renders the same plan.
- `Template::load_from_file(path, FileBuffer::Skip | Load)`; `load_all` uses `Skip`; the editors, previews, and `apply`'s variable detection use `Load`. `files_dir()` walking for create is unchanged (it never used the buffer).
- `cache_upsert`: compute the new entry's `dir` once and compare strings.
- Proof: Phase 9's `util::trace` counters. Add a `tests/cli_surface.rs` case that runs `new` with `FASTF_TRACE_FILE` and asserts `template_load` ≤ 2, `discover`/`scan_base` ≤ 2, `read_metadata` bounded by project count, and a `template list` case asserting zero file-content reads (add a `template_file_read` trace point in `scan_files`).

**Steps.**
- [ ] Trace-count tests first (they fail with today's counts; record today's numbers in the PR).
- [ ] `RenderContext` and single-pass interpolation; property tests in `tests/properties.rs`.
- [ ] Load-once threading; memoized bases; single apply plan; lazy file buffer; `cache_upsert`.
- [ ] `CLAUDE.md`: rewrite the "Interpolation" section for the context; add "one load per operation" to the `DataLock` gotcha; the lazy-buffer rule next to the v0.8 `files/` gotcha. `ROADMAP.md`: strike "Lazy template loading" and "Deterministic one-pass interpolation" from the backlog.

**Acceptance.** Trace-count tests pass. Every existing naming, interpolation, and preview test passes unchanged. `fastf template list` reads no template file contents.

## Phase 15: Types over strings, path fidelity, bounded recursion

**Goal.** Closed sets are enums, paths are paths, and a pathological tree degrades instead of aborting.

**Evidence.**
- `Progress.status`/`phase` (`assets.rs:63,66`) are `String` over `running|done|failed|cancelled` and `copying|verifying|finalizing|done`, set by literal at ~15 sites (`library.rs:789,807,816,911,967`, `transactions.rs`, `ui/mod.rs`). `Incomplete.kind` (`provisioning.rs:228`) has six magic strings. `Config.on_name_collision` (`config.rs:148`) compares `"error"` case-insensitively.
- `assets::walk_inner:284` builds `AssetEntry.rel` with `to_string_lossy()` and `project.rs:963` joins it back to open the source; `template_import::classify_file:124` has the same shape. `transactions.rs:91-92` states the correct invariant and implements it for moves.
- `Config.base_dir: String`, `bases: Vec<String>`; writes go through `display().to_string()` (`config.rs:278`, `operations.rs:66`, `cli/new.rs:28`), lossy for non-UTF-8 paths.
- Unbounded recursion: `assets::walk_inner`, `transactions::scan_inner`, `template_import::scan_dir`, `tree_size::directory_size_inner`, `fs_retry::clear_readonly_tree`, `project::walk_structure`. `tree_size` walks user-chosen trees from the browser.

**Design decisions (made).**
- `#[serde(rename_all = "lowercase")]` enums for `Progress::Status`, `Progress::Phase`, `IncompleteKind`, `NameCollision`; JSON and TOML bytes are unchanged (assert with the existing UI tests and a new serialization test per enum). Unknown `on_name_collision` values keep today's "anything but error means suffix" behaviour via a custom `Deserialize` or `#[serde(other)]`.
- `AssetEntry.rel: PathBuf`; interpolation of names operates per `OsStr` component and only converts a component to `str` when it contains a `{`; a non-UTF-8 component that contains no token is copied verbatim. Add a Unix-only test with a non-UTF-8 template filename (`OsStr::from_bytes`).
- `Config` paths: TOML cannot hold non-UTF-8, so keep `String` in the struct but replace every `display().to_string()` on a path that will be saved with `to_str().context("path is not valid UTF-8 and cannot be stored in config")?`. Same at `operations.rs:66` and `cli/new.rs:28`.
- Recursion: a `MAX_DEPTH` (for example 256) in each walker returning an error naming the path at the limit; `tree_size` returns `None` at the limit (consistent with "any read failure is `None`"). `hostile_fs.rs` gets a deep-tree case (build 300 nested dirs) for `tree_size` and `assets::walk`.

**Steps.**
- [ ] Serialization tests first; enums; call sites.
- [ ] `PathBuf` entries and the non-UTF-8 test; config path conversions.
- [ ] Depth limits and the `hostile_fs.rs` cases.
- [ ] `CLAUDE.md`: the `Progress` gotcha (v0.11) updated to the enum; a line in "Path-escape safety" on native-path fidelity for the create engine.

**Acceptance.** No `String` field in `src/` models a closed set that the code matches on by literal. The non-UTF-8 template file test passes on Linux. A 300-deep tree yields an error or `None`, never an abort.

## Phase 16: One test harness, parallel suites, the coverage gaps

**Goal.** The mandatory harness rules live in one place, the two large suites stop serializing 109 tests behind two mutexes, and the modules with no coverage get some.

**Evidence.**
- `with_fresh_install` is duplicated in `tests/integration.rs:23-50` and `tests/ui_server.rs:19-44` (comments differ only); near-copies in `tests/crash_recovery.rs:32-78` and `tests/hostile_fs.rs:21-48`; `write_template` exists five times (`integration.rs:58`, `ui_server.rs:46`, `crash_recovery.rs:78`, `hostile_fs.rs:48`, `common/mod.rs:62`). `tests/CLAUDE.md` says every harness must redirect `HOME`; it is re-typed four times.
- `integration.rs` (2560 lines, 67 tests) and `ui_server.rs` (42) each hold one `SERIAL` mutex for every test body. `integration.rs` already has banner comments at `:118` (create), `:980` (metadata), `:1306` (search), `:1569` (template engine), `:1791` (register), `:2379` (move), `:2441` (paths/bootstrap/mangen).
- No tests: `config::expand_base_path`/`resolve_base_dir_input` ("the only way in" for base paths), `Config::suffix_on_name_collision`, direct `Template::validate`, `template_import`, `cli/paths_cmd.rs`, `cli/reindex.rs`, `cli/reconcile.rs` wrappers, `tag add/remove/list` as processes, `template list/show/delete` as processes. `gallery_templates_parse_and_plan` (`integration.rs:1564`) asserts `seen >= 5` against 8 templates and never plans.
- `photography` and `video-production` gallery templates have no `files/` dir (legal, undocumented).

**Design decisions (made).**
- `tests/common/` gains `env.rs` with `with_fresh_install(serial: &Mutex<()>, f)` (each binary keeps its own `static SERIAL`, as `tests/CLAUDE.md` requires) and `fixtures.rs` with one `write_template` that accepts the inline-`files:` form. The four in-process suites use them; the process-driving `Sandbox` stays.
- Split `integration.rs` into `create.rs`, `metadata.rs`, `search.rs`, `template_engine.rs`, `register.rs`, `move.rs`, `data_dir.rs` along the banner lines; split `ui_server.rs` into `ui_projects.rs`, `ui_templates.rs`, `ui_jobs.rs`, `ui_security.rs`. Tests move unchanged.
- New tests: unit tests for `expand_base_path` (relative rejected, `~` expanded, absolute kept, no `create_dir_all`), `resolve_base_dir_input` (creates and canonicalizes), `suffix_on_name_collision`; `template_import` unit tests for text/binary/large classification and the reserved root file; `cli_surface.rs` cases for `paths`, `reindex`, `reconcile` (clean and with a planted v2 transaction), `tag add/remove/list`, `template list/show/delete --yes`; the gallery test asserts all 8 by name and runs `project::plan` on each with sample variables; document in `docs/templates.md` that a gallery template without `files/` is structure-only, or add a minimal `files/` to the two (decide by what each template is for; prefer documenting).

**Steps.**
- [ ] Harness unification; suites split; `tests/CLAUDE.md` updated (it becomes the place that says "use `common::env`"), README suite table updated.
- [ ] New tests as listed.
- [ ] `cargo test` wall time recorded before and after in the PR (expect the in-process suites to parallelize across binaries).

**Acceptance.** `grep -rn "fn with_fresh_install" tests` returns one definition. `tests/` has no file over 1000 lines. Every `src/cli/*.rs` module is driven by at least one process-level test. The gallery test names all 8 templates.

## Phase 17: CLAUDE.md as a working document, dependencies refreshed

**Goal.** `CLAUDE.md` is under ~400 lines of current instructions, and the dependency set has no unmaintained crate under the identity format.

**Evidence.**
- `CLAUDE.md` is 660 lines / 66 KB after Phase 0. Remaining history-as-instructions: the four version-tagged gotcha subsections (`### v1.1 hardening gotchas` onward, ~230 lines), dead-history bullets ("`Template` needs `#[derive(Default)]`", "`console` crate was removed in v0.2", "`IdConfig` no longer has `auto_increment`", "`save_to_file()` no longer has `#[allow(dead_code)]`", "`Template::validate()` is `pub` (was private before v0.2)"), the `## Browser UI` paragraph duplicating `src/ui/CLAUDE.md`, version annotations in design-section headers, and a `x86_64-pc-windows-gnu` cross-compile under "Build commands" that CI never builds. Items that earlier phases changed (`classify_extra`, `verify_tree`, `move_project_with`, `recent.rs` as the picker home, rendering in core) must already be corrected by those phases; this phase consolidates.
- `serde_yaml 0.9.34+deprecated` (archived upstream) sits under `PROJECT_INFO.md` and `template.yaml`; 7 call sites in `template.rs:182,238`, `project_info.rs:142,269,299,304`, `library.rs:277`. The body and frontmatter byte-identity tests constrain the emitter.
- `chrono` default features pull `oldtime`/`wasmbind`/`unstable-locales` that nothing uses; `colored` 2.x is one major behind.

**Design decisions (made).**
- Restructure `CLAUDE.md` by topic, not by release: each design section keeps its content and loses its version header; each still-true gotcha moves under the design section it belongs to; every gotcha that a type, a test, or a guard now enforces is deleted with a pointer to the guard in the commit message (not in the file). Keep: build commands that are real, layout, design decisions, the tooling traps (PowerShell encoding, backticks in `app.js`), the harness pointer to `tests/CLAUDE.md`, the UI pointer to `src/ui/CLAUDE.md`. Target 350-420 lines.
- `serde_yaml` → `serde_yaml_ng` (drop-in fork) behind a small `core::yaml` shim (`to_string`, `from_str`, `Value`) so the crate is named in one file. Acceptance is the unchanged byte output of every existing round-trip and gallery test plus a new test that serializes a `Metadata` with every field populated (multi-line values, colons, quotes, unicode) and compares against a checked-in expected string produced by the old crate before the switch.
- `chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }`; `colored = "3"` after confirming `NO_COLOR`/non-TTY behaviour is unchanged (the `cat -v` piped-output checks from Phase 1 are the test). Record binary size before and after in the PR; revert any change that grows it.

**Steps.**
- [ ] `CLAUDE.md` restructure; read every section against the code once (the agent is expected to open each referenced file).
- [ ] `core::yaml` shim and the expected-output test with the old crate; switch crates; tests unchanged.
- [ ] `chrono` features, `colored` bump, `cargo audit` clean, `cargo tree -d` empty.
- [ ] `ROADMAP.md`: strike "Dependency and binary-size cleanup" from the backlog with the numbers.

**Acceptance.** `wc -l CLAUDE.md` ≤ 420. `cargo tree | grep -c deprecated` is 0. Byte-identity tests pass. Binary size is not larger than before the phase.

## Release checkpoint: v1.7.1

Separate session with the `release` skill. `ROADMAP.md` gets a `v1.7.1` row ("structure: one clock, one load, one sink") with Phases 12-17.

---

## Parking lot

Findings from the audit that are real but not scheduled. An agent that hits one of these adds nothing to its phase; it may append a line here with `file:line`.

Scriptability (Track D, not selected):
- No `--json`/`--format` anywhere; `recent --plain` is two lines per project with unicode markers; `search` lacks `--limit/--template/--since/--tag`; no `fastf path <query>`; `print_path` is a config toggle, not a `new` flag; no `--color=auto|always|never` (`colored` gates on stdout only, so stderr gets ANSI when redirected); exit codes 0/1/2/130 undocumented; `completions <shell>` is a `String` (use `clap_complete::Shell` as a `value_enum`), no `ValueHint::DirPath` on path args, no dynamic slug/ID completion; `search 'created>notadate'` matches nothing silently (a stderr hint would help); `fastf note` is a noun with a single verb.

Browser UI (Track E, not selected):
- XSS safety is 125 manual `esc()` calls with no lint; a tagged-template `html` helper and a source-scan test would make it mechanical. Job polling is triplicated (`app.js:2255`, `:2824`, `:2879`). Every mutation reloads `/api/state` (full discovery) and re-renders the page (43-key global `state`, 71 `innerHTML` writes). a11y: no focus trap in modals (`app.js:3405`), five `outline: none` with no `:focus-visible`, two `aria-label`s in the file, no `prefers-reduced-motion`. No frontend tests beyond `node --check`; `node --test` over the ~12 pure helpers would be cheap. Keep-alive and chunked bodies are unsupported (documented, fine for loopback). `open_path`'s `cmd /c start` quoting on Windows mirrors `reveal_folder`; both should pass the path through `explorer`-safe quoting if a case ever appears.

Other:
- `Counters::load().unwrap_or_default()` (`core/counter.rs:127`, inside
  `propagate`) swallows a parse or I/O error the same way `Config::load` did
  before Phase 1. Less damaging — `Counters::floor` also reads every base and
  `library::max_id`, so the number self-heals — but it silently drops the
  "unplugged base cannot restart numbering" protection, which is the one thing
  that file exists for.
- `query::resolve_field` clones per field access and `Predicate::Free` lowercases per comparison; fine at current scale.
- `size_scan::request` has an O(n²) `contains` over the queue; bounded by page size.
- `docs/` annotate features as "v1.5.0" but no such tag exists (tags go v1.4.0 → v1.5.1).
- The action menu offers "Move" only when another base is mounted; after Phase 9 an `Unresponsive` base could offer a "retry probe" item.
- `tests/tui_pty.rs:300` (`projects_browser_fills_in_sizes_without_any_input`) is
  timing-sensitive: it asserts a background size snapshot reaches the list within
  a repaint tick, and failed once under the CPU load of a full `cargo test
  --all-targets` while passing every run on its own (Phase 2, 2026-08-22). The
  guarantee is right; the deadline is what is thin. Give it a longer window, or
  anchor it on the scanner rather than the clock.
- Phase 12's guard ("only `live_select.rs` names `dialoguer` under `src/util`") must also allow `src/util/interrupt.rs:restore_terminal`, whose non-unix branch uses `dialoguer::console::Term` for the console cursor API (the unix branch is raw `isatty`/`write` because it runs in a signal handler). Converting the Windows branch to `GetConsoleCursorInfo`/`SetConsoleCursorInfo` FFI would remove the exception; it was not worth an untestable Windows change in Phase 1.

## Phase log

| Phase | Date | PR | Notes for later phases |
|---|---|---|---|
| 0 | 2026-08-21 | #6 (`7bccde5`) | CLAUDE.md trim and tests/CLAUDE.md landed; Phase 17 can assume both. |
| 1 | 2026-08-21 | [#7](https://github.com/cristocola/fast-folder/pull/7) | `Config::load()` propagates everywhere; `PreviewKind` on both printers; `cli::config::normalize_base_entry` is the shared base validator; `util::interrupt::restore_terminal` is the one cursor restore; `operations::reconcile` and `run_paged_browser`'s loader now return `Result`. |
| 3 | 2026-08-22 | PR pending | Branched on Phase 2 (#7 and #8 both still open) — retarget to `main` after they merge. `util::yaml::to_string_preserving_unknown` + `Metadata::OWNED_KEYS`/`Template::OWNED_KEYS` is how any file fastf does not fully own gets rewritten; `project_info::render` returns `Result`; `provisioning::reconcile` is now `reconcile_unlocked` (Phase 5's `_unlocked` rule starts here); new fault point `template:mid-save`, and `ALL_FAULT_POINTS` is now enforced against the call sites, so Phase 5 must update the list when it deletes code. |
| 2 | 2026-08-22 | [#8](https://github.com/cristocola/fast-folder/pull/8) | Branched on Phase 1 (#7 still open) — retarget to `main` after it merges. `cli::extra::classify_extra(extra, &clap::Command)` + per-command `apply_extra`; `RegisterFlags::validate` owns register's constraints; `util::tty::{prompt_available, require_tty}` is the one prompt probe (stderr) and Phase 6/7 should route new prompts through it; `RecursiveArgs` gained `vars`; `template from-folder` gained `--yes`/`--dry-run` and `FromFolderArgs`; `Sandbox::run_headless` and `pty::run_stdout_to` are new harness helpers. |
