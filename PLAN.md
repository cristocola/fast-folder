# PLAN.md — v3.2.0

The eight findings the Windows pass reproduced and did not fix, plus the release
the eleven commits since v3.1.4 are waiting for. None of the eight is
Windows-specific; they were recorded rather than fixed because that session was
scoped to Windows filesystems. One branch, one commit per phase, one PR, then
the release.

**v3.2.0, not v3.1.5.** The backlog is not only fixes — the template builder
gained validation, an Esc ladder and footer help — and this plan adds wildcard
matching to the search grammar and new refusals to `search` and `recent`. Flags
and schemas stay compatible.

## Phase 1 — the diagnostics the app already has
`query::diagnose` and `query::looks_like_a_date` exist and are called from the
guided app's search bar and nowhere else.

- [ ] `fastf search` refuses a malformed clause instead of printing "No projects
      match" with exit 0. `created<tomorrow` matched *every* project through a
      lexicographic compare. Placed **after** `hand_off_to_a_terminal`, so a
      launcher-started query shows its refusal in the window it opens.
- [ ] `fastf recent` validates `--since` (`2026-6-1` silently dropped every 2026
      project), `--base` and `--template`, in the shape `--limit 0` already uses.
- [ ] `base_matches` takes a `&Path`, so the filter and the validation cannot
      spell "which base is that" two ways.

## Phase 2 — the counter is one number, shown once
- [ ] `print_counter` prints the value raw and takes "next" from
      `Counters::next_value`, the one expression for it. It formatted with
      `templates[0]`'s prefix behind a comment claiming templates share one, so
      `fastf id show` printed `Global project ID: 202001  (next will be 2002)`.

## Phase 3 — a preview names only what a create will write
The dry run's "Files:" list walks the real tree and filters correctly; the
"Previews:" section iterates the in-memory text buffer and applies neither
`exclude` nor `verbatim`, so an excluded file is shown and a verbatim one is
shown with its `{braces}` filled in — the opposite of what the copy does.

- [ ] One classifier in `core::assets` (`Disposition::{Skip, Interpolate,
      Verbatim}`) that the preview and `copy_template_files` both call, so the
      two agree by construction rather than by two lists that happen to match.
- [ ] Both reserved predicates fold into `Skip`; the file list checked only
      `project_info`'s while the copy checked `provisioning`'s too.
- [ ] Preview paths go through `assets::interp_rel_with` with `plan.ctx`, like
      the file list — not `naming::interpolate` with a second clock sample.

## Phase 4 — a template is what its directory says
- [ ] `load_all` refuses a manifest whose `slug` is not its directory name, the
      same way it refuses an invalid slug. Listed under a name no command
      accepts, such a template made `fastf new` print a full preview and *then*
      fail. The slug is the directory: `save_template` renames the folder to
      match it, so only a hand-edited manifest can reach this state.

## Phase 5 — the small truths
- [ ] `template show` stops listing a stripped root `PROJECT_INFO.md` as an
      asset "copied byte-for-byte" when every copy path drops it.
- [ ] `from-folder` writes `display_path`, not `\\?\C:\…`, into a description.
- [ ] The "no folder" refusal says what actually went missing, once, through
      `display_path` at all three layers.
- [ ] `key=*value*` matches what it looks like it matches: `Pattern` gains
      `Suffix` and `Contains`, and six sentences calling it a glob become true.

## Phase 6 — housekeeping
- [ ] The stale v3.1.0 PLAN.md replaced by this one; root `CLAUDE.md`'s
      `fs_retry` line carries the read-only attribute; ROADMAP's open-findings
      section deleted and its regression coverage recorded against v3.2.0.

## Phase 7 — release v3.2.0
- [ ] Version bump, ROADMAP row, release notes, dependabot #49 merged first.
- [ ] **A green PR run on both platforms before the tag.** `release.yml`'s
      `gates` is the whole of CI, so any CI failure is a failed release — the
      step whose absence cost three attempts on v2.0.0 and four on v3.0.0.
- [ ] Both AUR packages bumped and pushed; this file deleted.
