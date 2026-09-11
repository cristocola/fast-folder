# CLAUDE.md — `src/core/` and `src/util/`

The engine: what a project *is*, how one is created, moved and recovered, and
what may never happen while any of that is in flight. The root `CLAUDE.md` has
the orientation, the layering rule and the data-dir and counter models; this file
is the part that bites when you edit these directories. **`core` and `util`
import nothing from `cli` or `tui`, never prompt, and never print**
(`tests/layering.rs`); `util::diag` is the one sink for anything they have to say.

## Templates

`templates/<slug>/template.yaml` is **metadata only**; the file spec is the
sibling `files/` directory. Keys: `naming_pattern` (tokens `{date}`, `{YYYY}`,
`{MM}`, `{DD}`, `{id}`, plus any variable slug), `variables` (`text` or `select`,
transforms `none` / `title_underscore` / `upper_underscore` / `lower_underscore`),
`structure` (nested `FolderNode`s — the archive-safe way to declare **empty**
dirs), `verbatim` (globs copied literally, to keep literal `{braces}`),
`exclude`, and an optional `post_create`.

**`files/` on disk is the source of truth for create and apply, not
`Template.files`**, which is `#[serde(skip)]` (a hand-written `files:` block is
ignored) and only a load-time scan of text files for the editors, previews and
apply's variable detection; an in-memory `Template` with `files` creates nothing.
`Template::load_with(path, FileBuffer::Skip | Load)` decides whether to read it.
`load_all` skips, since listing never needs contents, so
`tui::pickers::pick_template` re-loads what it picked for the preview.

**The directory is the template's identity; the manifest's `slug:` is cosmetic.**
`load_with` takes the folder name as the slug (a disagreeing manifest value is
kept in `declared_slug` for `template show`), and `find_by_slug` is the only door,
so a manifest can neither hide a listed template nor let two folders answer to one
name. Projects are the mirror image: discovered, so their folder is cosmetic and
`id` is identity; a template is looked up by a path component, so the path wins.
fastf never writes a mismatch (`save_template` writes to `template_dir(slug)`);
`cp -r` plus a half-finished edit does, and a save repairs it.

There is **no migrate command** and no flat-form fallback for pre-v0.8
`<slug>.yaml` templates. `Template::OWNED_KEYS` must keep `files` and `dir`, or a
flat `files:` block would be preserved instead of dropped.

`--force` on `template from-folder` **must clear `files/` first**: that subtree
is the create spec, and regenerating over it would merge old files under a
replaced `structure`. `core::template_import` is the from-folder engine and
`operations::template_from_folder` the locked entry point, which rescans under
`DataLock` whatever the CLI pre-scanned. Text (UTF-8 ≤ 64 KB) becomes editable
files, binary and large files are bundled only on request, and a root
`PROJECT_INFO.md` is excluded.

## Interpolation: one context, one pass

`naming::RenderContext { date, yyyy, mm, dd }` is built **once per operation**
(`project::plan` carries it on `ProjectPlan`; apply builds one at entry), so a
create spanning midnight dates folder and files alike. Substitution is **one
left-to-right pass that never re-scans a substituted value**, so a value
containing `{token}` stays literal in any variable order; `tests/properties.rs`
pins order-independence, no re-scan, and unknown tokens passing through.

Two shapes, and mixing them is the classic mistake: **`interpolate_with`** for
**file content** (raw, so `__version__` survives), and **`interpolate_name_with`**
for **folder and file names**, which also collapses runs of `_`/`-` and trims the
ends so an empty optional variable leaves no dangling separator. A run collapses
to its **last** separator — `{user}_{artist}-{title}` with no artist is
`french-Seeping` — and **single separators are never touched**, so `2026-07-28`
survives. `assets::interp_rel` interpolates per path segment, never across `/`;
`interp_rel_os` does the same over a native path.

## One classification per template file

