# PLAN.md — v2.0.0: two surfaces, one engine, nothing trusted by accident

How to use this file: one phase per session. Start with "read PLAN.md, do phase N,
verify, tick the boxes". Tick a box only when the named verification ran. Every
phase ends in its own PR (`phase-NN-<slug>`), with `ROADMAP.md` updated in the same
commit. Findings outside the phase go to the Parking lot, never fixed in passing.
Publication (version bump, tag, GitHub Release, AUR) happens only on an explicit
"release" — see the final section.

## Why this plan exists

An external review (2026-08-23) listed 13 areas. Each claim was verified against the
source before this plan was written; the table below records what was accepted,
changed or rejected, and why. The two product decisions behind the plan:

- **The browser UI is removed.** The maintainer does not use it, it is roughly a
  quarter of the repository, and it is the only network surface — an
  unauthenticated local HTTP server that could create folders and (as the
  verification found) silently run configured shell commands. Deleting it removes
  about 40% of the review's findings as a class rather than fixing them one by one.
  v1.7.1 stays available as the last release with `fastf ui`.
- **Feature freeze.** v2.0.0 is a hardening release. The `ROADMAP.md` backlog waits.

## Reviewer's list → decisions

| # | Reviewer item | Decision | Why |
|---|---|---|---|
| 1 | Reject invisible names; checked counter; lock message; atomic browser save; native Windows open | **Accept**, split over Phases 2 and 3 | All verified true. Browser save disappears with Phase 1; the same read-error collapse exists in `Template::save_to_file` and is fixed there. |
| 2 | Every template write through `operations` + `DataLock` | **Accept** (Phase 4) | 8 of 9 template writers bypass the lock today; after Phase 1 two remain (TUI builder save, CLI delete). |
| 3 | Symlink-safe destination helper | **Accept** (Phase 5) | No component-wise link check exists anywhere beneath a destination; apply and template ingestion are reachable. "No openat2 fortress" — agreed. |
| 4 | Post-create: env var, deprecate `{path}`, browser disclosure | **Modified** (Phase 3) | Env var + cwd: yes. `{path}` is **not** deprecated: it is rewritten to a quoted env-var reference, so existing configs keep working and the path never enters shell source. Browser parts vanish with Phase 1. `run` returns `Vec<Note>`. |
| 5 | Write the UI threat model | **Modified** (Phase 6) | With no UI there is no HTTP threat model. The trust model that remains (one account, trusted templates/config, caches never authorize paths outside a base) is written into `ROADMAP.md`'s product contract. |
| 6 | Validate cache entries; read-side revalidation | **Accept** (Phase 6) | `CacheEntry::into_project` performs zero validation; `fastf open` and the TUI spawn the file manager on a cache-derived path. |
| 7 | Test isolation: one env lock, RAII restore, sandboxed `DataLock` | **Accept** (Phase 7) | Two mutexes guard env mutation in one test binary; library unit tests lock the real data dir. Not accepted: `--test-threads=1`, nor a pure `Environment`-struct refactor (overkill). |
| 8 | Split `ui/mod.rs`, `app.js`, `project.rs`, `provisioning.rs`, `menu.rs`, `main.rs` | **Rejected** except what Phase 1 deletes | UI files are deleted. `main.rs` is 61% clap definitions, which is what a clap binary looks like. `project.rs`/`provisioning.rs` are guarded by crash-recovery tests and have not needed to change; a split would be churn with no defect behind it. Revisit only when a file has to change for two reasons at once. |
| 9 | Frontend safe-by-construction rendering | **Moot** | No frontend. (Verified: no unescaped user data existed; the discipline held.) |
| 10 | Node + headless browser tests | **Moot** | No frontend. |
| 11 | Comment archaeology, stale references, ADRs | **Modified** (Phase 9) | Stale references and duplicated blocks: yes (9 references to a nonexistent `tests/integration.rs`, 11 copies of one doc comment, a documented lock that does not exist). ADRs: **no** — `ROADMAP.md` rules them out; the `CLAUDE.md` files beside the code are the decision record, and the rule stays "a decision lives next to the code it constrains". |
| 12 | Release pipeline: gates before publish, least-privilege tokens, SHA pins, smoke tests, provenance, scheduled audit, pinned toolchain | **Accept** (Phase 8) | Verified: a tag can ship without a single test running; CI has no `permissions:` block; one action is pinned to a moving `@master`. Toolchain pin is release-workflow-only (no `rust-toolchain.toml`, so the AUR source build and contributors keep their own stable). |
| 13 | Reduce the product surface | **Accept, further than proposed** (Phase 1) | The review suggested narrowing the template editor; the maintainer chose to remove the browser UI entirely. |

Preserved untouched, as the review asked: `util::atomic`, the staged move and
recovery model, destructive-operation revalidation, the validated path types, the
real-process concurrency tests, fault injection, filesystem-as-truth, and the
`core`/surface layering.

## Gates — every phase, before its PR

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
  (needs `rustup target add x86_64-pc-windows-gnu` + mingw-w64-gcc; CI's Windows
  runner is the authority, this is the local early warning)
- `cargo test --all-targets` and `cargo test --release` (debug and release counts
  differ by design — failpoints and trace are compiled out of release)
- `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --locked`
- `cargo test --test repo_hygiene` (nothing tracked may describe a real machine)
- `ROADMAP.md` updated in the same commit; the matching `docs/` page when behaviour
  changed; the relevant `CLAUDE.md` only once the decision has landed.

---

## Phase 1 — Remove the browser UI

**Goal.** `fastf ui` no longer exists. The crate builds one binary, and nothing in
the tree — code, tests, docs, packaging, CI — mentions the browser UI except the
v2.0.0 release notes and the ROADMAP's release table.

**Evidence.** `src/ui/mod.rs` (2570 lines), `src/ui/assets.rs`, `src/ui/web/`
(`app.js` 3574, `styles.css` 2769, `index.html`, `icon.svg`), `src/bin/fastf-ui.rs`
(the windowless Windows launcher), `src/cli/ui.rs` (298), `tests/ui_{projects,
templates,jobs,security}.rs`, `tests/common/ui.rs`, `docs/UI.md`, `src/ui/CLAUDE.md`.
Outside those files the UI is referenced from: `src/lib.rs:9`, `src/cli/mod.rs:17`,
`src/main.rs` (the `Ui` variant near line 426 with its help examples at 421–424, and
the dispatch arm near 868), `tests/layering.rs:121` (`UPWARD` list) plus its doc
comments at 43 and 128, `tests/common/mod.rs:19`, `src/core/assets.rs:73` (doc link
to `ui::jobs_active`), `src/core/project.rs:520`, `src/cli/new.rs:212`,
`src/cli/render.rs:4`, `Cargo.toml` (`[[bin]] fastf-ui`, `description`, the
`panic = "abort"` comment in `[profile.release]`).

UI-only core code: `project::create_deferred` (`src/core/project.rs:304`), the
`defer_over` parameter threaded through `create_inner` / `provision_project` /
`copy_template_files` (lines ~320, 442, 784, 842), `CreateOptions::defer_over`
(`src/core/operations.rs:36`, used at 73–75), `assets::JOB_DEFER_BYTES`
(`src/core/assets.rs:31–38`). No test outside the deleted `tests/ui_jobs.rs`
exercises the deferred path (`tests/crash_recovery.rs` does not name it).

