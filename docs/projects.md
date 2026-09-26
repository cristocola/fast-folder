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

### Post
- [ ] colour grade
- [ ] deliver the masters
```

A new project starts with an empty `## Notes` and no `## Todo`. The two notes and the list above are what a few weeks of work leave behind: `fastf note add` or the app's quick note writes a note, `fastf todo add` and the app's pane add and tick the todos, `fastf todo edit` and `fastf todo remove` reword and remove them, and you can type either in any editor.

A `###` line inside `## Todo` is a **phase**: every task under it belongs to it, until the next one. Write them when a list grows long enough to want grouping, and leave them out when it does not. The pane draws each label over the run of tasks it names; a label is not a task, so it changes no numbering and ticking a todo never touches it.

The frontmatter `id` is authoritative. The folder name is cosmetic, so renaming a folder never breaks tracking. Only `id` and `template` are required to read the file; the rest is repaired from the folder when it is missing (see below).

`id_number` is the number behind that id, written down rather than parsed back out of it. A template may declare any `id.prefix`, digits included, and `ID0047`, `47` and `2047` cannot all be told apart by reading their trailing digits — so the number is recorded when the project is created. Projects made before fastf stored it have their number read from the id string instead, and `fastf reindex` fills the field in for them.

`auto_tags` names which of `tags` came from the template's `tag_from`, so `fastf tag reauto` can replace exactly those and nothing else. Without it the only question re-deriving could ask was whether a tag started with a `tag_from` slug and a slash, which is also true of a literal tag the template declares and of any tag you typed yourself — so it deleted those too. The key is absent when a project has no derived tags, and a project created before fastf recorded it keeps working: re-deriving reconstructs what it can from the variables in the file, and writes the record as it goes.

After creation the file is yours. fastf rewrites the frontmatter when you tag, move, rename, or register a project, or set a variable from the app's detail pane, and every rewrite leaves the rest of the file byte for byte as it was — with the exceptions that are the point of the verb. Setting a variable also rewrites the variables table under `# Project Info`, so the table stays true, but only while it is still the table fastf wrote: reshape it, rename its header, and fastf leaves it alone. A note is appended at the end of the `## Notes` section; editing one from the pane rewrites that note's own lines; toggling a todo rewrites the one character inside its brackets; rewording a todo rewrites the text after its brackets and keeps the indent, the list marker, the tick and the line ending; removing a todo takes its one line, and a blank line only where one would otherwise be left doubled. A file saved with Windows line endings keeps them on every line fastf adds or rewrites. That includes **keys fastf does not recognise**: add `obsidian_folder:` or anything else your own tooling needs, and it stays where you put it, in the same position in the file.

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
- Across filesystems (or to network storage), fastf writes a record of the
  move under `.fastf-transactions/<operation-id>/` in the target base, copies
  everything but `PROJECT_INFO.md` into the project's final folder there,
  looks at the original again and copies whatever changed meanwhile, checks
  exact relative paths, entry types, byte lengths and link targets, and then
  writes `PROJECT_INFO.md`. That one file is the publish: until it lands the
  folder is not a project, and no folder on the target is ever renamed.
  **Only then does the original leave the library, in one step**: it is
  renamed, beside itself, to a hidden `.fastf-moved-<operation-id>` folder —
  or, on a cloud mount, its `PROJECT_INFO.md` is removed first (below). What is
  left of it is then merged into the moved copy and removed, entry by entry.
- Every filename is project data. Names ending in `.tmp` or `.part` are copied and verified like any other name.
- **Several entries at once.** A walk, a copy and a removal each ask the
  filesystem about several entries at the same time — a few on a local disk,
  more on a network share or sshfs, most on a cloud mount, where every one is
  a request that waits on the network.
- **Links travel as links.** A symbolic link — or, on Windows, a junction — is
  made again at the destination pointing exactly where it pointed, by the text
  of its target. It is never followed: nothing behind it is copied, and removing
  the original never deletes through it. A link whose target does not exist
  (what `node_modules/.bin` often holds) moves like any other. When a link's
  meaning may change at the new place — a relative link that climbs out of the
  project, an absolute one into the original folder — the move says which.