**`assets::plan_entries` alone decides what happens to a file under `files/`**
(`FileAction::{Skipped, Folder, Unsupported, Interpolated, Verbatim}`): `exclude`
on the path as the template spells it, interpolate, the two reserved predicates,
then `verbatim` or oversize. `copy_template_files`, `apply_plan_resolved`, the dry
run's file list and its previews all read it. A preview must never iterate
`Template.files`, or an excluded file previews a body and a verbatim one previews
substituted, breaking `docs/cli.md`'s promise that the preview is built by the
code that commits.

It yields one `PlannedEntry` per walked entry, `Skipped` included, so anything
counting entries (a failpoint, a progress bar) sees them all. It is **infallible**
policy, and each caller keeps its own `SafeRelativePath` check, which is
load-bearing in `apply`, the one path that never goes through `plan()`. The walk
decides which files exist, so a template with a `files` buffer and no directory
previews and writes nothing.

## Path safety

`validated::TemplateSlug` is one ASCII alphanumeric/`-`/`_` component;
`validated::SafeRelativePath` normalizes slashes and rejects empty and dot
components, absolute and drive paths, and `..`. Both run before path derivation
at template lookup and save and on raw `files`/`structure` entries;
`project::plan` re-validates every path **after interpolation**, and the create
and apply walks repeat the check.

`assets::AssetEntry` carries `rel: String` (lossy, for globs and validation) and
`os_rel: PathBuf` (exact, for opening). **Never join `rel`**: a non-UTF-8 name
would open a `?`-substituted path that does not exist. Validation stays textual
because every dangerous component is ASCII.

**Path safety is two layers.** `SafeRelativePath` and
`paths::require_native_relative` prove the *text* cannot escape, but
`create_dir_all` walks straight through an existing `docs -> /outside`.
`paths::contained_destination(root, rel)` is the physical layer: `root` a real
directory, every existing component of `root/rel` a real directory (the last may
be a file), none a link; components that do not exist yet are fine. **Call it
immediately before the write.** `assets::copy_file` takes `(dest_root, rel)` for
that reason; `create_structure`, apply's structure loop, `copy_template_files`,
`Template::save_to_file`, `template_import`'s bundling and `provisioning`'s resume
all use it; `copy_job`'s caller derives its destination through it first. This is
not a race-free `openat2` fortress: the threat model is one user's own filesystem
(`docs/projects.md`, "What fastf promises").

`paths::is_link_like` is the **one** definition of "link", and the widest — any
Windows reparse point, junctions included — and `tree_size` shares it.
`paths::display_path` strips `\\?\` **for display and metadata only**; the
verbatim form is what makes paths past MAX_PATH work.

**Every recursive walk stops at `paths::MAX_WALK_DEPTH` (64)** through
`paths::too_deep` (`tree_size` maps it to `None`). 64, not 256, because a Windows
*thread* has a 1 MiB stack and the size scan and discovery run on workers; a stack
overflow is not an unwind, which is also why those workers set `stack_size`.
**Thread the depth**: each walk is a `_at` function taking `depth` behind a
zero-initialising entry point, and recursing through the entry point resets the
count and makes the guard unreachable. A template's `structure:` is bounded once,
in `template::validate_structure`, which every load and save passes through.

## `PROJECT_INFO.md`

**YAML frontmatter** is the typed `Metadata`: `id`, `template`, `template_name`,
`created`, `folder`, `path`, `tags`, `auto_tags` (the derived subset), and
`variables: BTreeMap` holding **every** template variable (sorted, for stable
diffs). The **body** holds a variables table, a `## Notes` section and, after the
first `add_todo`, a `## Todo` list; its grammar is `core/body.rs`, and outside
those helpers fastf never touches the file after creation.

`write_frontmatter(path, |meta| …)` reads, splits, parses, applies and writes
atomically, byte-identical after a no-op (one test per half). It wraps
`write_document(path, |meta, body| …)` — **one read, one mutation over both
halves, one atomic write** — for a verb that changes both, since two writes could
leave (or be killed leaving) a table that disagrees with its frontmatter.

