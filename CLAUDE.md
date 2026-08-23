# CLAUDE.md — fastf development context

## What this project is

`fastf` (Fast Folder Creator) is a Rust CLI for creating structured project
folders from **folder templates** — code, research, finance, music video,
photography, film. Config, templates and the counter live together in one data
dir; projects live in one or more *bases*.

Three surfaces sit on one core: the guided terminal menu (`fastf`, the daily
one), the command line (`fastf new`, …), and a local browser UI (`fastf ui`).
`core` and `util` know about none of them.

## Build commands

Standard cargo throughout. Clippy must be clean with `--all-targets -- -D
warnings`, **on Windows targets too** — the lint thresholds differ, and
`large_enum_variant` has fired there and nowhere else. The non-obvious parts:

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

# Browser UI — same `cargo build` (server + embedded frontend live in the lib)
FASTF_UI_DIR=src/ui/web cargo run -- ui   # frontend live-reload (assets from disk)
node --check src/ui/web/app.js            # run after every frontend edit
```

Both `FASTF_FAULT` and `FASTF_TRACE_FILE` are compiled out of release builds, so
their tests are `#[cfg(debug_assertions)]` and debug and release test counts
differ. Several Windows cases are Windows-only and the pty suite is Unix-only.
Do not hard-code a total here; run the gates in [`ROADMAP.md`](ROADMAP.md).

## Project layout

`ls` and the module names cover the shape. This is only what a filename does not
tell you.

- `src/lib.rs` exposes `core/ cli/ tui/ ui/ util/ bootstrap/` so integration
  tests can `use fastf::…`. `src/main.rs` is the clap binary.
- `src/bin/fastf-ui.rs` — second `[[bin]]`, a windowless Windows launcher shim
  over `cli::ui::run(app: true)`. **Not a second server**; the server lives only
  in `src/ui/`. Errors surface through a raw `MessageBoxW` extern, because std
  discards `println!` with no console attached. Only Windows artifacts ship it.
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
  byte-copy), `validated.rs` (typed slugs and relative paths), `project_info.rs`.
- `src/util/` — `lockfile` (cross-process `DataLock`), `atomic` (THE atomic
  write), `fs_retry` (Windows sharing violations), `interrupt` (Ctrl-C
  rollback), `faults` (failpoints), `trace` (work counting), `diag` (the one
  warning sink), `yaml` (the one place the YAML crate is named), `time` (one
  clock), `paths` (data-dir resolution, `display_path`, the shared boundary
  checks, base probing), `tree_size`, `size_scan`, `live_select`,
  `human_bytes`, `clipboard`, `tty`.
- `src/cli/` — one module per subcommand, plus `render.rs`, the only module that
  prints a plan, a create or an apply. `move_project.rs` is named that way
  because `move` is a keyword.
- `src/tui/` — every interactive terminal surface: `menu.rs` (the guided menu and
  Settings), `frame.rs` (the library summary under it), `browser.rs` (the paged
  project browser), `actions.rs` (the project action menu), `rows.rs` (the one
  project-row builder), `pickers.rs` (one template picker, one base picker),
  `prompt.rs` (**the only module that may name a dialoguer prompt**), `vars.rs`,
  `template_builder.rs`.
- `src/ui/` — the browser UI. Has its own CLAUDE.md; `docs/UI.md` is the
  route-by-route reference. Do not re-list endpoints here.
- `docs/` — the user-facing reference. **When behaviour changes, update the
  matching `docs/` file, not the README.**
- `packaging/` + `.github/workflows/` — release machinery; see the `release`
  skill. Release automation must never mutate installed packages.
- `tests/` — see `tests/CLAUDE.md` for the harness rules and what each suite
  guards.

**Four CLAUDE.md files, each next to what it governs and each loading when you
touch that directory.** This one is orientation, layering, the data-dir and
counter models, the argument layer, and the traps that belong to no single
module. `src/core/CLAUDE.md` is the engine — templates, interpolation, path
safety, `PROJECT_INFO.md`, create/apply/register, moving, recovery, locking,
search, output. `src/tui/CLAUDE.md` is the guided menu. `src/ui/CLAUDE.md` is the
browser UI (`docs/UI.md` is its route-by-route reference). `tests/CLAUDE.md` is
the suites. Put a decision beside the code it constrains, not here.