- **You can keep working.** A change made to the original while it moves —
  a dev server writing its log, a temp folder a build makes and removes, a
  file you save — is copied too: fastf looks at the original again after the
  copy and brings across what changed, up to three times, and what changes
  after that is carried into the moved copy when the original is removed. A
  program still running in the old folder keeps writing where it was started,
  though: restart it from the new place.
- **An empty folder at the target name gives way.** A cloud mount can leave
  one behind for a while after a folder is removed, and it holds nothing to
  lose; anything else there refuses the move, and says when it is a folder
  fastf is still removing.

**Only what fastf cannot fix stops a move**, and it says what and what to do:
no permission where it has to write or remove, **a program writing a file in
the project** (Linux names it — `python3 (pid 4242) has session.txt open for
writing` — since after the move its writes would land in the old copy), a full
drive, a read-only one, a name the drive will not take. Everything else is
waited out: an input/output error or a lock a moment long is asked again with
a growing pause, a mount that drops is waited for until the same mount is back
— an unmounted mount point is an empty folder, and nothing under it is taken
for gone — and a move whose mount stays gone for two minutes before its
publish **pauses**, keeping what it copied; moving the project again, or `fastf
reconcile` once the mount is back, takes the copy over and copies only what is
missing. A program merely running in the folder — a dev server, a shell — does
not stop the move; the result names it, and says to restart it from the new
place.

**Before a byte is copied**, a cross-filesystem move checks what would
otherwise stop it at the end, and names every problem it finds at once:

- entries it cannot copy — a socket, a pipe, a device, an entry the filesystem
  lists but will not let it examine, a different filesystem mounted inside the
  project;
- that it can write and rename in the source base, by trying — a read-only
  base, or a share you can only read, is refused, since a move has to remove
  the original afterwards (`fastf copy-to` copies without removing);
- that the source base shows a link as a link — see *mounts that resolve
  links* below;
- that the target has room, when its filesystem says;
- that the target can hold every name: every folder and link is made before
  any content is copied, and a drive that ignores case is asked so, and the
  project's names checked against each other, first. A file name the drive
  will not take is found when that file is reached, still before anything is
  published. Each file is written exactly once, which is what a cloud mount
  needs: it uploads every write.

**Why no folder on the target is renamed.** A cloud mount such as rclone with
a cache uploads in the background, and renaming a folder while its uploads are
still in flight can land some of them at the old path; the mount's own view
shows nothing wrong. fastf 3.12.0 renamed its finished copy into place, and on
a Google Drive mount three files of a moved project ended up under the folder
it had just left. Writing the copy in its final place, with `PROJECT_INFO.md`
last, means there is nothing to misplace.

**Why the original leaves in one step before it is removed.** Removing a
folder is not one step: anything that stops it part of the way — a read-only
folder inside, a file another program holds open, a dropped network connection
— would leave part of the project where it was, still holding its
`PROJECT_INFO.md`, so still listed as the project. So the original first leaves
the library in one step that either happens or does not, and only then is the
rest removed:

- On a local disk, sshfs, SMB or NFS it is **renamed** beside itself to a
  hidden `.fastf-moved-<operation-id>` folder.
- On a **cloud mount** (rclone, or a mount fastf does not know) a folder rename
  is a copy and a delete for every file in it — twelve minutes for one web
  project on an S3 bucket — and moves uploads still in flight to the old path.
  There the original is **emptied where it stands**: a small note
  `.fastf-moved-<operation-id>.json` beside it names the move, its
  `PROJECT_INFO.md` goes first, which takes it out of the library, and the rest
  follows. Its folder name stays taken until it is empty.

**What is left of the original is merged, not just deleted.** Every entry is
looked at once, and removed only when the moved copy holds it:

- something the moved copy is missing — a cloud mount that misplaced an upload,
  a file deleted there since — is put back into it from the original first;
- something that changed in the original since it was copied — a log line the
  dev server wrote a moment before the move finished — replaces the moved
  copy's version, when that one is untouched since the move and it is within
  the hour after it;
- something new in the original is copied across;
- a file whose size agrees and whose time alone moved (a cloud mount's rename
  can do that) is compared byte for byte before it is called changed.

What is left afterwards is exactly what needs you: a file changed both in the
original and in the moved copy since the move, or one that the moved copy holds
as something else. Only those stay, hidden, with the move's record, and the
report names each one; everything else is gone. Nothing is removed that exists
nowhere else, you can go on working in the moved copy while a cleanup waits,
and a moved copy restored from a backup taken before the move keeps the
original's version of what it holds older. `PROJECT_INFO.md` is never written
into the moved copy: its own is the project's.

