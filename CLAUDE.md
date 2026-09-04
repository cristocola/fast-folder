# CLAUDE.md — fastf development context

## What this project is

`fastf` (Fast Folder Creator) is a Rust CLI for creating structured project
folders from **folder templates** — code, research, finance, music video,
photography, film. Config, templates and the counter live together in one data
dir; projects live in one or more *bases*.

Two surfaces sit on one core: the guided app (`fastf`, the daily one — a
full-screen ratatui dashboard) and the command line (`fastf new`, …). `core`
and `util` know about neither.

## A workaround is not a fix

fastf is installed by people who will never read this file. A defect is finished
when a released version no longer has it — not when there is a command that
steps around it, a variable to unset first, or a note about which shell to run
the installer from. If an install or a build fails, the answer is the fix, the
tag and the package bump, in that order, and nothing short of that is an answer
to give. The same goes for work left uncommitted "for now": either the tree is
clean and the release is out, or the job is not done.

## Build commands

Standard cargo throughout. Clippy must be clean with `--all-targets -- -D
warnings`, **on Windows targets too** — the lint thresholds differ, and
`large_enum_variant` has fired there and nowhere else — and **in release too**:
`#[cfg(debug_assertions)]` code does not exist there, so an item used only from
a failpoint or tracer test is dead in release and live in debug. The non-obvious
parts:

`.cargo/config.toml` gives `x86_64-pc-windows-msvc` one flag,
`target-feature=+crt-static`, so the Windows binary carries its own C runtime.
Without it the exe imports `VCRUNTIME140.dll` and dies before `main` on any
machine without the Visual C++ Redistributable — a clean install or a fresh VM,
never a developer's box, which is why it shipped that way through v2.0.0. It is
scoped to the one triple, so Linux and musl builds never read it. Deleting it
fails CI: `packaging/windows/assert-standalone.ps1` reads the exe's PE import
table on every Windows job. A `RUSTFLAGS` in the environment *replaces* that
config rather than adding to it.

```bash
# Cross-compile for Windows (from Linux). A local convenience; CI builds on a
# real Windows runner. Needs `rustup target add x86_64-pc-windows-gnu` plus a
# pacman-installed mingw-w64-gcc — and a rustup toolchain, not a pacman one.
cargo build --release --target x86_64-pc-windows-gnu

# Fault injection — trip a named boundary deterministically:
FASTF_FAULT=create:mid-copy cargo test            # returns an error there
FASTF_FAULT=move:before-commit-rename:abort ...   # kills the process there

# Work counting — how many times an expensive thing happened:
FASTF_TRACE_FILE=/tmp/counts cargo test           # one line per traced operation
```

Both `FASTF_FAULT` and `FASTF_TRACE_FILE` are compiled out of release builds, so
their tests are `#[cfg(debug_assertions)]` and debug and release test counts
differ. Several Windows cases are Windows-only and the pty suite is Unix-only.
Do not hard-code a total here; run the gates in [`ROADMAP.md`](ROADMAP.md).

## Project layout

`ls` and the module names cover the shape. This is only what a filename does not
tell you.

- `src/lib.rs` exposes `core/ cli/ tui/ util/ bootstrap/` so integration
  tests can `use fastf::…`. `src/main.rs` is the clap binary.
- `src/bootstrap.rs` — first-run setup. Ships two deliberately universal
  templates (`general`, `client-project`); the domain templates in
  `examples/templates/` are a gallery to copy from, not bundled.
- `src/core/` — the library proper. `library/` (filesystem-as-truth discovery: a
  facade over `model` / `discovery` / `cache` / `guard` / `lifecycle` /
  `resolve`, so every `library::…` path resolves as before), `move_engine.rs`
  (the staged move the facade delegates to — it depends on transactions, staged
  copies and progress, which nothing else in the library does),
  `operations.rs` (the shared mutation boundary), `project.rs` (plan / create /
  apply, and the preview *reports*), `plan.rs` (`ProjectPlan`),
  `transactions.rs` (v2 staged moves), `provisioning.rs` (v2 recovery plus
  report-only pre-v2 discovery), `template_import.rs` (the from-folder engine),
  `assets.rs` (the template-file copy engine: walk, classify, interpolate or
  byte-copy), `validated.rs` (typed slugs, relative paths and project folder
  names), `project_info.rs`.
