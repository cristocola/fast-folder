# CLAUDE.md — `src/core/` and `src/util/`

The engine: what a project *is*, how one is created, moved and recovered, and
what may never happen while any of that is in flight. The root `CLAUDE.md` has
the orientation, the layering rule and the data-dir and counter models; this file
is the part that bites when you edit these directories.

**The layering rule applies here above all: `core` and `util` import nothing
from `cli` or `tui`, never prompt, and never print.** `tests/layering.rs`
enforces it; `util::diag` is the one sink for anything these layers have to say.

## Templates

`templates/<slug>/template.yaml` is **metadata only**; the file spec is the
sibling `files/` directory. Keys: `naming_pattern` (tokens `{date}`, `{YYYY}`,
`{MM}`, `{DD}`, `{id}`, plus any variable slug), `variables` (`text` or
`select`, with transforms `none` / `title_underscore` / `upper_underscore` /
`lower_underscore`), `structure` (nested `FolderNode`s — the archive-safe way to
declare **empty** dirs), `verbatim` (globs copied literally even if text, which
is how you preserve literal `{braces}`), `exclude`, and an optional per-template
`post_create`.

**`files/` on disk is the source of truth for create and apply, not
`Template.files`.** That field is `#[serde(skip)]` — a hand-written `files:`
block is silently ignored on load — and exists only as a load-time scan of *text*
files for the editors, previews and apply's variable detection. Building a
`Template` in memory with `files` and calling `create` writes nothing.

`Template::load_with(path, FileBuffer::Skip | Load)` decides whether to read that
buffer. `load_all` uses `Skip`: listing, picking and counting templates never
needs contents, and reading every file of every template to print a name was
work nobody asked for. `tui::pickers::pick_template` therefore re-loads the
template it picked, because its caller previews it.

There is **no migrate command** for pre-v0.8 flat `<slug>.yaml` templates and no
flat-form fallback — `load_all` only reads subdirectories with a `template.yaml`.
`Template::OWNED_KEYS` must keep listing `files` and `dir`: without them a flat
`files:` block would start being *preserved* instead of dropped.

`--force` on `template from-folder` **must clear `files/` first**: that subtree
*is* the create spec, so regenerating without clearing merges the old generation
into the new template while the manifest's `structure` is correctly replaced —
two halves that disagree silently.

`core::template_import` is the noninteractive from-folder engine;
`operations::template_from_folder` is the locked entry point. The CLI may
pre-scan read-only for its size confirmation, but the operation rescans beneath
`DataLock` before writing. Text (UTF-8 ≤ 64 KB) becomes editable template files;
binary and large files are bundled only when asked for. A root `PROJECT_INFO.md`
is excluded.

## Interpolation: one context, one pass

`naming::RenderContext { date, yyyy, mm, dd }` is built **once per operation** and
threaded through everything it renders (`project::plan` builds one and carries it
on `ProjectPlan`; apply builds one at entry). The clock used to be sampled inside
`interpolate`, which runs per path segment and per file, so a create spanning
midnight could date the folder differently from the files in it.

Substitution is **one left-to-right pass, and a substituted value is never
re-scanned** — it was a `String::replace` per variable in `HashMap` order, so a
value containing `{another_token}` expanded or not depending on hashing.
Proptests in `tests/properties.rs` pin order-independence, no re-scanning, and
unknown tokens passing through verbatim.

Two shapes, and mixing them is the classic mistake:

- **`interpolate_with`** — raw substitution, for **file content**. Preserves `__`
  so Python's `__version__` survives.
- **`interpolate_name_with`** — then collapses runs of `_`/`-` and trims the
  ends, for **folder and file names**, so an empty optional variable does not
  leave a dangling separator.

A run of two or more separators collapses to the **last** one, because the
leading one belonged to the variable that vanished: `{user}_{artist}-{title}`
with no artist gives `french-Seeping`, not `french_-Seeping`. **Single separators
are never touched**, so a `{date}` of `2026-07-28` passes through intact — that
is the property to protect.

`assets::interp_rel` interpolates each path segment separately, so collapse
happens *within* a name and never across `/`. `interp_rel_os` does the same over
a native path, converting a component only when it contains `{`.

## Path safety

`validated::TemplateSlug` accepts one ASCII alphanumeric/`-`/`_` component.
`validated::SafeRelativePath` normalizes slash styles and rejects empty and dot
components, absolute and drive paths, and `..`; `src/components` stays valid.

Validation happens before path derivation at template lookup and save, and on raw
`files` plus `structure` entries. `project::plan` then validates every physical
and structure path **again after interpolation**, before claiming a folder; the
create and apply file walks repeat the typed boundary check.