If the original cannot leave at all — on Windows, a program has a file in it
open — the move reports that **the original is still there, whole, and fastf
removed nothing**, and why. The moved copy is complete and is the project.
Once the reason is gone, `fastf reconcile` finishes the move without copying
again. If removing the old copy stops part of the way, what is left is hidden,
never listed as a project, and everything in it is also in the moved copy;
`fastf reconcile` finishes it.

Copy moves keep what `mv` keeps: regular-file contents, directory topology,
links, and every file's and folder's permission bits (on Windows, the
read-only attribute) and modification and access times. A read-only folder
inside the project is made read-only again after its files have arrived. The
project folder itself keeps the target's default permissions, and its time is
the move's, since the move rewrites its `PROJECT_INFO.md`. They do not promise
hashes, ACLs, ownership, extended attributes, sparse layout, hard-link
relationships (hard-linked files arrive as separate files), or storage-level
durability. The checks are intended to prevent
application mistakes and ordinary interrupted copies; hardware failure, power
loss, bit rot, and storage corruption belong to the filesystem and backups.

**Mounts that resolve links.** fastf can only see what the filesystem shows
it. A network mount that follows links on the server — sshfs's
`follow_symlinks` option — shows a link as whatever it points to: a copy would
take in the target's content in the link's place, and removing the original
would delete through it. The source-base check makes a link and reads it back,
so on such a mount the move is refused and names the option; mount it without
that option. Every removal asks the same question first — `fastf delete`, and
reconcile removing a set-aside original or a deleted project's hidden folder —
and on such a mount removes nothing. A Samba share that follows links for a client without unix
extensions behaves the same way but cannot be checked, because the client
cannot make a link there to read back — mount it with unix extensions, or move
on the server.

sshfs has a second default in the way: `contain_symlinks` refuses to read any
link that is absolute or climbs with `..` — which is every link in a
`node_modules/.bin`. fastf cannot copy a link it cannot read, so the move names
each one and suggests mounting with `-o no_contain_symlinks`. For moving
projects over sshfs, mount with that option and without `follow_symlinks`.

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

**A move, a copy, a delete and a reconcile each run in a process of their own**
— a *job* — started by the app or the command line and followed by it, but
not owned by it. Closing the terminal, quitting the app or killing either
leaves the job to finish, and any fastf started meanwhile sees it: `fastf jobs`
lists it, `L` in the app shows it. A job killed itself (a power cut, a
`kill -9`) leaves its record, and `fastf reconcile` finishes it by the table
below. While a job runs, reconcile leaves its records alone and the header does
not count them as needing attention.

A move holds the library's lock only until the original is set aside;
removing the old copy — the slow part on a cloud mount, one request per entry
— runs after, so a tag or a note added meanwhile lands at once.

Cross-filesystem moves use a private transaction beneath the target base:

- `Copying`: the source is authoritative. A folder at the destination without
  `PROJECT_INFO.md` is fastf's own unfinished copy, and goes with the record;
  one that already holds the project's `PROJECT_INFO.md` was published the
  instant before the crash, and the move is finished from there.
- Records that fastf 3.12.0 wrote staged under the transaction and renamed the
  copy into place. Reconcile finishes those the old way, and any file a cloud
  mount uploaded into that old staging folder after the rename is moved into
  place, never deleted.
- `CleanupPending`: the original is still at its path. Reconcile checks that
  it and the moved copy are both still this project, sets it aside, and merges
  it away as above.
- `Retired`: the original has been set aside. Reconcile merges what is left of
  it. More than an hour after the move, and for anything that came back after
  it was gone, the merge only removes what the move recorded, unchanged and in
  the moved copy, and writes nothing into the moved copy. If the moved copy
  has since gone, the old copy may be the only one left, and reconcile says
  where it is and how to rename it back.

A move left half-finished by fastf 3.11 or older may have deleted part of its
original already, since those versions removed it in place. Reconcile finishes
it when everything left is exactly what the move recorded, and otherwise lists
what differs. A move another machine began is reported, never acted on: the
source path it names means something else here.