Packaging and CI: `packaging/fastf.desktop:6` (`Exec=fastf ui --app`),
`packaging/wix/main.wxs` (`UiBinPath`, `FastfUiExe` at 53, the Start Menu shortcut
target at 98), `.github/workflows/release.yml:101` and `:122` (ships
`fastf-ui.exe`), `.github/workflows/ci.yml:33–36` (`node --check`), both
`packaging/aur/*/PKGBUILD:11` and `.SRCINFO` (`optdepends=chromium`),
`packaging/aur/PUBLISHING.md:66`.

Docs: `README.md` lines 33, 37–42 (the hero image is a screenshot of the browser
UI), 51–52, 153, 158, 170, 205; `docs/cli.md:3,12`; `docs/templates.md:3`;
`docs/windows.md:8,23,29,94,96`; root `CLAUDE.md` lines 11, 34–35, 50–53, 84,
98–99, 310–312 ("three surfaces" becomes two); `ROADMAP.md:12,60,120`;
`tests/CLAUDE.md` (the `ui_*` suite paragraph and `common::ui`).

**Decisions.**
- Delete, do not feature-flag. Git history keeps it; a flag would keep every
  Windows CI leg and refactor paying for code nobody runs.
- Keep `assets::copy_job` / `CopyJob` / `Progress` and the deferred-resume branch in
  `provisioning::reconcile_create` (`src/core/provisioning.rs:538`): they *read*
  journals a v1.x binary may have left on a shared drive (dual boot). Only the
  *writer* side (`create_deferred`, `defer_over`, `JOB_DEFER_BYTES`) goes. Pruning
  the reader is a Parking-lot item for a later major.
- Keep `CreateOutcome::take_mutation_lock` — `src/cli/new.rs:102` uses it to drop
  the lock before post-create.
- Keep `util::size_scan` and `paths::MAX_WALK_DEPTH = 64`: the TUI browser runs the
  same worker threads (`src/tui/browser.rs:15`), so the 1 MiB Windows thread-stack
  rationale still holds.
