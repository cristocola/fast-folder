# CLAUDE.md — browser UI (`fastf ui`)

Loaded when working under `src/ui/`. Moved out of the root CLAUDE.md so it costs
nothing in sessions that never touch the UI.

`fastf ui` starts a local loopback HTTP server and opens the browser UI. The
server and all API logic live only in the `fastf` lib (`src/ui/`) — no separate
server binary, no external web directory. (The v1.0.2 `fastf-ui` bin is a
windowless *launcher shim* over `cli::ui::run`, not a second server.) Full
reference: `docs/UI.md`.

The UI is at feature parity with the CLI (detail drawer, search, register, apply,
template generate-from-folder + file editor, settings, moves). **`docs/UI.md` is
the route-by-route reference — read it there rather than re-listing endpoints
here**, and see "UI gotchas" below for the constraints that bite.

Three thresholds/guards that live only in code, worth knowing before you touch
the create or template-file paths:
- `assets::JOB_DEFER_BYTES` (**4 MiB**) is the create cutoff: `POST /api/create`
  does structure + text/small files + counter + metadata + cache synchronously and
  returns `{project, job_id}`; anything larger copies on a background thread
  **outside `WRITE_LOCK`**. `job_id` is `null` when nothing needed deferring.
- `require_template_exists` gates every template-file op — `files/` on disk is
  the source of truth, so there is no in-memory buffer and the template must be
  saved first. The file routes are **independent of `templates/save`**, which
  writes metadata only.
- `normalize_template_rel` is the traversal/reserved-name guard on every
  template-file write.

Layout:
- `src/ui/mod.rs` — the HTTP server (`std::net::TcpListener`, one thread per
  connection, no web framework) + all API handlers. `pub fn serve(address)`
  blocks; `pub fn route_request(method, route, body) -> Response` is the pure
  router (no socket) and is what `tests/ui_server.rs` drives. `pub fn
  health_check(address)` lets `fastf ui` detect an already-running server. Write
  routes serialize through a private `static WRITE_LOCK: Mutex<()>`.
- `src/ui/assets.rs` — the four frontend files embedded via `include_str!`. If
  `FASTF_UI_DIR` is set, files are read from disk instead (frontend live-reload).
- `src/ui/web/` — `index.html`, `app.js`, `styles.css`, `icon.svg` (vanilla JS,
  no framework/npm/bundler).
- `src/cli/ui.rs` — the `fastf ui` command: health-check → open browser → serve.
  `--address`, `--no-open`, `--app` (Chromium/Chrome app window with a dedicated
  `~/.cache/fast-folder-ui/chromium` profile; falls back to the default browser).

The server calls the library directly (`project::plan`/`create`, `Config`,
`Counters`, `template`, `library` discovery, `post_create`), so the UI and CLI
share one source of truth and the same on-disk files. The `Ui` arm in `main.rs`
forwards to `cli::ui::run`. `Response` derives `Debug` (tests `unwrap_err` on the router).

## UI gotchas
- v0.6: Only `GET` (assets + read APIs) and `POST` (writes) are routed —
  `HEAD`/others 404. Browsers GET, so this is fine; don't be surprised when
  `curl -I` shows the JSON 404 error body's content-type.
- v0.6: Adding a new write endpoint? Take `lock_writes()` inside the match arm
  (like `/api/create`) so it serializes with the other writers; reads don't lock.
- v0.6: Keep the server **loopback-only**. There is no auth/CSRF. `FASTF_UI_DIR`
  is the frontend dev override (serve assets from disk instead of embedded).
- v0.6: Embedded assets mean a frontend edit needs a `cargo build` to ship — but
  `FASTF_UI_DIR=$PWD/src/ui/web fastf ui` serves from disk for dev without rebuilding.
- v0.7/v0.8: Path/query GET routes (`/api/project?path=`, `/api/job/<id>`) are
  matched with `if path.starts_with(...)` guards placed BEFORE the static-asset
  catch-all (`("GET", path) if !path.starts_with("/api/")`). Order matters — a new
  `/api/...` GET route must go above the catch-all. Query values are percent-decoded by
  the in-module `query_param` + `percent_decode` (no url crate); the frontend uses
  `encodeURIComponent`, which emits `%20` (not `+`) for spaces, so the decoder does
  NOT treat `+` as space (paths can legitimately contain `+`).