**The variables table is regenerated only while it is fastf's.**
`variables_table` is the one definition (`render_at` writes it,
`sync_variables_table` rewrites it), recognised by shape: the first `|` run under
`# Project Info` with `Variable`/`Value` headers and a dash line. A reshaped,
renamed or absent table is left byte for byte, because the body is the user's —
`docs/projects.md` promises exactly this.

**Sections are found by one rule**: `body::section_span(content, Section)` is the
byte range from a `##` heading line to before the next `##` line, walked after
`split_frontmatter_body` and returned as whole-file offsets for splicing. Headings
match at line start, in any case, with or without a trailing colon; `###` neither
starts nor ends one. Every writer changes only the bytes it is about.

**Unknown keys survive every mutation**:
`util::yaml::to_string_preserving_unknown(&meta, frontmatter, Metadata::OWNED_KEYS)`
merges onto the parsed `Mapping` (an `IndexMap`, so position is kept), and
`OWNED_KEYS` separates "ours, remove" from "not ours, keep", with an exhaustiveness
test. **Do not use `#[serde(flatten)]`**: it routes fields through serde's
`Content` buffer, so `year: 2026` arrives as an integer, the `String` field fails,
and the project drops out of discovery — by design
(`private::de::ContentDeserializer`), not a bug to wait out.

`render` returns `Result`: a placeholder such as `# yaml-serialize-error` between
`---` lines parses as an empty document, which would create a non-project under a
success message. Never substitute a placeholder for identity-defining content.
`Metadata::from_plan_at` / `write_at` / `render_at` take the timestamp, so register
writes the file once.

**The filename is fixed** (`RESERVED_FILENAME`). `path_is_reserved` is root-only
(a case-insensitive leaf with no `/`), so `docs/PROJECT_INFO.md` is fine; template
load and save strip a root-level declaration, and the builder refuses the name as
it is typed (`studio::file_from`), since such an entry would be dropped in
silence. `pinfo_path(dir)` builds the path. **`apply` does not write
`PROJECT_INFO.md`**: it retrofits structure into a folder fastf may not own, and
only `new` and `register` claim one.

## Create, apply, register

`project::plan()` resolves variables, mints the ID with `next_value(...)?` (so
preview and commit agree), interpolates the name with `interpolate_name_with`,
validates every rendered path, and writes nothing.

**`validated::ProjectFolderName` is the one folder-name validator**: `plan`,
`library::rename_project_inner` and register's rename all use it. Beneath it
`naming::sanitize_name` maps illegal characters and trims what Windows trims but
*refuses nothing*; `ProjectFolderName` refuses empty, dot-prefixed (discovery
skips those) and multi-component names, naming the rendered value and its pattern,
because the user typed a variable, not a folder name. `Template::validate` refuses
a `naming_pattern` starting with `.`, an empty `id.prefix` (or `parse_id_token`
would read `Album_2024` as ID 2024), and `id.digits` outside `1..=MAX_ID_DIGITS`.

`create_inner` claims with `fs::create_dir`, **not** `create_dir_all`, which
would let two racers merge into one folder, after re-checking that
`root_path.parent()` **is** the base (an empty name joins to the base itself,
whose parent is outside the library). Everything after the claim is in
`provision_project`, which rolls the folder back on failure; **nothing may sit
between the claim and that call**. A collision walks `name`, `name_2`, … each an
atomic `create_dir`, so racers take different suffixes; the loop wraps only the
claim, and `create` returns the plan **as realized**, which callers must report
from. `on_name_collision = "error"` refuses instead.

**Register** writes a `PROJECT_INFO.md` into a folder that lacks one and builds
its own `ProjectPlan`, because `plan` sets `root_path = base.join(folder_name)`
and register's root is an existing canonical path. Its ID is an `ID####` token in
the folder name (`naming::parse_id_token`, the *only* place a folder name affects
identity) or fresh from the floor; a recovered low ID never lowers the counter.
Without `--template` it uses the `(registered)` stub. `PinfoConflict` (Abort /
Skip / Overwrite) is what "already a project" means. `--recursive` covers every
metadata-less direct child; `--dry-run` writes nothing; `--apply` needs
`--template`; `--rename` falls back to `cfg.register_naming_pattern`, which
`config set` refuses without `{id}`, or same-named folders would rename onto each
other. The template-less path has no transform, so it uses `slugify_folder_name`
(`sanitize_name` keeps spaces). `parse_id_token(name, prefix)` is register's;
`naming::id_value` is prefix-agnostic trailing digits. Do not swap them.

