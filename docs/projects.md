# The project model

fastf treats the filesystem as the source of truth. There is no project database. A folder is a project because it contains a `PROJECT_INFO.md` file, and everything else follows from that.

## PROJECT_INFO.md

Every project created with `fastf new` (or onboarded with `fastf register`) gets a `PROJECT_INFO.md` in its root. The file has two layers:

1. **YAML frontmatter**, machine readable. Parse it with Obsidian, Hugo, `yq`, `grep`, or your own tooling. It records the ID, template, creation time, folder, path, and every template variable.
2. **Markdown body**, human readable. A variables table plus a `## Notes` section you own, and a `## Journal` section once you add the first note.

```markdown
---
id: ID0047
template: music-video
template_name: Music Video
created: 2026-04-19T14:32:11Z
folder: 2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
path: /home/user/Projects/2026-04-19_Ariana_Grande_Lullaby_Indie_ID0047
variables:
  artist: Ariana_Grande
  client_type: Indie
  title: Lullaby
---

# Project Info

| Variable           | Value         |
|--------------------|---------------|
| Artist / Band Name | Ariana_Grande |
| Project Title      | Lullaby       |
| Client Type        | Indie         |

## Notes
```

The frontmatter `id` is authoritative. The folder name is cosmetic, so renaming a folder never breaks tracking. After creation, fastf only touches the file through `fastf tag` and `fastf note`, both of which preserve the rest of the file byte for byte.

## Discovery and bases

fastf looks for projects in your **base directory** (`base-dir`, where new projects are created) plus any extra **bases** you configure:

```bash
fastf config set bases "/mnt/proj/01_PROJECTS,/srv/archive"
```

Discovery scans the direct children of each base and treats every folder holding a `PROJECT_INFO.md` as a project. A base that is not mounted is skipped quietly, which makes external drives and network shares practical.

Each base carries a small `.fastf-index.json` cache at its root, next to the projects. The cache stores base-relative paths, so it stays valid when a drive is mounted at a different letter or path on another machine. It is a disposable accelerator, never an authority:

- If the base changed since the cache was written, fastf rescans and rewrites it.
- Cached entries whose folders disappeared are dropped automatically.
- Deleting the cache file costs one rescan and nothing else.

There is no prune command because none is needed. For changes fastf could not observe (folders moved on another machine, hand-edited metadata), run:

```bash
fastf reindex
```

## Live folder sizes

The browser UI shows a current Size snapshot for each project it displays. The
guided TUI measures a project when you open its action menu, rather than
scanning every row of a page before the list can draw. Size is the sum of the
logical lengths of all regular files below the
project folder, including hidden files and `PROJECT_INFO.md`. Empty directories
add zero bytes. Symlinks, Windows junctions, and other links are never followed;
sockets, devices, and other special filesystem nodes are ignored.

The result is all-or-nothing. If any directory or regular file cannot be read,
the size is shown as unavailable instead of reporting a misleading partial
total. Values describe file length, not allocated disk blocks, compression, or
the size of anything reached through a link.

Sizes are deliberately absent from `PROJECT_INFO.md`, the disposable
`.fastf-index.json` cache, the in-memory `Project` model, and `/api/state`.
Project contents can change outside fastf at any time, so a persisted value
would immediately become stale. Re-open the project in the TUI Projects browser
or use the browser UI's Refresh action to obtain a new snapshot.

## The ID counter self-heals

The counter lives **inside each base**, as `.fastf-counter.toml` next to the projects it numbers. Your project drive is already mounted by every operating system you boot, so they all read the same number — nothing to symlink, nothing to keep in sync, and a base on an external drive carries its numbering with it.

It cannot drift into collision with your projects. When planning a new one, fastf takes the highest of: every base's counter file, the highest ID actually present in your projects, and (for upgrades) the old counter from the config directory. Delete the counter file and the next ID still clears every project you have.

**Every base converges on that number.** Add three folders as bases holding `ID0004`, `ID0082` and `ID0017` and each one's counter file comes out at 82, so the next project is `ID0083` wherever you create it. A base's file wins when it is *higher* than the projects in that folder — that is what carries the number to a machine which cannot see your other drives; when the projects are higher, the file is raised and the new value pushed to the rest. This happens on every create and every `fastf id show`; `fastf id sync` forces it after something changed outside fastf.

Because nothing ever lowers it, there is no `fastf id reset`, and `fastf id set` accepts only values above the current floor — a lower one would hand out an ID that already exists.

## Moving projects between bases

```bash
fastf move ID0047 archive
```

Moves are also available from the `fastf recent` action menu and the browser UI (including multi-select bulk moves). The rules:

- Targets must be configured bases, so a moved project always stays discoverable.
- On the same filesystem, a move is an instant atomic rename.
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
locations before manually removing any obsolete artifact. The browser UI shows
the same v2 recovery and obsolete-marker report.

## Onboarding folders fastf did not create

See `fastf register` in the [CLI reference](cli.md#registering-existing-folders). In short: it writes a `PROJECT_INFO.md` into an existing folder, recovering an `ID####` token from the folder name when present, and `--recursive` onboards a whole base's children in one pass.