Hidden folders fastf leaves beside the projects are handled too. A set-aside
original with no record left — fastf 3.13 cleared records too early — is
finished by content: one holding nothing but empty folders is removed, and
otherwise everything in it that the project it held (found by its ID) holds the
same, byte for byte, is removed and the rest kept and named; when fastf cannot
find that project it keeps all of it and says it may be the only copy. A
deleted project's `.fastf-deleted-*` folder is removed (you confirmed the
delete), and so is a move's `.fastf-probe-*` folder.

**A delete on a cloud mount** empties the folder where it stands, like a move's
original: a `.fastf-deleted-<operation-id>.json` beside it lists everything the
project held when you deleted it, its `PROJECT_INFO.md` goes first, and then
only what the list names — anything a program wrote there after the delete
began is kept, and named.

**A record is never lost while anything it owns is left.** fastf counts
something as gone only when the filesystem says "nothing there"; a mount that
answers with an error — dropped, restarting, not answering — says nothing, and
the record waits for it. Reconcile then reports the move as **waiting for a
base to answer**, changes nothing, and finishes it once the base is back; an
unplugged drive is waiting too, never something you have to look at. fastf
also keeps a small index of every record in its data folder, so a record in a
base you have since removed from `bases`, or beside a `copy-to` destination, is
still found and finished.

On an rclone mount a removed folder can come back: an upload still queued in
rclone's cache lands after the removal. There, the record of a move — or of a
delete — stays for ten minutes after its old copy is removed, unseen; a
reconcile in that time removes whatever came back, and one after it clears the
record.

A rename that stopped part of the way — an S3 bucket through rclone renames
object by object — leaves part of the original at its path and part at its
hidden name. Both are the move's own: reconcile merges the hidden part, then
removes what is left at the original's path where it is what the move
recorded, and clears the record only after both.

Every line reconcile prints names the project, its transaction and what is on
disk. Missing configured bases, identity mismatches, malformed journals, and
unknown states are reported without mutation. Reconciliation is idempotent,
and the next pass notices anything you settle yourself in the meantime.

Create and move markers written before journal v2 are **obsolete and
report-only**. They contain arbitrary absolute paths, so `reconcile` never
parses or migrates them, follows their paths, or deletes anything they name. It
lists each marker and leaves it plus all related paths untouched. Inspect both
locations before manually removing any obsolete artifact.

## What fastf promises

fastf is a local, single-user tool for self-contained trees of ordinary
directories, regular files and links. It has two surfaces, the command line and the
guided app, and no network surface at all. Its commands may wait behind one
coarse mutation lock per data folder; it does not coordinate simultaneous
writers on two computers, so two machines writing one shared base at the same
moment can mint the same ID.

**What it starts.** fastf runs other programs on your behalf in five places,
all of them configuration rather than input: a template's `post_create`
commands, your editor, the file manager for Reveal, a clipboard tool
(`wl-copy`, `xclip`, `xsel`, `clip`, `pbcopy`), and — unix only — a terminal
emulator plus `notify-send`. It also starts itself: a move, a copy, a delete
and a reconcile each run as a copy of fastf, detached from the terminal, with
nothing on its command line but the job it is to read from the data folder. On
Linux with a systemd user session it is started through `systemd-run --user
--scope`, so it belongs to no other program's unit: a desktop launcher that
stops the app's unit when the app quits does not stop the move with it. The emulator is started only when fastf has been
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

**What a move or copy does not promise.** Hashes, ACLs, ownership,
extended attributes, sparse-file layout, hard-link relationships and
storage-level durability are outside the contract; permission bits and times
are kept, as `mv` keeps them. Links are reproduced by
their target text; sockets, pipes and device files are refused before anything
is copied. A filesystem that hides links (a mount that resolves them on the
server) is outside what fastf can see — the move checks for the one kind it can
detect. Process-crash recovery is in scope; hardware failure, power loss, bit
rot and storage corruption remain the job of the filesystem and your backups.

**What stays compatible.** Command-line flags, `config.toml`, templates,
`PROJECT_INFO.md` and the caches stay compatible within a major version.
Refusing input that was once accepted but unsafe does not count as a break.

## Onboarding folders fastf did not create

See `fastf register` in the [CLI reference](cli.md#registering-existing-folders). In short: it writes a `PROJECT_INFO.md` into an existing folder, recovering an `ID####` token from the folder name when present, and `--recursive` onboards a whole base's children in one pass.