- `src/util/` — `lockfile` (cross-process `DataLock`; says what it is waiting
  for after a second), `atomic` (THE atomic write), `fs_retry` (Windows
  sharing violations), `interrupt` (Ctrl-C rollback, SIGHUP, and the
  `set_restore` hook the surfaces register for the second signal), `faults`
  (failpoints), `trace` (work counting), `diag` (the one warning sink), `yaml`
  (the one place the YAML crate is named), `time` (one clock), `paths`
  (data-dir resolution, `display_path`, the shared boundary checks including
  `contained_destination` and `is_link_like`, base probing), `shell_open`
  (Windows `ShellExecuteW`), `relaunch` + `notify` (unix-only: the
  headless-GUI terminal relaunch and `notify-send`), `test_env` (the one
  env-mutation guard, test-only), `tree_size`, `size_scan`, `human_bytes`,
  `clipboard`, `tty` (the terminal questions: `require_tty`, `has_display`,
  the remembered cooked mode a signal handler restores).
- `src/cli/` — one module per subcommand (`folder_verbs.rs` is `rename`,
  `unregister` and `delete` together), plus `render.rs`, the only module that
  prints a plan, a create or an apply; `target.rs` (resolve a query to one
  project, asking when it is ambiguous — the flow `open`/`copy`/`path`/`term`
  and the folder verbs share);
  and `terminal.rs` (the `Config`↔`util::relaunch` seam, since `util` may not
  read `Config`). `move_project.rs` is named that way because `move` is a
  keyword, and `path_cmd.rs`/`paths_cmd.rs` because both would otherwise read
  like `std::path` at their call sites.
- `src/tui/` — every interactive terminal surface, all of it ratatui. The
  guided app: `runtime.rs` (the one module that owns the alternate screen, the
  threads and the loop), `entry.rs` (how the app was opened), `app/` (`App`,
  `update`, and one module per flow — `library`, `search`, `actions`, `jobs`,
  `wizard`, `register`, `studio`, `settings`, `palette`, `modal`, `data`),
  `view/` (renderers only, `&App` in), `command.rs` (**the one registry**
  every key, palette entry, help line, key line and hint comes from — the
  dialogs' too), `msg.rs`/`effect.rs`, `theme.rs` (the palette, a pure
  function of an `Env`), `session.rs` (what a run leaves for the next one),
  `frame.rs` (the session ring), `validators.rs` (the prompt texts),
  `fuzzy.rs`, `layout.rs` (every box's geometry, read by `update` and `view`
  alike), `loaders.rs` (the workers' reads), `widgets/` (`input`, `text_area`,
  `form`, `tree`, `nav`), `testing.rs` (fixtures for the suites).
  The command line's own prompts: `inline.rs` (**the other module that may take
  the terminal** — a few rows at the cursor, never the alternate screen),
  `prompt.rs` (the contract over it), `pickers.rs`, `vars.rs`, `rows.rs`.
- `docs/` — the user-facing reference. **When behaviour changes, update the
  matching `docs/` file, not the README.**
- `packaging/` + `.github/workflows/` — release machinery; see the `release`
  skill. Release automation must never mutate installed packages.
- `tests/` — see `tests/CLAUDE.md` for the harness rules and what each suite
  guards.

**Three CLAUDE.md files, each next to what it governs and each loading when you
touch that directory.** This one is orientation, layering, the data-dir and
counter models, the argument layer, and the traps that belong to no single
module. `src/core/CLAUDE.md` is the engine — templates, interpolation, path
safety, `PROJECT_INFO.md`, create/apply/register, moving, recovery, locking,
search, output. `src/tui/CLAUDE.md` is the guided app. `tests/CLAUDE.md` is the
suites. Put a decision beside the code it constrains, not here.

### The layering rule

**`core` and `util` import nothing from `cli` or `tui`, never prompt, and never
print.** Both surfaces run the same functions, and a scripted `fastf new` has
nobody to answer a prompt: a `println!` in `core` is output no caller can
suppress, and `colored` in `core` is ANSI in a piped stdout. `cli` may call `tui`
helpers; new shared interactive code goes in `src/tui/`, not `cli/`.