- `packaging/fastf.desktop` → `Exec=fastf`, `Terminal=true` (the app-menu entry now
  opens the guided TUI in the user's terminal). Keep the icons.
- WiX: the Start Menu shortcut targets `fastf.exe` (console). Drop `UiBinPath`.
- `serde_json` stays a dependency (cache, transactions, provisioning, atomic).
- `[profile.release]` keeps `panic = "unwind"`; rewrite the comment (the TUI restores
  the terminal on unwind; nothing else needs stating).
- The version bump to 2.0.0 is **not** part of this phase — the `release` skill does
  it at publication. Update `Cargo.toml`'s `description` now.

**Steps.**
1. `git rm -r src/ui src/bin src/cli/ui.rs tests/ui_projects.rs tests/ui_templates.rs
   tests/ui_jobs.rs tests/ui_security.rs tests/common/ui.rs docs/UI.md`.
2. Remove the `Ui` command from `src/main.rs` (variant, help examples, dispatch arm);
   `pub mod ui` from `src/lib.rs` and `src/cli/mod.rs`; `mod ui` from
   `tests/common/mod.rs`; `crate::ui` from `tests/layering.rs`'s `UPWARD` list and
   reword its two doc comments.
3. Remove the deferred-create writer side listed above; let the compiler find the
   rest. `copy_template_files` loses its `defer_over` branch; `create_inner` and
   `provision_project` lose the parameter.
4. `Cargo.toml`: drop the `[[bin]] fastf-ui` block and its comment; new
   `description` ("template-driven project folder generator with a guided TUI and
   CLI"); rewrite the `[profile.release]` comment.
5. Packaging and CI edits listed under Evidence. In `release.yml` the Windows staging
   step copies only `fastf.exe`; the WiX invocation drops `-d UiBinPath`. In `ci.yml`
   delete the `node --check` step and its comment. PKGBUILDs lose `optdepends`
   (the AUR push itself happens at release time).
6. Docs: rewrite the README hero block for CLI + TUI (remove the browser screenshot
   and its paragraph — **the maintainer supplies a TUI screenshot or asciinema
   later**; leave no broken image); fix every listed line in `docs/*.md`; root
   `CLAUDE.md` becomes a two-surface document (drop the backtick-in-template-literal
   trap, the `FASTF_UI_DIR` line, the `fastf-ui` bullet, the four-files paragraph
   becomes three); `tests/CLAUDE.md` drops the `ui_*` paragraph; `ROADMAP.md`
   removes the `node --check` gate and the browser backlog bullets, and records the
   removal in the "Current phase" block.
7. `git grep -n -iE 'fastf ui|fastf-ui|src/ui|app\.js|UI\.md|FASTF_UI_DIR|WRITE_LOCK|route_request'`
   must return only `ROADMAP.md`'s release table and this file.

**Acceptance.**
- [x] All gates pass; `cargo test --all-targets` runs no `ui_*` binary.
- [~] `cargo build --release --target x86_64-pc-windows-gnu` produces exactly one exe.
      **Not run locally**: this machine has no `mingw-w64-gcc`, so the cross-link
      fails in `getrandom`/`windows-sys` before reaching fastf. What did run:
      `cargo clippy --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
      (clean) and `cargo metadata`, which reports exactly one `bin` target.
      CI's Windows runner is the authority.
- [x] The `git grep` in step 7 is clean.
- [x] `fastf --help` lists no `ui`; `fastf ui` is "unrecognized subcommand".
- [x] `tui_pty` (34 tests: menu, project browser, flows, template builder) passes
      unchanged — nothing in the TUI depended on `src/ui`.
- [x] ROADMAP, README, docs, CLAUDE.md files updated in the same PR.

**Notes for the next phase.** After this phase the only in-process mutation lock is
gone with `WRITE_LOCK`; the remaining concurrency is cross-process (`DataLock`) plus
the TUI's size-scan worker threads, which only read.

---

## Phase 2 — Names and numbers

**Goal.** One validator decides what a project folder may be called, and create,
rename and register all use it. The ID counter has a maximum and cannot overflow.
The lock timeout tells the truth. A template manifest is never replaced because it
could not be read.

**Evidence.**
- Create's only name check is `sanitize_name` at `src/core/project.rs:241`;
  `sanitize_name` (`src/core/naming.rs:265–287`) trims trailing dots/spaces and
  maps illegal characters but leaves a leading `.` and reduces `..` to `""`,
  documenting that "callers such as `rename_project_inner` already reject" the
  empty result — create is not one of those callers. The rule lives only in
  `rename_project_inner` (`src/core/library/lifecycle.rs:144–152`); register's
  `--rename` (`src/core/operations.rs:388–392`) checks empty but not dot.
- Discovery skips dot-prefixed directories (`src/core/library/discovery.rs:119–133`),
  so `--name=.hidden` with the gallery `{name}` patterns creates an invisible
  project — visible until the next rescan because `cache_upsert` records it, then
  gone. `--name=..` renders to `""`; with `on_name_collision = Suffix` the plan loop
  (`project.rs:258–262`) sees `base.join("")` "exists" and names the folder `_2`;
  if the base does not exist yet, `create_inner` (`project.rs:327–334, 359–372`)
  resolves `root_path.parent()` to the base's **parent** and creates `_2` there.
- `Template::validate` (`src/core/template.rs:343`) only requires a non-empty
  `naming_pattern`; nothing validates `id.prefix` / `id.digits` in core. The TUI
  builder allows an empty prefix and digits 1..=9 (`src/tui/template_builder.rs:
  228–232, 253–274`).
- `Counters::next_value` (`src/core/counter.rs:165–167`) is `… + 1`, unchecked;
  `operations::set_counter` (`operations.rs:583–595`) accepts any value above the
  floor, including `u64::MAX`, so the next create overflows (panic in debug, wrap
  to 0 in release). `format_id` (`counter.rs:244`) is the one formatter.
- `Counters::propagate` calls `Counters::load().unwrap_or_default()`
  (`counter.rs:126`), swallowing a parse/IO error of the data-dir counter file —
  the ROADMAP's v1.7.1 audit item.
- Lock timeout (`src/util/lockfile.rs:76–83`) says "delete the lock file and
  retry". On Unix the lock is `flock` on the inode (line 143): deleting the file
  does not release it, and the next process locks a *new* inode — two processes
  then both hold "the" lock, which is the duplicate-ID bug the module exists to
  prevent. The module's own doc (lines 37–39) says there is no stale lock.
- `Template::save_to_file` (`src/core/template.rs:296–306`) matches
  `Err(_)` on reading the existing manifest and writes a fresh one — EACCES, EIO
  and non-UTF-8 all become "new template", discarding the unknown-key preservation
  that `src/core/CLAUDE.md` calls load-bearing. Reached from the TUI builder
  (`template_builder.rs:96`) and `template_import` (`template_import.rs:213`).
- Tests: `tests/properties.rs:64–90` `prop_assume!(!safe.is_empty())` with the
  comment "callers reject that explicitly" — false for `plan`. The only dot test
  is a rename test (`src/core/library/tests.rs:1051–1055`).

**Decisions.**
- `validated::ProjectFolderName::parse(raw: &str) -> Result<Self>`: trim,
  `sanitize_name`, then reject empty, `.`-prefixed, and anything that is not a
  single path component. Error messages name what was rejected and why
  ("a folder name may not start with '.': fastf would not see the project").
  `plan` reports the rendered value and the template's pattern.
- `Template::validate` additionally rejects a `naming_pattern` whose first
  character is `.` (catches `.{id}` at save time, before any project exists),
  requires `id.prefix` non-empty, and requires `1 <= id.digits <= 12`. Prefix is
  required because `naming::parse_id_token(name, prefix)` (register's ID recovery)
  would match any trailing digits with an empty prefix — `Album_2024` would
  register as ID 2024. The TUI builder's prompts adopt the same bounds
  (`TextOpts::validate`, so the rejection happens at the prompt).
- Defense in depth in `create_inner`: bail if `plan.root_path.parent()` is not the
  plan's base, before `create_dir_all` — a wrong root must never reach the claim.
- `Counters::MAX_VALUE = 999_999_999_999` (twelve digits: every allowed `digits`
  renders it without widening, and it is below 2^53 so any JSON consumer reads it
  exactly). `next_value` returns `Result<u64>` using `checked_add` and the maximum;
  `set_counter` refuses values above `MAX_VALUE` with the same message. Callers are
  found by the compiler (`project::plan`, `operations::register`, `cli::id`).
- `propagate`: replace `unwrap_or_default()` with a match that `diag::warn`s and
  skips the data-dir write on error, so a corrupt `counters.toml` is reported, not
  silently treated as zero.
- Lock message: "another fastf process is busy (waited 30s for <path>). It still
  holds the data lock — close it or wait, then retry. Deleting the lock file does
  not help: the lock belongs to the process, not the file." No PID in the file:
  Windows `share_mode(0)` makes it unreadable to the waiter anyway, and the OS
  releases the lock, so there is nothing a PID would let the user act on.
- `save_to_file`: `Err(e) if e.kind() == NotFound` → fresh manifest; any other
  error propagates with context ("refusing to replace an unreadable manifest").

**Steps.**
1. Add `ProjectFolderName` to `src/core/validated.rs` with unit tests (`.hidden`,
   `..`, `.`, `""`, `"  "`, `a/b`, `a\\b`, trailing dots, a valid name).
2. Use it in `project::plan`, `rename_project_inner`, `operations::register`'s
   rename branch. Delete the duplicated rule from `lifecycle.rs`.
3. `Template::validate` additions + TUI builder prompt bounds + `docs/templates.md`
   (state the bounds next to the `id:` block).
4. `create_inner` parent check.
5. Counter: `MAX_VALUE`, checked `next_value`, `set_counter` ceiling, `propagate`
   warning. `docs/cli.md` `id set` row states the maximum.
6. Lock message. Unit test in `lockfile.rs`: `acquire_at` twice with a tiny timeout;
   assert the message does not contain "delete" and does contain "holds".
7. `save_to_file` read-error split. Test: plant a *directory* at
   `templates/<slug>/template.yaml` (errors with non-NotFound on every platform);
   save must fail and the directory must still be there.
8. Tests: in `tests/create.rs` — gallery-style `{name}` template with
   `--name=.hidden`, `--name=..`, `--name=` of only illegal characters; each fails
   before any directory is created (assert the base is empty after) and the counter
   file is unchanged. A template whose pattern is `.{id}` is refused by
   `template new`/save. `tests/properties.rs`: replace the `prop_assume!` with a
   property that `ProjectFolderName::parse` accepts exactly the non-empty,
   non-dot-prefixed `sanitize_name` outputs. `tests/cli_counter.rs`: `fastf id set
   999999999999` succeeds, `1000000000000` is refused, and `fastf new` at the
   maximum fails with the maximum named and no folder created.

**Acceptance.**
- [x] All gates pass (Windows clippy included; the cross-*link* still cannot run
      here — see Phase 1).
- [x] `grep -rn '+ 1' src/core/counter.rs` matches only the comment explaining
      the overflow it replaced; `grep -rn "delete the lock" src/` matches only
      the test that asserts the message no longer says it.
- [x] Every new test was run against the pre-change build and observed failing —
      see the Phase log for what each one did there.
- [x] `docs/templates.md`, `docs/cli.md`, `ROADMAP.md` updated; `src/core/CLAUDE.md`
      "Create, apply, register" names `ProjectFolderName` as the one validator.

---

## Phase 3 — Shelling out

**Goal.** A project path never appears inside shell source. Opening a folder on
Windows passes the path as data. `post_create::run` has the type it already has in
practice.

**Evidence.**
- `src/core/post_create.rs:116`: `raw.replace("{path}", &project_path.display()
  .to_string())`, then `run_shell` → `sh -c` (lines 192–199) or `cmd /c`
  (184–190) with `current_dir` set to the project. `sanitize_name` leaves `; & $ ( )
  \` ^ %` in folder names, so a name with a `;` splits the command.
- `reveal_folder` on Windows (`post_create.rs:139–146`): `cmd /c start "" <path>`.
  std quotes the argument, but `cmd.exe` still expands `%VAR%` inside the
  reconstructed line — a folder named `%USERPROFILE%` opens the wrong directory.
  Callers: `src/cli/new.rs:217`, `src/cli/recent.rs:170` (`fastf open`),
  `src/tui/actions.rs:151`, `post_create.rs:98`.
- `spawn_editor` on Windows (`post_create.rs:161–170`) goes through `cmd /c` too
  (needed: `code` is a `.cmd` shim).
- `run` returns `Result<Vec<Note>>` (`post_create.rs:75`) and contains no `?` and no
  `Err` — it cannot fail. `project::run_post_create` (`project.rs:573–585`) has a
  dead `Err` arm.
- `docs/templates.md:151–164` documents `{path}` and the trust warning.

**Decisions.**
- Every child fastf spawns for a project (git init, editor, commands) gets
  `FASTF_PROJECT_PATH=<path>` in its environment and the project as `current_dir`.
- `{path}` is kept and rewritten: on Unix to `"$FASTF_PROJECT_PATH"`, on Windows to
  `"%FASTF_PROJECT_PATH%"`. If the token is already wrapped in a matching pair of
  `"` or `'`, the wrapped token is replaced as a unit so `code "{path}"` does not
  double-quote. Windows paths cannot contain `"`, so the quoted expansion is safe
  for every legal path; `sh` never re-parses a double-quoted variable. No
  deprecation warning — there is nothing to migrate.
- Editor on Windows stays on `cmd /c` for shim resolution, but the path argument
  becomes `"%FASTF_PROJECT_PATH%"`. Unix: `Command::new(editor).arg(path)` is
  already data; only the env var is added.
- Reveal on Windows: `ShellExecuteW(NULL, "open", path, NULL, NULL, SW_SHOWNORMAL)`
  declared with a hand-written `unsafe extern "system"` block + `#[link(name =
  "shell32")]` in a new `cfg(windows)` `util::shell_open` module — the same pattern
  the removed `fastf-ui.rs` used for `MessageBoxW`, so no `windows-sys` dependency.
  A return value ≤ 32 is an error, reported with the code. It honours the user's
  default folder handler exactly as `start` did. No COM initialisation is required
  for opening a folder; note that in the module doc.
- `post_create::run -> Vec<Note>`; delete the dead `Err` arm in `run_post_create`.
- `docs/templates.md`: recommend `.` or `$FASTF_PROJECT_PATH` in new commands,
  explain that `{path}` expands to the quoted variable, keep the trust warning.

**Steps.**
1. `util::shell_open` (Windows) + switch `reveal_folder`'s Windows arm to it. Unix
   arms unchanged. Unit-test the UTF-16 conversion (nul-terminated, no interior nul
   → error).
2. `post_create`: env var + cwd on every child; token rewrite as a pure function
   `rewrite_path_token(raw: &str) -> String` with unit tests (`{path}`, `"{path}"`,
   `'{path}'`, two tokens, none, token at start/end).
3. Return type change and the dead arm.
4. Tests in `tests/create.rs` (unix-gated where the shell differs): a post-create
   command `test -d {path} && touch "$FASTF_PROJECT_PATH/ok"` run against a project
   whose name contains `;` and `$(` (sanitized as today) — `ok` must exist; under the
   current build the `;` splits the command and the test fails. A command `pwd >
   cwd.txt` proves `current_dir`. Windows CI variant with `&` and `%` in the name.
5. `src/core/CLAUDE.md` "Post-create actions" rewritten to the new contract.

**Acceptance.**
- [x] All gates pass. The `extern "system"` block is linted by local
      `--target x86_64-pc-windows-gnu` clippy and by CI's Windows runner.
- [x] `grep -rn 'Command::new("cmd")' src/` finds only three test helpers
      (`transactions.rs`, `assets.rs`, `library/tests.rs`, all inside
      `mod tests`). No `start` outside the editor shim — which the Decisions
      above deliberately keep on `cmd /c` for `.cmd` shim resolution, with the
      path passed as the quoted variable.
- [x] The injection test was observed failing on the pre-change build: with the
      old `raw.replace("{path}", …)` restored it reported `'pwned' was created
      inside the project — the name was executed`.
- [ ] **Needs the maintainer:** a Windows smoke of "Reveal" from the TUI action
      menu and of `fastf open` (recorded in `ROADMAP.md`).

---

## Phase 4 — Template writes behind the lock

**Goal.** Every mutation of the templates directory goes through
`core::operations` and holds `DataLock`, as the documentation already claims.

**Evidence.** After Phase 1 the writers are: the TUI builder's
`tmpl.save_to_file(&dest)` (`src/tui/template_builder.rs:96`), CLI `template
delete`'s `fs::remove_dir_all(&dir)` (`src/cli/template.rs:214`, after a confirm),
and `operations::template_from_folder` (`src/core/operations.rs:604–612`, already
locked). `bootstrap.rs:117–145` writes the two bundled templates on first run,
guarded by "templates dir is empty". `src/core/CLAUDE.md` "Locking and mutation":
"Prompt first, then lock, then reload."

**Decisions.**
- `operations::save_template(template: &Template, original_slug: Option<&str>)
  -> Result<PathBuf>`: acquire `DataLock`; `Template::validate`; `TemplateSlug` on
  both slugs; if `original_slug` is `Some` and differs, `fs::rename` the template
  directory first (refuse if the target slug exists); then `save_to_file`. Returns
  the manifest path.
- `operations::delete_template(slug: &str) -> Result<()>`: acquire `DataLock`;
  `TemplateSlug`; the directory must be a real directory (not a link) directly
  under the templates dir — reuse `paths::require_real_directory` once Phase 5
  moves it, or `assets::require_real_directory` until then; `remove_dir_all` through
  `fs_retry`.
- The builder keeps collecting every answer before calling the operation (no prompt
  under the lock). The CLI confirm stays where it is, before the call.
- Bootstrap stays unlocked: it runs before the data dir has a lock file to take and
  only writes into an empty directory. Record that exception in `src/core/CLAUDE.md`.

**Steps.**
1. Add the two operations with unit tests (rename to an existing slug refused; a
   symlinked template dir refused for delete).
2. Switch the builder and the CLI to them; `Template::save_to_file` becomes
   `pub(crate)`.
3. Source-scan test in `tests/layering.rs` (same style as the prompt rule): under
   `src/cli` and `src/tui`, `save_to_file(` and `remove_dir_all(` may not appear.
4. Process-level test in `tests/concurrency.rs`: one process holds the lock (an
   armed `FASTF_FAULT` that sleeps, or the existing long-running create pattern in
   that file) while `fastf template delete --yes` runs; the delete must wait, not
   interleave (assert on ordering via timestamps written by each side).
5. `docs/templates.md` "Where templates live" gains one sentence: template edits
   and deletes wait for any running fastf operation.

**Acceptance.**
- [x] All gates pass.
- [x] `grep -rn 'save_to_file\|remove_dir_all' src/cli src/tui` is empty, and
      `tests/layering.rs` now fails the build if either reappears.
- [x] The concurrency test passes and was observed interleaving on the old
      build: with the direct `remove_dir_all` restored it reported "the delete
      ran to completion while the data lock was held".
- [x] `src/core/CLAUDE.md` lists the new operations and the bootstrap exception.

---

## Phase 5 — Destination containment

**Goal.** No write fastf performs beneath a root it controls follows an existing
symlink, junction or reparse point. Lexical safety (`SafeRelativePath`) plus a
physical check immediately before the write.

**Evidence.** `SafeRelativePath` (`src/core/validated.rs:51–98`) and
`paths::require_native_relative` (`src/util/paths.rs:238–252`) are lexical. The
existing real-path helpers check one named path only: `assets::require_real_directory`
(`src/core/assets.rs:375–383`, used for the apply target root at `project.rs:684`),
`paths::require_real_file` (`paths.rs:202–209`), `assets::entry_exists`
(`assets.rs:387–394`, the leaf). The writes themselves: `assets::copy_file`
(`assets.rs:499–531`) does `create_dir_all(dest.parent())` then the atomic write —
`create_dir_all` walks through an existing `docs -> /outside` link;
`project::apply` (`project.rs:701`) and `create_structure` (`project.rs:715–732`)
do the same; `template_import` writes at `template_import.rs:218`; `copy_job`
(`assets.rs:174–187`) guards only the leaf. The staged move already has the
deny-by-default link detection this phase needs on Windows
(`MoveManifest::scan`, `src/core/transactions.rs:229–235`; junction coverage in
`tests/windows_semantics.rs:265–305`). `tests/hostile_fs.rs` has no symlink case.

**Decisions.**
- Move `require_real_directory` from `assets` to `util::paths` beside
  `require_real_file` (util may not import core). Add
  `paths::contained_destination(root: &Path, rel: &Path) -> Result<PathBuf>`:
  `root` must be a real directory; for every existing prefix of `root/rel`,
  `symlink_metadata` must be a directory and not a link (use the same link test
  `MoveManifest::scan` uses so junctions are caught on Windows); the final
  component, if it exists, must not be a link. Returns the joined path. Checks
  happen immediately before the write — that is the stated single-user threat
  model; no `openat2`.
- Use it at every write site above: `copy_file` (and therefore template
  ingestion and `copy_job`), `apply`'s structure loop, `create_structure`,
  `copy_template_files`. The create root is freshly claimed with `create_dir`, so
  the check there costs one `symlink_metadata` per component and closes the
  remaining race.
- The error names the link: "refusing to write through a link: <path> -> <target>".

**Steps.**
1. Helper + unit tests (root is a link; link mid-path; link at leaf; plain tree
   passes; non-existent tail passes).
2. Wire the call sites; run the full suite — `create.rs`, `template_engine.rs`,
   `register.rs` (apply) cover the happy paths.
3. `tests/hostile_fs.rs`: `target/docs -> outside/` then `fastf apply` with a
   template holding `docs/new.md` → refused, `outside/` untouched, message names
   `docs`. `template from-folder --force` where `templates/<slug>/files/sub` is a
   pre-planted link → refused before any byte is written. Unix symlinks;
   `tests/windows_semantics.rs` gets the junction variant.
4. `ROADMAP.md` product contract gains the guarantee in one sentence;
   `src/core/CLAUDE.md` "Path safety" describes lexical + physical as two layers.

**Acceptance.**
- [x] All gates pass; the junction test is `cfg(windows)` and lints clean under
      the `x86_64-pc-windows-gnu` target. CI's Windows runner runs it.
- [x] The apply-through-link test was observed failing on the pre-change build
      (`apply must refuse to write through the link: ()` — it returned `Ok`).
- [x] `grep -rn 'create_dir_all' src/core/assets.rs src/core/project.rs`: every
      production call is preceded by `contained_destination`, except two that are
      correct as they are — `create_inner`'s base creation (the base itself, just
      checked to be the configured one; a base that is a link to a mounted drive
      is legitimate) and `copy_job`'s parent (its caller derives the destination
      through the helper, noted in its doc comment).

