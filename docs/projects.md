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

The global counter and your projects can never drift into collision. When planning a new project, fastf uses `max(counter, highest ID found across all bases) + 1`. Even if the counter file is deleted or reset, the next minted ID stays above every existing project.

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

`reconcile` scans every base for interrupted work and finishes or rolls it back: background asset copies resume from their durable marker, and interrupted moves either complete (if the commit already happened) or roll back with the source intact. The browser UI shows a banner with a Retry button when it detects interrupted work. An unreconciled crash is always safe, just untidy.

## Onboarding folders fastf did not create

See `fastf register` in the [CLI reference](cli.md#registering-existing-folders). In short: it writes a `PROJECT_INFO.md` into an existing folder, recovering an `ID####` token from the folder name when present, and `--recursive` onboards a whole base's children in one pass.
