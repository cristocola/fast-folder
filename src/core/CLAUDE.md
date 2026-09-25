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
copied, verified and published — and then it leaves the library in one rename,
never by being deleted where it stands.**

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

**The copy is made in its final place, and `PROJECT_INFO.md` is written last**
(`MoveManifest::without_root_metadata` / `only_root_metadata`,
`MoveTransaction::staging_path` = the final path for an `in_place` record). Until
that file lands the folder is not a project, so the publish is one file write and
**no folder on the target is ever renamed**: 3.12.0 staged under the transaction
and renamed the tree into place, and rclone's Drive mount put files of a moved
project back under the staging path it had just left while its cache said all
was well. `MoveTransaction::remove` takes an unpublished in-place copy with the
record and leaves a published one (`final_is_published`) alone; a record without
`in_place` (3.12.0) is finished the old way, and `sweep_strays` moves any file the
mount put in its old staging folder late into place before the record goes —
never deleting one.

**Nothing in the record is ever renamed either.** `move.json`, `manifest.json`
and `published.json` are each written once, `create_new` (`write_record_file`),
and a phase is a marker file `phase.<Name>` created and never touched again
(`set_phase`; `read_journal` takes the highest marker present over what
`move.json` says). 3.12.0 rewrote `move.json` through an atomic rename for each
phase, and the Drive mount kept `Copying` on the remote about a move that had
retired its original; the only write a cloud mount cannot misplace is one that
creates a file. The one rename left is the version-2 → 3 rewrite of a 3.11
record. `MoveTransaction::remove` waits up to twenty seconds through `EIO`,
which is a cloud mount still uploading the record's last files, and refuses
while a published record's old staging folder still holds files.

The record lives at `.fastf-transactions/<timestamp-pid-counter>/` in the target
base. `move.json` (version 3; version 2 is read) holds version, operation id,
project id, source base, validated folder components, the phase
(`Copying | ReadyToCommit | CleanupPending | Retired`), `operation` (`Move |
Copy`), the `host` and `machine` (`util::machine`, compared first: a hostname
changes with DHCP) that began it, and `legacy_cleanup`; paths derive from the
transaction's own location, the retired name from the operation. fastf's hidden
folders are recognised by prefix **and** an operation id (`retired_operation`,
`deleted_operation`, `probe_operation`), never by prefix alone, and after the
case-rename check: a project may be named `fastf-deleted-Scenes`.
`MoveManifest::scan` is **deny-by-default** for what it cannot copy — a special
entry, one it cannot examine, another filesystem — and
`verify_destination` compares the exact path/type/size/link-target manifest,
because a verification narrower than the copy could remove a source that never
fully arrived. No hashes, no advanced metadata. Every walked name is payload —
there is no transient-suffix filter.

**Links are content** (manifest version 2): recorded by their target text, never
followed, dangling allowed — what `mv` does, and what the rename always did. The
walk never descends into one, the names pass makes them last (so no later write
can pass through one), verification compares `read_link` text, and
`remove_tree` unlinks them. `transactions::entry_for` is the one classification:
every unix symlink is `Symlink`; on Windows the reparse tag decides
(`util::win_reparse`) — `SYMLINK` is `Symlink` or `DirSymlink` by the link's own
directory attribute (its target may not exist), `MOUNT_POINT` is `Junction`
unless it names a volume (`OtherFilesystem`), anything else is
`UnsupportedLink`. A link whose `read_link` is refused is `LinkNotReadable`, whose
message names sshfs's `-o no_contain_symlinks`: sshfs's default refuses every link
that is absolute or climbs with `..`, found against a real mount. The probe judges
by what `lstat` finds at the link's path, not by what `symlink()` said — on a
`follow_symlinks` mount the call makes the link on the server and then fails with
`EIO`. Cloud placeholders are not name surrogates, so `std` reads them
as files and they are copied as data. `std::fs::read_link` turns `\??\C:\x` into
the plain `C:\x` wherever it can, for the original and the copy alike, which is
what lets the two compare; `win_reparse::create_junction` takes either form.
`MoveManifest::link_notes` names the links whose meaning a new place may change
(relative ones that climb out, absolute ones into the original), and both
surfaces print them.