`assets::AssetEntry` carries **both** `rel: String` (textual, lossy — what globs,
`SafeRelativePath` reasons about) and `os_rel: PathBuf`
(exact — what you join to open or create a file). **Never join `rel`.** A
template file whose name is not valid UTF-8 was opened at a `?`-substituted path
that does not exist, so the create aborted naming a path the user never wrote.
Validation stays textual because every dangerous component is ASCII.

`paths::display_path` strips `\\?\` **for display and metadata only**. The
verbatim form is what makes paths past MAX_PATH work. Strip at display, never at
storage.

Every recursive walk stops at `paths::MAX_WALK_DEPTH` (**64**) and reports
through `paths::too_deep`; `tree_size` turns it into `None` like any other read
failure. 64 rather than 256 because a Windows *thread* gets a 1 MiB stack and the
TUI browser's size scan runs on worker threads — 256 frames of `read_dir`
iterator overflowed one, which is the exact failure the limit exists to prevent.

## `PROJECT_INFO.md`

Two layers. **YAML frontmatter** — the typed `Metadata`: `id`, `template`,
`template_name`, `created`, `folder`, `path`, `tags`, and `variables:
BTreeMap` holding **every** template variable whether or not it appears in the
naming pattern (a `BTreeMap` for diff-stable ordering). Then a **human body** —
a variables table plus a `## Notes` section the user owns, and a `## Journal`
section from the first `append_journal_entry`. Outside those helpers fastf never
touches the file after creation.

`write_frontmatter(path, |meta| …)` reads → splits → parses → applies → writes
atomically. Body **and** frontmatter bytes are byte-identical after a no-op
mutation, and there is one integration test each.

**Unknown keys survive every mutation.** The re-serialize step is
`util::yaml::to_string_preserving_unknown(&meta, frontmatter, Metadata::OWNED_KEYS)`,
which merges the fresh struct onto the parsed `Mapping` — an `IndexMap`, so a key
fastf has no field for keeps its **position**, not just its value. `OWNED_KEYS`
distinguishes "ours and no longer emitted, remove it" from "not ours, leave it",
and an exhaustiveness test fails if a field is added without updating the list.

**Do not replace this with `#[serde(flatten)]`.** It was the obvious design and
it is wrong: `flatten` routes every field through serde's `Content` buffer, so a
plain unquoted scalar in a hand-edited file (`year: 2026`) arrives as an integer
and the `String` field rejects it — and `read_project_meta` drops that error, so
the project disappears from discovery. Verified in `serde-1.0.229/src/private/de.rs:1255`
and the YAML crate's deserializer.

`render` returns `Result` because the fallback it replaced wrote an *invisible
project*: a `# yaml-serialize-error` comment between valid `---` delimiters
parses as an empty document, so the folder was not a project from the moment it
was created — with a success message on screen. Never substitute a placeholder
for content that defines a file's identity.

`Metadata::from_plan_at` / `write_at` / `render_at` take the timestamp, so
register writes the file **once** rather than writing it with `now` and rewriting
the frontmatter to patch `created`.

**The filename is fixed** (`RESERVED_FILENAME`). `path_is_reserved` is root-only
(leaf match, case-insensitive, no `/` in the normalised path), so
`docs/PROJECT_INFO.md` is fine. `Template::load_from_file`/`save_to_file` strip a
root-level declaration; the builder rejects the name inline and offers `NOTES.md`
as the example, which matters because `PROJECT_INFO.md` as an example teaches
people to write entries that get silently dropped. `pinfo_path(dir)` is the one
helper that builds the path.

**`apply` does not write `PROJECT_INFO.md`** — by design. Only `new` and
`register` do. `apply` retrofits structure into a folder fastf does not
necessarily own; `register` explicitly claims one.

## Create, apply, register

`project::plan()` resolves variables, mints the ID (`counter_value =
next_value(...)`, so preview and commit agree), interpolates the folder name with
`interpolate_name_with`, and validates every rendered path. It writes nothing.

`create_inner` claims the folder with `fs::create_dir` — **not**
`create_dir_all`, which succeeds on an existing directory and let two racers
merge into one folder. Everything after the claim lives in `provision_project` so
a failure rolls the folder back; **nothing may sit between the claim and that
call**, or an early return skips the rollback and leaks the folder. A failpoint
placed one line too early found exactly that.

The `_2` collision suffix: `create_inner` walks `name`, `name_2`, `name_3`, each
a single atomic `create_dir`, so racing processes land on different suffixes and
can never merge. The loop wraps **only** the claim. `create` therefore returns
the plan **as realized** — callers must report from that, not from the plan they
passed in. `config.on_name_collision = "error"` restores
refuse-a-duplicate.

