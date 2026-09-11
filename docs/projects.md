# The project model

fastf treats the filesystem as the source of truth. There is no project database. A folder is a project because it contains a `PROJECT_INFO.md` file, and everything else follows from that.

## PROJECT_INFO.md

Every project created with `fastf new` (or onboarded with `fastf register`) gets a `PROJECT_INFO.md` in its root. The file has two layers:

1. **YAML frontmatter**, machine readable. Parse it with Obsidian, Hugo, `yq`, `grep`, or your own tooling. It records the ID, template, creation time, folder, path, and every template variable.
2. **Markdown body**, human readable. A variables table, a `## Notes` section of dated notes, and a `## Todo` list once you add the first todo.

```markdown
---
id: ID0047
id_number: 47
template: music-video
template_name: Music Video
created: 2026-04-19T14:32:11Z
folder: 2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
path: /home/user/Projects/2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
variables:
  artist: Ariana_Grande
  client_type: Indie
  title: Lullaby
tags:
- music-video
- client_type/Indie
auto_tags:
- client_type/Indie
---

# Project Info

| Variable           | Value         |
|--------------------|---------------|
| Artist / Band Name | Ariana_Grande |
| Project Title      | Lullaby       |
| Client Type        | Indie         |

## Notes

- 2026-04-20T09:12:00Z — treatment approved, shoot on the 28th
- 2026-05-02T18:40:00Z — first cut sent
  the label wants the chorus shots held longer

## Todo

- [x] shoot
- [ ] colour grade
- [ ] deliver the masters
```

A new project starts with an empty `## Notes` and no `## Todo`. The two notes and the list above are what a few weeks of work leave behind: `fastf note add` or the app's quick note writes a note, the app's pane adds and ticks the todos, and you can type either in any editor.

The frontmatter `id` is authoritative. The folder name is cosmetic, so renaming a folder never breaks tracking. Only `id` and `template` are required to read the file; the rest is repaired from the folder when it is missing (see below).

`id_number` is the number behind that id, written down rather than parsed back out of it. A template may declare any `id.prefix`, digits included, and `ID0047`, `47` and `2047` cannot all be told apart by reading their trailing digits — so the number is recorded when the project is created. Projects made before fastf stored it have their number read from the id string instead, and `fastf reindex` fills the field in for them.

`auto_tags` names which of `tags` came from the template's `tag_from`, so `fastf tag reauto` can replace exactly those and nothing else. Without it the only question re-deriving could ask was whether a tag started with a `tag_from` slug and a slash, which is also true of a literal tag the template declares and of any tag you typed yourself — so it deleted those too. The key is absent when a project has no derived tags, and a project created before fastf recorded it keeps working: re-deriving reconstructs what it can from the variables in the file, and writes the record as it goes.

After creation the file is yours. fastf rewrites the frontmatter when you tag, move, rename, or register a project, or set a variable from the app's detail pane, and every rewrite leaves the rest of the file byte for byte as it was — with the exceptions that are the point of the verb. Setting a variable also rewrites the variables table under `# Project Info`, so the table stays true, but only while it is still the table fastf wrote: reshape it, rename its header, and fastf leaves it alone. A note is appended at the end of the `## Notes` section; editing one from the pane rewrites that note's own lines; toggling a todo rewrites the one character inside its brackets. That includes **keys fastf does not recognise**: add `obsidian_folder:` or anything else your own tooling needs, and it stays where you put it, in the same position in the file.