**A project's number is written down, not re-derived.** `Metadata.id_number`
holds what the counter minted; `Project::number()` prefers it and falls back to
`id_value` for older files. `Counters::format_id` is lossy (prefix `20` with two
digits and prefix `2` with three both render 1 as `2001`), and parsing a
digits-only prefix back guesses high — which, fed to the monotonic floor,
renumbers the library for good. A prefix-aware parse would cost a template load
per row on every create and preview, has no answer for template-less or foreign
projects, and mints duplicates when it guesses low, so the lookup happens once, in
`reindex`, which backfills what it can resolve. The field is `Option<u64>` with
`skip_serializing_if`, and `CacheEntry` carries it `serde(default)` with **no
`CACHE_VERSION` bump** — an older file reads `None` and falls back.

## Moving projects

**Invariant: a source is never removed until a complete destination has been
copied, verified and published.**

`library::move_project` is the compatibility shape (lock, revalidate);
applications use `operations::move_project` →
`move_project_configured_with_outcome`, which also revalidates the target against
freshly loaded configuration. Targets are **configured bases only**, so a moved
project stays discoverable.

A same-filesystem move is a plain `fs::rename` with no journal. The staged path is
taken **only** for Unix `EXDEV` or Windows `ERROR_NOT_SAME_DEVICE`; every other
rename error returns unchanged — never broaden that match. The rename probe skips
`fs_retry`, because its failure is the signal to stage and retrying would add the
backoff to every cross-drive move.

Staged moves live at `.fastf-transactions/<timestamp-pid-counter>/` in the target
base. `move.json` holds only version, operation id, project id, source base,
validated folder components and `Copying | ReadyToCommit | CleanupPending`; paths
derive from the transaction's own location. `MoveManifest::scan` is
**deny-by-default** — a link or special entry fails the whole move — and
`verify_destination` compares the exact path/type/size manifest, because a
verification narrower than the copy could remove a source that never fully
arrived. No hashes, no advanced metadata. Links are refused only on the staged
path; a rename preserves them. Every walked name is payload — there is no
transient-suffix filter.

Before publication, a cancel or failure removes only the owned transaction. After
it, cancel is too late, and a failed source removal keeps `CleanupPending` and
reports the destination published.

## Copying projects out

`copy_engine::copy_project_configured` is a move that keeps its source: the same
`MoveManifest::scan`, `MoveTransaction`, `copy_to_staging`, `verify_destination`,
`verify_source_unchanged` and atomic publish, then nothing. Both engines share
`transactions`, so **the invariant lives in one place**; a copy has no
cleanup-pending state.

**`resolve_destination` is the whole rule**, checked before any confirmation: a
real directory, not inside the project (checked first, for the right message), not
a configured base or inside one — because **the copy keeps its id**, and one id
twice in one library cannot answer "which one". Once the copy's folder is adopted
as a base, `discover` lists both (bases are unioned, not deduped);
`revalidate_project_in_base` checks path **and** id, `resolve` names the bases in
the ambiguity, and `max_id` is unaffected.

## Recovery

**Create** writes `PROJECT_INFO.md` first with `provisioning: true`, clears the
flag before removing the journal, and writes `.fastf-create-v2.json` on **every**
path. An empty journal cannot prove which interpolated files landed, so it is
reported for inspection. Creates defer no copies, but the resume branch stays for
a journal an older binary left on a shared drive, resuming after identity, type
and length checks.