### The layering rule

**`core` and `util` import nothing from `cli`, `tui` or `ui`, never prompt, and
never print.** Three surfaces run the same functions: a `println!` in `core` is
output the browser cannot suppress, and `colored` in `core` is ANSI in a JSON
response. `cli` may call `tui` helpers; new shared interactive code goes in
`src/tui/`, not `cli/`.

`tests/layering.rs` enforces all of it by reading the source — an import is not
something a runtime test can see. Three exceptions, named there and nowhere
else: `util::diag` (the warning sink), `util::live_select` (which draws a picker
by design), `util::trace`.

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
`config show`, and `dir_mode` in `/api/state`. Bootstrap is skipped for
`completions`/`mangen` so packaging steps never write to `$HOME`.

**An unconfigured `base_dir` falls back to HOME, not the cwd.** The cwd fallback
scattered projects and caches into whatever directory a command ran from. This is
why every test harness must redirect `HOME` (`tests/common/env`).

## Configuration

**`Config::load()` is never `unwrap_or_default()`ed.** It already returns
`Ok(default)` when the file is *absent*, so a fallback can only mask a parse or
I/O error — and the config decides which directories are the library, so
defaulting answers a different question with a success. Every surface propagates
it: the CLI exits 1 (`main.rs` adds a `hint:` line naming the file), the TUI
refuses to open a menu, the browser's `ui::load_config` tags it with
`SERVER_ERROR_PREFIX` so `status_for` answers 500 rather than blaming the
request.

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
`pinfo_*`) keep parsing because `Config` has no `deny_unknown_fields`. Do not
re-add a metadata-filename knob: the reservation and discovery both assume the
fixed name.

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
version costs one rescan rather than hiding projects. **No manual prune, ever** —
"missing" is a transient state. `fastf reindex` forces a full rescan for external
edits fastf cannot observe.

`library::max_id(cfg)` **must stay read-only** — `plan()`/preview call it through
the counter self-heal, so it uses `read_base_readonly` (fresh cache or scan, no
write), never `discover` (which writes). Route it through `discover` and previews
start writing caches.

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
at or below the floor and names what holds it, `id reset` is gone, and
`POST /api/counter` applies the same rule.

`naming::id_value` rejects any id containing a hyphen. An interim build wrote
UUIDs, and reading the trailing digits of one would put the floor at 20044.

**Known limit:** `DataLock` is per data-directory, so two *different machines*
writing one shared base are not serialized and can mint the same number.
Same-machine concurrency is safe.

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

- `dialoguer::Input::interact_text()` takes ownership of `self`. Never reuse an
  `Input` across iterations — recreate it each time.
- Rust 2024 makes `std::env::set_var`/`remove_var` unsafe. In tests they are
  wrapped in `unsafe { }` with the suite's `SERIAL` mutex held.
- `clippy::field_reassign_with_default` is allowed at the test-file level;
  rewriting every `Config::default()` builder into struct-literal form is churn.
- `FASTF_FAULT`, `FASTF_TRACE_FILE` and the interrupt flag are process-global, so
  their test locks live beside the state they guard — a private mutex per test
  module looks right and silently races.
- Concurrency tests must spawn **processes**, not threads: a thread test passes
  against an in-process `Mutex` while production stays broken.
- **A Windows thread's stack is 1 MiB, not the main thread's 8.** The browser's
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
- Never put a backtick inside a JS template literal in `app.js`, including inside
  an HTML comment — it terminates the string and the error points at the
  following identifier. `node --check src/ui/web/app.js` catches it.
- `util::yaml` is the only module that names the YAML crate, and
  `the_emitted_bytes_are_the_ones_we_have_always_emitted` pins its output. These
  files are diffed, committed and hand-edited by users, so the bytes may not move.