**`transactions::Walk` never stops at an odd entry**: it records what a manifest
can hold and, beside it, every `Problem` (listed but not examinable, unreadable
folder, link, special, a folder on another filesystem, a non-Unicode name, too
deep), so a refusal names all of them and a comparison can say "a link now, was a
312-byte file". **`MoveManifest::compare(&walk, Match)` is the one comparison**
and it compares **entries only** — never the manifest's `version`, or every
version-1 manifest an older binary left would read as changed forever. `Match`
is `Exact` (the source before publish), `Whole` (folder times ignored, since
removing a child moves them) or `Content` (a copy); `ManifestDiff::is_clean` and
`is_residue` (everything left is recorded and unchanged, entries may be missing)
are the two questions asked of it. Folder devices are compared on unix only, and
only folders: on overlayfs a file reports the device of its layer.

Before publication, a cancel or failure removes only the owned transaction. After
it, cancel is too late — **and the engine means it**: the publish's one-file copy
is handed a flag nobody sets, and every step after it runs on
`Ticker::uncancellable()`, so a late Ctrl-C can never stop a removal part of the
way. `Progress.committed` is set as the publish starts, and is what surfaces
read to answer "too late" instead of pretending to stop.

**Every step of a long job is named and counted** (`core::progress::Ticker`,
handed to `Walk::of_with`, `MoveManifest::scan_with`/`verify_*_with`,
`Cleanup.ticker` and `remove_tree`). A staged move passes through
`move_engine::MOVE_STEPS` in order — scanning, probing, copying, verifying,
publishing, setting the original aside, checking the old copy, removing it,
clearing the record — each counting what it touches, and `Ticker::phase` keeps
the finished ones in `Progress.finished` with their counts. 3.12 ran everything
after the copy under one "finalizing" with a full bar, and removing 1473 entries
through a Drive mount took ten minutes there. **A check never stops for a
cancel** (`check_removable` walks on `cleanup.ticker.uncancellable()`), because a
check stopped part of the way reads as one that failed; the removal after it
stops when its ticker honours one, leaving a redundant leftover. The public entry
points (`move_project_configured_with_outcome`, `copy_project_configured`,
`operations::reconcile_with`) end the progress through
`core::progress::settle`: `Done`, `Cancelled` when the flag stopped it, `Failed`
with the reason. `Ticker::none()` is what every caller with nobody to tell
passes, and changes nothing.

**After publication the source is retired, not deleted** (`core::move_cleanup`):
renamed in one step to `<source base>/.fastf-moved-<operation>` — same folder, so
the same filesystem, and dot-prefixed, so discovery skips it — then removed. A
tree removal is not one operation; anything that stops `remove_dir_all` part of
the way (a mode-555 folder, a file a program holds open, a network drop, an entry
a mount lists but hides) used to leave a husk that still held `PROJECT_INFO.md`
and was listed as the project. That is how 3.11 left 13 of 1473 files behind on
an sshfs base and then called the source untouched. The order is fsync the target
base (unix), `CleanupPending`, check, retire, `Retired` (written *after* the
rename; a crash between reads the same, since the retired folder is there),
bookkeeping, remove the retired copy, remove the transaction. Publishing first is
deliberate: a Windows "file in use" at the retire then costs a reconcile, not a
second copy. `MoveOutcome.source` says what became of it (`Removed`, `Leftover`,
`KeptWhole`, `Unknown`), and `SourceOutcome::warning` is the one wording both
surfaces print. `fastf delete` retires through `.fastf-deleted-<operation>` the
same way.

**The original is the authority until it is retired**
(`move_cleanup::complete_destination`): a moved copy missing entries the original
still holds exactly as recorded gets them back from the original — folders,
files through `atomic::copy` (an atomic sibling and a rename, so a crash mid-copy
never leaves a half-written file the next pass would take for the user's newer
one), links — before the rule below is asked again. Only *missing* entries are
put back; an entry that is there but differs (older than published, another
kind) is somebody's decision, and both copies are kept and listed. Found on a
real rclone Drive mount: it misplaced three uploads while the staging folder was
renamed into place. No message ever advises deleting anything.