`tests/layering.rs` enforces all of it by reading the source — an import is not
something a runtime test can see. Two exceptions, named there and nowhere
else: `util::diag` (the warning sink) and `util::trace`. Only `tui::runtime`
and `tui::inline` take the terminal.

## The repository is public

Nothing tracked here may describe the machine it was written on: no real home
directory (`/home/<name>`, `C:\Users\<name>`), no personal mount point, no local
project-folder path, no maintainer's name in prose. Write `/home/user`,
`/mnt/projects/...`, "the maintainer". Attribution is the exception and belongs
in `LICENSE`, `Cargo.toml`, `README.md`, the PKGBUILDs and the installer.

`tests/repo_hygiene.rs` enforces this over `git ls-files`, so the rule fails the
build rather than relying on anyone remembering it. It runs only when the crate
directory is the root of the checkout: the AUR source package builds an unpacked
tarball inside an ignored directory of a real clone, where `git` answers about
the wrong tree. Before tracking a file that was written as a private note, read
it as a stranger would.

---

# Design decisions

## Where fastf keeps its things

`paths::try_install_dir() -> Result<(PathBuf, DirMode)>` resolves the one data
dir (config + templates + counters) in three tiers: `FASTF_INSTALL_DIR`;
**portable mode** — the exe's canonicalized parent iff it holds `config.toml` or
`templates/`, which keeps binary-plus-data folders working; otherwise the user
config dir (`$XDG_CONFIG_HOME/fastf`, `%APPDATA%\fastf`), hand-rolled, no `dirs`
crate. Tier 3 is what makes a package-manager install to a read-only `/usr/bin`
work: `ensure_bootstrapped()` creates the directory on first run.

`install_dir() -> PathBuf` keeps its infallible signature for ~30 call sites but
never panics — it exits(2) with an actionable message, and `main.rs` calls
`try_install_dir()?` first so real users get a pretty error instead. **Never
re-add `.expect` there, and never memoize the resolution**: tests swap the env
var within one process.

A bare binary copied without its data lands in the user config dir *by design*;
the first-run banner names the mode. Surfaced by `fastf paths`, a mode line in
`config show`, and the settings screen's Data locations row. Bootstrap is skipped for
`completions`/`mangen` so packaging steps never write to `$HOME`.

**An unconfigured `base_dir` falls back to HOME, not the cwd.** The cwd fallback
scattered projects and caches into whatever directory a command ran from. This is
why every test harness must redirect `HOME` (`tests/common/env`).

## Configuration

**`Config::load()` is never `unwrap_or_default()`ed.** It already returns
`Ok(default)` when the file is *absent*, so a fallback can only mask a parse or
I/O error — and the config decides which directories are the library, so
defaulting answers a different question with a success. Every surface propagates
it: the CLI exits 1 (`main.rs` adds a `hint:` line naming the file) and the app
never takes the screen — and a launcher-started `fastf` still opens a window,
with the default terminal probe, so the error can be read.

**`config::expand_base_path` / `resolve_base_dir_input` are the only way in for a
base path.** The first expands `~` and requires absolute; the second adds
`create_dir_all` + canonicalize. Extra `bases` use the **first** deliberately:
creating a missing one would plant an empty directory at an unmounted mount point
and shadow the drive it stands for. Neither takes the lock (`DataLock` is not
reentrant and `config::set` already holds it).

**A path that will be stored goes through `util::paths::storable`**, which
refuses rather than mangling: `display().to_string()` substitutes `?` for
non-UTF-8 bytes, and writing that into `config.toml` records a directory that
does not exist.

`effective_bases()` memoizes against the configuration it was computed from — a
`Config` whose `base_dir` was mutated afterwards misses and recomputes, so the
memo can save work but cannot answer the wrong question.