---

## Phase 6 — Cache entries are hints, and `open` checks before it spawns

**Goal.** A `.fastf-index.json` entry can never name a path outside its base, and
the two places that hand a discovered path to another program check it first. The
trust model fastf actually has is written down.

**Evidence.** `CacheEntry::into_project` (`src/core/library/cache.rs:59–73`) joins
`dir` onto the base with no validation: an absolute `dir` *replaces* the base
(`Path::join` semantics), `../..` survives `strip_prefix` on the next rewrite. The
fast path (`discovery.rs:44–62`, and `resolve.rs:88–98` for `max_id`) checks only
`is_dir()`. Overwriting the cache file in place does not bump the base directory's
mtime, so a planted cache reads as fresh (`cache_is_stale`, `discovery.rs:71–79`).
`guard.rs:50–94` revalidates for **mutations** only (canonicalize, direct child of a
configured base, real `PROJECT_INFO.md`, id match). Read-side consumers with no
check: `fastf open` (`src/cli/recent.rs:148–170` → `reveal_folder`), the TUI action
menu's reveal (`src/tui/actions.rs:151`), `fastf recent`/`search`, and every
metadata read off `project.path`. Caches travel with the projects by design
(`cache.rs:1–6`) — a synced folder or an unpacked archive is the delivery vector.
`tests/hostile_fs.rs:65–89` covers only malformed JSON. Write paths are safe
already (`operations::delete` etc. revalidate).

