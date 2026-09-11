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

Standard cargo. Clippy is clean with `--all-targets -- -D warnings` **on Windows
too**, where some thresholds differ (`large_enum_variant` fires there alone), and
**in release too**, where `#[cfg(debug_assertions)]` code does not exist and an
item used only from a failpoint or tracer test is dead. The full gate list is in
the `release` skill (`.claude/skills/release/SKILL.md`).

`.cargo/config.toml` sets `target-feature=+crt-static` for
`x86_64-pc-windows-msvc` alone, so the Windows exe carries its own C runtime and
starts where no Visual C++ Redistributable is installed — a clean install, a
fresh VM, never a developer's box. `packaging/windows/assert-standalone.ps1`
checks the PE import table on every Windows job. A `RUSTFLAGS` in the environment
*replaces* that config rather than adding to it.

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

`FASTF_FAULT` and `FASTF_TRACE_FILE` are compiled out of release builds, so their
tests are `#[cfg(debug_assertions)]`; with Windows-only cases and the Unix-only
pty suite, test totals depend on platform and profile, so never hard-code one.

## Project layout

`ls` and the module names cover the shape. This is only what a filename does not
tell you.

- `src/lib.rs` exposes `core/ cli/ tui/ util/ bootstrap/` so integration tests
  can `use fastf::…`. `src/main.rs` is the clap binary.
- `src/bootstrap.rs` — first-run setup. Ships two deliberately universal
  templates (`general`, `client-project`); `examples/templates/` is a gallery to
  copy from, not bundled.
- `src/core/` — the library proper. `library/` (filesystem-as-truth discovery, a
  facade over `model` / `discovery` / `cache` / `guard` / `lifecycle` /
  `resolve`), `move_engine.rs` (the staged move the facade delegates to — it
  needs transactions, staged copies and progress, which nothing else in the
  library does), `copy_engine.rs` (a move that keeps its source),
  `operations.rs` (the shared mutation boundary), `project.rs` (plan / create /
  apply, and the preview *reports*), `plan.rs` (`ProjectPlan`),
  `transactions.rs` (v2 staged moves), `provisioning.rs` (v2 recovery plus
  report-only pre-v2 discovery), `template_import.rs` (the from-folder engine),
  `assets.rs` (the template-file copy engine: walk, classify, interpolate or
  byte-copy), `body.rs` (the grammar of `PROJECT_INFO.md`'s body: sections,
  notes, todos), `validated.rs` (typed slugs, relative paths, tags, project
  folder names), `project_info.rs`.