**Reconcile** holds `DataLock` for the whole pass and is idempotent. `Copying` and
unpublished `ReadyToCommit` discard only the owned transaction; published
`ReadyToCommit` compares identity and manifests before cleanup; `CleanupPending`
repeats those checks before removing the source. Missing bases, malformed
journals, identity mismatches and unknown states are report-only.

**A case-only rename** stages through `.<target>.fastf-case`
(`library::lifecycle::case_staging_name`/`case_staging_target`, one spelling for
writer and recovery). Error paths roll back, and a failed rollback says where it
left the folder; a hard kill between the two renames leaves a dot-folder discovery
skips, so reconcile finishes it **forward** — the staging name carries the target,
and the old name is recorded nowhere. It requires a lifecycle-shaped name *and* a
`PROJECT_INFO.md` inside, and refuses an occupied target, because renaming a
stranger's `.X.fastf-case` over `X` would be worse than the state it repairs.
`ReconcileReport.restored` counts it, and the restore calls
`library::refresh_cache`, because a Windows rename may not move the directory's
mtime and the staleness gate alone would leave the project missing.

**Pre-v2 markers hold arbitrary absolute paths and are never authority.**
Reconcile reports them `obsolete` without parsing, migrating, following, copying,
deleting or sweeping. Never resurrect v1 JSON migration; the four tests that need
those bytes plant them literally.

## Locking and mutation

`util::lockfile::DataLock` is the cross-process lock over the data dir
(`.fastf.lock`); every read-modify-write of `counters.toml` or `config.toml` holds
it, since an in-process `Mutex` cannot see another fastf. Windows uses
`share_mode(0)`, Unix `flock`, and the OS releases both on process death, so there
is no stale lock. **Never hold it across a prompt, an editor, a reveal or a
post-create hook**: `cli::new` re-plans inside it and runs post-create outside.