**Decisions.**
- `into_project` returns `Option<Project>`: `dir` must be a `SafeRelativePath`
  with exactly one component (discovery is depth-1, `SCAN_DEPTH`) and not
  dot-prefixed; anything else drops the entry and sets the `dropped` flag that
  already triggers a rescan. Do **not** add `base` to `CacheEntry`.
- `library::revalidate_for_read(project: &Project) -> Result<()>`: `symlink_metadata`
  of `project.path` is a real directory, `project.path.parent() == Some(&project.base)`,
  and `pinfo_path(&project.path)` exists. No canonicalize, no config reload, no id
  check — the cheap sibling of `guard::revalidate_project`. Called by `fastf open`
  and the TUI reveal before spawning. Metadata reads keep trusting discovery: after
  the `into_project` rule the path is a direct child of the base by construction,
  and reading the user's own `PROJECT_INFO.md` is what discovery is.
- Trust model, written into `ROADMAP.md`'s "Product contract" (and a short
  user-facing paragraph in `docs/projects.md`): one OS account; bases, templates,
  `config.toml`, counters and caches are the user's own files and are trusted as
  content, with two exceptions fastf enforces — a cache entry can only ever point at
  a direct child of its own base, and a write never follows a link (Phase 5);
  post-create commands and templates are executable configuration; there is no
  network surface.

**Steps.**
1. `into_project` validation + unit tests in `src/core/library/tests.rs`
   (`/etc`, `../../x`, `D:/x`, `.hidden`, `a/b`, `""`, and a valid name).