**Register** writes a `PROJECT_INFO.md` into a folder that lacks one. It builds
its `ProjectPlan` directly rather than calling `plan()`, because `plan` always
sets `root_path = base.join(folder_name)` and register's root is an existing
canonical path — keep the two flows separate. The ID comes from an `ID####` token
recovered from the folder name (`naming::parse_id_token`, the *only* place folder
names influence identity) or is minted fresh from the floor; recovering a low ID
never lowers the counter.

Without `--template` it uses a stub (`slug = "(registered)"`). `PinfoConflict`
(Abort / Skip / Overwrite) is what "already a project" means now — there is no
index to consult. `--recursive` writes into every metadata-less direct child of a
base; `--dry-run` previews and writes nothing. `--apply` requires `--template`;
`--rename` falls back to `cfg.register_naming_pattern`, which `config set`
refuses without `{id}` (without it, several folders with the same `{name}` all
rename onto each other).

`naming::sanitize_name` swaps filesystem-illegal characters and does **not**
replace spaces — `fastf new` gets that from the variable's transform. Register's
no-template path has no transform, so it uses `slugify_folder_name`.

`naming::parse_id_token(name, prefix)` (register only) versus `naming::id_value`
(prefix-agnostic trailing digits, for `max_id`). Do not swap them.

## Moving projects

**Invariant: a source is never removed until a complete destination has been
copied, verified and published.**

`library::move_project` is the compatibility shape (takes the lock, revalidates
the recorded project); applications use `operations::move_project` →
`move_project_configured_with_outcome`, which also revalidates the target against
freshly loaded configuration. Move targets are **configured bases only**, enforced
by every caller so a moved project stays discoverable.

Same-filesystem moves are a direct `fs::rename` with no journal. The staged path
is taken **only** for Unix `EXDEV` or Windows `ERROR_NOT_SAME_DEVICE` — permission,
sharing, missing-path and every other rename error returns unchanged. Never
broaden that match. The rename probe is deliberately not wrapped in `fs_retry`:
its failure is the signal to stage, so retrying would add the full backoff to
every cross-drive move.

Staged moves live below the target base at
`.fastf-transactions/<timestamp-pid-counter>/`. `move.json` holds only version,
operation id, project id, configured source base, validated folder components and
`Copying | ReadyToCommit | CleanupPending`; paths are derived from the
transaction's own location. `MoveManifest::scan` is **deny-by-default** — a link
or special entry fails the whole move rather than being omitted — and
`verify_destination` compares the exact path/type/size manifest the scan
produced. Verification must never be narrower than the copy it checks: that is
how a move once deleted a source whose junctions never reached the destination.
Manifests do not hash and do not promise advanced metadata.

Links are refused only on the **staged** path. The same-filesystem rename copies
nothing and preserves links perfectly, so refusing there would block the common
case for no benefit.

Every walked source name is payload — `.tmp`, `.part`, `.fastf-index.json`,
marker-looking names, all of it. There is no suffix-based transient filter.

Before publication, a cancellation or failure removes only the owned transaction
and leaves the source. After publication, cancellation is too late; a
source-removal failure retains `CleanupPending` and reports the destination as
published.

## Recovery

**Create** writes `PROJECT_INFO.md` first with `provisioning: true`, clears the
flag before removing the journal, and writes `.fastf-create-v2.json` on **every**
path. An empty journal cannot prove which interpolated files landed, so it is
reported for inspection. Creates no longer defer any copy, but the resume branch
stays: a journal listing pending copies can still be on a shared drive, written
by a v1.x binary on the other operating system, and it resumes those files after
identity, type and length checks.

**Reconcile** holds `DataLock` for the whole pass and is idempotent. `Copying`
and an unpublished `ReadyToCommit` discard only the owned transaction; a
published `ReadyToCommit` compares project identity and saved manifests before
entering cleanup; `CleanupPending` repeats those checks before source removal.
Missing bases, malformed journals, identity mismatches and unknown states are
report-only.

**Pre-v2 markers contain arbitrary absolute paths and are never read as
authority.** Reconcile reports them as `obsolete` without parsing, migrating,
following, copying, deleting or suffix-sweeping. Never resurrect v1 JSON
migration — fastf having no writer for that format is the point, which is why the
four tests that need those bytes plant them literally.

## Locking and mutation

`util::lockfile::DataLock` is the cross-process lock over the data dir
(`.fastf.lock`). Any read-modify-write of `counters.toml` or `config.toml` must
hold it — an in-process `Mutex` would not see a second fastf running in another
terminal. Windows uses `share_mode(0)`, Unix `flock`; both are released by the OS
on process death, so there is no stale lock to recover.

**Never hold it across a prompt, an editor, a reveal or a post-create hook.**
`cli::new` re-plans inside the lock and runs post-create outside it for exactly
that reason.

