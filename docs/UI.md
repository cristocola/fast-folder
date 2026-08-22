# Fast Folder Browser UI (`fastf ui`)

A local, single-user browser UI for Fast Folder. It is part of the `fastf`
binary — there is no separate server process and no external web directory.

## Running it

```bash
fastf ui                 # start the server and open the default browser
fastf ui --app           # open a dedicated app window (Chromium/Chrome) if available
fastf ui --no-open       # start the server only (no browser)
fastf ui --address 127.0.0.1:47840   # bind a different loopback port
```

`fastf ui` is idempotent: if a server is already answering on the address it
just opens the browser.

Lifecycle (v1.0.1): with `--app` and a Chromium-family browser available, the
server's lifetime is tied to the app window. Closing the window stops the
server, so every launch starts fresh. In every other mode (terminal `fastf ui`,
`--no-open`, or the default-browser fallback) the server runs in the foreground
until Ctrl-C, since a browser tab cannot be waited on. The packaged desktop
entry (`packaging/fastf.desktop`, installed by the AUR packages as a "Fast
Folder" app-menu entry) runs `fastf ui --app`.

## Architecture

Three layers, one binary:

1. **Frontend** — `src/ui/web/{index.html,app.js,styles.css,icon.svg}`: a
   dependency-free single-page app (no framework, no npm, no bundler). Embedded
   into the binary at build time via `include_str!` (see `src/ui/assets.rs`), so
   `fastf` stays a portable single-file distribution.
2. **Server** — `src/ui/mod.rs`: a small loopback HTTP server built on
   `std::net::TcpListener` (one thread per connection, 2 MiB request cap). No web
   framework.
3. **Domain logic** — mutation handlers call shared `core::operations` functions;
   read handlers use `core::library` discovery and the template/config readers.
   The server never shells out to the CLI or parses terminal output, so the UI,
   TUI, and CLI use the same validation, locking, and on-disk state.

`fastf ui` lives in `src/cli/ui.rs` (process orchestration + browser launching);
the HTTP server and all API handlers live in `src/ui/`. `ui::route_request` is a
pure `(method, route, body) -> Response` function with no socket, so it is unit-
tested directly in `tests/ui_server.rs`.

## HTTP API

The default is loopback (`127.0.0.1:47831`). The server resolves the requested
bind address and refuses it unless every result is loopback. Socket requests
must carry a loopback `Host` whose port matches the listener; when an `Origin`
header is present it must be `http` and have the same authority. There is no
authentication, so this remains a local, single-user interface.

| Method | Endpoint | Purpose |
|---|---|---|
| GET  | `/api/health` | Health check |
| GET  | `/api/state` | Config, templates, discovered projects, counter, data paths, `base_configured` + `suggested_base` (first-run onboarding) |
| POST | `/api/base/init` | First-run onboarding: create the chosen projects folder if missing and set it as `base_dir` (`{path}`; accepts `~/…`, must be absolute) |
| POST | `/api/pick-path` | Open the native OS folder/file picker on the server's machine (`{kind: "folder"\|"file", start?}`); returns `{path}` or `path: null` on cancel. One dialog at a time; kdialog/zenity on Linux, PowerShell dialogs on Windows |
| POST | `/api/preview` | Validate variables, return a project plan (no writes) |
| POST | `/api/create` | Create a project + run post-create; returns `{project, job_id}` (`job_id` set only when large assets are copied in the background) |
| GET  | `/api/job/<id>` | Poll a background copy/move job's progress (`copying`→`verifying`→`finalizing`→`done`; 404 once evicted after completion). Additive move fields may report `cleanup_pending` and a `warning` |
| POST | `/api/job/<id>/cancel` | Request cancellation of a running job (cleans only its exact owned temp/staging path; a move source remains untouched before publication) |
| POST | `/api/settings` | Update supported `config.toml` values |
| POST | `/api/templates/save` | Create/update a template's metadata (`template.yaml`) |
| POST | `/api/templates/delete` | Delete a template (its whole `<slug>/` folder) |
| GET  | `/api/template-files?slug=<slug>` | List a template's `files/` subtree (path, size, text content or binary) |
| POST | `/api/templates/file-save` | Write a UTF-8 text file into `files/` (empty content = placeholder) |
| POST | `/api/templates/file-add` | Copy a file from a disk path into `files/` (binaries land byte-identical) |
| POST | `/api/templates/file-delete` | Remove one file from `files/` |
| POST | `/api/open` | Open a path in the system file manager. The path must be a currently discovered project, the data directory, or the templates directory; anything else is 403 |
| POST | `/api/search` | Run the `fastf search` query language (`tag:`, `key=`, `key>date`, free text); empty terms returns all |
| GET  | `/api/project?path=<abs>` | Full metadata + journal for one project. A path that is not a currently discovered project is 403 |
| GET  | `/api/project/size?path=<abs>` | Live logical-byte folder size for a currently discovered project; `size_bytes` is `null` when the tree becomes unreadable/unavailable |
| POST | `/api/project/tag` | Add/remove one tag (`{path, action, tag}`) |
| POST | `/api/project/note` | Append a journal entry (`{path, message}`) |
| POST | `/api/project/move` | Move a project into another configured base (`{path, base}`); target must be in `effective_bases()`. Returns `{job_id}` — the move runs in the background (same-fs = instant rename; cross-fs = staged copy → verify → commit → remove source). Poll `/api/job/<id>` |
| POST | `/api/project/unregister` | Remove a project's `PROJECT_INFO.md` so fastf forgets it (`{path}`); files stay on disk |
| POST | `/api/project/delete` | Recursively delete a project folder (`{path, confirm_name}` — `confirm_name` must equal the folder name; restricted to configured bases) |
| POST | `/api/project/rename` | Rename a project folder in place (`{path, folder}`); metadata `folder`/`path` patched, cache updated |
| POST | `/api/register` | Onboard an existing folder (`{path, template?, variables, rename, apply, created?, use_today, overwrite}`) |
| POST | `/api/apply/preview` | Dry-run an apply, return create/skip actions (no writes) |
| POST | `/api/apply` | Create missing folders/files in an existing folder |
| POST | `/api/templates/from-folder` | Generate a template from a folder (`{source, slug, force, bundle_assets}`); returns a `report` of counts (folders / text files / bundled + bytes / skipped) |
| POST | `/api/reindex` | Force a full rescan of every base, rewriting each `.fastf-index.json`; returns `{projects}` |
| POST | `/api/reconcile` | Reconcile scoped v2 creates/moves across all bases. Additive counts report resumed deferred copies, completed published moves, or rolled-back unpublished transactions; `obsolete` lists untouched pre-v2 markers, `incomplete` lists provisioning flags without usable scoped recovery, and `unrecoverable` lists states rejected without mutation. `swept` remains for response compatibility and is always zero |
| POST | `/api/counter` | Raise the global ID counter (`{value}`). The counter is the highest ID seen anywhere and only moves up, so a value at or below the current floor is **refused**; the write propagates to every mounted base |

Most synchronous write routes serialize handler entry through a process-wide
mutex (`WRITE_LOCK`). It is only an in-process UI guard, not the correctness
boundary. Shared create, register, apply, move, reconcile, configuration,
counter/reindex, tag/note, rename, unregister, and delete operations acquire the
cross-process `DataLock`, reload authoritative state beneath it, mutate, and
refresh caches. Background create and move workers do not hold `WRITE_LOCK`, but
retain `DataLock` for the complete filesystem operation. Read routes
(`/api/search`, `/api/project`, `/api/project/size`, `/api/job/<id>`,
`/api/template-files`, the GET state route) are lock-free. Static GET routes
serve only the four embedded frontend files.

`DataLock` is deliberately one coarse mutation lock: another fastf mutation
waits, while reads and job cancellation remain available. It is never held
across a user prompt, editor launch, folder reveal, or post-create command.

**First-run onboarding (v1.0.2).** When no base is configured anywhere
(`base_dir` empty and `bases` empty), `/api/state` reports
`base_configured: false` plus a `suggested_base` (the user's home `Projects`
folder). The frontend then shows a welcome dialog that explains the base
concept, pre-fills the suggestion, notes that more bases can be added later
under Settings > Library bases, and submits to `/api/base/init`, which creates
the folder and saves it as `base_dir`. Dismissing the dialog skips it for the
session; it returns on the next launch until a base is set. The TUI runs the
same flow as a prompt on launch — both surfaces share
`config::init_base_dir` / `config::suggested_base_dir`.

**Native path pickers (v1.0.2).** Every manual path input (onboarding,
register, generate-from-folder source, template file-add source, Settings base
directory and Library bases) has a Browse button wired to `/api/pick-path`,
which opens the real OS dialog server-side (same machine as the browser). The
bases textarea appends the picked folder as a new line instead of replacing.
Only one dialog can be open at once; a second request errors cleanly.

**Health watch + sleep recovery (v1.0.2).** The frontend polls `/api/health`
every 5 s (single in-flight request, 2.5 s abort timeout) to drive the offline
banner, and re-checks immediately on `visibilitychange`. If a poll tick arrives
more than 10 minutes late, the renderer's timers were frozen (system suspend or
deep window throttling) — Chromium can wake from that with corrupted surfaces
and stuck native popups, so the page reloads itself for a clean slate (or
refreshes in place when unsaved template edits would be lost). The `--app`
launcher also passes `--disable-backgrounding-occluded-windows` and
`--disable-renderer-backgrounding` to keep the window from being throttled
into that state in the first place.