Old keys that no longer exist (`project_info_enabled`, `project_info_filename`,
`pinfo_*`, and `show_banner`/`show_frame`, which went with the menu they drew)
keep parsing because `Config` has no `deny_unknown_fields`; `config set` accepts
the two retired ones and says they are ignored, because a script that still sets
one must not start failing. `recent-default-limit` was renamed `recent-limit` at
v3.0.0 and the old key still parses. Do not re-add a metadata-filename knob: the
reservation and discovery both assume the fixed name.

## Filesystem as truth

There is no project database. **A folder is a project iff it holds a
`PROJECT_INFO.md`**, whose frontmatter `id` is authoritative; the folder name is
cosmetic and never consulted for discovery. `discover(cfg)` unions
`cfg.effective_bases()`, newest first. Discovery is **depth-1** (`SCAN_DEPTH` in
`library/model.rs`) — direct children only, matching flat project layouts.

Each base carries a **disposable** `.fastf-index.json` at its root, co-located
with the projects so it travels with them; entries store a base-relative `dir`,
valid across `/mnt/…` and `D:\…`. It is never authority: `discover_base`
self-heals (rescan when the base mtime is newer, existence-check every cached
entry otherwise), all writes are best-effort and atomic, and a rejected cache
version costs one rescan rather than hiding projects. `write_cache` re-stamps
the index after the rename that publishes it: the rename bumps the base
directory *after* the file was written, and on the kernel's coarse file clock
that left the base a tick newer than its own index every so often, so the next
discovery rescanned for nothing. **No manual prune, ever** —
"missing" is a transient state. `fastf reindex` forces a full rescan for external
edits fastf cannot observe.

`library::max_id(cfg)` **must stay read-only** — `plan()`/preview call it through
the counter self-heal, so it uses `read_base_readonly` (fresh cache or scan, no
write), never `discover` (which writes). Route it through `discover` and previews
start writing caches.

**A `CacheEntry`'s `dir` must be exactly one ordinary path component, not
dot-prefixed.** `into_project` returns `Option` and drops anything else: an
absolute `dir` *replaces* the base under `Path::join`, and `../..` survived the
`strip_prefix` on the next rewrite, so an entry reading `/etc` produced a
"project" at `/etc`. One component because discovery is depth-1; not
dot-prefixed because `scan_base` skips those. A rejected entry is **not** treated
like a vanished folder — a folder that has gone is transient and the row is
dropped, but an entry pointing outside its base means the file is no longer
fastf's own bookkeeping, so the cache is abandoned and the base rescanned.

`library::revalidate_for_read` is the cheap sibling of `guard`'s mutation
revalidation, for handing a discovered path to **another program**: a real
directory, a direct child of its own base, holding a real `PROJECT_INFO.md`. No
canonicalize, no config reload, no id check — those protect a mutation. `fastf
open` and the TUI's Reveal call it before spawning the file manager. Ordinary
metadata reads keep trusting discovery: after the one-component rule the path is
a direct child by construction, and reading the user's own `PROJECT_INFO.md` is
what discovery is.

`scan_base` skips dot-prefixed directories, including `.fastf-transactions`, so
private staging containing a `PROJECT_INFO.md` cannot appear as a duplicate
project.

Every `Project` carries `base: PathBuf`, set at both construction points
(`CacheEntry::into_project` and `project_from_meta`). **Do not add `base` to
`CacheEntry`** — base-relative portability is the point of the format.
`library::base_label(base)` is the short display name every list uses.

## The global ID counter

One counter for all templates, stored as **`<base>/.fastf-counter.toml`** beside
that base's index. It moved out of the data directory because of where it must be
*readable* from: `%APPDATA%\fastf` and `~/.config/fastf` are different files, so a
dual-boot machine had two counters. The projects never had that problem — they
already sit on a drive both systems mount.

`Counters::floor(cfg)` takes the max of three inputs: every mounted base's
counter file (shared across operating systems); this machine's data-dir
`counters.toml` (which spans every base it has written to, so an unplugged drive
cannot restart numbering); and `library::max_id(cfg)`, the highest ID actually in
metadata (which is why losing a counter file is untidy rather than harmful).

**The number only ever goes up, and every base converges on it.**
`Counters::record` writes the target base then `propagate`s to every other
mounted base plus the data dir. Three bases holding ID0004 / ID0082 / ID0017 all
come out at 82. Propagating on **every create** is load-bearing: if Linux mints
ID0101 in a base Windows cannot see, the base Windows *can* see has to learn now.