2. `revalidate_for_read` + wiring in `cli::recent::open` and `tui::actions`.
3. `tests/hostile_fs.rs`: plant a fresh cache (write it **after** the base mtime so
   the staleness gate trusts it) with the hostile `dir` values; `fastf recent` lists
   none of them; `fastf open <id>` for a forged id fails without spawning (assert on
   the error, and on a sentinel file outside the base being untouched). A base whose
   project directory is replaced by a symlink to elsewhere: `fastf open` refuses.
4. ROADMAP + `docs/projects.md` text.

**Acceptance.**
- [x] All gates pass.
- [x] The forged-cache test was observed listing `/etc` on the old build —
      `forged entries were served as projects: ["/etc", ".../base/..",
      ".../install/outside"]`.
- [x] The one-component rule is stated in the **root** `CLAUDE.md`'s "Filesystem
      as truth", which is where the cache and discovery model lives (the plan
      named `src/core/CLAUDE.md`; that file covers the engine, not discovery).

---

## Phase 7 — Test isolation

**Goal.** Inside one test binary, exactly one lock guards process environment
mutation; every guard restores what it changed even when the test panics; no unit
test can touch the developer's real data directory.

**Evidence.** The lib's unit-test binary has two independent locks around
`set_var`: `trace::tests::TEST_LOCK` (`src/util/trace.rs:66`, guards
`FASTF_TRACE_FILE` at 77–101) and `interrupt::TEST_LOCK` borrowed as `SERIAL` by
`src/core/project.rs:870` (guards `FASTF_INSTALL_DIR` at 905–985). `setenv` is not
thread-safe at the libc level, so these race each other and every `env::var` in the
binary. `src/core/library/tests.rs:313, 1013, 1018` call `move_project` with no env
set; `move_engine.rs:49` takes `DataLock::acquire()`, whose path is
`paths::install_dir().join(".fastf.lock")` (`lockfile.rs:95–97`) — the developer's
real data dir, created if missing (`lockfile.rs:54–59`). A `cargo test` therefore
blocks a concurrently running `fastf` for up to the 30 s timeout.
`tests/common/env.rs` restores env after `body()` returns — a panic skips it and the
next test in the binary sees a deleted tempdir as `HOME`. `grep "impl Drop"` finds no
env guard. No `--test-threads=1` anywhere (correct; keep it that way).
`tests/CLAUDE.md:82–84` documents `faults::TEST_LOCK`, which does not exist
(`faults.rs:41` is a thread-local).

**Decisions.**
- New `src/util/test_env.rs`, `#[cfg(test)]`, `pub(crate)`: one
  `pub static ENV_LOCK: Mutex<()>` and an RAII `EnvGuard` that takes the lock,
  records prior values, sets the requested variables, and restores them in `Drop`.
  `EnvGuard::sandbox() -> (EnvGuard, TempDir)` sets `FASTF_INSTALL_DIR` and
  `HOME`/`USERPROFILE` to a fresh tempdir. Lock order when a test also needs
  `interrupt::TEST_LOCK`: `ENV_LOCK` first; documented in both modules.
- Convert `trace` tests, `project` tests, and every `library/tests.rs` test that
  reaches `DataLock` (at minimum the three `move_project` calls) to `EnvGuard`.
  Delete `trace::tests::TEST_LOCK`.
- `tests/common/env.rs`: rebuild `with_fresh_install` / `with_sandbox` on a
  Drop-restoring guard (restores `FASTF_INSTALL_DIR`, `HOME`, `FASTF_FAULT`). Keep
  one `SERIAL` per integration binary — correct and sufficient. Replace the eleven
  copies of the `SERIAL` doc comment with one line pointing at `common::env`.
- Not doing: a pure `Environment` struct threaded through `paths` — the env is read
  in ~30 places and the guard gives the same isolation for a fraction of the churn.

**Steps.**
1. `test_env.rs` + conversions; a test that asserts `lockfile::lock_path()` is under
   the guard's tempdir while the guard is held.
2. Source-scan test (in `tests/layering.rs`): every `set_var(`/`remove_var(` under
   `src/` is in `util/test_env.rs`; under `tests/`, in `common/env.rs` or
   `data_dir.rs`'s `with_user_dir_env` (which must use the guard too).
3. `tests/common/env.rs` rewrite; a deliberate panicking test proves restoration
   (catch_unwind around `with_fresh_install`, then assert `HOME` is back).
4. `tests/CLAUDE.md`: fix the `faults::TEST_LOCK` sentence, describe the guard.
5. Run `cargo test` ten times in a loop (`for i in $(seq 10); do cargo test
   --all-targets -q || break; done`) — the race was intermittent.

**Acceptance.**
- [x] All gates pass; the ten-run loop is clean (ten consecutive
      `cargo test --all-targets -q`, no failures).
- [x] `cargo test` leaves no `.fastf.lock` in the real data dir — deleted before
      the loop, still absent after all ten runs. (The "TUI open in another
      terminal" half is the same property observed from the other side; the
      lock file is the check that does not need a human.)
- [x] `grep -rn 'TEST_LOCK' src/` shows only `interrupt::TEST_LOCK` (its
      definition, and the two modules that take it).

---

## Phase 8 — Release pipeline

**Goal.** A tag cannot publish what CI would reject; the publishing token is the
only one with write access; every third-party action is pinned; the artifacts that
are published are the ones that were smoke-tested; provenance is attested.

**Evidence.** `.github/workflows/release.yml`: `build` runs only `cargo build
--release` (73–75); `verify-version` (19–39) compares the tag to `Cargo.toml` and
nothing else; top-level `permissions: contents: write` (10–11) applies to every job;
no `concurrency:`; the `release` job (138–155) checksums the downloaded artifacts and
unpacks none. `ci.yml` has no `permissions:` block and does not fire on tags (3–7);
the audit job (88–96) runs only on push/PR — a new advisory against an unchanged
lockfile never surfaces. Every `uses:` is a floating tag; `dtolnay/rust-toolchain@
master` at `ci.yml:81` is a moving branch. WiX is the one thing pinned (5.0.2).
`ROADMAP.md` enforces "tests before release" as a manual checklist.

**Decisions.**
- `ci.yml`: add `workflow_call`; top-level `permissions: contents: read`; add
  `schedule: cron: "17 6 * * 1"` (weekly) — the audit job runs on it, the rest skip
  on schedule via `if`.
- `release.yml`: top-level `permissions: contents: read`; new `gates` job
  `uses: ./.github/workflows/ci.yml`; `build` needs `[verify-version, gates]`;
  `verify-version` additionally fetches `main` and requires
  `git merge-base --is-ancestor "$GITHUB_SHA" origin/main`; `concurrency:
  release-${{ github.ref }}`; the `release` job alone gets `contents: write`,
  `id-token: write`, `attestations: write` and runs `actions/attest-build-provenance`
  over the assets; smoke: the Linux archives are unpacked on ubuntu and the ZIP on
  windows, each running `fastf --version` (must equal the tag) and `fastf completions
  bash`; the MSI is extracted with `msiexec /a <msi> /qn TARGETDIR=<dir>` and the
  extracted exe runs `--version`.