**Live project sizes.** `/api/state` intentionally contains neither size values
nor project-tree walks, so the application shell and project metadata render
without waiting for large or network-hosted folders. After rendering, the
frontend progressively calls `GET /api/project/size?path=…` for the six Overview
rows, the current Projects/Search result set, and an open drawer project (drawer
requests have queue priority). No more than two size requests run concurrently.

The endpoint first requires the absolute path to match a currently discovered
project, then performs a lock-free, read-only walk. Its response is
`{"ok":true,"path":"…","size_bytes":1234}`; a project that becomes
unreadable or disappears between authorization and walking returns
`size_bytes: null`. Arbitrary paths are rejected. The walk sums regular-file
logical lengths, including hidden files and `PROJECT_INFO.md`, never follows
symlinks/junctions, and ignores special nodes. Any read failure makes the whole
snapshot unavailable rather than partial.

Results live only in frontend session state. A state refresh advances a
generation and stale in-flight responses are discarded. Rows and the drawer
show quiet Scanning/Unavailable states; completed values extend through GB/TB
and expose the exact byte count in a tooltip. Size is deliberately non-sortable,
Created remains the default sort, and the project table scrolls horizontally
when its full column set cannot fit.

**Template file editing (v0.8).** The template editor's Files section works
directly on the `files/` subtree on disk — `template-files` lists it, `file-save`
writes/updates a text file (or creates a placeholder), `file-add` copies an asset
from a disk path (the local-first ingestion route — no large file round-trips
through the browser), and `file-delete` removes one. These are independent of the
metadata `templates/save` button, which only writes `template.yaml` (a template's
`files` buffer is `#[serde(skip)]`, so a metadata save never touches `files/`).
Because file ops act on disk, the editor requires the template to already exist —
new templates must be saved once before their files can be managed. Dest paths are
traversal-guarded and reject the reserved `PROJECT_INFO.md`. `verbatim`/`exclude`
globs are ordinary metadata, edited in the editor and persisted via
`templates/save`.

