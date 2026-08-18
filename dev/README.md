# dev/ — developer fixtures

Not shipped, not packaged, not referenced by the binary. Everything here exists
to make a **manual** pass repeatable, because the automated gates cannot see
the thing manual passes are for: how the TUI feels.

## `tui-sandbox.sh` — drive the TUI against a disposable library

```bash
dev/tui-sandbox.sh             # build (debug) + open the guided TUI
dev/tui-sandbox.sh --release   # same, optimized — slower build (LTO)
dev/tui-sandbox.sh --reset     # delete the fixture; next run reseeds it
dev/tui-sandbox.sh --shell     # print env exports for ad-hoc CLI commands
```

It **builds from source every run**, so what you are testing is always the
working tree. It cannot be stale relative to your edits.

### What makes it safe

The sandbox lives at `target/tui-sandbox/` and is isolated by two environment
variables:

- `FASTF_INSTALL_DIR` → the sandbox's own config + templates + counters
- `HOME` → redirected into the sandbox

That second one is not optional. An unconfigured `base_dir` falls back to the
**home directory** (v1.0.2), so a harness that skips the `HOME` redirect scans
your real home and self-heals its counter from your real projects. This is the
same rule every test harness in `tests/` follows.

Consequence: nothing this script runs can see `~/.config/fastf`, `/mnt/proj`,
`/mnt/base`, or the installed `/usr/bin/fastf`. `target/` is gitignored, so the
fixture never lands in a commit.

### What the fixture contains, and why each piece is there

Six projects across **two configured bases**, seeded through the binary under
test — so the fixture can never describe a library shape the current code would
not actually produce.

| | ID | Base | Template | Notable |
|---|---|---|---|---|
| Client_Reel | ID0001 | projects | general | 40 MB payload, 2 journal entries, `draft` |
| Album_Art | ID0002 | projects | general | **4 tags** — exercises the `+1` truncation |
| Acme / Launch_Film | ID0003 | projects | client-project | auto-tags from `tier` |
| Doc_Cut | ID0004 | projects | general | untagged, no journal — the empty cases |
| Globex / Sizzle | ID0005 | projects | client-project | second template variant |
| Old_Session | ID0006 | **archive** | general | makes "Move to another base" reachable |

Plus `projects/loose_folder_no_metadata/` — a real folder with no
`PROJECT_INFO.md`, so **Register** has something to point at.

Deliberate settings:

- `recent-default-limit = 4` — six projects means **two pages**, so paging is
  always exercised rather than only on a big library.
- `default-template = client-project` — so the template picker's preselect and
  its `(default)` marker are visible.
- Creation dates are **backdated** across six months. Everything seeded in one
  run carries the same timestamp, which makes newest-first ordering and
  `created>` filters untestable. The script rewrites `created` in the metadata
  and then runs `fastf reindex`, because each base's `.fastf-index.json` keeps
  its own copy of that field.
- One project is 40 MB, so the on-demand size walk is *visible* instead of
  instant — which is the point of the lazy-size behaviour.

### Running CLI commands against the same fixture

```bash
eval "$(dev/tui-sandbox.sh --shell)"
target/debug/fastf recent --plain
target/debug/fastf search "tag:draft"
```

Those exports last for the life of that shell. Open a new terminal to get your
real environment back.

### Testing against your real library instead

There is no flag for it, on purpose — it is not a sandbox operation:

```bash
cargo run --release
```

Browsing and metadata are read-only, but the action menu genuinely renames,
moves, unregisters, and deletes. Worth doing at least once per release on
`/mnt/base`, since ntfs-3g is the filesystem whose walk speed motivated
measuring sizes on demand.

## A manual TUI pass

The automated gates (`cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo test`, `cargo test --release`) come first. This is what they
cannot check:

1. **Esc** — three levels deep (Settings → Project basics → a value) and back
   out one level per press. Esc at the main menu quits. `q` does the same on any
   list.
2. **First paint** — the Projects list draws with no size scan. Open ID0001 and
   watch `measuring folder size…`, then `size:` in the header. Re-open it:
   cached. Tag it, re-open: measured again.
3. **Paging** — Next/Previous across the two pages; the second page holds two
   projects and the ordering stays newest-first.
4. **Tag pickers** — Add tag on ID0004 (untagged) offers the library's tags plus
   `+ type a new tag…`. Remove tag on ID0002 gives a checkbox list of its four.
   Add tag on a fixture with no tags at all falls through to free text.
5. **Move** — ID0006 lives in `archive`, so "Move to another base" appears; move
   it to `projects` and back.
6. **Open in editor** — opens the folder in `$EDITOR`.
7. **The frame** — create, tag, and delete a few things; the `recent` block fills
   and rolls over at three entries, and the counts and `next ID` track.
8. **Known gap** — start a create and try to Esc during the variable questions.
   It will not work: `dialoguer::Input` has no cancel. The only escape is
   declining at the final confirm.

Reset between passes with `--reset` so state from the last run cannot flatter
the next one.