- `src/util/` — `lockfile` (cross-process `DataLock`; says what it waits for
  after a second), `atomic` (THE atomic write), `fs_retry` (Windows sharing
  violations, and the read-only attribute a publish must set aside), `interrupt`
  (Ctrl-C rollback, SIGHUP, and the `set_restore` hook for the second signal),
  `faults` (failpoints), `trace` (work counting), `diag` (the one warning sink),
  `yaml` (the one place the YAML crate is named), `time` (one clock), `paths`
  (data-dir resolution, `display_path`, the boundary checks including
  `contained_destination` and `is_link_like`, base probing), `shell_open`
  (Windows `ShellExecuteW`), `relaunch` + `notify` (unix-only: the headless-GUI
  terminal relaunch and `notify-send`), `term_open` (an emulator whose shell
  starts in a project's folder: `fastf term`, "Open terminal here"), `test_env`
  (the one env-mutation guard, test-only), `tree_size`, `size_scan`,
  `human_bytes`, `clipboard`, `tty` (`require_tty`, `has_display`, the remembered
  cooked mode a signal handler restores).
- `src/cli/` — one module per subcommand (`folder_verbs.rs` is `rename`,
  `unregister` and `delete`), plus `render.rs`, the only module that prints a
  plan, a create or an apply; `target.rs` (resolve a query to one project, asking
  when it is ambiguous — shared by `open`/`copy`/`path`/`term`/`cd` and the
  folder verbs); `cd_cmd.rs` + `shell_init.rs` (the two halves of `fastf cd`:
  the binary prints the path, the emitted shell function enters it — nothing
  else can); `terminal.rs` (the `Config`↔`util::relaunch` seam, since `util` may
  not read `Config`). `move_project.rs`, `path_cmd.rs` and `paths_cmd.rs` are
  named around a keyword and `std::path`.
- `src/tui/` — every interactive terminal surface, all ratatui. The guided app:
  `runtime.rs` (the one owner of the alternate screen, the threads and the loop),
  `entry.rs` (how the app was opened), `app/` (`App`, `update`, and a module per
  flow — `library`, `search`, `actions`, `jobs`, `wizard`, `register`, `studio`,
  `settings`, `palette`, `pane`, `modal`, `data`), `view/` (renderers only, `&App`
  in), `command.rs` (**the one registry** every key, palette entry, help line,
  key line and hint comes from), `guide.rs` (**the one place** an explanation is
  written: the builder's panel, the guide, the coach), `motion.rs` (pure motion
  arithmetic over milliseconds it is handed), `msg.rs`/`effect.rs`, `theme.rs`
  (the palette, a pure function of an `Env`), `session.rs` (what a run leaves for
  the next), `frame.rs` (the session ring), `validators.rs` (the prompt texts),
  `fuzzy.rs`, `layout.rs` (every box's geometry, read by `update` and `view`),
  `loaders.rs` (the workers' reads), `widgets/` (`input`, `text_area`, `form`,
  `tree`, `nav`), `testing.rs` (suite fixtures). The command line's prompts:
  `inline.rs` (**the other module that may take the terminal** — rows at the
  cursor, never the alternate screen), `prompt.rs` (the contract over it),
  `pickers.rs`, `vars.rs`, `rows.rs`.
- `docs/` — the user-facing reference, six guides. **When behaviour changes,
  update the matching `docs/` file, not the README.**
- `packaging/` + `.github/workflows/` — release machinery; see the `release`
  skill. Release automation must never mutate installed packages.
- `tests/` — see `tests/CLAUDE.md`.

**Four CLAUDE.md files, each loading when you touch its directory.** This one is
orientation, layering, the data-dir and counter models, the argument layer and
the traps that belong to no single module; `src/core/CLAUDE.md` is the engine,
`src/tui/CLAUDE.md` the guided app, `tests/CLAUDE.md` the suites and harness
rules. Put a decision beside the code it constrains.

### The layering rule

**`core` and `util` import nothing from `cli` or `tui`, never prompt, and never
print**, because both surfaces run the same functions and a scripted `fastf new`
has nobody to answer: a `println!` in `core` is output no caller can suppress, and
`colored` there is ANSI in a piped stdout. `cli` may call `tui` helpers; new
shared interactive code goes in `src/tui/`. `tests/layering.rs` enforces it by
reading the source, with two named exceptions, `util::diag` and `util::trace`.
Only `tui::runtime` and `tui::inline` take the terminal.

## The repository is public

Nothing tracked may describe the machine it was written on: no real home
directory (`/home/<name>`, `C:\Users\<name>`), no personal mount point, no local
project-folder path, no maintainer's name in prose, no personal email address.
Write `/home/user`, `/mnt/projects/...`, "the maintainer". Attribution is the
exception — `LICENSE`, `Cargo.toml`, `README.md`, the PKGBUILDs, the installer —
and its address is `hello@argyrolabs.com`. `tests/repo_hygiene.rs` enforces this
over `git ls-files`, only when the crate is the checkout's root (the AUR source
build unpacks inside an ignored directory of a real clone). Read a private note as
a stranger would before tracking it.

---

# Design decisions

## Where fastf keeps its things

`paths::try_install_dir() -> Result<(PathBuf, DirMode)>` resolves the one data
dir in three tiers: `FASTF_INSTALL_DIR`; **portable mode** — the exe's
canonicalized parent iff it holds `config.toml` or `templates/`; otherwise the
user config dir (`$XDG_CONFIG_HOME/fastf`, `%APPDATA%\fastf`), hand-rolled, no
`dirs` crate. Tier 3 is what lets a package install to a read-only `/usr/bin`
work; `ensure_bootstrapped()` creates it on first run and is skipped for
`completions`/`mangen`, so packaging never writes to `$HOME`. A bare binary lands
in tier 3 by design; `fastf paths`, `config show` and the settings screen name the
mode.

`install_dir() -> PathBuf` stays infallible for its many call sites but exits(2)
with a message instead of panicking; `main.rs` calls `try_install_dir()?` first
for a readable error. **Never re-add `.expect` there, and never memoize the
resolution** — tests swap the env var within one process.

**An unconfigured `base_dir` falls back to HOME, not the cwd**, which would
scatter projects and caches wherever a command ran — and it is why every test
harness redirects `HOME`.

## Configuration

**`Config::load()` is never `unwrap_or_default()`ed.** It already returns
`Ok(default)` for an *absent* file, so a fallback only masks a parse or I/O error,
and the config decides which directories are the library. The CLI exits 1 with a
`hint:` naming the file; the app never takes the screen, and a launcher-started
`fastf` still opens a window so the error can be read.

**`config::expand_base_path` / `resolve_base_dir_input` are the only way in for a
base path.** The first expands `~` and requires absolute; the second also creates
and canonicalizes. Extra `bases` use the first, because creating a missing one
would plant an empty directory at an unmounted mount point. Neither takes the
lock (`DataLock` is not reentrant, and `config::set` holds it).

**A path that will be stored goes through `util::paths::storable`**, which refuses
non-UTF-8 rather than recording the `?`-substituted path `display()` produces.
`effective_bases()` memoizes against the configuration it was computed from, so a
mutated `Config` recomputes instead of answering the wrong question.

**The key `config set` takes is the key `config.toml` holds is the key `config
show` prints.** `Config` has no `deny_unknown_fields`, so a mismatched spelling is
silently ignored; a field whose Rust name differs carries `serde(rename)` plus an
`alias` for older spellings (`recent_default_limit`).

**Retired keys stay harmless.** `config set` accepts `show_banner`, `show_frame`
and `mouse` and says each is unused, so no script starts failing;
`project_info_enabled`, `project_info_filename`, `pinfo_*` and
`recent-default-limit` still parse. Do not re-add a metadata-filename knob: the
reservation and discovery both assume the fixed name.

## Filesystem as truth

There is no project database. **A folder is a project iff it holds a
`PROJECT_INFO.md`**, whose frontmatter `id` is authoritative; the folder name is
cosmetic. `discover(cfg)` unions `cfg.effective_bases()`, newest first, at
**depth 1** (`SCAN_DEPTH` in `library/model.rs`). `scan_base` skips dot-prefixed
directories, so `.fastf-transactions` staging never appears as a duplicate.

Each base carries a **disposable** `.fastf-index.json` with base-relative `dir`
entries, so it travels with the projects across `/mnt/…` and `D:\…`. It is never
authority: `discover_base` rescans when the base mtime is newer and
existence-checks entries otherwise, writes are best-effort and atomic, and a
rejected cache costs one rescan. `write_cache` re-stamps the index after the
rename that publishes it, or that rename's mtime bump would make the base look
newer than its own index. **No manual prune, ever** — "missing" is transient;
`fastf reindex` rescans for edits fastf cannot observe.

`library::max_id(cfg)` **must stay read-only** — previews reach it through the
counter self-heal — so it uses `read_base_readonly`, never `discover`. **It
abandons a bad cache the way `discover_base` does** and never reads the entries
that survive, because it is the counter floor, and a dropped highest entry would
mint a duplicate.

**A `CacheEntry`'s `dir` must be one ordinary, non-dot path component**;
`into_project` drops anything else, because an absolute `dir` replaces the base
under `Path::join` and `..` climbs out. A rejected entry abandons the whole cache —
the file is no longer fastf's bookkeeping — unlike a vanished folder, which only
drops its row.

**A folder fastf cannot read is not a folder that is not a project.**
`read_project_meta_reporting` answers `NotAProject::NoMetadata` (silent — every
base has ordinary folders) or `Unreadable` (bad bytes, no `---`, YAML that will
not deserialize), which `project_at` names through `diag::warn`, so a project
never leaves the library in silence.

**Only `id` and `template` are required to deserialize**; `template_name`,
`created`, `folder` and `path` are `#[serde(default)]`, because deserialization is
all-or-nothing and one deleted line in a hand-edited file must not drop a project
(`folder_created_fallback` covers `created`; `project_from_meta` re-derives
`folder`/`path`). Serialization is unchanged.

`library::revalidate_for_read` is the cheap check before handing a discovered
path to **another program** (`fastf open`, the app's Reveal): a real directory, a
direct child of its base, holding a real `PROJECT_INFO.md`. No canonicalize,
reload or id check — those protect mutations.

Every `Project` carries `base: PathBuf`, set in `CacheEntry::into_project` and
`project_from_meta`. **Do not add `base` to `CacheEntry`** — base-relative
portability is the point of the format. `library::base_label(base)` is the short
name every list shows.

## The global ID counter

One counter for all templates, at **`<base>/.fastf-counter.toml`** beside each
base's index, because a dual-boot machine's data dirs (`%APPDATA%\fastf`,
`~/.config/fastf`) are different files while its bases sit on drives both systems
mount.

`Counters::floor(cfg)` is the max of every mounted base's counter file, this
machine's data-dir `counters.toml` (which remembers bases that are unplugged), and
`library::max_id(cfg)` (so losing a counter file is untidy, not harmful).

**The number only goes up, and every base converges on it.** `Counters::record`
writes the target base, then `propagate`s to every other mounted base and the data
dir on **every create**, so a base the other OS can see learns of an ID minted
where it cannot. `Counters::save_base` is monotonic and reports whether it wrote,
so `propagate` re-stamps (`library::touch_cache`) only the caches it touched.

**`Counters::next_value(cfg, counters)` is the one expression for "which ID comes
next"**, so a preview and its commit agree. Nothing lowers it: `id set` refuses
anything at or below the floor and names what holds it.

**The counter is bounded at both ends.** `Counters::MAX_VALUE` is
`999_999_999_999`: inside the widest `id.digits`, below 2^53 for JSON readers.
`next_value` `checked_add`s against it and `operations::set_counter` refuses above
it, or `+ 1` on a huge value panics in debug and wraps to zero in release.

**Neither `Counters::propagate` nor `Counters::floor` `unwrap_or_default()`s the
data-dir counter.** An unreadable file read as zero would be overwritten by the
one and ignored by the other, and it is the file that stops an unplugged base
restarting numbering. Both use `Counters::load_or_report`, which warns **once per
process** (the floor runs several times per command) and answers `None`. For the
same reason `cli::id::print_counter` calls an unreadable counter unknown, never
"at its ceiling".

`naming::id_value` rejects any id containing a hyphen, so a UUID-shaped id never
raises the floor.

**Known limit:** `DataLock` is per data directory, so two *machines* writing one
shared base can mint the same number. Same-machine concurrency is safe.

## Answering the launcher

From a desktop launcher there is no terminal: stdin `/dev/null`, stdout and stderr
journald **sockets**, nothing printed ever seen. Three rules follow.

**The picker serves the verb it interrupted.** An ambiguous `open`/`copy`/`path`
shows `tui::pickers::pick_project`, then performs the typed verb — never the
action menu, which is `fastf` and `fastf recent`. `cli::target::one_project`
returns `Target::{Project, Cancelled, HandedOff}`; the last two both exit 0 but
are not interchangeable, or a relaunch is journaled as `Cancelled —`.

**The picker's gate is stderr, not stdout.** `recent`/`search` probe stdout to
choose an output *format*; "can I ask?" is `util::tty`'s question, answered on
stderr, or `cd "$(fastf path lullaby)"` could never show the picker.

**"I am the rerun" is a flag on argv; the environment only says "do not
relaunch".** `respawn_in_terminal` prepends `--relaunched`;
`main::take_the_relaunch_flag` strips it from that first position **before clap
sees it** (a hidden declared flag would still reach the shell completions), and
`cli::terminal::relaunched_window()` is its only reader — the pause before a
window closes, and `window_is_ours()`. A variable is inherited by the window's
shell and everything typed there, so `FASTF_RELAUNCHED` only suppresses a
relaunch: a descendant that misreads it opens no window, the safe direction.

**The relaunch fires only where output provably has no reader**: no stream is a
TTY; stdout *and* stderr are each a socket, character device or closed (`EBADF`) —
never a file or FIFO, which somebody is keeping; a display is set;
`SSH_CONNECTION` is unset; neither `FASTF_RELAUNCHED` (the loop guard) nor
`FASTF_NO_RELAUNCH` is set. `INVOCATION_ID` and `JOURNAL_STREAM` discriminate
nothing on a systemd desktop. Two misfires are accepted — a systemd user service
running an interactive command with the display imported, and cron with
`>/dev/null 2>&1` plus an exported display — with three documented escape hatches
(`--plain`, `FASTF_NO_RELAUNCH=1`, `terminal = "none"`).

Emulator argv conventions differ and the wrong one silently does something else:
`gnome-terminal --`, `xfce4-terminal -x`, `xterm -e` last, `kitty`/`foot`
trailing. `candidate_commands` is pure so each is unit-tested; argv is passed as
argv, never through a shell.

`util::tty::mark_interactive_surface` has exactly **two** choke points —
`require_tty`'s success path and `tui::runtime::Runtime::init` — and `main` reads
it only to decide whether a relaunched window pauses before closing.

Everything above is `cfg(unix)`: Windows gives a console application launched from
the shell a console, so these modules do not exist there, and the Windows clippy
leg must stay clean without them.

---

# Flags and arguments

**`cli::extra::classify_extra` reads the flag list from clap**
(`cmd.get_arguments()` for that subcommand), because `trailing_var_arg` puts every
token after the first one clap cannot parse into `extra`. Adding a flag is two
steps: declare it in clap, handle it in that command's `apply_extra`. The `_ =>`
arm bails by name and `main.rs`'s exhaustiveness test calls it with every declared
long, so a forgotten second step fails the suite instead of making the flag work
only before the positional. An undeclared `--key=value` is a template variable;
anything else is an error.

**A declared flag is parsed wherever it appears; only an undeclared token starts
the trailing bucket** (clap 4.6, per `Cargo.lock`). So `register <path>
--dry-run` meets clap's `requires`/`conflicts_with` and exits 2, while `register
<path> --artist=X --dry-run` latches into `extra`, reaches
`RegisterFlags::validate`, and exits 1. Both layers stay and `validate` is the
authority; if they are ever unified, unify onto it, because a rule only clap
enforces misses every flag that arrives through `extra`.

**Do not read a `Config` field raw when a `resolve_*` exists** — `cfg.editor`
skips the `$EDITOR` fallback `cfg.resolve_editor()` applies.

---

# Gotchas that are just true

- The test harness rules — one env-mutation guard per binary, processes not
  threads, the lock order, the `HOME` redirect — are in `tests/CLAUDE.md`.
- `clippy::field_reassign_with_default` is allowed at the test-file level;
  rewriting every `Config::default()` builder is churn.
- **Never match a source path with a `/` suffix**: `Path::display()` uses the
  platform separator, so `ends_with("util/diag.rs")` never matches on Windows.
  Compare `file_name()`, or use the component-wise `Path::ends_with`.
- Do not bulk-edit source with PowerShell 5.1 `Get-Content -Raw` +
  `Set-Content`: it reads the ANSI codepage and writes UTF-8, double-encoding every
  `—`, `→`, `…` and `✓` in this repo, `char` literals included.
- `util::yaml` is the only module that names the YAML crate, and
  `the_emitted_bytes_are_the_ones_we_have_always_emitted` pins its output: users
  diff, commit and hand-edit these files, so the bytes may not move.