- v0.7: `register_core` is the non-interactive engine; `fastf register`'s `run` is a
  thin interactive shell over it (rename preview + pinfo-overwrite prompts there,
  not in core). `RegisterArgs` (CLI) and `RegisterOptions` (engine) are separate
  structs — `run` translates one to the other. The CLI rename-confirm shows the
  exact `old → new` name via a preview computed from `build_plan_vars` +
  `desired_rename` (shared helpers), reading `counter.get()+1` without incrementing;
  the engine recomputes the same value and is authoritative.
- v0.7/v0.9: `PinfoConflict` controls the existing-`PROJECT_INFO.md` policy. `Abort`
  bails BEFORE any write (so the UI can confirm + retry with `overwrite:true`); `Skip`
  keeps the existing file (no rewrite, no cache_upsert — used by `--recursive`);
  `Overwrite` rewrites it. The UI sends `Abort` unless the user confirmed overwrite;
  the CLI single-register sends `Overwrite`/`Skip` from its prompt.
- v0.7: The UI `apply` routes pass **raw** variables to `project::apply_plan` /
  `apply` (no transform/sanitize) — matching `fastf apply`'s semantics, NOT `new`'s.
  Don't "fix" this to apply transforms; it would diverge the UI from the CLI apply.
- v0.7: `/api/search` reuses `core::query` exactly, so the deliberate "path excluded
  from free-text" guarantee holds (there's a regression test). Empty `terms` returns
  all projects (newest first) — the server-side equivalent of the plain list.
- v0.7: The frontend renders the drawer + apply modal + generic (from-folder)
  modal as state-driven layers appended after `shell(content)` in `render()`; each
  has a `bind*` call. Mutating actions re-fetch + full `render()` (acceptable — the
  only focus-sensitive surface is the projects search, which updates `#project-results`
  in place via `runProjectSearch` instead of re-rendering). `(registered)`-slug
  projects are filtered out of the apply/register template pickers.
