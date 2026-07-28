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
- Across filesystems (or to network storage), fastf stages a copy into a hidden temporary folder, verifies it (size, count, existence), commits it with an atomic rename, and only then removes the source.

The source is never removed until the destination is verified. If a move is interrupted by a crash or power loss, nothing is lost: the staged copy either completes or rolls back on the next recovery pass.

## Crash recovery

```bash
fastf reconcile
```

`reconcile` scans every base for interrupted work and reports or repairs it:

- **Background asset copies** resume from their durable marker.
- **Interrupted moves** either complete (if the commit already happened) or roll back with the source intact. Before removing a source, fastf confirms the destination really is the same project — a folder merely having the right name is not proof.
- **Abandoned temporary files** left by a killed copy are swept once they are over an hour old. The delay is deliberate: a `.part` file may belong to a copy running right now in another window.
- **Projects that were never finished being created** are listed. A project interrupted mid-create is still visible in `fastf recent`, marked as unfinished. `reconcile` cannot rebuild one — the values you typed are gone with the process — so it names the folder and leaves the decision to you: delete it and run `fastf new` again.

The browser UI shows a banner with a Retry button when it detects interrupted work. An unreconciled crash is always safe, just untidy.

## Onboarding folders fastf did not create

See `fastf register` in the [CLI reference](cli.md#registering-existing-folders). In short: it writes a `PROJECT_INFO.md` into an existing folder, recovering an `ID####` token from the folder name when present, and `--recursive` onboards a whole base's children in one pass.