**Prompt first, then lock, then reload.** `edit_postcreate_commands` and
`menu_settings_bases` collect the answer, *then* call `operations::update_config`,
which takes the lock and re-reads. Holding a loaded `Config` across a human prompt
and saving it afterwards reverted whatever another `config set` had written
meanwhile. Both remove by **text**, not by the index the user saw.

`core::operations` is the shared mutation entry point: it holds `DataLock`,
reloads config and authoritative project identity beneath it, then refreshes the
disposable caches. **A cached `Project` is a hint and never authorizes deletion by
itself.** A `pub fn` under `core/` that mutates without the lock says `_unlocked`
in its name (`reconcile_unlocked`, `unregister_project_unlocked`,
`delete_project_unlocked`, `rename_project_unlocked`), each `#[doc(hidden)]` with
a `*_configured` application entry point.

`util::fs_retry` wraps the destructive filesystem calls (Windows sharing
violations from Defender or the indexer, plus read-only attribute clearing).

## Tags, search, journal

**Tags** live in `Metadata.tags`. Free-form strings, plus auto-derived ones from
`Template.tag_from`: slug `client_type` with value `Indie` becomes
`client_type/Indie`, and empty values are skipped so there are no orphan `slug/`
tags. `Template::validate()` rejects a `tag_from` entry that is not a declared
variable. `fastf tag reauto` is the safety valve: it removes tags whose prefix
matches a `tag_from` slug and re-derives them, leaving free-form tags untouched.

**Search** (`core/query.rs`) ANDs its predicates; no OR, no parens. Operators:
bare term (free-text substring fallthrough), `key=value`, `key=prefix*`,
`key>date`, `key<date`, `tag:value`, `tag:prefix*`. Fields resolve from
`Metadata` first, then `meta.variables.<slug>`; an unknown key returns `false`
rather than erroring, which keeps it forward-compatible.

`Predicate::Free` is the parser's fallthrough — do not add another below it, it
would be unreachable — and searches **case-insensitive substring** over tags,
variable values, folder, template, template name and id. **`path` is deliberately
excluded**, with a regression test: home-directory text must never produce
phantom matches.

**Journal** entries are append-only markdown lines under `## Journal`:
`- 2026-04-20T14:32:11Z — message`. `notes --since` compares timestamps
lexicographically, which is cheap and correct because ISO-8601 sorts as text.
**Slice a timestamp with `.get(..10)`, never `[..10]`** — a hand-edited file can
put anything there, and byte-slicing panicked on the first multi-byte character.

`library::resolve(cfg, query)` is the shared resolver: exact id → id prefix →
case-insensitive name substring, with a structured error listing candidates when
ambiguous.

Tag mutations call `library::refresh_cache` so lists stay fresh without a
rescan; `note` does not, because the cache stores no journal.

## Post-create actions

`PostCreate` on both `Config` and `Template`; a template-level block overrides the
global one entirely. All fields default to off: `git_init`, `reveal`,
`open_in_editor`, `print_path` (for `$(fastf new ...)` pipelines), and
`commands`, whose `{path}` token is substituted before execution.

Commands run synchronously through the user's shell (`cmd /c` on Windows, `sh -c`
elsewhere). **There is no sandbox** — template authors control this.

`core::post_create::run` returns `Vec<Note>` and does **not** print: `core` may
not write to a stdout the caller may be piping. `Note::Path` is separate from `Note::Done` because
`print_path`'s line is the run's *output*, so it goes to stdout alone and last.

`resolve_post_create()` is `pub` so `cli::new`'s open-prompt can avoid
double-opening when `reveal: true` is already set.

## Output

**`core` produces data; `cli::render` turns it into text.** `project::plan_report`
and `ApplyReport::of` build `DryRunReport`/`ApplyReport`; `cli/render.rs` is the
only module that prints them. 255 lines of `colored` output used to sit in
`core::project`, where the only way to test what a preview *said* was to read
terminal output — so none of it was tested. The report structs have unit tests
now.

`print_tree(nodes, indent)` is the one tree renderer, used by the dry run,
`template show` and the builder. It does not take variables: a preview
interpolates its tree when it builds its report, and `template show` deliberately
prints the raw `{token}` form.

`PreviewKind::{DryRun, BeforeCommit}` decides the header. Both printers are
called on both paths, so a new caller must say which side of the commit it is on
— printing the dry-run header over a real create is the defect this replaced.

`util::diag` is the one sink for anything `core` or `util` says: `warn` for a
best-effort failure that must not change what the operation did, `note` for
something the caller could not have known (a partial project rolled back),
`fatal` for the two paths that cannot return a `Result` (an armed failpoint
calling `abort`, a data directory that cannot be resolved).