- Pin every `uses:` to a full commit SHA with a `# vX.Y.Z` comment; add
  `.github/dependabot.yml` for the `github-actions` ecosystem so pins get bumped by
  PR. The release toolchain is pinned in `release.yml` only (`toolchain:
  <version>` on the pinned `dtolnay/rust-toolchain`), not a `rust-toolchain.toml`
  — the AUR source package and contributors keep their own stable, the MSRV job
  still derives from `Cargo.toml`.
- Update `.claude/skills/release/SKILL.md` (untracked, local) and
  `packaging/aur/PUBLISHING.md`; `ROADMAP.md`'s gate list drops the manual
  "run the suite before tagging" item because the pipeline does it.

**Steps.** Edit the two workflows and add dependabot; run the existing
`workflow_dispatch` dry run of `release.yml` (it builds without publishing) and a
push to a branch to exercise `ci.yml` via PR; inspect that `gates` ran and the smoke
steps printed the version.

**Acceptance.**
- [x] The dry-run release
      ([run 32645482828](https://github.com/cristocola/fast-folder/actions/runs/32645482828))
      is green end to end: `gates` (all eight CI jobs) → `build` (three targets)
      → `smoke` (three jobs) → `publish release: skipped`. Both Windows smoke
      steps printed `fastf 2.0.0` — the ZIP's exe and the MSI payload's.
- [x] `grep -n 'uses:' .github/workflows/*.yml` shows only SHA pins. The one
      exception is `uses: ./.github/workflows/ci.yml`, the repository's own
      reusable workflow, which is a path and not a pinnable ref.
- [x] `ROADMAP.md` gates and `PUBLISHING.md` updated, plus the `release` skill.

---

## Phase 9 — Documentation and comments

**Goal.** A comment says why the code must behave as it does; history lives in git;
nothing in the tree points at a file that does not exist.

**Evidence.** Nine references to a nonexistent `tests/integration.rs` (`src/lib.rs:1`
is a live claim; seven "split out of" headers; `tests/CLAUDE.md:23`); eight to
`ui_server.rs` and three to `cli_surface.rs` (`tests/CLAUDE.md:72` and
`tests/common/mod.rs:6` are live claims). `tests/CLAUDE.md:82–84` names a lock that
does not exist (fixed in Phase 7). Eleven identical copies of the `SERIAL` doc
comment already forked into two wordings (Phase 7 removes them); three copies each
of the pty and CLI suite preambles. 35 version-number comments and 5 `Phase N`
comments in `src/`+`tests/` that point at commits, not at anything in the tree.
Line numbers in `ROADMAP.md:128,140` and a crate version in
`src/core/CLAUDE.md:137` (already drifted: the lockfile has serde 1.0.228).
`ROADMAP.md:83` rules out ADRs — the `CLAUDE.md` files are the decision record.

**Decisions.**
- No ADR directory. Each `CLAUDE.md` is condensed to invariants, boundaries and
  commands; release-by-release prefixes go; a decision stays next to the code it
  constrains.
- Version/phase references in code comments: rewrite as the invariant ("an
  unconfigured `base_dir` falls back to the home directory") or delete; keep those
  that name a *format* (`pre-v2 markers`, `pre-v0.8 flat templates`).
- Duplicated preambles: one copy in the module that owns the rule, one line
  elsewhere.
- Line numbers in long-lived docs → function names.
- README: rewritten for CLI + TUI as the two surfaces (Phase 1 did the removal; this
  phase does the prose). **The maintainer supplies the hero screenshot/asciinema of
  the TUI.**
- `ROADMAP.md`: v2.0.0 row in the release table, "Current phase" block, backlog
  without browser items, the v1.7.1 audit findings that this plan fixed removed.

**Steps.** Work file by file from the evidence list; `git grep -n -E
'integration\.rs|ui_server|cli_surface|faults::TEST_LOCK|v[01]\.[0-9]+(\.[0-9]+)?:|Phase [0-9]+'`
must be clean for `src/ tests/ docs/ *.md` except format names and the ROADMAP
release table.

**Acceptance.**
- [x] All gates pass (docs build included).
- [x] The `git grep` above is clean for `src/ tests/ docs/`; the only remaining
      matches anywhere are `ROADMAP.md`'s release table and this file, both
      named as exceptions.
- [x] Each `CLAUDE.md` read top to bottom; anything describing a past version
      rather than a present rule was removed, and three live errors were found
      that way — `tree_size::directory_size` no longer exists (Phase 1 removed
      it), `tests/CLAUDE.md` stated the harness rules twice after Phase 7, and
      the root module list was missing `shell_open`, `test_env` and the new
      `paths` helpers.

---

## Release — v2.0.0

**All nine phases are complete.** `Cargo.toml` is at `2.0.0` so the built binary
reports the release it is; **no tag has been pushed and no release cut** —
publication is the maintainer's explicit call, and `release.yml` refuses a tag
that does not match this number anyway.

Only on an explicit "release" from the maintainer; follow the `release` skill.
Release notes must say: the browser UI (`fastf ui`, `fastf-ui.exe`, the
`--app` window) is removed and v1.7.1 is the last release that has it; project
names that start with `.` or render empty are refused; the ID counter has a
maximum; `{path}` in post-create commands now expands to a quoted environment
variable and `FASTF_PROJECT_PATH` is available; the lock timeout message changed;
fastf refuses to write through links; cache entries outside a base are ignored.

**Needs the maintainer** (the only items nobody else can do):
- A TUI screenshot or asciinema for the README hero (Phase 1 removes the browser
  one; Phase 9 writes the prose around the replacement).
- The Windows smoke after Phase 3 (Reveal from the TUI, `fastf open`) and the
  outstanding ROADMAP item: a same-drive rename and a cross-drive move with the
  published MSI or ZIP.
- The word "release".

## Parking lot

- Prune the deferred-create reader (`assets::copy_job`, `CopyJob`, `Progress`,
  the resume branch in `provisioning::reconcile_create`) once no v1.x journals can
  remain — a later major; it reads journals a dual-boot v1.x might have left.
- Bulk move (several projects at once) existed only in the browser. The TUI has
  single "Move to another base"; add a multi-select variant if it is missed.
- Bootstrap writes the bundled templates without `DataLock` (Phase 4 exception).
- `query::resolve_field` clones per access; `size_scan::request` dedups O(n²);
  the action menu offers no "retry probe" for an `Unresponsive` base; the
  `projects_browser_fills_in_sizes_without_any_input` pty test is timing-sensitive —
  all carried over from the v1.7.1 audit, none worth a phase.
- A DataLock wait behind a long staged move reports "another fastf process is
  busy" even when it is this process's own background work (TUI move). Cosmetic.

## Phase log

(One line per finished phase: date, PR, what differed from the plan.)

- **Phase 1 — 2026-08-23, `phase-01-remove-browser-ui`.** As planned. Three things
  the plan did not name: `util::tree_size::directory_size` (the uncancelled walk)
  had no caller left once the UI went, so it became a test helper and
  `NEVER_CANCELLED` went with it; `assets::JOB_DEFER_BYTES` took its
  `TEXT_MAX_BYTES` compile-time assert with it, since nothing defers any more; and
  the WiX `UiLauncher` component belonged to no `ComponentGroup`, so removing it
  needed no other edit. The Windows cross-*link* could not run here (no
  `mingw-w64-gcc`); Windows clippy did.
- **Phase 2 — 2026-08-23, `phase-02-names-and-numbers`.** Observed failing on the
  pre-change build: the `.hidden`/`..` create test (planned `.hidden` and got a
  `ProjectPlan`), the `.{id}` template test (`save_to_file` returned `Ok`),
  `fastf id set 1000000000000` (succeeded, printing "counter raised to
  1000000000000"), and both manifest tests — the undecodable one is the
  important one, because `save_to_file` returned `Ok` there and the file was
  gone. The `ProjectFolderName` property could not compile, the type not
  existing yet.

  Four deviations from the plan, all narrowing:
  - `--name=` "of only illegal characters" is **not** refused and should not be:
    `?*|` sanitizes to `___`, a real visible folder. Only names that sanitize
    away to *nothing* are refused. The test cases are `..`, `.`, `...`.
  - `--name="   "` is refused one layer earlier, by the required-variable check.
    Noted in the test rather than asserted on `ProjectFolderName`.
  - The unreadable-manifest fixture the plan named (a *directory* at the manifest
    path) proves the message but not the defect: the old code's atomic rename
    onto a non-empty directory failed anyway. A manifest containing invalid UTF-8
    is the fixture where the old code returned `Ok` and destroyed the file. Both
    cases are tested.
  - `Template::validate`'s digit ceiling is `template::MAX_ID_DIGITS` (12,
    matching `Counters::MAX_VALUE`'s width), which *widens* the TUI builder's
    previous 1..=9. The builder now shares the constant instead of its own bound.
- **Phase 3 — 2026-08-23, `phase-03-shelling-out`.** As planned. Two notes:
  `post_create::project_command(program, project_path)` is the single funnel the
  plan implied but did not name — it sets `current_dir` and `FASTF_PROJECT_PATH`
  together, so a future child cannot get one without the other. And the "no
  `start` anywhere" acceptance line contradicts the phase's own decision to keep
  the editor on `cmd /c start` for `.cmd` shims; read as "no `start` in
  `reveal_folder`", which holds. The Windows reveal smoke stays open for the
  maintainer — CI compiles and lints `ShellExecuteW` but cannot open a window.
- **Phase 4 — 2026-08-23, `phase-04-template-writes-locked`.** Two deviations.
  The operations' tests are **integration** tests in `tests/template_engine.rs`
  rather than unit tests in `operations.rs`: both take `DataLock`, whose path is
  `install_dir().join(".fastf.lock")`, so a unit test would lock the developer's
  real data directory — the very defect Phase 7 exists to fix. And the
  concurrency test's lock holder is **the test process itself**
  (`DataLock::acquire_at` on the sandbox's lock file) rather than a second
  spawned fastf: a race between two children is a race, and would pass or fail
  on scheduling. Holding the real lock makes the question exact.

  Found while wiring it: the builder's edit mode could already change a slug, and
  `save_to_file(&tmpl.file_path())` wrote the new manifest into a fresh directory
  and left the old one behind as a second template with the same contents.
  `save_template`'s `original_slug` fixes that; it is why the parameter exists.
- **Phase 5 — 2026-08-23, `phase-05-destination-containment`.** As planned, plus
  three things the plan did not name. `assets::copy_file` now takes
  `(dest_root, rel)` instead of a joined `dest`, because a caller that joins
  first has already skipped the check. `paths::is_link_like` is a new single
  definition of "link" — the reparse-point attribute, not just
  `FileType::is_symlink()` — and `tree_size`'s private copy is gone, so a walker
  cannot end up with a weaker rule than a writer. And `require_real_directory`
  moved to `paths` as the plan said, which upgrades **every** existing caller
  (transactions, provisioning, move_engine, guard, and Phase 4's
  `delete_template`) from the symlink-only test to the reparse-point one.
- **Phase 6 — 2026-08-23, `phase-06-cache-hints`.** One correction to the plan.
  It said a rejected entry "sets the `dropped` flag that already triggers a
  rescan" — `dropped` triggers a *rewrite*, not a rescan, so rejecting every
  entry of a forged cache rewrote it empty and the base's real projects
  disappeared until its mtime next changed. The test caught it. A rejected entry
  now abandons the cache and rescans, which is the honest response: a vanished
  folder is a transient row to drop, but an entry naming a path outside its base
  means the file is no longer fastf's own bookkeeping.
- **Phase 7 — 2026-08-23, `phase-07-test-isolation`.** As planned, with three
  notes. `common::env::EnvGuard` became **public and general**
  (`apply`/`set`/`remove`) rather than private, because `data_dir.rs`'s
  `with_user_dir_env` and `crash_recovery.rs`'s `arm`/`disarm` both had to go
  through it — `with_sandbox` now hands the guard to its body for that reason.
  The `tests/` half of the source scan therefore has real work to do, not just
  the lib's.

  Two mistakes worth recording because the phase is about exactly them.
  `EnvGuard` guards do **not** nest: `ENV_LOCK` is a plain `Mutex` and taking it
  twice on one thread deadlocks — the first version of the unwind test did, and
  hung. And the first version of `a_panicking_test_body_still_restores_the_environment`
  used a private `PANIC_SERIAL` instead of the binary's `SERIAL`, so it raced the
  other tests in `data_dir.rs` and failed intermittently: a second mutex over the
  same process-global, which is the defect this phase removes.
- **Phase 9 — 2026-08-23, `phase-09-docs-and-comments`.** As planned. The
  ROADMAP's "Current phase" became a v2.0.0 summary in *guarantees* rather than
  the nine-deep stack of "Previously:" entries the phases had accumulated —
  `PLAN.md` is the phase-by-phase record and the ROADMAP should not be a second
  one. `src/core/CLAUDE.md`'s pinned `serde-1.0.229/src/private/de.rs:1255`
  became the item name rather than a newer line number: the `flatten` behaviour
  it documents is serde's design, not a version's bug, so there is nothing to
  re-pin.

  Also carried here from the Phase 8 dry run: the first `gates` run failed on
  the Windows leg because Phase 6's forged-cache test compared a canonicalized
  discovery path to a raw `base.join(...)` — different strings for one directory
  once `\\?\` and 8.3 short names are involved. Fixed on the Phase 6 branch and
  merged up the stack, which is the gate doing exactly its job on its first
  run.

  The gate earned itself twice more after that. A second dry run failed on
  Windows because Phase 3's `folder_names_that_are_cmd_syntax_round_trip_through_discovery`
  hand-wrote `folder: %USERPROFILE%` unquoted, and a plain YAML scalar may not
  begin with `%` — the directive indicator. The product was never wrong
  (`project_info::write` goes through `util::yaml` and emits
  `folder: '%USERPROFILE%'`; verified against the release binary), but the
  fixture was, and the test is no longer `cfg(windows)`: `%` and `&` are legal
  folder names everywhere, so gating it to Windows meant it could only fail on
  CI. And smoke-testing the release binary by hand turned up a message that read
  "'' leaves no usable folder name: every character in it is one a folder name
  may not contain" for `--name=..` — nonsense about a string with no characters,
  now two distinct messages.