**Background copy jobs.** `/api/create` copies structure + text/small files
synchronously and returns immediately; bundled files over 4 MiB are copied on a
background thread with chunked progress. The worker does not hold the UI-only
`WRITE_LOCK`, but it retains the shared `DataLock` until every deferred file has
landed and the create journal/provisioning flag are cleared. The frontend polls
`/api/job/<id>` (~500 ms) and shows a progress bar; a 404 (job evicted after it
finishes) is treated as done. `job_id` is `null` when nothing was deferred.
Template-level post-create policy overrides the global policy consistently. All
file-dependent post-create actions run only after provisioning completes and
after the mutation lock is released.

**Scoped journals + verified moves.** A deferred create writes
`.fastf-create-v2.json`, containing only a template slug and validated relative
copy paths. Reconcile can resume a missing deferred file after checking identity,
type, and byte length; completion clears the provisioning flag before the
journal. A move returns a `job_id` and runs in the background: same filesystem =
direct OS rename with no journal; cross-filesystem/network = exclusive
`.fastf-transactions/<operation-id>/staging` → verify exact path/type/size and
unchanged source metadata → publish → record `CleanupPending` → remove source.
**The source is never removed until the destination is verified and published.**
Jobs report a `phase` (`copying`/`verifying`/`finalizing`/`done`) and can be
cancelled before publication. If source removal fails, the job finishes with
`cleanup_pending: true`, a warning, and the transaction retained for retry.
Malformed/unknown v2 state is reported without mutation. Pre-v2 markers remain
obsolete and byte-untouched; reconcile never parses or follows their paths and
never sweeps suffix-named files.

