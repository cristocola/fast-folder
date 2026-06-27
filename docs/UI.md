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
| POST | `/api/create` | Create a project + run requested post-create actions |
| POST | `/api/settings` | Update supported `config.toml` values |
| POST | `/api/templates/save` | Create/update a template YAML |
| POST | `/api/templates/delete` | Delete a template YAML |
| POST | `/api/open` | Open a path in the system file manager |
| POST | `/api/search` | Run the `fastf search` query language (`tag:`, `key=`, `key>date`, free text); empty terms returns all |
| GET  | `/api/project?path=<abs>` | Full metadata + journal for one project |
| POST | `/api/project/tag` | Add/remove one tag (`{path, action, tag}`) |
| POST | `/api/project/note` | Append a journal entry (`{path, message}`) |
| POST | `/api/register` | Onboard an existing folder (`{path, template?, variables, rename, apply, created?, use_today, overwrite}`) |
| POST | `/api/apply/preview` | Dry-run an apply, return create/skip actions (no writes) |
| POST | `/api/apply` | Create missing folders/files in an existing folder |
| POST | `/api/templates/import` | Save a template from pasted YAML (`{yaml, overwrite}`) |
| GET  | `/api/templates/export?slug=<slug>` | Download a template's raw YAML |
| POST | `/api/templates/from-folder` | Generate a template from a folder (`{source, slug, force}`) |
| POST | `/api/projects/prune` | Drop index records whose folders are gone |
| POST | `/api/counter` | Set the global ID counter (`{value}`) |

Write routes (`create`, `settings`, template save/import/from-folder/delete,
`project/tag`, `project/note`, `register`, `apply`, `projects/prune`, `counter`)
serialize through a process-wide mutex (`WRITE_LOCK`) so concurrent requests
can't corrupt files. Read routes (`/api/search`, `/api/project`, the GET state
and export routes) are lock-free. Static GET routes serve only the four embedded
frontend files.

The query-string GET routes (`/api/project?path=`, `/api/templates/export?slug=`)
are matched before the static-asset catch-all; values are percent-decoded with a
small built-in decoder (`encodeURIComponent` on the frontend).

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
