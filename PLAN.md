# PLAN.md — v3.1.0

Eleven findings from driving v3.0.0 against a real library, plus a release that
has never passed CI on its first tag. One branch, one commit per phase, one PR,
then the release.

## Phase 1 — the dashboard says each thing once  ✅
Items 1, 5, 6, 8a, 8b, 10.

- [x] The table gets a 1-column right gutter; `SIZE` header right-aligned;
      `choose_columns` becomes a strict prefix (it could show BASE with no SIZE).
- [x] Column priority: size, base, created, template, tags when more than one
      base is configured.
- [x] A base filter (`b`), cleared with the template filter by `F`.
      `Order::Base` compares the label then the full path.
- [x] `fastf recent --base <label>`.
- [x] The count is stated once, in the search bar. Header says `fast-folder`,
      drops the project and template counts. Status line drops
      "N of M projects" and "? for help".
- [x] `MarkToggle` is in the hint bar and the palette.

## Phase 2 — a templates tab  ✅
Items 7, 9. A real tab strip (`library │ templates`), the bottom strip deleted,
the studio promoted to a screen with a filter box.

## Phase 3 — batch tags that land, and a move that explains itself  ✅
Items 2, 3. Four job-path defects (dropped effects freezing the list, the
`batching()`/`targets()` mismatch failing silently, marks never cleared,
`JobStatus` write-once), a progress bar, and a move that says whether it
renamed or copied.

## Phase 4 — copy a project  ✅
Item 4. `copy_engine.rs`, `fastf copy-to`, `C` in the app. The copy keeps its
ID; the base tells duplicates apart. `patch` locates by path first.

## Phase 5 — release v3.1.0  🔄  (bumped, documented; PR run, then tag)
Item 11. Document the five CI failure patterns in the release skill, add the
"green PR run before the tag" step, bump, tag, AUR.

## Why the first tag always failed

`release.yml`'s `gates` runs the whole of `ci.yml`, so any CI flake is a release
failure. Every failure in the last 40 runs was a test failure — never a build,
MSI or AUR failure — and every one is an environment delta that cannot reproduce
on an Arch desktop: Windows 8.3 short paths, a torn tracer write under 2-core
parallelism, the pty harness acting before the first frame, a headless runner
with no DISPLAY, Windows path/cmd semantics. The missing step is a green PR run
on both platforms before the tag.