**Moving projects.** A single project moves from its detail drawer (base select
+ Move button). The Projects table also supports **multi-select**: a checkbox per
row + a select-all header checkbox drive a bulk toolbar (`N selected` · target
base `<select>` · `Move N`), which relocates every checked project into the
chosen base **sequentially** (one `/api/project/move` job at a time so they never
race the base caches), skipping any already there. Both single and bulk moves
show a `copying → verifying → finalizing` progress overlay with a Cancel button;
selection state lives in `state.selected` (a `Set` of project paths).

The path/query GET routes (`/api/project?path=`, `/api/project/size?path=`,
`/api/job/<id>`) are matched
before the static-asset catch-all; query values are percent-decoded with a small
built-in decoder (`encodeURIComponent` on the frontend).

Templates are folders — sharing is a folder copy, so there are no import/export
routes (removed in v0.8).

`core::operations::register` is the non-interactive engine used by both the UI
and CLI. It accepts direct children of configured bases only, makes `Skip` an
immediate no-op, rejects duplicate recovered IDs, commits registration before
optional rename/apply follow-ups, and reports those follow-ups as partial
outcomes if they fail. On a `PROJECT_INFO.md` collision the UI initially passes
`PinfoConflict::Abort`, then retries with explicit overwrite after confirmation.

## Frontend development (live reload)

Embedded assets are baked in at build time. To edit the frontend without
rebuilding, point `FASTF_UI_DIR` at the source web directory — the server then
reads those files from disk per request (responses are `Cache-Control: no-store`,
so a browser refresh is enough):

```bash
FASTF_UI_DIR=$PWD/src/ui/web fastf ui --no-open
```

Backend (Rust) changes still require a rebuild and server restart.

## Data location

Fast Folder resolves its data dir with a three-tier precedence (v1.0,
`paths::try_install_dir()`): the `FASTF_INSTALL_DIR` env override, else the
binary's own directory when a `config.toml`/`templates/` sits next to it
(portable mode), else the per-user config directory (`~/.config/fastf`,
`%APPDATA%\fastf`) — which is what a package-manager install uses. `fastf
paths` prints the resolved dir + mode; `/api/state` exposes it as `dir_mode`.
Because the UI is the same `fastf` binary, it reads/writes the same
`config.toml` and `templates/` as the CLI, and the same per-base
`.fastf-counter.toml` files (the data dir's `counters.toml` is one backup input
to the floor, not the record). Projects are
discovered from their `PROJECT_INFO.md` across the configured bases (v0.9 — no
`projects.jsonl`), each base holding its own `.fastf-index.json` cache.

Request bodies are capped at 2 MiB (`MAX_REQUEST_SIZE`) — oversized or
malformed requests get a clean JSON 400, never a crashed connection thread.

## Security boundary

The server has no authentication and can create folders/files, change settings,
edit/delete templates, open local paths, and optionally run `git init`. It
enforces loopback binding, a loopback `Host` with the listener port, and
same-origin `Origin` when present. Those checks reduce the local attack surface;
they do not turn the UI into a remotely deployable service. LAN/Internet use
would additionally require authentication, TLS, and a different security model.

Three further limits on what a request can reach:

- **Every route that names a path resolves it through discovery first.** Only a
  folder that is currently a project can be read, mutated, or opened; `/api/open`
  additionally allows the data and templates directories, because Settings has a
  button for each. Anything else is 403. Membership is whole-path identity, never
  a prefix, so a sibling folder whose name merely starts the same way is refused.
- **A declared `Content-Length` over the 2 MiB cap is refused before the body is
  read**, so no arithmetic on it can overflow. That is what makes the clean-400
  promise above true rather than aspirational.
- **Every response carries a `Content-Security-Policy`** of `default-src 'self'`
  with `frame-ancestors 'none'`. The frontend is same-origin and has no inline
  script, so `script-src 'self'` holds; adding an inline handler or a
  `javascript:` URL to `app.js` would break the page silently.