`Counters::save_base` is monotonic and returns **whether it wrote**, so
`propagate` calls `library::touch_cache` only for bases it actually touched —
without that re-stamp, every create would invalidate every other base's cache.

**`Counters::next_value(cfg, counters)` is the one expression for "which ID comes
next."** Three callers drifted before, and the preview confirmed `ID0001` while
the commit wrote `ID0011`. There is no way to lower it: `id set` refuses anything
at or below the floor and names what holds it, and `id reset` is gone.

**The counter is bounded at both ends.** `Counters::MAX_VALUE` is
`999_999_999_999` — twelve digits, so every reachable value renders inside the
widest `id.digits` a template may declare, and below 2^53 so a JSON consumer
reads it exactly. `next_value` returns `Result<u64>` and `checked_add`s against
it; `operations::set_counter` refuses anything above it. Without the ceiling,
`id set` accepted `u64::MAX` (above the floor was the only rule) and the next
create's `+ 1` overflowed: a panic in debug, a wrap to zero in release.

`Counters::propagate` **must not** `unwrap_or_default()` the data-dir counter.
A read error reads as zero, zero is below everything, so the write proceeds and
overwrites what could not be read — the exact file whose job is to stop an
unplugged base from restarting numbering. It warns and skips instead.

`naming::id_value` rejects any id containing a hyphen. An interim build wrote
UUIDs, and reading the trailing digits of one would put the floor at 20044.

**Known limit:** `DataLock` is per data-directory, so two *different machines*
writing one shared base are not serialized and can mint the same number.
Same-machine concurrency is safe.

## Answering the launcher

fastf is started from a shell **and** from a desktop launcher, and from a
launcher there is no terminal at all: stdin `/dev/null`, stdout and stderr
journald **sockets**, nothing printed ever seen. Three rules came out of that,
and they are the ones to keep straight.

**The picker serves the verb it interrupted.** An ambiguous `open`/`copy`/`path`
shows `tui::pickers::pick_project` and then performs the verb that was typed. It
must never open the project action menu — that is `fastf` and `fastf recent`.
`cli::target::one_project` is the one flow, returning `Target::{Project,
Cancelled, HandedOff}`; the last two are both exit 0 and are **not**
interchangeable, because reporting a relaunch as `Cancelled —` writes a lie into
the journal of a command that was just handed to a window.

**The picker's gate is stderr, not stdout.** `recent`/`search` probe stdout
because there it chooses an output *format*. "Can I ask?" is `util::tty`'s
question and its answer is stderr — gating on stdout would make the picker
unreachable from `cd "$(fastf path lullaby)"`, which redirects stdout by
construction, and would reintroduce the exact defect `util::tty` exists to fix.