**Notes are dated entries under `## Notes`** — `- 2026-04-20T14:32:11Z — text`, with any further lines of the note indented under it. The reader is lenient: a line at the left margin starting with `- ` and a date starts a note, with or without the `—`; every line until the next one belongs to it; text above the first entry is one undated note; the heading is matched in any case and with or without a colon. A project whose notes are under a `## Journal` heading keeps them there. **Todos** are `- [ ] text` and `- [x] text` lines under `## Todo`. See [docs/cli.md](cli.md#notes).

What the pane lets you type is what the file can hold. A tag is one word; a variable is one line, and a `select` variable is one of its options — the value lands through the same transform a create applies; an undated note may not begin a line with `##`, because a second-level heading is how the file marks where a section ends, and one inside the notes would end them there (a dated note's lines are indented, so nothing in them can). Each refusal names the rule.

This matters most when two machines share one library. A newer fastf can write a key an older one has never heard of, and the older one will not delete it.

**If you break it, fastf says so.** Only `id` and `template` are required; `created`, `folder`, `path` and `template_name` can go missing and the project is still a project (`created` falls back to the folder's own date, and the other two are read from the folder itself anyway). But a `PROJECT_INFO.md` fastf genuinely cannot read — bytes that are not UTF-8, a missing `---` line, YAML that does not parse — means the folder drops out of `recent`, `search` and the app, and every command that walks the library prints a warning naming the folder and the file. Fix the file and it comes straight back; nothing else about the project has changed.

The one to watch for on a shared drive is the encoding: a Windows editor saving as the system codepage rather than UTF-8 turns one accented character into bytes fastf cannot read.

## Discovery and bases

fastf looks for projects in your **base directory** (`base-dir`, where new projects are created) plus any extra **bases** you configure:

```bash
fastf config set bases "/mnt/projects/clients,/srv/archive"
```

Discovery scans the direct children of each base and treats every folder holding a `PROJECT_INFO.md` as a project. A base that is not mounted is skipped quietly, which makes external drives and network shares practical.

A base that is mounted but does not *answer* — a network share whose host has gone away — is a different case, and fastf names it. Every list of bases is probed with a short timeout, so a dead mount is reported as `(unresponsive)` and left out of that session's move targets, instead of blocking the app for however long the operating system takes to give up on it. `(not mounted)` means the ordinary thing: nothing is there.

Each base carries a small `.fastf-index.json` cache at its root, next to the projects. The cache stores base-relative paths, so it stays valid when a drive is mounted at a different letter or path on another machine. It is a disposable accelerator, never an authority:

- If the base changed since the cache was written, fastf rescans and rewrites it.
- Cached entries whose folders disappeared are dropped automatically.
- Deleting the cache file costs one rescan and nothing else.
- **An entry can only ever point at a direct child of its own base.** The cache
  travels with the projects, which is what makes it portable — and also means a
  synced folder or an unpacked archive can bring somebody else's along. An entry
  naming anything but a plain folder name inside the base is rejected, the whole
  cache is abandoned, and the base is rescanned from the folders.
- Handing a project's path to another program — `fastf open`, `fastf copy`,
  `fastf path`, or Open project folder in the app — checks that the folder
  really is a direct child of its base and really does hold a `PROJECT_INFO.md`
  first. A cached row is a hint until it has been looked at.

There is no prune command because none is needed. For changes fastf could not observe (folders moved on another machine, hand-edited metadata), run:

```bash
fastf reindex
```

## Live folder sizes

The guided TUI shows a current Size snapshot for each project it displays, and
never waits for one: it draws its list first and fills the column in from
background workers (see [app.md](app.md#sizes)). Size is the sum of the
logical lengths of all regular files below the project folder, including hidden
files and `PROJECT_INFO.md`. Empty directories
add zero bytes. Symlinks, Windows junctions, and other links are never followed;
sockets, devices, and other special filesystem nodes are ignored.

The result is all-or-nothing. If any directory or regular file cannot be read,
the size is shown as unavailable instead of reporting a misleading partial
total. Values describe file length, not allocated disk blocks, compression, or
the size of anything reached through a link.

Sizes are deliberately absent from `PROJECT_INFO.md`, the disposable
`.fastf-index.json` cache, and the in-memory `Project` model. Project contents
can change outside fastf at any time, so a persisted value would immediately
become stale. Reopen the guided app to obtain a new snapshot. Acting
on a project in the guided app drops that project's snapshot. Tag it, rename it, or move it, and
the row is measured again when you return to the list.

## The ID counter self-heals

The counter lives **inside each base**, as `.fastf-counter.toml` next to the projects it numbers. Your project drive is already mounted by every operating system you boot, so they all read the same number — nothing to symlink, nothing to keep in sync, and a base on an external drive carries its numbering with it.

It cannot drift into collision with your projects. When planning a new one, fastf takes the highest of: every base's counter file, the highest ID actually present in your projects, and the record in fastf's own data folder. Delete the counter file and the next ID still clears every project you have. The commands are in [config.md](config.md#the-id-counter).

**Every base converges on that number.** Add three folders as bases holding `ID0004`, `ID0082` and `ID0017` and each one's counter file comes out at 82, so the next project is `ID0083` wherever you create it. A base's file wins when it is *higher* than the projects in that folder — that is what carries the number to a machine which cannot see your other drives; when the projects are higher, the file is raised and the new value pushed to the rest. This happens on every create and every `fastf id show`; `fastf id sync` forces it after something changed outside fastf.

Because nothing ever lowers it, there is no `fastf id reset`, and `fastf id set` accepts only values above the current floor — a lower one would hand out an ID that already exists.

## Moving projects between bases

```bash
fastf move ID0047 archive
```

The app's `m` does the same, over every marked project when there are marks. The rules:

- Targets must be configured bases, so a moved project always stays discoverable.
- On the same filesystem, a move is an instant atomic rename. fastf says so —
  `renamed on the same filesystem, nothing copied` — because an instant finish
  on a large folder otherwise reads as a move that did nothing.
- Across filesystems (or to network storage), fastf creates an exclusive
  `.fastf-transactions/<operation-id>/` directory beneath the target base,
  copies into its private `staging/` tree, checks exact relative paths, entry
  types, and byte lengths, confirms that the source metadata did not change,
  publishes with an atomic rename, and only then removes the source.
- Every filename is project data. Names ending in `.tmp` or `.part` are copied and verified like any other name.
- Keep the project untouched while it moves. Editing it from another program during the copy is outside the supported contract.

The source is never removed until the destination is published from a verified
staging tree. If publication succeeds but source cleanup fails, the command
reports **cleanup pending**, leaves the source and transaction in place, and
treats the destination as the completed move. `fastf reconcile` can retry that
cleanup after rechecking both project identities and the saved manifest.

Copy moves preserve regular-file contents and directory topology. They do not
promise hashes, ACLs, extended attributes, sparse layout, hard-link
relationships, symlink/junction reproduction, or storage-level durability.
Links and special entries are refused when copying would be required. The
checks are intended to prevent application mistakes and ordinary interrupted
copies; hardware failure, power loss, bit rot, and storage corruption belong to
the filesystem and backups.

## Copies, and two bases holding one ID

`fastf copy-to <query> <folder>` copies a project out of the library, keeping
its `PROJECT_INFO.md` — and therefore its ID — unchanged. The destination must
be outside every configured base, so a copy can never make a duplicate inside
one library.

Adding that folder as a base afterwards is the supported case, and it lists
both rows. Discovery is a union over the bases and does not dedupe, which is
what makes that work: the BASE column tells them apart, `s` sorts by base, and
`b` filters to one. A query that matches both is reported as ambiguous, naming
the bases rather than telling you to be more specific about an ID that is
already exact.

Each row acts on its own. Every mutation revalidates the project by **path and
ID together** before touching anything, so tagging, renaming, moving or deleting
one copy cannot reach the other. The global counter is unaffected: the same ID
twice is still the same highest ID.

## Process-crash recovery

```bash
fastf reconcile
```

Version-2 journals authorize only paths fastf can derive and validate. For
deferred creates, the journal stores a template slug and source/destination
paths relative to the template and new project. Reconciliation resumes missing
deferred files only when identity, type, and byte-length checks pass, then clears
the project's provisioning flag before removing the journal. A provisioning
flag without a usable v2 journal is reported for manual inspection.

Cross-filesystem moves use a private transaction beneath the target base:

- `Copying`: the source is authoritative; reconcile discards only that owned
  transaction.
- `ReadyToCommit` with staging still present: reconcile discards the transaction
  and leaves the source for a fresh move.
- `ReadyToCommit` after publication: reconcile requires matching source/final
  project identities and path/type/size manifests before entering cleanup.
- `CleanupPending`: reconcile rechecks the published project and, while the
  source still exists, the source/final manifests, then retries source removal.

Missing configured bases, identity mismatches, malformed journals, and unknown
states are reported without mutation. Reconciliation is explicit and
idempotent.

Create and move markers written before journal v2 are **obsolete and
report-only**. They contain arbitrary absolute paths, so `reconcile` never
parses or migrates them, follows their paths, or deletes anything they name. It
lists each marker and leaves it plus all related paths untouched. Inspect both
locations before manually removing any obsolete artifact.

## What fastf promises

fastf is a local, single-user tool for self-contained trees of ordinary
directories and regular files. It has two surfaces, the command line and the
guided app, and no network surface at all. Its commands may wait behind one
coarse mutation lock per data folder; it does not coordinate simultaneous
writers on two computers, so two machines writing one shared base at the same
moment can mint the same ID.

**What it starts.** fastf runs other programs on your behalf in five places,
all of them configuration rather than input: a template's `post_create`
commands, your editor, the file manager for Reveal, a clipboard tool
(`wl-copy`, `xclip`, `xsel`, `clip`, `pbcopy`), and — unix only — a terminal
emulator plus `notify-send`. The emulator is started only when fastf has been
asked for something interactive and can prove nothing can read its output, and
it is given fastf's own arguments as arguments, never through a shell.

**What it trusts.** One OS account. Bases, templates, `config.toml`, the
counters and the caches are your own files and are trusted as content — a
template's `post_create` commands run with your privileges, which is the
feature. Two things are enforced anyway, because they are the routes by which
a file that travels could start naming somewhere it should not: a cache entry
can only ever point at a direct child of its own base (above), and a write
fastf performs beneath a root it controls — a new project, an apply target, a
template's `files/` — never follows a link, junction or reparse point that is
already there. The path text cannot escape its root, and the filesystem
beneath it is checked component by component immediately before each write.

**What a move or copy does not promise.** Hashes, ACLs, extended attributes,
sparse-file layout, hard-link relationships, symlink or junction reproduction,
and storage-level durability are outside the contract; links and special
entries are refused when copying would be required. Process-crash recovery is
in scope; hardware failure, power loss, bit rot and storage corruption remain
the job of the filesystem and your backups.

**What stays compatible.** Command-line flags, `config.toml`, templates,
`PROJECT_INFO.md` and the caches stay compatible within a major version.
Refusing input that was once accepted but unsafe does not count as a break.

## Onboarding folders fastf did not create

See `fastf register` in the [CLI reference](cli.md#registering-existing-folders). In short: it writes a `PROJECT_INFO.md` into an existing folder, recovering an `ID####` token from the folder name when present, and `--recursive` onboards a whole base's children in one pass.