- v0.8: The `JOBS` registry is `static Mutex<Option<HashMap<..>>>` (const-init'd to
  `None`, lazily filled) keyed by `job-<n>` (`AtomicUsize`). The copy thread updates
  an `Arc<Mutex<Progress>>`; `spawn_copy_job` evicts finished jobs on each new create
  so the map stays bounded, so the frontend `pollJob` treats a `/api/job` 404 as done.
  The background copy must NOT take `WRITE_LOCK` (it only writes inside the new
  project's folder) — holding it would serialize a 200 MB copy against every other UI
  write, defeating the point. Deferred files are always verbatim (threshold ≥ text cap),
  so `copy_job` never needs vars.
- v0.8: `showSuccess(result)` (frontend) now takes the whole create response (not just
  `result.project`) so it can read `job_id`. The progress bar updates only the
  `#job-progress` node inside the imperatively-built success overlay — no `render()`.
- v0.8 (phase 3): The template editor's **Files section is live-on-disk**, NOT part
  of the `templateEditor` object. Never re-add a `files` array to `newTemplateDraft`
  / the save payload — `Template.files` is `#[serde(skip)]`, so it round-trips to
  nothing; a metadata save deliberately doesn't touch `files/`. File CRUD uses the
  four dedicated endpoints and re-fetches `state.templateFiles`. Because they act on
  disk, they need the on-disk slug (`state.templateOriginalSlug`, not the edited
  slug) and the template to exist — new templates show a "save first" notice.
- v0.8 (phase 3): file writes reuse `assets::copy_file(force_verbatim=true)` for
  ingestion (byte copy, atomic `.part`+rename) and `normalize_template_rel` for the
  traversal/reserved guard. Don't interpolate at ingestion time — a template asset
  is stored raw and only interpolated at project-create time. `list_template_files`
  omits directories (empty dirs are the `structure:` section's job).
- v0.9: `load_state`/`search_projects`/`project_detail` all source from
  `library::discover(&config)` (no `index::load_all`). They still read each project's
  metadata fresh for the `tags` field so display stays correct even against a
  momentarily-stale cache. `project_json(project: &Project, metadata)` takes a
  discovered `Project`. `/api/reindex` (write route, takes `lock_writes`) calls
  `library::reindex`; `set_config` accepts a `bases` array (trimmed, non-empty).
  The `/api/projects/prune` route + `prune_projects` fn are **gone** (the cache
  self-heals) — a POST to it 404s (regression test).
- v0.11: `POST /api/project/move` runs OFF `WRITE_LOCK` (background thread) so a slow
  network copy can't block other UI writes — mirroring the create copy job. The two
  base-cache writes it does are atomic + best-effort (last-writer-wins, self-heals via
  the staleness gate), so not holding the global lock is safe. Don't re-add
  `lock_writes()` to that arm.
- v0.11 (UI polish): the Projects table supports **multi-select bulk move** — a
  `.project-select` checkbox per row + a select-all header checkbox feed
  `state.selected` (a `Set` of paths); `bulkBar()` renders the toolbar (target-base
  `<select>` + `Move N`). `runBulkMove` moves them **sequentially** (one job at a time,
  never racing base caches) via `pollMoveJob`, skipping projects already in the target.
  The `.project-row` grid gained a leading `22px` checkbox column (header + rows must
  both add the `.project-select` cell or the grid misaligns). The row click handler
  skips `.project-select` so ticking a box doesn't open the drawer. `bindProjectBulk()`
  (toolbar/select-all) is bound in `bindCommon` only — NOT in `runProjectSearch`'s
  in-place rebind — to avoid double-binding page-level controls. Move overlays wrap
  long names via `overflow-wrap: anywhere` on `.success-modal h2/p` + `.job-label`.
- v1.0: frontend dialogs — `confirmModal()`/`promptModal()` (app.js) are promise-based
  and replace native `confirm()`/`prompt()` (which look alien in --app windows).
  `closeModal()` resolves confirm=false / prompt=null, so Escape/scrim always answer
  "no". The typed-phrase input toggles the danger button **in place** (no render —
  rendering would drop focus).
- v1.0: `render()` saves/restores the focused element by **id** + caret. New
  interactive inputs must have a stable `id` to survive re-renders. The offline banner
  (`setOffline`) and job overlays are managed imperatively outside `render()` on purpose.
- v1.0: `pollMoveJob(jobId, onUpdate, isAlive)` — pass `() => document.body.contains(overlay)`
  from any overlay-scoped caller, or a navigation mid-poll leaks the loop.
- v1.0.1: `write_response` assembles the whole HTTP response and sends it in ONE
  `write_all` — `write!` straight to a `TcpStream` is unbuffered, so each format
  fragment became its own TCP segment and `health_check`'s single read could land
  mid-status-line ("HTTP/1.1 " = 9 bytes), flakily reporting a live server as dead (the
  launcher then tried to re-bind the busy port and died — the "sometimes the icon does
  nothing" bug). `health_check` also now loops its read until the 12-byte status prefix
  is complete. Don't revert either side to single-read/multi-write.
- v1.0.1: **`fastf ui --app` ties the server's lifetime to the app window** —
  `cli::ui::run` serves on a background thread, `child.wait()`s the spawned Chromium
  process, then exits (after draining `ui::jobs_active()` so an in-flight copy is never
  stranded). Closing the window fully stops fastf; the next launcher click starts fresh.
  Only the app-window path does this — terminal `fastf ui`, `--no-open`, and the
  default-browser fallback still serve until Ctrl-C (a browser tab can't be waited on).
  `open_app_window` returns the `Child` for this; `open_browser` (already-running path)
  still spawn-and-drops.
- v1.0.1: `ui::paths_match` canonicalizes both sides when the separator-normalized
  string compare misses — on Windows one folder arrives spelled multiple ways (`\\?\`
  verbatim canonical from discovery vs 8.3 short names like `RUNNER~1` from the
  frontend/tests). String-only comparison broke unregister/delete/rename on Windows CI.
- v1.0.2: **Windows browser probing** — `cli::ui::find_app_browser()` is platform-split:
  unix keeps the exact old chromium/google-chrome list; Windows probes PATH for
  `chrome.exe`/`msedge.exe`/`chromium.exe` then well-known install dirs (Chrome under
  ProgramFiles/x86/LOCALAPPDATA, Edge under ProgramFiles(x86) first — Edge lives in x86
  PF even on x64). Chrome before Edge (user choice wins; Edge is the guaranteed fallback
  and supports `--app`/`--user-data-dir`, keeping the `child.wait()` lifetime tie).
  `which()` now returns the full `PathBuf`. `chromium_profile_dir()` is split too:
  Windows = `%LOCALAPPDATA%\fast-folder-ui\chromium`; both sides fall back to
  `std::env::temp_dir()` (the old `"."` fallback wrote a profile into the CWD).
- v1.0.2: **`ui::open_path` = `reveal_folder` parity** — the web UI's Windows
  open-folder uses `cmd /c start "" <path>` (ShellExecute default verb → respects the
  user's default file manager) instead of hardcoded `explorer.exe`. Keep it in sync with
  `core::post_create::reveal_folder`; never reintroduce `explorer`.
- v1.0.2: **web-UI first-run onboarding** — `/api/state` gains `base_configured` +
  `suggested_base` (home `Projects` folder); write route `POST /api/base/init {path}`
  (accepts `~/…`, rejects relative paths, `create_dir_all` + saves `base_dir`).
  Frontend: `state.modal = {kind:"onboard"}` set in `loadState` when unconfigured;
  dismiss = skip for the session (`state.onboardDismissed`), reappears next launch. The
  modal copy mentions multi-base support (Settings → Library bases) by design.
- v1.0.2: **frozen-renderer recovery** (the "returned after 7 h and the UI was stuck
  with a white popup" bug) — the health poll is a named `healthTick()` with a
  single-in-flight guard + 2.5 s AbortController timeout, re-run on `visibilitychange`.
  A tick arriving >10 min late means the renderer's timers were frozen (suspend/deep
  throttle) → `location.reload()` for fresh surfaces (in-place `loadState()` instead
  when `state.templateDirty`). `open_app_window` additionally passes
  `--disable-backgrounding-occluded-windows` + `--disable-renderer-backgrounding`. Root
  cause is Chromium GPU-surface corruption after long suspend/occlusion — app code can
  only mitigate, so both the prevention flags and the wake-up reload exist.
- v1.0.2: **native path pickers** — `POST /api/pick-path {kind: "folder"|"file", start?}`
  opens the OS dialog server-side (loopback = same machine): Linux kdialog→zenity
  (missing binary falls through, nonzero exit = cancel → `path: null`), Windows
  PowerShell WinForms dialogs (`-STA` required; `CREATE_NO_WINDOW` creation flag so no
  console flashes under the windowless fastf-ui), macOS osascript. Guarded by a
  `PICKER_BUSY` AtomicBool (one dialog at a time) and deliberately does NOT take
  `WRITE_LOCK` (a dialog can sit open for minutes). Frontend: `browseButton(kind, id)`
  + a generic `[data-browse="kind:id"]` handler in `bindCommon` that sets the target
  input's value and dispatches an `input` event (textareas append a line — the bases
  list). Wired on: onboard-base, register-path, ff-source, file-src (file kind),
  settings-base-dir, settings-bases.
- v1.2: **`projectRow` renders a bulk-select checkbox, so every view showing project
  rows needs a `#bulk-bar-slot`.** The dashboard had the checkboxes but not the slot, so
  ticking one highlighted the row and did nothing else. `bindProjectBulk` already runs
  on every render and `refreshSelectionUi` already fills the slot when present.
- v1.2: **the frontend strips `\\?\`, the server does not.** `displayPath()` in `app.js`
  mirrors `util::paths::display_path` and is called by `shortPath()` and every `title`
  attribute. The JSON keeps sending canonical paths because the frontend echoes `path`
  back as the **identifier** on every write route, and the verbatim prefix is what makes
  paths past MAX_PATH work. `data-*` attributes stay canonical; anything rendered goes
  through `displayPath`. A server-side `display_path` field was tried and reverted — two
  mechanisms for one job.
- Live sizes: **never add size to `Project`, `CacheEntry`, or `/api/state`.** Project
  contents change outside fastf, so persisted/boot-time sizes are stale or slow by
  construction. `GET /api/project/size?path=…` authorizes against fresh discovery,
  then calls the crate-internal all-or-nothing walker without `WRITE_LOCK`; unreadable
  or concurrently vanished trees return `size_bytes: null`. The frontend owns the
  only cache (`state.projectSizes`), limits scans to two, prioritizes the drawer, and
  uses `projectSizeGeneration` to discard responses from before a state refresh.
  Keep Size non-sortable and keep the project table horizontally scrollable.

Run `node --check src/ui/web/app.js` after every frontend edit — see the root
CLAUDE.md "Two tooling traps" note for why (backticks inside template literals).