**One rule decides removal, in-process and in reconcile**
(`move_cleanup::check_removable`): every entry is one the move recorded,
unchanged (`is_clean` for a version-3 source, `is_residue` for a retired copy and
for a source 3.11 may have half-deleted, which `legacy_cleanup` marks and carries
through the version-3 rewrite), and the moved copy holds an entry of the same kind
at every such path that is as published (`published.json`, the staging walk) or
newer — so the user may edit the moved copy while a cleanup waits, and a moved
copy restored from an older backup keeps the original. Without a published record
(3.11) "newer" is measured against the original's own time. The removal itself is
`move_cleanup::remove_tree`, not std's: it never follows a link or crosses a
device, re-checks each entry against the manifest, carries on past a failure,
gives a folder its owner's permission back, and counts what it left.

**Before a byte is copied** (`core::move_preflight`, a courtesy — correctness
never depends on it): the scan's problems refuse the move, all of them named; a
move (never a copy) probes its source base with `.fastf-probe-<operation>` —
create, write, link, lstat the link, rename, remove — because only the real
operation answers "can fastf write here" on a network mount whose server decides,
and a link that reads back as a file means the mount resolves links itself (sshfs
`follow_symlinks`), which is refused; a sticky base holding another user's folder
is refused; `util::disk_space` refuses a copy that cannot fit, and an answer it
cannot give (`None`) never refuses. Then `copy_to_staging` makes **every folder and
link before any content, and writes each file once** — folders with `create_dir`
in manifest order, links last, then each file `create_new` with its contents. It
asks the target once whether it ignores case (`target_ignores_case`, a probe
pair in staging) and, if so, reads the record for names that differ only in case
(`case_clashes`) before copying. 3.12.0 made every file empty first and filled
it in a second pass: on a cloud mount that uploaded each file twice, the second
write cancelling the first a thousand times over, and with rclone's
`--vfs-cache-mode off` the second open is refused outright. A file name the
target will not take is now found when the file is reached (`name_refusal`),
still before anything is published. Reconcile clears a probe
a killed move left, and only the names a probe holds.

**Every removal asks first whether the mount hides links**
(`move_preflight::links_hidden_in`, inside `remove_tree`, and in `fastf delete`
before its rename): the walk decides "folder" from `lstat`, and on a
`follow_symlinks` mount a linked folder says it is one. `fastf delete` also
refuses a project with another filesystem mounted inside, since the walk would
keep it and its hidden folder for good. `Removal::Leftover.kept_on_purpose` is
what separates "kept because not provably safe" from "a removal failed" — only
the second is called redundant. A publish rename that errors is checked, not
believed (`move_engine::publish`). Bookkeeping re-reads whatever project is at
the original's path instead of dropping its row. Windows needs a folder's
read-only attribute cleared before `RemoveDirectoryW` (`clear_read_only_folder`,
real folders only — on a link it would reach the target).

**A folder rename uses `fs_retry::rename_dir`**, whose ≈ 2.5 s schedule outlasts an
indexer holding a freshly written tree; the short one discarded a verified staging
copy at publish. On Windows a refused folder rename means a program has something
in it open (`describe_rename_error`).

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

**Reconcile** holds `DataLock` for the whole pass and is idempotent. Its
progress (`reconcile_unlocked_with`) counts items — what `list_incomplete`
counts, the header's "needs attention", growing if it finds more — each named
as it is taken, with that item's steps under it; a cancel is honoured between
items and inside a removal, and sets `ReconcileReport.cancelled`, which is never
an empty report. By transaction (S source, R retired copy, T staging, F
destination):

| phase | on disk | action |
|---|---|---|
| any | `host` is another machine's | report only |
| any | `operation = Copy` | discard T if unpublished, clear the record; never touch S |
| Copying, ReadyToCommit | R present | report |
| Copying | S ours; F absent, or fastf's own unfinished in-place copy (no readable identity) | discard, F included |
| Copying (in place) | F holds our `PROJECT_INFO.md` | published an instant before the crash: as CleanupPending |
| Copying | otherwise | report |
| ReadyToCommit (in place) | any | never written by this version: report |
| ReadyToCommit | T, no F | discard |
| ReadyToCommit | F ours, no T | exact source + content, then as CleanupPending |
| CleanupPending, Retired | F not ours | report; with R present, say R may be the only copy |
| CleanupPending | R present | write `Retired`, bookkeeping, remove R |
| CleanupPending | S present | check, retire, `Retired`, bookkeeping, remove R |
| CleanupPending, Retired | no R, and no S or `Retired` | bookkeeping, sweep the old staging's strays into place (`finish_record`), clear the record |

