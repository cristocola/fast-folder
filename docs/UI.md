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
just opens the browser. Stop a foreground server with Ctrl-C. The desktop entry
`Launch Fast Folder UI.desktop` runs `fastf ui --app`.

## Architecture

Three layers, one binary:

1. **Frontend** — `src/ui/web/{index.html,app.js,styles.css,icon.svg}`: a
   dependency-free single-page app (no framework, no npm, no bundler). Embedded
   into the binary at build time via `include_str!` (see `src/ui/assets.rs`), so
   `fastf` stays a portable single-file distribution.
2. **Server** — `src/ui/mod.rs`: a small loopback HTTP server built on
   `std::net::TcpListener` (one thread per connection, 2 MiB request cap). No web
   framework.
3. **Domain logic** — the server calls the `fastf` library directly
   (`core::project::plan`/`create`, `Config`, `Counters`, `core::template`,
   `core::index`, `core::post_create`). It never shells out to the CLI or parses
   terminal output, so the UI and CLI share one source of truth and the same
   on-disk files (config, templates, counter, project index).

`fastf ui` lives in `src/cli/ui.rs` (process orchestration + browser launching);
the HTTP server and all API handlers live in `src/ui/`. `ui::route_request` is a
pure `(method, route, body) -> Response` function with no socket, so it is unit-
tested directly in `tests/ui_server.rs`.

## HTTP API

Loopback only (`127.0.0.1:47831` by default). No auth/CSRF — **do not** bind to a
non-loopback address.

| Method | Endpoint | Purpose |
|---|---|---|
| GET  | `/api/health` | Health check |
| GET  | `/api/state` | Config, templates, projects, counter, data paths |
| POST | `/api/preview` | Validate variables, return a project plan (no writes) |
| POST | `/api/create` | Create a project + run post-create; returns `{project, job_id}` (`job_id` set only when large assets are copied in the background) |
| GET  | `/api/job/<id>` | Poll a background asset-copy job's progress (404 once evicted after completion) |
| POST | `/api/settings` | Update supported `config.toml` values |
| POST | `/api/templates/save` | Create/update a template's metadata (`template.yaml`) |
| POST | `/api/templates/delete` | Delete a template (its whole `<slug>/` folder) |
| GET  | `/api/template-files?slug=<slug>` | List a template's `files/` subtree (path, size, text content or binary) |
| POST | `/api/templates/file-save` | Write a UTF-8 text file into `files/` (empty content = placeholder) |
| POST | `/api/templates/file-add` | Copy a file from a disk path into `files/` (binaries land byte-identical) |
| POST | `/api/templates/file-delete` | Remove one file from `files/` |
| POST | `/api/open` | Open a path in the system file manager |
| POST | `/api/search` | Run the `fastf search` query language (`tag:`, `key=`, `key>date`, free text); empty terms returns all |
| GET  | `/api/project?path=<abs>` | Full metadata + journal for one project |
| POST | `/api/project/tag` | Add/remove one tag (`{path, action, tag}`) |
| POST | `/api/project/note` | Append a journal entry (`{path, message}`) |
| POST | `/api/register` | Onboard an existing folder (`{path, template?, variables, rename, apply, created?, use_today, overwrite}`) |
| POST | `/api/apply/preview` | Dry-run an apply, return create/skip actions (no writes) |
| POST | `/api/apply` | Create missing folders/files in an existing folder |
| POST | `/api/templates/from-folder` | Generate a template from a folder (`{source, slug, force}`) |
| POST | `/api/projects/prune` | Drop index records whose folders are gone |
| POST | `/api/counter` | Set the global ID counter (`{value}`) |

Write routes (`create`, `settings`, template save/from-folder/delete, template
`file-save`/`file-add`/`file-delete`, `project/tag`, `project/note`, `register`,
`apply`, `projects/prune`, `counter`) serialize through a process-wide mutex
(`WRITE_LOCK`) so concurrent requests can't corrupt files. Read routes
(`/api/search`, `/api/project`, `/api/job/<id>`, `/api/template-files`, the GET
state route) are lock-free. Static GET routes serve only the four embedded
frontend files.

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

**Background copy jobs (v0.8).** `/api/create` copies structure + text/small
files synchronously and returns immediately; bundled files over 4 MiB are copied
on a background thread (**not** holding `WRITE_LOCK` — the copy only touches the
new project's own folder) with chunked progress. The frontend polls
`/api/job/<id>` (~500 ms) and shows a progress bar; a 404 (job evicted after it
finishes) is treated as done. `job_id` is `null` when nothing was deferred.

The path/query GET routes (`/api/project?path=`, `/api/job/<id>`) are matched
before the static-asset catch-all; query values are percent-decoded with a small
built-in decoder (`encodeURIComponent` on the frontend).

Templates are folders — sharing is a folder copy, so there are no import/export
routes (removed in v0.8).

`register_core` (in `src/cli/register.rs`) is the non-interactive engine the
`/api/register` route calls — the CLI `fastf register` is a thin interactive
shell over it. On a `PROJECT_INFO.md` collision the route passes
`PinfoConflict::Abort`, which bails **before** the counter/index writes so the UI
can confirm and retry with `overwrite: true` cleanly.

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

Fast Folder resolves its data dir from the directory containing the running
executable (`paths::install_dir()`, overridable with `FASTF_INSTALL_DIR`).
Because the UI is the same `fastf` binary, it reads/writes the same
`config.toml`, `templates/`, `counters.toml`, and `projects.jsonl` as the CLI.

## Security boundary

The server has no authentication or CSRF protection and can create folders/files,
change settings, edit/delete templates, open local paths, and optionally run
`git init`. Keep it bound to loopback. Exposing it to a LAN/Internet would
require auth, origin validation, strict path authorization, and TLS — none of
which exist today.
