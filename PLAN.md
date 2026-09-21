# PLAN — the todo section grows up

Six phases, one PR each, branch per phase off `main`. The gate list is the `release`
skill's; run it before every PR, not only before the tag.

## Why

`## Todo` in `PROJECT_INFO.md` is the one part of a project fastf can read and write but
only the guided app can reach. There is no command line for it (`ROADMAP.md` backlog: "a
`fastf todo` verb — list, add, toggle — so the command line has what the pane has"), no
machine-readable output for any of it (backlog: "Scriptability: `--json`/`--format`"), and
no way for a template to hand a new project the checklist its workflow always starts with.

A long list needs grouping to stay readable. `core::body`'s grammar already tolerates a
`### Phase` line inside the section — `heading_name` refuses to treat three hashes as a
section boundary and `place_todos` filters non-task lines out — so files with phases parse
today with the phase merely invisible. This plan makes it visible, reachable from the
command line, and seedable from a template.

## How to work a phase

Branch `phase-N-<slug>` off `main`. Scope is that phase only; anything else found goes to
the Parking lot below. Before the PR: `cargo clippy --all-targets -- -D warnings` in debug
and release, `cargo test`, and for a phase that changes a screen, look at the frame with
`tests/tui_pty/screenshot.rs` before writing its snapshot. Update `docs/` in the same PR as
the behaviour. Record what happened in the Phase log.

---

## Phase 1 — a todo carries its phase

`core::body::Todo` gains `phase: Option<String>`. `place_todos` (`src/core/body.rs:520`)
tracks the last `### ` line seen inside the section as it walks; `section_lines` already
yields those lines. Ordinals stay indices into the task list, so nothing renumbers and
`toggle_todo`'s `(ordinal, expected)` contract is untouched.

Tests: `body.rs`'s own `mod tests` for a file with phases, one without, a `###` before any
task, and a phase line at the very end. `tests/metadata.rs` for the round trip through
`operations`.

## Phase 2 — the pane shows phases

`pane_rows` (`src/tui/app/pane.rs:159`, todos at :250) emits a label row when `todo.phase`
changes between consecutive items. Render it the way `PaneRow::Rule` is rendered
(`src/tui/view/projects.rs:477`), dim, one row tall, no wrap. Exclude it from
`PaneRow::selectable()` and `step_cursor`. `tests/tui_snapshots.rs` and the `pane.rs` unit
tests assert row order and need updating.

Watch: `TextThen::AddTodo` (`src/tui/app/actions.rs:283`) sets `pane_pending` to
`detail.todos.len()` so the cursor follows the new todo. That stays right while the app
appends at the end; phase 3's mid-file insert is a command-line path only, and `find_row`
already falls back rather than panicking.

## Phase 3 — `fastf todo list | add | done`

Modelled on `note` end to end: `Commands::Todo` + `TodoAction` in `src/main.rs` beside
`Note` (~:481, :667, dispatch ~:1096), a new `src/cli/todo.rs` printing its own `colored`
output the way `src/cli/note.rs` does, `library::resolve` then the `core::operations`
wrappers that already exist and have no CLI caller (`add_todo` :686, `toggle_todo` :677).

- `todo list <query>` prints the phases and the tasks with their indices and state.
- `todo add <query> <text> [--phase "Name"]` — without `--phase`, today's append. With it,
  one new core function: find the `### Name` block inside `section_span(Todo)`, insert at
  its end, create the heading before `### Other` if the name is absent. Reuse `add_todo`'s
  own refusals (one line, non-empty) rather than re-validating in `cli`.
- `todo done <query> <n>` takes the index `list` printed, reads the list first and passes
  that entry's text as `expected`, so a file changed meanwhile is refused, not mis-ticked.

`cli::extra::classify_extra` is not involved (it serves only `new`/`apply`/`register`), and
`render.rs` is not involved (it prints plans only). Docs: the table in `docs/cli.md:7-36`, a
`## Todos` section beside `## Notes` (:300), and `docs/projects.md`, which today says todos
are pane-only. Tests in `tests/cli_output.rs`.

## Phase 4 — machine output

`--json` on `search` and `recent`, beside `cli::recent::print_plain` rather than inside it,
and `fastf show <query> [--json]` printing one project whole: id, path, base, template,
variables, tags, notes, todos with their phases. `core::library::model::Project` gains
`Serialize`; `project_info::Metadata` already has it. `--plain` keeps its current meaning,
picker versus list, and is untouched. The counter is capped below 2^53 for JSON readers
already.

## Phase 5 — starter todos, and `{id}` at apply

**Starter todos.** `Template` (`src/core/template.rs:16`) gains `#[serde(default)] todo:
Vec<TodoBlock>`, where `TodoBlock { phase: Option<String>, tasks: Vec<String> }`. Validated
in `Template::validate()` (:467) against `add_todo`'s rules, so a bad template is refused
when saved. `project_info::render_at` (`src/core/project_info.rs:274`) writes the `## Todo`
section after `## Notes` using the same interpolation the rest of the create uses, where
`{id}` already resolves. A template `files/PROJECT_INFO.md` cannot do this: the name is
reserved case-insensitively in five places and the metadata write happens before the file
copy. Surface the field in `fastf template show` and the studio; document it in
`docs/templates.md`.

**`{id}` at apply.** `create` inserts `vars["id"]` at `src/core/project.rs:285`; `apply`
(`:713`) and `apply_plan` (`:645`) do not, so a template file containing `{id}` is applied
with the literal token — silently, and `register --apply` hits it hardest
(`src/core/operations.rs:331` runs apply immediately after writing a fresh ID to disk).
Give `{id}` a home in `RenderContext` (`src/core/naming.rs:16`) resolved from
`project_info::read_metadata(target)`, so every surface gets it from one place. Where the
target is not a project the token stays literal and the apply preview says so in a line.
Note `apply_plan` takes `&str date_format` where `apply` takes `&Config`; both must change
together or the preview and the commit disagree.

## Phase 6 — release

Version bump, `docs/` and `ROADMAP.md` (the two backlog entries this plan consumes), the
full gate list including the Windows and release-profile clippy legs, tag, Release workflow,
both AUR packages. Then, outside the repo, `fiverrv` gets its starter todos from the music
video SOP's required modules and Hermes' ingestion skill learns to add only the purchased
extras on top.

---

## Parking lot

- Removing or rewording a todo (`ROADMAP.md` backlog); the pane cannot either.
- An ambiguity picker for `note`, `tag`, `move` and the new `todo` (backlog).
- Per-phase counts in the pane's figures row.

## Phase log

(nothing yet)