`reconcile_base` and `list_incomplete` never look inside `.fastf-moved-*` or
`.fastf-deleted-*` for a create to resume. A retired folder no transaction owns is
reported, never removed; a deleted project's folder is removed (the word confirmed
it). Every message names the project, its record and phase, and what is on disk;
"left untouched" about a pass is not a statement about the disk. `leftovers` holds
the hidden folders not removed yet, `cleared` the deleted ones that were.

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
and for a todo `toggle_todo`, `replace_todo`, `add_todo_in` and `add_todos_in`,
which `fastf todo` calls too — take the same steps as every mutation.
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

**Edits name the text they read.** `replace_note(path, ordinal, expected, text)`,
`toggle_todo(path, ordinal, expected)` and `replace_todo(path, ordinal, expected,
text)` refuse "changed meanwhile" when the item at `ordinal` no longer reads
`expected`, because an ordinal cannot tell this note from whatever now sits in its
place; the command line passes the text its own `list` read, the pane the row's.
`replace_note` splices over the note's own span (a dated note keeps its timestamp,
the undated one its `##` refusal; empty text removes it without doubling a blank
line); a new note is always dated.

**A todo is edited through the ranges `place_todos` records**: `marker` (the one
character inside the brackets, empty for `[]`) is what `toggle_todo` rewrites;
`text` (just after the `]` to before any `\r`/`\n`) is what `replace_todo`
replaces with ` <text>`, so indent, `-`/`*`, bracket state and a CRLF ending
survive; `line` (the whole line, its ending included) is what an emptied
`replace_todo` hands `remove_lines`. `section_span` stops short of the `\n` before
the next heading, so the last task of a section above another is extended by that
byte, or a removal would leave its newline behind. `remove_lines` counts `\r\n` as
a blank line too. A phase label is never removed with the last task under it: it
is the user's line. A todo is one line; `replace_todo` and the adders refuse
`\n`/`\r` before reading, and the `operations` wrappers before the lock.

`add_todos_at(path, texts, place)` is **one** atomic write of the whole run —
trimmed, empties skipped, one line each or nothing written — at a `TodoPlace`:
`Phase(name)`, after the last task of the last run of that label (matched ignoring
case), or under a new label above `### Other` or at the section's end; `Loose`,
after the last task that sits under no label, or above the first label when there
is none; `End`, appended at the end of `## Todo`; each opening the section when
needed. **It answers the ordinal the first todo got** — read back from what it
wrote, from where the block starts — because the app's guess at it was wrong
wherever a label's name repeated or differed in case, and the pane settles its
cursor on that answer. `add_todos_in`, `add_todo_in` and `add_todo` are it with a
phase or none, so there is one placement rule. A phase name goes through
`phase_label` first, which strips every leading `#`/space and trailing `:`/space
at once: a name written as `### Grade:` reads back as `Grade`, and would open a
new label on every add. `phase_labels_in` reads every label with the tasks above
it, empty ones included, for a reader that must draw where the writer will put
a task.

**A file saved with `\r\n` keeps them** (`with_line_endings_of`): the adders, the
note appender and `replace_note` make their edit on the file's `\n` reading and
give it back in `\r\n` when every line of the file ended that way; a file mixing
the two is written as it is, since there is no one ending to keep. `replace_todo`
and the toggle touch no line ending at all.

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
`Result` (an armed failpoint's `abort`, an unresolvable data directory). Each is
also written to the log (`util::log`), whichever surface shows it.

**The log is facts, messages are sentences.** `util::log` writes
`<data dir>/logs/fastf.log`, one line per event (`stamp LEVEL job pid text`,
continuation lines indented), with one `write` on an append-mode file so
processes interleave whole lines, and rotates past 4 MiB under a try-lock.
A job's steps reach it through the `Ticker`, which logs each step at info once
`Ticker::subject` names the job, and each entry at debug. `util::messages` keeps
what a person was shown (`messages.log`, JSON lines, a tolerant reader) and
writes each to the log too. In a `cfg(test)` build the log writes only where
`FASTF_INSTALL_DIR` is set, so `cargo test` never fills the developer's own.