**"I am the rerun" is a flag on argv; the environment only ever says "do not
relaunch".** `respawn_in_terminal` puts a hidden `--relaunched` ahead of the
argv it repeats, and `cli::terminal::relaunched_window()` is the only reader —
the pause `main` takes before a window closes, and `window_is_ours()`. An
environment variable cannot carry that claim: `FASTF_RELAUNCHED` is inherited by
the window's shell and by everything typed into it, which is how `fastf
completions bash` came to stop for a keypress inside a package build and `fastf
term proj` came to replace the shell it was typed in. The variable keeps only
the half inheritance cannot spoil — suppressing a relaunch — because a
descendant that reads it wrongly opens no window, and no window is the safe
direction.

**The relaunch fires only where output provably has no reader.** All of:
no stream is a TTY; stdout *and* stderr are each a socket, character device, or
closed (`EBADF`) — never a regular file or FIFO, which mean somebody is keeping
the bytes; a display is set; `SSH_CONNECTION` unset; neither `FASTF_RELAUNCHED`
(the loop guard, set on the child) nor `FASTF_NO_RELAUNCH` set. `INVOCATION_ID`
and `JOURNAL_STREAM` are useless discriminators — a systemd-managed desktop sets
them for everything. Two accepted misfires, documented with their three escape
hatches (`--plain`, `FASTF_NO_RELAUNCH=1`, `terminal = "none"`): a systemd user
service running an interactive fastf command with the session display imported,
and cron with `>/dev/null 2>&1` **plus** an exported display.

The emulator argv conventions are not interchangeable and the wrong one silently
does something else: `gnome-terminal --` (its `-e` is deprecated and takes one
string), `xfce4-terminal -x` (its `-e` shell-parses one string), `xterm -e` last,
`kitty`/`foot` trailing. `candidate_commands` is pure so each is unit-tested.
argv is passed as argv, never through a shell.

`util::tty::mark_interactive_surface` has exactly **two** choke points —
`require_tty`'s success path (every inline prompt and picker) and
`tui::runtime::Runtime::init` (the guided app, after its own `require_tty`) —
and `main` reads it only to decide whether a relaunched window pauses before
closing.

Everything above is `cfg(unix)`. Windows allocates a console for a console
application launched from the shell surface, so none of these modules exist
there — and the Windows clippy leg must stay clean with them absent.

---

# Flags and arguments

**`cli::extra::classify_extra` reads the flag list from clap, not a hand-written
match.** `trailing_var_arg` means every token after the first one clap cannot
parse lands in `extra`; the classifier sorts that bucket using
`cmd.get_arguments()` for *that* subcommand. So adding a flag is two steps:
declare it in clap, handle it in that command's `apply_extra`. The `_ =>` arm
bails by name and `main.rs`'s exhaustiveness test calls it with every declared
long, so forgetting the second step fails the suite instead of making the flag
work before the positional and silently do nothing after it. An undeclared
`--key=value` is a template variable; anything else is an error.

**`trailing_var_arg` also means clap's `requires`/`conflicts_with` only see flags
typed *before* the positional.** `register --dry-run` after the path lands in
`extra`, so the attribute never fired — which is how `--dry-run` came to write the
folder for real. The constraint lives in `RegisterFlags::validate`, run on the
merged set. Keep the clap attributes for the help text; put the enforcement in
`validate`.

**Do not read a `Config` field raw when a `resolve_*` exists.** `cli::note` passed
`&cfg.editor` where everything else calls `cfg.resolve_editor()`, so the
documented `$EDITOR` fallback never ran.

---

# Gotchas that are just true

- Rust 2024 makes `std::env::set_var`/`remove_var` unsafe, and `setenv` is not
  thread-safe underneath. **One guard per binary**: `util::test_env` under
  `src/`, `tests/common/env` under `tests/`, and `tests/layering.rs` fails the
  build if either name appears anywhere else.
- `clippy::field_reassign_with_default` is allowed at the test-file level;
  rewriting every `Config::default()` builder into struct-literal form is churn.
- The interrupt flag is process-global, so its test lock lives beside it
  (`interrupt::TEST_LOCK`) — a private mutex per test module looks right and
  silently races. Lock order is `test_env::ENV_LOCK` first, then that one.
  `faults` needs no lock: its arming is thread-local.
- Concurrency tests must spawn **processes**, not threads: a thread test passes
  against an in-process `Mutex` while production stays broken.
- **A Windows thread's stack is 1 MiB, not the main thread's 8.** The app's
  size scan runs on worker threads, so any recursion bound must hold there —
  `paths::MAX_WALK_DEPTH` is 64 for that reason, not 256.
- **Never match a source path with a `/` suffix.** `Path::display()` uses the
  platform separator, so `shown.ends_with("util/diag.rs")` silently matches
  nothing on Windows. Compare `file_name()`, or use `Path::ends_with`, which is
  component-wise.
- Do not bulk-edit source with PowerShell `Get-Content -Raw` + `Set-Content`. 5.1
  reads as the ANSI codepage and writes UTF-8, double-encoding every non-ASCII
  character; this repo is full of `—`, `→`, `…`, `✓`. It silently corrupted nine
  files and broke a `char` literal.
- `util::yaml` is the only module that names the YAML crate, and
  `the_emitted_bytes_are_the_ones_we_have_always_emitted` pins its output. These
  files are diffed, committed and hand-edited by users, so the bytes may not move.