**Prompt first, then lock, then reload.** The settings screen collects the answer,
then `Action::SetConfig` calls `operations::update_config`, which locks and
re-reads, because a `Config` held across a prompt would revert another `config
set` made meanwhile. Removals go by **text**, not by index. (`post_create.commands`
is read-only on both surfaces; a template's own `commands` is where it is edited.)

`core::operations` is the shared mutation entry point: lock, reload config and
authoritative identity, mutate, refresh the disposable caches. **A cached
`Project` is a hint and never authorizes deletion.** A `pub fn` under `core/` that
mutates without the lock is named `_unlocked` (`reconcile_unlocked`,
`unregister_project_unlocked`, `delete_project_unlocked`,
`rename_project_unlocked`), is `#[doc(hidden)]`, and has a `*_configured` entry
point.

**Every write to the templates directory goes through `operations`**:
`save_template(&template, original_slug)` and `delete_template(slug)`.
`Template::save_to_file` is `pub(crate)`, and `tests/layering.rs` refuses
`save_to_file(` or `remove_dir_all(` under `src/cli` or `src/tui`. When the slug
differs from `original_slug`, the directory is renamed before the manifest is
written, so no stale duplicate remains. **A save may overwrite an existing
template only when it was loaded from that slug** — a new template (`None`) named
`general` is refused — and "existing" means **a manifest**, since a bare directory
is a from-folder leftover that must stay claimable. `delete_template` refuses
anything but a real directory directly under the templates dir, because
`remove_dir_all` follows links. **Bootstrap is the one exception**: it writes the
two bundled templates without the lock, before a lock file exists, into a
templates directory it has just found empty.

`util::fs_retry` wraps the destructive calls (Windows sharing violations from
Defender or the indexer; read-only attribute clearing).

## Tags

`Metadata.tags` holds free-form strings plus tags derived from `Template.tag_from`
(`client_type` = `Indie` becomes `client_type/Indie`; empty values are skipped).
`Template::validate()` rejects a `tag_from` entry that is not a declared variable.
**`Template::auto_tags` is the one definition of the derived half**, used by
`project::provision_project`, `operations::derived_tags` and
`operations::replace_auto_tags`.

**`fastf tag reauto` removes only the tags it wrote**, recorded in
`Metadata.auto_tags`, because a prefix match on `slug/` would also delete a
template's literal `tier/legacy` and a hand-typed `tier/manual`. `auto_tags` is
`skip_serializing_if = "Vec::is_empty"`. For a file older than the field,
`Metadata::previous_auto_tags` replays the derivation and claims only results
present in `tags` — no migration; a hand-edited variable's old derived tag is
unidentifiable and left alone. An unchanged derived tag keeps its position, so a
no-op reauto writes identical bytes, and `remove_tags` prunes the record to what
`tags` still holds.

**The pane's edits** — `operations::set_variable`, `replace_tag`, `replace_note`,
`toggle_todo`, `add_todo` — take the same steps as every mutation.
`set_variable` stores a value the way a create would (`vars::validated_raw_values`
and `rendered_values` with this one replaced, so a `select` stays inside its
options and a `text` gets its transform; an undeclared variable, or any on a
registered project, is one line of free text), then re-derives the auto-tags
(`rederive_auto_tags`) and syncs the table in one `write_document`. A changed
derived tag **takes the place of the one it replaces**. `replace_tag` renames in
place, or removes when `to` is `None`.

**A tag is one word** (`validated::Tag`: trimmed, non-empty, no whitespace, at
most 64 characters, letters, digits and `- _ . /`, a `/` only between parts),
parsed at `operations::add_tags` — the one door for the CLI, the app's prompt and
the pane — so a tag never carries a newline that becomes a second YAML item. Tag
mutations call `library::refresh_cache`; the note and todo verbs do not, since the
cache stores neither.

## Search, and resolving a query

**Search** (`core/query.rs`) ANDs its predicates — no OR, no parens: bare term,
`key=value`, `key=prefix*`, `key>date`, `key<date`, `tag:value`, `tag:prefix*`.
Fields resolve from `Metadata`, then `meta.variables.<slug>`; an unknown key is
`false` rather than an error, for forward compatibility. `Predicate::Free` is the
fallthrough (anything below it is unreachable): a case-insensitive substring over
tags, variable values, folder, template, template name and id. **`path` is
excluded**, with a regression test, so home-directory text never matches.

**`library::resolve_matches(cfg, query) -> Resolution` is the shared resolver**;
`resolve` wraps it with three `pub(crate)` error builders, so a picker-driven and
a piped caller print the same messages. Tiers: exact id → **id number** → id
prefix → case-insensitive name substring. `Resolution::{NoProjects, NoMatch,
One(Box<Project>), Many(Vec<Project>)}` hands candidates to a picker as data
(`cli::target::one_project`); `One` is boxed for the Windows clippy leg's
`large_enum_variant`; `Many` carries every candidate, and only the error text is
capped at ten.

The numeric tier reads an all-digits query as an id *number* (`naming::id_value`),
so `fastf open 37` finds ID0037 under any prefix and padding. It sits **below**
exact id, because a digits-only prefix makes an all-digits string a complete id,
and **above** the prefix tier, or `4` would match ID0040–ID0049. A digit run too
long for `u64` falls through: `numeric_query` returns `None` rather than
saturating.

## Notes and todos

**`core/body.rs` is the body's grammar, and the journal is the notes.** A note is a
dated entry under `## Notes` — `- 2026-04-20T14:32:11Z — text`, further lines
indented two spaces — and `body::notes_span` is **the one definition of where
notes live, shared by the writer and the reader**: a legacy `## Journal` section
when the file has one, else `## Notes`. Separate answers lose notes written past
the point where the reader stops. New files never get a `## Journal`; a legacy
file keeps its shape.

**`render_entry` indents every line after the first**, so nothing inside a note
can start an entry or a section, and a one-line note's bytes are unchanged.
`render_preamble` (the undated note) refuses a `##` line by name, since its lines
are not indented.

**The reader never fails and never drops a line it could show.** A column-0
`- `/`* ` line whose rest holds ` — `, or whose first word is `YYYY-MM-DD…` (a
trailing `:` allowed), starts an entry; every line up to the next start is its
text, un-indented, trailing blanks trimmed, so continuations written without the
indent still read whole. Text above the first entry is one **undated** note
(`Note { timestamp: None }`) — the legacy free-text `## Notes`. `notes_in` walks
Notes, then Journal. Timestamps are not validated, so **slice one with
`.get(..10)`, never `[..10]`**, which panics on a multi-byte character. `notes
--since` compares timestamps as text (ISO-8601 sorts that way) and refuses a date
fastf does not write through `cli::recent::check_since`, since `2026-6-1` sorts
after every `2026-0…`.

**Edits name the text they read.** `replace_note(path, ordinal, expected, text)`
and `toggle_todo(path, ordinal, expected)` refuse "changed meanwhile" when the item
at `ordinal` no longer reads `expected`, because an ordinal cannot tell this note
from whatever now sits in its place. `replace_note` splices over the note's own
span (a dated note keeps its timestamp, the undated one its `##` refusal; empty
text removes it without doubling a blank line); a new note is always dated.
`toggle_todo` rewrites only the character inside the brackets; `add_todo` appends
`- [ ] text` to `## Todo`, opening the section at the end of the file when needed.

**A list opens under a blank line**: `append_in_section` writes `\n\n` before the
first item of a section that has none and `\n` before later ones, so the shape is
heading, blank line, list wherever the section sits; "holds an item" is the
reader's answer, so writer and reader agree.

## Post-create actions

`PostCreate` sits on both `Config` and `Template`; a template's block replaces the
global one entirely. Every field defaults off: `git_init`, `reveal`,
`open_in_editor`, `print_path` (for `$(fastf new ...)`), and `commands`.

**A project path never appears inside shell source.** Every child spawned for a
project goes through `post_create::project_command`, which sets the project as
`current_dir` **and** as `PROJECT_PATH_VAR` (`FASTF_PROJECT_PATH`), and
`rewrite_path_token` turns `{path}` into `"$FASTF_PROJECT_PATH"`
(`"%FASTF_PROJECT_PATH%"` on Windows), because a folder name may hold `;`, `&`,
`$`, `(`, `)` or a backtick that `sanitize_name` keeps. `{path}` is **not**
deprecated. A token already in matching quotes is replaced whole, so `code
"{path}"` is not double-quoted; Windows paths cannot contain `"`. Commands run
synchronously through `cmd /c` or `sh -c`, and **there is no sandbox** — template
authors control this.

**Reveal on Windows is `util::shell_open` (`ShellExecuteW`), not `cmd /c start`**,
because `cmd.exe` expands `%VAR%` in the command line it rebuilds, so a folder
named `%USERPROFILE%` would open the home directory. The editor stays on `cmd /c
start` (`code` is a `.cmd` shim only cmd resolves), but its path argument is the
quoted variable.

`core::post_create::run` returns `Vec<Note>`, not `Result` — each failure is a
`Note::Warning`, since the project is complete whatever the editor did — and does
**not** print. `Note::Path` is separate from `Note::Done` because `print_path`'s
line is the run's *output*: stdout, alone, last. `resolve_post_create()` is `pub`
so `cli::new`'s open-prompt does not double-open when `reveal: true` is set.

## Output

**`core` produces data; `cli::render` turns it into text.** `project::plan_report`
and `ApplyReport::of` build `DryRunReport`/`ApplyReport`, and `cli/render.rs` alone
prints them, so what a preview says is unit-tested as data. `print_tree(nodes,
indent)` is the one tree renderer (the dry run, `template show`, the builder) and
takes no variables: a preview interpolates as it builds its report, and `template
show` prints the raw `{token}` form. `PreviewKind::{DryRun, BeforeCommit}` picks
the header, and every caller must say which side of the commit it is on.

`util::diag` is the one sink for `core` and `util`: `warn` for a best-effort
failure that must not change the outcome, `note` for something the caller could
not have known (a partial project rolled back), `fatal` for the two paths with no
`Result` (an armed failpoint's `abort`, an unresolvable data directory).
